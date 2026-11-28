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

//! sing-box compiler: `Profile` → `config.json`.

use serde_json::{json, Map, Value};

use crate::core::CoreKind;
use crate::profile::node::{Credentials, Node, Transport, VlessFlow};
use crate::profile::Profile;

use super::{
    CompileError, CompileOptions, CompiledConfig, ConfigCompiler, CustomRule, DnsMode, RuleKind,
    RuleMode, RuleSet, RuleSetFormat, RuleTarget, SkippedNode, AUTO_GROUP, AUTO_TEST_INTERVAL_SECS,
    AUTO_TEST_URL, PROXY_GROUP,
};

pub struct SingBoxCompiler;

impl ConfigCompiler for SingBoxCompiler {
    fn core(&self) -> CoreKind {
        CoreKind::SingBox
    }

    fn file_name(&self) -> &'static str {
        "config.json"
    }

    fn compile(
        &self,
        profile: &Profile,
        options: &CompileOptions,
    ) -> Result<CompiledConfig, CompileError> {
        let mut outbounds: Vec<Value> = Vec::new();
        let mut included = Vec::new();
        let mut skipped: Vec<SkippedNode> = Vec::new();

        for node in &profile.nodes {
            match outbound(node) {
                Ok(value) => {
                    included.push(node.name.clone());
                    outbounds.push(value);
                }
                Err(reason) => skipped.push(SkippedNode {
                    name: node.name.clone(),
                    reason,
                }),
            }
        }

        // The automatic group, and the selector that offers it next to the
        // individual nodes. `urltest` probes every member and picks the fastest;
        // the selector's default decides what runs when the user has not chosen.
        outbounds.push(json!({
            "type": "urltest",
            "tag": AUTO_GROUP,
            "outbounds": included.clone(),
            "url": AUTO_TEST_URL,
            "interval": format!("{AUTO_TEST_INTERVAL_SECS}s"),
            "tolerance": 50,
        }));

        let mut members: Vec<Value> = Vec::new();
        let mut default = AUTO_GROUP.to_string();
        // An explicit pick outranks the automatic group, same rule as Mihomo.
        let picked = options
            .selected_name
            .as_ref()
            .filter(|name| included.contains(name));
        if let Some(name) = picked {
            members.push(json!(name));
            default = name.clone();
        }
        members.push(json!(AUTO_GROUP));
        members.extend(
            included
                .iter()
                .filter(|name| picked != Some(*name))
                .map(|name| json!(name)),
        );
        members.push(json!("direct"));

        outbounds.push(json!({
            "type": "selector",
            "tag": PROXY_GROUP,
            "outbounds": members,
            "default": default,
        }));
        outbounds.push(json!({ "type": "direct", "tag": "direct" }));

        let listen = if options.allow_lan {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };

        let mut inbounds = vec![json!({
            "type": "mixed",
            "tag": "mixed-in",
            "listen": listen,
            "listen_port": options.listen_port,
        })];

        if options.tun {
            // The interface is named so a leftover adapter can be identified
            // after a crash; the address is inside a range RFC 1918 reserves for
            // this kind of point-to-point link.
            #[allow(unused_mut)]
            let mut tun_inbound = json!({
                "type": "tun",
                "tag": "tun-in",
                "address": ["172.19.0.1/30"],
                "mtu": 9000,
                "auto_route": true,
                "strict_route": true,
                "stack": "mixed",
            });
            #[cfg(not(target_os = "macos"))]
            {
                tun_inbound["interface_name"] = json!(crate::network::tun::INTERFACE_NAME);
            }
            inbounds.push(tun_inbound);
        }

        let dns_mode = options.dns.effective(options.tun);
        let mirror = resolve_mirror(options.rule_set_mirror.as_deref());
        let mut required_geo_sets: Vec<(String, String)> = Vec::new();

        // Without a hijack rule the core would forward DNS packets as plain UDP
        // to the original resolver: slow, and the query itself still leaves the
        // tunnel. hijack-dns makes the core answer from its own dns section.
        let mut rules: Vec<Value> = Vec::new();
        if dns_mode != DnsMode::Off {
            rules.push(json!({ "protocol": "dns", "action": "hijack-dns" }));
        }

        // Custom rules first: they are the user's explicit intent and must be
        // able to override mode switches, subscription rules, and built-in presets.
        push_batched_rules(
            &mut rules,
            &options.custom_rules,
            mirror.as_deref(),
            &mut required_geo_sets,
        );

        if options.rules == RuleMode::Split {
            rules.push(json!({ "clash_mode": "direct", "action": "route", "outbound": "direct" }));
            rules.push(
                json!({ "clash_mode": "global", "action": "route", "outbound": PROXY_GROUP }),
            );
        }

        // Remote rule-sets (`.srs` binary only for sing-box).
        let mut seen_tags = std::collections::HashSet::new();
        let mut declared: Vec<Value> = Vec::new();

        let srs_sets: Vec<&RuleSet> = options
            .rule_sets
            .iter()
            .filter(|set| set.format == RuleSetFormat::SingBox)
            .collect();
        for set in &srs_sets {
            if seen_tags.insert(set.name.clone()) {
                rules.push(rule_set_match(&set.name, set.target));
                declared.push(remote_srs_declaration(&set.name, &set.url));
            }
        }

        let use_subscription_rules =
            options.rules == RuleMode::Split && profile.has_subscription_rules();

        let mut terminal_reject = false;
        let final_outbound = match options.rules {
            RuleMode::Global => PROXY_GROUP,
            RuleMode::Direct => "direct",
            RuleMode::Split => {
                if use_subscription_rules {
                    for set in &profile.rule_sets {
                        if set.format == RuleSetFormat::SingBox
                            && seen_tags.insert(set.name.clone())
                        {
                            rules.push(rule_set_match(&set.name, set.target));
                            declared.push(remote_srs_declaration(&set.name, &set.url));
                        }
                    }
                    push_batched_rules(
                        &mut rules,
                        &profile.rules,
                        mirror.as_deref(),
                        &mut required_geo_sets,
                    );
                    match profile.default_target.unwrap_or(RuleTarget::Proxy) {
                        RuleTarget::Proxy => PROXY_GROUP,
                        RuleTarget::Direct => "direct",
                        RuleTarget::Reject => {
                            terminal_reject = true;
                            PROXY_GROUP
                        }
                    }
                } else {
                    // Built-in split rules work immediately offline, backed up
                    // by remote SagerNet `.srs` rule-sets for full geo coverage.
                    let builtin = super::builtin_rules::builtin_split_rules();
                    push_batched_rules(
                        &mut rules,
                        &builtin,
                        mirror.as_deref(),
                        &mut required_geo_sets,
                    );
                    if seen_tags.insert("geosite-cn".to_string()) {
                        let url = apply_mirror(
                            "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-cn.srs",
                            mirror.as_deref(),
                        );
                        declared.push(remote_srs_declaration("geosite-cn", &url));
                    }
                    if seen_tags.insert("geoip-cn".to_string()) {
                        let url = apply_mirror(
                            "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-cn.srs",
                            mirror.as_deref(),
                        );
                        declared.push(remote_srs_declaration("geoip-cn", &url));
                    }
                    rules.push(json!({ "rule_set": "geosite-cn", "action": "route", "outbound": "direct" }));
                    rules.push(
                        json!({ "rule_set": "geoip-cn", "action": "route", "outbound": "direct" }),
                    );
                    PROXY_GROUP
                }
            }
        };

        for (tag, url) in required_geo_sets {
            if seen_tags.insert(tag.clone()) {
                declared.push(remote_srs_declaration(&tag, &url));
            }
        }

        if terminal_reject {
            rules.push(json!({ "action": "reject" }));
        }

        let mut route = json!({ "final": final_outbound });
        if !rules.is_empty() {
            route["rules"] = json!(rules);
        }
        if !declared.is_empty() {
            route["rule_set"] = json!(declared);
        }
        if dns_mode != DnsMode::Off {
            // sing-box 1.12+ refuses a config with a dns section but no bootstrap
            // resolver: something has to resolve the domain names used by the
            // outbounds themselves, before any rule can apply.
            route["default_domain_resolver"] = json!("local-dns");
        }

        let has_geosite_cn = seen_tags.contains("geosite-cn");
        let mut clash_api = json!({
            "external_controller": format!("127.0.0.1:{}", options.api_port),
            "default_mode": options.rules.clash_mode(),
        });
        if let Some(secret) = options
            .api_secret
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            clash_api["secret"] = json!(secret);
        }

        let mut document = json!({
            "log": {
                "level": options.log_level,
                "timestamp": true,
            },
            "inbounds": inbounds,
            "outbounds": outbounds,
            "route": route,
            "experimental": {
                "clash_api": clash_api
            }
        });

        if dns_mode != DnsMode::Off {
            document["dns"] = dns_object(dns_mode, options.rules, has_geosite_cn);
        }

        let mut text =
            serde_json::to_string_pretty(&document).map_err(|error| CompileError::Internal {
                core: CoreKind::SingBox,
                reason: error.to_string(),
            })?;
        text.push('\n');

        Ok(CompiledConfig {
            core: CoreKind::SingBox,
            file_name: self.file_name(),
            text,
            included,
            skipped,
        })
    }
}

