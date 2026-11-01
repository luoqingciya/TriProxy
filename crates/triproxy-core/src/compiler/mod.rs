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

//! Config Compiler — Unified Profile → core-native config.
//!
//! ```text
//! Profile
//!   ├─→ MihomoCompiler   → config.yaml
//!   ├─→ SingBoxCompiler  → config.json
//!   └─→ XrayCompiler     → config.json
//! ```
//!
//! Design rules:
//!
//! - **A node that cannot be expressed faithfully is skipped, not approximated.**
//!   Silently dropping a transport option produces a config that connects but
//!   behaves differently, which is far worse than a node that is visibly absent.
//!   Skipped nodes are reported in [`CompiledConfig::skipped`].
//! - **Hard capability mismatches fail the whole compile.** If the core cannot
//!   serve the profile at all (plan ch.13), producing a config that will be
//!   rejected at startup helps nobody.
//! - The output is validated by the core itself (`mihomo -t`, `sing-box check`,
//!   `xray -test`) before it is ever used — see `CoreManager::start_with_config`.

pub mod mihomo;
pub mod singbox;
pub mod xray;
pub mod yaml;

use serde::{Deserialize, Serialize};

use crate::capability::CoreCapability;
use crate::core::CoreKind;
use crate::profile::Profile;

/// Name of the selector group every compiler creates, when the core has one.
pub const PROXY_GROUP: &str = "PROXY";

/// Knobs that are about the local listener, not about the profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileOptions {
    /// Local inbound port (mixed / socks).
    pub listen_port: u16,
    /// Local controller / API port, where the core exposes one.
    pub api_port: u16,
    pub log_level: String,
    pub allow_lan: bool,
    /// Create a TUN interface instead of only a local mixed port.
    pub tun: bool,
    /// How the core resolves names. See [`DnsMode`].
    pub dns: DnsMode,
    /// How traffic is split between the proxy and a direct connection.
    pub rules: RuleMode,
}

/// Routing strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuleMode {
    /// Everything through the proxy — what the compiler did before rules existed.
    #[default]
    Global,
    /// Local networks and Chinese sites go direct, everything else is proxied.
    Split,
}

impl RuleMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleMode::Global => "global",
            RuleMode::Split => "split",
        }
    }
}

/// DNS strategy baked into the generated config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DnsMode {
    /// Emit no DNS section; the core uses its own defaults.
    #[default]
    Off,
    /// Resolve proxied domains through the proxy, local domains directly.
    Split,
    /// Answer proxied domains with fake IPs and resolve the real name inside
    /// the tunnel. Only meaningful with TUN.
    FakeIp,
}

impl DnsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DnsMode::Off => "off",
            DnsMode::Split => "split",
            DnsMode::FakeIp => "fake-ip",
        }
    }

    /// TUN cannot work well without the core answering DNS itself: without a
    /// hijack rule the queries are just tunnelled to the original resolver as
    /// plain UDP, which is both slow and leaky. So TUN upgrades Off to Split
    /// unless the caller explicitly asked for something else.
    pub fn effective(self, tun: bool) -> DnsMode {
        match (self, tun) {
            (DnsMode::Off, true) => DnsMode::Split,
            (mode, _) => mode,
        }
    }
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            listen_port: 7890,
            api_port: 9090,
            log_level: "info".to_string(),
            allow_lan: false,
            tun: false,
            dns: DnsMode::Off,
            rules: RuleMode::Global,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedNode {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledConfig {
    pub core: CoreKind,
    /// File name the core expects (`config.yaml` / `config.json`).
    pub file_name: &'static str,
    pub text: String,
    /// Nodes that made it into the config.
    pub included: Vec<String>,
    /// Nodes the compiler refused to approximate.
    pub skipped: Vec<SkippedNode>,
}

impl CompiledConfig {
    pub fn is_usable(&self) -> bool {
        !self.included.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    /// The profile has no nodes at all.
    NoNodes,
    /// The core cannot serve this profile (plan ch.13 hard constraints).
    Incompatible { core: CoreKind, unmet: Vec<String> },
    /// Every node was skipped; the reason of the first skip is carried along.
    NothingCompilable {
        core: CoreKind,
        skipped: Vec<SkippedNode>,
    },
    /// Serialising the generated document failed. Should not happen.
    Internal { core: CoreKind, reason: String },
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::NoNodes => write!(f, "profile contains no nodes"),
            CompileError::Incompatible { core, unmet } => write!(
                f,
                "`{core}` cannot serve this profile: {}",
                unmet.join(", ")
            ),
            CompileError::NothingCompilable { core, skipped } => {
                let first = skipped
                    .first()
                    .map(|entry| format!("{}: {}", entry.name, entry.reason))
                    .unwrap_or_default();
                write!(
                    f,
                    "no node could be compiled for `{core}` ({} skipped, first: {first})",
                    skipped.len()
                )
            }
            CompileError::Internal { core, reason } => {
                write!(f, "cannot serialise the `{core}` config: {reason}")
            }
        }
    }
}

impl std::error::Error for CompileError {}

pub trait ConfigCompiler: Send + Sync {
    fn core(&self) -> CoreKind;

    /// File name the core expects for this format.
    fn file_name(&self) -> &'static str;

