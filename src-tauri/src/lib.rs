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

//! TriProxy desktop shell.
//!
//! The GUI owns no business logic: it constructs a [`CoreManager`] and exposes
//! it over Tauri commands. Everything the window shows comes from the same core
//! crate the CLI uses.

pub mod backup;
pub mod commands;
pub mod deep_link;
pub mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tauri::Emitter;
use triproxy_core::{
    AppSettings, AppliedProxy, ControllerClient, CoreManager, Layout, ProfileStore, RefreshStatus,
    SubscriptionClient, SubscriptionService, SystemProxyManager, TrafficSample,
};

/// Guards the exit path: cleanup is asynchronous, so the app has to be held open
/// once, then allowed through on the second exit request.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

pub const PROFILES_CHANGED_EVENT: &str = "triproxy-profiles-changed";
pub const TRAFFIC_EVENT: &str = "triproxy-traffic";
pub const STATUS_CHANGED_EVENT: &str = "triproxy-status-changed";
pub const LATENCY_SAMPLE_EVENT: &str = "triproxy-latency-sample";

/// Shared state handed to every command.
pub struct AppState {
    pub manager: Arc<CoreManager>,
    pub subscription_client: SubscriptionClient,
    pub controller: tokio::sync::RwLock<Option<(u16, Option<String>, ControllerClient)>>,
    pub latest_traffic: Arc<RwLock<Option<TrafficSample>>>,
    pub settings: RwLock<AppSettings>,
    pub system_proxy: RwLock<Option<AppliedProxy>>,
    pub window_visible: tokio::sync::watch::Sender<bool>,
    pub traffic_wake: Arc<tokio::sync::Notify>,
}

impl AppState {
    pub fn cached_settings(&self) -> AppSettings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_cached_settings(&self, settings: AppSettings) {
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = settings;
    }

