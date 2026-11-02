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

//! Unified node model.
//!
//! One node representation for every input format and every core. Nothing in
//! this module knows about Clash YAML, sing-box JSON or Xray JSON — that mapping
//! is the job of the config compiler (Phase 3).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::capability::Protocol;

/// Stable identity of a node.
///
/// Derived from the connection parameters only, never from the display name, so
/// renaming a node (or a provider renaming it) does not break favourites or
/// latency history.
pub type NodeId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    /// Display name, usually the URI fragment.
    pub name: String,
    /// Kept alongside `credentials` for filtering and display; the validator
    /// checks that the two agree.
    pub protocol: Protocol,
    pub server: String,
    pub port: u16,
    pub credentials: Credentials,
    #[serde(default)]
    pub transport: Transport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsConfig>,
    #[serde(default = "default_true")]
    pub udp: bool,
    /// Shadowsocks plugin string, kept verbatim (`obfs-local;obfs=http`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Node {
    /// Recompute the identity after mutating connection parameters.
    pub fn refresh_id(&mut self) {
        self.id = self.compute_id();
    }

    pub fn compute_id(&self) -> NodeId {
        let mut hasher = Sha256::new();
        for part in [
            self.protocol.as_str().to_string(),
            self.server.to_ascii_lowercase(),
            self.port.to_string(),
            self.credentials.identity(),
            self.transport.identity(),
        ] {
            hasher.update(part.as_bytes());
            hasher.update(b"\x1f");
        }
        hex::encode(hasher.finalize())[..16].to_string()
    }

    /// `server:port`, bracketing IPv6 literals.
    pub fn endpoint(&self) -> String {
        if self.server.contains(':') {
            format!("[{}]:{}", self.server, self.port)
        } else {
            format!("{}:{}", self.server, self.port)
        }
    }

    /// Effective SNI: explicit value, else the ws/http host header, else server.
    pub fn server_name(&self) -> Option<&str> {
        if let Some(tls) = &self.tls {
            if let Some(name) = &tls.server_name {
                return Some(name);
            }
        }
        match &self.transport {
            Transport::Ws(options) => options.host.as_deref(),
            Transport::Http2(options) => options.host.first().map(String::as_str),
            _ => None,
        }
        .or(Some(self.server.as_str()))
    }

    pub fn uses_reality(&self) -> bool {
        self.tls
            .as_ref()
            .map(|tls| tls.reality.is_some())
            .unwrap_or(false)
    }

    pub fn uses_vision(&self) -> bool {
        matches!(
            self.credentials,
            Credentials::Vless {
                flow: VlessFlow::Vision,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Credentials {
    Shadowsocks {
        method: String,
        password: String,
    },
    Vmess {
        uuid: String,
        #[serde(default)]
        alter_id: u32,
        /// VMess encryption, `auto` when unspecified.
        #[serde(default = "default_vmess_security")]
        security: String,
    },
    Vless {
        uuid: String,
        #[serde(default)]
        flow: VlessFlow,
    },
    Trojan {
        password: String,
    },
    /// QUIC-based protocol. Always TLS; the obfuscation layer is optional and
    /// only  is in scope.
    Hysteria2 {
        password: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        obfs: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        obfs_password: Option<String>,
    },
}

fn default_vmess_security() -> String {
    "auto".to_string()
}

impl Credentials {
    /// Canonical form used for node identity. Includes secrets on purpose: two
    /// nodes differing only by password are genuinely different nodes.
    fn identity(&self) -> String {
        match self {
            Credentials::Shadowsocks { method, password } => {
                format!("ss:{method}:{password}")
            }
            Credentials::Vmess {
                uuid,
                alter_id,
                security,
            } => format!("vmess:{uuid}:{alter_id}:{security}"),
            Credentials::Vless { uuid, flow } => format!("vless:{uuid}:{}", flow.as_str()),
            Credentials::Trojan { password } => format!("trojan:{password}"),
            Credentials::Hysteria2 {
                password,
                obfs,
                obfs_password,
            } => format!(
                "hysteria2:{password}:{}:{}",
                obfs.clone().unwrap_or_default(),
                obfs_password.clone().unwrap_or_default()
            ),
        }
    }

    pub fn protocol(&self) -> Protocol {
        match self {
            Credentials::Shadowsocks { .. } => Protocol::Shadowsocks,
            Credentials::Vmess { .. } => Protocol::Vmess,
            Credentials::Vless { .. } => Protocol::Vless,
            Credentials::Trojan { .. } => Protocol::Trojan,
            Credentials::Hysteria2 { .. } => Protocol::Hysteria2,
        }
    }
}

/// VLESS flow control. Only `xtls-rprx-vision` is in scope; anything else is
/// rejected by the validator rather than silently dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VlessFlow {
    #[default]
    None,
    Vision,
}

impl VlessFlow {
    pub fn as_str(self) -> &'static str {
        match self {
            VlessFlow::None => "",
            VlessFlow::Vision => "xtls-rprx-vision",
        }
    }

    /// Unknown flows are returned as `None` together with the raw value so the
    /// caller can report them instead of guessing.
    pub fn parse(value: &str) -> (Self, Option<String>) {
        match value.trim() {
            "" => (VlessFlow::None, None),
            "xtls-rprx-vision" => (VlessFlow::Vision, None),
            other => (VlessFlow::None, Some(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Transport {
    #[default]
    Tcp,
    Ws(WsOptions),
    Grpc(GrpcOptions),
    Http2(Http2Options),
}

impl Transport {
    fn identity(&self) -> String {
        match self {
            Transport::Tcp => "tcp".to_string(),
            Transport::Ws(options) => format!(
                "ws:{}:{}",
                options.path,
                options.host.clone().unwrap_or_default()
            ),
            Transport::Grpc(options) => format!("grpc:{}", options.service_name),
            Transport::Http2(options) => format!("h2:{}:{}", options.path, options.host.join(",")),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Transport::Tcp => "tcp",
            Transport::Ws(_) => "ws",
            Transport::Grpc(_) => "grpc",
            Transport::Http2(_) => "h2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsOptions {
    #[serde(default = "default_ws_path")]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_early_data: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub early_data_header: Option<String>,
}

impl Default for WsOptions {
    fn default() -> Self {
        Self {
            path: default_ws_path(),
            host: None,
            max_early_data: None,
            early_data_header: None,
        }
    }
}

fn default_ws_path() -> String {
    "/".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GrpcOptions {
    #[serde(default)]
    pub service_name: String,
    #[serde(default)]
    pub multi_mode: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Http2Options {
    #[serde(default = "default_ws_path")]
    pub path: String,
    #[serde(default)]
    pub host: Vec<String>,
}

impl Default for Http2Options {
    fn default() -> Self {
        Self {
            path: default_ws_path(),
            host: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(default)]
    pub allow_insecure: bool,
    #[serde(default)]
    pub alpn: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// Present when the node uses Reality instead of ordinary TLS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reality: Option<RealityConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RealityConfig {
    /// base64url x25519 public key (`pbk`).
    #[serde(default)]
    pub public_key: String,
    /// Hex short id (`sid`), may be empty.
    #[serde(default)]
    pub short_id: String,
    /// SpiderX path (`spx`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spider_x: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(server: &str, password: &str, name: &str) -> Node {
        let mut node = Node {
            id: String::new(),
            name: name.to_string(),
            protocol: Protocol::Trojan,
            server: server.to_string(),
            port: 443,
            credentials: Credentials::Trojan {
                password: password.to_string(),
            },
            transport: Transport::Tcp,
            tls: Some(TlsConfig::default()),
            udp: true,
            plugin: None,
        };
        node.refresh_id();
        node
    }

    #[test]
    fn identity_ignores_the_display_name() {
        let a = sample("example.com", "secret", "Tokyo 01");
        let b = sample("example.com", "secret", "renamed by provider");
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn identity_changes_with_connection_parameters() {
        let base = sample("example.com", "secret", "n");
        assert_ne!(base.id, sample("example.org", "secret", "n").id);
        assert_ne!(base.id, sample("example.com", "other", "n").id);
        // Hostnames are case-insensitive, so the same endpoint spelled
        // differently must keep the same identity.
        assert_eq!(base.id, sample("EXAMPLE.com", "secret", "n").id);
    }

    #[test]
    fn identity_separates_transports_on_the_same_endpoint() {
        let mut tcp = sample("example.com", "secret", "n");
        let mut ws = tcp.clone();
        ws.transport = Transport::Ws(WsOptions {
            path: "/a".to_string(),
            ..Default::default()
        });
        ws.refresh_id();
        tcp.refresh_id();
        assert_ne!(tcp.id, ws.id);

        let mut ws2 = ws.clone();
        ws2.transport = Transport::Ws(WsOptions {
            path: "/b".to_string(),
            ..Default::default()
        });
        ws2.refresh_id();
        assert_ne!(ws.id, ws2.id);
    }

    #[test]
    fn endpoint_brackets_ipv6_literals() {
        assert_eq!(
            sample("example.com", "p", "n").endpoint(),
            "example.com:443"
        );
        assert_eq!(
            sample("2001:db8::1", "p", "n").endpoint(),
            "[2001:db8::1]:443"
        );
    }

    #[test]
    fn server_name_falls_back_to_the_ws_host_header() {
        let mut node = sample("1.2.3.4", "p", "n");
        node.tls = Some(TlsConfig::default());
        node.transport = Transport::Ws(WsOptions {
            host: Some("cdn.example.com".to_string()),
            ..Default::default()
        });
        assert_eq!(node.server_name(), Some("cdn.example.com"));

        node.tls = Some(TlsConfig {
            server_name: Some("sni.example.com".to_string()),
            ..Default::default()
        });
        assert_eq!(node.server_name(), Some("sni.example.com"));

        node.tls = Some(TlsConfig::default());
        node.transport = Transport::Tcp;
        assert_eq!(node.server_name(), Some("1.2.3.4"));
    }

    #[test]
    fn flow_parse_reports_unknown_values() {
        assert_eq!(VlessFlow::parse(""), (VlessFlow::None, None));
        assert_eq!(
            VlessFlow::parse("xtls-rprx-vision"),
            (VlessFlow::Vision, None)
        );
        let (flow, unknown) = VlessFlow::parse("xtls-rprx-origin");
        assert_eq!(flow, VlessFlow::None);
        assert_eq!(unknown.as_deref(), Some("xtls-rprx-origin"));
    }

    #[test]
    fn node_roundtrips_through_json() {
        let node = sample("example.com", "secret", "n");
        let text = serde_json::to_string(&node).unwrap();
        let back: Node = serde_json::from_str(&text).unwrap();
        assert_eq!(node, back);
    }
}