    fn compile(
        &self,
        profile: &Profile,
        options: &CompileOptions,
    ) -> Result<CompiledConfig, CompileError>;
}

pub fn compiler_for(kind: CoreKind) -> Box<dyn ConfigCompiler> {
    match kind {
        CoreKind::Mihomo => Box::new(mihomo::MihomoCompiler),
        CoreKind::SingBox => Box::new(singbox::SingBoxCompiler),
        CoreKind::Xray => Box::new(xray::XrayCompiler),
    }
}

/// Compile a profile for one core, applying the hard capability gate first.
pub fn compile(
    kind: CoreKind,
    profile: &Profile,
    options: &CompileOptions,
) -> Result<CompiledConfig, CompileError> {
    if profile.nodes.is_empty() {
        return Err(CompileError::NoNodes);
    }

    let capability = CoreCapability::for_core(kind);
    let mut unmet = profile.requirement().unmet_by(&capability);
    // TUN is a runtime option rather than a profile property, so it is checked
    // here instead of coming out of `requirement()`.
    if options.tun && !capability.supports_tun {
        unmet.push("TUN".to_string());
    }
    if !unmet.is_empty() {
        return Err(CompileError::Incompatible { core: kind, unmet });
    }

    let compiled = compiler_for(kind).compile(profile, options)?;
    if compiled.included.is_empty() {
        return Err(CompileError::NothingCompilable {
            core: kind,
            skipped: compiled.skipped,
        });
    }
    Ok(compiled)
}

/// Compile for every core, collecting per-core outcomes.
///
/// This is the shape Phase 4 needs: the selector compares what each core can do
/// with the same profile.
pub fn compile_all(
    profile: &Profile,
    options: &CompileOptions,
) -> Vec<(CoreKind, Result<CompiledConfig, CompileError>)> {
    CoreKind::ALL
        .iter()
        .map(|kind| (*kind, compile(*kind, profile, options)))
        .collect()
}

/// Reasons no core can express a node faithfully.
///
/// These are not "unsupported by this core" but "unsupported by the protocol":
/// a trojan node without TLS is not a trojan node, and every core would either
/// reject the config or silently speak plaintext.
pub(crate) fn common_skip_reason(node: &crate::profile::Node) -> Option<String> {
    use crate::profile::Credentials;

    if matches!(node.credentials, Credentials::Trojan { .. }) && node.tls.is_none() {
        return Some("trojan requires TLS, but this node has none".to_string());
    }
    if node.uses_vision() && node.tls.is_none() {
        return Some("VLESS flow `xtls-rprx-vision` requires TLS".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Protocol;
    use crate::profile::{Credentials, Node, Transport};

    fn node(protocol: Protocol, server: &str) -> Node {
        let credentials = match protocol {
            Protocol::Trojan => Credentials::Trojan {
                password: "pw".to_string(),
            },
            Protocol::Shadowsocks => Credentials::Shadowsocks {
                method: "aes-256-gcm".to_string(),
                password: "pw".to_string(),
            },
            _ => Credentials::Trojan {
                password: "pw".to_string(),
            },
        };
        // Trojan is only a valid node with TLS; without it every compiler
        // rejects the node on purpose.
        let tls = matches!(credentials, Credentials::Trojan { .. })
            .then(crate::profile::TlsConfig::default);
        let mut node = Node {
            id: String::new(),
            name: format!("{protocol}-{server}"),
            protocol: credentials.protocol(),
            server: server.to_string(),
            port: 443,
            credentials,
            transport: Transport::Tcp,
            tls,
            udp: true,
            plugin: None,
        };
        node.refresh_id();
        node
    }

    #[test]
    fn empty_profile_is_rejected() {
        let profile = Profile::new("p", Vec::new());
        assert_eq!(
            compile(CoreKind::Mihomo, &profile, &CompileOptions::default()),
            Err(CompileError::NoNodes)
        );
    }

    #[test]
    fn all_three_cores_compile_the_same_profile() {
        let profile = Profile::new(
            "p",
            vec![
                node(Protocol::Trojan, "a.example.com"),
                node(Protocol::Shadowsocks, "b.example.com"),
            ],
        );
        for kind in CoreKind::ALL {
            let compiled = compile(kind, &profile, &CompileOptions::default())
                .unwrap_or_else(|error| panic!("{kind} failed: {error}"));
            assert_eq!(compiled.core, kind);
            assert_eq!(compiled.included.len(), 2);
            assert!(compiled.skipped.is_empty());
            assert!(compiled.is_usable());
        }
    }

    #[test]
    fn compile_all_reports_one_outcome_per_core() {
        let profile = Profile::new("p", vec![node(Protocol::Trojan, "a.example.com")]);
        let outcomes = compile_all(&profile, &CompileOptions::default());
        assert_eq!(outcomes.len(), 3);
        assert!(outcomes.iter().all(|(_, result)| result.is_ok()));
    }

    #[test]
    fn incompatible_core_is_rejected_before_compiling() {
        // Xray has no native TUN; simulate a profile that needs it.
        let profile = Profile::new("p", vec![node(Protocol::Trojan, "a.example.com")]);
        let mut requirement = profile.requirement();
        requirement.requires_tun = true;
        let capability = CoreCapability::for_core(CoreKind::Xray);
        assert_eq!(requirement.unmet_by(&capability), vec!["TUN".to_string()]);
    }
}