    pub fn cached_system_proxy(&self) -> Option<AppliedProxy> {
        self.system_proxy
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_cached_system_proxy(&self, proxy: Option<AppliedProxy>) {
        *self.system_proxy.write().unwrap_or_else(|e| e.into_inner()) = proxy;
    }

    pub fn set_window_visible(&self, visible: bool) {
        let _ = self.window_visible.send_if_modified(|current| {
            if *current == visible {
                false
            } else {
                *current = visible;
                true
            }
        });
    }

    pub fn notify_traffic(&self) {
        self.traffic_wake.notify_one();
    }
}

pub fn run() {
    init_tracing();

    let layout = Layout::discover().expect("cannot resolve the application data directory");
    let manager = Arc::new(CoreManager::new(layout).expect("cannot initialise the core manager"));
    let subscription_client =
        SubscriptionClient::new().expect("cannot initialise the subscription client");
    let initial_settings = AppSettings::load(manager.layout());
    let initial_proxy = SystemProxyManager::new(manager.layout()).status();
    let latest_traffic = Arc::new(RwLock::new(None));
    let (window_visible_tx, window_visible_rx) = tokio::sync::watch::channel(true);
    let traffic_wake = Arc::new(tokio::sync::Notify::new());

    tauri::Builder::default()
        // Registered before anything else: it has to be the first thing that
        // runs in a second process, otherwise two copies would race for the
        // listen port before the second one hands over its arguments and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            for arg in &argv {
                deep_link::handle(app, arg);
            }
            // A second launch means "show me the window", link or no link.
            tray::show_window(app);
        }))
        .plugin(tauri_plugin_deep_link::init())
        // File pickers for backup and restore. Rust-side only, so no JS
        // permission is involved.
        .plugin(tauri_plugin_dialog::init())
        // Self-update. The plugin is what makes an installed copy able to move
        // to a newer release without the user downloading an installer again.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Autostart. The OS owns the registration, so the toggle reads its state
        // back from the OS rather than from our settings file.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState {
            manager: manager.clone(),
            subscription_client,
            controller: tokio::sync::RwLock::new(None),
            latest_traffic: latest_traffic.clone(),
            settings: RwLock::new(initial_settings),
            system_proxy: RwLock::new(initial_proxy),
            window_visible: window_visible_tx,
            traffic_wake: traffic_wake.clone(),
        })
        .setup(move |app| {
            use tauri::Manager;

            // Windows and Linux hand a clicked link to us as an argument, so the
            // scheme has to be registered with the OS. macOS reads the bundle's
            // Info.plist instead and delivers the URL as an event, handled below.
            #[cfg(any(windows, target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                if let Err(error) = app.deep_link().register_all() {
                    tracing::warn!(%error, "could not register the triproxy:// scheme");
                }
            }

            // Handle any triproxy:// URL passed on cold launch.
            for arg in std::env::args().skip(1) {
                deep_link::handle(app.handle(), &arg);
            }

            // A link clicked while the app is already running: macOS delivers an
            // Apple Event, Windows and Linux go through the single-instance
            // plugin above.
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        deep_link::handle(&handle, url.as_str());
                    }
                });
            }

            // Bundled cores do not sit beside the executable on every platform:
            // macOS keeps them in TriProxy.app/Contents/Resources/cores, and a
            // Linux .deb installs them under /usr/lib/triproxy/cores. The shell
            // knows the real path, the core crate cannot derive it, so it is
            // resolved here — before any command asks for a core.
            resolve_bundled_cores(app);

            // The tray must exist before the window can be closed, otherwise
            // hiding it would leave the user with no way back. A failure is
            // therefore non-fatal: the app simply behaves like a normal window.
            if let Err(error) = tray::install(app) {
                tracing::error!(%error, "could not create the tray icon");
            }

            // Adopt any leftover core, restore a stale system proxy snapshot,
            // refresh tray/UI status, and honor `auto_connect` without blocking
            // initial window creation.
            let startup_handle = app.handle().clone();
            let startup_manager = manager.clone();
            tauri::async_runtime::spawn(async move {
                match startup_manager.recover().await {
                    Ok(report) => {
                        if let Some(kind) = report.adopted {
                            tracing::info!(core = kind.as_str(), "adopted running core");
                        }
                    }
                    Err(error) => tracing::warn!(%error, "recovery failed"),
                }

                let has_running = startup_manager.has_running_core().await;
                let layout = startup_manager.layout().clone();
                let recovered_proxy = tokio::task::spawn_blocking(move || {
                    let proxy = SystemProxyManager::new(&layout);
                    if proxy.status().is_some() && !has_running {
                        if let Err(error) = proxy.recover() {
                            tracing::warn!(%error, "could not restore the system proxy");
                        }
                    }
                    proxy.status()
                })
                .await
                .ok()
                .flatten();

                {
                    let state = startup_handle.state::<AppState>();
                    state.set_cached_system_proxy(recovered_proxy);
                    state.notify_traffic();
                }

                if tray::is_ready() {
                    commands::sync_tray(&startup_handle).await;
                }
                let _ = startup_handle.emit(STATUS_CHANGED_EVENT, ());

                maybe_auto_connect_async(&startup_handle).await;
            });

            // Subscriptions used to refresh only when the user asked. A profile
            // that quietly ages out is worse than one that never updates, so the
            // scheduler runs for as long as the app does — including while it
            // sits in the tray, which is now most of the time.
            let scheduler = app.state::<AppState>().manager.clone();
            let scheduler_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                refresh_subscriptions_forever(scheduler_handle, scheduler).await;
            });

            // Keep one persistent /traffic stream open while a controller-capable
            // core is running and the main window is visible.
            let traffic_handle = app.handle().clone();
            let traffic_manager = app.state::<AppState>().manager.clone();
            let traffic_slot = app.state::<AppState>().latest_traffic.clone();
            let traffic_wake = app.state::<AppState>().traffic_wake.clone();
            tauri::async_runtime::spawn(async move {
                stream_traffic_forever(
                    traffic_handle,
                    traffic_manager,
                    traffic_slot,
                    window_visible_rx,
                    traffic_wake,
                )
                .await;
            });

            Ok(())
        })
        // Closing the window hides it; the proxy is supposed to keep running.
        // Only an explicit quit (tray menu) or `close_to_tray = false` exits.
        .on_window_event(|window, event| {
            use tauri::Manager;
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let app = window.app_handle();
                    let close_to_tray = app.state::<AppState>().cached_settings().close_to_tray;
                    if close_to_tray && tray::is_ready() {
                        api.prevent_close();
                        let _ = window.hide();
                        app.state::<AppState>().set_window_visible(false);
                    }
                }
                tauri::WindowEvent::Focused(true) => {
                    window
                        .app_handle()
                        .state::<AppState>()
                        .set_window_visible(true);
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::profiles,
            commands::active_nodes,
            commands::library_snapshot,
            commands::import_profile,
            commands::set_active_profile,
            commands::delete_profile,
            commands::refresh_profile,
            commands::settings,
            commands::save_settings,
            commands::validate_rules,
            commands::validate_rule_sets,
            commands::export_backup,
            commands::import_backup,
            commands::preview_selection,
            commands::start_core,
            commands::stop_core,
            commands::switch_core,
            commands::install_core,
            commands::check_core_update,
            commands::autostart_enabled,
            commands::set_autostart,
            commands::select_node,
            commands::set_favorite,
            commands::test_proxy_delay,
            commands::active_delay,
            commands::traffic,
            commands::connections,
            commands::close_connection,
            commands::test_latency,
            commands::recent_logs,
            commands::tun_status,
            commands::system_proxy_status,
            commands::set_system_proxy,
        ])
        .build(tauri::generate_context!())
        .expect("error while running TriProxy")
        .run(|app, event| {
            // Cleanup is asynchronous, so the exit is held once and released
            // after it completes; the second request falls through the guard.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
                    return;
                }
                api.prevent_exit();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    shutdown(&app).await;
                    app.exit(0);
                });
            }
        });
}

