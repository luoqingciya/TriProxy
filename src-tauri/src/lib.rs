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
use std::sync::Arc;
use std::time::Duration;

use triproxy_core::{
    AppSettings, CoreManager, Layout, ProfileStore, RefreshStatus, SubscriptionService,
    SystemProxyManager,
};

/// Guards the exit path: cleanup is asynchronous, so the app has to be held open
/// once, then allowed through on the second exit request.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Shared state handed to every command.
pub struct AppState {
    pub manager: Arc<CoreManager>,
}

pub fn run() {
    init_tracing();

    let layout = Layout::discover().expect("cannot resolve the application data directory");
    let manager = Arc::new(CoreManager::new(layout).expect("cannot initialise the core manager"));

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

            // Adopt a core left running by a previous session before the window
            // draws, so the first status query reports the truth.
            let manager = manager.clone();
            tauri::async_runtime::block_on(async move {
                match manager.recover().await {
                    Ok(report) => {
                        if let Some(kind) = report.adopted {
                            tracing::info!(core = kind.as_str(), "adopted running core");
                        }
                    }
                    Err(error) => tracing::warn!(%error, "recovery failed"),
                }

                // A proxy snapshot with no running core means the previous
                // session died before restoring it; undo that before drawing.
                let proxy = SystemProxyManager::new(manager.layout());
                if proxy.status().is_some() && manager.status().await.active_core.is_none() {
                    if let Err(error) = proxy.recover() {
                        tracing::warn!(%error, "could not restore the system proxy");
                    }
                }
            });

            // The tray must exist before the window can be closed, otherwise
            // hiding it would leave the user with no way back. A failure is
            // therefore non-fatal: the app simply behaves like a normal window.
            if let Err(error) = tray::install(app) {
                tracing::error!(%error, "could not create the tray icon");
            } else {
                // The menu opens with "未运行"; fill in what recovery found.
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    commands::sync_tray(&handle).await;
                });
            }

            // Subscriptions used to refresh only when the user asked. A profile
            // that quietly ages out is worse than one that never updates, so the
            // scheduler runs for as long as the app does — including while it
            // sits in the tray, which is now most of the time.
            let scheduler = app.state::<AppState>().manager.clone();
            tauri::async_runtime::spawn(async move {
                refresh_subscriptions_forever(scheduler).await;
            });

            // Start the core without being asked, if the user wants that.
            maybe_auto_connect(app.handle());
            Ok(())
        })
        // Closing the window hides it; the proxy is supposed to keep running.
        // Only an explicit quit (tray menu) or `close_to_tray = false` exits.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                use tauri::Manager;
                let app = window.app_handle();
                let settings = AppSettings::load(app.state::<AppState>().manager.layout());
                if settings.close_to_tray && tray::is_ready() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::profiles,
            commands::active_nodes,
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
async fn refresh_subscriptions_forever(manager: Arc<CoreManager>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(60));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;

    loop {
        ticker.tick().await;

        let store = ProfileStore::new(manager.layout());
        let service = match SubscriptionService::new(&store) {
            Ok(service) => service,
            Err(error) => {
                // A client that cannot be built will not build later either;
                // keeping the loop alive would just log the same line forever.
                tracing::warn!(%error, "subscription scheduler stopped");
                return;
            }
        };

        match service.refresh_due().await {
            Ok(outcomes) if outcomes.is_empty() => {}
            Ok(outcomes) => {
                let failed = outcomes
                    .iter()
                    .filter(|outcome| matches!(outcome.status, RefreshStatus::Failed { .. }))
                    .count();
                tracing::info!(
                    refreshed = outcomes.len(),
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

/// Start a core on launch when the user asked for it.
///
/// Deliberately routed through the same command the UI uses: the TUN privilege
/// check, profile compilation and system-proxy handoff all live there, and an
/// auto-connect path that skipped them would start a core that behaves
/// differently from one the user started by hand.
fn maybe_auto_connect(app: &tauri::AppHandle) {
    use tauri::Manager;

    let settings = AppSettings::load(app.state::<AppState>().manager.layout());
    if !settings.auto_connect {
        return;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let manager = app.state::<AppState>().manager.clone();

        if manager.status().await.active_core.is_some() {
            return;
        }
        if ProfileStore::new(manager.layout())
            .active()
            .ok()
            .flatten()
            .is_none()
        {
            tracing::info!("auto-connect skipped: no active profile");
            return;
        }

        match commands::start_core(app.clone(), app.state::<AppState>(), None).await {
            Ok(status) => tracing::info!(
                core = status.active_core.map(|kind| kind.as_str()).unwrap_or("-"),
                "auto-connected on launch"
            ),
            Err(error) => tracing::warn!(%error, "auto-connect failed"),
        }
    });
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
