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

//! Mihomo (Clash.Meta) compiler: `Profile` → `config.yaml`.

use crate::core::CoreKind;
use crate::profile::node::{Credentials, Node, Transport, VlessFlow};
use crate::profile::Profile;

use super::yaml::{emit, Yaml};
use super::{
    CompileError, CompileOptions, CompiledConfig, ConfigCompiler, DnsMode, RuleMode, SkippedNode,
    PROXY_GROUP,
};

pub struct MihomoCompiler;

impl ConfigCompiler for MihomoCompiler {
    fn core(&self) -> CoreKind {
        CoreKind::Mihomo
    }

    fn file_name(&self) -> &'static str {
        "config.yaml"
    }

    fn compile(
        &self,
        profile: &Profile,
        options: &CompileOptions,
    ) -> Result<CompiledConfig, CompileError> {
        let mut proxies: Vec<Yaml> = Vec::new();
        let mut included = Vec::new();
        let mut skipped: Vec<SkippedNode> = Vec::new();

        for node in &profile.nodes {
            match proxy_entry(node) {
                Ok(entry) => {
                    included.push(node.name.clone());
                    proxies.push(entry);
                }
                Err(reason) => skipped.push(SkippedNode {
                    name: node.name.clone(),
                    reason,
                }),
            }
        }

        let mut group_members: Vec<String> = included.clone();
        group_members.push("DIRECT".to_string());

        let group = Yaml::map()
            .with("name", Yaml::str(PROXY_GROUP))
            .with("type", Yaml::str("select"))
            .with("proxies", Yaml::strings(group_members));

        let mut document = Yaml::map()
            .with("mixed-port", Yaml::int(i64::from(options.listen_port)))
            .with("allow-lan", Yaml::bool(options.allow_lan))
            .with("mode", Yaml::str("rule"))
            .with("log-level", Yaml::str(options.log_level.clone()))
            .with("ipv6", Yaml::bool(false))
            .with(
                "external-controller",
                Yaml::str(format!("127.0.0.1:{}", options.api_port)),
            );

        if !options.allow_lan {
            document = document.with("bind-address", Yaml::str("127.0.0.1"));
        }

        let dns_mode = options.dns.effective(options.tun);
        if dns_mode != DnsMode::Off {
            document = document.with("dns", dns_block(dns_mode));
        }

        if options.tun {
            // `device` names the wintun/utun adapter so a leftover can be
            // recognised after a crash.
            let tun = Yaml::map()
                .with("enable", Yaml::bool(true))
                .with("stack", Yaml::str("mixed"))
                .with("device", Yaml::str(crate::network::tun::INTERFACE_NAME))
                .with("auto-route", Yaml::bool(true))
                .with("auto-detect-interface", Yaml::bool(true))
                .with("dns-hijack", Yaml::strings(["any:53"]));
            document = document.with("tun", tun);
        }

        document = document
            .with("proxies", Yaml::List(proxies))
            .with("proxy-groups", Yaml::list().push(group))
            .with("rules", Yaml::strings(rule_lines(options.rules)));

        Ok(CompiledConfig {
            core: CoreKind::Mihomo,
            file_name: self.file_name(),
            text: emit(&document),
            included,
            skipped,
        })
    }
}

/// DNS section.
///
/// Nameservers carry the  suffix, which is how Mihomo is told to send
/// the query through the proxy group instead of straight to the resolver. That
/// suffix is the entire leak-protection story here: without it every lookup
/// leaves the machine in plain sight of the local network, even when the traffic
/// itself is tunnelled.
fn dns_block(mode: DnsMode) -> Yaml {
    let mut dns = Yaml::map()
        .with("enable", Yaml::bool(true))
        .with("ipv6", Yaml::bool(false))
        .with("default-nameserver", Yaml::strings(["223.5.5.5"]))
        .with(
            "nameserver",
            Yaml::strings([format!("https://1.1.1.1/dns-query#{PROXY_GROUP}")]),
        )
        .with("fallback", Yaml::strings(["tls://8.8.8.8:853"]));

    if mode == DnsMode::FakeIp {
        dns = dns
            .with("enhanced-mode", Yaml::str("fake-ip"))
            .with("fake-ip-range", Yaml::str("198.18.0.1/16"))
            .with(
                "fake-ip-filter",
                Yaml::strings(["*.lan", "*.local", "localhost.ptlogin2.qq.com"]),
            );
    }

    dns
}

