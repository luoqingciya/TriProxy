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

use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use triproxy_core::health::port_is_free;
use triproxy_core::platform;
use triproxy_core::process::run_capture;
use triproxy_core::{
    select_node, AppSettings, ApplyOutcome, CoreKind, CoreManager, ProfileStore,
    SystemProxyManager, DEFAULT_PROBE_URL,
};

use super::{controller, parse_core, running_controller};
use crate::fmt::{first_line, format_bytes, format_ts, probe_writable, render_table};
use crate::NodeCommand;

pub async fn cmd_connections(
    manager: &CoreManager,
    close: Option<&str>,
    close_all: bool,
    json: bool,
) -> Result<i32> {
    let client = controller(manager)?;

    if close_all {
        client.close_all().await?;
        println!("all connections closed");
        return Ok(0);
    }
    if let Some(id) = close {
        client.close_connection(id).await?;
        println!("closed {id}");
        return Ok(0);
    }

    let snapshot = client.connections().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(0);
    }

    if snapshot.connections.is_empty() {
        println!("no live connections");
        return Ok(0);
    }

    let headers = ["ID", "TARGET", "NET", "CHAIN", "UP", "DOWN"];
    let rows: Vec<Vec<String>> = snapshot
        .connections
        .iter()
        .map(|connection| {
            vec![
                connection.id.chars().take(8).collect::<String>(),
                connection.target(),
                connection.metadata.network.clone(),
                connection.chains.join(" ← "),
                format_bytes(connection.upload),
                format_bytes(connection.download),
            ]
        })
        .collect();
    print!("{}", render_table(&headers, &rows));
    println!(
        "\n{} live · total up {} / down {}",
        snapshot.connections.len(),
        format_bytes(snapshot.upload_total),
        format_bytes(snapshot.download_total)
    );
    Ok(0)
}

pub async fn cmd_traffic(manager: &CoreManager, json: bool) -> Result<i32> {
    let client = controller(manager)?;
    let sample = client.traffic_once().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&sample)?);
        return Ok(0);
    }

    println!(
        "↑ {} /s    ↓ {} /s",
        format_bytes(sample.up),
        format_bytes(sample.down)
    );
    Ok(0)
}

pub async fn cmd_delay(manager: &CoreManager, timeout_secs: u64, json: bool) -> Result<i32> {
    if manager.status().await.active_core == Some(CoreKind::Xray) {
        let reason =
            "Xray 没有控制器 API，无法测量真实延迟；`triproxy profile test` 测的是 TCP 握手";
        if json {
            let result = triproxy_core::ActiveDelay {
                node: None,
                latency_ms: None,
                error: Some(reason.to_string()),
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("—  {reason}");
        }
        return Ok(1);
    }

    let client = controller(manager)?;
    let result = triproxy_core::active_delay(
        &client,
        triproxy_core::PROXY_GROUP,
        DEFAULT_PROBE_URL,
        timeout_secs.max(1) * 1000,
    )
    .await;

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        match (&result.node, result.latency_ms) {
            (Some(node), Some(ms)) => println!("{node}  {ms} ms"),
            (Some(node), None) => println!(
                "{node}  —  {}",
                result.error.as_deref().unwrap_or("measurement failed")
            ),
            (None, _) => println!("—  {}", result.error.as_deref().unwrap_or("no node")),
        }
    }

    Ok(if result.latency_ms.is_some() { 0 } else { 1 })
}

pub async fn cmd_node(manager: &CoreManager, command: NodeCommand) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());

    let Some((stored, profile)) = store.active()? else {
        bail!("no active profile; choose one with profile use first");
    };

    match command {
        NodeCommand::Current => {
            match stored.selected_node.as_deref() {
                Some(id) => {
                    let name = profile
                        .nodes
                        .iter()
                        .find(|node| node.id == id)
                        .map(|node| node.name.clone())
                        .unwrap_or_else(|| "(missing)".to_string());
                    println!("{name}");
                }
                None => println!("(none — the profile order decides)"),
            }
            Ok(0)
        }
        NodeCommand::Clear => {
            let client = running_controller(manager).await;
            let outcome = select_node(&store, client.as_ref(), None).await?;
            match outcome {
                ApplyOutcome::Applied { .. } => println!("selection cleared and applied"),
                ApplyOutcome::SavedOnly { reason, .. } => {
                    println!("selection cleared; takes effect on next start ({reason})")
                }
            }
            Ok(0)
        }
        NodeCommand::Use { target } => {
            let node = profile
                .nodes
                .iter()
                .find(|node| node.name.eq_ignore_ascii_case(&target))
                .or_else(|| {
                    profile
                        .nodes
                        .iter()
                        .find(|node| node.id.starts_with(&target))
                })
                .ok_or_else(|| {
                    anyhow!(
                        "no node matching {target} in the active profile; run profile show to list them"
                    )
                })?;

            let client = running_controller(manager).await;
            let outcome = select_node(&store, client.as_ref(), Some(&node.id)).await?;
            match outcome {
                ApplyOutcome::Applied { node } => {
                    println!("using {node} (applied to the running core)")
                }
                ApplyOutcome::SavedOnly { node, reason } => {
                    let name = node.unwrap_or_default();
                    println!("using {name}");
                    println!("  saved; takes effect on next start ({reason})");
                }
            }
            Ok(0)
        }
    }
}

struct LogEntry {
    ts_ms: u64,
    core: CoreKind,
    level: char,
    text: String,
}

fn read_log_file(path: &Path, core: CoreKind) -> Vec<LogEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| parse_log_line(line, core))
        .collect()
}

