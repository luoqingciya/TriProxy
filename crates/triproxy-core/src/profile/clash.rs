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

//! Clash / Clash.Meta configuration import.
//!
//! Parses `proxies`, `proxy-groups`, `rule-providers`, and `rules` so that
//! subscriptions with provider-defined routing rules carry them cleanly into
//! TriProxy's unified `Profile` model. Unsupported proxy types are reported as
//! warnings rather than dropping the rest of the config.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;

use crate::compiler::{
    rules::{valid_cidr, valid_port_spec},
    CustomRule, RuleKind, RuleSet, RuleSetBehavior, RuleSetFormat, RuleTarget,
};

use super::node::{
    Credentials, GrpcOptions, Http2Options, Node, RealityConfig, TlsConfig, Transport, VlessFlow,
    WsOptions,
};
use super::validate::{Diagnostic, Severity};

#[derive(Debug, Deserialize)]
struct ClashConfig {
    #[serde(default)]
    proxies: Vec<ClashProxy>,
    #[serde(rename = "proxy-groups", default)]
    proxy_groups: Vec<ClashGroup>,
    #[serde(rename = "rule-providers", default)]
    rule_providers: BTreeMap<String, ClashRuleProvider>,
    #[serde(default)]
    rules: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ClashGroup {
    name: String,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    proxies: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ClashRuleProvider {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    behavior: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// Complete output of parsing a Clash / Mihomo YAML configuration.
#[derive(Debug, Clone)]
pub struct ClashParsed {
    pub nodes: Vec<Node>,
    pub groups: Vec<super::ProxyGroup>,
    pub rules: Vec<CustomRule>,
    pub rule_sets: Vec<RuleSet>,
    pub default_target: Option<RuleTarget>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Deserialize)]
struct ClashProxy {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    server: String,
    #[serde(default)]
    port: u16,

    // Shadowsocks
    cipher: Option<String>,
    password: Option<String>,
    plugin: Option<String>,
    #[serde(rename = "plugin-opts")]
    plugin_opts: Option<PluginOpts>,

    // VMess / VLESS
    uuid: Option<String>,
    #[serde(rename = "alterId")]
    alter_id: Option<u32>,
    #[serde(rename = "security")]
    vmess_security: Option<String>,
    flow: Option<String>,

    // Transport
    network: Option<String>,
    #[serde(rename = "ws-opts")]
    ws_opts: Option<WsOpts>,
    #[serde(rename = "grpc-opts")]
    grpc_opts: Option<GrpcOpts>,
    #[serde(rename = "h2-opts")]
    h2_opts: Option<H2Opts>,

    // TLS
    tls: Option<bool>,
    #[serde(rename = "servername")]
    servername: Option<String>,
    sni: Option<String>,
    #[serde(rename = "skip-cert-verify")]
    skip_cert_verify: Option<bool>,
    alpn: Option<Vec<String>>,
    #[serde(rename = "client-fingerprint")]
    client_fingerprint: Option<String>,
    #[serde(rename = "reality-opts")]
    reality_opts: Option<RealityOpts>,

    // TUIC
    #[serde(rename = "congestion-controller")]
    congestion_controller: Option<String>,
    #[serde(rename = "udp-relay-mode")]
    udp_relay_mode: Option<String>,

    // Hysteria2
    obfs: Option<String>,
    #[serde(rename = "obfs-password")]
    obfs_password: Option<String>,

    udp: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct WsOpts {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    headers: Option<WsHeaders>,
    #[serde(rename = "max-early-data")]
    max_early_data: Option<u32>,
    #[serde(rename = "early-data-header-name")]
    early_data_header: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct WsHeaders {
    #[serde(rename = "Host")]
    host: Option<String>,
    #[serde(rename = "host")]
    host_lower: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct GrpcOpts {
    #[serde(rename = "grpc-service-name")]
    service_name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct H2Opts {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    host: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
struct RealityOpts {
    #[serde(rename = "public-key")]
    public_key: Option<String>,
    #[serde(rename = "short-id")]
    short_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct PluginOpts {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    tls: Option<bool>,
}

/// Parse a Clash configuration into nodes, routing rules, rule providers, and diagnostics.
pub fn parse_full(text: &str) -> ClashParsed {
    let mut diagnostics = Vec::new();

    let config: ClashConfig = match serde_norway::from_str(text) {
        Ok(config) => config,
        Err(error) => {
            diagnostics.push(Diagnostic::new(
                Severity::Error,
                "clash.invalid_yaml",
                format!("cannot parse the configuration: {error}"),
            ));
            return ClashParsed {
                nodes: Vec::new(),
                groups: Vec::new(),
                rules: Vec::new(),
                rule_sets: Vec::new(),
                default_target: None,
                diagnostics,
            };
        }
    };

    if config.proxies.is_empty() {
        diagnostics.push(Diagnostic::new(
            Severity::Error,
            "clash.no_proxies",
            "the configuration has no proxies",
        ));
        return ClashParsed {
            nodes: Vec::new(),
            groups: Vec::new(),
            rules: Vec::new(),
            rule_sets: Vec::new(),
            default_target: None,
            diagnostics,
        };
    }

    let mut nodes = Vec::with_capacity(config.proxies.len());
    for proxy in config.proxies {
        match convert(proxy) {
            Ok(node) => nodes.push(node),
            Err(reason) => diagnostics.push(Diagnostic::new(
                Severity::Warning,
                "clash.unsupported_proxy",
                reason,
            )),
        }
    }

    let mut groups: Vec<super::ProxyGroup> = Vec::with_capacity(config.proxy_groups.len());
    let mut group_map: HashMap<String, Vec<String>> = HashMap::new();
    for group in config.proxy_groups {
        let name = group.name.trim().to_string();
        if name.is_empty() || group.proxies.is_empty() {
            continue;
        }
        let kind = match group
            .kind
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("url-test") => super::GroupKind::UrlTest,
            Some("fallback") => super::GroupKind::Fallback,
            Some("load-balance") => super::GroupKind::LoadBalance,
            _ => super::GroupKind::Select,
        };
        group_map.insert(name.clone(), group.proxies.clone());
        groups.push(super::ProxyGroup {
            name,
            kind,
            members: group.proxies,
        });
    }

    let (rules, rule_sets, default_target) =
        parse_clash_rules(&config.rules, &config.rule_providers, &group_map);

    ClashParsed {
        nodes,
        groups,
        rules,
        rule_sets,
        default_target,
        diagnostics,
    }
}

/// Parse a Clash configuration into nodes plus diagnostics.
pub fn parse(text: &str) -> (Vec<Node>, Vec<Diagnostic>) {
    let parsed = parse_full(text);
    (parsed.nodes, parsed.diagnostics)
}

fn resolve_target(name: &str, groups: &HashMap<String, Vec<String>>, depth: u8) -> RuleTarget {
    let trimmed = name.trim();
    let upper = trimmed.to_ascii_uppercase();
    match upper.as_str() {
        "DIRECT" => return RuleTarget::Direct,
        "REJECT" | "REJECT-DROP" => return RuleTarget::Reject,
        "PROXY" => return RuleTarget::Proxy,
        _ => {}
    }

    if depth >= 4 {
        return RuleTarget::Proxy;
    }

    if let Some(members) = groups.get(trimmed) {
        if let Some(first) = members.first() {
            let first_trim = first.trim();
            let first_upper = first_trim.to_ascii_uppercase();
            if first_upper == "DIRECT" {
                return RuleTarget::Direct;
            }
            if matches!(first_upper.as_str(), "REJECT" | "REJECT-DROP") {
                return RuleTarget::Reject;
            }
            if groups.contains_key(first_trim) {
                return resolve_target(first_trim, groups, depth + 1);
            }
            return RuleTarget::Proxy;
        }
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("direct") || trimmed.contains("直连") {
        RuleTarget::Direct
    } else if lower.contains("reject")
        || lower.contains("adblock")
        || trimmed.contains("拦截")
        || trimmed.contains("净化")
        || trimmed.contains("广告")
    {
        RuleTarget::Reject
    } else {
        RuleTarget::Proxy
    }
}

fn parse_clash_rules(
    raw_rules: &[String],
    providers: &BTreeMap<String, ClashRuleProvider>,
    groups: &HashMap<String, Vec<String>>,
) -> (Vec<CustomRule>, Vec<RuleSet>, Option<RuleTarget>) {
    let mut rules = Vec::new();
    let mut rule_sets = Vec::new();
    let mut seen_provider_names: HashSet<String> = HashSet::new();
    let mut default_target = None;

    for raw in raw_rules {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }

        let fields: Vec<&str> = trimmed.split(',').map(str::trim).collect();
        if fields.len() < 2 {
            continue;
        }

        let kind_upper = fields[0].to_ascii_uppercase();
        if matches!(kind_upper.as_str(), "MATCH" | "FINAL") {
            default_target = Some(resolve_target(fields[1], groups, 0));
            continue;
        }

        if fields.len() < 3 {
            continue;
        }

        let raw_target = fields[2].trim();
        let target = resolve_target(raw_target, groups, 0);
        let group = (!matches!(
            raw_target.to_ascii_uppercase().as_str(),
            "DIRECT" | "REJECT" | "REJECT-DROP" | "PROXY"
        ) && groups.contains_key(raw_target))
        .then(|| raw_target.to_string());
        let value = fields[1];
        if value.is_empty() {
            continue;
        }

        if kind_upper == "RULE-SET" {
            if let Some(provider) = providers.get(value) {
                if let Some(set) = build_clash_rule_set(
                    value,
                    provider,
                    target,
                    group.clone(),
                    &mut seen_provider_names,
                ) {
                    rule_sets.push(set);
                }
            }
            continue;
        }

        let kind = match kind_upper.as_str() {
            "DOMAIN" => RuleKind::Domain,
            "DOMAIN-SUFFIX" => RuleKind::DomainSuffix,
            "DOMAIN-KEYWORD" => RuleKind::DomainKeyword,
            "IP-CIDR" | "IP-CIDR6" => {
                if !valid_cidr(value) {
                    continue;
                }
                RuleKind::IpCidr
            }
            "GEOSITE" => RuleKind::GeoSite,
            "GEOIP" => RuleKind::GeoIp,
            "PROCESS-NAME" => RuleKind::ProcessName,
            "DST-PORT" => {
                if !valid_port_spec(value) {
                    continue;
                }
                RuleKind::DstPort
            }
            _ => continue,
        };

        rules.push(CustomRule {
            kind,
            value: value.to_string(),
            target,
            group,
        });
    }

    (rules, rule_sets, default_target)
}

fn build_clash_rule_set(
    provider_key: &str,
    provider: &ClashRuleProvider,
    target: RuleTarget,
    group: Option<String>,
    seen_names: &mut HashSet<String>,
) -> Option<RuleSet> {
    if provider
        .kind
        .as_deref()
        .is_some_and(|kind| !kind.eq_ignore_ascii_case("http"))
    {
        return None;
    }

    let url = provider.url.as_deref()?.trim();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return None;
    }

    let behavior = match provider
        .behavior
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("domain") => RuleSetBehavior::Domain,
        Some("ipcidr") => RuleSetBehavior::IpCidr,
        _ => RuleSetBehavior::Classical,
    };

    let url_path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    let format = match provider
        .format
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mrs") | Some("yaml") | Some("yml") => RuleSetFormat::Clash,
        Some("srs") => RuleSetFormat::SingBox,
        _ if url_path.ends_with(".srs") => RuleSetFormat::SingBox,
        _ => RuleSetFormat::Clash,
    };

    let base: String = provider_key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let base = if base.is_empty() {
        "sub-rules".to_string()
    } else {
        base
    };
    let mut name = base.clone();
    if !seen_names.insert(name.clone()) {
        for suffix in 2.. {
            let candidate = format!("{base}-{suffix}");
            if seen_names.insert(candidate.clone()) {
                name = candidate;
                break;
            }
        }
    }

    Some(RuleSet {
        name,
        url: url.to_string(),
        target,
        behavior,
        format,
        group,
    })
}

fn convert(proxy: ClashProxy) -> Result<Node, String> {
    let name = proxy.name.clone();

    if proxy.server.trim().is_empty() {
        return Err(format!("`{name}`: no server"));
    }
    if proxy.port == 0 {
        return Err(format!("`{name}`: no port"));
    }

    let credentials = match proxy.kind.as_str() {
        "ss" => {
            let method = proxy
                .cipher
                .clone()
                .ok_or_else(|| format!("`{name}`: shadowsocks without a cipher"))?;
            let password = proxy
                .password
                .clone()
                .ok_or_else(|| format!("`{name}`: shadowsocks without a password"))?;
            Credentials::Shadowsocks { method, password }
        }
        "vmess" => Credentials::Vmess {
            uuid: proxy
                .uuid
                .clone()
                .ok_or_else(|| format!("`{name}`: vmess without a uuid"))?,
            alter_id: proxy.alter_id.unwrap_or(0),
            security: proxy
                .vmess_security
                .clone()
                .unwrap_or_else(|| "auto".to_string()),
        },
        "vless" => {
            let (flow, unknown) = VlessFlow::parse(proxy.flow.as_deref().unwrap_or(""));
            if let Some(unknown) = unknown {
                // Guessing a flow silently would change how traffic is framed.
                return Err(format!("`{name}`: unsupported vless flow `{unknown}`"));
            }
            Credentials::Vless {
                uuid: proxy
                    .uuid
                    .clone()
                    .ok_or_else(|| format!("`{name}`: vless without a uuid"))?,
                flow,
            }
        }
        "trojan" => Credentials::Trojan {
            password: proxy
                .password
                .clone()
                .ok_or_else(|| format!("`{name}`: trojan without a password"))?,
        },
        "tuic" => Credentials::Tuic {
            uuid: proxy
                .uuid
                .clone()
                .ok_or_else(|| format!("`{name}`: tuic without a uuid"))?,
            password: proxy
                .password
                .clone()
                .ok_or_else(|| format!("`{name}`: tuic without a password"))?,
            // Clash spells these with hyphens, the link format with underscores.
            congestion_control: proxy.congestion_controller.clone(),
            udp_relay_mode: proxy.udp_relay_mode.clone(),
        },
        "hysteria2" => Credentials::Hysteria2 {
            password: proxy
                .password
                .clone()
                .ok_or_else(|| format!("`{name}`: hysteria2 without a password"))?,
            obfs: proxy.obfs.clone(),
            obfs_password: proxy.obfs_password.clone(),
        },
        other => return Err(format!("`{name}`: proxy type `{other}` is not supported")),
    };

    // Trojan and Hysteria2 are TLS-only in practice; Clash configs often omit
    // the key for both, and plaintext is not a thing either protocol has.
    let tls_wanted = proxy
        .tls
        .unwrap_or(matches!(proxy.kind.as_str(), "trojan" | "hysteria2"))
        || proxy.reality_opts.is_some();

    let tls = tls_wanted.then(|| TlsConfig {
        server_name: proxy.servername.clone().or_else(|| proxy.sni.clone()),
        allow_insecure: proxy.skip_cert_verify.unwrap_or(false),
        alpn: proxy.alpn.clone().unwrap_or_default(),
        fingerprint: proxy.client_fingerprint.clone(),
        reality: proxy.reality_opts.as_ref().map(|reality| RealityConfig {
            public_key: reality.public_key.clone().unwrap_or_default(),
            short_id: reality.short_id.clone().unwrap_or_default(),
            spider_x: None,
        }),
    });

    let transport = match proxy.network.as_deref().unwrap_or("tcp") {
        "tcp" | "" => Transport::Tcp,
        "ws" => {
            let options = proxy.ws_opts.unwrap_or_default();
            Transport::Ws(WsOptions {
                path: options.path.unwrap_or_else(|| "/".to_string()),
                host: options.headers.as_ref().and_then(|headers| {
                    headers.host.clone().or_else(|| headers.host_lower.clone())
                }),
                max_early_data: options.max_early_data,
                early_data_header: options.early_data_header,
            })
        }
        "grpc" => Transport::Grpc(GrpcOptions {
            service_name: proxy
                .grpc_opts
                .and_then(|options| options.service_name)
                .unwrap_or_default(),
            multi_mode: false,
        }),
        "h2" | "http" => {
            let options = proxy.h2_opts.unwrap_or_default();
            Transport::Http2(Http2Options {
                path: options.path.unwrap_or_else(|| "/".to_string()),
                host: options.host.unwrap_or_default(),
            })
        }
        other => return Err(format!("`{name}`: transport `{other}` is not supported")),
    };

    let plugin = convert_plugin(&name, proxy.plugin.as_deref(), proxy.plugin_opts.clone())?;

    let mut node = Node {
        id: String::new(),
        name,
        protocol: credentials.protocol(),
        server: proxy.server,
        port: proxy.port,
        credentials,
        transport,
        tls,
        udp: proxy.udp.unwrap_or(true),
        plugin,
    };
    node.refresh_id();
    Ok(node)
}

/// Rebuild the SIP002 plugin string from Clash's structured form.
///
/// One representation reaches the compilers regardless of whether the node came
/// from a share link or from YAML; keeping two shapes would mean two code paths
/// in three compilers.
fn convert_plugin(
    name: &str,
    plugin: Option<&str>,
    options: Option<PluginOpts>,
) -> Result<Option<String>, String> {
    let Some(plugin) = plugin else {
        return Ok(None);
    };
    let options = options.unwrap_or_default();

    match plugin {
        "obfs" => Ok(Some(format!(
            "obfs-local;obfs={};obfs-host={}",
            options.mode.unwrap_or_else(|| "http".to_string()),
            options.host.unwrap_or_default()
        ))),
        "v2ray-plugin" => {
            let mut parts = vec![
                "v2ray-plugin".to_string(),
                format!(
                    "mode={}",
                    options.mode.unwrap_or_else(|| "websocket".to_string())
                ),
            ];
            if let Some(host) = options.host {
                parts.push(format!("host={host}"));
            }
            if let Some(path) = options.path {
                parts.push(format!("path={path}"));
            }
            if options.tls.unwrap_or(false) {
                parts.push("tls".to_string());
            }
            Ok(Some(parts.join(";")))
        }
        other => Err(format!("`{name}`: plugin `{other}` is not supported")),
    }
}

/// Does this text look like a Clash configuration rather than a URI list?
pub fn looks_like_clash(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_end();
        line == "proxies:" || line.starts_with("proxies: ") || line.starts_with("proxies: [")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
port: 7890
proxies:
  - name: "JP Reality"
    type: vless
    server: jp.example.com
    port: 443
    uuid: 11111111-2222-3333-4444-555555555555
    network: tcp
    tls: true
    servername: www.microsoft.com
    client-fingerprint: chrome
    reality-opts:
      public-key: abc123
      short-id: "01"
  - name: "SG WS"
    type: vmess
    server: sg.example.com
    port: 443
    uuid: 66666666-7777-8888-9999-000000000000
    alterId: 0
    cipher: auto
    network: ws
    tls: true
    ws-opts:
      path: /ray
      headers:
        Host: sg.example.com
  - name: "SS"
    type: ss
    server: ss.example.com
    port: 8388
    cipher: aes-256-gcm
    password: secret
    udp: true
  - name: "Socks"
    type: socks5
    server: socks.example.com
    port: 1080
  - name: "Hy2"
    type: hysteria2
    server: hy.example.com
    port: 443
    password: secret
    obfs: salamander
    obfs-password: obfspw
"#;

    #[test]
    fn imports_supported_proxies_and_reports_the_rest() {
        let (nodes, diagnostics) = parse(SAMPLE);

        assert_eq!(nodes.len(), 4, "only the socks5 entry should be skipped");
        assert_eq!(nodes[0].name, "JP Reality");
        assert!(nodes[0].uses_reality());
        assert_eq!(
            nodes[0].tls.as_ref().unwrap().fingerprint.as_deref(),
            Some("chrome")
        );

        assert_eq!(nodes[1].transport.label(), "ws");
        match &nodes[1].transport {
            Transport::Ws(options) => {
                assert_eq!(options.path, "/ray");
                assert_eq!(options.host.as_deref(), Some("sg.example.com"));
            }
            other => panic!("expected ws, got {other:?}"),
        }

        assert_eq!(nodes[2].protocol, crate::capability::Protocol::Shadowsocks);

        // Hysteria2 is supported now; it must come through with TLS attached,
        // since the protocol has no plaintext mode.
        assert_eq!(nodes[3].protocol, crate::capability::Protocol::Hysteria2);
        assert!(nodes[3].tls.is_some());

        let skipped: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.code == "clash.unsupported_proxy")
            .collect();
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].message.contains("socks5"),
            "{}",
            skipped[0].message
        );
    }

    #[test]
    fn trojan_without_an_explicit_tls_key_is_tls() {
        let text = r#"
proxies:
  - name: T
    type: trojan
    server: t.example.com
    port: 443
    password: pw
"#;
        let (nodes, _) = parse(text);
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].tls.is_some(), "trojan is TLS by default");
    }

    #[test]
    fn an_unknown_transport_is_reported_not_guessed() {
        let text = r#"
proxies:
  - name: X
    type: vmess
    server: x.example.com
    port: 443
    uuid: 11111111-2222-3333-4444-555555555555
    network: quic
"#;
        let (nodes, diagnostics) = parse(text);
        assert!(nodes.is_empty());
        assert!(diagnostics[0].message.contains("quic"));
    }

    #[test]
    fn detects_clash_configs_but_not_uri_lists() {
        assert!(looks_like_clash("proxies:\n  - name: a\n"));
        assert!(looks_like_clash("port: 7890\nproxies: []\n"));
        assert!(!looks_like_clash("trojan://pw@a.example.com:443#A\n"));
    }

    #[test]
    fn imports_clash_rules_groups_and_rule_providers() {
        let text = r#"
proxies:
  - name: NodeA
    type: trojan
    server: t.example.com
    port: 443
    password: pw
proxy-groups:
  - name: 🚀 节点选择
    type: select
    proxies:
      - NodeA
  - name: 🎯 全球直连
    type: select
    proxies:
      - DIRECT
  - name: 🛑 全球拦截
    type: select
    proxies:
      - REJECT
rule-providers:
  reject_ads:
    type: http
    behavior: domain
    format: mrs
    url: https://example.com/ads.mrs
rules:
  - RULE-SET,reject_ads,🛑 全球拦截
  - DOMAIN-SUFFIX,internal.corp,🎯 全球直连
  - DOMAIN,openai.com,🚀 节点选择
  - IP-CIDR,192.168.0.0/16,DIRECT,no-resolve
  - GEOIP,CN,🎯 全球直连
  - MATCH,🚀 节点选择
"#;
        let parsed = parse_full(text);
        assert_eq!(parsed.nodes.len(), 1);
        assert_eq!(parsed.rule_sets.len(), 1);
        assert_eq!(parsed.rule_sets[0].target, RuleTarget::Reject);
        assert_eq!(parsed.rules.len(), 4);
        assert_eq!(parsed.rules[0].kind, RuleKind::DomainSuffix);
        assert_eq!(parsed.rules[0].target, RuleTarget::Direct);
        assert_eq!(parsed.rules[1].target, RuleTarget::Proxy);
        assert_eq!(parsed.rules[2].kind, RuleKind::IpCidr);
        assert_eq!(parsed.rules[3].kind, RuleKind::GeoIp);
        assert_eq!(parsed.default_target, Some(RuleTarget::Proxy));
    }
}
