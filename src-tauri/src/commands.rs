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

use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::State;

use triproxy_core::compiler::compile;
use triproxy_core::profile::{parse_payload, Node};
use triproxy_core::{
    AppSettings, AppliedProxy, CompiledConfig, ControllerClient, CoreKind, CorePreference,
    InstallOptions, LogLevel, LogLine, ManagerStatus, ProfileStore, Selection, Selector,
    StoredProfile, SubscriptionService, SystemProxyManager, UpdateCheck,
};

use crate::AppState;

type CmdResult<T> = Result<T, String>;

fn err(message: impl std::fmt::Display) -> String {
    message.to_string()
}

async fn blocking<F, T>(f: F) -> CmdResult<T>
where
    F: FnOnce() -> CmdResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|error| error.to_string())?
}

async fn controller_client(state: &AppState) -> CmdResult<ControllerClient> {
    let port = state.manager.api_port();
    let secret = state.manager.api_secret();
    if let Some((cached_port, cached_secret, client)) = state.controller.read().await.as_ref() {
        if *cached_port == port && *cached_secret == secret {
            return Ok(client.clone());
        }
    }
    let client = state.manager.controller_client().map_err(err)?;
    *state.controller.write().await = Some((port, secret, client.clone()));
    Ok(client)
}

/// Pre-flight and interactive elevation for TUN in the desktop app.
///
/// If privilege is missing on Linux/macOS, this copies any bundled/PATH TUN cores
/// into `<data>/bin` and triggers the OS password dialog (`pkexec` / `osascript`)
/// so the user does not have to run a terminal command manually.
async fn ensure_tun_ready(state: &AppState, kind: Option<CoreKind>) -> CmdResult<()> {
    let check_kind = kind.unwrap_or(CoreKind::SingBox);
    let binary = state.manager.binary_path_for(check_kind);
    let initial_reason =
        blocking(move || Ok(triproxy_core::tun_unavailable_reason_for(binary.as_deref()))).await?;
    if initial_reason.is_none() {
        return Ok(());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let manager = state.manager.clone();
        blocking(move || {
            let paths = manager
                .ensure_managed_for_tun(&[CoreKind::SingBox, CoreKind::Mihomo])
                .map_err(err)?;
            triproxy_core::authorize_tun_binaries(&paths).map_err(err)?;
            let updated_binary = manager.binary_path_for(check_kind);
            if let Some(reason) =
                triproxy_core::tun_unavailable_reason_for(updated_binary.as_deref())
            {
                return Err(reason);
            }
            Ok(())
        })
        .await
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(initial_reason.unwrap_or_else(|| "TUN 模式需要管理员权限".to_string()))
    }
}

async fn apply_network_proxy_state(state: &AppState, settings: &AppSettings) {
    if settings.tun {
        let layout = state.manager.layout().clone();
        if blocking(move || SystemProxyManager::new(&layout).disable().map_err(err))
            .await
            .is_ok()
        {
            state.set_cached_system_proxy(None);
        }
    } else if settings.system_proxy {
        if let Some(port) = state.manager.persisted_http_proxy_port() {
            let layout = state.manager.layout().clone();
            match blocking(move || {
                SystemProxyManager::new(&layout)
                    .enable(&format!("127.0.0.1:{port}"))
                    .map_err(err)
            })
            .await
            {
                Ok(applied) => state.set_cached_system_proxy(Some(applied)),
                Err(error) => {
                    tracing::warn!(%error, "could not enable the system proxy after reload")
                }
            }
        }
    }
}

async fn apply_compiled_switch(
    app: &tauri::AppHandle,
    state: &AppState,
    settings: &AppSettings,
    requested: Option<CoreKind>,
) -> CmdResult<ManagerStatus> {
    let layout = state.manager.layout().clone();
    let api_secret = state.manager.ensure_api_secret();
    let compile_settings = settings.clone();
    let (kind, compiled) =
        blocking(move || compile_with_settings(&layout, &compile_settings, requested, &api_secret))
            .await?;

    if settings.tun {
        ensure_tun_ready(state, Some(kind)).await?;
    }

    state
        .manager
        .switch_with_config(kind, &compiled.text, compiled.file_name)
        .await
        .map_err(err)?;
    state.compiled_with_rules.store(
        settings.rule_mode == triproxy_core::RuleMode::Split,
        Ordering::Relaxed,
    );

    apply_network_proxy_state(state, settings).await;
    state.notify_traffic();
    sync_tray(app).await;
    Ok(state.manager.status().await)
}

