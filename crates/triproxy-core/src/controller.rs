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

//! Clash-compatible controller API client.
//!
//! Mihomo and sing-box both expose the same HTTP API on the controller port
//! (Mihomo calls it `external-controller`, sing-box `clash_api`). That shared
//! surface is what makes live traffic figures, the connection list and *real*
//! proxy latency possible.
//!
//! **Xray has no equivalent.** It speaks its own gRPC API, which is a different
//! shape and a much larger surface. Anything built on this client is therefore
//! available for two cores out of three, and the UI has to say so rather than
//! showing an empty panel that looks like a bug.
//!
//! No secret is configured in the generated configs, so requests carry no
//! `Authorization` header. If that changes, this client needs the token too.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// How long to wait for the streaming `/traffic` endpoint to emit one sample.
const TRAFFIC_READ_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TrafficSample {
    /// Bytes per second outbound.
    #[serde(default)]
    pub up: u64,
    /// Bytes per second inbound.
    #[serde(default)]
    pub down: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConnectionsSnapshot {
    #[serde(rename = "uploadTotal", default)]
    pub upload_total: u64,
    #[serde(rename = "downloadTotal", default)]
    pub download_total: u64,
    // The API sends null rather than [] when there is nothing connected.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub connections: Vec<ConnectionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: String,
    #[serde(default)]
    pub metadata: ConnectionMetadata,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    /// Proxy chain, outermost last (e.g. `["PROXY", "Tokyo 01"]`).
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: String,
    #[serde(rename = "rulePayload", default)]
    pub rule_payload: String,
    #[serde(default)]
    pub start: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConnectionMetadata {
    #[serde(default)]
    pub network: String,
    /// Application-level guess (`HTTP`, `HTTPS`, ...).
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(rename = "sourceIP", default)]
    pub source_ip: String,
    #[serde(rename = "destinationIP", default)]
    pub destination_ip: String,
    #[serde(rename = "destinationPort", default)]
    pub destination_port: String,
    #[serde(default)]
    pub host: String,
    /// Process that opened the connection, when the core can tell.
    ///
    /// Mihomo reports a bare name on Windows and macOS; sing-box reports a full
    /// path in `processPath`. Both are parsed so the UI does not have to know
    /// which core answered.
    #[serde(default)]
    pub process: String,
    #[serde(rename = "processPath", default)]
    pub process_path: String,
}

impl ConnectionInfo {
    /// What to show in a list: the host when known, otherwise the address.
    pub fn target(&self) -> String {
        if !self.metadata.host.is_empty() {
            self.metadata.host.clone()
        } else if !self.metadata.destination_ip.is_empty() {
            format!(
                "{}:{}",
                self.metadata.destination_ip, self.metadata.destination_port
            )
        } else {
            "unknown".to_string()
        }
    }
}

pub struct ControllerClient {
    base: String,
    http: reqwest::Client,
}

impl ControllerClient {
    pub fn new(api_port: u16) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| Error::Other(format!("cannot build the API client: {error}")))?;
        Ok(Self {
            base: format!("http://127.0.0.1:{api_port}"),
            http,
        })
    }

    /// Liveness probe for the API itself.
    pub async fn version(&self) -> Result<String> {
        #[derive(Deserialize)]
        struct Version {
            #[serde(default)]
            version: String,
        }

        let response = self
            .http
            .get(format!("{}/version", self.base))
            .send()
            .await
            .map_err(|error| {
                Error::Other(format!(
                    "controller API not reachable on {}: {error}",
                    self.base
                ))
            })?;

        if !response.status().is_success() {
            return Err(Error::Other(format!(
                "controller API returned HTTP {}",
                response.status()
            )));
        }

        let version: Version = response
            .json()
            .await
            .map_err(|error| Error::Other(format!("cannot read /version: {error}")))?;
        Ok(version.version)
    }

    /// Read one traffic sample.
    ///
    /// `/traffic` is a streaming endpoint that emits a JSON object every second
    /// and never closes. Taking one sample means reading the first complete
    /// chunk and dropping the connection, so the caller is not left holding an
    /// open stream it never asked for.
    pub async fn traffic_once(&self) -> Result<TrafficSample> {
        let response = self
            .http
            .get(format!("{}/traffic", self.base))
            .send()
            .await
            .map_err(|error| Error::Other(format!("cannot open /traffic: {error}")))?;

        if !response.status().is_success() {
            return Err(Error::Other(format!(
                "/traffic returned HTTP {}",
                response.status()
            )));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        let read = tokio::time::timeout(TRAFFIC_READ_TIMEOUT, async {
            use futures_util::StreamExt;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| {
                    Error::Other(format!("cannot read the traffic stream: {error}"))
                })?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                // Samples are newline-free JSON objects; one complete object is
                // enough for a single reading.
                if let Some(sample) = parse_first_json_object(&buffer) {
                    return Ok(sample);
                }
            }
            Err(Error::Other(
                "the traffic stream closed without a sample".to_string(),
            ))
        })
        .await
        .map_err(|_| Error::Other("no traffic sample within the timeout".to_string()))?;

        read
    }

    pub async fn connections(&self) -> Result<ConnectionsSnapshot> {
        let response = self
            .http
            .get(format!("{}/connections", self.base))
            .send()
            .await
            .map_err(|error| Error::Other(format!("cannot read /connections: {error}")))?;

        if !response.status().is_success() {
            return Err(Error::Other(format!(
                "/connections returned HTTP {}",
                response.status()
            )));
        }

        response
            .json()
            .await
            .map_err(|error| Error::Other(format!("cannot parse /connections: {error}")))
    }

    pub async fn close_connection(&self, id: &str) -> Result<()> {
        let response = self
            .http
            .delete(format!("{}/connections/{id}", self.base))
            .send()
            .await
            .map_err(|error| Error::Other(format!("cannot close the connection: {error}")))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Other(format!(
                "closing the connection returned HTTP {}",
                response.status()
            )))
        }
    }

    pub async fn close_all(&self) -> Result<()> {
        let response = self
            .http
            .delete(format!("{}/connections", self.base))
            .send()
            .await
            .map_err(|error| Error::Other(format!("cannot close connections: {error}")))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Other(format!(
                "closing connections returned HTTP {}",
                response.status()
            )))
        }
    }

    /// Switch which node the PROXY group uses, at runtime.
    ///
    /// This is what makes node switching instant instead of a config rewrite
    /// plus a restart: the generated group is a selector, so the core will
    /// honour the choice immediately. Xray has no controller API, so there the
    /// choice only takes effect on the next start.
    pub async fn select_proxy(&self, group: &str, node_name: &str) -> Result<()> {
        let encoded = urlencode(group);
        let response = self
            .http
            .put(format!("{}/proxies/{encoded}", self.base))
            .json(&serde_json::json!({ "name": node_name }))
            .send()
            .await
            .map_err(|error| Error::Other(format!("cannot switch node: {error}")))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(Error::Other(format!(
                "switching to {node_name} failed (HTTP {status}): {body}"
            )))
        }
    }

    /// Which node the group currently uses, as reported by the core.
    pub async fn selected_proxy(&self, group: &str) -> Result<Option<String>> {
        #[derive(Deserialize)]
        struct Group {
            #[serde(default)]
            now: Option<String>,
        }

        let encoded = urlencode(group);
        let response = self
            .http
            .get(format!("{}/proxies/{encoded}", self.base))
            .send()
            .await
            .map_err(|error| Error::Other(format!("cannot read the group: {error}")))?;

        if !response.status().is_success() {
            return Ok(None);
        }
        let group: Group = response
            .json()
            .await
            .map_err(|error| Error::Other(format!("cannot parse the group: {error}")))?;
        Ok(group.now)
    }

    /// Real proxy latency: the core makes an HTTP request through the node.
    ///
    /// Unlike the TCP probe, this fails when the credentials or protocol are
    /// wrong — it is the only measurement that proves the proxy works.
    pub async fn proxy_delay(
        &self,
        proxy_name: &str,
        probe_url: &str,
        timeout_ms: u64,
    ) -> Result<u64> {
        #[derive(Deserialize)]
        struct Delay {
            #[serde(default)]
            delay: u64,
        }

        let encoded_name = urlencode(proxy_name);
        let response = self
            .http
            .get(format!("{}/proxies/{encoded_name}/delay", self.base))
            .query(&[("url", probe_url), ("timeout", &timeout_ms.to_string())])
            .send()
            .await
            .map_err(|error| Error::Other(format!("delay request failed: {error}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Other(format!(
                "delay probe for `{proxy_name}` failed (HTTP {status}): {body}"
            )));
        }

        let delay: Delay = response
            .json()
            .await
            .map_err(|error| Error::Other(format!("cannot parse the delay response: {error}")))?;
        Ok(delay.delay)
    }
}

