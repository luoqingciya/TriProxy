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

//! Subscription fetching.
//!
//! Only the network half lives here: fetch a URL, honour conditional requests,
//! return the body. Turning the body into a profile is the parser's job, and
//! deciding what to keep when a fetch fails is the store's — see
//! [`crate::subscription_service`].
//!
//! Conditional requests matter more than they look: providers change their node
//! list rarely, and re-downloading a megabyte every launch just to discard it is
//! both slow and rude to the provider. `ETag` and `Last-Modified` are stored with
//! the subscription and replayed on the next fetch.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Refuse anything larger than this. A subscription is a text list; a response
/// bigger than a few megabytes is a misconfigured URL, not a node list.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

const USER_AGENT: &str = concat!("TriProxy/", env!("CARGO_PKG_VERSION"));

/// Everything needed to refresh a subscription later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub url: String,
    /// Seconds between automatic refreshes. `0` disables automatic refresh.
    #[serde(default)]
    pub interval_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub last_fetch_ms: u64,
    /// Why the last attempt failed, cleared on the next success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Subscription {
    pub fn new(url: impl Into<String>, interval_secs: u64) -> Self {
        Self {
            url: url.into(),
            interval_secs,
            etag: None,
            last_modified: None,
            last_fetch_ms: 0,
            last_error: None,
        }
    }

    /// Whether the subscription is due for an automatic refresh.
    pub fn is_due(&self, now_ms: u64) -> bool {
        if self.interval_secs == 0 {
            return false;
        }
        let interval_ms = self.interval_secs.saturating_mul(1000);
        now_ms.saturating_sub(self.last_fetch_ms) >= interval_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// The provider returned a body; the caller should re-parse it.
    Updated {
        text: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
    /// The provider confirmed the cached copy is current (HTTP 304).
    NotModified,
}

pub struct SubscriptionClient {
    http: reqwest::Client,
}

impl SubscriptionClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            // Providers commonly serve subscriptions from CDNs with redirects.
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|error| Error::Other(format!("cannot build HTTP client: {error}")))?;
        Ok(Self { http })
    }

    /// Fetch a subscription, replaying the cached validators.
    pub async fn fetch(&self, subscription: &Subscription) -> Result<FetchOutcome> {
        let mut request = self.http.get(&subscription.url);

        if let Some(etag) = &subscription.etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = &subscription.last_modified {
            request = request.header(reqwest::header::IF_MODIFIED_SINCE, last_modified);
        }

        let response = request
            .send()
            .await
            .map_err(|error| Error::Other(format!("cannot reach {}: {error}", subscription.url)))?;

        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(FetchOutcome::NotModified);
        }

        let status = response.status();
        if !status.is_success() {
            return Err(Error::Other(format!(
                "{} returned HTTP {status}",
                subscription.url
            )));
        }

        if let Some(length) = response.content_length() {
            if length as usize > MAX_BODY_BYTES {
                return Err(Error::Other(format!(
                    "{} returned {length} bytes, over the {} byte limit",
                    subscription.url, MAX_BODY_BYTES
                )));
            }
        }

        let etag = header_string(&response, reqwest::header::ETAG);
        let last_modified = header_string(&response, reqwest::header::LAST_MODIFIED);

        let bytes = response
            .bytes()
            .await
            .map_err(|error| Error::Other(format!("cannot read the response body: {error}")))?;

        if bytes.len() > MAX_BODY_BYTES {
            return Err(Error::Other(format!(
                "{} returned {} bytes, over the {} byte limit",
                subscription.url,
                bytes.len(),
                MAX_BODY_BYTES
            )));
        }

        let text = String::from_utf8(bytes.to_vec())
            .map_err(|_| Error::Other("subscription body is not valid UTF-8".to_string()))?;

        Ok(FetchOutcome::Updated {
            text,
            etag,
            last_modified,
        })
    }
}

fn header_string(
    response: &reqwest::Response,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_calculation_respects_the_interval() {
        let mut subscription = Subscription::new("https://example.com/sub", 3600);
        subscription.last_fetch_ms = 1_000_000;

        assert!(!subscription.is_due(1_000_000));
        assert!(!subscription.is_due(1_000_000 + 3_599_000));
        assert!(subscription.is_due(1_000_000 + 3_600_000));
    }

    #[test]
    fn zero_interval_never_becomes_due() {
        let mut subscription = Subscription::new("https://example.com/sub", 0);
        subscription.last_fetch_ms = 0;
        assert!(!subscription.is_due(u64::MAX));
    }

    #[test]
    fn subscription_roundtrips_through_json() {
        let mut subscription = Subscription::new("https://example.com/sub", 7200);
        subscription.etag = Some("\"abc\"".to_string());
        subscription.last_modified = Some("Wed, 21 Oct 2015 07:28:00 GMT".to_string());
        subscription.last_fetch_ms = 123;
        subscription.last_error = Some("timeout".to_string());

        let text = serde_json::to_string(&subscription).unwrap();
        let back: Subscription = serde_json::from_str(&text).unwrap();
        assert_eq!(subscription, back);
    }
}
