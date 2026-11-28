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

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{OnceCell, Semaphore};

use crate::error::{Error, Result};
use crate::logging::now_ms;
use crate::profile::Node;

/// How many TCP probes run at once.
///
/// Enough to keep a hundred-node list from taking long, low enough that a
/// subscription's servers do not see a burst that looks like an attack.
const DEFAULT_CONCURRENCY: usize = 16;
const PROXY_DELAY_CONCURRENCY: usize = 12;

type DnsCache = Arc<
    tokio::sync::Mutex<HashMap<String, Arc<OnceCell<std::result::Result<Vec<IpAddr>, String>>>>>,
>;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyKind {
    /// TCP handshake to the node endpoint.
    Tcp,
    /// HTTP request made by the core through the node. Only possible while a
    /// Mihomo or sing-box instance is running with its controller API.
    Http,
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
    let deadline = Instant::now() + timeout;
    let ips = if let Ok(ip) = server.parse::<IpAddr>() {
        vec![ip]
    } else {
        resolve_host_ips(server, timeout)
            .await
            .map_err(Error::Other)?
    };
    connect_ips(&ips, port, deadline).await
}

async fn tcp_latency_with_cache(
    server: &str,
    port: u16,
    timeout: Duration,
    cache: &DnsCache,
) -> Result<u64> {
    let deadline = Instant::now() + timeout;
    let ips = if let Ok(ip) = server.parse::<IpAddr>() {
        vec![ip]
    } else {
        resolve_cached(cache, server, timeout).await?
    };
    connect_ips(&ips, port, deadline).await
}

async fn resolve_host_ips(
    server: &str,
    timeout: Duration,
) -> std::result::Result<Vec<IpAddr>, String> {
    let lookup = tokio::time::timeout(timeout, tokio::net::lookup_host((server, 0)))
        .await
        .map_err(|_| "DNS lookup timed out".to_string())?
        .map_err(|error| format!("DNS lookup failed: {error}"))?;

    let mut ips = Vec::new();
    for address in lookup {
        if !ips.contains(&address.ip()) {
            ips.push(address.ip());
        }
    }
    if ips.is_empty() {
        return Err("no address for the node".to_string());
    }
    // Prefer IPv4: dual-stack nodes are common and a broken IPv6 route would
    // otherwise report a healthy IPv4 endpoint as unreachable.
    ips.sort_by_key(|ip| !ip.is_ipv4());
    Ok(ips)
}

async fn resolve_cached(cache: &DnsCache, server: &str, timeout: Duration) -> Result<Vec<IpAddr>> {
    let key = server.to_ascii_lowercase();
    let cell = {
        let mut guard = cache.lock().await;
        guard
            .entry(key)
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .clone()
    };
    let server_owned = server.to_string();
    let result = cell
        .get_or_init(|| async move { resolve_host_ips(&server_owned, timeout).await })
        .await;
    result.clone().map_err(Error::Other)
}

async fn connect_ips(ips: &[IpAddr], port: u16, deadline: Instant) -> Result<u64> {
    let mut last_error = None;
    for &ip in ips {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            last_error.get_or_insert_with(|| "connect timed out".to_string());
            break;
        }
        let address = SocketAddr::new(ip, port);
        let connect_started = Instant::now();
        match tokio::time::timeout(remaining, tokio::net::TcpStream::connect(address)).await {
            Ok(Ok(_stream)) => return Ok(connect_started.elapsed().as_millis() as u64),
            Ok(Err(error)) => last_error = Some(format!("connect failed: {error}")),
            Err(_) => last_error = Some("connect timed out".to_string()),
        }
    }

    Err(Error::Other(last_error.unwrap_or_else(|| {
        "no address could be reached".to_string()
    })))
}

/// Default URL for the HTTP probe.
///
/// A 204 endpoint is the convention: it returns no body, so the measurement is
/// the round trip rather than the download.
pub const DEFAULT_PROBE_URL: &str = "http://www.gstatic.com/generate_204";

/// Measure real proxy delay through the running core, concurrently.
///
/// This is the only measurement that proves a node works: the core opens an HTTP
/// request through the node, so wrong credentials or a broken protocol fail here
/// while a TCP handshake would still succeed. Requires a controller API, which
/// Xray does not have.
pub async fn measure_proxy_delay(
    client: &crate::controller::ControllerClient,
    nodes: &[Node],
    probe_url: &str,
    timeout_ms: u64,
) -> Vec<LatencySample> {
    measure_proxy_delay_with(
        client,
        nodes,
        probe_url,
        timeout_ms,
        PROXY_DELAY_CONCURRENCY,
    )
    .await
}

pub async fn measure_proxy_delay_with(
    client: &crate::controller::ControllerClient,
    nodes: &[Node],
    probe_url: &str,
    timeout_ms: u64,
    concurrency: usize,
) -> Vec<LatencySample> {
    measure_proxy_delay_streaming(client, nodes, probe_url, timeout_ms, concurrency, |_| {}).await
}

