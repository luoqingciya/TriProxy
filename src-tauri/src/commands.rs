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

//! Tauri command surface.
//!
//! This layer is deliberately thin: every command maps to one `triproxy-core`
//! operation. Business rules (core selection, compilation, exclusivity) stay in
//! the core crate so the CLI and the GUI cannot drift apart.

use serde::Serialize;
use tauri::State;

use triproxy_core::compiler::compile;
use triproxy_core::profile::{parse_payload, Node};
use triproxy_core::{
    AppSettings, AppliedProxy, CompiledConfig, CoreKind, CorePreference, LogLevel, LogLine,
    ManagerStatus, ProfileStore, Selection, Selector, StoredProfile, SystemProxyManager,
};

use crate::AppState;

type CmdResult<T> = Result<T, String>;

fn err(message: impl std::fmt::Display) -> String {
    message.to_string()
}

#[tauri::command]
pub async fn status(state: State<'_, AppState>) -> CmdResult<ManagerStatus> {
    Ok(state.manager.status().await)
}

#[derive(Serialize)]
pub struct ProfileLibrary {
    active: Option<String>,
    profiles: Vec<StoredProfile>,
}

#[tauri::command]
pub async fn profiles(state: State<'_, AppState>) -> CmdResult<ProfileLibrary> {
    let store = ProfileStore::new(state.manager.layout());
    Ok(ProfileLibrary {
        active: store.active_id().map_err(err)?,
        profiles: store.list().map_err(err)?,
    })
}

/// Flattened node view for the list. The full node model carries credentials,
/// which the UI never needs and which should not cross the IPC boundary.
#[derive(Serialize)]
pub struct NodeSummary {
    id: String,
    name: String,
    protocol: String,
    server: String,
    port: u16,
    transport: String,
    security: String,
    udp: bool,
}

fn summarise(node: &Node) -> NodeSummary {
    let security = match &node.tls {
        None => "-".to_string(),
        Some(tls) if tls.reality.is_some() => "reality".to_string(),
        Some(_) => "tls".to_string(),
    };
    NodeSummary {
        id: node.id.clone(),
        name: node.name.clone(),
        protocol: node.protocol.as_str().to_string(),
        server: node.server.clone(),
        port: node.port,
        transport: node.transport.label().to_string(),
        security,
        udp: node.udp,
    }
}

#[tauri::command]
pub async fn active_nodes(state: State<'_, AppState>) -> CmdResult<Vec<NodeSummary>> {
    let store = ProfileStore::new(state.manager.layout());
    let Some((_, profile)) = store.active().map_err(err)? else {
        return Ok(Vec::new());
    };
    Ok(profile.nodes.iter().map(summarise).collect())
}

#[tauri::command]
pub async fn import_profile(
    state: State<'_, AppState>,
    text: String,
    name: String,
) -> CmdResult<StoredProfile> {
    let store = ProfileStore::new(state.manager.layout());
    let mut parsed = parse_payload(&text, &name, "pasted");
    parsed.profile.name = name.clone();

    // Re-importing under an existing name updates that record rather than
    // creating a second entry with the same title.
    let existing = store.find_by_name(&name).map_err(err)?;
    store
        .save(
            &parsed.profile,
            existing.as_ref().map(|entry| entry.id.as_str()),
        )
        .map_err(err)
}

#[tauri::command]
pub async fn set_active_profile(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let store = ProfileStore::new(state.manager.layout());
    store.set_active(Some(&id)).map_err(err)
}