/// What the tray should say it is currently doing.
///
/// One line, because a tray menu is read at a glance: which core, and whether it
/// is following the automatic group or a node the user picked.
fn tray_status_node(layout: &triproxy_core::Layout) -> String {
    let store = ProfileStore::new(layout);
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
    let layout = state.manager.layout().clone();
    let node = blocking(move || Ok(tray_status_node(&layout)))
        .await
        .unwrap_or_else(|_| "无配置".to_string());
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
    state
        .manager
        .binaries()
        .check_update(core)
        .await
        .map_err(err)
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
    let layout = state.manager.layout().clone();
    blocking(move || {
        let store = ProfileStore::new(&layout);
        Ok(ProfileLibrary {
            active: store.active_id().map_err(err)?,
            profiles: store.list().map_err(err)?,
        })
    })
    .await
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

#[derive(Serialize)]
pub struct LibrarySnapshot {
    library: ProfileLibrary,
    nodes: Vec<NodeSummary>,
    selection: Option<Selection>,
}

#[tauri::command]
pub async fn library_snapshot(state: State<'_, AppState>) -> CmdResult<LibrarySnapshot> {
    let layout = state.manager.layout().clone();
    let settings = state.cached_settings();
    blocking(move || {
        let store = ProfileStore::new(&layout);
        let active = store.active_id().map_err(err)?;
        let profiles = store.list().map_err(err)?;
        let active_profile = store.active().map_err(err)?;
        let (nodes, selection) = match active_profile {
            Some((_, profile)) => {
                let nodes = profile.nodes.iter().map(summarise).collect();
                let selector = Selector::with_preference(settings.preference());
                let selection = selector.select(&profile, settings.network()).ok();
                (nodes, selection)
            }
            None => (Vec::new(), None),
        };
        Ok(LibrarySnapshot {
            library: ProfileLibrary { active, profiles },
            nodes,
            selection,
        })
    })
    .await
}

#[tauri::command]
pub async fn active_nodes(state: State<'_, AppState>) -> CmdResult<Vec<NodeSummary>> {
    let layout = state.manager.layout().clone();
    blocking(move || {
        let store = ProfileStore::new(&layout);
        let Some((_, profile)) = store.active().map_err(err)? else {
            return Ok(Vec::new());
        };
        Ok(profile.nodes.iter().map(summarise).collect())
    })
    .await
}

#[tauri::command]
pub async fn import_profile(
    state: State<'_, AppState>,
    text: String,
    name: String,
) -> CmdResult<StoredProfile> {
    let trimmed = text.trim();
    let is_single_url = !trimmed.is_empty()
        && !trimmed.contains('\n')
        && !trimmed.contains('\r')
        && !trimmed.chars().any(char::is_whitespace)
        && (trimmed.starts_with("http://") || trimmed.starts_with("https://"));

    if is_single_url {
        let effective_name = if name.trim().is_empty() || name.trim() == "未命名配置" {
            tauri::Url::parse(trimmed)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_string))
                .unwrap_or_else(|| "订阅".to_string())
        } else {
            name.trim().to_string()
        };
        let store = ProfileStore::new(state.manager.layout());
        let service = SubscriptionService::with_client(&store, state.subscription_client.clone());
        let saved = service
            .add(trimmed, &effective_name, 3600)
            .await
            .map_err(err)?;
        if store.active_id().map_err(err)?.is_none() {
            store.set_active(Some(&saved.id)).map_err(err)?;
        }
        return Ok(saved);
    }

    let layout = state.manager.layout().clone();
    blocking(move || {
        let store = ProfileStore::new(&layout);
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
    })
    .await
}

