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

//! Profile persistence.
//!
//! Storage v1 is one JSON file per profile plus a small index, not SQLite.
//! Rationale: profiles are read as a whole and written as a whole, the count is
//! in the dozens, and a directory of readable files is far easier to debug and to
//! back up than a database. SQLite becomes worthwhile when there is data that is
//! actually queried (connection history, latency samples), and that is a later
//! phase — see `docs/ARCHITECTURE.md`.
//!
//! Two different identifiers exist on purpose:
//!
//! - [`Profile::id`] is a **content fingerprint**: re-importing an unchanged
//!   subscription produces the same value, which is what makes de-duplication
//!   possible.
//! - [`StoredProfile::id`] is the **record identity**: it survives node updates,
//!   so a subscription can be refreshed without losing its place in the UI or
//!   its "active" flag.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::logging::now_ms;
use crate::paths::Layout;
use crate::profile::{InputFormat, Profile};
use crate::subscription::Subscription;

const INDEX_FILE: &str = "index.json";

/// Summary shown in listings; the full profile lives in its own file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredProfile {
    /// Record identity, stable across content updates.
    pub id: String,
    pub name: String,
    pub source: String,
    pub format: InputFormat,
    pub node_count: usize,
    /// Content fingerprint of the stored profile (see [`Profile::id`]).
    pub content_id: String,
    pub updated_at_ms: u64,
    /// Present when the profile came from a URL and can be refreshed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription: Option<Subscription>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    active: Option<String>,
}

/// On-disk wrapper around a profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    id: String,
    stored_at_ms: u64,
    profile: Profile,
    /// Refresh state, present for URL-sourced profiles. Optional so records
    /// written before subscriptions existed still load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subscription: Option<Subscription>,
}

/// A record as stored on disk, including refresh state.
#[derive(Debug, Clone)]
pub struct StoredRecord {
    pub id: String,
    pub stored_at_ms: u64,
    pub profile: Profile,
    pub subscription: Option<Subscription>,
}

pub struct ProfileStore {
    root: PathBuf,
}

impl ProfileStore {
    pub fn new(layout: &Layout) -> Self {
        Self {
            root: layout.profiles_dir(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn ensure(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root).map_err(|e| Error::io(&self.root, e))
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_FILE)
    }

    /// All stored profiles, newest first.
    pub fn list(&self) -> Result<Vec<StoredProfile>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(error) => return Err(Error::io(&self.root, error)),
        };

        for entry in entries {
            let entry = entry.map_err(|e| Error::io(&self.root, e))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some(INDEX_FILE) {
                continue;
            }
            // A corrupt file must not hide the rest of the library.
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<Record>(&text) else {
                tracing::warn!(path = %path.display(), "skipping unreadable profile file");
                continue;
            };
            out.push(summarise(&record));
        }

