// TriProxy — a modern multi-core proxy client.
// Copyright (C) 2026 luoqingci
// SPDX-License-Identifier: GPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. It is distributed in the hope that it will be useful, but WITHOUT ANY
// WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR
// A PARTICULAR PURPOSE. See LICENSE for the full text.
//
// Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.

//! System tray.
//!
//! A proxy client is expected to keep working with its window closed — that is
//! the point of running one. The tray is therefore the application's real home:
//! the window is a panel you open when you need it, and closing the window only
//! hides it. Quitting goes through the tray menu, which stops the core and
//! restores the system proxy first (see `shutdown` in `lib.rs`).

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::{commands, AppState};

/// Window label from `tauri.conf.json` (the implicit default).
const MAIN_WINDOW: &str = "main";

/// Whether a tray icon actually exists.
///
/// Creating one can fail — a Linux session without a StatusNotifier host is the
/// usual case. Closing the window may only hide it when there is a tray to come
/// back from; otherwise the user would be left with a running process and no UI.
static TRAY_READY: AtomicBool = AtomicBool::new(false);

/// Event the tray uses to report the result of an action to the UI.
///
/// Tray clicks happen in Rust with no window in front of the user, so a failure
/// would otherwise be invisible. The frontend listens for this and shows the
/// same banner it uses for its own errors.
pub const NOTICE_EVENT: &str = "triproxy-notice";

pub fn is_ready() -> bool {
    TRAY_READY.load(Ordering::Relaxed)
}

pub fn install<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle-core", "启动 / 停止核心", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 TriProxy", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &toggle,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("triproxy")
        .tooltip("TriProxy")
        .menu(&menu)
        // Left click is "show/hide the window"; the menu is right-click only.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_window(app),
            "toggle-core" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { toggle_core(app).await });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    TRAY_READY.store(true, Ordering::Relaxed);
    Ok(())
}

/// Show and focus the main window.
pub fn show_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Left-click behaviour: focus the window if it is already in front, otherwise
/// bring it back. Clicking a visible-but-behind window should raise it, not hide
/// it, which is why focus — not visibility — decides.
fn toggle_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let in_front = window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false);
    if in_front {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Start or stop the core without opening the window.
///
/// This reuses the Tauri commands rather than reimplementing the logic: the
/// profile compilation, the TUN privilege check and the system-proxy handoff all
/// live there, and a tray path that drifted from the UI path would be a bug
/// factory.
async fn toggle_core<R: Runtime>(app: AppHandle<R>) {
    let running = {
        let state = app.state::<AppState>();
        state.manager.status().await.active_core.is_some()
    };

    let result = if running {
        commands::stop_core(app.state::<AppState>())
            .await
            .map(|()| "核心已停止，系统代理已还原".to_string())
    } else {
        commands::start_core(app.state::<AppState>(), None)
            .await
            .map(|status| match status.active_core {
                Some(kind) => format!("{} 已启动", kind.display_name()),
                None => "核心已启动".to_string(),
            })
    };

    match result {
        Ok(message) => {
            let _ = app.emit(NOTICE_EVENT, message);
        }
        Err(error) => {
            tracing::warn!(%error, "tray action failed");
            let _ = app.emit(NOTICE_EVENT, error);
            // The banner lives in the window, so a failure has to be visible.
            show_window(&app);
        }
    }
}
