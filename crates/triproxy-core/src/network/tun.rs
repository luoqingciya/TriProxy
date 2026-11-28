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

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};

/// Adapter name pinned into every generated TUN config.
pub const INTERFACE_NAME: &str = "triproxy";

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

/// Linux `CAP_NET_ADMIN` capability index (`include/uapi/linux/capability.h`).
#[cfg(any(target_os = "linux", test))]
const CAP_NET_ADMIN_BIT: u32 = 1 << 12;
#[cfg(any(target_os = "linux", test))]
const VFS_CAP_REVISION_2: u32 = 0x0200_0000;
#[cfg(any(target_os = "linux", test))]
const VFS_CAP_REVISION_3: u32 = 0x0300_0000;
#[cfg(any(target_os = "linux", test))]
const VFS_CAP_FLAGS_EFFECTIVE: u32 = 0x0000_0001;

/// Parse Linux `security.capability` (`vfs_cap_data` / `vfs_ns_cap_data`) bytes
/// and check whether `CAP_NET_ADMIN` is present in the permitted + effective sets.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_linux_vfs_cap_data(bytes: &[u8]) -> bool {
    if bytes.len() < 20 {
        return false;
    }
    let magic_etc = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let version = magic_etc & 0xFF00_0000;
    if !matches!(version, VFS_CAP_REVISION_2 | VFS_CAP_REVISION_3) {
        return false;
    }
    let effective = (magic_etc & VFS_CAP_FLAGS_EFFECTIVE) != 0;
    let permitted_lo = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    effective && (permitted_lo & CAP_NET_ADMIN_BIT) != 0
}

/// Whether the core executable itself has persistent privilege to open TUN
/// (Linux `cap_net_admin` xattr or macOS root setuid).
pub fn binary_has_tun_capability(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        extern "C" {
            fn getxattr(
                path: *const std::ffi::c_char,
                name: *const std::ffi::c_char,
                value: *mut u8,
                size: usize,
            ) -> isize;
        }

        let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        let Ok(c_attr) = CString::new("security.capability") else {
            return false;
        };
        let mut buf = [0u8; 64];
        // SAFETY: both pointers are valid NUL-terminated C strings and `buf` is writable for `buf.len()` bytes.
        let len = unsafe {
            getxattr(
                c_path.as_ptr(),
                c_attr.as_ptr(),
                buf.as_mut_ptr(),
                buf.len(),
            )
        };
        if len < 0 {
            return false;
        }
        parse_linux_vfs_cap_data(&buf[..len as usize])
    }

    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path)
            .map(|meta| meta.uid() == 0 && (meta.mode() & 0o4000) != 0)
            .unwrap_or(false)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        false
    }
}

/// Whether TUN can be started using either current process privileges or
/// capabilities attached to `binary`.
pub fn is_tun_authorized_for(binary: Option<&Path>) -> bool {
    is_elevated() || binary.is_some_and(binary_has_tun_capability)
}

/// Why TUN cannot start for the given core binary, in words the user can act on.
///
/// Two things can be missing before a core ever tries: the privilege to create
/// an interface, and — on Linux — the device node the kernel exposes for it.
/// Both otherwise surface as a core-internal failure that names neither, which
/// is the worst possible shape for an error: the user is told something broke
/// and given nothing to do about it.
///
/// Returns `None` when nothing is visibly missing. That is not a promise the
/// start will succeed — only that the check found no reason to refuse early,
/// and the core's own validation stays authoritative.
pub fn unavailable_reason_for(binary: Option<&Path>) -> Option<String> {
    if !is_tun_authorized_for(binary) {
        return Some(if cfg!(windows) {
            "TUN 模式需要管理员权限：请以管理员身份重新启动 TriProxy".to_string()
        } else {
            "TUN 模式需要管理员权限：请在弹窗中输入密码授权，或手动为该核心授予 cap_net_admin"
                .to_string()
        });
    }

    if crate::platform::tun_device_present() == Some(false) {
        return Some(format!(
            "TUN 模式需要 {}，但该设备不存在：先执行 `sudo modprobe tun` 再重试",
            crate::platform::TUN_DEVICE
        ));
    }

    None
}

/// Convenience wrapper when no specific core binary path is available.
pub fn unavailable_reason() -> Option<String> {
    unavailable_reason_for(None)
}

/// Ensure `/dev/net/tun` exists on Linux before starting a core in TUN mode.
/// When running as root, attempts `modprobe tun` if the device node is absent.
pub fn ensure_tun_device() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if crate::platform::tun_device_present() != Some(false) {
            return Ok(());
        }
        if !is_elevated() {
            return Err(Error::Other(format!(
                "缺少 {} 设备节点，请先执行 `sudo modprobe tun`",
                crate::platform::TUN_DEVICE
            )));
        }
        let modprobe = find_linux_tool("modprobe", &["/usr/sbin/modprobe", "/sbin/modprobe"])
            .unwrap_or_else(|| PathBuf::from("modprobe"));
        let _ = std::process::Command::new(modprobe).arg("tun").output();
        if crate::platform::tun_device_present() == Some(false) {
            return Err(Error::Other(format!(
                "已执行 modprobe tun，但 {} 仍不存在",
                crate::platform::TUN_DEVICE
            )));
        }
    }
    Ok(())
}