/// Refresh every subscription whose interval has elapsed.
///
/// The tick is one minute because `refresh_due` decides per profile: a longer
/// tick would make a one-hour subscription drift by however much the tick is.
/// The first tick is consumed without doing work so startup stays quiet.
async fn refresh_subscriptions_forever(app: tauri::AppHandle, manager: Arc<CoreManager>) {
    use tauri::Manager;

    let mut ticker = tokio::time::interval(Duration::from_secs(60));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;

    loop {
        ticker.tick().await;

        let store = ProfileStore::new(manager.layout());
        let client = app.state::<AppState>().subscription_client.clone();
        let service = SubscriptionService::with_client(&store, client);

        match service.refresh_due().await {
            Ok(outcomes) if outcomes.is_empty() => {}
            Ok(outcomes) => {
                let updated = outcomes
                    .iter()
                    .filter(|outcome| matches!(outcome.status, RefreshStatus::Updated { .. }))
                    .count();
                let failed = outcomes
                    .iter()
                    .filter(|outcome| matches!(outcome.status, RefreshStatus::Failed { .. }))
                    .count();
                if updated > 0 {
                    let _ = app.emit(PROFILES_CHANGED_EVENT, ());
                }
                tracing::info!(
                    refreshed = outcomes.len(),
                    updated,
                    failed,
                    "subscriptions refreshed in the background"
                );
            }
            // One unreachable provider must not stop the scheduler: the next
            // tick retries it along with everything else that is due.
            Err(error) => tracing::warn!(%error, "background subscription refresh failed"),
        }
    }
}