fn parse_log_line(line: &str, core: CoreKind) -> Option<LogEntry> {
    let mut parts = line.splitn(3, '\t');
    let ts_ms = parts.next()?.parse::<u64>().ok()?;
    let level = parts.next()?.chars().next().unwrap_or('I');
    let text = parts.next().unwrap_or("").to_string();
    Some(LogEntry {
        ts_ms,
        core,
        level,
        text,
    })
}

fn print_entries(entries: &[LogEntry], show_core: bool) {
    for entry in entries {
        let text = entry.text.trim_end();
        if show_core {
            println!(
                "{}  {:<8} {} {}",
                format_ts(entry.ts_ms),
                entry.core.as_str(),
                entry.level,
                text
            );
        } else {
            println!("{}  {} {}", format_ts(entry.ts_ms), entry.level, text);
        }
    }
}

pub async fn cmd_logs(
    manager: &CoreManager,
    lines: usize,
    core: Option<&str>,
    follow: bool,
) -> Result<i32> {
    let kinds: Vec<CoreKind> = match core {
        Some(name) => vec![parse_core(name)?],
        None => CoreKind::ALL.to_vec(),
    };
    let show_core = kinds.len() > 1;

    let mut entries: Vec<LogEntry> = kinds
        .iter()
        .flat_map(|kind| read_log_file(&manager.layout().core_log_file(kind.as_str()), *kind))
        .collect();

    if entries.is_empty() {
        eprintln!("no logs captured yet — start a core first");
        if !follow {
            return Ok(0);
        }
    }

    entries.sort_by_key(|entry| entry.ts_ms);
    let skip = entries.len().saturating_sub(lines);
    print_entries(&entries[skip..], show_core);

    if !follow {
        return Ok(0);
    }

    let mut offsets: Vec<(CoreKind, PathBuf, u64)> = kinds
        .iter()
        .map(|kind| {
            let path = manager.layout().core_log_file(kind.as_str());
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            (*kind, path, len)
        })
        .collect();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = tokio::time::sleep(Duration::from_millis(300)) => {
                for (kind, path, offset) in offsets.iter_mut() {
                    let Ok(meta) = std::fs::metadata(&*path) else { continue };
                    if meta.len() < *offset {
                        *offset = 0;
                    }
                    if meta.len() == *offset {
                        continue;
                    }
                    if let Ok(mut file) = std::fs::File::open(&*path) {
                        if file.seek(SeekFrom::Start(*offset)).is_ok() {
                            let mut chunk = String::new();
                            if file.read_to_string(&mut chunk).is_ok() {
                                *offset = meta.len();
                                for line in chunk.lines() {
                                    if let Some(entry) = parse_log_line(line, *kind) {
                                        print_entries(&[entry], show_core);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(0)
}

pub async fn cmd_doctor(manager: &CoreManager) -> Result<i32> {
    println!("TriProxy doctor");
    println!();

    let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    println!("  platform   {target}");

    if let Some(distro) = platform::distro() {
        let family = distro.family();
        println!(
            "  distro     {} ({}, {})",
            distro.label(),
            distro.id,
            family.as_str()
        );
    }

    if let Some(desktop) = platform::desktop() {
        match platform::tray_host_available() {
            Some(true) => println!("  desktop    {} — 托盘可用", desktop.as_str()),
            Some(false) => {
                println!(
                    "  desktop    {} — 托盘不可用，关闭窗口将退出",
                    desktop.as_str()
                );
                println!("             {}", platform::appindicator_install_hint());
            }
            None => println!("  desktop    {}", desktop.as_str()),
        }
    }

    if let Some(selinux) = platform::selinux() {
        println!("  selinux    {}", selinux.as_str());
    }

    if platform::tun_device_present() == Some(false) {
        println!(
            "  tun device {} 不存在 — 先执行 sudo modprobe tun",
            platform::TUN_DEVICE
        );
    }

    let proxy = SystemProxyManager::new(manager.layout());
    let proxy_state = if proxy.supported() {
        "可用"
    } else {
        "不可用（当前桌面没有可驱动的代理设置）"
    };
    println!("  proxy      {} — {proxy_state}", proxy.backend_name());

    let data_dir = manager.layout().root();
    let writable = probe_writable(data_dir);
    println!(
        "  data dir   {} ({})",
        data_dir.display(),
        if writable { "writable" } else { "NOT writable" }
    );

    println!();
    for kind in CoreKind::ALL {
        match manager.binaries().resolve(kind) {
            Some((path, source)) => {
                let args: Vec<OsString> = kind
                    .version_probe_args()
                    .iter()
                    .map(OsString::from)
                    .collect();
                let probe = run_capture(&path, &args).await;
                let detail = match probe {
                    Ok(output) if output.success => {
                        format!("ok — {}", first_line(&output.combined()))
                    }
                    Ok(output) => format!("FAILED to report version (exit {:?})", output.code),
                    Err(e) => format!("FAILED to execute: {e}"),
                };
                println!(
                    "  {:<9} {:<8} {} — {detail}",
                    kind.as_str(),
                    source.as_str(),
                    path.display()
                );
            }
            None => println!(
                "  {:<9} {:<8} not installed (triproxy core install {kind})",
                kind.as_str(),
                "-"
            ),
        }
    }

    let settings = AppSettings::load(manager.layout());
    let xray_http = CoreKind::Xray.http_proxy_port(settings.listen_port);
    println!();
    for port in [settings.listen_port, xray_http, settings.api_port] {
        println!(
            "  port {port:<5} {}",
            if port_is_free(port) { "free" } else { "IN USE" }
        );
    }

    Ok(0)
}
