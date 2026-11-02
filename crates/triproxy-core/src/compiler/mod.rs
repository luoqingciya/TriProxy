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
pub mod rules;
pub mod singbox;
pub mod xray;
pub mod yaml;

pub use rules::{
    parse_rule_sets, parse_rules, CustomRule, RuleError, RuleKind, RuleSet, RuleSetBehavior,
    RuleSetFormat, RuleTarget,
};

use serde::{Deserialize, Serialize};

use crate::capability::CoreCapability;
use crate::core::CoreKind;
use crate::profile::Profile;

/// Name of the selector group every compiler creates, when the core has one.
pub const PROXY_GROUP: &str = "PROXY";

/// Name of the automatic group nested inside [`PROXY_GROUP`].
///
/// One vocabulary across the two cores that have groups, so the controller call
/// that switches to it is the same string everywhere.
pub const AUTO_GROUP: &str = "AUTO";

/// How long the automatic group waits between latency probes.
pub const AUTO_TEST_INTERVAL_SECS: u64 = 300;

/// Probe URL for the automatic group.
///
/// Same endpoint the latency measurement uses, and the one every Clash-family
/// client tests against: a 204 with no body, so it measures the round trip and
/// nothing else.
pub const AUTO_TEST_URL: &str = "http://www.gstatic.com/generate_204";

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
    /// Node the user picked, by id. Moved to the front of the outbound list so
    /// every core starts on it; a running core with a controller API is switched
    /// at runtime instead of being restarted.
    pub selected_node: Option<String>,
    /// The same choice resolved to a name, filled in by [`compile`].
    ///
    /// Compilers work in node names — that is what goes into a config — while
    /// the persisted choice is an id. Resolving it once, in `compile`, keeps
    /// every compiler from repeating the lookup.
    #[serde(skip)]
    pub selected_name: Option<String>,
    /// User-written rules, applied before the presets so they can override them.
    #[serde(default)]
    pub custom_rules: Vec<CustomRule>,
    /// Remote rule-sets, emitted only to the cores that can read their format.
    #[serde(default)]
    pub rule_sets: Vec<RuleSet>,
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
            selected_node: None,
            selected_name: None,
            custom_rules: Vec::new(),
            rule_sets: Vec::new(),
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

