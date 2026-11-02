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

//! Parsers for `ss://`, `vmess://`, `vless://` and `trojan://` share links.
//!
//! Written by hand rather than with a generic URL crate because the share
//! formats are only URI-shaped: `ss://` has a legacy form whose entire body is
//! base64, `vmess://` carries base64 JSON, and query keys are not standardised
//! across clients (`serviceName` vs `path`, `security` vs `tls`).

use serde::Deserialize;
use serde_json::Value;

use super::encoding::{decode_base64_string, percent_decode};
use super::node::{
    Credentials, GrpcOptions, Http2Options, Node, RealityConfig, TlsConfig, Transport, VlessFlow,
    WsOptions,
};
use crate::capability::Protocol;

/// A recoverable parse failure. Reported as a diagnostic instead of aborting the
/// whole subscription, so one broken line cannot hide the other 200.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseIssue {
    pub code: &'static str,
    pub message: String,
}

impl ParseIssue {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ParseIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Parse a single share link.
pub fn parse_uri(line: &str) -> Result<Node, ParseIssue> {
    let trimmed = line.trim();
    let (scheme, rest) = trimmed.split_once("://").ok_or_else(|| {
        ParseIssue::new(
            "missing_scheme",
            format!("not a share link (no `://`): {}", truncate(trimmed)),
        )
    })?;

    match scheme.to_ascii_lowercase().as_str() {
        "ss" => parse_ss(rest),
        "vmess" => parse_vmess(rest),
        "vless" => parse_vless(rest),
        "trojan" => parse_trojan(rest),
        "hysteria2" | "hy2" => parse_hysteria2(rest),
        "tuic" => parse_tuic(rest),
        other => Err(ParseIssue::new(
            "unsupported_scheme",
            format!("unsupported scheme `{other}://`"),
        )),
    }
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= 40 {
        text.to_string()
    } else {
        let head: String = text.chars().take(40).collect();
        format!("{head}…")
    }
}

// ---------------------------------------------------------------------------
// shared URI splitting
// ---------------------------------------------------------------------------

struct UriParts {
    body: String,
    query: Vec<(String, String)>,
    fragment: Option<String>,
}

fn split_parts(rest: &str) -> UriParts {
    let (before_fragment, fragment) = match rest.rfind('#') {
        Some(index) => (&rest[..index], Some(percent_decode(&rest[index + 1..]))),
        None => (rest, None),
    };
    let (body, query) = match before_fragment.find('?') {
        Some(index) => (
            &before_fragment[..index],
            parse_query(&before_fragment[index + 1..]),
        ),
        None => (before_fragment, Vec::new()),
    };

    UriParts {
        body: body.to_string(),
        query,
        fragment: fragment
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty()),
    }
}

fn parse_query(raw: &str) -> Vec<(String, String)> {
    raw.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => (
                percent_decode(key).to_ascii_lowercase(),
                percent_decode(value),
            ),
            None => (percent_decode(pair).to_ascii_lowercase(), String::new()),
        })
        .collect()
}

fn query_get<'a>(query: &'a [(String, String)], key: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.as_str())
}

fn query_non_empty<'a>(query: &'a [(String, String)], key: &str) -> Option<&'a str> {
    query_get(query, key).filter(|value| !value.trim().is_empty())
}

fn split_userinfo(body: &str) -> Option<(String, String)> {
    let (userinfo, hostport) = body.rsplit_once('@')?;
    Some((userinfo.to_string(), hostport.to_string()))
}

/// Parse `host:port`, supporting IPv6 literals in brackets and a trailing slash.
fn parse_host_port(hostport: &str) -> Option<(String, u16)> {
    let hostport = hostport.trim_end_matches('/');
    if let Some(stripped) = hostport.strip_prefix('[') {
        let (host, rest) = stripped.split_once(']')?;
        let port = rest.strip_prefix(':')?;
        return Some((host.to_string(), port.parse().ok()?));
    }
    let (host, port) = hostport.rsplit_once(':')?;
    if host.trim().is_empty() {
        return None;
    }
    Some((host.to_string(), port.parse().ok()?))
}

