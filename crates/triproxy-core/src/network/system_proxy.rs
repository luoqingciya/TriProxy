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

//! System proxy management.
//!
//! The hard part is not setting the proxy, it is **not leaving the machine in a
//! broken state**. Three rules make that hold:
//!
//! 1. The previous settings are written to disk *before* the system is touched.
//!    A crash between the write and the change leaves a snapshot whose restore
//!    is harmless (it puts back the values that are already there).
//! 2. Enabling twice never overwrites the snapshot with our own values, so the
//!    original state survives repeated toggles.
//! 3. Startup calls [`SystemProxyManager::recover`], which restores a leftover
//!    snapshot from a previous crash before anything else runs.
//!
//! Additionally, [`SystemProxyManager::enable`] refuses to point the system at a
//! port nothing is listening on: switching the whole machine to a dead endpoint
//! is worse than not switching at all.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::health::probe_tcp_blocking;
use crate::logging::now_ms;
use crate::paths::Layout;

/// What the system currently routes through, as far as TriProxy knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedProxy {
    pub endpoint: String,
    pub backend: String,
    pub applied_at_ms: u64,
}

/// One platform's way of setting the system proxy.
///
/// `capture` returns an opaque JSON blob: each platform snapshots different
/// fields, and the snapshot file is meant to be readable by a human debugging a
/// leftover state, so a typed union would add ceremony without adding safety.
pub trait ProxyBackend: Send + Sync {
    fn name(&self) -> &'static str;

    /// False when the platform or desktop environment has no proxy setting this
    /// build knows how to drive.
    fn supported(&self) -> bool;

    fn capture(&self) -> Result<Value>;

    fn apply(&self, endpoint: &str, bypass: &str) -> Result<()>;

    fn restore(&self, snapshot: &Value) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotFile {
    backend: String,
    endpoint: String,
    applied_at_ms: u64,
    /// Verbatim platform state from before TriProxy touched anything.
    previous: Value,
}

/// Default bypass list, matching what mainstream clients use.
pub const DEFAULT_BYPASS: &str = "<local>;localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*";

pub struct SystemProxyManager {
    snapshot_path: PathBuf,
    backend: Box<dyn ProxyBackend>,
}

impl SystemProxyManager {
    pub fn new(layout: &Layout) -> Self {
        Self {
            snapshot_path: layout.root().join("system-proxy.json"),
            backend: platform_backend(),
        }
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    pub fn supported(&self) -> bool {
        self.backend.supported()
    }

    /// The proxy TriProxy believes it has installed, if any.
    pub fn status(&self) -> Option<AppliedProxy> {
        self.read_snapshot().ok().map(|snapshot| AppliedProxy {
            endpoint: snapshot.endpoint,
            backend: snapshot.backend,
            applied_at_ms: snapshot.applied_at_ms,
        })
    }

    /// Point the system proxy at `endpoint` (`127.0.0.1:7890`).
    ///
    /// Idempotent: calling it again with a different endpoint re-applies, but
    /// keeps the original snapshot so a later disable still returns the machine
    /// to its pre-TriProxy state.
    pub fn enable(&self, endpoint: &str) -> Result<AppliedProxy> {
        if !self.backend.supported() {
            return Err(Error::Other(format!(
                "system proxy is not supported on this platform (backend `{}`)",
                self.backend.name()
            )));
        }

        let port = endpoint
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse::<u16>().ok())
            .ok_or_else(|| Error::Other(format!("invalid proxy endpoint `{endpoint}`")))?;

        // Refuse to hand the whole machine to a dead port.
        if probe_tcp_blocking(port, std::time::Duration::from_millis(500)).is_none() {
            return Err(Error::Other(format!(
                "nothing is listening on {endpoint}; refusing to point the system proxy at it"
            )));
        }

        let previous = match self.read_snapshot() {
            Ok(existing) => existing.previous,
            Err(_) => {
                let captured = self.backend.capture()?;
                // Persist before touching the system, so a crash here is safe.
                let pending = SnapshotFile {
                    backend: self.backend.name().to_string(),
                    endpoint: endpoint.to_string(),
                    applied_at_ms: now_ms(),
                    previous: captured.clone(),
                };
                self.write_snapshot(&pending)?;
                captured
            }
        };

        self.backend.apply(endpoint, DEFAULT_BYPASS)?;

        let snapshot = SnapshotFile {
            backend: self.backend.name().to_string(),
            endpoint: endpoint.to_string(),
            applied_at_ms: now_ms(),
            previous,
        };
        self.write_snapshot(&snapshot)?;

        Ok(AppliedProxy {
            endpoint: snapshot.endpoint,
            backend: snapshot.backend,
            applied_at_ms: snapshot.applied_at_ms,
        })
    }

