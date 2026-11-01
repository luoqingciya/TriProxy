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

use std::net::SocketAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::state::CoreState;

/// Result of a runtime health probe.
///
/// Phase 1 deliberately keeps health checking cheap and core-agnostic: process
/// liveness plus a TCP probe against the local inbound port. Core-specific
/// probes (Mihomo controller API, sing-box Clash API) belong to Phase 4+.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreHealth {
    pub state: CoreState,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    pub listen_port: Option<u16>,
    /// `Some(true)` when the local inbound accepted a TCP connection.
    pub listening: Option<bool>,
    /// TCP connect latency in milliseconds.
    pub latency_ms: Option<u64>,
    pub last_error: Option<String>,
}

impl CoreHealth {
    pub fn stopped() -> Self {
        Self {
            state: CoreState::Stopped,
            pid: None,
            uptime_secs: None,
            listen_port: None,
            listening: None,
            latency_ms: None,
            last_error: None,
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.state.is_running() && self.listening.unwrap_or(false)
    }
}

/// Attempt a TCP connection to a loopback port, returning latency in ms.
pub async fn probe_tcp(port: u16) -> Option<u64> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let started = std::time::Instant::now();
    match tokio::time::timeout(
        Duration::from_millis(800),
        tokio::net::TcpStream::connect(addr),
    )
    .await
    {
        Ok(Ok(_)) => Some(started.elapsed().as_millis() as u64),
        _ => None,
    }
}

/// Check whether a loopback port can still be bound.
///
/// Used as a pre-flight guard: starting a core that cannot bind would otherwise
/// surface as a confusing early exit.
pub fn port_is_free(port: u16) -> bool {
    use std::net::TcpListener;
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probe_reports_closed_port_as_none() {
        // Port 1 on loopback is not expected to be listening in a test sandbox.
        assert_eq!(probe_tcp(1).await, None);
    }

    #[tokio::test]
    async fn probe_succeeds_against_a_bound_listener() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        assert!(probe_tcp(port).await.is_some());
    }
}