#[tauri::command]
pub async fn set_active_profile(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let layout = state.manager.layout().clone();
    blocking(move || {
        ProfileStore::new(&layout)
            .set_active(Some(&id))
            .map_err(err)
    })
    .await
}

/// Star or unstar a node in the active profile.
#[tauri::command]
pub async fn set_favorite(
    state: State<'_, AppState>,
    node_id: String,
    favorite: bool,
) -> CmdResult<bool> {
    let layout = state.manager.layout().clone();
    blocking(move || {
        let store = ProfileStore::new(&layout);
        let (stored, _) = store
            .active()
            .map_err(err)?
            .ok_or_else(|| "还没有活动配置，请先在「配置」页导入并设为活动".to_string())?;
        store
            .set_favorite(&stored.id, &node_id, favorite)
            .map_err(err)
    })
    .await
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
    let service = SubscriptionService::with_client(&store, state.subscription_client.clone());
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
    let layout = state.manager.layout().clone();
    blocking(move || ProfileStore::new(&layout).delete(&id).map_err(err)).await
}

#[tauri::command]
pub async fn settings(state: State<'_, AppState>) -> CmdResult<AppSettings> {
    Ok(state.cached_settings())
}

#[tauri::command]
pub async fn save_settings(state: State<'_, AppState>, settings: AppSettings) -> CmdResult<()> {
    let layout = state.manager.layout().clone();
    let saved = settings.clone();
    blocking(move || {
        // Validate before writing: the compiler silently drops unparsable rules so
        // that one typo cannot take the connection down, which means a bad line would
        // otherwise disappear with nothing but a log entry to show for it.
        if let Some(text) = settings.custom_rules.as_deref() {
            triproxy_core::parse_rules(text).map_err(|error| format!("自定义规则 {error}"))?;
        }
        if let Some(text) = settings.rule_sets.as_deref() {
            triproxy_core::parse_rule_sets(text).map_err(|error| format!("规则集 {error}"))?;
        }
        settings.save(&layout).map_err(err)
    })
    .await?;
    state.set_cached_settings(saved);
    Ok(())
}

