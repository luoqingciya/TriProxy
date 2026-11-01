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

//! Validation of parsed nodes and profiles.
//!
//! Validation never mutates and never throws: it returns diagnostics so the UI
//! can show "42 nodes imported, 3 problems" instead of refusing the whole
//! subscription. Errors mark nodes that cannot work; warnings mark nodes that
//! will connect but are probably not what the user wants.

use serde::{Deserialize, Serialize};

use super::node::{Credentials, Node, Transport, VlessFlow};
use super::Profile;
use crate::capability::Protocol;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Index into `Profile::nodes`, when the diagnostic is about one node.
    pub node: Option<usize>,
    pub node_name: Option<String>,
    /// Stable machine-readable code (`node.empty_server`, ...).
    pub code: String,
    pub message: String,
}

impl Diagnostic {
    fn new(severity: Severity, code: &str, message: impl Into<String>) -> Self {
        Self {
            severity,
            node: None,
            node_name: None,
            code: code.to_string(),
            message: message.into(),
        }
    }

    fn for_node(
        severity: Severity,
        index: usize,
        node: &Node,
        code: &str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            node: Some(index),
            node_name: Some(node.name.clone()),
            code: code.to_string(),
            message: message.into(),
        }
    }
}

/// ShadowSocks ciphers understood by all three supported cores.
const KNOWN_SS_METHODS: [&str; 11] = [
    "aes-128-gcm",
    "aes-192-gcm",
    "aes-256-gcm",
    "chacha20-ietf-poly1305",
    "xchacha20-ietf-poly1305",
    "2022-blake3-aes-128-gcm",
    "2022-blake3-aes-256-gcm",
    "2022-blake3-chacha20-poly1305",
    "rc4-md5",
    "none",
    "plain",
];