/// Prompt the user with an OS authentication dialog and grant TUN permissions
/// to the given core binaries (`setcap cap_net_admin,cap_net_bind_service,cap_net_raw=+ep`
/// on Linux, root setuid on macOS).
pub fn authorize_tun_binaries(paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        return Err(Error::Other(
            "未找到可用的代理核心（Mihomo 或 sing-box），请先下载核心".to_string(),
        ));
    }
    for path in paths {
        if !path.is_file() {
            return Err(Error::Other(format!(
                "核心文件不存在，无法授权：{}",
                path.display()
            )));
        }
    }

    #[cfg(target_os = "linux")]
    {
        let pkexec = find_linux_tool("pkexec", &["/usr/bin/pkexec", "/bin/pkexec"]).ok_or_else(
            || {
                Error::Other(format!(
                    "未找到 pkexec（polkit），无法弹出系统密码框；请在终端执行：\nsudo setcap cap_net_admin,cap_net_bind_service,cap_net_raw=+ep {}",
                    paths[0].display()
                ))
            },
        )?;
        let setcap = find_linux_tool(
            "setcap",
            &["/usr/sbin/setcap", "/sbin/setcap", "/usr/bin/setcap"],
        )
        .unwrap_or_else(|| PathBuf::from("setcap"));
        let modprobe = find_linux_tool("modprobe", &["/usr/sbin/modprobe", "/sbin/modprobe"])
            .unwrap_or_else(|| PathBuf::from("modprobe"));

        let mut command = std::process::Command::new(&pkexec);
        command.arg("/bin/sh").arg("-c").arg(
            "if [ ! -e /dev/net/tun ]; then \"$1\" tun || true; fi; setcap_bin=\"$2\"; shift 2; for p in \"$@\"; do \"$setcap_bin\" \"cap_net_admin,cap_net_bind_service,cap_net_raw=+ep\" \"$p\" || exit 1; done",
        );
        command.arg("_").arg(&modprobe).arg(&setcap);
        for path in paths {
            command.arg(path);
        }

        let output = command
            .output()
            .map_err(|error| Error::Other(format!("启动管理员密码授权框失败：{error}")))?;

        if !output.status.success() {
            if matches!(output.status.code(), Some(126) | Some(127)) {
                return Err(Error::Other("已取消管理员密码授权".to_string()));
            }
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let reason = if !detail.is_empty() { detail } else { stdout };
            return Err(Error::Other(if reason.is_empty() {
                format!("TUN 权限授权失败（退出码 {:?}）", output.status.code())
            } else {
                format!("TUN 权限授权失败：{reason}")
            }));
        }

        if crate::platform::tun_device_present() == Some(false) {
            return Err(Error::Other(format!(
                "已授权核心，但内核设备 {} 仍不存在，请检查 tun 内核模块",
                crate::platform::TUN_DEVICE
            )));
        }

        for path in paths {
            if !binary_has_tun_capability(path) {
                return Err(Error::Other(format!(
                    "已执行授权，但未能验证 {} 的 cap_net_admin 权限",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let mut commands = Vec::with_capacity(paths.len());
        for path in paths {
            let quoted = shell_single_quote(&path.to_string_lossy());
            commands.push(format!("chown root:wheel {quoted} && chmod 4755 {quoted}"));
        }
        let script = commands.join(" && ");
        let escaped = script.replace('\\', "\\\\").replace('"', "\\\"");
        let apple_script = format!("do shell script \"{escaped}\" with administrator privileges");

        let output = std::process::Command::new("osascript")
            .args(["-e", &apple_script])
            .output()
            .map_err(|error| Error::Other(format!("启动 macOS 管理员授权框失败：{error}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.contains("-128") || stderr.to_ascii_lowercase().contains("cancel") {
                return Err(Error::Other("已取消管理员密码授权".to_string()));
            }
            return Err(Error::Other(if stderr.is_empty() {
                "macOS 管理员权限授权失败".to_string()
            } else {
                format!("macOS 管理员权限授权失败：{stderr}")
            }));
        }

        for path in paths {
            if !binary_has_tun_capability(path) {
                return Err(Error::Other(format!(
                    "已执行授权，但未能验证 {} 的 root setuid 权限",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    #[cfg(windows)]
    {
        if is_elevated() {
            Ok(())
        } else {
            Err(Error::Other(
                "Windows 启用 TUN 需要管理员权限，请在弹窗确认以管理员身份重启 TriProxy"
                    .to_string(),
            ))
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        Err(Error::Other("当前平台不支持图形化 TUN 提权".to_string()))
    }
}

/// Trigger a native UAC elevation prompt to relaunch the current executable
/// with administrator privileges on Windows.
pub fn relaunch_elevated() -> Result<()> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()
            .map_err(|error| Error::Other(format!("无法确定当前程序路径：{error}")))?;
        let escaped = exe.to_string_lossy().replace('\'', "''");
        let ps = format!("Start-Process -FilePath '{escaped}' -Verb RunAs");
        let output = crate::process::hidden_command("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps])
            .output()
            .map_err(|error| Error::Other(format!("触发 Windows UAC 提权失败：{error}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.is_empty() {
                return Err(Error::Other("已取消管理员授权".to_string()));
            }
            return Err(Error::Other(format!("管理员提权失败：{stderr}")));
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        Err(Error::Other("仅 Windows 支持整应用管理员重启".to_string()))
    }
}

#[cfg(target_os = "linux")]
fn find_linux_tool(name: &str, fallbacks: &[&str]) -> Option<PathBuf> {
    if let Some(found) = crate::paths::find_on_path(name) {
        return Some(found);
    }
    fallbacks
        .iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
}

#[cfg(target_os = "macos")]
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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
    ///
    /// **The registry is not authoritative on its own.** Windows keeps the
    /// `Connection` key after an adapter is removed — that is how it remembers
    /// per-adapter settings when the device comes back. So a name found here may
    /// belong to an adapter that no longer exists, and reporting it as a leftover
    /// would warn every user who has ever used TUN. Each hit is therefore
    /// confirmed against the live interface list first.
    const NETWORK_CLASS: &str =
        r"SYSTEM\CurrentControlSet\Control\Network\{4d36e972-e325-11ce-bfc1-08002be10318}";

    /// Does this interface actually exist right now?
    ///
    /// Only the exit status is inspected, never the output, so this works on a
    /// localised Windows. If `netsh` cannot be run at all the entry is kept:
    /// warning about an adapter that may not exist is annoying, but hiding a real
    /// leftover means traffic silently goes into a dead interface.
    fn interface_exists(name: &str) -> bool {
        crate::process::hidden_command("netsh")
            .args(["interface", "show", "interface", &format!("name={name}")])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(true)
    }

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
            if looks_like_ours(&name) && interface_exists(&name) {
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

    /// The pre-flight check has to say *what* is missing, not just that
    /// something is: "TUN failed" leaves the user with nothing to do.
    #[test]
    fn an_unelevated_process_is_told_about_the_privilege() {
        if is_elevated() {
            // Running as root: there is no privilege to complain about, and the
            // device check below it is machine-dependent.
            return;
        }
        let reason = unavailable_reason().expect("an unelevated process is refused");
        assert!(reason.contains("权限"), "{reason}");
    }

    #[test]
    fn tun_interface_name_is_lowercase_triproxy() {
        assert_eq!(INTERFACE_NAME, "triproxy");
    }

    #[test]
    fn parses_linux_vfs_cap_data_accurately() {
        let mut valid_v2 = [0u8; 20];
        valid_v2[0..4]
            .copy_from_slice(&(VFS_CAP_REVISION_2 | VFS_CAP_FLAGS_EFFECTIVE).to_le_bytes());
        valid_v2[4..8].copy_from_slice(&(CAP_NET_ADMIN_BIT | (1 << 13)).to_le_bytes());
        assert!(parse_linux_vfs_cap_data(&valid_v2));

        let mut valid_v3 = [0u8; 24];
        valid_v3[0..4]
            .copy_from_slice(&(VFS_CAP_REVISION_3 | VFS_CAP_FLAGS_EFFECTIVE).to_le_bytes());
        valid_v3[4..8].copy_from_slice(&CAP_NET_ADMIN_BIT.to_le_bytes());
        assert!(parse_linux_vfs_cap_data(&valid_v3));

        // Missing effective flag
        let mut not_effective = [0u8; 20];
        not_effective[0..4].copy_from_slice(&VFS_CAP_REVISION_2.to_le_bytes());
        not_effective[4..8].copy_from_slice(&CAP_NET_ADMIN_BIT.to_le_bytes());
        assert!(!parse_linux_vfs_cap_data(&not_effective));

        // Wrong capability bit
        let mut wrong_cap = [0u8; 20];
        wrong_cap[0..4]
            .copy_from_slice(&(VFS_CAP_REVISION_2 | VFS_CAP_FLAGS_EFFECTIVE).to_le_bytes());
        wrong_cap[4..8].copy_from_slice(&(1u32 << 10).to_le_bytes());
        assert!(!parse_linux_vfs_cap_data(&wrong_cap));

        // Truncated
        assert!(!parse_linux_vfs_cap_data(&valid_v2[..12]));
    }
}
