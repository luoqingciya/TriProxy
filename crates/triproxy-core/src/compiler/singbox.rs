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
    CompileError, CompileOptions, CompiledConfig, ConfigCompiler, SkippedNode, PROXY_GROUP,
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

        // The selector is what gives sing-box a user-visible node picker, the
        // equivalent of a Clash `select` group.
        let mut members: Vec<Value> = included.iter().map(|name| json!(name)).collect();
        members.push(json!("direct"));
        let default = included
            .first()
            .cloned()
            .unwrap_or_else(|| "direct".to_string());

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
            inbounds.push(json!({
                "type": "tun",
                "tag": "tun-in",
                "interface_name": crate::network::tun::INTERFACE_NAME,
                "address": ["172.19.0.1/30"],
                "mtu": 9000,
                "auto_route": true,
                "strict_route": true,
                "stack": "mixed",
            }));
        }

        let document = json!({
            "log": {
                "level": options.log_level,
                "timestamp": true,
            },
            "inbounds": inbounds,
            "outbounds": outbounds,
            "route": {
                "final": PROXY_GROUP,
            },
            "experimental": {
                "clash_api": {
                    "external_controller": format!("127.0.0.1:{}", options.api_port),
                }
            }
        });

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

        let selector = &document["outbounds"][1];
        assert_eq!(selector["type"], "selector");
        assert_eq!(selector["tag"], "PROXY");
        assert_eq!(selector["default"], "Tokyo 01");
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
        assert_eq!(tun["interface_name"], crate::network::tun::INTERFACE_NAME);
        assert_eq!(tun["auto_route"], true);
        // The mixed inbound stays: explicit clients still need a port.
        assert_eq!(document["inbounds"][0]["type"], "mixed");
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
}
