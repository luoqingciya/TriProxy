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

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use triproxy_core::RuleMode;

use crate::{commands, AppState};

/// Window label from `tauri.conf.json` (the implicit default).
const MAIN_WINDOW: &str = "main";

/// Id of the tray icon, used to find it again when the menu changes.
const TRAY_ID: &str = "triproxy";

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

pub struct TrayState {
    pub status: String,
    pub running: bool,
    pub rule_mode: RuleMode,
    pub system_proxy: bool,
    pub tun: bool,
}

impl Default for TrayState {
    fn default() -> Self {
        Self {
            status: "未运行".to_string(),
            running: false,
            rule_mode: RuleMode::Split,
            system_proxy: false,
            tun: false,
        }
    }
}

pub fn is_ready() -> bool {
    TRAY_READY.load(Ordering::Relaxed)
}

/// Build the FlClash-style tray menu with live core status, mode switcher, and network toggles.
fn build_menu(
    manager: &impl Manager<tauri::Wry>,
    state: &TrayState,
) -> tauri::Result<Menu<tauri::Wry>> {
    let status_item = MenuItem::with_id(manager, "status", &state.status, false, None::<&str>)?;
    let show = MenuItem::with_id(manager, "show", "显示主窗口", true, None::<&str>)?;
    let toggle_core_label = if state.running {
        "停止核心"
    } else {
        "启动核心"
    };
    let toggle = MenuItem::with_id(
        manager,
        "toggle-core",
        toggle_core_label,
        true,
        None::<&str>,
    )?;

    let mode_label = |mode: RuleMode, name: &str| {
        if state.rule_mode == mode {
            format!("● {name}")
        } else {
            format!("○ {name}")
        }
    };
    let mode_rule = MenuItem::with_id(
        manager,
        "mode-rule",
        mode_label(RuleMode::Split, "规则模式 (Rule)"),
        true,
        None::<&str>,
    )?;
    let mode_global = MenuItem::with_id(
        manager,
        "mode-global",
        mode_label(RuleMode::Global, "全局模式 (Global)"),
        true,
        None::<&str>,
    )?;
    let mode_direct = MenuItem::with_id(
        manager,
        "mode-direct",
        mode_label(RuleMode::Direct, "直连模式 (Direct)"),
        true,
        None::<&str>,
    )?;

    let proxy_label = if state.system_proxy {
        "✓ 系统代理"
    } else {
        "   系统代理"
    };
    let tun_label = if state.tun {
        "✓ TUN 虚拟网卡"
    } else {
        "   TUN 虚拟网卡"
    };
    let toggle_proxy = MenuItem::with_id(manager, "toggle-proxy", proxy_label, true, None::<&str>)?;
    let toggle_tun = MenuItem::with_id(manager, "toggle-tun", tun_label, true, None::<&str>)?;
    let quit = MenuItem::with_id(manager, "quit", "退出 TriProxy", true, None::<&str>)?;

    Menu::with_items(
        manager,
        &[
            &status_item,
            &PredefinedMenuItem::separator(manager)?,
            &show,
            &toggle,
            &PredefinedMenuItem::separator(manager)?,
            &mode_rule,
            &mode_global,
            &mode_direct,
            &PredefinedMenuItem::separator(manager)?,
            &toggle_proxy,
            &toggle_tun,
            &PredefinedMenuItem::separator(manager)?,
            &quit,
        ],
    )
}

/// Replace the menu with one that reflects `state`.
pub fn sync(app: &AppHandle, state: &TrayState) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    match build_menu(app, state) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(error) => tracing::warn!(%error, "could not update the tray menu"),
    }
}

