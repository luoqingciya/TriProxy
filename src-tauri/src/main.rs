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

// Keep the console window from appearing alongside the GUI on Windows release
// builds; `cargo run` still prints logs in debug.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
fn maybe_elevate_linux() {
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;
    use std::process::Command;

    if triproxy_core::is_elevated()
        || std::env::var_os("TRIPROXY_ELEVATION_TRIED").is_some()
        || std::env::var_os("TRIPROXY_NO_ELEVATE").is_some()
    {
        return;
    }

    let has_display = ["DISPLAY", "WAYLAND_DISPLAY"]
        .into_iter()
        .any(|key| std::env::var_os(key).is_some_and(|val| !val.is_empty()));
    if !has_display {
        return;
    }

    let Some(pkexec) = triproxy_core::paths::find_on_path("pkexec").or_else(|| {
        ["/usr/bin/pkexec", "/bin/pkexec"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
    }) else {
        return;
    };

    let Ok(exe) = std::env::current_exe() else {
        return;
    };

    // Allow root to connect to X11/XWayland when running under Xorg or hybrid Wayland.
    if std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) {
        if let Some(xhost) = triproxy_core::paths::find_on_path("xhost").or_else(|| {
            ["/usr/bin/xhost", "/bin/xhost"]
                .into_iter()
                .map(PathBuf::from)
                .find(|p| p.is_file())
        }) {
            let _ = Command::new(xhost).arg("+SI:localuser:root").output();
        }
    }

    let mut cmd = Command::new(pkexec);
    cmd.arg("env").arg("TRIPROXY_ELEVATION_TRIED=1");

    for (src, dst) in [
        ("HOME", "TRIPROXY_ORIG_HOME"),
        ("USER", "TRIPROXY_ORIG_USER"),
        ("XDG_DATA_HOME", "TRIPROXY_ORIG_XDG_DATA_HOME"),
        ("XDG_RUNTIME_DIR", "TRIPROXY_ORIG_XDG_RUNTIME_DIR"),
        (
            "DBUS_SESSION_BUS_ADDRESS",
            "TRIPROXY_ORIG_DBUS_SESSION_BUS_ADDRESS",
        ),
    ] {
        if let Ok(val) = std::env::var(src) {
            if !val.trim().is_empty() {
                cmd.arg(format!("{dst}={val}"));
            }
        }
    }

    if let Ok(meta) = std::fs::metadata("/proc/self") {
        cmd.arg(format!("TRIPROXY_ORIG_UID={}", meta.uid()));
    }

    for key in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_TYPE",
        "GDK_BACKEND",
        "QT_QPA_PLATFORM",
        "NO_AT_BRIDGE",
        "TRIPROXY_HOME",
    ] {
        if let Ok(val) = std::env::var(key) {
            if !val.trim().is_empty() {
                cmd.arg(format!("{key}={val}"));
            }
        }
    }

    cmd.arg(exe);
    cmd.args(std::env::args_os().skip(1));

    match cmd.status() {
        Ok(status) => {
            // 126: polkit dialog cancelled, 127: polkit auth failed/unavailable -> fall back gracefully.
            if matches!(status.code(), Some(126) | Some(127)) {
                return;
            }
            std::process::exit(status.code().unwrap_or(0));
        }
        Err(_) => {
            // pkexec could not be spawned; continue as normal user.
        }
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    maybe_elevate_linux();

    triproxy_app::run()
}
