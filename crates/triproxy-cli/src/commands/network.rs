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

use anyhow::{anyhow, bail, Result};
use triproxy_core::{is_elevated, leftovers, platform, CoreManager, SystemProxyManager};

use crate::fmt::format_datetime;

pub fn cmd_tun_status(manager: &CoreManager) -> Result<i32> {
    println!("TUN 模式");
    if is_elevated() {
        println!("  权限      管理员");
    } else {
        println!("  权限      普通用户 — TUN 需要以管理员身份运行");
    }

    if let Some(present) = platform::tun_device_present() {
        let device = platform::TUN_DEVICE;
        if present {
            println!("  设备      {device}");
        } else {
            println!("  设备      {device} 不存在 — 执行 sudo modprobe tun 后重试");
        }
    }

    match manager.persisted_listen_port() {
        Some(port) => println!("  当前核心  127.0.0.1:{port}"),
        None => println!("  当前核心  未运行"),
    }

    let found = leftovers()?;
    if found.is_empty() {
        println!("  残留网卡  无");
    } else {
        println!("  残留网卡  {} 个（可能是上次异常退出留下的）", found.len());
        for adapter in &found {
            println!("    {}  [{}]", adapter.name, adapter.id);
            println!("      {}", adapter.cleanup_hint());
        }
    }
    Ok(0)
}

pub async fn cmd_proxy_on(manager: &CoreManager, endpoint: Option<&str>) -> Result<i32> {
    let proxy = SystemProxyManager::new(manager.layout());
    if !proxy.supported() {
        bail!(
            "system proxy is not supported on this platform (backend `{}`)",
            proxy.backend_name()
        );
    }

    let endpoint = match endpoint {
        Some(endpoint) => endpoint.to_string(),
        None => {
            let port = manager.persisted_http_proxy_port().ok_or_else(|| {
                anyhow!("no core is running; pass --endpoint or start a core first")
            })?;
            format!("127.0.0.1:{port}")
        }
    };

    let applied = proxy.enable(&endpoint)?;
    println!("system proxy enabled → {}", applied.endpoint);
    println!("  backend  {}", applied.backend);
    println!("  previous settings saved; `triproxy proxy off` restores them");
    Ok(0)
}

pub fn cmd_proxy_off(manager: &CoreManager) -> Result<i32> {
    let proxy = SystemProxyManager::new(manager.layout());
    if proxy.disable()? {
        println!("system proxy restored");
    } else {
        println!("system proxy was not enabled by TriProxy");
    }
    Ok(0)
}

pub fn cmd_proxy_status(manager: &CoreManager) -> Result<i32> {
    let proxy = SystemProxyManager::new(manager.layout());
    match proxy.status() {
        Some(applied) => {
            println!("system proxy  enabled → {}", applied.endpoint);
            println!("  backend      {}", applied.backend);
            println!("  applied at   {}", format_datetime(applied.applied_at_ms));
        }
        None => println!("system proxy  not managed by TriProxy"),
    }
    Ok(0)
}