/// Validate a single node. `index` is only used to label diagnostics.
pub fn validate_node(node: &Node, index: usize) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    if node.protocol != node.credentials.protocol() {
        out.push(Diagnostic::for_node(
            Severity::Error,
            index,
            node,
            "node.protocol_mismatch",
            format!(
                "declared protocol `{}` does not match credentials `{}`",
                node.protocol,
                node.credentials.protocol()
            ),
        ));
    }

    if node.server.trim().is_empty() {
        out.push(Diagnostic::for_node(
            Severity::Error,
            index,
            node,
            "node.empty_server",
            "server address is empty",
        ));
    } else if node.server.contains(char::is_whitespace) {
        out.push(Diagnostic::for_node(
            Severity::Error,
            index,
            node,
            "node.invalid_server",
            format!("server `{}` contains whitespace", node.server),
        ));
    }

    if node.port == 0 {
        out.push(Diagnostic::for_node(
            Severity::Error,
            index,
            node,
            "node.invalid_port",
            "port is 0",
        ));
    }

    match &node.credentials {
        Credentials::Shadowsocks { method, password } => {
            if method.trim().is_empty() {
                out.push(Diagnostic::for_node(
                    Severity::Error,
                    index,
                    node,
                    "ss.empty_method",
                    "shadowsocks cipher is empty",
                ));
            } else if !KNOWN_SS_METHODS.contains(&method.to_ascii_lowercase().as_str()) {
                out.push(Diagnostic::for_node(
                    Severity::Warning,
                    index,
                    node,
                    "ss.unknown_method",
                    format!("unrecognised shadowsocks cipher `{method}`"),
                ));
            }
            if password.is_empty() {
                out.push(Diagnostic::for_node(
                    Severity::Error,
                    index,
                    node,
                    "ss.empty_password",
                    "shadowsocks password is empty",
                ));
            }
        }
        Credentials::Vmess { uuid, .. } | Credentials::Vless { uuid, .. } => {
            check_uuid(&mut out, index, node, uuid);
        }
        Credentials::Trojan { password } => {
            if password.is_empty() {
                out.push(Diagnostic::for_node(
                    Severity::Error,
                    index,
                    node,
                    "trojan.empty_password",
                    "trojan password is empty",
                ));
            }
        }
    }

    if let Credentials::Vless { flow, .. } = &node.credentials {
        if *flow == VlessFlow::Vision && node.tls.is_none() {
            out.push(Diagnostic::for_node(
                Severity::Warning,
                index,
                node,
                "vless.vision_without_tls",
                "flow `xtls-rprx-vision` requires TLS or Reality, but the node has none",
            ));
        }
    }

    if let Some(tls) = &node.tls {
        if tls.allow_insecure {
            out.push(Diagnostic::for_node(
                Severity::Warning,
                index,
                node,
                "tls.insecure",
                "certificate verification is disabled (allowInsecure)",
            ));
        }
        if let Some(reality) = &tls.reality {
            if reality.public_key.is_empty() {
                out.push(Diagnostic::for_node(
                    Severity::Error,
                    index,
                    node,
                    "reality.empty_public_key",
                    "Reality node has no public key (pbk)",
                ));
            } else if reality.public_key.len() != 43 {
                out.push(Diagnostic::for_node(
                    Severity::Warning,
                    index,
                    node,
                    "reality.public_key_length",
                    format!(
                        "Reality public key is {} characters, expected 43",
                        reality.public_key.len()
                    ),
                ));
            }
            if reality.short_id.len() > 16 {
                out.push(Diagnostic::for_node(
                    Severity::Warning,
                    index,
                    node,
                    "reality.short_id_length",
                    "Reality short id is longer than 16 characters",
                ));
            } else if !reality.short_id.is_empty()
                && !reality.short_id.chars().all(|c| c.is_ascii_hexdigit())
            {
                out.push(Diagnostic::for_node(
                    Severity::Warning,
                    index,
                    node,
                    "reality.short_id_not_hex",
                    "Reality short id should be hexadecimal",
                ));
            }
        }
    }

    match &node.transport {
        Transport::Ws(options) if !options.path.starts_with('/') => {
            out.push(Diagnostic::for_node(
                Severity::Warning,
                index,
                node,
                "ws.path_without_slash",
                format!("websocket path `{}` should start with `/`", options.path),
            ));
        }
        Transport::Grpc(options) if options.service_name.is_empty() => {
            out.push(Diagnostic::for_node(
                Severity::Warning,
                index,
                node,
                "grpc.empty_service_name",
                "gRPC transport has an empty service name",
            ));
        }
        _ => {}
    }

    if node.protocol == Protocol::Vless && node.server_name().is_none() {
        out.push(Diagnostic::for_node(
            Severity::Warning,
            index,
            node,
            "tls.no_server_name",
            "no server name available for TLS",
        ));
    }

    out
}

fn check_uuid(out: &mut Vec<Diagnostic>, index: usize, node: &Node, uuid: &str) {
    if uuid.trim().is_empty() {
        out.push(Diagnostic::for_node(
            Severity::Error,
            index,
            node,
            "node.empty_uuid",
            "UUID is empty",
        ));
        return;
    }
    if !is_uuid(uuid.trim()) {
        out.push(Diagnostic::for_node(
            Severity::Warning,
            index,
            node,
            "node.nonstandard_uuid",
            format!("`{uuid}` is not a standard UUID"),
        ));
    }
}

/// `8-4-4-4-12` hexadecimal, the only form VMess/VLESS servers accept.
fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if *byte != b'-' {
                    return false;
                }
            }
            _ => {
                if !byte.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

/// Validate a whole profile.
pub fn validate_profile(profile: &Profile) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    if profile.nodes.is_empty() {
        out.push(Diagnostic::new(
            Severity::Error,
            "profile.no_nodes",
            "profile contains no usable nodes",
        ));
        return out;
    }

    for (index, node) in profile.nodes.iter().enumerate() {
        out.extend(validate_node(node, index));
    }

    // Duplicate identity means the same endpoint is listed twice; harmless but
    // usually a provider mistake worth surfacing.
    let mut seen = std::collections::HashMap::new();
    for (index, node) in profile.nodes.iter().enumerate() {
        if let Some(previous) = seen.insert(node.id.clone(), index) {
            out.push(Diagnostic::for_node(
                Severity::Warning,
                index,
                node,
                "profile.duplicate_node",
                format!(
                    "duplicates node #{} (same endpoint and credentials)",
                    previous + 1
                ),
            ));
        }
    }

    if !out.iter().any(|d| d.severity == Severity::Error) {
        // nothing to add
    }

    out.sort_by(|a, b| {
        b.severity.cmp(&a.severity).then(
            a.node
                .unwrap_or(usize::MAX)
                .cmp(&b.node.unwrap_or(usize::MAX)),
        )
    });
    out
}

