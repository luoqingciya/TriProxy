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

use std::sync::Arc;

use triproxy_core::{CoreManager, Layout, SystemProxyManager};

/// Shared state handed to every command.
pub struct AppState {
    pub manager: Arc<CoreManager>,
}

pub fn run() {
    init_tracing();

    let layout = Layout::discover().expect("cannot resolve the application data directory");
    let manager = Arc::new(CoreManager::new(layout).expect("cannot initialise the core manager"));

    tauri::Builder::default()
        .manage(AppState {
            manager: manager.clone(),
        })
        .setup(move |_app| {
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
            Ok(())
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
        .run(tauri::generate_context!())
        .expect("error while running TriProxy");
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