fn rule_action_and_outbound(target: RuleTarget) -> (&'static str, Option<&'static str>) {
    match target {
        RuleTarget::Proxy => ("route", Some(PROXY_GROUP)),
        RuleTarget::Direct => ("route", Some("direct")),
        RuleTarget::Reject => ("reject", None),
    }
}

fn rule_set_match(name: &str, target: RuleTarget) -> Value {
    let (action, outbound) = rule_action_and_outbound(target);
    let mut obj = Map::new();
    obj.insert("rule_set".to_string(), json!(name));
    obj.insert("action".to_string(), json!(action));
    if let Some(out) = outbound {
        obj.insert("outbound".to_string(), json!(out));
    }
    Value::Object(obj)
}

fn remote_srs_declaration(tag: &str, url: &str) -> Value {
    json!({
        "tag": tag,
        "type": "remote",
        "format": "binary",
        "url": url,
        // Fetched directly: routing the download through the proxy would
        // deadlock, since the rule-set decides what goes through it.
        "download_detour": "direct",
    })
}

fn resolve_mirror(mirror: Option<&str>) -> Option<String> {
    mirror
        .and_then(crate::binaries::normalize_mirror)
        .or_else(|| {
            std::env::var(crate::binaries::MIRROR_ENV)
                .ok()
                .and_then(|value| crate::binaries::normalize_mirror(&value))
        })
}