/// Maintain a single `/traffic` stream while Mihomo or sing-box is active
/// and the main window is visible.
async fn stream_traffic_forever(
    app: tauri::AppHandle,
    manager: Arc<CoreManager>,
    slot: Arc<RwLock<Option<TrafficSample>>>,
    mut visible_rx: tokio::sync::watch::Receiver<bool>,
    traffic_wake: Arc<tokio::sync::Notify>,
) {
    let set_slot = |value: Option<TrafficSample>| {
        if let Ok(mut guard) = slot.write() {
            *guard = value;
        }
    };

    loop {
        if !*visible_rx.borrow_and_update() {
            set_slot(None);
            if visible_rx.wait_for(|&visible| visible).await.is_err() {
                return;
            }
            continue;
        }

        let active = manager.persisted_active().await;
        if !active.is_some_and(|kind| kind.has_controller()) {
            set_slot(None);
            tokio::select! {
                _ = traffic_wake.notified() => {}
                res = visible_rx.wait_for(|&visible| !visible) => {
                    if res.is_err() {
                        return;
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            }
            continue;
        }

        let Ok(client) = manager.controller_client() else {
            tokio::select! {
                _ = traffic_wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
            continue;
        };

        let stream_slot = slot.clone();
        let stream_app = app.clone();
        tokio::select! {
            _ = client.stream_traffic(move |sample| {
                if let Ok(mut guard) = stream_slot.write() {
                    *guard = Some(sample);
                }
                let _ = stream_app.emit(TRAFFIC_EVENT, sample);
            }) => {}
            res = visible_rx.wait_for(|&visible| !visible) => {
                if res.is_err() {
                    return;
                }
            }
            _ = traffic_wake.notified() => {}
        }

        set_slot(None);
        if *visible_rx.borrow() {
            tokio::select! {
                _ = traffic_wake.notified() => {}
                res = visible_rx.wait_for(|&visible| !visible) => {
                    if res.is_err() {
                        return;
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    }
}

/// Start a core on launch when the user asked for it.
///
/// Deliberately routed through the same command the UI uses: the TUN privilege
/// check, profile compilation and system-proxy handoff all live there, and an
/// auto-connect path that skipped them would start a core that behaves
/// differently from one the user started by hand.
async fn maybe_auto_connect_async(app: &tauri::AppHandle) {
    use tauri::Manager;

    let state = app.state::<AppState>();
    if !state.cached_settings().auto_connect {
        return;
    }

    let manager = state.manager.clone();
    if manager.has_running_core().await {
        return;
    }

    let layout = manager.layout().clone();
    let has_active = tokio::task::spawn_blocking(move || {
        ProfileStore::new(&layout)
            .active_id()
            .ok()
            .flatten()
            .is_some()
    })
    .await
    .unwrap_or(false);

    if !has_active {
        tracing::info!("auto-connect skipped: no active profile");
        return;
    }

    match commands::start_core(app.clone(), state, None).await {
        Ok(status) => tracing::info!(
            core = status.active_core.map(|kind| kind.as_str()).unwrap_or("-"),
            "auto-connected on launch"
        ),
        Err(error) => tracing::warn!(%error, "auto-connect failed"),
    }
}

/// Stop every core and restore the system proxy before the process goes away.
/// Without this, quitting from the tray would leave a headless core holding the
/// proxy port with the machine still pointed at it — the user would believe the
/// proxy is off while their traffic kept flowing through it.
async fn shutdown<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;

    let manager = app.state::<AppState>().manager.clone();

    match manager.stop().await {
        Ok(kinds) if !kinds.is_empty() => tracing::info!(cores = kinds.len(), "stopped on exit"),
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "could not stop the core on exit"),
    }

    let proxy = SystemProxyManager::new(manager.layout());
    if proxy.status().is_some() {
        match proxy.disable() {
            Ok(_) => tracing::info!("system proxy restored on exit"),
            Err(error) => tracing::warn!(%error, "could not restore the system proxy on exit"),
        }
    }
}
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,triproxy_core=info,triproxy_app=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

/// Point the core crate at the bundled cores.
///
/// Windows keeps them beside the executable (`<install>\cores`), macOS inside
/// the bundle (`Contents/Resources/cores`), and a Linux `.deb` under
/// `/usr/lib/triproxy/cores`. Only the shell knows which; the core crate cannot
/// derive it from `current_exe()` on the latter two.
fn resolve_bundled_cores<R: tauri::Runtime>(app: &tauri::App<R>) {
    use tauri::Manager;

    let mut candidates = Vec::new();

    if let Ok(resources) = app.path().resource_dir() {
        candidates.push(resources.join("cores"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("cores"));
        }
    }

    for candidate in candidates {
        if candidate.is_dir() {
            tracing::info!(dir = %candidate.display(), "using bundled cores");
            std::env::set_var("TRIPROXY_BUNDLED_CORES", &candidate);
            return;
        }
    }

    tracing::debug!("no bundled cores; the manager will fall back to PATH");
}
