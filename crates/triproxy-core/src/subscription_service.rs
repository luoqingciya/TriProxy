// TriProxy — a modern multi-core proxy client.
// Copyright (C) 2026 luoqingci
// SPDX-License-Identifier: GPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. It is distributed in the hope that it will be useful, but WITHOUT ANY
// WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
// PARTICULAR PURPOSE. See LICENSE for the full text.
//
// Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.

//! Subscription lifecycle: fetch, parse, store, refresh.
//!
//! The rule that shapes this module: **a failed refresh never destroys what the
//! user already has.** Providers go down, URLs expire, networks flake. Losing a
//! working node list because one request timed out would be the worst possible
//! behaviour, so a failure only records `last_error` and leaves the profile
//! untouched.

use crate::error::{Error, Result};
use crate::logging::now_ms;
use crate::profile::parse_payload;
use crate::store::{ProfileStore, StoredProfile, StoredRecord};
use crate::subscription::{FetchOutcome, Subscription, SubscriptionClient};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshStatus {
    Updated {
        node_count: usize,
    },
    /// Provider said the cached copy is current (HTTP 304).
    NotModified,
    /// The attempt failed; the stored profile is unchanged.
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshOutcome {
    pub id: String,
    pub name: String,
    pub status: RefreshStatus,
}

pub struct SubscriptionService<'a> {
    store: &'a ProfileStore,
    client: SubscriptionClient,
}

impl<'a> SubscriptionService<'a> {
    pub fn new(store: &'a ProfileStore) -> Result<Self> {
        Ok(Self {
            store,
            client: SubscriptionClient::new()?,
        })
    }

    /// Fetch a URL and store it as a new subscription.
    ///
    /// Unlike a refresh, the first fetch must succeed: there is nothing to fall
    /// back to, and storing an empty profile would be worse than an error.
    pub async fn add(&self, url: &str, name: &str, interval_secs: u64) -> Result<StoredProfile> {
        let mut subscription = Subscription::new(url, interval_secs);

        let text = match self.client.fetch(&subscription).await? {
            FetchOutcome::Updated {
                text,
                etag,
                last_modified,
            } => {
                subscription.etag = etag;
                subscription.last_modified = last_modified;
                text
            }
            // A conditional request cannot be answered 304 when we sent no
            // validators, so this only happens with a broken server.
            FetchOutcome::NotModified => {
                return Err(Error::Other(format!(
                    "{url} answered 304 to an unconditional request"
                )))
            }
        };

        let mut parsed = parse_payload(&text, name, url);
        parsed.profile.name = name.to_string();
        subscription.last_fetch_ms = now_ms();

        // Re-adding under an existing name updates that record instead of
        // creating a second entry with the same title.
        let existing = self.store.find_by_name(name)?;
        self.store.save_subscription(
            &parsed.profile,
            existing.as_ref().map(|entry| entry.id.as_str()),
            subscription,
        )
    }

    /// Refresh one subscription.
    pub async fn refresh(&self, id: &str) -> Result<RefreshOutcome> {
        let mut record = self.store.record(id)?;
        let name = record.profile.name.clone();

        let Some(mut subscription) = record.subscription.clone() else {
            return Err(Error::Other(format!(
                "`{name}` is not a subscription; it has no URL to refresh"
            )));
        };

        match self.client.fetch(&subscription).await {
            Ok(FetchOutcome::Updated {
                text,
                etag,
                last_modified,
            }) => {
                let mut parsed = parse_payload(&text, &name, &subscription.url);
                parsed.profile.name = name.clone();

                subscription.etag = etag;
                subscription.last_modified = last_modified;
                subscription.last_fetch_ms = now_ms();
                subscription.last_error = None;

                record.profile = parsed.profile;
                record.subscription = Some(subscription);
                record.stored_at_ms = now_ms();
                self.store.save_record(&record)?;

                Ok(RefreshOutcome {
                    id: id.to_string(),
                    name,
                    status: RefreshStatus::Updated {
                        node_count: record.profile.nodes.len(),
                    },
                })
            }
            Ok(FetchOutcome::NotModified) => {
                subscription.last_fetch_ms = now_ms();
                subscription.last_error = None;
                record.subscription = Some(subscription);
                self.store.save_record(&record)?;

                Ok(RefreshOutcome {
                    id: id.to_string(),
                    name,
                    status: RefreshStatus::NotModified,
                })
            }
            Err(error) => {
                let message = error.to_string();
                subscription.last_error = Some(message.clone());
                // Advance the timestamp anyway: without it every launch would
                // hammer a provider that is already failing.
                subscription.last_fetch_ms = now_ms();
                record.subscription = Some(subscription);
                self.store.save_record(&record)?;

                Ok(RefreshOutcome {
                    id: id.to_string(),
                    name,
                    status: RefreshStatus::Failed { error: message },
                })
            }
        }
    }