fn apply_mirror(url: &str, mirror: Option<&str>) -> String {
    match mirror {
        Some(prefix) => format!("{prefix}{url}"),
        None => url.to_string(),
    }
}

/// Supported SagerNet `.srs` rule-set mapping for Clash `GEOSITE` / `GEOIP` rules.
fn sagernet_geo_rule_set(
    kind: RuleKind,
    raw_tag: &str,
    mirror: Option<&str>,
) -> Option<(String, String)> {
    let normalized = raw_tag.trim().to_ascii_lowercase();
    match kind {
        RuleKind::GeoSite => {
            let known = matches!(
                normalized.as_str(),
                "cn" | "geolocation-!cn"
                    | "category-ads-all"
                    | "apple"
                    | "google"
                    | "github"
                    | "openai"
                    | "telegram"
                    | "youtube"
                    | "microsoft"
                    | "netflix"
                    | "private"
            );
            if !known {
                return None;
            }
            let tag = format!("geosite-{normalized}");
            let url = apply_mirror(
                &format!(
                    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/{tag}.srs"
                ),
                mirror,
            );
            Some((tag, url))
        }
        RuleKind::GeoIp => {
            if normalized != "cn" {
                return None;
            }
            let tag = "geoip-cn".to_string();
            let url = apply_mirror(
                "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-cn.srs",
                mirror,
            );
            Some((tag, url))
        }
        _ => None,
    }
}

