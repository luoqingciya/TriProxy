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

//! TUN mode support.
//!
//! TriProxy does not create the TUN device itself — the cores do (wintun on
//! Windows, utun on macOS, /dev/net/tun on Linux). What this module owns is the
//! part that goes wrong after a crash:
//!
//! - **Elevation.** Creating a TUN interface needs administrator/root. Checking
//!   up front turns a cryptic core failure into a sentence the user can act on.
//! - **Leftovers.** If the core dies hard, the adapter and its routes can outlive
//!   it. [`leftovers`] finds adapters that look like ours so the UI can say
//!   something instead of silently routing traffic into a dead interface.
//!
//! Detection is by adapter *name*, which is why the compilers pin
//! [`INTERFACE_NAME`] into the generated config. Nothing here deletes devices:
//! removing a network adapter is an administrative act with system-wide effects,
//! and doing it automatically on a false positive would be worse than reporting.

use serde::Serialize;

use crate::error::Result;

/// Adapter name pinned into every generated TUN config.
pub const INTERFACE_NAME: &str = "TriProxy";

/// Adapter names that indicate a leftover from a supported core.
///
/// Not used on macOS: utun interfaces are numbered dynamically and carry no name
/// we could match, so  always returns empty there.
#[cfg(not(target_os = "macos"))]
const KNOWN_PREFIXES: [&str; 3] = ["triproxy", "mihomo", "sing-box"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TunAdapter {
    pub name: String,
    /// Platform identifier (registry GUID on Windows, interface name elsewhere).
    pub id: String,
}

impl TunAdapter {
    /// Command that removes the adapter, for the message shown to the user.
    pub fn cleanup_hint(&self) -> String {
        #[cfg(windows)]
        {
            format!(
                "以管理员身份运行：pnputil /remove-device \"{}\"（或在设备管理器中禁用该网卡）",
                self.name
            )
        }
        #[cfg(not(windows))]
        {
            format!("sudo ip link delete {}", self.name)
        }
    }
}

/// Whether the current process can create a TUN interface.
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        extern "system" {
            fn IsUserAnAdmin() -> i32;
        }
        // SAFETY: no arguments, no pointers, documented as a simple query.
        unsafe { IsUserAnAdmin() != 0 }
    }

    #[cfg(unix)]
    {
        extern "C" {
            fn geteuid() -> u32;
        }
        // SAFETY: no arguments, returns the effective uid.
        unsafe { geteuid() == 0 }
    }

    #[cfg(not(any(windows, unix)))]
    {
        false
    }
}

/// Adapters that look like a TUN interface left behind by a core.
pub fn leftovers() -> Result<Vec<TunAdapter>> {
    #[cfg(windows)]
    {
        windows::enumerate()
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux::enumerate()
    }

    #[cfg(target_os = "macos")]
    {
        // utun interfaces are numbered dynamically and disappear with the
        // process that created them; there is no name to match against.
        Ok(Vec::new())
    }

    #[cfg(not(any(windows, unix)))]
    {
        Ok(Vec::new())
    }
}

#[cfg(not(target_os = "macos"))]
fn looks_like_ours(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    KNOWN_PREFIXES
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
}

#[cfg(windows)]
mod windows {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    use super::{looks_like_ours, TunAdapter};
    use crate::error::Result;

    /// `HKLM\SYSTEM\CurrentControlSet\Control\Network\{4d36e972-...}` maps each
    /// network class instance GUID to a `Connection\Name` value.
    ///
    /// The registry is used rather than `netsh interface show interface` because
    /// that output is localised and the interface column can contain spaces.
    const NETWORK_CLASS: &str =
        r"SYSTEM\CurrentControlSet\Control\Network\{4d36e972-e325-11ce-bfc1-08002be10318}";

    pub fn enumerate() -> Result<Vec<TunAdapter>> {
        let root = match RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(NETWORK_CLASS, KEY_READ)
        {
            Ok(key) => key,
            // A machine without any network class entries has nothing to report.
            Err(_) => return Ok(Vec::new()),
        };

        let mut found = Vec::new();
        for guid in root.enum_keys().flatten() {
            // The class root also contains non-GUID subkeys such as `Descriptions`.
            if !guid.starts_with('{') {
                continue;
            }
            let Ok(instance) = root.open_subkey_with_flags(format!("{guid}\\Connection"), KEY_READ)
            else {
                continue;
            };
            let Ok(name) = instance.get_value::<String, _>("Name") else {
                continue;
            };
            if looks_like_ours(&name) {
                found.push(TunAdapter { name, id: guid });
            }
        }

        found.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(found)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod linux {
    use super::{looks_like_ours, TunAdapter};
    use crate::error::Result;

    pub fn enumerate() -> Result<Vec<TunAdapter>> {
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
            return Ok(found);
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if looks_like_ours(&name) {
                found.push(TunAdapter {
                    id: name.clone(),
                    name,
                });
            }
        }
        found.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn recognises_known_adapter_names() {
        assert!(looks_like_ours("TriProxy"));
        assert!(looks_like_ours("triproxy"));
        assert!(looks_like_ours("Mihomo"));
        assert!(looks_like_ours("sing-box"));
        assert!(!looks_like_ours("以太网 4"));
        assert!(!looks_like_ours("WLAN"));
    }

    #[test]
    fn cleanup_hint_names_the_adapter() {
        let adapter = TunAdapter {
            name: INTERFACE_NAME.to_string(),
            id: "x".to_string(),
        };
        assert!(adapter.cleanup_hint().contains(INTERFACE_NAME));
    }

    #[test]
    fn enumeration_does_not_fail_on_this_machine() {
        // Whether anything is found depends on the machine; the call itself must
        // always succeed so startup is never blocked by it.
        let found = leftovers().expect("enumeration must not fail");
        #[cfg(not(target_os = "macos"))]
        for adapter in found {
            assert!(looks_like_ours(&adapter.name));
        }
        #[cfg(target_os = "macos")]
        assert!(found.is_empty());
    }

    #[test]
    fn elevation_check_returns_a_bool() {
        // The value depends on how the test runner was started; this asserts the
        // FFI call does not crash or return garbage.
        let _elevated = is_elevated();
    }
}
