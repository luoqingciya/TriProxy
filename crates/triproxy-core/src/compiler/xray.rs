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

//! Xray compiler: `Profile` → `config.json`.
//!
//! Xray has no selector concept, so every node becomes an outbound but only the
//! **first** one is used — Xray picks the first outbound as the default route.
//! Picking a specific node therefore requires recompiling, which is acceptable
//! because node selection is a Phase 4 concern and the other two cores offer a
//! proper selector.

use serde_json::{json, Map, Value};

use crate::core::CoreKind;
use crate::profile::node::{Credentials, Node, Transport, VlessFlow};
use crate::profile::Profile;

use super::{
    common_skip_reason, CompileError, CompileOptions, CompiledConfig, ConfigCompiler, CustomRule,
    DnsMode, RuleKind, RuleMode, RuleTarget, SkippedNode,
};

pub struct XrayCompiler;

impl ConfigCompiler for XrayCompiler {
    fn core(&self) -> CoreKind {
        CoreKind::Xray
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

        outbounds.push(json!({ "tag": "direct", "protocol": "freedom" }));

        let listen = if options.allow_lan {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };

        let dns_mode = options.dns.effective(options.tun);

        let document = json!({
            "log": {
                "loglevel": xray_log_level(&options.log_level),
            },
            "inbounds": [
                {
                    "tag": "socks-in",
                    "listen": listen,
                    "port": options.listen_port,
                    "protocol": "socks",
                    "settings": { "udp": true }
                },
                {
                    "tag": "http-in",
                    "listen": listen,
                    "port": options.listen_port.saturating_add(1),
                    "protocol": "http"
                }
            ],
            "outbounds": outbounds,
        });

        // Omit the keys entirely rather than emitting nulls: Xray validates
        // fields it finds, and a null dns/routing block reads as malformed.
        let mut document = document;
        if dns_mode != DnsMode::Off {
            document["dns"] = dns_object();
        }
        // Custom rules need a routing section even in global mode: they are the
        // only thing that can send a request somewhere other than the proxy.
        if options.rules != RuleMode::Global || !options.custom_rules.is_empty() {
            document["routing"] = routing_object(
                included.first().map(String::as_str).unwrap_or("proxy"),
                options.rules,
                &options.custom_rules,
            );
        }

        let mut text =
            serde_json::to_string_pretty(&document).map_err(|error| CompileError::Internal {
                core: CoreKind::Xray,
                reason: error.to_string(),
            })?;
        text.push('\n');

        Ok(CompiledConfig {
            core: CoreKind::Xray,
            file_name: self.file_name(),
            text,
            included,
            skipped,
        })
    }
}

/// Routing section.
///
/// Xray ships its own geosite/geoip data files, so the split lists need no
/// download. Rules send Chinese domains and addresses to the  outbound;
/// anything unmatched falls through to the first outbound, which is the proxy.
///
/// `proxy_tag` is the first outbound's tag — Xray has no group to name, so a
/// custom rule pointing at the proxy has to name the node that currently holds
/// that position.
fn routing_object(proxy_tag: &str, mode: RuleMode, custom: &[CustomRule]) -> Value {
    let mut rules: Vec<Value> = custom
        .iter()
        .map(|rule| custom_rule(rule, proxy_tag))
        .collect();

    if mode != RuleMode::Global {
        rules.push(json!({ "type": "field", "outboundTag": "direct", "domain": ["geosite:cn"] }));
        rules.push(
            json!({ "type": "field", "outboundTag": "direct", "ip": ["geoip:cn", "geoip:private"] }),
        );
    }

    json!({
        "domainStrategy": "IPIfNonMatch",
        "rules": rules,
    })
}

/// One custom rule as an Xray routing rule.
///
/// Xray encodes the match kind inside the value (`domain:`, `full:`, `keyword:`)
/// rather than as a separate field.
fn custom_rule(rule: &CustomRule, proxy_tag: &str) -> Value {
    let tag = match rule.target {
        RuleTarget::Proxy => proxy_tag,
        RuleTarget::Direct => "direct",
    };

    match rule.kind {
        RuleKind::Domain => json!({
            "type": "field",
            "outboundTag": tag,
            "domain": [format!("full:{}", rule.value)],
        }),
        RuleKind::DomainSuffix => json!({
            "type": "field",
            "outboundTag": tag,
            "domain": [format!("domain:{}", rule.value)],
        }),
        RuleKind::DomainKeyword => json!({
            "type": "field",
            "outboundTag": tag,
            "domain": [format!("keyword:{}", rule.value)],
        }),
        RuleKind::IpCidr => json!({
            "type": "field",
            "outboundTag": tag,
            "ip": [rule.value],
        }),
    }
}