    /// Restore the captured settings and forget the snapshot.
    ///
    /// Returns `true` when something was actually restored, `false` when there
    /// was nothing to do (idempotent, so callers can call it unconditionally).
    pub fn disable(&self) -> Result<bool> {
        let snapshot = match self.read_snapshot() {
            Ok(snapshot) => snapshot,
            Err(_) => return Ok(false),
        };

        self.backend.restore(&snapshot.previous)?;
        self.remove_snapshot()?;
        Ok(true)
    }

    /// Startup hook: undo a proxy left behind by a crash.
    pub fn recover(&self) -> Result<Option<AppliedProxy>> {
        let snapshot = match self.read_snapshot() {
            Ok(snapshot) => snapshot,
            Err(_) => return Ok(None),
        };

        tracing::warn!(
            endpoint = %snapshot.endpoint,
            "restoring system proxy left behind by a previous session"
        );
        self.backend.restore(&snapshot.previous)?;
        self.remove_snapshot()?;

        Ok(Some(AppliedProxy {
            endpoint: snapshot.endpoint,
            backend: snapshot.backend,
            applied_at_ms: snapshot.applied_at_ms,
        }))
    }

    fn read_snapshot(&self) -> Result<SnapshotFile> {
        let text = std::fs::read_to_string(&self.snapshot_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::Other("no system proxy snapshot".to_string())
            } else {
                Error::io(&self.snapshot_path, error)
            }
        })?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    fn write_snapshot(&self, snapshot: &SnapshotFile) -> Result<()> {
        if let Some(parent) = self.snapshot_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let text = serde_json::to_string_pretty(snapshot)?;
        let staging = self.snapshot_path.with_extension("tmp");
        std::fs::write(&staging, text).map_err(|e| Error::io(&staging, e))?;
        std::fs::rename(&staging, &self.snapshot_path)
            .map_err(|e| Error::io(&self.snapshot_path, e))?;
        Ok(())
    }

    fn remove_snapshot(&self) -> Result<()> {
        match std::fs::remove_file(&self.snapshot_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::io(&self.snapshot_path, error)),
        }
    }
}

#[cfg(windows)]
fn platform_backend() -> Box<dyn ProxyBackend> {
    Box::new(windows::WindowsProxyBackend)
}

#[cfg(target_os = "macos")]
fn platform_backend() -> Box<dyn ProxyBackend> {
    Box::new(macos::MacProxyBackend)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_backend() -> Box<dyn ProxyBackend> {
    Box::new(linux::LinuxProxyBackend)
}

#[cfg(not(any(windows, unix)))]
fn platform_backend() -> Box<dyn ProxyBackend> {
    Box::new(UnsupportedBackend)
}

#[cfg(not(any(windows, unix)))]
struct UnsupportedBackend;

#[cfg(not(any(windows, unix)))]
impl ProxyBackend for UnsupportedBackend {
    fn name(&self) -> &'static str {
        "unsupported"
    }
    fn supported(&self) -> bool {
        false
    }
    fn capture(&self) -> Result<Value> {
        Err(Error::Other("unsupported platform".to_string()))
    }
    fn apply(&self, _endpoint: &str, _bypass: &str) -> Result<()> {
        Err(Error::Other("unsupported platform".to_string()))
    }
    fn restore(&self, _snapshot: &Value) -> Result<()> {
        Err(Error::Other("unsupported platform".to_string()))
    }
}