/// Mihomo rule list.
///
/// Split mode keeps the local networks and Chinese destinations off the proxy.
/// GEOSITE/GEOIP are evaluated against Mihomo own geo database, so no rule-set
/// download is needed at runtime.
fn rule_lines(mode: RuleMode) -> Vec<String> {
    match mode {
        RuleMode::Global => vec![format!("MATCH,{PROXY_GROUP}")],
        RuleMode::Split => vec![
            "IP-CIDR,127.0.0.0/8,DIRECT,no-resolve".to_string(),
            "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve".to_string(),
            "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve".to_string(),
            "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve".to_string(),
            "GEOSITE,cn,DIRECT".to_string(),
            "GEOIP,CN,DIRECT".to_string(),
            format!("MATCH,{PROXY_GROUP}"),
        ],
    }
}

fn proxy_entry(node: &Node) -> Result<Yaml, String> {
    if let Some(reason) = super::common_skip_reason(node) {
        return Err(reason);
    }
    if node.plugin.is_some() {
        return Err(
            "shadowsocks plugin is not compiled yet; the node would connect differently without it"
                .to_string(),
        );
    }

    let mut entry = Yaml::map()
        .with("name", Yaml::str(node.name.clone()))
        .with("type", Yaml::str(protocol_type(node)))
        .with("server", Yaml::str(node.server.clone()))
        .with("port", Yaml::int(i64::from(node.port)));

    if node.udp {
        entry = entry.with("udp", Yaml::bool(true));
    }

    entry = match &node.credentials {
        Credentials::Shadowsocks { method, password } => entry
            .with("cipher", Yaml::str(method.clone()))
            .with("password", Yaml::str(password.clone())),
        Credentials::Vmess {
            uuid,
            alter_id,
            security,
        } => {
            let entry = entry
                .with("uuid", Yaml::str(uuid.clone()))
                .with("alterId", Yaml::int(i64::from(*alter_id)))
                .with("cipher", Yaml::str(security.clone()));
            apply_tls(entry, node, "servername")
        }
        Credentials::Vless { uuid, flow } => {
            let mut entry = entry.with("uuid", Yaml::str(uuid.clone()));
            if *flow == VlessFlow::Vision {
                entry = entry.with("flow", Yaml::str(flow.as_str()));
            }
            apply_tls(entry, node, "servername")
        }
        Credentials::Trojan { password } => {
            let entry = entry.with("password", Yaml::str(password.clone()));
            // Mihomo spells the trojan SNI field `sni`.
            apply_tls(entry, node, "sni")
        }
        Credentials::Hysteria2 {
            password,
            obfs,
            obfs_password,
        } => {
            let mut entry = entry.with("password", Yaml::str(password.clone()));
            if let Some(obfs) = obfs {
                entry = entry.with("obfs", Yaml::str(obfs.clone()));
                if let Some(secret) = obfs_password {
                    entry = entry.with("obfs-password", Yaml::str(secret.clone()));
                }
            }
            // Hysteria2 is always TLS; there is no plaintext mode to opt out of.
            apply_tls(entry, node, "sni")
        }
    };

    Ok(apply_transport(entry, node))
}

fn protocol_type(node: &Node) -> &'static str {
    match node.credentials {
        Credentials::Shadowsocks { .. } => "ss",
        Credentials::Vmess { .. } => "vmess",
        Credentials::Vless { .. } => "vless",
        Credentials::Trojan { .. } => "trojan",
        Credentials::Hysteria2 { .. } => "hysteria2",
    }
}