/// DNS section.
///
/// Xray resolves through its own DNS service; there is no  equivalent,
/// so these servers are queried over whatever routing applies to their address.
/// Emitting the block still helps: it stops the system resolver from being the
/// only answer source, and it keeps lookups consistent with the proxy rules.
fn dns_object() -> Value {
    json!({
        "servers": ["https://1.1.1.1/dns-query", "223.5.5.5"],
        "queryStrategy": "UseIPv4",
    })
}

/// Xray only accepts debug / info / warning / error / none.
fn xray_log_level(level: &str) -> &'static str {
    match level.to_ascii_lowercase().as_str() {
        "debug" | "trace" => "debug",
        "warn" | "warning" => "warning",
        "error" | "fatal" => "error",
        "none" => "none",
        _ => "info",
    }
}

fn outbound(node: &Node) -> Result<Value, String> {
    if let Some(reason) = common_skip_reason(node) {
        return Err(reason);
    }
    // Learned from Xray's own validator: "REALITY only supports RAW, XHTTP and
    // gRPC for now." A Reality-over-WebSocket node cannot be expressed here.
    if node.uses_reality() && matches!(node.transport, Transport::Ws(_) | Transport::Http2(_)) {
        return Err(
            "Xray REALITY supports only TCP and gRPC transports, not WebSocket/HTTP2".to_string(),
        );
    }
    if node.plugin.is_some() {
        return Err(
            "Xray has no shadowsocks plugin support; the node would not connect".to_string(),
        );
    }

    let mut outbound = Map::new();
    outbound.insert("tag".to_string(), json!(node.name));

    match &node.credentials {
        Credentials::Shadowsocks { method, password } => {
            outbound.insert("protocol".to_string(), json!("shadowsocks"));
            outbound.insert(
                "settings".to_string(),
                json!({
                    "servers": [{
                        "address": node.server,
                        "port": node.port,
                        "method": method,
                        "password": password,
                    }]
                }),
            );
        }
        Credentials::Vmess {
            uuid,
            alter_id,
            security,
        } => {
            outbound.insert("protocol".to_string(), json!("vmess"));
            outbound.insert(
                "settings".to_string(),
                json!({
                    "vnext": [{
                        "address": node.server,
                        "port": node.port,
                        "users": [{
                            "id": uuid,
                            "alterId": alter_id,
                            "security": security,
                        }]
                    }]
                }),
            );
        }
        Credentials::Vless { uuid, flow } => {
            let mut user = Map::new();
            user.insert("id".to_string(), json!(uuid));
            user.insert("encryption".to_string(), json!("none"));
            if *flow == VlessFlow::Vision {
                user.insert("flow".to_string(), json!(flow.as_str()));
            }
            outbound.insert("protocol".to_string(), json!("vless"));
            outbound.insert(
                "settings".to_string(),
                json!({
                    "vnext": [{
                        "address": node.server,
                        "port": node.port,
                        "users": [Value::Object(user)],
                    }]
                }),
            );
        }
        // Unreachable in practice: the capability gate rejects a profile that
        // needs hysteria2 before this compiler runs. Kept explicit so the reason
        // stays readable if that gate is ever relaxed.
        Credentials::Hysteria2 { .. } => {
            return Err("Xray does not support hysteria2".to_string());
        }
        Credentials::Tuic { .. } => {
            return Err("Xray does not support tuic".to_string());
        }
        Credentials::Trojan { password } => {
            outbound.insert("protocol".to_string(), json!("trojan"));
            outbound.insert(
                "settings".to_string(),
                json!({
                    "servers": [{
                        "address": node.server,
                        "port": node.port,
                        "password": password,
                    }]
                }),
            );
        }
        Credentials::Socks5 { username, password } => {
            // Xray models both socks and http the same way: a `servers` array
            // whose `users` list is optional.
            let mut server = Map::new();
            server.insert("address".to_string(), json!(node.server));
            server.insert("port".to_string(), json!(node.port));
            if !username.is_empty() {
                server.insert(
                    "users".to_string(),
                    json!([{ "user": username, "pass": password }]),
                );
            }
            outbound.insert("protocol".to_string(), json!("socks"));
            outbound.insert(
                "settings".to_string(),
                json!({ "servers": [Value::Object(server)] }),
            );
        }
        Credentials::Http { username, password } => {
            let mut server = Map::new();
            server.insert("address".to_string(), json!(node.server));
            server.insert("port".to_string(), json!(node.port));
            if !username.is_empty() {
                server.insert(
                    "users".to_string(),
                    json!([{ "user": username, "pass": password }]),
                );
            }
            outbound.insert("protocol".to_string(), json!("http"));
            outbound.insert(
                "settings".to_string(),
                json!({ "servers": [Value::Object(server)] }),
            );
        }
    }

    outbound.insert("streamSettings".to_string(), stream_settings(node));
    Ok(Value::Object(outbound))
}