        out.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then(a.name.cmp(&b.name))
        });
        Ok(out)
    }

    pub fn load(&self, id: &str) -> Result<Profile> {
        let path = self.record_path(id);
        let text = std::fs::read_to_string(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::Other(format!("no profile with id `{id}`"))
            } else {
                Error::io(&path, error)
            }
        })?;
        let record: Record = serde_json::from_str(&text)?;
        Ok(record.profile)
    }

    pub fn find(&self, id: &str) -> Result<Option<StoredProfile>> {
        let path = self.record_path(id);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(None);
        };
        let record: Record = serde_json::from_str(&text)?;
        Ok(Some(summarise(&record)))
    }

    pub fn find_by_name(&self, name: &str) -> Result<Option<StoredProfile>> {
        Ok(self
            .list()?
            .into_iter()
            .find(|stored| stored.name.eq_ignore_ascii_case(name)))
    }

    /// Store a profile.
    ///
    /// Passing an existing `id` updates that record (a subscription refresh);
    /// omitting it creates a new record whose id is derived from the name and
    /// the moment of first import.
    ///
    /// An existing subscription is preserved: refreshing a URL profile must not
    /// silently turn it into a manual one.
    pub fn save(&self, profile: &Profile, id: Option<&str>) -> Result<StoredProfile> {
        self.save_inner(profile, id, None)
    }

    /// Store a profile together with the subscription it came from.
    pub fn save_subscription(
        &self,
        profile: &Profile,
        id: Option<&str>,
        subscription: Subscription,
    ) -> Result<StoredProfile> {
        self.save_inner(profile, id, Some(subscription))
    }

    fn save_inner(
        &self,
        profile: &Profile,
        id: Option<&str>,
        subscription: Option<Subscription>,
    ) -> Result<StoredProfile> {
        self.ensure()?;

        let id = match id {
            Some(id) => id.to_string(),
            None => new_record_id(&profile.name),
        };

        let subscription = match subscription {
            Some(subscription) => Some(subscription),
            None => self
                .record(&id)
                .ok()
                .and_then(|existing| existing.subscription),
        };

        let record = Record {
            id: id.clone(),
            stored_at_ms: now_ms(),
            profile: profile.clone(),
            subscription,
        };
        let text = serde_json::to_string_pretty(&record)?;
        write_atomic(&self.record_path(&id), &text)?;

        Ok(summarise(&record))
    }

    /// Full record, including refresh state.
    pub fn record(&self, id: &str) -> Result<StoredRecord> {
        let path = self.record_path(id);
        let text = std::fs::read_to_string(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::Other(format!("no profile with id `{id}`"))
            } else {
                Error::io(&path, error)
            }
        })?;
        let record: Record = serde_json::from_str(&text)?;
        Ok(StoredRecord {
            id: record.id,
            stored_at_ms: record.stored_at_ms,
            profile: record.profile,
            subscription: record.subscription,
        })
    }

    /// Every readable record, including refresh state.
    pub fn records(&self) -> Result<Vec<StoredRecord>> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(Error::io(&self.root, error)),
        };

        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some(INDEX_FILE) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<Record>(&text) else {
                continue;
            };
            out.push(StoredRecord {
                id: record.id,
                stored_at_ms: record.stored_at_ms,
                profile: record.profile,
                subscription: record.subscription,
            });
        }

        out.sort_by(|a, b| b.stored_at_ms.cmp(&a.stored_at_ms).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    /// Persist a record verbatim (used after a refresh attempt updates the
    /// subscription state without changing the profile).
    pub fn save_record(&self, record: &StoredRecord) -> Result<StoredProfile> {
        self.ensure()?;
        let stored = Record {
            id: record.id.clone(),
            stored_at_ms: record.stored_at_ms,
            profile: record.profile.clone(),
            subscription: record.subscription.clone(),
        };
        let text = serde_json::to_string_pretty(&stored)?;
        write_atomic(&self.record_path(&record.id), &text)?;
        Ok(summarise(&stored))
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.record_path(id);
        std::fs::remove_file(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::Other(format!("no profile with id `{id}`"))
            } else {
                Error::io(&path, error)
            }
        })?;

        if self.active_id()?.as_deref() == Some(id) {
            self.set_active(None)?;
        }
        Ok(())
    }

    pub fn active_id(&self) -> Result<Option<String>> {
        Ok(self.read_index()?.active)
    }

    /// Mark a profile active, or clear the flag with `None`.
    pub fn set_active(&self, id: Option<&str>) -> Result<()> {
        self.ensure()?;
        if let Some(id) = id {
            if self.find(id)?.is_none() {
                return Err(Error::Other(format!("no profile with id `{id}`")));
            }
        }
        let index = Index {
            active: id.map(str::to_string),
        };
        let text = serde_json::to_string_pretty(&index)?;
        write_atomic(&self.index_path(), &text)?;
        Ok(())
    }

    /// The active profile, if one is set and still readable.
    pub fn active(&self) -> Result<Option<(StoredProfile, Profile)>> {
        let Some(id) = self.active_id()? else {
            return Ok(None);
        };
        let Some(summary) = self.find(&id)? else {
            // The record disappeared behind our back; clear the dangling flag.
            self.set_active(None)?;
            return Ok(None);
        };
        let profile = self.load(&id)?;
        Ok(Some((summary, profile)))
    }

    fn read_index(&self) -> Result<Index> {
        let path = self.index_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(serde_json::from_str(&text).unwrap_or_default()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Index::default()),
            Err(error) => Err(Error::io(&path, error)),
        }
    }
}

fn summarise(record: &Record) -> StoredProfile {
    StoredProfile {
        id: record.id.clone(),
        name: record.profile.name.clone(),
        source: record.profile.metadata.source.clone(),
        format: record.profile.metadata.format,
        node_count: record.profile.nodes.len(),
        content_id: record.profile.id.clone(),
        updated_at_ms: record.stored_at_ms,
        subscription: record.subscription.clone(),
    }
}

/// Record ids only need to be unique and stable, not meaningful.
fn new_record_id(name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update(b"\x1f");
    hasher.update(now_ms().to_string().as_bytes());
    hex::encode(hasher.finalize())[..12].to_string()
}