/// Deserialize  into an empty vector.
///
/// Clash-compatible cores answer  when nothing is
/// connected; a plain  would reject that and make an idle core look broken.
fn null_as_empty<'de, D>(deserializer: D) -> std::result::Result<Vec<ConnectionInfo>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Vec<ConnectionInfo>>::deserialize(deserializer)?.unwrap_or_default())
}

/// Percent-encode a path segment. Proxy names contain spaces and CJK, which
/// cannot go into a URL path as-is.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Pull the first balanced `{...}` object out of a buffer.
fn parse_first_json_object(buffer: &str) -> Option<TrafficSample> {
    let start = buffer.find('{')?;
    let end = buffer[start..].find('}')? + start + 1;
    serde_json::from_str(&buffer[start..end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_traffic_object() {
        let sample = parse_first_json_object(r#"{"up":123,"down":456}"#).unwrap();
        assert_eq!(sample.up, 123);
        assert_eq!(sample.down, 456);
    }

    #[test]
    fn an_idle_core_reports_null_connections() {
        // The API really sends this shape when nothing is connected; treating it
        // as a parse error would make an idle core look broken.
        let snapshot: ConnectionsSnapshot = serde_json::from_str(
            r#"{"downloadTotal":0,"uploadTotal":0,"connections":null,"memory":0}"#,
        )
        .unwrap();
        assert!(snapshot.connections.is_empty());
    }

    #[test]
    fn parses_a_populated_connection_list() {
        let json = r#"{
            "downloadTotal": 1024,
            "uploadTotal": 512,
            "connections": [{
                "id": "abc",
                "metadata": {
                    "network": "tcp",
                    "type": "HTTPS",
                    "sourceIP": "127.0.0.1",
                    "destinationIP": "93.184.216.34",
                    "destinationPort": "443",
                    "host": "example.com",
                    "process": "chrome.exe"
                },
                "upload": 10,
                "download": 20,
                "chains": ["PROXY", "Tokyo 01"],
                "rule": "GEOSITE",
                "rulePayload": "cn",
                "start": "2026-11-01T22:00:00Z"
            }]
        }"#;
        let snapshot: ConnectionsSnapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snapshot.upload_total, 512);
        assert_eq!(snapshot.connections.len(), 1);
        let connection = &snapshot.connections[0];
        assert_eq!(connection.target(), "example.com");
        assert_eq!(connection.chains, vec!["PROXY", "Tokyo 01"]);
        // The list shows which rule caught a connection; without it "why did this
        // site go direct" is unanswerable from the UI.
        assert_eq!(connection.rule, "GEOSITE");
        assert_eq!(connection.rule_payload, "cn");
        assert_eq!(connection.metadata.process, "chrome.exe");
    }

    /// sing-box reports a full path instead of a name; both fields exist so the
    /// UI does not have to branch on which core answered.
    #[test]
    fn parses_a_process_path_from_sing_box() {
        let metadata: ConnectionMetadata =
            serde_json::from_str(r#"{"processPath":"C:\\Program Files\\App\\app.exe"}"#).unwrap();
        assert_eq!(metadata.process_path, "C:\\Program Files\\App\\app.exe");
        assert!(metadata.process.is_empty());
    }

    #[test]
    fn picks_the_first_object_when_several_arrive() {
        let sample = parse_first_json_object(r#"{"up":1,"down":2}{"up":3,"down":4}"#).unwrap();
        assert_eq!(sample.up, 1);
    }

    #[test]
    fn ignores_incomplete_json() {
        assert!(parse_first_json_object(r#"{"up":1,"do"#).is_none());
    }

    #[test]
    fn encodes_proxy_names_for_the_path() {
        assert_eq!(urlencode("Tokyo 01"), "Tokyo%2001");
        assert_eq!(urlencode("日本"), "%E6%97%A5%E6%9C%AC");
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn connection_target_prefers_the_host() {
        let info = ConnectionInfo {
            id: "1".to_string(),
            metadata: ConnectionMetadata {
                host: "example.com".to_string(),
                destination_ip: "1.2.3.4".to_string(),
                destination_port: "443".to_string(),
                ..Default::default()
            },
            upload: 0,
            download: 0,
            chains: vec!["PROXY".to_string()],
            rule: "Match".to_string(),
            rule_payload: String::new(),
            start: String::new(),
        };
        assert_eq!(info.target(), "example.com");

        let ip_only = ConnectionInfo {
            metadata: ConnectionMetadata {
                destination_ip: "1.2.3.4".to_string(),
                destination_port: "443".to_string(),
                ..Default::default()
            },
            ..info
        };
        assert_eq!(ip_only.target(), "1.2.3.4:443");
    }
}
