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

pub mod mihomo;
pub mod singbox;
pub mod xray;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use crate::bootstrap::BootstrapOptions;
use crate::capability::CoreCapability;
use crate::core::{CoreKind, ProxyCore};
use crate::error::{Error, Result};
use crate::health::CoreHealth;
use crate::logging::LogBuffer;
use crate::process::{run_capture, Supervisor, DEFAULT_READY_TIMEOUT};
use crate::state::CoreState;

/// Per-core knowledge: how to talk to this specific binary.
///
/// Keeping the differences behind one small trait is what allows a single
/// `ManagedCore` to implement `ProxyCore` for all three cores, instead of three
/// near-duplicate implementations that drift apart.
pub trait CoreDialect: Send + Sync {
    fn kind(&self) -> CoreKind;

    fn capability(&self) -> CoreCapability;

    fn version_args(&self) -> Vec<OsString>;

    fn parse_version(&self, output: &str) -> Option<String>;

    /// Arguments that make the core validate a config and exit.
    fn check_args(&self, config: &Path) -> Vec<OsString>;

    /// Arguments that make the core run with a config.
    fn run_args(&self, config: &Path) -> Vec<OsString>;

    fn bootstrap(&self, options: &BootstrapOptions) -> String;
}

pub struct ManagedCore {
    dialect: Box<dyn CoreDialect>,
    supervisor: Supervisor,
    listen_port: AtomicU16,
}

impl ManagedCore {
    pub fn new(
        dialect: Box<dyn CoreDialect>,
        binary: PathBuf,
        logs: Arc<LogBuffer>,
        log_file: Option<PathBuf>,
    ) -> Self {
        let listen_port = dialect.kind().default_listen_port();
        let supervisor = Supervisor::new(dialect.kind(), binary, logs, log_file);
        Self {
            dialect,
            supervisor,
            listen_port: AtomicU16::new(listen_port),
        }
    }

    pub fn supervisor(&self) -> &Supervisor {
        &self.supervisor
    }
}

#[async_trait]
impl ProxyCore for ManagedCore {
    fn kind(&self) -> CoreKind {
        self.dialect.kind()
    }

    fn capability(&self) -> CoreCapability {
        self.dialect.capability()
    }

    fn binary_path(&self) -> &Path {
        self.supervisor.binary_path()
    }

    fn listen_port(&self) -> u16 {
        self.listen_port.load(Ordering::Relaxed)
    }

    fn set_listen_port(&self, port: u16) {
        self.listen_port.store(port, Ordering::Relaxed);
    }

    fn bootstrap_config(&self, api_port: u16, log_level: &str) -> String {
        self.dialect.bootstrap(&BootstrapOptions {
            listen_port: self.listen_port(),
            api_port,
            log_level: log_level.to_string(),
        })
    }

    async fn version(&self) -> Result<String> {
        if let Some(cached) = self.supervisor.cached_version() {
            return Ok(cached.clone());
        }
        let output = run_capture(self.binary_path(), &self.dialect.version_args()).await?;
        if !output.success {
            return Err(Error::Other(format!(
                "`{}` failed to report its version: {}",
                self.kind(),
                output.combined()
            )));
        }
        let version = self
            .dialect
            .parse_version(&output.stdout)
            .or_else(|| self.dialect.parse_version(&output.stderr))
            .unwrap_or_else(|| output.combined());
        self.supervisor.set_cached_version(version.clone());
        Ok(version)
    }

    async fn check_config(&self, config: &Path) -> Result<()> {
        let output = run_capture(self.binary_path(), &self.dialect.check_args(config)).await?;
        if output.success {
            return Ok(());
        }
        let detail = output.combined();
        Err(Error::ConfigInvalid {
            core: self.kind().as_str().to_string(),
            reason: if detail.is_empty() {
                format!("exit code {:?}", output.code)
            } else {
                detail
            },
        })
    }

    async fn start(&self, config: &Path) -> Result<()> {
        self.supervisor
            .spawn(
                config,
                self.dialect.run_args(config),
                self.listen_port(),
                DEFAULT_READY_TIMEOUT,
            )
            .await
    }

    async fn stop(&self) -> Result<()> {
        self.supervisor.stop().await
    }

    async fn restart(&self, config: &Path) -> Result<()> {
        self.supervisor.stop().await?;
        self.start(config).await
    }

    async fn state(&self) -> CoreState {
        self.supervisor.state().await
    }

    async fn pid(&self) -> Option<u32> {
        self.supervisor.pid().await
    }

    async fn adopt(&self, pid: u32, port: u16) -> bool {
        self.supervisor.adopt(pid, port, None).await
    }

    async fn health_check(&self) -> Result<CoreHealth> {
        Ok(self.supervisor.health().await)
    }

    fn logs(&self) -> Arc<LogBuffer> {
        self.supervisor.logs().clone()
    }
}

/// Find the first version-looking token in a banner line.
///
/// The three cores print wildly different banners ("Mihomo Meta v1.19.12",
/// "sing-box version 1.11.15", "Xray 25.9.11"), but all of them contain a
/// `major.minor[.patch]` token, optionally prefixed with `v`.
pub(crate) fn first_version_token(output: &str) -> Option<String> {
    for line in output.lines() {
        for token in line.split_whitespace() {
            let cleaned = token.trim_matches(|c: char| {
                !c.is_ascii_alphanumeric() && c != '.' && c != '-' && c != '_'
            });
            let candidate = cleaned.strip_prefix('v').unwrap_or(cleaned);
            let mut parts = candidate.split('.');
            let (Some(major), Some(minor)) = (parts.next(), parts.next()) else {
                continue;
            };
            if !major.chars().all(|c| c.is_ascii_digit()) || major.is_empty() {
                continue;
            }
            if !minor.chars().all(|c| c.is_ascii_digit()) || minor.is_empty() {
                continue;
            }
            return Some(candidate.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_token_handles_all_three_banners() {
        assert_eq!(
            first_version_token("Mihomo Meta v1.19.12 windows amd64 with go1.23.4"),
            Some("1.19.12".to_string())
        );
        assert_eq!(
            first_version_token("sing-box version 1.11.15"),
            Some("1.11.15".to_string())
        );
        assert_eq!(
            first_version_token("Xray 25.9.11 (Xray, Penetrates Everything.)"),
            Some("25.9.11".to_string())
        );
    }

    #[test]
    fn version_token_ignores_plain_words() {
        assert_eq!(first_version_token("alpha build, no version here"), None);
    }
}
