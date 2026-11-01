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

use serde::{Deserialize, Serialize};

use crate::core::CoreKind;

/// Proxy protocols understood by the unified node model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Shadowsocks,
    Vmess,
    Vless,
    Trojan,
    WireGuard,
    Hysteria2,
    Tuic,
    Socks,
    Http,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Shadowsocks => "shadowsocks",
            Protocol::Vmess => "vmess",
            Protocol::Vless => "vless",
            Protocol::Trojan => "trojan",
            Protocol::WireGuard => "wireguard",
            Protocol::Hysteria2 => "hysteria2",
            Protocol::Tuic => "tuic",
            Protocol::Socks => "socks",
            Protocol::Http => "http",
        }
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Static capability declaration of a core.
///
/// This is *declared* capability, not runtime probing: it is the input of the
/// Smart Core Selector (Phase 4). Keeping it static and explicit is what makes
/// automatic core selection explainable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreCapability {
    pub protocols: Vec<Protocol>,
    pub supports_tun: bool,
    pub supports_dns: bool,
    pub supports_reality: bool,
    pub supports_vision: bool,
    pub supports_wireguard: bool,
    pub supports_rule_set: bool,
    pub supports_proxy_groups: bool,
}

impl CoreCapability {
    pub fn supports_protocol(&self, protocol: Protocol) -> bool {
        self.protocols.contains(&protocol)
    }

    pub fn for_core(kind: CoreKind) -> Self {
        match kind {
            // Clash.Meta lineage: broad protocol coverage, rule-providers
            // (not native rule-sets), mature TUN and DNS stacks.
            CoreKind::Mihomo => CoreCapability {
                protocols: vec![
                    Protocol::Shadowsocks,
                    Protocol::Vmess,
                    Protocol::Vless,
                    Protocol::Trojan,
                    Protocol::WireGuard,
                    Protocol::Hysteria2,
                    Protocol::Tuic,
                    Protocol::Socks,
                    Protocol::Http,
                ],
                supports_tun: true,
                supports_dns: true,
                supports_reality: true,
                supports_vision: true,
                supports_wireguard: true,
                supports_rule_set: false,
                supports_proxy_groups: true,
            },
            // Modern general purpose core: native rule-sets, selector/urltest
            // outbound groups, first class TUN and DNS.
            CoreKind::SingBox => CoreCapability {
                protocols: vec![
                    Protocol::Shadowsocks,
                    Protocol::Vmess,
                    Protocol::Vless,
                    Protocol::Trojan,
                    Protocol::WireGuard,
                    Protocol::Hysteria2,
                    Protocol::Tuic,
                    Protocol::Socks,
                    Protocol::Http,
                ],
                supports_tun: true,
                supports_dns: true,
                supports_reality: true,
                supports_vision: true,
                supports_wireguard: true,
                supports_rule_set: true,
                supports_proxy_groups: true,
            },
            // V2Ray/Xray lineage: strongest transport/reality story, but no
            // native TUN and no rule-set concept.
            CoreKind::Xray => CoreCapability {
                protocols: vec![
                    Protocol::Shadowsocks,
                    Protocol::Vmess,
                    Protocol::Vless,
                    Protocol::Trojan,
                    Protocol::WireGuard,
                    Protocol::Socks,
                    Protocol::Http,
                ],
                supports_tun: false,
                supports_dns: true,
                supports_reality: true,
                supports_vision: true,
                supports_wireguard: true,
                supports_rule_set: false,
                supports_proxy_groups: false,
            },
        }
    }
}

/// What a profile actually demands from a core.
///
/// Populated by the Unified Profile analyzer (Phase 2/4). Phase 1 only needs the
/// type so that the compatibility check has a stable shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRequirement {
    pub protocols: Vec<Protocol>,
    pub requires_tun: bool,
    pub requires_dns: bool,
    pub requires_reality: bool,
    pub requires_vision: bool,
    pub requires_wireguard: bool,
    pub requires_rule_set: bool,
    pub requires_proxy_groups: bool,
}

impl ProfileRequirement {
    /// Hard-constraint check. Returns the list of unmet requirements; an empty
    /// list means the core can serve the profile.
    ///
    /// Per the project plan (ch.13), hard incompatibility must eliminate a core
    /// outright instead of being absorbed by a weighted score.
    pub fn unmet_by(&self, capability: &CoreCapability) -> Vec<String> {
        let mut unmet = Vec::new();
        for protocol in &self.protocols {
            if !capability.supports_protocol(*protocol) {
                unmet.push(format!("protocol {protocol}"));
            }
        }
        if self.requires_tun && !capability.supports_tun {
            unmet.push("TUN".to_string());
        }
        if self.requires_dns && !capability.supports_dns {
            unmet.push("DNS".to_string());
        }
        if self.requires_reality && !capability.supports_reality {
            unmet.push("Reality".to_string());
        }
        if self.requires_vision && !capability.supports_vision {
            unmet.push("Vision".to_string());
        }
        if self.requires_wireguard && !capability.supports_wireguard {
            unmet.push("WireGuard".to_string());
        }
        if self.requires_rule_set && !capability.supports_rule_set {
            unmet.push("rule-set".to_string());
        }
        if self.requires_proxy_groups && !capability.supports_proxy_groups {
            unmet.push("proxy-groups".to_string());
        }
        unmet
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xray_is_rejected_when_tun_is_required() {
        let requirement = ProfileRequirement {
            protocols: vec![Protocol::Vless],
            requires_tun: true,
            requires_reality: true,
            ..Default::default()
        };
        let unmet = requirement.unmet_by(&CoreCapability::for_core(CoreKind::Xray));
        assert_eq!(unmet, vec!["TUN".to_string()]);
    }

    #[test]
    fn mihomo_covers_first_stage_protocols() {
        let capability = CoreCapability::for_core(CoreKind::Mihomo);
        for protocol in [
            Protocol::Shadowsocks,
            Protocol::Vmess,
            Protocol::Vless,
            Protocol::Trojan,
            Protocol::WireGuard,
        ] {
            assert!(capability.supports_protocol(protocol), "{protocol}");
        }
    }

    #[test]
    fn only_singbox_and_mihomo_claim_native_rule_sets() {
        assert!(CoreCapability::for_core(CoreKind::SingBox).supports_rule_set);
        assert!(!CoreCapability::for_core(CoreKind::Xray).supports_rule_set);
    }
}