/// Write via a temporary file so a crash cannot leave a half-written profile.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let staging = path.with_extension("tmp");
    std::fs::write(&staging, text).map_err(|e| Error::io(&staging, e))?;
    std::fs::rename(&staging, path).map_err(|e| Error::io(path, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Protocol;
    use crate::profile::{Credentials, Node, Transport};

    fn layout(name: &str) -> Layout {
        let root = std::env::temp_dir().join(format!(
            "triproxy-store-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let layout = Layout::new(root);
        layout.ensure().unwrap();
        layout
    }

    fn node(server: &str) -> Node {
        let mut node = Node {
            id: String::new(),
            name: format!("n-{server}"),
            protocol: Protocol::Trojan,
            server: server.to_string(),
            port: 443,
            credentials: Credentials::Trojan {
                password: "pw".to_string(),
            },
            transport: Transport::Tcp,
            tls: None,
            udp: true,
            plugin: None,
        };
        node.refresh_id();
        node
    }

    fn profile(name: &str, servers: &[&str]) -> Profile {
        Profile::new(name, servers.iter().map(|s| node(s)).collect())
    }

    #[test]
    fn save_and_load_roundtrip() {
        let layout = layout("roundtrip");
        let store = ProfileStore::new(&layout);
        let saved = store
            .save(&profile("sub", &["a.example.com"]), None)
            .unwrap();

        let loaded = store.load(&saved.id).unwrap();
        assert_eq!(loaded.name, "sub");
        assert_eq!(loaded.nodes.len(), 1);
        assert_eq!(saved.node_count, 1);
        assert_eq!(saved.content_id, loaded.id);
    }

    #[test]
    fn list_is_newest_first_and_skips_the_index() {
        let layout = layout("list");
        let store = ProfileStore::new(&layout);
        let first = store
            .save(&profile("one", &["a.example.com"]), None)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = store
            .save(&profile("two", &["b.example.com"]), None)
            .unwrap();

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, second.id);
        assert_eq!(listed[1].id, first.id);
    }

    #[test]
    fn updating_a_record_keeps_its_id_across_content_changes() {
        let layout = layout("update");
        let store = ProfileStore::new(&layout);
        let saved = store
            .save(&profile("sub", &["a.example.com"]), None)
            .unwrap();
        let before = store.load(&saved.id).unwrap();

        // A subscription refresh changes the nodes but must not change the record.
        let refreshed = profile("sub", &["a.example.com", "c.example.com"]);
        let updated = store.save(&refreshed, Some(&saved.id)).unwrap();

        assert_eq!(updated.id, saved.id);
        assert_eq!(updated.node_count, 2);
        assert_ne!(updated.content_id, before.id);
    }

    #[test]
    fn find_by_name_is_case_insensitive() {
        let layout = layout("byname");
        let store = ProfileStore::new(&layout);
        let saved = store
            .save(&profile("MySub", &["a.example.com"]), None)
            .unwrap();
        assert_eq!(
            store.find_by_name("mysub").unwrap().map(|s| s.id),
            Some(saved.id)
        );
        assert!(store.find_by_name("other").unwrap().is_none());
    }

    #[test]
    fn active_profile_roundtrips_and_delete_clears_it() {
        let layout = layout("active");
        let store = ProfileStore::new(&layout);
        let saved = store
            .save(&profile("sub", &["a.example.com"]), None)
            .unwrap();

        store.set_active(Some(&saved.id)).unwrap();
        assert_eq!(
            store.active_id().unwrap().as_deref(),
            Some(saved.id.as_str())
        );
        let (summary, profile) = store.active().unwrap().unwrap();
        assert_eq!(summary.id, saved.id);
        assert_eq!(profile.name, "sub");

        store.delete(&saved.id).unwrap();
        assert!(store.active_id().unwrap().is_none());
        assert!(store.active().unwrap().is_none());
    }

    #[test]
    fn activating_an_unknown_id_is_refused() {
        let layout = layout("badactive");
        let store = ProfileStore::new(&layout);
        let error = store.set_active(Some("nope")).unwrap_err();
        assert!(error.to_string().contains("no profile with id"));
    }

    #[test]
    fn deleting_an_unknown_id_is_refused() {
        let layout = layout("baddelete");
        let store = ProfileStore::new(&layout);
        let error = store.delete("nope").unwrap_err();
        assert!(error.to_string().contains("no profile with id"));
    }

    #[test]
    fn empty_store_lists_nothing_and_has_no_active_profile() {
        let layout = layout("empty");
        let store = ProfileStore::new(&layout);
        assert!(store.list().unwrap().is_empty());
        assert!(store.active().unwrap().is_none());
    }

    #[test]
    fn corrupt_files_are_skipped_instead_of_failing_the_listing() {
        let layout = layout("corrupt");
        let store = ProfileStore::new(&layout);
        let saved = store
            .save(&profile("good", &["a.example.com"]), None)
            .unwrap();
        std::fs::write(store.root().join("broken.json"), "{ not json").unwrap();

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, saved.id);
    }

    #[test]
    fn a_dangling_active_flag_is_cleared() {
        let layout = layout("dangling");
        let store = ProfileStore::new(&layout);
        let saved = store
            .save(&profile("sub", &["a.example.com"]), None)
            .unwrap();
        store.set_active(Some(&saved.id)).unwrap();
        // Remove the record behind the store's back.
        std::fs::remove_file(store.root().join(format!("{}.json", saved.id))).unwrap();

        assert!(store.active().unwrap().is_none());
        assert!(store.active_id().unwrap().is_none());
    }
}