    /// Refresh every subscription whose interval has elapsed.
    pub async fn refresh_due(&self) -> Result<Vec<RefreshOutcome>> {
        let now = now_ms();
        let due: Vec<StoredRecord> = self
            .store
            .records()?
            .into_iter()
            .filter(|record| {
                record
                    .subscription
                    .as_ref()
                    .map(|subscription| subscription.is_due(now))
                    .unwrap_or(false)
            })
            .collect();

        let mut outcomes = Vec::with_capacity(due.len());
        for record in due {
            outcomes.push(self.refresh(&record.id).await?);
        }
        Ok(outcomes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Layout;

    fn layout(name: &str) -> Layout {
        let root = std::env::temp_dir().join(format!(
            "triproxy-subsvc-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let layout = Layout::new(root);
        layout.ensure().unwrap();
        layout
    }

    /// A refresh against an unreachable host must record the error and leave the
    /// stored profile alone.
    #[tokio::test]
    async fn failed_refresh_keeps_the_existing_profile() {
        let layout = layout("failkeep");
        let store = ProfileStore::new(&layout);

        // Seed a record that looks like a subscription to a dead port.
        let text = "trojan://pw@a.example.com:443#A";
        let mut parsed = parse_payload(text, "dead", "https://example.invalid/sub");
        parsed.profile.name = "dead".to_string();
        let mut subscription = Subscription::new("http://127.0.0.1:1/sub", 3600);
        subscription.last_fetch_ms = 0;
        let saved = store
            .save_subscription(&parsed.profile, None, subscription)
            .unwrap();

        let service = SubscriptionService::new(&store).unwrap();
        let outcome = service.refresh(&saved.id).await.unwrap();

        match outcome.status {
            RefreshStatus::Failed { .. } => {}
            other => panic!("expected a failure, got {other:?}"),
        }

        // The profile is untouched and still has its node.
        let after = store.record(&saved.id).unwrap();
        assert_eq!(after.profile.nodes.len(), 1);
        assert_eq!(after.profile.name, "dead");
        // The error is recorded, and the timestamp advanced so we back off.
        let subscription = after.subscription.expect("subscription kept");
        assert!(subscription.last_error.is_some());
        assert!(subscription.last_fetch_ms > 0);
    }

    #[tokio::test]
    async fn refreshing_a_manual_profile_is_refused() {
        let layout = layout("manual");
        let store = ProfileStore::new(&layout);
        let parsed = parse_payload("trojan://pw@a.example.com:443#A", "manual", "pasted");
        let saved = store.save(&parsed.profile, None).unwrap();

        let service = SubscriptionService::new(&store).unwrap();
        let error = service.refresh(&saved.id).await.unwrap_err();
        assert!(error.to_string().contains("not a subscription"));
    }

    #[tokio::test]
    async fn refresh_due_skips_subscriptions_with_no_interval() {
        let layout = layout("nointerval");
        let store = ProfileStore::new(&layout);
        let parsed = parse_payload("trojan://pw@a.example.com:443#A", "x", "u");
        let subscription = Subscription::new("http://127.0.0.1:1/sub", 0);
        store
            .save_subscription(&parsed.profile, None, subscription)
            .unwrap();

        let service = SubscriptionService::new(&store).unwrap();
        assert!(service.refresh_due().await.unwrap().is_empty());
    }
}
