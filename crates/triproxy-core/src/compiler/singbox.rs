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

use std::collections::HashSet;

use serde_json::{json, Map, Value};

use crate::core::CoreKind;
use crate::profile::node::{Credentials, Node, Transport, VlessFlow};
use crate::profile::{GroupKind, Profile};

use super::{
    resolved_subscription_groups, CompileError, CompileOptions, CompiledConfig, ConfigCompiler,
    CustomRule, DnsMode, RuleKind, RuleMode, RuleSet, RuleSetFormat, RuleTarget, SkippedNode,
    AUTO_GROUP, AUTO_TEST_INTERVAL_SECS, AUTO_TEST_URL, PROXY_GROUP,
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

        let compiled_groups = resolved_subscription_groups(profile, &included, false);
        let mut compiled_group_names: HashSet<String> = HashSet::new();
        for group in &compiled_groups {
            let group_members: Vec<Value> = group
                .members
                .iter()
                .map(|m| {
                    if m == "DIRECT" {
                        json!("direct")
                    } else {
                        json!(m)
                    }
                })
                .collect();
            if group_members.is_empty() {
                continue;
            }
            compiled_group_names.insert(group.name.clone());
            match group.kind {
                GroupKind::UrlTest | GroupKind::Fallback => outbounds.push(json!({
                    "type": "urltest",
                    "tag": group.name,
                    "outbounds": group_members,
                    "url": AUTO_TEST_URL,
                    "interval": format!("{AUTO_TEST_INTERVAL_SECS}s"),
                    "tolerance": 50,
                })),
                GroupKind::Select | GroupKind::LoadBalance => {
                    let group_default = group_members[0].clone();
                    outbounds.push(json!({
                        "type": "selector",
                        "tag": group.name,
                        "outbounds": group_members,
                        "default": group_default,
                    }));
                }
            }
        }

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
            let mut addresses = vec!["172.19.0.1/30"];
            if options.ipv6 {
                addresses.push("fdfe:dcba:9876::1/126");
            }
            #[allow(unused_mut)]
            let mut tun_inbound = json!({
                "type": "tun",
                "tag": "tun-in",
                "address": addresses,
                "mtu": 9000,
                "auto_route": true,
                "strict_route": options.strict_route,
                "stack": options.tun_stack.as_str(),
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

        // Always sniff first so domain-based rules & DNS match even for IP-only TUN packets.
        let mut rules: Vec<Value> = vec![json!({ "action": "sniff" })];
        if dns_mode != DnsMode::Off {
            rules.push(json!({
                "type": "logical",
                "mode": "or",
                "rules": [{ "protocol": "dns" }, { "port": 53 }],
                "action": "hijack-dns"
            }));
        }
        // Protect private/LAN subnets in every mode (including Global TUN).
        rules.push(json!({ "ip_is_private": true, "action": "route", "outbound": "direct" }));

        // Custom rules outrank mode shortcuts, subscription rules, and built-in presets.
        push_batched_rules(
            &mut rules,
            &options.custom_rules,
            &compiled_group_names,
            mirror.as_deref(),
            &mut required_geo_sets,
        );

        // Live clash_mode rules enable zero-restart switching between rule/global/direct.
        rules.push(json!({ "clash_mode": "direct", "action": "route", "outbound": "direct" }));
        rules.push(json!({ "clash_mode": "global", "action": "route", "outbound": PROXY_GROUP }));

        // Remote rule-sets (`.srs` binary only for sing-box).
        let mut seen_tags = HashSet::new();
        let mut declared: Vec<Value> = Vec::new();

        let srs_sets: Vec<&RuleSet> = options
            .rule_sets
            .iter()
            .filter(|set| set.format == RuleSetFormat::SingBox)
            .collect();
        for set in &srs_sets {
            if seen_tags.insert(set.name.clone()) {
                rules.push(rule_set_match(
                    &set.name,
                    set.target,
                    set.group.as_deref(),
                    &compiled_group_names,
                ));
                declared.push(remote_srs_declaration(&set.name, &set.url));
            }
        }

        let mut terminal_reject = false;
        let final_outbound = if profile.has_subscription_rules() {
            for set in &profile.rule_sets {
                if set.format == RuleSetFormat::SingBox && seen_tags.insert(set.name.clone()) {
                    rules.push(rule_set_match(
                        &set.name,
                        set.target,
                        set.group.as_deref(),
                        &compiled_group_names,
                    ));
                    declared.push(remote_srs_declaration(&set.name, &set.url));
                }
            }
            push_batched_rules(
                &mut rules,
                &profile.rules,
                &compiled_group_names,
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
            // Built-in split rules stay 100% offline so cold-starting a bare
            // profile never blocks on remote GitHub downloads.
            let builtin = super::builtin_rules::builtin_split_rules();
            push_batched_rules(
                &mut rules,
                &builtin,
                &compiled_group_names,
                mirror.as_deref(),
                &mut required_geo_sets,
            );
            PROXY_GROUP
        };

        for (tag, url) in required_geo_sets {
            if seen_tags.insert(tag.clone()) {
                declared.push(remote_srs_declaration(&tag, &url));
            }
        }

        if terminal_reject {
            rules.push(json!({ "action": "reject" }));
        }

        let mut route = json!({
            "final": final_outbound,
        });
        if options.tun {
            route["auto_detect_interface"] = json!(true);
        }
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

        let mut experimental = json!({
            "clash_api": clash_api
        });
        if !declared.is_empty() || dns_mode == DnsMode::FakeIp {
            experimental["cache_file"] = json!({
                "enabled": true,
                "store_fakeip": true,
            });
        }

        let mut server_domains: Vec<String> = Vec::new();
        let mut seen_domains = HashSet::new();
        for node in &profile.nodes {
            let host = node.server.trim();
            if !host.is_empty()
                && host.parse::<std::net::IpAddr>().is_err()
                && seen_domains.insert(host.to_string())
            {
                server_domains.push(host.to_string());
            }
        }

        let mut document = json!({
            "log": {
                "level": options.log_level,
                "timestamp": true,
            },
            "inbounds": inbounds,
            "outbounds": outbounds,
            "route": route,
            "experimental": experimental
        });

        if dns_mode != DnsMode::Off {
            document["dns"] = dns_object(
                dns_mode,
                options.rules,
                has_geosite_cn,
                options.ipv6,
                &server_domains,
            );
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

fn rule_action_and_outbound<'a>(
    target: RuleTarget,
    group: Option<&'a str>,
    compiled_groups: &HashSet<String>,
) -> (&'static str, Option<&'a str>) {
    match target {
        RuleTarget::Reject => ("reject", None),
        RuleTarget::Direct => ("route", Some("direct")),
        RuleTarget::Proxy => {
            if let Some(name) = group.filter(|name| compiled_groups.contains(*name)) {
                ("route", Some(name))
            } else {
                ("route", Some(PROXY_GROUP))
            }
        }
    }
}

fn rule_set_match(
    name: &str,
    target: RuleTarget,
    group: Option<&str>,
    compiled_groups: &HashSet<String>,
) -> Value {
    let (action, outbound) = rule_action_and_outbound(target, group, compiled_groups);
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
/// rules of the same `(kind, action, outbound)` into one JSON array rule.
fn push_batched_rules(
    out: &mut Vec<Value>,
    rules: &[CustomRule],
    compiled_groups: &HashSet<String>,
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

    #[derive(Clone, PartialEq, Eq)]
    struct BatchState<'a> {
        key: BatchKey,
        action: &'static str,
        outbound: Option<&'a str>,
        values: Vec<Value>,
    }

    let mut current: Option<BatchState<'_>> = None;
    let flush = |out: &mut Vec<Value>, current: &mut Option<BatchState<'_>>| {
        let Some(state) = current.take() else {
            return;
        };
        if state.values.is_empty() {
            return;
        }
        let field = match state.key {
            BatchKey::Domain => "domain",
            BatchKey::DomainSuffix => "domain_suffix",
            BatchKey::DomainKeyword => "domain_keyword",
            BatchKey::IpCidr => "ip_cidr",
            BatchKey::ProcessName => "process_name",
            BatchKey::Port => "port",
            BatchKey::PortRange => "port_range",
        };
        let mut object = Map::new();
        object.insert(field.to_string(), Value::Array(state.values));
        object.insert("action".to_string(), json!(state.action));
        if let Some(outbound) = state.outbound {
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
                let (action, outbound) =
                    rule_action_and_outbound(rule.target, rule.group.as_deref(), compiled_groups);
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
                    out.push(rule_set_match(
                        &tag,
                        rule.target,
                        rule.group.as_deref(),
                        compiled_groups,
                    ));
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
                let (action, outbound) =
                    rule_action_and_outbound(rule.target, rule.group.as_deref(), compiled_groups);
                match &mut current {
                    Some(state)
                        if state.key == key
                            && state.action == action
                            && state.outbound == outbound =>
                    {
                        state.values.push(val);
                    }
                    _ => {
                        flush(out, &mut current);
                        current = Some(BatchState {
                            key,
                            action,
                            outbound,
                            values: vec![val],
                        });
                    }
                }
            }
        }
    }
    flush(out, &mut current);
}

fn dns_object(
    mode: DnsMode,
    rules_mode: RuleMode,
    has_geosite_cn: bool,
    ipv6: bool,
    server_domains: &[String],
) -> Value {
    let mut servers = vec![
        json!({
            "tag": "proxy-dns",
            "type": "https",
            "server": "1.1.1.1",
            "detour": PROXY_GROUP,
        }),
        json!({
            "tag": "local-dns",
            "type": "https",
            "server": "1.12.12.12",
        }),
    ];

    let mut dns = serde_json::Map::new();
    dns.insert("final".to_string(), json!("proxy-dns"));
    dns.insert(
        "strategy".to_string(),
        json!(if ipv6 { "prefer_ipv4" } else { "ipv4_only" }),
    );

    let mut dns_rules = Vec::new();
    // Resolve proxy server domains directly via DoH before FakeIP or mode rules
    // so outbound dials never receive synthetic 198.18.0.x addresses.
    if !server_domains.is_empty() {
        dns_rules.push(json!({ "domain": server_domains, "server": "local-dns" }));
    }
    dns_rules.push(json!({ "clash_mode": "direct", "server": "local-dns" }));

    if mode == DnsMode::FakeIp {
        let mut fakeip = json!({
            "tag": "fakeip",
            "type": "fakeip",
            "inet4_range": "198.18.0.0/15",
        });
        if ipv6 {
            fakeip["inet6_range"] = json!("fc00::/18");
        }
        servers.push(fakeip);
        dns_rules.push(json!({ "clash_mode": "global", "server": "fakeip" }));
    } else {
        dns_rules.push(json!({ "clash_mode": "global", "server": "proxy-dns" }));
    }

    if has_geosite_cn {
        dns_rules.push(json!({ "rule_set": "geosite-cn", "server": "local-dns" }));
    } else if rules_mode != RuleMode::Direct {
        dns_rules.push(json!({
            "domain_suffix": super::builtin_rules::builtin_cn_suffixes(),
            "server": "local-dns"
        }));
    }

    if mode == DnsMode::FakeIp {
        dns_rules.push(json!({ "query_type": ["A", "AAAA"], "server": "fakeip" }));
    }

    dns.insert("rules".to_string(), json!(dns_rules));
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
    fn split_mode_uses_offline_direct_rules_without_remote_downloads() {
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
        assert!(!compiled.text.contains("raw.githubusercontent.com"));
        assert!(!compiled.text.contains("independent_cache"));
        assert!(!compiled.text.contains("download_detour"));
        let document: Value = serde_json::from_str(&compiled.text).unwrap();

        assert!(document["route"].get("rule_set").is_none());
        let rules = document["route"]["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| r["domain_suffix"]
            .as_array()
            .is_some_and(|arr| arr.contains(&json!("qq.com")))
            && r["outbound"] == "direct"));
        assert!(rules.iter().any(|r| r["ip_cidr"].is_array()));
    }

    #[test]
    fn global_mode_sets_clash_mode_and_tun_gates_auto_detect_and_dns() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let profile = Profile::new("p", vec![node]);

        let compiled = SingBoxCompiler
            .compile(&profile, &CompileOptions::default())
            .unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();
        assert!(document["route"].get("auto_detect_interface").is_none());
        assert_eq!(
            document["experimental"]["clash_api"]["default_mode"],
            "global"
        );
        let rules = document["route"]["rules"].as_array().unwrap();
        assert_eq!(rules[0]["action"], "sniff");
        assert!(rules
            .iter()
            .any(|r| r["clash_mode"] == "global" && r["outbound"] == PROXY_GROUP));

        let tun_compiled = SingBoxCompiler
            .compile(
                &profile,
                &CompileOptions {
                    tun: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let tun_doc: Value = serde_json::from_str(&tun_compiled.text).unwrap();
        assert_eq!(tun_doc["route"]["auto_detect_interface"], true);
        assert_eq!(tun_doc["experimental"]["cache_file"]["enabled"], true);
        assert_eq!(tun_doc["experimental"]["cache_file"]["store_fakeip"], true);
        assert_eq!(tun_doc["dns"]["servers"][1]["type"], "https");
        assert_eq!(tun_doc["dns"]["servers"][1]["server"], "1.12.12.12");
        assert_eq!(
            tun_doc["dns"]["rules"][0]["domain"],
            json!(["jp.example.com"])
        );
        assert_eq!(tun_doc["dns"]["rules"][0]["server"], "local-dns");
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
        let rules = document["route"]["rules"].as_array().unwrap();
        assert_eq!(rules[0]["action"], "sniff");
        assert_eq!(rules[1]["action"], "hijack-dns");
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
        assert!(rules.iter().any(|r| r["server"] == "local-dns"));
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
        let rules = document["route"]["rules"].as_array().unwrap();
        assert!(!rules.iter().any(|r| r["action"] == "hijack-dns"));
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
            custom_rules: vec![CustomRule {
                kind: RuleKind::GeoSite,
                value: "cn".to_string(),
                target: RuleTarget::Direct,
                group: None,
            }],
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
        assert_eq!(document["experimental"]["cache_file"]["enabled"], true);
        assert!(document["dns"].get("independent_cache").is_none());
        assert!(document["route"]["rule_set"][0]
            .get("download_detour")
            .is_none());
        assert!(document["dns"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule["rule_set"] == "geosite-cn" && rule["server"] == "local-dns"));
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
                group: None,
            },
            CustomRule {
                kind: RuleKind::DomainSuffix,
                value: "sub.example.com".to_string(),
                target: RuleTarget::Direct,
                group: None,
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