/// Convert neutral rules to compact sing-box route rules, merging consecutive
/// rules of the same `(kind, target)` into one JSON array rule.
fn push_batched_rules(
    out: &mut Vec<Value>,
    rules: &[CustomRule],
    mirror: Option<&str>,
    geo_sets: &mut Vec<(String, String)>,
) {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum BatchKey {
        Domain,
        DomainSuffix,
        DomainKeyword,
        IpCidr,
        ProcessName,
        Port,
        PortRange,
    }

    let mut current: Option<(BatchKey, RuleTarget, Vec<Value>)> = None;
    let flush = |out: &mut Vec<Value>, current: &mut Option<(BatchKey, RuleTarget, Vec<Value>)>| {
        let Some((key, target, values)) = current.take() else {
            return;
        };
        if values.is_empty() {
            return;
        }
        let field = match key {
            BatchKey::Domain => "domain",
            BatchKey::DomainSuffix => "domain_suffix",
            BatchKey::DomainKeyword => "domain_keyword",
            BatchKey::IpCidr => "ip_cidr",
            BatchKey::ProcessName => "process_name",
            BatchKey::Port => "port",
            BatchKey::PortRange => "port_range",
        };
        let (action, outbound) = rule_action_and_outbound(target);
        let mut object = Map::new();
        object.insert(field.to_string(), Value::Array(values));
        object.insert("action".to_string(), json!(action));
        if let Some(outbound) = outbound {
            object.insert("outbound".to_string(), json!(outbound));
        }
        out.push(Value::Object(object));
    };

    for rule in rules {
        match rule.kind {
            RuleKind::GeoIp
                if matches!(
                    rule.value.trim().to_ascii_lowercase().as_str(),
                    "lan" | "private"
                ) =>
            {
                flush(out, &mut current);
                let (action, outbound) = rule_action_and_outbound(rule.target);
                let mut object = Map::new();
                object.insert("ip_is_private".to_string(), json!(true));
                object.insert("action".to_string(), json!(action));
                if let Some(outbound) = outbound {
                    object.insert("outbound".to_string(), json!(outbound));
                }
                out.push(Value::Object(object));
            }
            RuleKind::GeoSite | RuleKind::GeoIp => {
                flush(out, &mut current);
                if let Some((tag, url)) = sagernet_geo_rule_set(rule.kind, &rule.value, mirror) {
                    if !geo_sets.iter().any(|(t, _)| t == &tag) {
                        geo_sets.push((tag.clone(), url));
                    }
                    out.push(rule_set_match(&tag, rule.target));
                }
            }
            _ => {
                let (key, val) = match rule.kind {
                    RuleKind::Domain => (BatchKey::Domain, json!(rule.value)),
                    RuleKind::DomainSuffix => (BatchKey::DomainSuffix, json!(rule.value)),
                    RuleKind::DomainKeyword => (BatchKey::DomainKeyword, json!(rule.value)),
                    RuleKind::IpCidr => (BatchKey::IpCidr, json!(rule.value)),
                    RuleKind::ProcessName => (BatchKey::ProcessName, json!(rule.value)),
                    RuleKind::DstPort => {
                        if let Ok(port) = rule.value.trim().parse::<u16>() {
                            (BatchKey::Port, json!(port))
                        } else if let Some((s, e)) = rule.value.trim().split_once('-') {
                            (
                                BatchKey::PortRange,
                                json!(format!("{}:{}", s.trim(), e.trim())),
                            )
                        } else {
                            continue;
                        }
                    }
                    RuleKind::GeoSite | RuleKind::GeoIp => unreachable!(),
                };
                match &mut current {
                    Some((k, t, values)) if *k == key && *t == rule.target => {
                        values.push(val);
                    }
                    _ => {
                        flush(out, &mut current);
                        current = Some((key, rule.target, vec![val]));
                    }
                }
            }
        }
    }
    flush(out, &mut current);
}

