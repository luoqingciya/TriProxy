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
    AppSettings, AppliedProxy, CompiledConfig, ControllerClient, CoreKind, CorePreference,
    InstallOptions, LogLevel, LogLine, ManagerStatus, ProfileStore, Selection, Selector,
    StoredProfile, SystemProxyManager, UpdateCheck,
};

use crate::AppState;

/// Port the cores expose their controller API on.
///
/// Separate from the inbound port: it is loopback-only and never user-facing,
/// so it stays fixed rather than becoming another setting to get wrong.
const API_PORT: u16 = triproxy_core::core::DEFAULT_API_PORT;

type CmdResult<T> = Result<T, String>;

fn err(message: impl std::fmt::Display) -> String {
    message.to_string()
}

/// What the tray should say it is currently doing.
///
/// One line, because a tray menu is read at a glance: which core, and whether it
/// is following the automatic group or a node the user picked.
fn tray_status_node(state: &AppState) -> String {
    let store = ProfileStore::new(state.manager.layout());
    match store.active() {
        Ok(Some((record, profile))) => match record.selected_node {
            Some(id) => profile
                .nodes
                .iter()
                .find(|node| node.id == id)
                .map(|node| node.name.clone())
                .unwrap_or_else(|| "自动选择".to_string()),
            None => "自动选择".to_string(),
        },
        _ => "无配置".to_string(),
    }
}

/// Refresh the tray menu after anything that changes what it reports.
pub(crate) async fn sync_tray(app: &tauri::AppHandle) {
    use tauri::Manager;

    let state = app.state::<AppState>();
    let node = tray_status_node(&state);
    let label = match state.manager.persisted_active().await {
        Some(kind) => format!("{} · {node}", kind.display_name()),
        None => "未运行".to_string(),
    };
    crate::tray::set_status(app, &label);
}

#[tauri::command]
pub async fn status(state: State<'_, AppState>) -> CmdResult<ManagerStatus> {
    Ok(state.manager.status().await)
}

/// Download and install a core binary.
///
/// The lite installer ships no cores, so without this a user who picked lite
/// sees "未安装" and no way forward inside the app — the only documented path
/// was `triproxy core install` on a terminal they may not have opened.
#[tauri::command]
pub async fn install_core(
    state: State<'_, AppState>,
    core: CoreKind,
    mirror: Option<String>,
) -> CmdResult<ManagerStatus> {
    let options = InstallOptions {
        version: None,
        force: false,
        allow_unverified: false,
        local_file: None,
        mirror,
    };
    state.manager.install(core, &options).await.map_err(err)?;
    Ok(state.manager.status().await)
}

/// Compare the installed core against the latest upstream release.
#[tauri::command]
pub async fn check_core_update(
    state: State<'_, AppState>,
    core: CoreKind,
) -> CmdResult<UpdateCheck> {
    state.manager.binaries().check_update(core).await.map_err(err)
}

/// Whether the OS will launch TriProxy at login.
///
/// Read from the OS, not from `settings.json`: the user can remove the entry by
/// hand, and a toggle that claims "on" while nothing is registered is a lie the
/// user only discovers at the next reboot.
#[tauri::command]
pub async fn autostart_enabled(app: tauri::AppHandle) -> CmdResult<bool> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(err)
}