pub fn install(app: &tauri::App) -> tauri::Result<()> {
    let menu = build_menu(app, &TrayState::default())?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("TriProxy")
        .menu(&menu)
        // Left click is "show/hide the window"; the menu is right-click only.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let app = app.clone();
            match event.id().as_ref() {
                "show" => show_window(&app),
                "toggle-core" => {
                    tauri::async_runtime::spawn(async move { toggle_core(app).await });
                }
                "mode-rule" => {
                    tauri::async_runtime::spawn(async move {
                        switch_rule_mode(app, RuleMode::Split).await
                    });
                }
                "mode-global" => {
                    tauri::async_runtime::spawn(async move {
                        switch_rule_mode(app, RuleMode::Global).await
                    });
                }
                "mode-direct" => {
                    tauri::async_runtime::spawn(async move {
                        switch_rule_mode(app, RuleMode::Direct).await
                    });
                }
                "toggle-proxy" => {
                    tauri::async_runtime::spawn(async move { toggle_system_proxy(app).await });
                }
                "toggle-tun" => {
                    tauri::async_runtime::spawn(async move { toggle_tun(app).await });
                }
                "quit" => app.exit(0),
                _ => {}
            }
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
pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        app.state::<AppState>().set_window_visible(true);
    }
}

/// Left-click behaviour: focus the window if it is already in front, otherwise
/// bring it back. Clicking a visible-but-behind window should raise it, not hide
/// it, which is why focus — not visibility — decides.
fn toggle_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let in_front = window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false);
    if in_front {
        let _ = window.hide();
        app.state::<AppState>().set_window_visible(false);
    } else {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        app.state::<AppState>().set_window_visible(true);
    }
}

async fn run_tray_action<F>(app: &AppHandle, action: F)
where
    F: Future<Output = Result<String, String>>,
{
    match action.await {
        Ok(message) => {
            let _ = app.emit(NOTICE_EVENT, message);
        }
        Err(error) => {
            tracing::warn!(%error, "tray action failed");
            let _ = app.emit(NOTICE_EVENT, error);
            // The banner lives in the window, so a failure has to be visible.
            show_window(app);
        }
    }
}

/// Start or stop the core without opening the window.
///
/// This reuses the Tauri commands rather than reimplementing the logic: the
/// profile compilation, the TUN privilege check and the system-proxy handoff all
/// live there, and a tray path that drifted from the UI path would be a bug
/// factory.
async fn toggle_core(app: AppHandle) {
    let running = {
        let state = app.state::<AppState>();
        state.manager.has_running_core().await
    };

    run_tray_action(&app, async {
        if running {
            commands::stop_core(app.clone(), app.state::<AppState>())
                .await
                .map(|()| "核心已停止，系统代理已还原".to_string())
        } else {
            commands::start_core(app.clone(), app.state::<AppState>(), None)
                .await
                .map(|status| match status.active_core {
                    Some(kind) => format!("{} 已启动", kind.display_name()),
                    None => "核心已启动".to_string(),
                })
        }
    })
    .await;
}

async fn switch_rule_mode(app: AppHandle, mode: RuleMode) {
    let label = match mode {
        RuleMode::Split => "规则分流模式",
        RuleMode::Global => "全局代理模式",
        RuleMode::Direct => "直接连接模式",
    };
    run_tray_action(&app, async {
        commands::set_rule_mode(app.clone(), app.state::<AppState>(), mode)
            .await
            .map(|_| format!("已切换到{label}"))
    })
    .await;
}

async fn toggle_system_proxy(app: AppHandle) {
    let next = app.state::<AppState>().cached_system_proxy().is_none();
    run_tray_action(&app, async {
        commands::set_system_proxy(app.clone(), app.state::<AppState>(), next)
            .await
            .map(|applied| {
                if applied.is_some() {
                    "系统代理已开启".to_string()
                } else {
                    "系统代理已关闭".to_string()
                }
            })
    })
    .await;
}

async fn toggle_tun(app: AppHandle) {
    let next = !app.state::<AppState>().cached_settings().tun;
    run_tray_action(&app, async {
        commands::set_tun(app.clone(), app.state::<AppState>(), next)
            .await
            .map(|_| {
                if next {
                    "TUN 虚拟网卡已开启".to_string()
                } else {
                    "TUN 虚拟网卡已关闭".to_string()
                }
            })
    })
    .await;
}
