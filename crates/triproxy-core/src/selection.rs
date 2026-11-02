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

//! Picking which node the proxy uses.
//!
//! Two things have to happen and they are not the same:
//!
//! 1. **Persist the choice**, so the next start uses it. The compiler moves the
//!    selected node to the front of the outbound list, and every core treats the
//!    first outbound as the default.
//! 2. **Apply it now**, when a core is already running. Mihomo and sing-box
//!    expose a `select` group through their controller API, so switching is one
//!    request instead of a config rewrite and restart.
//!
//! Xray has no controller API, so on Xray the second step is impossible: the
//! choice is saved and takes effect on the next start. That difference is
//! reported to the caller rather than hidden, because "I clicked the node and
//! nothing changed" is exactly the kind of thing users cannot diagnose.

use crate::compiler::{AUTO_GROUP, PROXY_GROUP};
use crate::controller::ControllerClient;
use crate::error::{Error, Result};
use crate::store::ProfileStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Saved and applied to the running core.
    Applied { node: String },
    /// Saved; takes effect the next time the core starts.
    SavedOnly {
        node: Option<String>,
        reason: String,
    },
}

/// Select a node for the active profile.
///
/// `node_id` of `None` clears the selection and falls back to the profile order.
pub async fn select_node(
    store: &ProfileStore,
    client: Option<&ControllerClient>,
    node_id: Option<&str>,
) -> Result<ApplyOutcome> {
    let Some((stored, profile)) = store.active()? else {
        return Err(Error::Other(
            "no active profile; choose one with `profile use` first".to_string(),
        ));
    };

    let node_name = match node_id {
        Some(id) => {
            let node = profile
                .nodes
                .iter()
                .find(|node| node.id == id)
                .ok_or_else(|| Error::Other(format!("no node with id `{id}` in this profile")))?;
            Some(node.name.clone())
        }
        None => None,
    };

    // Persist first: even if the running core refuses the switch, the choice is
    // still what the user asked for and should survive a restart.
    store.set_selected_node(&stored.id, node_id)?;

    let Some(node_name) = node_name else {
        // Clearing the pick means "use the automatic group", not "keep doing
        // whatever the core is doing". A running core has to be told, exactly
        // like it is told about an explicit node.
        let Some(client) = client else {
            return Ok(ApplyOutcome::SavedOnly {
                node: None,
                reason: "selection cleared; takes effect on the next start".to_string(),
            });
        };
        return match client.select_proxy(PROXY_GROUP, AUTO_GROUP).await {
            Ok(()) => Ok(ApplyOutcome::Applied {
                node: AUTO_GROUP.to_string(),
            }),
            Err(error) => Ok(ApplyOutcome::SavedOnly {
                node: None,
                reason: error.to_string(),
            }),
        };
    };

    let Some(client) = client else {
        return Ok(ApplyOutcome::SavedOnly {
            node: Some(node_name),
            reason: "no running core with a controller API".to_string(),
        });
    };

    match client.select_proxy(PROXY_GROUP, &node_name).await {
        Ok(()) => Ok(ApplyOutcome::Applied { node: node_name }),
        // Keep the core error verbatim: "nothing changed" without the reason is
        // exactly what users cannot act on.
        Err(error) => Ok(ApplyOutcome::SavedOnly {
            node: Some(node_name),
            reason: error.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Protocol;
    use crate::paths::Layout;
    use crate::profile::{Credentials, Node, Profile, Transport};

    fn layout(name: &str) -> Layout {
        let root = std::env::temp_dir().join(format!(
            "triproxy-select-{name}-{}-{}",
            std::process::id(),
            crate::logging::now_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let layout = Layout::new(root);
        layout.ensure().unwrap();
        layout
    }

    fn node(name: &str) -> Node {
        let mut node = Node {
            id: String::new(),
            name: name.to_string(),
            protocol: Protocol::Trojan,
            server: format!("{name}.example.com"),
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

    #[tokio::test]
    async fn selecting_without_a_core_saves_the_choice() {
        let layout = layout("saved");
        let store = ProfileStore::new(&layout);
        let profile = Profile::new("p", vec![node("a"), node("b")]);
        let stored = store.save_activating_if_first(&profile, None).unwrap();
        let target = profile.nodes[1].id.clone();

        let outcome = select_node(&store, None, Some(&target)).await.unwrap();
        assert!(matches!(outcome, ApplyOutcome::SavedOnly { .. }));

        // And it survives a reload.
        let record = store.record(&stored.id).unwrap();
        assert_eq!(record.selected_node.as_deref(), Some(target.as_str()));
    }

    #[tokio::test]
    async fn selecting_an_unknown_node_is_refused() {
        let layout = layout("unknown");
        let store = ProfileStore::new(&layout);
        store
            .save_activating_if_first(&Profile::new("p", vec![node("a")]), None)
            .unwrap();

        let error = select_node(&store, None, Some("nope")).await.unwrap_err();
        assert!(error.to_string().contains("no node with id"));
    }

    #[tokio::test]
    async fn selecting_without_an_active_profile_is_refused() {
        let layout = layout("noactive");
        let store = ProfileStore::new(&layout);
        let error = select_node(&store, None, Some("x")).await.unwrap_err();
        assert!(error.to_string().contains("no active profile"));
    }
}