pub fn count(diagnostics: &[Diagnostic], severity: Severity) -> usize {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == severity)
        .count()
}

pub fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::node::{TlsConfig, WsOptions};

    fn base() -> Node {
        let mut node = Node {
            id: String::new(),
            name: "n".to_string(),
            protocol: Protocol::Trojan,
            server: "example.com".to_string(),
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
    fn a_clean_node_has_no_diagnostics() {
        assert!(validate_node(&base(), 0).is_empty());
    }

    #[test]
    fn empty_password_is_an_error() {
        let mut node = base();
        node.credentials = Credentials::Trojan {
            password: String::new(),
        };
        let diagnostics = validate_node(&node, 0);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == "trojan.empty_password"));
        assert_eq!(diagnostics[0].severity, Severity::Error);
    }

    #[test]
    fn reality_without_public_key_is_an_error() {
        let mut node = base();
        node.tls = Some(TlsConfig {
            reality: Some(Default::default()),
            ..Default::default()
        });
        let diagnostics = validate_node(&node, 0);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == "reality.empty_public_key"));
    }

    #[test]
    fn insecure_tls_is_only_a_warning() {
        let mut node = base();
        node.tls = Some(TlsConfig {
            allow_insecure: true,
            ..Default::default()
        });
        let diagnostics = validate_node(&node, 0);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Warning);
    }

    #[test]
    fn vision_without_tls_warns() {
        let mut node = base();
        node.protocol = Protocol::Vless;
        node.credentials = Credentials::Vless {
            uuid: "11111111-2222-3333-4444-555555555555".to_string(),
            flow: VlessFlow::Vision,
        };
        let diagnostics = validate_node(&node, 0);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == "vless.vision_without_tls"));
    }

    #[test]
    fn protocol_and_credentials_must_agree() {
        let mut node = base();
        node.protocol = Protocol::Vmess;
        let diagnostics = validate_node(&node, 0);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == "node.protocol_mismatch"));
    }

    #[test]
    fn ws_path_without_slash_warns() {
        let mut node = base();
        node.transport = Transport::Ws(WsOptions {
            path: "no-slash".to_string(),
            ..Default::default()
        });
        let diagnostics = validate_node(&node, 0);
        assert!(diagnostics
            .iter()
            .any(|d| d.code == "ws.path_without_slash"));
    }

    #[test]
    fn uuid_shape_is_checked() {
        assert!(is_uuid("11111111-2222-3333-4444-555555555555"));
        assert!(!is_uuid("11111111-2222-3333-4444-55555555555"));
        assert!(!is_uuid("zzzzzzzz-2222-3333-4444-555555555555"));
        assert!(!is_uuid("11111111222233334444555555555555"));
    }

    #[test]
    fn empty_profile_is_an_error() {
        let profile = Profile::new("empty", Vec::new());
        let diagnostics = validate_profile(&profile);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "profile.no_nodes");
        assert!(has_errors(&diagnostics));
    }

    #[test]
    fn duplicate_nodes_are_reported_after_errors() {
        let node = base();
        let profile = Profile::new("dup", vec![node.clone(), node]);
        let diagnostics = validate_profile(&profile);
        assert_eq!(count(&diagnostics, Severity::Warning), 1);
        assert!(!has_errors(&diagnostics));
    }
}
