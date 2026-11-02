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

pub mod commands;
pub mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use triproxy_core::{AppSettings, CoreManager, Layout, SystemProxyManager};

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
        // Self-update. The plugin is what makes an installed copy able to move
        // to a newer release without the user downloading an installer again.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState {
            manager: manager.clone(),
        })
        .setup(move |app| {
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
            }
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
            commands::preview_selection,
            commands::start_core,
            commands::stop_core,
            commands::switch_core,
            commands::select_node,
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

/// Stop every core and restore the system proxy before the process goes away.
///
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