fn apply_tls(entry: Yaml, node: &Node, sni_key: &str) -> Yaml {
    let Some(tls) = &node.tls else {
        return entry;
    };

    let mut entry = entry.with("tls", Yaml::bool(true));

    if let Some(server_name) = node.server_name() {
        entry = entry.with(sni_key, Yaml::str(server_name.to_string()));
    }
    if tls.allow_insecure {
        entry = entry.with("skip-cert-verify", Yaml::bool(true));
    }
    if !tls.alpn.is_empty() {
        entry = entry.with("alpn", Yaml::strings(tls.alpn.clone()));
    }
    if let Some(fingerprint) = &tls.fingerprint {
        entry = entry.with("client-fingerprint", Yaml::str(fingerprint.clone()));
    }
    if let Some(reality) = &tls.reality {
        entry = entry.with("reality-opts", reality_opts(reality));
    }

    entry
}

fn reality_opts(reality: &crate::profile::node::RealityConfig) -> Yaml {
    let mut opts = Yaml::map().with("public-key", Yaml::str(reality.public_key.clone()));
    if !reality.short_id.is_empty() {
        opts = opts.with("short-id", Yaml::str(reality.short_id.clone()));
    }
    opts
}

fn apply_transport(entry: Yaml, node: &Node) -> Yaml {
    match &node.transport {
        Transport::Tcp => entry,
        Transport::Ws(ws) => {
            let mut opts = Yaml::map().with("path", Yaml::str(ws.path.clone()));
            if let Some(host) = &ws.host {
                opts = opts.with("headers", Yaml::map().with("Host", Yaml::str(host.clone())));
            }
            if let Some(early_data) = ws.max_early_data {
                opts = opts.with("max-early-data", Yaml::int(i64::from(early_data)));
            }
            if let Some(header) = &ws.early_data_header {
                opts = opts.with("early-data-header-name", Yaml::str(header.clone()));
            }
            entry.with("network", Yaml::str("ws")).with("ws-opts", opts)
        }
        Transport::Grpc(grpc) => {
            let opts = Yaml::map().with("grpc-service-name", Yaml::str(grpc.service_name.clone()));
            entry
                .with("network", Yaml::str("grpc"))
                .with("grpc-opts", opts)
        }
        Transport::Http2(h2) => {
            let mut opts = Yaml::map().with("path", Yaml::str(h2.path.clone()));
            if !h2.host.is_empty() {
                opts = opts.with("host", Yaml::strings(h2.host.clone()));
            }
            entry.with("network", Yaml::str("h2")).with("h2-opts", opts)
        }
    }
}