fn dns_object(mode: DnsMode, rules_mode: RuleMode, has_geosite_cn: bool) -> Value {
    let mut servers = vec![
        json!({
            "tag": "proxy-dns",
            "type": "https",
            "server": "1.1.1.1",
            "detour": PROXY_GROUP,
        }),
        json!({
            "tag": "local-dns",
            "type": "udp",
            "server": "223.5.5.5",
        }),
    ];

    let mut dns = serde_json::Map::new();
    dns.insert(
        "final".to_string(),
        json!(if rules_mode == RuleMode::Direct {
            "local-dns"
        } else {
            "proxy-dns"
        }),
    );

    let mut dns_rules = Vec::new();
    match rules_mode {
        RuleMode::Split if has_geosite_cn => {
            dns_rules.push(json!({ "rule_set": "geosite-cn", "server": "local-dns" }))
        }
        RuleMode::Split | RuleMode::Global => {
            dns_rules.push(json!({ "domain_suffix": ["cn"], "server": "local-dns" }))
        }
        RuleMode::Direct => {}
    }

    if mode == DnsMode::FakeIp {
        servers.push(json!({
            "tag": "fakeip",
            "type": "fakeip",
            "inet4_range": "198.18.0.0/15",
        }));
        dns_rules.push(json!({ "query_type": ["A", "AAAA"], "server": "fakeip" }));
    }

    if !dns_rules.is_empty() {
        dns.insert("rules".to_string(), json!(dns_rules));
    }

    dns.insert("servers".to_string(), json!(servers));
    Value::Object(dns)
}

fn outbound(node: &Node) -> Result<Value, String> {
    if let Some(reason) = super::common_skip_reason(node) {
        return Err(reason);
    }

    let mut map = Map::new();
    map.insert("tag".to_string(), json!(node.name));
    map.insert("server".to_string(), json!(node.server));
    map.insert("server_port".to_string(), json!(node.port));

    match &node.credentials {
        Credentials::Shadowsocks { method, password } => {
            map.insert("type".to_string(), json!("shadowsocks"));
            map.insert("method".to_string(), json!(method));
            map.insert("password".to_string(), json!(password));
            if let Some(plugin) = &node.plugin {
                // SIP003 form: `name;key=value;...`
                let mut parts = plugin.splitn(2, ';');
                let name = parts.next().unwrap_or("").trim();
                if name.is_empty() {
                    return Err("shadowsocks plugin string has no plugin name".to_string());
                }
                map.insert("plugin".to_string(), json!(name));
                if let Some(options_text) = parts.next() {
                    map.insert("plugin_opts".to_string(), json!(options_text));
                }
            }
        }
        Credentials::Vmess {
            uuid,
            alter_id,
            security,
        } => {
            map.insert("type".to_string(), json!("vmess"));
            map.insert("uuid".to_string(), json!(uuid));
            map.insert("security".to_string(), json!(security));
            map.insert("alter_id".to_string(), json!(alter_id));
        }
        Credentials::Vless { uuid, flow } => {
            map.insert("type".to_string(), json!("vless"));
            map.insert("uuid".to_string(), json!(uuid));
            if *flow == VlessFlow::Vision {
                map.insert("flow".to_string(), json!(flow.as_str()));
            }
        }
        Credentials::Trojan { password } => {
            map.insert("type".to_string(), json!("trojan"));
            map.insert("password".to_string(), json!(password));
        }
        Credentials::Hysteria2 {
            password,
            obfs,
            obfs_password,
        } => {
            map.insert("type".to_string(), json!("hysteria2"));
            map.insert("password".to_string(), json!(password));
            if let Some(obfs) = obfs {
                let mut obfs_object = Map::new();
                obfs_object.insert("type".to_string(), json!(obfs));
                if let Some(secret) = obfs_password {
                    obfs_object.insert("password".to_string(), json!(secret));
                }
                map.insert("obfs".to_string(), Value::Object(obfs_object));
            }
        }
        Credentials::Tuic {
            uuid,
            password,
            congestion_control,
            udp_relay_mode,
        } => {
            map.insert("type".to_string(), json!("tuic"));
            map.insert("uuid".to_string(), json!(uuid));
            map.insert("password".to_string(), json!(password));
            // sing-box spells these with underscores; mihomo uses hyphens.
            if let Some(control) = congestion_control {
                map.insert("congestion_control".to_string(), json!(control));
            }
            if let Some(mode) = udp_relay_mode {
                map.insert("udp_relay_mode".to_string(), json!(mode));
            }
        }
        Credentials::Socks5 { username, password } => {
            map.insert("type".to_string(), json!("socks"));
            if !username.is_empty() {
                map.insert("username".to_string(), json!(username));
                map.insert("password".to_string(), json!(password));
            }
            // A socks outbound with no credentials is version 5 without auth;
            // sing-box infers that, so no explicit field is needed.
            map.insert("version".to_string(), json!("5"));
        }
        Credentials::Http { username, password } => {
            map.insert("type".to_string(), json!("http"));
            if !username.is_empty() {
                map.insert("username".to_string(), json!(username));
                map.insert("password".to_string(), json!(password));
            }
        }
    }

    if node.tls.is_some() {
        map.insert("tls".to_string(), tls_object(node));
    }

    match &node.transport {
        Transport::Tcp => {}
        Transport::Ws(ws) => {
            let mut transport = Map::new();
            transport.insert("type".to_string(), json!("ws"));
            transport.insert("path".to_string(), json!(ws.path));
            if let Some(host) = &ws.host {
                transport.insert("headers".to_string(), json!({ "Host": host }));
            }
            if let Some(early_data) = ws.max_early_data {
                transport.insert("max_early_data".to_string(), json!(early_data));
                transport.insert(
                    "early_data_header_name".to_string(),
                    json!(ws
                        .early_data_header
                        .clone()
                        .unwrap_or_else(|| "Sec-WebSocket-Protocol".to_string())),
                );
            }
            map.insert("transport".to_string(), Value::Object(transport));
        }
        Transport::Grpc(grpc) => {
            map.insert(
                "transport".to_string(),
                json!({ "type": "grpc", "service_name": grpc.service_name }),
            );
        }
        Transport::Http2(h2) => {
            map.insert(
                "transport".to_string(),
                json!({ "type": "http", "path": h2.path, "host": h2.host }),
            );
        }
    }

    Ok(Value::Object(map))
}

