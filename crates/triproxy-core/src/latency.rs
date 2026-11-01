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

//! Node latency measurement.
//!
//! **What this measures, and what it does not.** The probe is a TCP handshake to
//! the node's own `server:port`. That answers "can I reach this endpoint, and how
//! far away is it", which is what users read a latency column for. It does *not*
//! prove the proxy works: wrong credentials, an expired UUID or a broken cipher
//! all still complete a TCP handshake. Proving that needs an HTTP request through
//! the running core, which requires a controller API and therefore only works for
//! mihomo and sing-box — so it is deliberately a separate measurement, not a
//! substitute that would silently mislabel TCP reachability as proxy health.
//!
//! Results are not persisted. Latency is a property of the moment; storing it
//! would invite the UI to show stale numbers as if they were current.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::error::{Error, Result};
use crate::logging::now_ms;
use crate::profile::Node;

/// How many probes run at once.
///
/// Enough to keep a hundred-node list from taking a minute, low enough that a
/// subscription's servers do not see a burst that looks like an attack.
const DEFAULT_CONCURRENCY: usize = 8;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyKind {
    /// TCP handshake to the node endpoint.
    Tcp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencySample {
    pub node_id: String,
    pub name: String,
    pub endpoint: String,
    pub kind: LatencyKind,
    /// Milliseconds on success, `None` when the probe failed.
    pub latency_ms: Option<u64>,
    /// Why the probe failed; `None` on success.
    pub error: Option<String>,
    pub measured_at_ms: u64,
}

impl LatencySample {
    pub fn reachable(&self) -> bool {
        self.latency_ms.is_some()
    }
}

/// Measure the TCP handshake time to one endpoint.
pub async fn tcp_latency(server: &str, port: u16, timeout: Duration) -> Result<u64> {
    let started = Instant::now();

    let lookup = tokio::time::timeout(timeout, tokio::net::lookup_host((server, port)))
        .await
        .map_err(|_| Error::Other("DNS lookup timed out".to_string()))?
        .map_err(|error| Error::Other(format!("DNS lookup failed: {error}")))?;

    let mut addresses = lookup.collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(Error::Other("no address for the node".to_string()));
    }
    // Prefer IPv4: dual-stack nodes are common and a broken IPv6 route would
    // otherwise report a healthy IPv4 endpoint as unreachable.
    addresses.sort_by_key(|address| !address.is_ipv4());

    let mut last_error = None;
    for address in addresses {
        match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(address)).await {
            Ok(Ok(_stream)) => return Ok(started.elapsed().as_millis() as u64),
            Ok(Err(error)) => last_error = Some(format!("connect failed: {error}")),
            Err(_) => last_error = Some("connect timed out".to_string()),
        }
    }

    Err(Error::Other(last_error.unwrap_or_else(|| {
        "no address could be reached".to_string()
    })))
}

/// Probe every node, at most `concurrency` at a time.
pub async fn measure_nodes(nodes: &[Node], timeout: Duration) -> Vec<LatencySample> {
    measure_nodes_with(nodes, timeout, DEFAULT_CONCURRENCY).await
}

pub async fn measure_nodes_with(
    nodes: &[Node],
    timeout: Duration,
    concurrency: usize,
) -> Vec<LatencySample> {
    let permits = std::sync::Arc::new(Semaphore::new(concurrency.max(1)));
    let mut tasks = Vec::with_capacity(nodes.len());

    for node in nodes {
        let node = node.clone();
        let permits = permits.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = permits.acquire().await.ok();
            let result = tcp_latency(&node.server, node.port, timeout).await;
            LatencySample {
                node_id: node.id.clone(),
                name: node.name.clone(),
                endpoint: node.endpoint(),
                kind: LatencyKind::Tcp,
                latency_ms: result.as_ref().ok().copied(),
                error: result.err().map(|error| error.to_string()),
                measured_at_ms: now_ms(),
            }
        }));
    }

    let mut samples = Vec::with_capacity(tasks.len());
    for task in tasks {
        if let Ok(sample) = task.await {
            samples.push(sample);
        }
    }
    // Stable order so the UI does not reshuffle between runs.
    samples.sort_by(|a, b| a.name.cmp(&b.name));
    samples
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Protocol;
    use crate::profile::{Credentials, Transport};

    fn node(server: &str, port: u16) -> Node {
        let mut node = Node {
            id: String::new(),
            name: format!("{server}:{port}"),
            protocol: Protocol::Trojan,
            server: server.to_string(),
            port,
            credentials: Credentials::Trojan {
                password: "pw".to_string(),
            },
            transport: Transport::Tcp,
            tls: None,
            udp: true,
            plugin: None,
        };
        node.refresh_id();
        node
    }

    #[tokio::test]
    async fn a_listening_port_measures_in_milliseconds() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        let latency = tcp_latency("127.0.0.1", port, Duration::from_secs(2))
            .await
            .expect("loopback should connect");
        assert!(latency < 1000, "unexpected latency {latency}ms");
    }

    #[tokio::test]
    async fn a_closed_port_is_reported_as_an_error() {
        // Port 1 on loopback is never a proxy; connecting is refused instantly.
        let error = tcp_latency("127.0.0.1", 1, Duration::from_millis(500))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("connect"));
    }

    #[tokio::test]
    async fn measure_reports_one_sample_per_node_in_stable_order() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        let nodes = vec![node("127.0.0.1", port), node("127.0.0.1", 1)];
        let samples = measure_nodes_with(&nodes, Duration::from_millis(500), 2).await;

        assert_eq!(samples.len(), 2);
        let reachable = samples.iter().filter(|sample| sample.reachable()).count();
        assert_eq!(reachable, 1);

        // Look up by endpoint: name order is lexicographic, so the closed port
        // (":1") sorts before an ephemeral one and an index assertion would be
        // asserting the wrong thing.
        let open = samples
            .iter()
            .find(|sample| sample.endpoint == format!("127.0.0.1:{port}"))
            .expect("sample for the listening port");
        assert!(open.reachable(), "{open:?}");

        let closed = samples
            .iter()
            .find(|sample| sample.endpoint == "127.0.0.1:1")
            .expect("sample for the closed port");
        assert!(closed.error.is_some(), "{closed:?}");
    }

    #[tokio::test]
    async fn empty_input_yields_no_samples() {
        assert!(measure_nodes(&[], Duration::from_millis(100))
            .await
            .is_empty());
    }
}