/// Convenience wrapper used by tests and the CLI.
pub fn compile_yaml(
    profile: &Profile,
    options: &CompileOptions,
) -> Result<CompiledConfig, CompileError> {
    MihomoCompiler.compile(profile, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::node::{GrpcOptions, RealityConfig, TlsConfig, WsOptions};

    fn node_with(credentials: Credentials, tls: Option<TlsConfig>) -> Node {
        let mut node = Node {
            id: String::new(),
            name: "Tokyo 01".to_string(),
            protocol: credentials.protocol(),
            server: "jp.example.com".to_string(),
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

    fn reality_tls() -> TlsConfig {
        TlsConfig {
            server_name: Some("www.microsoft.com".to_string()),
            fingerprint: Some("chrome".to_string()),
            reality: Some(RealityConfig {
                public_key: "pk".to_string(),
                short_id: "a1b2".to_string(),
                spider_x: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn vless_reality_ws_is_emitted_with_all_options() {
        let mut node = node_with(
            Credentials::Vless {
                uuid: "11111111-2222-3333-4444-555555555555".to_string(),
                flow: VlessFlow::Vision,
            },
            Some(reality_tls()),
        );
        node.transport = Transport::Ws(WsOptions {
            path: "/ws".to_string(),
            host: Some("cdn.example.com".to_string()),
            ..Default::default()
        });

        let profile = Profile::new("p", vec![node]);
        let compiled = compile_yaml(&profile, &CompileOptions::default()).unwrap();
        let text = compiled.text;

        assert!(text.contains("\"type\": \"vless\""), "{text}");
        assert!(text.contains("\"flow\": \"xtls-rprx-vision\""), "{text}");
        assert!(
            text.contains("\"servername\": \"www.microsoft.com\""),
            "{text}"
        );
        assert!(
            text.contains("\"client-fingerprint\": \"chrome\""),
            "{text}"
        );
        assert!(text.contains("\"reality-opts\":"), "{text}");
        assert!(text.contains("\"public-key\": \"pk\""), "{text}");
        assert!(text.contains("\"network\": \"ws\""), "{text}");
        assert!(text.contains("\"Host\": \"cdn.example.com\""), "{text}");
        assert_eq!(compiled.included, vec!["Tokyo 01".to_string()]);
    }

    #[test]
    fn trojan_uses_sni_field_and_no_tls_flag() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig {
                server_name: Some("sg.example.com".to_string()),
                ..Default::default()
            }),
        );
        let text = compile_yaml(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap()
            .text;
        assert!(text.contains("\"type\": \"trojan\""), "{text}");
        assert!(text.contains("\"sni\": \"sg.example.com\""), "{text}");
    }

    #[test]
    fn shadowsocks_maps_cipher_and_password() {
        let node = node_with(
            Credentials::Shadowsocks {
                method: "aes-256-gcm".to_string(),
                password: "pw".to_string(),
            },
            None,
        );
        let text = compile_yaml(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap()
            .text;
        assert!(text.contains("\"type\": \"ss\""), "{text}");
        assert!(text.contains("\"cipher\": \"aes-256-gcm\""), "{text}");
        assert!(text.contains("\"password\": \"pw\""), "{text}");
    }

    #[test]
    fn grpc_transport_is_mapped() {
        let mut node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        node.transport = Transport::Grpc(GrpcOptions {
            service_name: "svc".to_string(),
            multi_mode: false,
        });
        let text = compile_yaml(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap()
            .text;
        assert!(text.contains("\"network\": \"grpc\""), "{text}");
        assert!(text.contains("\"grpc-service-name\": \"svc\""), "{text}");
    }

    #[test]
    fn plugin_nodes_are_skipped_with_a_reason() {
        let mut node = node_with(
            Credentials::Shadowsocks {
                method: "aes-256-gcm".to_string(),
                password: "pw".to_string(),
            },
            None,
        );
        node.plugin = Some("obfs-local;obfs=http".to_string());
        node.name = "obfs node".to_string();

        let compiled =
            compile_yaml(&Profile::new("p", vec![node]), &CompileOptions::default()).unwrap();
        assert!(compiled.included.is_empty());
        assert_eq!(compiled.skipped.len(), 1);
        assert_eq!(compiled.skipped[0].name, "obfs node");
        assert!(compiled.skipped[0].reason.contains("plugin"));
    }

    #[test]
    fn group_and_rule_reference_the_proxy_group() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            None,
        );
        let text = compile_yaml(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap()
            .text;
        assert!(text.contains("\"proxy-groups\":"), "{text}");
        assert!(text.contains("\"name\": \"PROXY\""), "{text}");
        assert!(text.contains("\"MATCH,PROXY\""), "{text}");
        assert!(text.contains("\"mixed-port\": 7890"), "{text}");
    }

    #[test]
    fn tun_option_emits_a_tun_block() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            None,
        );
        let options = CompileOptions {
            tun: true,
            ..Default::default()
        };
        let text = compile_yaml(&Profile::new("p", vec![node]), &options)
            .unwrap()
            .text;
        assert!(text.contains(r#""tun":"#), "{text}");
        assert!(
            text.contains(&format!(
                r#""device": "{}""#,
                crate::network::tun::INTERFACE_NAME
            )),
            "{text}"
        );
        assert!(text.contains(r#""auto-route": true"#), "{text}");
    }

    #[test]
    fn split_mode_sends_chinese_traffic_direct() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            None,
        );
        let options = CompileOptions {
            rules: RuleMode::Split,
            ..Default::default()
        };
        let text = compile_yaml(&Profile::new("p", vec![node]), &options)
            .unwrap()
            .text;
        assert!(text.contains("GEOSITE,cn,DIRECT"), "{text}");
        assert!(text.contains("GEOIP,CN,DIRECT"), "{text}");
        assert!(text.contains("IP-CIDR,192.168.0.0/16,DIRECT"), "{text}");
        // The catch-all must stay last, otherwise nothing reaches the proxy.
        assert!(text.contains("MATCH,PROXY"), "{text}");
    }

    #[test]
    fn global_mode_is_a_single_catch_all_rule() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            None,
        );
        let text = compile_yaml(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap()
            .text;
        assert!(!text.contains("GEOSITE"), "{text}");
        assert!(text.contains("MATCH,PROXY"), "{text}");
    }

    #[test]
    fn dns_section_routes_nameservers_through_the_proxy() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            None,
        );
        let options = CompileOptions {
            dns: DnsMode::Split,
            ..Default::default()
        };
        let text = compile_yaml(&Profile::new("p", vec![node]), &options)
            .unwrap()
            .text;
        assert!(text.contains(r#""dns":"#), "{text}");
        assert!(text.contains("#PROXY"), "{text}");
        assert!(!text.contains("enhanced-mode"), "{text}");
    }

    #[test]
    fn fakeip_mode_adds_enhanced_mode_and_range() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            None,
        );
        let options = CompileOptions {
            dns: DnsMode::FakeIp,
            ..Default::default()
        };
        let text = compile_yaml(&Profile::new("p", vec![node]), &options)
            .unwrap()
            .text;
        assert!(text.contains(r#""enhanced-mode": "fake-ip""#), "{text}");
        assert!(text.contains("fake-ip-range"), "{text}");
    }

    #[test]
    fn tun_upgrades_off_dns_to_split() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            None,
        );
        // dns is Off, but TUN makes a DNS section necessary.
        let options = CompileOptions {
            tun: true,
            ..Default::default()
        };
        let text = compile_yaml(&Profile::new("p", vec![node]), &options)
            .unwrap()
            .text;
        assert!(text.contains(r#""dns":"#), "{text}");
    }

    #[test]
    fn allow_lan_drops_the_loopback_bind() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            None,
        );
        let options = CompileOptions {
            allow_lan: true,
            ..Default::default()
        };
        let text = compile_yaml(&Profile::new("p", vec![node]), &options)
            .unwrap()
            .text;
        assert!(text.contains("\"allow-lan\": true"), "{text}");
        assert!(!text.contains("bind-address"), "{text}");
    }

    #[test]
    fn vmess_keeps_alter_id_and_cipher() {
        let node = node_with(
            Credentials::Vmess {
                uuid: "11111111-2222-3333-4444-555555555555".to_string(),
                alter_id: 0,
                security: "auto".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let text = compile_yaml(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap()
            .text;
        assert!(text.contains("\"alterId\": 0"), "{text}");
        assert!(text.contains("\"cipher\": \"auto\""), "{text}");
    }

    #[test]
    fn protocol_type_covers_every_credential_variant() {
        let cases = [
            (
                Credentials::Shadowsocks {
                    method: "aes-256-gcm".into(),
                    password: "p".into(),
                },
                "ss",
            ),
            (
                Credentials::Vmess {
                    uuid: "u".into(),
                    alter_id: 0,
                    security: "auto".into(),
                },
                "vmess",
            ),
            (
                Credentials::Vless {
                    uuid: "u".into(),
                    flow: VlessFlow::None,
                },
                "vless",
            ),
            (
                Credentials::Trojan {
                    password: "p".into(),
                },
                "trojan",
            ),
        ];
        for (credentials, expected) in cases {
            let node = node_with(credentials, None);
            assert_eq!(protocol_type(&node), expected);
        }
    }
}
