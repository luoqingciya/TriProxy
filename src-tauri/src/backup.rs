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

//! Backup and restore.
//!
//! One JSON file with everything the user configured: settings, every profile,
//! and which one was active. Not the core binaries or the logs — those are
//! either re-downloadable or worthless.
//!
//! Restore **merges**: a backup brought from another machine must not delete the
//! profiles already here. Ids collide only when it is the same profile, and then
//! the backup wins, which is what "restore" means.

use serde::{Deserialize, Serialize};

use triproxy_core::{AppSettings, ProfileStore};

/// Bumped only when the file layout changes in a way older code cannot read.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct Backup {
    pub version: u32,
    /// Informational: a backup from a newer build still restores if the format
    /// version matches, because the record documents are self-describing.
    pub app_version: String,
    pub exported_at_ms: u64,
    pub settings: AppSettings,
    pub records: Vec<serde_json::Value>,
    #[serde(default)]
    pub active: Option<String>,
}

impl Backup {
    pub fn capture(store: &ProfileStore, settings: AppSettings) -> triproxy_core::Result<Self> {
        Ok(Self {
            version: FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            exported_at_ms: now_ms(),
            settings,
            records: store.export_records()?,
            active: store.active_id()?,
        })
    }

    /// Parse and check a backup before touching anything on disk.
    ///
    /// A wrong file picked by mistake should produce one clear error, not a
    /// half-restored library.
    pub fn parse(text: &str) -> Result<Self, String> {
        let backup: Backup =
            serde_json::from_str(text).map_err(|error| format!("不是有效的 TriProxy 备份：{error}"))?;
        if backup.version != FORMAT_VERSION {
            return Err(format!(
                "备份格式版本 {} 不受支持（当前支持 {FORMAT_VERSION}）",
                backup.version
            ));
        }
        Ok(backup)
    }

    /// Write everything back. Returns how many profiles were restored.
    pub fn restore(&self, store: &ProfileStore, layout: &triproxy_core::Layout) -> Result<usize, String> {
        let count = store
            .restore_records(&self.records)
            .map_err(|error| error.to_string())?;

        self.settings
            .save(layout)
            .map_err(|error| format!("设置写入失败：{error}"))?;

        // Only point at the restored profile when it actually arrived; a dangling
        // active id would be cleared on the next read anyway, but silently.
        if let Some(active) = self.active.as_deref() {
            if store.find(active).ok().flatten().is_some() {
                let _ = store.set_active(Some(active));
            }
        }

        Ok(count)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use triproxy_core::Layout;

    fn layout(name: &str) -> Layout {
        let root = std::env::temp_dir().join(format!(
            "triproxy-backup-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let layout = Layout::new(root);
        layout.ensure().unwrap();
        layout
    }

    #[test]
    fn a_backup_round_trips_through_another_data_directory() {
        let source = layout("source");
        let store = ProfileStore::new(&source);
        let profile = triproxy_core::profile::Profile::new("sub", Vec::new());
        let saved = store.save(&profile, None).unwrap();
        store.set_active(Some(&saved.id)).unwrap();

        let backup = Backup::capture(&store, AppSettings::default()).unwrap();
        let text = serde_json::to_string(&backup).unwrap();

        let target = layout("target");
        let target_store = ProfileStore::new(&target);
        let parsed = Backup::parse(&text).unwrap();
        assert_eq!(parsed.restore(&target_store, &target).unwrap(), 1);
        assert_eq!(target_store.active_id().unwrap(), Some(saved.id));
    }

    /// Importing onto a machine that already has profiles must not wipe them.
    #[test]
    fn restore_merges_instead_of_replacing() {
        let source = layout("merge-source");
        let store = ProfileStore::new(&source);
        store
            .save(&triproxy_core::profile::Profile::new("from-backup", Vec::new()), None)
            .unwrap();
        let text = serde_json::to_string(&Backup::capture(&store, AppSettings::default()).unwrap()).unwrap();

        let target = layout("merge-target");
        let target_store = ProfileStore::new(&target);
        target_store
            .save(&triproxy_core::profile::Profile::new("already-here", Vec::new()), None)
            .unwrap();

        Backup::parse(&text).unwrap().restore(&target_store, &target).unwrap();
        let names: Vec<String> = target_store
            .list()
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert!(names.contains(&"already-here".to_string()), "{names:?}");
        assert!(names.contains(&"from-backup".to_string()), "{names:?}");
    }

    #[test]
    fn refuses_a_file_that_is_not_a_backup() {
        let error = Backup::parse(r#"{"hello":"world"}"#).unwrap_err();
        assert!(error.contains("不是有效的 TriProxy 备份"), "{error}");
    }

    #[test]
    fn refuses_an_unknown_format_version() {
        let text = format!(
            r#"{{"version":99,"app_version":"x","exported_at_ms":0,"settings":{},"records":[]}}"#,
            serde_json::to_string(&AppSettings::default()).unwrap()
        );
        let error = Backup::parse(&text).unwrap_err();
        assert!(error.contains("99"), "{error}");
    }
}
