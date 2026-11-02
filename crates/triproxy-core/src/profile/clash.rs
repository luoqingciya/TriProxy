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
//! Scope: the `proxies:` list. Groups and rules are ignored, because this
//! program generates its own single `PROXY` group and its own routing — importing
//! a Clash `rules:` block would mean reimplementing Clash's rule engine, which is
//! a different product.
//!
//! Unsupported proxy types are reported as warnings and skipped, never silently
//! dropped: a user importing a 60-node config needs to know that 12 of them
//! did not make it and why.

use serde::Deserialize;

use super::node::{
    Credentials, GrpcOptions, Http2Options, Node, RealityConfig, TlsConfig, Transport, VlessFlow,
    WsOptions,
};
use super::validate::{Diagnostic, Severity};

#[derive(Debug, Deserialize)]
struct ClashConfig {
    #[serde(default)]
    proxies: Vec<ClashProxy>,
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

/// Parse a Clash configuration into nodes plus diagnostics.
pub fn parse(text: &str) -> (Vec<Node>, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();

    let config: ClashConfig = match serde_norway::from_str(text) {
        Ok(config) => config,
        Err(error) => {
            diagnostics.push(Diagnostic::new(
                Severity::Error,
                "clash.invalid_yaml",
                format!("cannot parse the configuration: {error}"),
            ));
            return (Vec::new(), diagnostics);
        }
    };

    if config.proxies.is_empty() {
        diagnostics.push(Diagnostic::new(
            Severity::Error,
            "clash.no_proxies",
            "the configuration has no proxies",
        ));
        return (Vec::new(), diagnostics);
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

    (nodes, diagnostics)
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
        other => return Err(format!("`{name}`: proxy type `{other}` is not supported")),
    };

    // Trojan is TLS-only in practice; Clash configs usually say so, but a missing
    // `tls` key on a trojan entry means TLS, not plaintext.
    let tls_wanted = proxy.tls.unwrap_or(proxy.kind == "trojan") || proxy.reality_opts.is_some();

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
  - name: "Hy2"
    type: hysteria2
    server: hy.example.com
    port: 443
    password: x
"#;

    #[test]
    fn imports_supported_proxies_and_reports_the_rest() {
        let (nodes, diagnostics) = parse(SAMPLE);

        assert_eq!(nodes.len(), 3, "hysteria2 should be skipped");
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

        let skipped: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.code == "clash.unsupported_proxy")
            .collect();
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].message.contains("hysteria2"),
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
}