/// Check rule text without saving it, so the editor can complain in place.
#[tauri::command]
pub async fn validate_rules(text: String) -> CmdResult<()> {
    triproxy_core::parse_rules(&text)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Check rule-set text without saving it, so the editor can complain in place.
#[tauri::command]
pub async fn validate_rule_sets(text: String) -> CmdResult<()> {
    triproxy_core::parse_rule_sets(&text)
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

    let layout = state.manager.layout().clone();
    blocking(move || {
        let store = ProfileStore::new(&layout);
        let backup =
            crate::backup::Backup::capture(&store, AppSettings::load(&layout)).map_err(err)?;
        let text = serde_json::to_string_pretty(&backup).map_err(err)?;
        triproxy_core::paths::write_atomic(&path, text).map_err(err)?;
        Ok(Some(path.display().to_string()))
    })
    .await
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

    let layout = state.manager.layout().clone();
    let (count, loaded_settings) = blocking(move || {
        let text = std::fs::read_to_string(&path).map_err(err)?;
        let backup = crate::backup::Backup::parse(&text)?;
        let store = ProfileStore::new(&layout);
        let count = backup.restore(&store, &layout)?;
        Ok((count, AppSettings::load(&layout)))
    })
    .await?;
    state.set_cached_settings(loaded_settings);
    Ok(Some(count))
}

/// What the selector currently thinks about the active profile.
#[tauri::command]
pub async fn preview_selection(state: State<'_, AppState>) -> CmdResult<Option<Selection>> {
    let layout = state.manager.layout().clone();
    let settings = state.cached_settings();
    blocking(move || {
        let store = ProfileStore::new(&layout);
        let Some((_, profile)) = store.active().map_err(err)? else {
            return Ok(None);
        };
        let selector = Selector::with_preference(settings.preference());
        Ok(selector.select(&profile, settings.network()).ok())
    })
    .await
}

enum StartPlan {
    Compiled(CoreKind, CompiledConfig),
    Bootstrap,
}

fn compile_with_settings(
    layout: &triproxy_core::Layout,
    settings: &AppSettings,
    target: Option<CoreKind>,
    api_secret: &str,
) -> Result<(CoreKind, CompiledConfig), String> {
    let store = ProfileStore::new(layout);
    let (stored, profile) = store
        .active()
        .map_err(err)?
        .ok_or_else(|| "还没有活动配置，请先在「配置」页导入并设为活动".to_string())?;

    let selector = Selector::with_preference(match target {
        Some(core) => CorePreference::Fixed(core),
        None => settings.preference(),
    });

    let selection = selector.select(&profile, settings.network()).map_err(err)?;
    let mut options = settings.compile_options(settings.listen_port, settings.api_port);
    options.api_secret = Some(api_secret.to_string());
    options.rule_set_mirror = std::env::var(triproxy_core::binaries::MIRROR_ENV).ok();
    // The node the user picked has to survive a restart, and the only way it does
    // is by going into the config: the runtime switch through the controller is
    // gone the moment the core stops.
    options.selected_node = stored.selected_node.clone();
    let compiled = compile(selection.selected, &profile, &options).map_err(err)?;
    Ok((selection.selected, compiled))
}

fn prepare_start_plan(
    layout: &triproxy_core::Layout,
    settings: &AppSettings,
    target: Option<CoreKind>,
    api_secret: &str,
) -> Result<StartPlan, String> {
    let store = ProfileStore::new(layout);
    let Some((stored, profile)) = store.active().map_err(err)? else {
        return Ok(StartPlan::Bootstrap);
    };

    let selector = Selector::with_preference(match target {
        Some(core) => CorePreference::Fixed(core),
        None => settings.preference(),
    });

    let selection = selector.select(&profile, settings.network()).map_err(err)?;
    let mut options = settings.compile_options(settings.listen_port, settings.api_port);
    options.api_secret = Some(api_secret.to_string());
    options.rule_set_mirror = std::env::var(triproxy_core::binaries::MIRROR_ENV).ok();
    options.selected_node = stored.selected_node.clone();
    let compiled = compile(selection.selected, &profile, &options).map_err(err)?;
    Ok(StartPlan::Compiled(selection.selected, compiled))
}

#[tauri::command]
pub async fn start_core(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    core: Option<CoreKind>,
) -> CmdResult<ManagerStatus> {
    let settings = state.cached_settings();
    let layout = state.manager.layout().clone();
    let api_secret = state.manager.ensure_api_secret();
    let plan_settings = settings.clone();
    let plan =
        blocking(move || prepare_start_plan(&layout, &plan_settings, core, &api_secret)).await?;

    match plan {
        StartPlan::Compiled(kind, compiled) => {
            if settings.tun {
                ensure_tun_ready(&state, Some(kind)).await?;
            }
            state
                .manager
                .start_with_config(kind, &compiled.text, compiled.file_name)
                .await
                .map_err(err)?;
            state.compiled_with_rules.store(
                settings.rule_mode == triproxy_core::RuleMode::Split,
                Ordering::Relaxed,
            );
        }
        StartPlan::Bootstrap => {
            let kind = match core {
                Some(kind) => kind,
                None => state
                    .manager
                    .persisted_active()
                    .await
                    .ok_or_else(|| "还没有活动配置，请先导入配置".to_string())?,
            };
            if settings.tun {
                ensure_tun_ready(&state, Some(kind)).await?;
            }
            state.manager.start(kind).await.map_err(err)?;
            state.compiled_with_rules.store(false, Ordering::Relaxed);
        }
    }

    apply_network_proxy_state(&state, &settings).await;
    state.notify_traffic();
    sync_tray(&app).await;
    Ok(state.manager.status().await)
}

#[tauri::command]
pub async fn stop_core(app: tauri::AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    state.manager.stop().await.map_err(err)?;
    // The proxy is useless without a core and would break connectivity.
    let layout = state.manager.layout().clone();
    blocking(move || {
        SystemProxyManager::new(&layout)
            .disable()
            .map(|_| ())
            .map_err(err)
    })
    .await?;
    state.set_cached_system_proxy(None);
    if let Ok(mut guard) = state.latest_traffic.write() {
        *guard = None;
    }
    state.notify_traffic();
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

    let settings = state.cached_settings();
    apply_compiled_switch(&app, &state, &settings, requested).await
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
    let client = match controller_client(&state).await {
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
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    timeout_secs: u64,
) -> CmdResult<Vec<triproxy_core::LatencySample>> {
    use tauri::Emitter;

    let layout = state.manager.layout().clone();
    let Some((_, profile)) =
        blocking(move || ProfileStore::new(&layout).active().map_err(err)).await?
    else {
        return Ok(Vec::new());
    };

    let client = controller_client(&state).await?;
    client.version().await.map_err(|error| {
        format!("真实延迟需要运行中的 Mihomo 或 sing-box（Xray 没有控制器 API）：{error}")
    })?;

    Ok(triproxy_core::measure_proxy_delay_streaming(
        &client,
        &profile.nodes,
        triproxy_core::DEFAULT_PROBE_URL,
        timeout_secs.max(1) * 1000,
        12,
        move |sample| {
            let _ = app.emit(crate::LATENCY_SAMPLE_EVENT, sample);
        },
    )
    .await)
}

#[tauri::command]
pub async fn active_delay(state: State<'_, AppState>) -> CmdResult<triproxy_core::ActiveDelay> {
    let client = match controller_client(&state).await {
        Ok(client) => client,
        Err(error) => {
            return Ok(triproxy_core::ActiveDelay {
                node: None,
                latency_ms: None,
                error: Some(error),
            })
        }
    };

    // A stopped core leaves a stale port behind; asking the controller first
    // turns that into one clear sentence instead of a connection error.
    if let Err(error) = client.version().await {
        return Ok(triproxy_core::ActiveDelay {
            node: None,
            latency_ms: None,
            error: Some(format!(
                "需要运行中的 Mihomo 或 sing-box（Xray 没有控制器 API）：{error}"
            )),
        });
    }

    Ok(triproxy_core::active_delay(
        &client,
        triproxy_core::PROXY_GROUP,
        triproxy_core::DEFAULT_PROBE_URL,
        3000,
    )
    .await)
}

#[tauri::command]
pub async fn traffic(
    state: State<'_, AppState>,
) -> CmdResult<Option<triproxy_core::TrafficSample>> {
    if let Ok(guard) = state.latest_traffic.read() {
        if let Some(sample) = *guard {
            return Ok(Some(sample));
        }
    }
    if !state
        .manager
        .persisted_active()
        .await
        .is_some_and(|kind| kind.has_controller())
    {
        return Ok(None);
    }
    let client = controller_client(&state).await?;
    Ok(client.traffic_once().await.ok())
}

#[tauri::command]
pub async fn connections(
    state: State<'_, AppState>,
) -> CmdResult<Option<triproxy_core::ConnectionsSnapshot>> {
    let client = controller_client(&state).await?;
    Ok(client.connections().await.ok())
}

#[tauri::command]
pub async fn close_connection(state: State<'_, AppState>, id: Option<String>) -> CmdResult<()> {
    let client = controller_client(&state).await?;
    match id {
        Some(id) => client.close_connection(&id).await.map_err(err),
        None => client.close_all().await.map_err(err),
    }
}

#[tauri::command]
pub async fn test_latency(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    timeout_secs: u64,
) -> CmdResult<Vec<triproxy_core::LatencySample>> {
    use tauri::Emitter;

    let layout = state.manager.layout().clone();
    let Some((_, profile)) =
        blocking(move || ProfileStore::new(&layout).active().map_err(err)).await?
    else {
        return Ok(Vec::new());
    };
    Ok(triproxy_core::measure_nodes_streaming(
        &profile.nodes,
        std::time::Duration::from_secs(timeout_secs.max(1)),
        16,
        move |sample| {
            let _ = app.emit(crate::LATENCY_SAMPLE_EVENT, sample);
        },
    )
    .await)
}

#[tauri::command]
pub async fn recent_logs(
    state: State<'_, AppState>,
    limit: usize,
    after_seq: Option<u64>,
) -> CmdResult<Vec<LogEntryView>> {
    let logs = state.manager.logs();
    Ok(logs
        .tail_after(after_seq, limit)
        .into_iter()
        .map(to_view)
        .collect())
}

#[derive(Serialize)]
pub struct TunStatus {
    elevated: bool,
    authorized: bool,
    interface_name: String,
    unavailable_reason: Option<String>,
    leftovers: Vec<triproxy_core::TunAdapter>,
}

fn build_tun_status(manager: &triproxy_core::CoreManager) -> CmdResult<TunStatus> {
    let elevated = triproxy_core::is_elevated();
    let singbox_bin = manager.binary_path_for(CoreKind::SingBox);
    let mihomo_bin = manager.binary_path_for(CoreKind::Mihomo);
    let installed_tun_bins: Vec<_> = [singbox_bin, mihomo_bin].into_iter().flatten().collect();

    let authorized = elevated
        || (!installed_tun_bins.is_empty()
            && installed_tun_bins
                .iter()
                .all(|path| triproxy_core::binary_has_tun_capability(path)));

    let unavailable_reason = if elevated {
        triproxy_core::tun_unavailable_reason_for(None)
    } else if installed_tun_bins.is_empty() {
        triproxy_core::tun_unavailable_reason_for(None)
    } else {
        let missing = installed_tun_bins
            .iter()
            .find(|path| !triproxy_core::binary_has_tun_capability(path))
            .unwrap_or(&installed_tun_bins[0]);
        triproxy_core::tun_unavailable_reason_for(Some(missing.as_path()))
    };

    Ok(TunStatus {
        elevated,
        authorized,
        interface_name: triproxy_core::INTERFACE_NAME.to_string(),
        unavailable_reason,
        leftovers: triproxy_core::leftovers().map_err(err)?,
    })
}

#[tauri::command]
pub async fn tun_status(state: State<'_, AppState>) -> CmdResult<TunStatus> {
    let manager = state.manager.clone();
    blocking(move || build_tun_status(&manager)).await
}

#[tauri::command]
pub async fn authorize_tun(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<TunStatus> {
    let _ = &app;
    #[cfg(windows)]
    if !triproxy_core::is_elevated() {
        let layout = state.manager.layout().clone();
        let mut settings = state.cached_settings();
        settings.tun = true;
        let to_save = settings.clone();
        blocking(move || to_save.save(&layout).map_err(err)).await?;
        state.set_cached_settings(settings);
        blocking(|| triproxy_core::relaunch_elevated().map_err(err)).await?;
        let exit_app = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            exit_app.exit(0);
        });
        let manager = state.manager.clone();
        return blocking(move || build_tun_status(&manager)).await;
    }

    let manager = state.manager.clone();
    blocking(move || {
        let paths = manager
            .ensure_managed_for_tun(&[CoreKind::SingBox, CoreKind::Mihomo])
            .map_err(err)?;
        triproxy_core::authorize_tun_binaries(&paths).map_err(err)?;
        build_tun_status(&manager)
    })
    .await
}

#[tauri::command]
pub async fn set_tun(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> CmdResult<ManagerStatus> {
    let mut next_settings = state.cached_settings();
    let target_core = next_settings.core_preference;

    if enabled {
        #[cfg(windows)]
        if !triproxy_core::is_elevated() {
            let layout = state.manager.layout().clone();
            next_settings.tun = true;
            let to_save = next_settings.clone();
            blocking(move || to_save.save(&layout).map_err(err)).await?;
            state.set_cached_settings(next_settings);
            blocking(|| triproxy_core::relaunch_elevated().map_err(err)).await?;
            let exit_app = app.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                exit_app.exit(0);
            });
            return Ok(state.manager.status().await);
        }

        ensure_tun_ready(&state, target_core).await?;
    }

    next_settings.tun = enabled;
    let layout = state.manager.layout().clone();
    let to_save = next_settings.clone();
    blocking(move || to_save.save(&layout).map_err(err)).await?;
    state.set_cached_settings(next_settings.clone());

    if state.manager.has_running_core().await {
        return apply_compiled_switch(&app, &state, &next_settings, target_core).await;
    }

    Ok(state.manager.status().await)
}

