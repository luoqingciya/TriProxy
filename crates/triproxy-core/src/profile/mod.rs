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

//! Unified Profile — the format-independent model every core is compiled from.
//!
//! ```text
//! input (URI list / base64 subscription)
//!     ↓  parse
//! Unified Profile   ← this module
//!     ↓  compile (Phase 3)
//! mihomo.yaml / sing-box.json / xray.json
//! ```
//!
//! Scope note: `rules`, `dns` and `network` from the project plan are **not**
//! modelled yet. They only become meaningful once the compilers that consume
//! them exist (Phase 3), and inventing a rule model before its two mappings
//! (Clash rules vs sing-box route rules) are known would guarantee a rewrite.

pub mod clash;
pub mod encoding;
pub mod node;
pub mod parse;
pub mod uri;
pub mod validate;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use node::{
    Credentials, GrpcOptions, Http2Options, Node, NodeId, RealityConfig, TlsConfig, Transport,
    VlessFlow, WsOptions,
};
pub use parse::{parse_payload, ParsedProfile};
pub use uri::{parse_uri, ParseIssue};
pub use validate::{count, has_errors, validate_node, validate_profile, Diagnostic, Severity};

use crate::capability::{ProfileRequirement, Protocol};

/// A parsed, core-independent profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Content-derived identity: two imports of the same subscription share it.
    pub id: String,
    pub name: String,
    pub metadata: ProfileMetadata,
    pub nodes: Vec<Node>,
    /// Proxy groups. Always empty for URI lists; filled in once Clash YAML
    /// import lands (Phase 3).
    #[serde(default)]
    pub groups: Vec<ProxyGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMetadata {
    /// Where the profile came from: a file path, `stdin`, or a URL later.
    pub source: String,
    pub format: InputFormat,
    pub node_count: usize,
    pub parsed_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputFormat {
    /// Newline-separated share links.
    UriList,
    /// A base64 blob wrapping a URI list — what most providers hand out.
    Base64Subscription,
    /// A Clash / Clash.Meta configuration file.
    ClashYaml,
}

impl InputFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            InputFormat::UriList => "uri-list",
            InputFormat::Base64Subscription => "base64-subscription",
            InputFormat::ClashYaml => "clash-yaml",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyGroup {
    pub name: String,
    pub kind: GroupKind,
    /// Node names or other group names, resolved at compile time.
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    Select,
    UrlTest,
    Fallback,
    LoadBalance,
}

impl Profile {
    pub fn new(name: impl Into<String>, nodes: Vec<Node>) -> Self {
        let name = name.into();
        let id = compute_profile_id(&nodes);
        let node_count = nodes.len();
        Self {
            id,
            name,
            metadata: ProfileMetadata {
                source: String::new(),
                format: InputFormat::UriList,
                node_count,
                parsed_at_ms: crate::logging::now_ms(),
            },
            nodes,
            groups: Vec::new(),
        }
    }

    pub fn with_metadata(mut self, source: impl Into<String>, format: InputFormat) -> Self {
        self.metadata.source = source.into();
        self.metadata.format = format;
        self.metadata.node_count = self.nodes.len();
        self
    }

    /// Recompute identity after nodes were added or removed.
    pub fn refresh_id(&mut self) {
        self.id = compute_profile_id(&self.nodes);
        self.metadata.node_count = self.nodes.len();
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn protocols(&self) -> Vec<Protocol> {
        let mut seen: Vec<Protocol> = Vec::new();
        for node in &self.nodes {
            if !seen.contains(&node.protocol) {
                seen.push(node.protocol);
            }
        }
        seen
    }

    /// What this profile demands from a core.
    ///
    /// This is the input of the Smart Core Selector (Phase 4). Only requirements
    /// visible from the nodes are reported: TUN, DNS and rule-set needs come from
    /// the network/rule configuration, which arrives in Phase 3.
    pub fn requirement(&self) -> ProfileRequirement {
        ProfileRequirement {
            protocols: self.protocols(),
            requires_tun: false,
            requires_dns: false,
            requires_reality: self.nodes.iter().any(Node::uses_reality),
            requires_vision: self.nodes.iter().any(Node::uses_vision),
            requires_wireguard: self
                .nodes
                .iter()
                .any(|node| node.protocol == Protocol::WireGuard),
            requires_rule_set: false,
            requires_proxy_groups: false,
        }
    }
}

/// Identity from node identities only, so re-importing an unchanged
/// subscription yields the same id.
fn compute_profile_id(nodes: &[Node]) -> String {
    let mut ids: Vec<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    ids.sort_unstable();
    let mut hasher = Sha256::new();
    for id in ids {
        hasher.update(id.as_bytes());
        hasher.update(b"\x1f");
    }
    hex::encode(hasher.finalize())[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trojan(server: &str, name: &str) -> Node {
        let mut node = Node {
            id: String::new(),
            name: name.to_string(),
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

    #[test]
    fn profile_id_is_stable_and_order_independent() {
        let a = Profile::new("p", vec![trojan("a.com", "a"), trojan("b.com", "b")]);
        let b = Profile::new(
            "other name",
            vec![trojan("b.com", "b"), trojan("a.com", "a")],
        );
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn profile_id_changes_with_content() {
        let a = Profile::new("p", vec![trojan("a.com", "a")]);
        let b = Profile::new("p", vec![trojan("a.com", "a"), trojan("c.com", "c")]);
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn requirement_collects_protocols_without_duplicates() {
        let profile = Profile::new("p", vec![trojan("a.com", "a"), trojan("b.com", "b")]);
        assert_eq!(profile.protocols(), vec![Protocol::Trojan]);
    }

    #[test]
    fn requirement_flags_reality_and_vision() {
        let mut node = trojan("a.com", "a");
        node.protocol = Protocol::Vless;
        node.credentials = Credentials::Vless {
            uuid: "11111111-2222-3333-4444-555555555555".to_string(),
            flow: VlessFlow::Vision,
        };
        node.tls = Some(TlsConfig {
            reality: Some(RealityConfig {
                public_key: "k".repeat(43),
                short_id: String::new(),
                spider_x: None,
            }),
            ..Default::default()
        });
        let profile = Profile::new("p", vec![node]);
        let requirement = profile.requirement();
        assert!(requirement.requires_reality);
        assert!(requirement.requires_vision);
        assert!(!requirement.requires_tun);
    }

    #[test]
    fn requirement_rejects_xray_when_tun_and_reality_are_needed() {
        let mut node = trojan("a.com", "a");
        node.protocol = Protocol::Vless;
        node.tls = Some(TlsConfig {
            reality: Some(RealityConfig {
                public_key: "k".repeat(43),
                ..Default::default()
            }),
            ..Default::default()
        });
        let profile = Profile::new("p", vec![node]);
        let mut requirement = profile.requirement();
        assert!(requirement.requires_reality);
        // TUN is a network-mode requirement, so simulate it here.
        requirement.requires_tun = true;

        let xray = crate::capability::CoreCapability::for_core(crate::core::CoreKind::Xray);
        assert_eq!(requirement.unmet_by(&xray), vec!["TUN".to_string()]);

        let singbox = crate::capability::CoreCapability::for_core(crate::core::CoreKind::SingBox);
        assert!(requirement.unmet_by(&singbox).is_empty());
    }
}
