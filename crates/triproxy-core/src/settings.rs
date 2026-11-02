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

//! Application settings.
//!
//! Kept in the core crate rather than the GUI so the CLI and the desktop app
//! read the same file: a setting toggled in the UI must affect `triproxy start`
//! too, otherwise the two frontends quietly disagree.

use serde::{Deserialize, Serialize};

use crate::compiler::CompileOptions;
use crate::core::CoreKind;
use crate::error::{Error, Result};
use crate::paths::Layout;
use crate::selector::{CorePreference, NetworkRequirements};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Light,
    #[default]
    Dark,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub theme: Theme,
    /// `None` means Smart selection.
    #[serde(default)]
    pub core_preference: Option<CoreKind>,
    /// Application-side requirements that feed the core selector.
    #[serde(default)]
    pub tun: bool,
    #[serde(default)]
    pub rule_set: bool,
    #[serde(default)]
    pub proxy_groups: bool,
    #[serde(default)]
    pub allow_lan: bool,
    /// Install the system proxy automatically when a core starts.
    #[serde(default)]
    pub system_proxy: bool,
    /// DNS strategy for the generated config.
    #[serde(default)]
    pub dns_mode: crate::compiler::DnsMode,
    /// Routing strategy for the generated config.
    #[serde(default)]
    pub rule_mode: crate::compiler::RuleMode,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// Local inbound port for the mixed listener.
    ///
    /// Fixed at 7890 until now, which made TriProxy unusable on a machine where
    /// something else already holds that port — and there is no fallback, the
    /// core simply fails to bind.
    #[serde(default = "default_listen_port")]
    pub listen_port: u16,
    /// Keep running in the system tray when the window is closed.
    ///
    /// Defaults to true: a proxy client that exits on close would cut the
    /// connection the user is in the middle of using.
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
    /// Start a core as soon as the app launches.
    ///
    /// Off by default: an app that opens connections before the user asks is a
    /// surprise, and on Windows it also means an elevated process starting
    /// unattended at login.
    #[serde(default)]
    pub auto_connect: bool,
    /// Semicolon-separated list of destinations that bypass the system proxy.
    ///
    /// `None` keeps the built-in list. Users who work against a private network
    /// need to add their own ranges, and a hard-coded list cannot know them.
    #[serde(default)]
    pub proxy_bypass: Option<String>,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_listen_port() -> u16 {
    7890
}

fn default_close_to_tray() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            core_preference: None,
            tun: false,
            rule_set: false,
            proxy_groups: false,
            allow_lan: false,
            system_proxy: false,
            dns_mode: crate::compiler::DnsMode::Off,
            rule_mode: crate::compiler::RuleMode::Global,
            log_level: default_log_level(),
            listen_port: default_listen_port(),
            close_to_tray: default_close_to_tray(),
            auto_connect: false,
            proxy_bypass: None,
        }
    }
}

impl AppSettings {
    fn path(layout: &Layout) -> std::path::PathBuf {
        layout.root().join("settings.json")
    }

    /// Load settings, falling back to defaults.
    ///
    /// A malformed settings file must never prevent the app from starting: the
    /// user would be unable to reach the settings screen that could fix it.
    pub fn load(layout: &Layout) -> Self {
        let path = Self::path(layout);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str(&text) {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "ignoring unreadable settings file");
                Self::default()
            }
        }
    }

    pub fn save(&self, layout: &Layout) -> Result<()> {
        layout.ensure()?;
        let path = Self::path(layout);
        let text = serde_json::to_string_pretty(self)?;
        let staging = path.with_extension("tmp");
        std::fs::write(&staging, text).map_err(|e| Error::io(&staging, e))?;
        std::fs::rename(&staging, &path).map_err(|e| Error::io(&path, e))?;
        Ok(())
    }

    /// Requirements the selector should apply on top of the profile.
    pub fn network(&self) -> NetworkRequirements {
        NetworkRequirements {
            tun: self.tun,
            dns: false,
            rule_set: self.rule_set,
            proxy_groups: self.proxy_groups,
        }
    }

    pub fn preference(&self) -> CorePreference {
        match self.core_preference {
            Some(core) => CorePreference::Fixed(core),
            None => CorePreference::Smart,
        }
    }

    pub fn compile_options(&self, listen_port: u16, api_port: u16) -> CompileOptions {
        CompileOptions {
            listen_port,
            api_port,
            log_level: self.log_level.clone(),
            allow_lan: self.allow_lan,
            tun: self.tun,
            dns: self.dns_mode,
            rules: self.rule_mode,
            selected_node: None,
            selected_name: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(name: &str) -> Layout {
        let root = std::env::temp_dir().join(format!(
            "triproxy-settings-{name}-{}-{}",
            std::process::id(),
            crate::logging::now_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let layout = Layout::new(root);
        layout.ensure().unwrap();
        layout
    }

    #[test]
    fn defaults_are_smart_and_off() {
        let settings = AppSettings::default();
        assert_eq!(settings.preference(), CorePreference::Smart);
        assert!(!settings.tun);
        assert_eq!(settings.theme, Theme::Dark);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let layout = layout("roundtrip");
        let settings = AppSettings {
            theme: Theme::Light,
            core_preference: Some(CoreKind::Xray),
            tun: true,
            rule_set: true,
            proxy_groups: false,
            allow_lan: true,
            system_proxy: true,
            dns_mode: crate::compiler::DnsMode::Split,
            rule_mode: crate::compiler::RuleMode::Split,
            log_level: "debug".to_string(),
            listen_port: 1080,
            close_to_tray: false,
            auto_connect: true,
            proxy_bypass: Some("localhost;*.internal".to_string()),
        };
        settings.save(&layout).unwrap();

        let loaded = AppSettings::load(&layout);
        assert_eq!(loaded, settings);
        assert_eq!(loaded.preference(), CorePreference::Fixed(CoreKind::Xray));
        assert!(loaded.network().tun);
        assert!(loaded.network().rule_set);
    }

    #[test]
    fn missing_file_yields_defaults() {
        let layout = layout("missing");
        assert_eq!(AppSettings::load(&layout), AppSettings::default());
        // Tray residency is on by default: closing the window must not cut the
        // connection the user is using.
        assert!(AppSettings::default().close_to_tray);
    }

    #[test]
    fn settings_file_from_before_the_tray_option_still_loads() {
        let layout = layout("legacy-tray");
        std::fs::write(layout.root().join("settings.json"), r#"{"theme":"dark"}"#).unwrap();
        assert!(AppSettings::load(&layout).close_to_tray);
    }

    #[test]
    fn corrupt_file_yields_defaults_instead_of_failing() {
        let layout = layout("corrupt");
        std::fs::write(layout.root().join("settings.json"), "{ not json").unwrap();
        assert_eq!(AppSettings::load(&layout), AppSettings::default());
    }

    #[test]
    fn compile_options_carry_the_network_settings() {
        let settings = AppSettings {
            allow_lan: true,
            log_level: "warn".to_string(),
            ..Default::default()
        };
        let options = settings.compile_options(7890, 9090);
        assert!(options.allow_lan);
        assert_eq!(options.log_level, "warn");
        assert_eq!(options.listen_port, 7890);
    }
}