// ---------------------------------------------------------------------------
// Windows: HKCU Internet Settings + WinINET refresh
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod windows {
    use serde_json::{json, Value};
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    use super::{Error, ProxyBackend, Result};

    const SETTINGS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    const INTERNET_OPTION_REFRESH: u32 = 37;
    const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;

    #[link(name = "wininet")]
    extern "system" {
        fn InternetSetOptionW(
            handle: *mut core::ffi::c_void,
            option: u32,
            buffer: *mut core::ffi::c_void,
            length: u32,
        ) -> i32;
    }

    /// Tell WinINET that the settings changed.
    ///
    /// Without this, already-running applications keep using the old proxy until
    /// they are restarted, which reads as "the toggle did nothing".
    fn refresh_wininet() {
        unsafe {
            InternetSetOptionW(
                std::ptr::null_mut(),
                INTERNET_OPTION_SETTINGS_CHANGED,
                std::ptr::null_mut(),
                0,
            );
            InternetSetOptionW(
                std::ptr::null_mut(),
                INTERNET_OPTION_REFRESH,
                std::ptr::null_mut(),
                0,
            );
        }
    }

    pub struct WindowsProxyBackend;

    impl ProxyBackend for WindowsProxyBackend {
        fn name(&self) -> &'static str {
            "windows-registry"
        }

        fn supported(&self) -> bool {
            true
        }

        fn capture(&self) -> Result<Value> {
            let key = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags(SETTINGS_KEY, KEY_READ)
                .map_err(|error| Error::Other(format!("cannot read Internet Settings: {error}")))?;

            // Missing values are normal on a fresh profile; treat them as off.
            let enable: u32 = key.get_value("ProxyEnable").unwrap_or(0);
            let server: String = key.get_value("ProxyServer").unwrap_or_default();
            let bypass: String = key.get_value("ProxyOverride").unwrap_or_default();

            Ok(json!({
                "ProxyEnable": enable,
                "ProxyServer": server,
                "ProxyOverride": bypass,
            }))
        }

        fn apply(&self, endpoint: &str, bypass: &str) -> Result<()> {
            let key = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags(SETTINGS_KEY, KEY_WRITE)
                .map_err(|error| {
                    Error::Other(format!("cannot write Internet Settings: {error}"))
                })?;

            key.set_value("ProxyServer", &endpoint)
                .map_err(|error| Error::Other(format!("cannot set ProxyServer: {error}")))?;
            key.set_value("ProxyOverride", &bypass)
                .map_err(|error| Error::Other(format!("cannot set ProxyOverride: {error}")))?;
            key.set_value("ProxyEnable", &1u32)
                .map_err(|error| Error::Other(format!("cannot enable proxy: {error}")))?;

            refresh_wininet();
            Ok(())
        }

        fn restore(&self, snapshot: &Value) -> Result<()> {
            let key = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags(SETTINGS_KEY, KEY_WRITE)
                .map_err(|error| {
                    Error::Other(format!("cannot write Internet Settings: {error}"))
                })?;

            let enable = snapshot
                .get("ProxyEnable")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            let server = snapshot
                .get("ProxyServer")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let bypass = snapshot
                .get("ProxyOverride")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            key.set_value("ProxyServer", &server)
                .map_err(|error| Error::Other(format!("cannot restore ProxyServer: {error}")))?;
            key.set_value("ProxyOverride", &bypass)
                .map_err(|error| Error::Other(format!("cannot restore ProxyOverride: {error}")))?;
            key.set_value("ProxyEnable", &enable)
                .map_err(|error| Error::Other(format!("cannot restore ProxyEnable: {error}")))?;

            refresh_wininet();
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// macOS: networksetup per network service
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod macos {
    use serde_json::{json, Value};

    use super::{Error, ProxyBackend, Result};

    pub struct MacProxyBackend;

    fn run(args: &[&str]) -> Result<String> {
        let output = std::process::Command::new("/usr/sbin/networksetup")
            .args(args)
            .output()
            .map_err(|error| Error::Other(format!("networksetup failed: {error}")))?;
        if !output.status.success() {
            return Err(Error::Other(format!(
                "networksetup {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn services() -> Result<Vec<String>> {
        let text = run(&["-listallnetworkservices"])?;
        Ok(text
            .lines()
            .skip(1) // first line is an explanatory banner
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('*'))
            .map(str::to_string)
            .collect())
    }

    /// Parse `networksetup -getwebproxy` output into (enabled, server, port).
    fn get_proxy(flag: &str, service: &str) -> Result<(bool, String, String)> {
        let text = run(&[flag, service])?;
        let mut enabled = false;
        let mut server = String::new();
        let mut port = String::new();
        for line in text.lines() {
            if let Some((key, value)) = line.split_once(':') {
                match key.trim() {
                    "Enabled" => enabled = value.trim().eq_ignore_ascii_case("yes"),
                    "Server" => server = value.trim().to_string(),
                    "Port" => port = value.trim().to_string(),
                    _ => {}
                }
            }
        }
        Ok((enabled, server, port))
    }

    impl ProxyBackend for MacProxyBackend {
        fn name(&self) -> &'static str {
            "macos-networksetup"
        }

        fn supported(&self) -> bool {
            std::path::Path::new("/usr/sbin/networksetup").exists()
        }

        fn capture(&self) -> Result<Value> {
            let mut entries = Vec::new();
            for service in services()? {
                let (web_enabled, web_server, web_port) = get_proxy("-getwebproxy", &service)?;
                let (tls_enabled, tls_server, tls_port) =
                    get_proxy("-getsecurewebproxy", &service)?;
                entries.push(json!({
                    "service": service,
                    "web": { "enabled": web_enabled, "server": web_server, "port": web_port },
                    "secure": { "enabled": tls_enabled, "server": tls_server, "port": tls_port },
                }));
            }
            Ok(Value::Array(entries))
        }

        fn apply(&self, endpoint: &str, _bypass: &str) -> Result<()> {
            let (host, port) = endpoint
                .rsplit_once(':')
                .ok_or_else(|| Error::Other(format!("invalid endpoint `{endpoint}`")))?;

            for service in services()? {
                run(&["-setwebproxy", &service, host, port])?;
                run(&["-setsecurewebproxy", &service, host, port])?;
                run(&["-setwebproxystate", &service, "on"])?;
                run(&["-setsecurewebproxystate", &service, "on"])?;
            }
            Ok(())
        }

        fn restore(&self, snapshot: &Value) -> Result<()> {
            let entries = snapshot
                .as_array()
                .ok_or_else(|| Error::Other("malformed macOS proxy snapshot".to_string()))?;

            for entry in entries {
                let service = entry
                    .get("service")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::Other("snapshot entry has no service".to_string()))?;

                for (key, set_flag, state_flag) in [
                    ("web", "-setwebproxy", "-setwebproxystate"),
                    ("secure", "-setsecurewebproxy", "-setsecurewebproxystate"),
                ] {
                    let section = &entry[key];
                    let enabled = section
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if enabled {
                        let server = section.get("server").and_then(Value::as_str).unwrap_or("");
                        let port = section.get("port").and_then(Value::as_str).unwrap_or("");
                        if !server.is_empty() && !port.is_empty() {
                            run(&[set_flag, service, server, port])?;
                        }
                        run(&[state_flag, service, "on"])?;
                    } else {
                        run(&[state_flag, service, "off"])?;
                    }
                }
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Linux: GNOME settings
// ---------------------------------------------------------------------------

#[cfg(all(unix, not(target_os = "macos")))]
mod linux {
    use serde_json::{json, Value};

    use super::{Error, ProxyBackend, Result};

    pub struct LinuxProxyBackend;

    fn gsettings(args: &[&str]) -> Result<String> {
        let output = std::process::Command::new("gsettings")
            .args(args)
            .output()
            .map_err(|error| Error::Other(format!("gsettings failed: {error}")))?;
        if !output.status.success() {
            return Err(Error::Other(format!(
                "gsettings {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn get(schema_key: &str) -> Result<String> {
        let (schema, key) = schema_key
            .split_once(' ')
            .ok_or_else(|| Error::Other("malformed gsettings key".to_string()))?;
        gsettings(&["get", schema, key])
    }

    fn set(schema_key: &str, value: &str) -> Result<()> {
        let (schema, key) = schema_key
            .split_once(' ')
            .ok_or_else(|| Error::Other("malformed gsettings key".to_string()))?;
        gsettings(&["set", schema, key, value]).map(|_| ())
    }

    const KEYS: [&str; 5] = [
        "org.gnome.system.proxy mode",
        "org.gnome.system.proxy.http host",
        "org.gnome.system.proxy.http port",
        "org.gnome.system.proxy.https host",
        "org.gnome.system.proxy.https port",
    ];

    impl ProxyBackend for LinuxProxyBackend {
        fn name(&self) -> &'static str {
            "linux-gsettings"
        }

        fn supported(&self) -> bool {
            std::process::Command::new("gsettings")
                .arg("get")
                .arg("org.gnome.system.proxy")
                .arg("mode")
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        }

        fn capture(&self) -> Result<Value> {
            let mut values = serde_json::Map::new();
            for key in KEYS {
                values.insert(key.to_string(), json!(get(key)?));
            }
            Ok(Value::Object(values))
        }

        fn apply(&self, endpoint: &str, _bypass: &str) -> Result<()> {
            let (host, port) = endpoint
                .rsplit_once(':')
                .ok_or_else(|| Error::Other(format!("invalid endpoint `{endpoint}`")))?;

            set("org.gnome.system.proxy mode", "'manual'")?;
            for scheme in ["http", "https"] {
                set(
                    &format!("org.gnome.system.proxy.{scheme} host"),
                    &format!("'{host}'"),
                )?;
                set(&format!("org.gnome.system.proxy.{scheme} port"), port)?;
            }
            Ok(())
        }

        fn restore(&self, snapshot: &Value) -> Result<()> {
            let values = snapshot
                .as_object()
                .ok_or_else(|| Error::Other("malformed Linux proxy snapshot".to_string()))?;
            for (key, value) in values {
                if let Some(value) = value.as_str() {
                    set(key, value)?;
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeBackend {
        state: std::sync::Mutex<Value>,
        supported: bool,
        fail_apply: bool,
    }

    impl FakeBackend {
        fn new(state: Value) -> Self {
            Self {
                state: std::sync::Mutex::new(state),
                supported: true,
                fail_apply: false,
            }
        }
    }

    impl ProxyBackend for FakeBackend {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn supported(&self) -> bool {
            self.supported
        }
        fn capture(&self) -> Result<Value> {
            Ok(self.state.lock().unwrap().clone())
        }
        fn apply(&self, endpoint: &str, _bypass: &str) -> Result<()> {
            if self.fail_apply {
                return Err(Error::Other("apply failed".to_string()));
            }
            *self.state.lock().unwrap() = serde_json::json!({ "server": endpoint, "enabled": 1 });
            Ok(())
        }
        fn restore(&self, snapshot: &Value) -> Result<()> {
            *self.state.lock().unwrap() = snapshot.clone();
            Ok(())
        }
    }

    fn manager(name: &str, backend: FakeBackend) -> (SystemProxyManager, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "triproxy-proxy-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let manager = SystemProxyManager {
            snapshot_path: root.join("system-proxy.json"),
            backend: Box::new(backend),
        };
        (manager, root)
    }

    /// Enables against a real loopback listener so the liveness check passes.
    ///
    /// A plain `std::net` listener is enough: the kernel completes the handshake
    /// up to the backlog, so no accept loop is needed.
    fn with_listener<T>(f: impl FnOnce(u16) -> T) -> T {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let result = f(port);
        drop(listener);
        result
    }

    #[test]
    fn enable_refuses_a_port_nothing_listens_on() {
        let (manager, _root) = manager("deadport", FakeBackend::new(serde_json::json!({})));
        let error = manager.enable("127.0.0.1:1").unwrap_err();
        assert!(error.to_string().contains("nothing is listening"));
        // Nothing was applied and no snapshot was written.
        assert!(manager.status().is_none());
    }

    #[test]
    fn unsupported_backend_is_refused() {
        let mut backend = FakeBackend::new(serde_json::json!({}));
        backend.supported = false;
        let (manager, _root) = manager("unsupported", backend);
        let error = manager.enable("127.0.0.1:7890").unwrap_err();
        assert!(error.to_string().contains("not supported"));
    }

    #[test]
    fn disable_without_a_snapshot_is_a_noop() {
        let (manager, _root) = manager("noidle", FakeBackend::new(serde_json::json!({})));
        assert!(!manager.disable().unwrap());
    }

    #[test]
    fn snapshot_survives_and_restores_the_original_values() {
        let original = serde_json::json!({ "server": "127.0.0.1:10808", "enabled": 1 });
        let (manager, root) = manager("roundtrip", FakeBackend::new(original.clone()));

        with_listener(|port| {
            let endpoint = format!("127.0.0.1:{port}");
            manager.enable(&endpoint).unwrap();

            let status = manager.status().expect("proxy should be recorded");
            assert_eq!(status.endpoint, endpoint);
            assert_eq!(status.backend, "fake");

            // Snapshot on disk holds the pre-TriProxy values.
            let text = std::fs::read_to_string(root.join("system-proxy.json")).unwrap();
            let saved: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(saved["previous"], original);
        });

        assert!(manager.disable().unwrap());
        assert!(manager.status().is_none());
        assert!(!root.join("system-proxy.json").exists());
    }

    #[test]
    fn enabling_twice_keeps_the_original_snapshot() {
        let original = serde_json::json!({ "server": "127.0.0.1:10808", "enabled": 1 });
        let (manager, root) = manager("twice", FakeBackend::new(original.clone()));

        with_listener(|port| {
            let first = format!("127.0.0.1:{port}");
            manager.enable(&first).unwrap();
            // Re-applying must not capture our own value as "previous".
            manager.enable(&first).unwrap();
        });

        let text = std::fs::read_to_string(root.join("system-proxy.json")).unwrap();
        let saved: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(saved["previous"], original);
    }

    #[test]
    fn recover_restores_a_leftover_snapshot() {
        let original = serde_json::json!({ "server": "127.0.0.1:10808", "enabled": 1 });
        let (manager, root) = manager("recover", FakeBackend::new(original.clone()));

        with_listener(|port| {
            manager.enable(&format!("127.0.0.1:{port}")).unwrap();
        });
        assert!(root.join("system-proxy.json").exists());

        let recovered = manager.recover().unwrap();
        assert!(recovered.is_some());
        assert!(!root.join("system-proxy.json").exists());
        // A second recovery has nothing left to do.
        assert!(manager.recover().unwrap().is_none());
    }

    #[test]
    fn invalid_endpoint_is_rejected() {
        let (manager, _root) = manager("badendpoint", FakeBackend::new(serde_json::json!({})));
        assert!(manager.enable("127.0.0.1").is_err());
    }
}