pub async fn measure_proxy_delay_streaming<F>(
    client: &crate::controller::ControllerClient,
    nodes: &[Node],
    probe_url: &str,
    timeout_ms: u64,
    concurrency: usize,
    on_sample: F,
) -> Vec<LatencySample>
where
    F: Fn(&LatencySample) + Send + Sync + 'static,
{
    let permits = Arc::new(Semaphore::new(concurrency.max(1)));
    let probe_url = probe_url.to_string();
    let on_sample = Arc::new(on_sample);
    let mut tasks = FuturesUnordered::new();

    for node in nodes {
        let node = node.clone();
        let client = client.clone();
        let probe_url = probe_url.clone();
        let permits = permits.clone();
        let on_sample = on_sample.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = permits.acquire().await.ok();
            let result = client.proxy_delay(&node.name, &probe_url, timeout_ms).await;
            let sample = LatencySample {
                node_id: node.id.clone(),
                name: node.name.clone(),
                endpoint: node.endpoint(),
                kind: LatencyKind::Http,
                latency_ms: result.as_ref().ok().copied(),
                error: result.err().map(|error| error.to_string()),
                measured_at_ms: now_ms(),
            };
            on_sample(&sample);
            sample
        }));
    }

    let mut samples = Vec::with_capacity(nodes.len());
    while let Some(res) = tasks.next().await {
        if let Ok(sample) = res {
            samples.push(sample);
        }
    }
    samples.sort_by(|a, b| a.name.cmp(&b.name));
    samples
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
    measure_nodes_streaming(nodes, timeout, concurrency, |_| {}).await
}

pub async fn measure_nodes_streaming<F>(
    nodes: &[Node],
    timeout: Duration,
    concurrency: usize,
    on_sample: F,
) -> Vec<LatencySample>
where
    F: Fn(&LatencySample) + Send + Sync + 'static,
{
    let permits = Arc::new(Semaphore::new(concurrency.max(1)));
    let cache: DnsCache = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let on_sample = Arc::new(on_sample);
    let mut tasks = FuturesUnordered::new();

    for node in nodes {
        let node = node.clone();
        let permits = permits.clone();
        let cache = cache.clone();
        let on_sample = on_sample.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = permits.acquire().await.ok();
            let result = tcp_latency_with_cache(&node.server, node.port, timeout, &cache).await;
            let sample = LatencySample {
                node_id: node.id.clone(),
                name: node.name.clone(),
                endpoint: node.endpoint(),
                kind: LatencyKind::Tcp,
                latency_ms: result.as_ref().ok().copied(),
                error: result.err().map(|error| error.to_string()),
                measured_at_ms: now_ms(),
            };
            on_sample(&sample);
            sample
        }));
    }

    let mut samples = Vec::with_capacity(nodes.len());
    while let Some(res) = tasks.next().await {
        if let Ok(sample) = res {
            samples.push(sample);
        }
    }
    // Stable order so the UI does not reshuffle between runs.
    samples.sort_by(|a, b| a.name.cmp(&b.name));
    samples
}

/// Delay of whatever the proxy group is currently using.
///
/// The status payload already carries a `latency_ms`, but that is the loopback
/// handshake against the inbound port — always under a millisecond, which is why
/// every dashboard rendered `0 ms`. This is the number people think they are
/// reading: one HTTP request made by the core through the selected node.
///
/// Failures are data, not errors: "Xray has no controller API" and "no node
/// picked yet" are permanent states of a running setup, and the caller needs to
/// tell the user *why* there is no figure instead of showing a bare dash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveDelay {
    /// Node the group resolved to, as reported by the core.
    pub node: Option<String>,
    /// Milliseconds, or `None` when the probe failed or is unavailable.
    pub latency_ms: Option<u64>,
    /// Why there is no figure; `None` on success.
    pub error: Option<String>,
}

impl ActiveDelay {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            node: None,
            latency_ms: None,
            error: Some(reason.into()),
        }
    }
}

/// Measure the group's current node, end to end.
pub async fn active_delay(
    client: &crate::controller::ControllerClient,
    group: &str,
    probe_url: &str,
    timeout_ms: u64,
) -> ActiveDelay {
    let node = match client.selected_proxy(group).await {
        Ok(Some(node)) => node,
        Ok(None) => return ActiveDelay::unavailable("核心还没有选定节点"),
        Err(error) => return ActiveDelay::unavailable(error.to_string()),
    };

    match client.proxy_delay(&node, probe_url, timeout_ms).await {
        Ok(latency_ms) => ActiveDelay {
            node: Some(node),
            latency_ms: Some(latency_ms),
            error: None,
        },
        Err(error) => ActiveDelay {
            node: Some(node),
            latency_ms: None,
            error: Some(error.to_string()),
        },
    }
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

    #[tokio::test]
    async fn streaming_measurement_emits_every_sample() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let nodes = vec![node("localhost", port), node("localhost", 1)];

        let streamed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = streamed.clone();
        let samples =
            measure_nodes_streaming(&nodes, Duration::from_millis(500), 2, move |sample| {
                sink.lock().unwrap().push(sample.node_id.clone());
            })
            .await;

        assert_eq!(samples.len(), 2);
        assert_eq!(streamed.lock().unwrap().len(), 2);
    }
}