#[derive(Serialize)]
pub struct RefreshResult {
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[tauri::command]
pub async fn refresh_profile(state: State<'_, AppState>, id: String) -> CmdResult<RefreshResult> {
    let store = ProfileStore::new(state.manager.layout());
    let service = triproxy_core::SubscriptionService::new(&store).map_err(err)?;
    let outcome = service.refresh(&id).await.map_err(err)?;
    Ok(match outcome.status {
        triproxy_core::RefreshStatus::Updated { node_count } => RefreshResult {
            kind: "updated".to_string(),
            node_count: Some(node_count),
            error: None,
        },
        triproxy_core::RefreshStatus::NotModified => RefreshResult {
            kind: "not_modified".to_string(),
            node_count: None,
            error: None,
        },
        triproxy_core::RefreshStatus::Failed { error } => RefreshResult {
            kind: "failed".to_string(),
            node_count: None,
            error: Some(error),
        },
    })
}

#[tauri::command]
pub async fn delete_profile(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let store = ProfileStore::new(state.manager.layout());
    store.delete(&id).map_err(err)
}

#[tauri::command]
pub async fn settings(state: State<'_, AppState>) -> CmdResult<AppSettings> {
    Ok(AppSettings::load(state.manager.layout()))
}

#[tauri::command]
pub async fn save_settings(state: State<'_, AppState>, settings: AppSettings) -> CmdResult<()> {
    settings.save(state.manager.layout()).map_err(err)
}

/// What the selector currently thinks about the active profile.
#[tauri::command]
pub async fn preview_selection(state: State<'_, AppState>) -> CmdResult<Option<Selection>> {
    let store = ProfileStore::new(state.manager.layout());
    let Some((_, profile)) = store.active().map_err(err)? else {
        return Ok(None);
    };
    let settings = AppSettings::load(state.manager.layout());
    let selector = Selector::with_preference(settings.preference());
    Ok(selector.select(&profile, settings.network()).ok())
}

/// Compile the active profile for one core, honouring the saved settings.
fn compile_active(
    state: &AppState,
    target: Option<CoreKind>,
) -> Result<(CoreKind, CompiledConfig), String> {
    let layout = state.manager.layout();
    let store = ProfileStore::new(layout);
    let (_, profile) = store
        .active()
        .map_err(err)?
        .ok_or_else(|| "还没有活动配置，请先在「配置」页导入并设为活动".to_string())?;

    let settings = AppSettings::load(layout);
    let selector = Selector::with_preference(match target {
        Some(core) => CorePreference::Fixed(core),
        None => settings.preference(),
    });

    let selection = selector.select(&profile, settings.network()).map_err(err)?;
    let options = settings.compile_options(7890, 9090);
    let compiled = compile(selection.selected, &profile, &options).map_err(err)?;
    Ok((selection.selected, compiled))
}

#[tauri::command]
pub async fn start_core(
    state: State<'_, AppState>,
    core: Option<CoreKind>,
) -> CmdResult<ManagerStatus> {
    // Without an active profile there is nothing to compile, so fall back to the
    // bootstrap config — the same precedence the CLI uses.
    let has_profile = ProfileStore::new(state.manager.layout())
        .active()
        .map_err(err)?
        .is_some();

    let settings = AppSettings::load(state.manager.layout());
    if settings.tun && !triproxy_core::is_elevated() {
        return Err("TUN 模式需要管理员权限：请以管理员身份重新启动 TriProxy".to_string());
    }

    if has_profile {
        let (kind, compiled) = compile_active(&state, core)?;
        state
            .manager
            .start_with_config(kind, &compiled.text, compiled.file_name)
            .await
            .map_err(err)?;
    } else {
        let kind = match core {
            Some(kind) => kind,
            None => state
                .manager
                .persisted_active()
                .await
                .ok_or_else(|| "还没有活动配置，请先导入配置".to_string())?,
        };
        state.manager.start(kind).await.map_err(err)?;
    }

    let settings = AppSettings::load(state.manager.layout());
    if settings.system_proxy {
        // A failure here must not undo a successful start; surface it instead.
        if let Err(error) = enable_system_proxy(&state) {
            tracing::warn!(%error, "could not enable the system proxy");
        }
    }

    Ok(state.manager.status().await)
}

fn enable_system_proxy(state: &AppState) -> Result<(), String> {
    let port = state
        .manager
        .persisted_listen_port()
        .ok_or_else(|| "no core is running".to_string())?;
    SystemProxyManager::new(state.manager.layout())
        .enable(&format!("127.0.0.1:{port}"))
        .map(|_| ())
        .map_err(err)
}

#[tauri::command]
pub async fn stop_core(state: State<'_, AppState>) -> CmdResult<()> {
    state.manager.stop().await.map_err(err)?;
    // The proxy is useless without a core and would break connectivity.
    SystemProxyManager::new(state.manager.layout())
        .disable()
        .map_err(err)?;
    Ok(())
}

#[tauri::command]
pub async fn switch_core(state: State<'_, AppState>, target: String) -> CmdResult<ManagerStatus> {
    let requested = if target == "smart" {
        None
    } else {
        Some(target.parse::<CoreKind>().map_err(err)?)
    };

    let settings = AppSettings::load(state.manager.layout());
    if settings.tun && !triproxy_core::is_elevated() {
        return Err("TUN 模式需要管理员权限：请以管理员身份重新启动 TriProxy".to_string());
    }

    let (kind, compiled) = compile_active(&state, requested)?;
    state
        .manager
        .switch_with_config(kind, &compiled.text, compiled.file_name)
        .await
        .map_err(err)?;
    Ok(state.manager.status().await)
}

#[derive(Serialize)]
pub struct LogEntryView {
    seq: u64,
    ts_ms: u64,
    core: String,
    level: String,
    text: String,
}

fn to_view(line: LogLine) -> LogEntryView {
    LogEntryView {
        seq: line.seq,
        ts_ms: line.ts_ms,
        core: line.core,
        level: match line.level {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
        .to_string(),
        text: line.text,
    }
}

#[tauri::command]
pub async fn test_latency(
    state: State<'_, AppState>,
    timeout_secs: u64,
) -> CmdResult<Vec<triproxy_core::LatencySample>> {
    let store = ProfileStore::new(state.manager.layout());
    let Some((_, profile)) = store.active().map_err(err)? else {
        return Ok(Vec::new());
    };
    Ok(triproxy_core::measure_nodes(
        &profile.nodes,
        std::time::Duration::from_secs(timeout_secs.max(1)),
    )
    .await)
}

#[tauri::command]
pub async fn recent_logs(state: State<'_, AppState>, limit: usize) -> CmdResult<Vec<LogEntryView>> {
    let logs = state.manager.logs();
    Ok(logs.tail(limit).into_iter().map(to_view).collect())
}

#[derive(Serialize)]
pub struct TunStatus {
    elevated: bool,
    leftovers: Vec<triproxy_core::TunAdapter>,
}

#[tauri::command]
pub async fn tun_status() -> CmdResult<TunStatus> {
    Ok(TunStatus {
        elevated: triproxy_core::is_elevated(),
        leftovers: triproxy_core::leftovers().map_err(err)?,
    })
}

#[tauri::command]
pub async fn system_proxy_status(state: State<'_, AppState>) -> CmdResult<Option<AppliedProxy>> {
    Ok(SystemProxyManager::new(state.manager.layout()).status())
}

#[tauri::command]
pub async fn set_system_proxy(
    state: State<'_, AppState>,
    enabled: bool,
) -> CmdResult<Option<AppliedProxy>> {
    let proxy = SystemProxyManager::new(state.manager.layout());
    if !enabled {
        proxy.disable().map_err(err)?;
        return Ok(None);
    }

    let port = state
        .manager
        .persisted_listen_port()
        .ok_or_else(|| "还没有核心在运行，无法启用系统代理".to_string())?;
    let applied = proxy.enable(&format!("127.0.0.1:{port}")).map_err(err)?;
    Ok(Some(applied))
}