fn split_alpn(value: Option<&str>) -> Vec<String> {
    value
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

/// Everything a parser collects before the protocol-specific credentials are
/// known, so `finish` does not need a nine-argument signature.
struct NodeParts {
    name: Option<String>,
    server: String,
    port: u16,
    transport: Transport,
    tls: Option<TlsConfig>,
    plugin: Option<String>,
}

fn finish(protocol: Protocol, credentials: Credentials, parts: NodeParts) -> Node {
    let mut node = Node {
        id: String::new(),
        name: parts.name.unwrap_or_default(),
        protocol,
        server: parts.server,
        port: parts.port,
        credentials,
        transport: parts.transport,
        tls: parts.tls,
        udp: true,
        plugin: parts.plugin,
    };
    node.refresh_id();
    node
}

// ---------------------------------------------------------------------------
// shadowsocks
// ---------------------------------------------------------------------------

/// `ss://` comes in two incompatible shapes:
///
/// - SIP002: `ss://base64(method:password)@host:port/?plugin=...#name`
/// - legacy: `ss://base64(method:password@host:port)#name`
///
/// Both are accepted; the presence of `@` decides which one it is.
fn parse_ss(rest: &str) -> Result<Node, ParseIssue> {
    let parts = split_parts(rest);
    let body = parts.body.trim().trim_end_matches('/');

    let (userinfo, hostport) = match split_userinfo(body) {
        Some((userinfo, hostport)) => (userinfo, hostport),
        None => {
            let decoded = decode_base64_string(body).ok_or_else(|| {
                ParseIssue::new(
                    "ss_not_decodable",
                    "shadowsocks body is neither `userinfo@host:port` nor base64",
                )
            })?;
            split_userinfo(&decoded).ok_or_else(|| {
                ParseIssue::new(
                    "ss_malformed",
                    "decoded shadowsocks payload has no `userinfo@host:port`",
                )
            })?
        }
    };

    let (method, password) = parse_ss_userinfo(&userinfo)?;
    let (server, port) = parse_host_port(&hostport).ok_or_else(|| {
        ParseIssue::new(
            "ss_bad_endpoint",
            format!("invalid endpoint `{hostport}` in shadowsocks link"),
        )
    })?;

    Ok(finish(
        Protocol::Shadowsocks,
        Credentials::Shadowsocks { method, password },
        NodeParts {
            name: parts.fragment,
            server,
            port,
            transport: Transport::Tcp,
            tls: None,
            plugin: query_non_empty(&parts.query, "plugin").map(str::to_string),
        },
    ))
}

fn parse_ss_userinfo(raw: &str) -> Result<(String, String), ParseIssue> {
    let decoded = percent_decode(raw);

    // Plain `method:password`. Base64 never contains `:`, so this is unambiguous.
    if let Some((method, password)) = decoded.split_once(':') {
        if !method.trim().is_empty() {
            return Ok((method.trim().to_string(), password.to_string()));
        }
    }

    let text = decode_base64_string(&decoded).ok_or_else(|| {
        ParseIssue::new(
            "ss_bad_userinfo",
            "shadowsocks userinfo is neither `method:password` nor base64 of it",
        )
    })?;
    text.split_once(':')
        .map(|(method, password)| (method.trim().to_string(), password.to_string()))
        .ok_or_else(|| {
            ParseIssue::new(
                "ss_bad_userinfo",
                "decoded shadowsocks userinfo has no `method:password`",
            )
        })
}

// ---------------------------------------------------------------------------
// vmess
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct VmessPayload {
    #[serde(default)]
    ps: String,
    add: String,
    port: Value,
    id: String,
    #[serde(default)]
    aid: Value,
    #[serde(default)]
    scy: String,
    #[serde(default)]
    net: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    tls: String,
    #[serde(default)]
    sni: String,
    #[serde(default)]
    alpn: String,
    #[serde(default)]
    fp: String,
}

fn value_to_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn parse_vmess(rest: &str) -> Result<Node, ParseIssue> {
    let parts = split_parts(rest);

    let text = decode_base64_string(&parts.body).or_else(|| {
        // A minority of providers emit the JSON directly.
        let trimmed = parts.body.trim();
        trimmed.starts_with('{').then(|| trimmed.to_string())
    });

    let text = text.ok_or_else(|| {
        ParseIssue::new(
            "vmess_not_decodable",
            "vmess body is neither base64 nor raw JSON",
        )
    })?;

    let payload: VmessPayload = serde_json::from_str(&text).map_err(|error| {
        ParseIssue::new(
            "vmess_bad_json",
            format!("vmess payload is not valid JSON: {error}"),
        )
    })?;

    let port = value_to_u64(&payload.port)
        .filter(|port| *port > 0 && *port <= u16::MAX as u64)
        .ok_or_else(|| {
            ParseIssue::new(
                "vmess_bad_port",
                format!("invalid vmess port `{}`", payload.port),
            )
        })? as u16;

    let alter_id = value_to_u64(&payload.aid).unwrap_or(0) as u32;
    let security = if payload.scy.trim().is_empty() {
        "auto".to_string()
    } else {
        payload.scy.trim().to_string()
    };

    let host_header = (!payload.host.trim().is_empty()).then(|| payload.host.trim().to_string());
    let path = if payload.path.trim().is_empty() {
        "/".to_string()
    } else {
        payload.path.trim().to_string()
    };

    let transport = match payload.net.trim().to_ascii_lowercase().as_str() {
        "" | "tcp" => Transport::Tcp,
        "ws" => Transport::Ws(WsOptions {
            path,
            host: host_header.clone(),
            max_early_data: None,
            early_data_header: None,
        }),
        "grpc" => Transport::Grpc(GrpcOptions {
            service_name: payload.path.trim().to_string(),
            multi_mode: false,
        }),
        "h2" | "http" => Transport::Http2(Http2Options {
            path,
            host: host_header.iter().cloned().collect(),
        }),
        other => {
            return Err(ParseIssue::new(
                "vmess_unsupported_transport",
                format!("unsupported vmess transport `{other}`"),
            ))
        }
    };

    let tls = match payload.tls.trim().to_ascii_lowercase().as_str() {
        "" | "none" => None,
        "tls" => Some(TlsConfig {
            server_name: Some(
                [payload.sni.trim(), payload.host.trim(), payload.add.trim()]
                    .iter()
                    .find(|candidate| !candidate.is_empty())
                    .map(|candidate| candidate.to_string())
                    .unwrap_or_default(),
            ),
            allow_insecure: false,
            alpn: split_alpn(Some(payload.alpn.as_str())),
            fingerprint: (!payload.fp.trim().is_empty()).then(|| payload.fp.trim().to_string()),
            reality: None,
        }),
        other => {
            return Err(ParseIssue::new(
                "vmess_unsupported_security",
                format!("unsupported vmess security `{other}`"),
            ))
        }
    };

    let name = parts
        .fragment
        .or_else(|| (!payload.ps.trim().is_empty()).then(|| payload.ps.trim().to_string()));

    Ok(finish(
        Protocol::Vmess,
        Credentials::Vmess {
            uuid: payload.id.trim().to_string(),
            alter_id,
            security,
        },
        NodeParts {
            name,
            server: payload.add.trim().to_string(),
            port,
            transport,
            tls,
            plugin: None,
        },
    ))
}

// ---------------------------------------------------------------------------
// vless / trojan
// ---------------------------------------------------------------------------

struct StandardUri {
    userinfo: String,
    server: String,
    port: u16,
    query: Vec<(String, String)>,
    fragment: Option<String>,
}

fn parse_standard(rest: &str) -> Result<StandardUri, ParseIssue> {
    let parts = split_parts(rest);
    let (userinfo, hostport) = split_userinfo(parts.body.trim())
        .ok_or_else(|| ParseIssue::new("missing_userinfo", "link has no `user@host:port`"))?;
    let (server, port) = parse_host_port(&hostport)
        .ok_or_else(|| ParseIssue::new("bad_endpoint", format!("invalid endpoint `{hostport}`")))?;

    Ok(StandardUri {
        userinfo: percent_decode(&userinfo),
        server,
        port,
        query: parts.query,
        fragment: parts.fragment,
    })
}

fn build_transport(query: &[(String, String)]) -> Result<Transport, ParseIssue> {
    let kind = query_get(query, "type")
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let path = query_non_empty(query, "path").unwrap_or("/").to_string();
    let host = query_non_empty(query, "host").map(str::to_string);

    match kind.as_str() {
        "" | "tcp" | "raw" => Ok(Transport::Tcp),
        "ws" | "websocket" => Ok(Transport::Ws(WsOptions {
            path,
            host,
            max_early_data: query_non_empty(query, "ed").and_then(|value| value.parse().ok()),
            early_data_header: query_non_empty(query, "eh").map(str::to_string),
        })),
        "grpc" => Ok(Transport::Grpc(GrpcOptions {
            service_name: query_non_empty(query, "servicename")
                .or_else(|| query_non_empty(query, "path"))
                .unwrap_or("")
                .to_string(),
            multi_mode: truthy(query_get(query, "mode")),
        })),
        "h2" | "http" => Ok(Transport::Http2(Http2Options {
            path,
            host: host.into_iter().collect(),
        })),
        other => Err(ParseIssue::new(
            "unsupported_transport",
            format!("unsupported transport `{other}`"),
        )),
    }
}

fn build_tls(query: &[(String, String)], server: &str) -> Result<Option<TlsConfig>, ParseIssue> {
    let security = query_get(query, "security")
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    match security.as_str() {
        "" | "none" => Ok(None),
        "tls" | "xtls" => Ok(Some(TlsConfig {
            server_name: Some(
                query_non_empty(query, "sni")
                    .or_else(|| query_non_empty(query, "host"))
                    .unwrap_or(server)
                    .to_string(),
            ),
            allow_insecure: truthy(query_get(query, "allowinsecure")),
            alpn: split_alpn(query_non_empty(query, "alpn")),
            fingerprint: query_non_empty(query, "fp").map(str::to_string),
            reality: None,
        })),
        "reality" => Ok(Some(TlsConfig {
            server_name: Some(
                query_non_empty(query, "sni")
                    .or_else(|| query_non_empty(query, "host"))
                    .unwrap_or(server)
                    .to_string(),
            ),
            allow_insecure: truthy(query_get(query, "allowinsecure")),
            alpn: split_alpn(query_non_empty(query, "alpn")),
            fingerprint: query_non_empty(query, "fp").map(str::to_string),
            // An empty public key is left for the validator to reject, so the
            // message can name the node instead of just the line.
            reality: Some(RealityConfig {
                public_key: query_get(query, "pbk").unwrap_or("").trim().to_string(),
                short_id: query_get(query, "sid").unwrap_or("").trim().to_string(),
                spider_x: query_non_empty(query, "spx").map(str::to_string),
            }),
        })),
        other => Err(ParseIssue::new(
            "unsupported_security",
            format!("unsupported security `{other}`"),
        )),
    }
}

fn parse_vless(rest: &str) -> Result<Node, ParseIssue> {
    let uri = parse_standard(rest)?;

    let encryption = query_get(&uri.query, "encryption").unwrap_or("none");
    if !encryption.trim().is_empty() && !encryption.eq_ignore_ascii_case("none") {
        return Err(ParseIssue::new(
            "vless_unsupported_encryption",
            format!("VLESS encryption `{encryption}` is not supported (only `none`)"),
        ));
    }

    let (flow, unknown_flow) = VlessFlow::parse(query_get(&uri.query, "flow").unwrap_or(""));
    if let Some(unknown) = unknown_flow {
        return Err(ParseIssue::new(
            "vless_unsupported_flow",
            format!("unsupported VLESS flow `{unknown}`"),
        ));
    }

    let transport = build_transport(&uri.query)?;
    let tls = build_tls(&uri.query, &uri.server)?;

    Ok(finish(
        Protocol::Vless,
        Credentials::Vless {
            uuid: uri.userinfo.trim().to_string(),
            flow,
        },
        NodeParts {
            name: uri.fragment,
            server: uri.server,
            port: uri.port,
            transport,
            tls,
            plugin: None,
        },
    ))
}

/// Hysteria2 share link.
///
/// Always TLS — the protocol has no plaintext mode, so a link without
/// `security` is still TLS. `insecure=1` maps to skip-cert-verify, and the
/// obfuscation pair only makes sense together.
/// TUIC share link.
///
/// The userinfo is `uuid:password`, unlike the other protocols which carry a
/// single secret. QUIC means TLS is mandatory, and the default ALPN is h3.
fn parse_tuic(rest: &str) -> Result<Node, ParseIssue> {
    let uri = parse_standard(rest)?;

    let (uuid, password) = uri
        .userinfo
        .split_once(':')
        .ok_or_else(|| ParseIssue::new("tuic.missing_password", "tuic link needs uuid:password"))?;

    if uuid.trim().is_empty() {
        return Err(ParseIssue::new("tuic.empty_uuid", "tuic link has no uuid"));
    }
    if password.is_empty() {
        return Err(ParseIssue::new(
            "tuic.empty_password",
            "tuic link has no password",
        ));
    }

    let congestion_control = query_non_empty(&uri.query, "congestion_control")
        .or_else(|| query_non_empty(&uri.query, "congestion_control_type"))
        .map(str::to_string);
    let udp_relay_mode = query_non_empty(&uri.query, "udp_relay_mode").map(str::to_string);

    let allow_insecure = query_non_empty(&uri.query, "allow_insecure")
        .or_else(|| query_non_empty(&uri.query, "insecure"))
        .map(|value| matches!(value, "1" | "true" | "yes"))
        .unwrap_or(false);

    let alpn = query_non_empty(&uri.query, "alpn")
        .map(|value| {
            value
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| vec!["h3".to_string()]);

    let tls = Some(TlsConfig {
        server_name: Some(
            query_non_empty(&uri.query, "sni")
                .unwrap_or(&uri.server)
                .to_string(),
        ),
        allow_insecure,
        alpn,
        fingerprint: None,
        reality: None,
    });

    Ok(finish(
        Protocol::Tuic,
        Credentials::Tuic {
            uuid: uuid.to_string(),
            password: password.to_string(),
            congestion_control,
            udp_relay_mode,
        },
        NodeParts {
            name: uri.fragment,
            server: uri.server,
            port: uri.port,
            transport: Transport::Tcp,
            tls,
            plugin: None,
        },
    ))
}

fn parse_hysteria2(rest: &str) -> Result<Node, ParseIssue> {
    let uri = parse_standard(rest)?;

    let password = uri.userinfo;
    if password.is_empty() {
        return Err(ParseIssue::new(
            "hysteria2.empty_password",
            "hysteria2 link has no password",
        ));
    }

    let obfs = query_non_empty(&uri.query, "obfs").map(str::to_string);
    let obfs_password = query_non_empty(&uri.query, "obfs-password").map(str::to_string);

    if obfs.is_some() && obfs_password.is_none() {
        return Err(ParseIssue::new(
            "hysteria2.obfs_without_password",
            "hysteria2 link sets obfs but no obfs-password",
        ));
    }

    // Hysteria2 has no plaintext mode, so TLS is built here rather than derived
    // from a `security` parameter that the link format does not define.
    let allow_insecure = query_non_empty(&uri.query, "insecure")
        .map(|value| matches!(value, "1" | "true" | "yes"))
        .unwrap_or(false);
    let tls = Some(TlsConfig {
        server_name: Some(
            query_non_empty(&uri.query, "sni")
                .or_else(|| query_non_empty(&uri.query, "peer"))
                .unwrap_or(&uri.server)
                .to_string(),
        ),
        allow_insecure,
        alpn: query_non_empty(&uri.query, "alpn")
            .map(|value| {
                value
                    .split(',')
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        fingerprint: None,
        reality: None,
    });

    Ok(finish(
        Protocol::Hysteria2,
        Credentials::Hysteria2 {
            password,
            obfs,
            obfs_password,
        },
        NodeParts {
            name: uri.fragment,
            server: uri.server,
            port: uri.port,
            transport: Transport::Tcp,
            tls,
            plugin: None,
        },
    ))
}

fn parse_trojan(rest: &str) -> Result<Node, ParseIssue> {
    let uri = parse_standard(rest)?;
    let transport = build_transport(&uri.query)?;
    let tls = build_tls(&uri.query, &uri.server)?;

    Ok(finish(
        Protocol::Trojan,
        Credentials::Trojan {
            password: uri.userinfo,
        },
        NodeParts {
            name: uri.fragment,
            server: uri.server,
            port: uri.port,
            transport,
            tls,
            plugin: None,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vless_reality_ws_is_fully_modelled() {
        let uri = "vless://11111111-2222-3333-4444-555555555555@example.com:443\
                   ?encryption=none&security=reality&sni=www.microsoft.com&fp=chrome\
                   &pbk=abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG&sid=aa\
                   &type=ws&path=%2Fws%3Fed%3D2048&host=cdn.example.com\
                   &flow=xtls-rprx-vision#Tokyo%2001";
        let node = parse_uri(uri).unwrap();

        assert_eq!(node.protocol, Protocol::Vless);
        assert_eq!(node.name, "Tokyo 01");
        assert_eq!(node.server, "example.com");
        assert_eq!(node.port, 443);
        assert!(node.uses_reality());
        assert!(node.uses_vision());

        let tls = node.tls.as_ref().unwrap();
        assert_eq!(tls.server_name.as_deref(), Some("www.microsoft.com"));
        assert_eq!(tls.fingerprint.as_deref(), Some("chrome"));
        let reality = tls.reality.as_ref().unwrap();
        assert_eq!(reality.short_id, "aa");
        assert_eq!(reality.public_key.len(), 43);

        match &node.transport {
            Transport::Ws(options) => {
                assert_eq!(options.path, "/ws?ed=2048");
                assert_eq!(options.host.as_deref(), Some("cdn.example.com"));
            }
            other => panic!("expected ws transport, got {other:?}"),
        }
    }

    #[test]
    fn vless_without_security_has_no_tls() {
        let node = parse_uri("vless://uuid@1.2.3.4:80?encryption=none&type=tcp#plain").unwrap();
        assert!(node.tls.is_none());
        assert_eq!(node.transport, Transport::Tcp);
    }

    #[test]
    fn vless_rejects_unknown_flow_instead_of_dropping_it() {
        let error = parse_uri("vless://uuid@example.com:443?security=tls&flow=xtls-rprx-origin#x")
            .unwrap_err();
        assert_eq!(error.code, "vless_unsupported_flow");
    }

    #[test]
    fn trojan_keeps_password_verbatim() {
        let node =
            parse_uri("trojan://p%40ss%3Aword@example.com:443?sni=example.com#Trojan").unwrap();
        assert_eq!(node.name, "Trojan");
        match node.credentials {
            Credentials::Trojan { password } => assert_eq!(password, "p@ss:word"),
            other => panic!("unexpected credentials {other:?}"),
        }
    }

    #[test]
    fn ss_sip002_with_base64_userinfo() {
        // base64("aes-256-gcm:secret")
        let node = parse_uri("ss://YWVzLTI1Ni1nY206c2VjcmV0@1.2.3.4:8388#SS%20node").unwrap();
        assert_eq!(node.protocol, Protocol::Shadowsocks);
        assert_eq!(node.name, "SS node");
        match node.credentials {
            Credentials::Shadowsocks { method, password } => {
                assert_eq!(method, "aes-256-gcm");
                assert_eq!(password, "secret");
            }
            other => panic!("unexpected credentials {other:?}"),
        }
    }

    #[test]
    fn ss_sip002_with_plain_userinfo_and_plugin() {
        let node =
            parse_uri("ss://aes-128-gcm:pw@example.com:443?plugin=obfs-local%3Bobfs%3Dhttp#n")
                .unwrap();
        assert_eq!(node.plugin.as_deref(), Some("obfs-local;obfs=http"));
        assert_eq!(node.port, 443);
    }

    #[test]
    fn ss_legacy_fully_base64_body() {
        // base64("chacha20-ietf-poly1305:pw@example.com:8388")
        let node =
            parse_uri("ss://Y2hhY2hhMjAtaWV0Zi1wb2x5MTMwNTpwd0BleGFtcGxlLmNvbTo4Mzg4#legacy")
                .unwrap();
        assert_eq!(node.server, "example.com");
        assert_eq!(node.port, 8388);
        assert_eq!(node.name, "legacy");
    }

    #[test]
    fn ss_accepts_ipv6_endpoint() {
        let node = parse_uri("ss://aes-128-gcm:pw@[2001:db8::1]:8388#v6").unwrap();
        assert_eq!(node.server, "2001:db8::1");
        assert_eq!(node.endpoint(), "[2001:db8::1]:8388");
    }

    #[test]
    fn vmess_decodes_json_payload() {
        let payload = serde_json::json!({
            "v": "2",
            "ps": "HK 01",
            "add": "hk.example.com",
            "port": "443",
            "id": "11111111-2222-3333-4444-555555555555",
            "aid": "0",
            "scy": "auto",
            "net": "ws",
            "host": "cdn.example.com",
            "path": "/ray",
            "tls": "tls",
            "sni": "cdn.example.com"
        });
        let encoded = {
            use base64::engine::general_purpose::STANDARD;
            use base64::Engine;
            STANDARD.encode(serde_json::to_vec(&payload).unwrap())
        };

        let node = parse_uri(&format!("vmess://{encoded}")).unwrap();
        assert_eq!(node.name, "HK 01");
        assert_eq!(node.port, 443);
        assert_eq!(node.server, "hk.example.com");
        match &node.transport {
            Transport::Ws(options) => assert_eq!(options.path, "/ray"),
            other => panic!("expected ws, got {other:?}"),
        }
        let tls = node.tls.as_ref().unwrap();
        assert_eq!(tls.server_name.as_deref(), Some("cdn.example.com"));
    }

    #[test]
    fn vmess_rejects_unsupported_transport() {
        let payload = serde_json::json!({
            "add": "a.example.com", "port": 443,
            "id": "11111111-2222-3333-4444-555555555555",
            "net": "kcp"
        });
        let encoded = {
            use base64::engine::general_purpose::STANDARD;
            use base64::Engine;
            STANDARD.encode(serde_json::to_vec(&payload).unwrap())
        };
        let error = parse_uri(&format!("vmess://{encoded}")).unwrap_err();
        assert_eq!(error.code, "vmess_unsupported_transport");
    }

    #[test]
    fn tuic_splits_uuid_and_password_and_defaults_to_h3() {
        let uri = "tuic://11111111-2222-3333-4444-555555555555:secret@tuic.example.com:443\
                   ?congestion_control=bbr&udp_relay_mode=native&sni=www.bing.com#Tuic";
        let node = parse_uri(uri).unwrap();

        assert_eq!(node.protocol, Protocol::Tuic);
        match &node.credentials {
            Credentials::Tuic {
                uuid,
                password,
                congestion_control,
                udp_relay_mode,
            } => {
                assert_eq!(uuid, "11111111-2222-3333-4444-555555555555");
                assert_eq!(password, "secret");
                assert_eq!(congestion_control.as_deref(), Some("bbr"));
                assert_eq!(udp_relay_mode.as_deref(), Some("native"));
            }
            other => panic!("expected tuic, got {other:?}"),
        }

        let tls = node.tls.expect("tuic is always TLS");
        // QUIC requires h3; a link that omits alpn must still end up with it.
        assert_eq!(tls.alpn, vec!["h3".to_string()]);
        assert_eq!(tls.server_name.as_deref(), Some("www.bing.com"));
    }

    #[test]
    fn tuic_without_a_password_is_rejected() {
        let error = parse_uri("tuic://11111111-2222-3333-4444-555555555555@x.example.com:443#x")
            .unwrap_err();
        assert_eq!(error.code, "tuic.missing_password");
    }

    #[test]
    fn hysteria2_keeps_obfuscation_and_forces_tls() {
        let uri = "hysteria2://secret@hy.example.com:443\
                   ?sni=www.bing.com&insecure=1&obfs=salamander&obfs-password=obfspw#Hy2";
        let node = parse_uri(uri).unwrap();

        assert_eq!(node.protocol, Protocol::Hysteria2);
        assert_eq!(node.name, "Hy2");
        match &node.credentials {
            Credentials::Hysteria2 {
                password,
                obfs,
                obfs_password,
            } => {
                assert_eq!(password, "secret");
                assert_eq!(obfs.as_deref(), Some("salamander"));
                assert_eq!(obfs_password.as_deref(), Some("obfspw"));
            }
            other => panic!("expected hysteria2, got {other:?}"),
        }

        // No `security` parameter exists in this link format; TLS is implied.
        let tls = node.tls.expect("hysteria2 is always TLS");
        assert_eq!(tls.server_name.as_deref(), Some("www.bing.com"));
        assert!(tls.allow_insecure);
    }

    #[test]
    fn hysteria2_without_obfs_password_is_rejected() {
        // The server would reject the connection anyway; failing at import says
        // why, instead of surfacing as a mysterious handshake failure.
        let error = parse_uri("hysteria2://pw@hy.example.com:443?obfs=salamander#x").unwrap_err();
        assert_eq!(error.code, "hysteria2.obfs_without_password");
    }

    #[test]
    fn unsupported_scheme_is_reported_not_panicked() {
        // hysteria2 used to live here as the example of an unknown scheme; it is
        // supported now, so the test needs a scheme that still is not.
        let error = parse_uri("ssr://cHc@example.com:443#x").unwrap_err();
        assert_eq!(error.code, "unsupported_scheme");
        assert!(error.message.contains("ssr"));
    }

    #[test]
    fn missing_scheme_is_reported() {
        let error = parse_uri("just some text").unwrap_err();
        assert_eq!(error.code, "missing_scheme");
    }

    #[test]
    fn garbage_after_scheme_is_reported() {
        let error = parse_uri("vless://nothing-here").unwrap_err();
        assert_eq!(error.code, "missing_userinfo");
    }
}
