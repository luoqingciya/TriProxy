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

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::capability::CoreCapability;
use crate::error::Result;
use crate::health::CoreHealth;
use crate::logging::LogBuffer;
use crate::state::CoreState;

pub mod manager;

pub use manager::{
    CoreManager, CoreReport, ManagerStatus, RecoveryReport, StartReport, SwitchReport, SwitchStep,
};

/// The three supported proxy cores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoreKind {
    Mihomo,
    SingBox,
    Xray,
}

impl CoreKind {
    pub const ALL: [CoreKind; 3] = [CoreKind::Mihomo, CoreKind::SingBox, CoreKind::Xray];

    /// Canonical identifier used in CLI arguments, config paths and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            CoreKind::Mihomo => "mihomo",
            CoreKind::SingBox => "sing-box",
            CoreKind::Xray => "xray",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            CoreKind::Mihomo => "Mihomo",
            CoreKind::SingBox => "sing-box",
            CoreKind::Xray => "Xray",
        }
    }

    /// Executable file stem, also the name used inside release archives.
    pub fn exe_stem(self) -> &'static str {
        self.as_str()
    }

    /// GitHub repository that publishes the official binary.
    pub fn repo(self) -> &'static str {
        match self {
            CoreKind::Mihomo => "MetaCubeX/mihomo",
            CoreKind::SingBox => "SagerNet/sing-box",
            CoreKind::Xray => "XTLS/Xray-core",
        }
    }

    /// Environment variable that overrides the resolved binary path.
    pub fn bin_env_var(self) -> &'static str {
        match self {
            CoreKind::Mihomo => "TRIPROXY_MIHOMO_BIN",
            CoreKind::SingBox => "TRIPROXY_SING_BOX_BIN",
            CoreKind::Xray => "TRIPROXY_XRAY_BIN",
        }
    }

    pub fn default_listen_port(self) -> u16 {
        7890
    }

    /// Arguments that make the binary print its version and exit.
    ///
    /// Kept here (not in the adapters) because the binary manager needs it to
    /// self-check a freshly downloaded binary before it is trusted.
    pub fn version_probe_args(self) -> &'static [&'static str] {
        match self {
            CoreKind::Mihomo => &["-v"],
            CoreKind::SingBox | CoreKind::Xray => &["version"],
        }
    }

    /// Whether this core exposes an HTTP controller on `api_port`.
    pub fn has_controller(self) -> bool {
        matches!(self, CoreKind::Mihomo | CoreKind::SingBox)
    }
}

impl std::fmt::Display for CoreKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CoreKind {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mihomo" | "clash" | "clash-meta" | "meta" => Ok(CoreKind::Mihomo),
            "sing-box" | "singbox" | "sing_box" | "sb" => Ok(CoreKind::SingBox),
            "xray" | "x-ray" | "v2ray" => Ok(CoreKind::Xray),
            other => Err(format!(
                "unknown core `{other}` (expected one of: mihomo, sing-box, xray)"
            )),
        }
    }
}

/// The unified core interface.
///
/// Every method takes `&self`: lifecycle state lives behind an internal async
/// mutex inside the implementation, so `CoreManager` can hold
/// `Arc<dyn ProxyCore>` and serialize operations itself instead of nesting locks.
#[async_trait]
pub trait ProxyCore: Send + Sync {
    fn kind(&self) -> CoreKind;

    fn capability(&self) -> CoreCapability;

    fn binary_path(&self) -> &Path;

    fn listen_port(&self) -> u16;

    fn set_listen_port(&self, port: u16);

    /// Render a minimal, direct-only config for this core.
    ///
    /// Phase-1 placeholder so that lifecycle work can be validated without the
    /// Unified Profile. Phase 3 replaces this with `ConfigCompiler`; the method
    /// must not grow into a second compiler in the meantime.
    fn bootstrap_config(&self, api_port: u16, log_level: &str) -> String;

    /// Version string reported by the binary itself.
    async fn version(&self) -> Result<String>;

    /// Ask the core to validate a config file without starting it.
    async fn check_config(&self, config: &Path) -> Result<()>;

    /// Start the core with the given config and wait until it serves traffic.
    async fn start(&self, config: &Path) -> Result<()>;

    async fn stop(&self) -> Result<()>;

    async fn restart(&self, config: &Path) -> Result<()>;

    async fn state(&self) -> CoreState;

    async fn pid(&self) -> Option<u32>;

    /// Take over an already-running process discovered from persisted state.
    /// Returns `false` when that PID is no longer alive.
    async fn adopt(&self, pid: u32, port: u16) -> bool;

    async fn health_check(&self) -> Result<CoreHealth>;

    fn logs(&self) -> Arc<LogBuffer>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_kind_parses_aliases() {
        assert_eq!(CoreKind::from_str("Mihomo").unwrap(), CoreKind::Mihomo);
        assert_eq!(CoreKind::from_str("clash").unwrap(), CoreKind::Mihomo);
        assert_eq!(CoreKind::from_str("singbox").unwrap(), CoreKind::SingBox);
        assert_eq!(CoreKind::from_str("sing-box").unwrap(), CoreKind::SingBox);
        assert_eq!(CoreKind::from_str("XRAY").unwrap(), CoreKind::Xray);
        assert!(CoreKind::from_str("v2rayng").is_err());
    }

    #[test]
    fn core_kind_roundtrips_through_json() {
        for kind in CoreKind::ALL {
            let text = serde_json::to_string(&kind).unwrap();
            assert_eq!(text, format!("\"{}\"", kind.as_str()));
            assert_eq!(serde_json::from_str::<CoreKind>(&text).unwrap(), kind);
        }
    }
}