fn tls_object(node: &Node) -> Value {
    let tls = node.tls.as_ref().expect("caller checked tls presence");
    let mut object = Map::new();
    object.insert("enabled".to_string(), json!(true));

    if let Some(server_name) = node.server_name() {
        object.insert("server_name".to_string(), json!(server_name));
    }
    if tls.allow_insecure {
        object.insert("insecure".to_string(), json!(true));
    }
    if !tls.alpn.is_empty() {
        object.insert("alpn".to_string(), json!(tls.alpn));
    }
    if let Some(fingerprint) = &tls.fingerprint {
        object.insert(
            "utls".to_string(),
            json!({ "enabled": true, "fingerprint": fingerprint }),
        );
    }
    if let Some(reality) = &tls.reality {
        let mut reality_object = Map::new();
        reality_object.insert("enabled".to_string(), json!(true));
        reality_object.insert("public_key".to_string(), json!(reality.public_key));
        if !reality.short_id.is_empty() {
            reality_object.insert("short_id".to_string(), json!(reality.short_id));
        }
        object.insert("reality".to_string(), Value::Object(reality_object));
    }

    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::node::{RealityConfig, TlsConfig, WsOptions};

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

    fn compiled(node: Node) -> Value {
        let compiled = SingBoxCompiler
            .compile(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap();
        serde_json::from_str(&compiled.text).unwrap()
    }

    #[test]
    fn vless_reality_ws_maps_to_tls_and_transport_objects() {
        let mut node = node_with(
            Credentials::Vless {
                uuid: "uuid".to_string(),
                flow: VlessFlow::Vision,
            },
            Some(TlsConfig {
                server_name: Some("www.microsoft.com".to_string()),
                fingerprint: Some("chrome".to_string()),
                reality: Some(RealityConfig {
                    public_key: "pk".to_string(),
                    short_id: "a1b2".to_string(),
                    spider_x: None,
                }),
                ..Default::default()
            }),
        );
        node.transport = Transport::Ws(WsOptions {
            path: "/ws".to_string(),
            host: Some("cdn.example.com".to_string()),
            ..Default::default()
        });

        let document = compiled(node);
        let outbound = &document["outbounds"][0];
        assert_eq!(outbound["type"], "vless");
        assert_eq!(outbound["flow"], "xtls-rprx-vision");
        assert_eq!(outbound["tls"]["enabled"], true);
        assert_eq!(outbound["tls"]["server_name"], "www.microsoft.com");
        assert_eq!(outbound["tls"]["utls"]["fingerprint"], "chrome");
        assert_eq!(outbound["tls"]["reality"]["public_key"], "pk");
        assert_eq!(outbound["transport"]["type"], "ws");
        assert_eq!(outbound["transport"]["headers"]["Host"], "cdn.example.com");
    }

    #[test]
    fn selector_and_route_final_are_wired() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let document = compiled(node);

        // outbounds: [node, urltest(AUTO), selector(PROXY), direct]
        let auto = &document["outbounds"][1];
        assert_eq!(auto["type"], "urltest");
        assert_eq!(auto["tag"], "AUTO");
        assert_eq!(auto["outbounds"][0], "Tokyo 01");

        let selector = &document["outbounds"][2];
        assert_eq!(selector["type"], "selector");
        assert_eq!(selector["tag"], "PROXY");
        // No explicit pick: the automatic group is the default.
        assert_eq!(selector["default"], "AUTO");
        assert_eq!(selector["outbounds"][0], "AUTO");
        assert_eq!(selector["outbounds"][1], "Tokyo 01");
        assert_eq!(document["route"]["final"], "PROXY");
    }

    #[test]
    fn shadowsocks_plugin_is_passed_through_as_sip003() {
        let mut node = node_with(
            Credentials::Shadowsocks {
                method: "aes-256-gcm".to_string(),
                password: "pw".to_string(),
            },
            None,
        );
        node.plugin = Some("obfs-local;obfs=http;obfs-host=cdn".to_string());

        let document = compiled(node);
        let outbound = &document["outbounds"][0];
        assert_eq!(outbound["plugin"], "obfs-local");
        assert_eq!(outbound["plugin_opts"], "obfs=http;obfs-host=cdn");
    }

    #[test]
    fn grpc_transport_uses_service_name() {
        let mut node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        node.transport = Transport::Grpc(crate::profile::node::GrpcOptions {
            service_name: "svc".to_string(),
            multi_mode: false,
        });
        let document = compiled(node);
        assert_eq!(document["outbounds"][0]["transport"]["type"], "grpc");
        assert_eq!(document["outbounds"][0]["transport"]["service_name"], "svc");
    }

    #[test]
    fn insecure_tls_sets_insecure_flag() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig {
                allow_insecure: true,
                ..Default::default()
            }),
        );
        let document = compiled(node);
        assert_eq!(document["outbounds"][0]["tls"]["insecure"], true);
    }

    #[test]
    fn tun_option_adds_a_tun_inbound() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let options = CompileOptions {
            tun: true,
            ..Default::default()
        };
        let compiled = SingBoxCompiler
            .compile(&Profile::new("p", vec![node]), &options)
            .unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();
        let tun = &document["inbounds"][1];
        assert_eq!(tun["type"], "tun");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(tun["interface_name"], crate::network::tun::INTERFACE_NAME);
        assert_eq!(tun["auto_route"], true);
        // The mixed inbound stays: explicit clients still need a port.
        assert_eq!(document["inbounds"][0]["type"], "mixed");
    }

    #[test]
    fn split_mode_adds_remote_rule_sets_and_direct_rules() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let options = CompileOptions {
            rules: RuleMode::Split,
            ..Default::default()
        };
        let compiled = SingBoxCompiler
            .compile(&Profile::new("p", vec![node]), &options)
            .unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();

        let rule_sets = document["route"]["rule_set"].as_array().unwrap();
        assert_eq!(rule_sets.len(), 2);
        // Fetched directly: routing them through the proxy would deadlock.
        assert_eq!(rule_sets[0]["download_detour"], "direct");

        let rules = document["route"]["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| r["rule_set"] == "geosite-cn"));
        assert!(rules.iter().any(|r| r["ip_cidr"].is_array()));
    }

    #[test]
    fn global_mode_has_no_rule_sets() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let compiled = SingBoxCompiler
            .compile(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();
        assert!(document["route"].get("rule_set").is_none());
    }

    #[test]
    fn dns_section_hijacks_queries_and_resolves_through_the_proxy() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let options = CompileOptions {
            dns: DnsMode::Split,
            ..Default::default()
        };
        let compiled = SingBoxCompiler
            .compile(&Profile::new("p", vec![node]), &options)
            .unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();

        assert_eq!(document["dns"]["final"], "proxy-dns");
        assert_eq!(document["dns"]["servers"][0]["detour"], "PROXY");
        assert_eq!(document["route"]["rules"][0]["action"], "hijack-dns");
        assert_eq!(document["route"]["default_domain_resolver"], "local-dns");
    }

    #[test]
    fn fakeip_mode_adds_a_fakeip_server_and_rule() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let options = CompileOptions {
            dns: DnsMode::FakeIp,
            ..Default::default()
        };
        let compiled = SingBoxCompiler
            .compile(&Profile::new("p", vec![node]), &options)
            .unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();
        let servers = document["dns"]["servers"].as_array().unwrap();
        assert!(servers.iter().any(|s| s["type"] == "fakeip"));
        let rules = document["dns"]["rules"].as_array().unwrap();
        assert_eq!(rules[0]["server"], "local-dns");
        assert!(rules.iter().any(|r| r["server"] == "fakeip"));
    }

    #[test]
    fn dns_off_emits_no_dns_section() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let compiled = SingBoxCompiler
            .compile(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();
        assert!(document.get("dns").is_none());
        assert!(document["route"].get("rules").is_none());
    }

    #[test]
    fn allow_lan_binds_all_interfaces() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let options = CompileOptions {
            allow_lan: true,
            ..Default::default()
        };
        let compiled = SingBoxCompiler
            .compile(&Profile::new("p", vec![node]), &options)
            .unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();
        assert_eq!(document["inbounds"][0]["listen"], "0.0.0.0");
    }

    #[test]
    fn selected_node_is_not_duplicated_and_api_secret_and_mirror_work() {
        let a = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let mut b = a.clone();
        b.name = "Seoul 01".to_string();
        b.server = "kr.example.com".to_string();
        b.refresh_id();

        let options = CompileOptions {
            selected_name: Some("Seoul 01".to_string()),
            api_secret: Some("tok123".to_string()),
            rules: RuleMode::Split,
            dns: DnsMode::Split,
            rule_set_mirror: Some("https://ghfast.top".to_string()),
            ..Default::default()
        };
        let compiled = SingBoxCompiler
            .compile(&Profile::new("p", vec![a, b]), &options)
            .unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();

        let selector = document["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["tag"] == PROXY_GROUP)
            .unwrap();
        let members = selector["outbounds"].as_array().unwrap();
        assert_eq!(
            members
                .iter()
                .filter(|m| m.as_str() == Some("Seoul 01"))
                .count(),
            1
        );
        assert_eq!(document["experimental"]["clash_api"]["secret"], "tok123");
        assert_eq!(document["dns"]["rules"][0]["rule_set"], "geosite-cn");
        assert_eq!(document["dns"]["rules"][0]["server"], "local-dns");
        assert!(document["route"]["rule_set"][0]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://ghfast.top/https://raw.githubusercontent.com/"));
    }

    #[test]
    fn reject_rule_uses_modern_reject_action_and_subscription_rules_take_precedence() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let mut profile = Profile::new("p", vec![node]);
        profile.rules = vec![
            CustomRule {
                kind: RuleKind::Domain,
                value: "ads.example.com".to_string(),
                target: RuleTarget::Reject,
            },
            CustomRule {
                kind: RuleKind::DomainSuffix,
                value: "sub.example.com".to_string(),
                target: RuleTarget::Direct,
            },
        ];
        let options = CompileOptions {
            rules: RuleMode::Split,
            ..Default::default()
        };
        let compiled = SingBoxCompiler.compile(&profile, &options).unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();
        let rules = document["route"]["rules"].as_array().unwrap();
        let reject_rule = rules
            .iter()
            .find(|r| r["domain"] == json!(["ads.example.com"]))
            .unwrap();
        assert_eq!(reject_rule["action"], "reject");
        assert!(reject_rule.get("outbound").is_none());
        assert!(
            rules
                .iter()
                .any(|r| r["domain_suffix"] == json!(["sub.example.com"])
                    && r["outbound"] == "direct")
        );
        assert!(!rules
            .iter()
            .any(|r| r["domain_suffix"] == json!(["qq.com"])));
    }
}