/// Clone a profile with the chosen node moved to the front.
///
/// Order matters because every core treats the first outbound as the default:
/// Mihomo uses the first member of a select group, sing-box the first outbound,
/// Xray the first entry in its outbounds list. Reordering here means the
/// compilers stay unaware of selection entirely.
fn with_selected_first(profile: &Profile, id: &str) -> Profile {
    let mut clone = profile.clone();
    if let Some(index) = clone.nodes.iter().position(|node| node.id == id) {
        let chosen = clone.nodes.remove(index);
        clone.nodes.insert(0, chosen);
    }
    clone
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

    // Put the user-selected node first. All three cores take the first outbound
    // (or first group member) as the default, so this is the one place that
    // needs to know about the choice.
    let ordered;
    let mut resolved = options.clone();
    let profile = match options.selected_node.as_deref() {
        Some(id) if profile.nodes.iter().any(|node| node.id == id) => {
            resolved.selected_name = profile
                .nodes
                .iter()
                .find(|node| node.id == id)
                .map(|node| node.name.clone());
            ordered = with_selected_first(profile, id);
            &ordered
        }
        _ => {
            resolved.selected_name = None;
            profile
        }
    };

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

    let compiled = compiler_for(kind).compile(profile, &resolved)?;
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
    fn the_selected_node_is_compiled_first() {
        // Every core treats the first outbound (or first group member) as the
        // default, so "which node is used" is purely a matter of ordering.
        let a = node(Protocol::Trojan, "a.example.com");
        let b = node(Protocol::Trojan, "b.example.com");
        let profile = Profile::new("p", vec![a.clone(), b.clone()]);

        let options = CompileOptions {
            selected_node: Some(b.id.clone()),
            ..Default::default()
        };

        let mihomo = compile(CoreKind::Mihomo, &profile, &options).unwrap();
        // The first member after the group name is the default selection.
        let group_start = mihomo.text.find("proxy-groups").unwrap();
        let group = &mihomo.text[group_start..];
        let b_at = group.find(&b.name).unwrap();
        let a_at = group.find(&a.name).unwrap();
        assert!(
            b_at < a_at,
            "selected node should come first:\n{}",
            mihomo.text
        );

        let singbox = compile(CoreKind::SingBox, &profile, &options).unwrap();
        let b_at = singbox.text.find(&b.name).unwrap();
        let a_at = singbox.text.find(&a.name).unwrap();
        assert!(
            b_at < a_at,
            "selected node should come first:\n{}",
            singbox.text
        );

        let xray = compile(CoreKind::Xray, &profile, &options).unwrap();
        let b_at = xray.text.find(&b.name).unwrap();
        let a_at = xray.text.find(&a.name).unwrap();
        assert!(
            b_at < a_at,
            "selected node should come first:\n{}",
            xray.text
        );
    }

    /// A pick has to beat the automatic group, not merely sit next to it.
    ///
    /// Both cores default a `select` group to its first member, and AUTO is a
    /// member too — so the only thing that decides this is whether the picked
    /// node was inserted before AUTO.
    #[test]
    fn an_explicit_pick_is_the_selector_default() {
        let a = node(Protocol::Trojan, "a.example.com");
        let b = node(Protocol::Trojan, "b.example.com");
        let profile = Profile::new("p", vec![a.clone(), b.clone()]);
        let options = CompileOptions {
            selected_node: Some(b.id.clone()),
            ..Default::default()
        };

        let singbox = compile(CoreKind::SingBox, &profile, &options).unwrap();
        let document: serde_json::Value = serde_json::from_str(&singbox.text).unwrap();
        let selector = document["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|outbound| outbound["tag"] == PROXY_GROUP)
            .expect("selector outbound");
        assert_eq!(selector["default"], b.name);
        assert_eq!(selector["outbounds"][0], b.name);
        assert_eq!(selector["outbounds"][1], AUTO_GROUP);

        let mihomo = compile(CoreKind::Mihomo, &profile, &options).unwrap();
        let proxy_at = mihomo
            .text
            .find(&format!("\"name\": \"{PROXY_GROUP}\""))
            .unwrap();
        let group = &mihomo.text[proxy_at..];
        let picked_at = group.find(&b.name).unwrap();
        let auto_at = group.find(AUTO_GROUP).unwrap();
        assert!(
            picked_at < auto_at,
            "the pick must precede AUTO inside {PROXY_GROUP}:\n{}",
            mihomo.text
        );
    }

    /// A user rule has to beat the preset it contradicts, in all three dialects.
    #[test]
    fn custom_rules_are_emitted_before_the_presets() {
        let node = node(Protocol::Trojan, "a.example.com");
        let profile = Profile::new("p", vec![node]);
        let options = CompileOptions {
            rules: RuleMode::Split,
            custom_rules: vec![CustomRule {
                kind: RuleKind::DomainSuffix,
                value: "corp.internal".to_string(),
                target: RuleTarget::Direct,
            }],
            ..Default::default()
        };

        let mihomo = compile(CoreKind::Mihomo, &profile, &options).unwrap();
        let custom_at = mihomo
            .text
            .find("DOMAIN-SUFFIX,corp.internal,DIRECT")
            .unwrap();
        let preset_at = mihomo.text.find("GEOSITE,cn,DIRECT").unwrap();
        assert!(
            custom_at < preset_at,
            "custom rule must come first:\n{}",
            mihomo.text
        );

        let singbox = compile(CoreKind::SingBox, &profile, &options).unwrap();
        let document: serde_json::Value = serde_json::from_str(&singbox.text).unwrap();
        let first = &document["route"]["rules"][0];
        assert_eq!(first["domain_suffix"][0], "corp.internal");
        assert_eq!(first["outbound"], "direct");

        let xray = compile(CoreKind::Xray, &profile, &options).unwrap();
        let document: serde_json::Value = serde_json::from_str(&xray.text).unwrap();
        let first = &document["routing"]["rules"][0];
        assert_eq!(first["domain"][0], "domain:corp.internal");
        assert_eq!(first["outboundTag"], "direct");
    }

    /// In global mode everything goes to the proxy, so a custom rule is the only
    /// thing that can send a request elsewhere — the routing section has to
    /// exist even though the preset contributes nothing.
    #[test]
    fn xray_emits_routing_for_custom_rules_in_global_mode() {
        let node = node(Protocol::Trojan, "a.example.com");
        let profile = Profile::new("p", vec![node]);
        let options = CompileOptions {
            custom_rules: vec![CustomRule {
                kind: RuleKind::IpCidr,
                value: "10.0.0.0/8".to_string(),
                target: RuleTarget::Direct,
            }],
            ..Default::default()
        };

        let xray = compile(CoreKind::Xray, &profile, &options).unwrap();
        let document: serde_json::Value = serde_json::from_str(&xray.text).unwrap();
        assert_eq!(document["routing"]["rules"][0]["ip"][0], "10.0.0.0/8");
    }

    /// SOCKS5 and HTTP are the two protocols every core speaks, and the two most
    /// likely to appear in a chain. Each spells them differently.
    #[test]
    fn proxy_outbound_nodes_compile_for_every_core() {
        let socks = crate::profile::parse_uri("socks5://user:pass@proxy.example.com:1080")
            .expect("socks uri");
        let profile = Profile::new("p", vec![socks]);
        let options = CompileOptions::default();

        let mihomo = compile(CoreKind::Mihomo, &profile, &options).unwrap();
        assert!(
            mihomo.text.contains("\"type\": \"socks5\""),
            "{}",
            mihomo.text
        );
        assert!(
            mihomo.text.contains("\"username\": \"user\""),
            "{}",
            mihomo.text
        );

        let singbox = compile(CoreKind::SingBox, &profile, &options).unwrap();
        assert!(
            singbox.text.contains("\"type\": \"socks\""),
            "{}",
            singbox.text
        );
        assert!(
            singbox.text.contains("\"version\": \"5\""),
            "{}",
            singbox.text
        );

        let xray = compile(CoreKind::Xray, &profile, &options).unwrap();
        assert!(
            xray.text.contains("\"protocol\": \"socks\""),
            "{}",
            xray.text
        );
        assert!(xray.text.contains("\"user\": \"user\""), "{}", xray.text);

        let http =
            crate::profile::parse_uri("http://user:pass@proxy.example.com:8080").expect("http uri");
        let profile = Profile::new("p", vec![http]);

        let mihomo = compile(CoreKind::Mihomo, &profile, &options).unwrap();
        assert!(
            mihomo.text.contains("\"type\": \"http\""),
            "{}",
            mihomo.text
        );

        let singbox = compile(CoreKind::SingBox, &profile, &options).unwrap();
        assert!(
            singbox.text.contains("\"type\": \"http\""),
            "{}",
            singbox.text
        );

        let xray = compile(CoreKind::Xray, &profile, &options).unwrap();
        assert!(
            xray.text.contains("\"protocol\": \"http\""),
            "{}",
            xray.text
        );
    }

    /// The two families read different files, so an entry goes to exactly one
    /// core. Sending a `.srs` to Mihomo would make it refuse the whole config.
    #[test]
    fn rule_sets_go_only_to_the_core_that_can_read_them() {
        let node = node(Protocol::Trojan, "a.example.com");
        let profile = Profile::new("p", vec![node]);
        let options = CompileOptions {
            rule_sets: parse_rule_sets(
                "DIRECT,domain,https://example.com/cn.mrs\nPROXY,domain,https://example.com/us.srs",
            )
            .unwrap(),
            ..Default::default()
        };

        let mihomo = compile(CoreKind::Mihomo, &profile, &options).unwrap();
        assert!(
            mihomo.text.contains("\"rule-providers\""),
            "{}",
            mihomo.text
        );
        assert!(
            mihomo.text.contains("RULE-SET,cn,DIRECT"),
            "{}",
            mihomo.text
        );
        assert!(!mihomo.text.contains("us.srs"), "{}", mihomo.text);

        let singbox = compile(CoreKind::SingBox, &profile, &options).unwrap();
        let document: serde_json::Value = serde_json::from_str(&singbox.text).unwrap();
        let tags: Vec<&str> = document["route"]["rule_set"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["tag"].as_str().unwrap())
            .collect();
        assert_eq!(tags, vec!["us"], "{tags:?}");

        // Xray has no rule-set concept at all; the entry must not leak into it.
        let xray = compile(CoreKind::Xray, &profile, &options).unwrap();
        assert!(!xray.text.contains("cn.mrs"), "{}", xray.text);
    }

    #[test]
    fn an_unknown_selection_falls_back_to_the_profile_order() {
        let a = node(Protocol::Trojan, "a.example.com");
        let b = node(Protocol::Trojan, "b.example.com");
        let profile = Profile::new("p", vec![a.clone(), b.clone()]);

        let options = CompileOptions {
            selected_node: Some("not-a-real-id".to_string()),
            ..Default::default()
        };

        // A stale selection must not fail the compile; the profile order wins.
        let compiled = compile(CoreKind::Mihomo, &profile, &options).unwrap();
        let group_start = compiled.text.find("proxy-groups").unwrap();
        let group = &compiled.text[group_start..];
        assert!(group.find(&a.name).unwrap() < group.find(&b.name).unwrap());
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

    #[test]
    fn hysteria2_compiles_for_two_cores_and_is_refused_by_the_third() {
        let hy2 = Node {
            id: String::new(),
            name: "hy2".to_string(),
            protocol: Protocol::Hysteria2,
            server: "hy.example.com".to_string(),
            port: 443,
            credentials: Credentials::Hysteria2 {
                password: "secret".to_string(),
                obfs: Some("salamander".to_string()),
                obfs_password: Some("obfspw".to_string()),
            },
            transport: Transport::Tcp,
            tls: Some(crate::profile::TlsConfig {
                server_name: Some("www.bing.com".to_string()),
                ..Default::default()
            }),
            udp: true,
            plugin: None,
        };
        let mut hy2 = hy2;
        hy2.refresh_id();
        let profile = Profile::new("p", vec![hy2]);

        let options = CompileOptions::default();

        // Mihomo spells the obfuscation fields with hyphens.
        let mihomo = compile(CoreKind::Mihomo, &profile, &options).unwrap();
        assert!(mihomo.text.contains("hysteria2"), "{}", mihomo.text);
        assert!(mihomo.text.contains("obfs-password"), "{}", mihomo.text);

        // sing-box nests obfuscation in an object.
        let singbox = compile(CoreKind::SingBox, &profile, &options).unwrap();
        assert!(singbox.text.contains("\"hysteria2\""), "{}", singbox.text);
        assert!(singbox.text.contains("\"obfs\""), "{}", singbox.text);

        // Xray cannot run it at all; the gate must say so by name rather than
        // producing a config that silently drops the node.
        let error = compile(CoreKind::Xray, &profile, &options).unwrap_err();
        let message = format!("{error}");
        assert!(message.contains("hysteria2"), "{message}");
    }
}