fn stream_settings(node: &Node) -> Value {
    let mut settings = Map::new();

    let network = match &node.transport {
        Transport::Tcp => None,
        Transport::Ws(_) => Some("ws"),
        Transport::Grpc(_) => Some("grpc"),
        Transport::Http2(_) => Some("h2"),
    };
    if let Some(network) = network {
        settings.insert("network".to_string(), json!(network));
    }

    match &node.transport {
        Transport::Ws(ws) => {
            let mut ws_settings = Map::new();
            ws_settings.insert("path".to_string(), json!(ws.path));
            if let Some(host) = &ws.host {
                ws_settings.insert("headers".to_string(), json!({ "Host": host }));
            }
            settings.insert("wsSettings".to_string(), Value::Object(ws_settings));
        }
        Transport::Grpc(grpc) => {
            settings.insert(
                "grpcSettings".to_string(),
                json!({ "serviceName": grpc.service_name }),
            );
        }
        Transport::Http2(h2) => {
            settings.insert(
                "httpSettings".to_string(),
                json!({ "path": h2.path, "host": h2.host }),
            );
        }
        Transport::Tcp => {}
    }

    let Some(tls) = &node.tls else {
        return Value::Object(settings);
    };

    let server_name = node.server_name().unwrap_or(&node.server).to_string();

    match &tls.reality {
        Some(reality) => {
            settings.insert("security".to_string(), json!("reality"));
            let mut reality_settings = Map::new();
            reality_settings.insert("serverName".to_string(), json!(server_name));
            reality_settings.insert("publicKey".to_string(), json!(reality.public_key));
            if !reality.short_id.is_empty() {
                reality_settings.insert("shortId".to_string(), json!(reality.short_id));
            }
            if let Some(fingerprint) = &tls.fingerprint {
                reality_settings.insert("fingerprint".to_string(), json!(fingerprint));
            }
            if let Some(spider_x) = &reality.spider_x {
                reality_settings.insert("spiderX".to_string(), json!(spider_x));
            }
            settings.insert(
                "realitySettings".to_string(),
                Value::Object(reality_settings),
            );
        }
        None => {
            settings.insert("security".to_string(), json!("tls"));
            let mut tls_settings = Map::new();
            tls_settings.insert("serverName".to_string(), json!(server_name));
            if tls.allow_insecure {
                tls_settings.insert("allowInsecure".to_string(), json!(true));
            }
            if !tls.alpn.is_empty() {
                tls_settings.insert("alpn".to_string(), json!(tls.alpn));
            }
            if let Some(fingerprint) = &tls.fingerprint {
                tls_settings.insert("fingerprint".to_string(), json!(fingerprint));
            }
            settings.insert("tlsSettings".to_string(), Value::Object(tls_settings));
        }
    }

    Value::Object(settings)
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
        let compiled = XrayCompiler
            .compile(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap();
        serde_json::from_str(&compiled.text).unwrap()
    }

    #[test]
    fn vless_tls_ws_maps_to_stream_settings() {
        let mut node = node_with(
            Credentials::Vless {
                uuid: "uuid".to_string(),
                flow: VlessFlow::Vision,
            },
            Some(TlsConfig {
                server_name: Some("www.microsoft.com".to_string()),
                fingerprint: Some("chrome".to_string()),
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
        assert_eq!(outbound["protocol"], "vless");
        assert_eq!(
            outbound["settings"]["vnext"][0]["users"][0]["flow"],
            "xtls-rprx-vision"
        );
        assert_eq!(
            outbound["settings"]["vnext"][0]["users"][0]["encryption"],
            "none"
        );
        assert_eq!(outbound["streamSettings"]["network"], "ws");
        assert_eq!(outbound["streamSettings"]["security"], "tls");
        assert_eq!(
            outbound["streamSettings"]["tlsSettings"]["fingerprint"],
            "chrome"
        );
        assert_eq!(outbound["streamSettings"]["wsSettings"]["path"], "/ws");
        assert_eq!(
            outbound["streamSettings"]["wsSettings"]["headers"]["Host"],
            "cdn.example.com"
        );
    }

    #[test]
    fn trojan_uses_servers_array_and_tls_settings() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig {
                server_name: Some("sg.example.com".to_string()),
                ..Default::default()
            }),
        );
        let document = compiled(node);
        let outbound = &document["outbounds"][0];
        assert_eq!(outbound["protocol"], "trojan");
        assert_eq!(outbound["settings"]["servers"][0]["password"], "pw");
        assert_eq!(outbound["streamSettings"]["security"], "tls");
        assert_eq!(
            outbound["streamSettings"]["tlsSettings"]["serverName"],
            "sg.example.com"
        );
    }

    #[test]
    fn trojan_without_tls_is_skipped() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            None,
        );
        let compiled = XrayCompiler
            .compile(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap();
        assert!(compiled.included.is_empty());
        assert!(compiled.skipped[0].reason.contains("requires TLS"));
    }

    #[test]
    fn shadowsocks_maps_to_servers_array() {
        let node = node_with(
            Credentials::Shadowsocks {
                method: "aes-256-gcm".to_string(),
                password: "pw".to_string(),
            },
            None,
        );
        let document = compiled(node);
        let outbound = &document["outbounds"][0];
        assert_eq!(outbound["protocol"], "shadowsocks");
        assert_eq!(outbound["settings"]["servers"][0]["method"], "aes-256-gcm");
    }

    #[test]
    fn direct_outbound_is_appended_last() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let document = compiled(node);
        let outbounds = document["outbounds"].as_array().unwrap();
        assert_eq!(outbounds.len(), 2);
        assert_eq!(outbounds[1]["protocol"], "freedom");
    }

    #[test]
    fn inbounds_use_socks_and_http_on_adjacent_ports() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let document = compiled(node);
        assert_eq!(document["inbounds"][0]["port"], 7890);
        assert_eq!(document["inbounds"][0]["protocol"], "socks");
        assert_eq!(document["inbounds"][1]["port"], 7891);
        assert_eq!(document["inbounds"][1]["protocol"], "http");
    }

    #[test]
    fn plugin_nodes_are_skipped() {
        let mut node = node_with(
            Credentials::Shadowsocks {
                method: "aes-256-gcm".to_string(),
                password: "pw".to_string(),
            },
            None,
        );
        node.plugin = Some("obfs-local;obfs=http".to_string());
        let compiled = XrayCompiler
            .compile(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap();
        assert!(compiled.included.is_empty());
        assert!(compiled.skipped[0].reason.contains("plugin"));
    }

    #[test]
    fn reality_over_websocket_is_skipped() {
        // Xray rejects this at load time: "REALITY only supports RAW, XHTTP and
        // gRPC for now."
        let mut node = node_with(
            Credentials::Vless {
                uuid: "uuid".to_string(),
                flow: VlessFlow::Vision,
            },
            Some(TlsConfig {
                reality: Some(RealityConfig {
                    public_key: "pk".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        );
        node.transport = Transport::Ws(WsOptions {
            path: "/ws".to_string(),
            ..Default::default()
        });

        let compiled = XrayCompiler
            .compile(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap();
        assert!(compiled.included.is_empty());
        assert!(compiled.skipped[0].reason.contains("REALITY"));
    }

    #[test]
    fn reality_over_tcp_is_accepted() {
        let node = node_with(
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
                    spider_x: Some("/".to_string()),
                }),
                ..Default::default()
            }),
        );
        let document = compiled(node);
        let stream = &document["outbounds"][0]["streamSettings"];
        assert_eq!(stream["security"], "reality");
        assert_eq!(stream["realitySettings"]["publicKey"], "pk");
        assert_eq!(stream["realitySettings"]["shortId"], "a1b2");
        assert_eq!(stream["realitySettings"]["fingerprint"], "chrome");
        assert_eq!(stream["realitySettings"]["spiderX"], "/");
        assert_eq!(stream["realitySettings"]["serverName"], "www.microsoft.com");
    }

    #[test]
    fn split_mode_adds_geo_routing_to_direct() {
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
        let compiled = XrayCompiler
            .compile(&Profile::new("p", vec![node]), &options)
            .unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();
        let rules = document["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["outboundTag"], "direct");
        assert_eq!(rules[1]["ip"][0], "geoip:cn");
    }

    #[test]
    fn global_mode_has_no_routing_section() {
        let node = node_with(
            Credentials::Trojan {
                password: "pw".to_string(),
            },
            Some(TlsConfig::default()),
        );
        let compiled = XrayCompiler
            .compile(&Profile::new("p", vec![node]), &CompileOptions::default())
            .unwrap();
        let document: Value = serde_json::from_str(&compiled.text).unwrap();
        assert!(document.get("routing").is_none());
    }

    #[test]
    fn log_levels_are_normalised() {
        assert_eq!(xray_log_level("info"), "info");
        assert_eq!(xray_log_level("WARN"), "warning");
        assert_eq!(xray_log_level("trace"), "debug");
        assert_eq!(xray_log_level("nonsense"), "info");
    }
}