#[tauri::command]
pub async fn set_rule_mode(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mode: triproxy_core::RuleMode,
) -> CmdResult<bool> {
    let mut next_settings = state.cached_settings();
    next_settings.rule_mode = mode;

    let layout = state.manager.layout().clone();
    let to_save = next_settings.clone();
    blocking(move || to_save.save(&layout).map_err(err)).await?;
    state.set_cached_settings(next_settings.clone());

    if !state.manager.has_running_core().await {
        return Ok(false);
    }

    if state.compiled_with_rules.load(Ordering::Relaxed) {
        if let Ok(client) = controller_client(&state).await {
            if client.version().await.is_ok() {
                client.set_mode(mode.clash_mode()).await.map_err(err)?;
                return Ok(true);
            }
        }
    }

    apply_compiled_switch(&app, &state, &next_settings, next_settings.core_preference).await?;
    Ok(true)
}

#[tauri::command]
pub async fn test_node_delay(
    state: State<'_, AppState>,
    node_id: String,
    timeout_secs: u64,
) -> CmdResult<triproxy_core::LatencySample> {
    let layout = state.manager.layout().clone();
    let node = blocking(move || {
        let Some((_, profile)) = ProfileStore::new(&layout).active().map_err(err)? else {
            return Err("还没有活动配置".to_string());
        };
        profile
            .nodes
            .into_iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| "未找到该节点".to_string())
    })
    .await?;

    let timeout = std::time::Duration::from_secs(timeout_secs.max(1));
    let timeout_ms = (timeout.as_millis() as u64).max(1000);

    if let Ok(client) = controller_client(&state).await {
        if client.version().await.is_ok() {
            let result = client
                .proxy_delay(&node.name, triproxy_core::DEFAULT_PROBE_URL, timeout_ms)
                .await;
            return Ok(triproxy_core::LatencySample {
                node_id: node.id.clone(),
                name: node.name.clone(),
                endpoint: format!("{}:{}", node.server, node.port),
                kind: triproxy_core::LatencyKind::Http,
                latency_ms: result.as_ref().ok().copied(),
                error: result.err().map(|e| e.to_string()),
                measured_at_ms: triproxy_core::logging::now_ms(),
            });
        }
    }

    let result = triproxy_core::tcp_latency(&node.server, node.port, timeout).await;
    Ok(triproxy_core::LatencySample {
        node_id: node.id.clone(),
        name: node.name.clone(),
        endpoint: format!("{}:{}", node.server, node.port),
        kind: triproxy_core::LatencyKind::Tcp,
        latency_ms: result.as_ref().ok().copied(),
        error: result.err().map(|e| e.to_string()),
        measured_at_ms: triproxy_core::logging::now_ms(),
    })
}

#[tauri::command]
pub async fn system_proxy_status(state: State<'_, AppState>) -> CmdResult<Option<AppliedProxy>> {
    Ok(state.cached_system_proxy())
}

#[tauri::command]
pub async fn set_system_proxy(
    state: State<'_, AppState>,
    enabled: bool,
) -> CmdResult<Option<AppliedProxy>> {
    let layout = state.manager.layout().clone();
    let http_port = state.manager.persisted_http_proxy_port();
    let applied = blocking(move || {
        let proxy = SystemProxyManager::new(&layout);
        if !enabled {
            proxy.disable().map_err(err)?;
            return Ok(None);
        }

        let port = http_port.ok_or_else(|| "还没有核心在运行，无法启用系统代理".to_string())?;
        let applied = proxy.enable(&format!("127.0.0.1:{port}")).map_err(err)?;
        Ok(Some(applied))
    })
    .await?;
    state.set_cached_system_proxy(applied.clone());
    Ok(applied)
}