/// Register or remove the login entry. Returns the state the OS reports after.
#[tauri::command]
pub async fn set_autostart(app: tauri::AppHandle, enabled: bool) -> CmdResult<bool> {
    use tauri_plugin_autostart::ManagerExt;

    let launcher = app.autolaunch();
    if enabled {
        launcher.enable().map_err(err)?;
    } else {
        launcher.disable().map_err(err)?;
    }
    launcher.is_enabled().map_err(err)
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
        .save_activating_if_first(
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

/// Star or unstar a node in the active profile.
#[tauri::command]
pub async fn set_favorite(
    state: State<'_, AppState>,
    node_id: String,
    favorite: bool,
) -> CmdResult<bool> {
    let store = ProfileStore::new(state.manager.layout());
    let (stored, _) = store
        .active()
        .map_err(err)?
        .ok_or_else(|| "还没有活动配置，请先在「配置」页导入并设为活动".to_string())?;
    store
        .set_favorite(&stored.id, &node_id, favorite)
        .map_err(err)
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
    // Validate before writing: the compiler silently drops unparsable rules so
    // that one typo cannot take the connection down, which means a bad line would
    // otherwise disappear with nothing but a log entry to show for it.
    if let Some(text) = settings.custom_rules.as_deref() {
        triproxy_core::parse_rules(text).map_err(|error| format!("自定义规则 {error}"))?;
    }
    settings.save(state.manager.layout()).map_err(err)
}

/// Check rule text without saving it, so the editor can complain in place.
#[tauri::command]
pub async fn validate_rules(text: String) -> CmdResult<()> {
    triproxy_core::parse_rules(&text)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Ask for a destination and write everything the user configured into it.
///
/// Returns `None` when the dialog was dismissed: cancelling is not an error, and
/// reporting it as one would train people to ignore the banner.
#[tauri::command]
pub async fn export_backup(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("TriProxy 备份", &["json"])
        .set_file_name("triproxy-backup.json")
        .save_file(move |path| {
            let _ = tx.send(path);
        });

    let Some(path) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = path.into_path().map_err(err)?;

    let layout = state.manager.layout();
    let store = ProfileStore::new(layout);
    let backup =
        crate::backup::Backup::capture(&store, AppSettings::load(layout)).map_err(err)?;
    let text = serde_json::to_string_pretty(&backup).map_err(err)?;
    std::fs::write(&path, text).map_err(err)?;

    Ok(Some(path.display().to_string()))
}

/// Ask for a backup file and merge it into the current data directory.
#[tauri::command]
pub async fn import_backup(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<usize>> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("TriProxy 备份", &["json"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });

    let Some(path) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = path.into_path().map_err(err)?;

    let text = std::fs::read_to_string(&path).map_err(err)?;
    let backup = crate::backup::Backup::parse(&text)?;

    let layout = state.manager.layout();
    let store = ProfileStore::new(layout);
    let count = backup.restore(&store, layout)?;

    Ok(Some(count))
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
    let (stored, profile) = store
        .active()
        .map_err(err)?
        .ok_or_else(|| "还没有活动配置，请先在「配置」页导入并设为活动".to_string())?;

    let settings = AppSettings::load(layout);
    let selector = Selector::with_preference(match target {
        Some(core) => CorePreference::Fixed(core),
        None => settings.preference(),
    });

    let selection = selector.select(&profile, settings.network()).map_err(err)?;
    let mut options = settings.compile_options(settings.listen_port, API_PORT);
    // The node the user picked has to survive a restart, and the only way it does
    // is by going into the config: the runtime switch through the controller is
    // gone the moment the core stops.
    options.selected_node = stored.selected_node.clone();
    let compiled = compile(selection.selected, &profile, &options).map_err(err)?;
    Ok((selection.selected, compiled))
}

#[tauri::command]
pub async fn start_core(
    app: tauri::AppHandle,
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

    sync_tray(&app).await;
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
pub async fn stop_core(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<()> {
    state.manager.stop().await.map_err(err)?;
    // The proxy is useless without a core and would break connectivity.
    SystemProxyManager::new(state.manager.layout())
        .disable()
        .map_err(err)?;
    sync_tray(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn switch_core(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    target: String,
) -> CmdResult<ManagerStatus> {
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
    sync_tray(&app).await;
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

#[derive(Serialize)]
pub struct SelectResult {
    applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[tauri::command]
pub async fn select_node(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    node_id: Option<String>,
) -> CmdResult<SelectResult> {
    let store = ProfileStore::new(state.manager.layout());

    // Only hand the client over when the API actually answers: a stale port on a
    // stopped core would otherwise turn every click into an error.
    let client = match ControllerClient::new(state.manager.api_port()) {
        Ok(client) if client.version().await.is_ok() => Some(client),
        _ => None,
    };

    let outcome = triproxy_core::select_node(&store, client.as_ref(), node_id.as_deref())
        .await
        .map_err(err)?;

    sync_tray(&app).await;

    Ok(match outcome {
        triproxy_core::ApplyOutcome::Applied { node } => SelectResult {
            applied: true,
            node: Some(node),
            reason: None,
        },
        triproxy_core::ApplyOutcome::SavedOnly { node, reason } => SelectResult {
            applied: false,
            node,
            reason: Some(reason),
        },
    })
}

#[tauri::command]
pub async fn test_proxy_delay(
    state: State<'_, AppState>,
    timeout_secs: u64,
) -> CmdResult<Vec<triproxy_core::LatencySample>> {
    let store = ProfileStore::new(state.manager.layout());
    let Some((_, profile)) = store.active().map_err(err)? else {
        return Ok(Vec::new());
    };

    let client = ControllerClient::new(state.manager.api_port()).map_err(err)?;
    client.version().await.map_err(|error| {
        format!("真实延迟需要运行中的 Mihomo 或 sing-box（Xray 没有控制器 API）：{error}")
    })?;

    Ok(triproxy_core::measure_proxy_delay(
        &client,
        &profile.nodes,
        triproxy_core::DEFAULT_PROBE_URL,
        timeout_secs.max(1) * 1000,
    )
    .await)
}

#[tauri::command]
pub async fn traffic(
    state: State<'_, AppState>,
) -> CmdResult<Option<triproxy_core::TrafficSample>> {
    let client = ControllerClient::new(state.manager.api_port()).map_err(err)?;
    // The controller is only present for Mihomo and sing-box; a missing API is
    // a normal state, not an error worth a banner.
    Ok(client.traffic_once().await.ok())
}

#[tauri::command]
pub async fn connections(
    state: State<'_, AppState>,
) -> CmdResult<Option<triproxy_core::ConnectionsSnapshot>> {
    let client = ControllerClient::new(state.manager.api_port()).map_err(err)?;
    Ok(client.connections().await.ok())
}

#[tauri::command]
pub async fn close_connection(state: State<'_, AppState>, id: Option<String>) -> CmdResult<()> {
    let client = ControllerClient::new(state.manager.api_port()).map_err(err)?;
    match id {
        Some(id) => client.close_connection(&id).await.map_err(err),
        None => client.close_all().await.map_err(err),
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
