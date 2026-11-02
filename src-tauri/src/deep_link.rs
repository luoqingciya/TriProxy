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

//! `triproxy://` links.
//!
//! A subscription arrives as a link far more often than as a file: it is in a
//! chat message, on a provider's page, behind a copy button. Pasting it into a
//! text box is the part people get wrong — they paste the URL into the name
//! field, or lose the token in the middle of a line break.
//!
//! ```text
//! triproxy://install?url=https%3A%2F%2Fexample.com%2Fsub%3Ftoken%3Dabc
//! ```
//!
//! The same handler runs for a link clicked in a browser (the OS starts us with
//! the URL as an argument) and for a link clicked while we are already running
//! (the single-instance plugin forwards the argument to the live process). Both
//! paths land here, so there is one behaviour to reason about, not two.

use tauri::{AppHandle, Emitter, Manager};

use triproxy_core::{ProfileStore, SubscriptionService};

use crate::AppState;

/// Scheme registered in `tauri.conf.json` and with the operating system.
pub const SCHEME: &str = "triproxy";

/// Default refresh interval for a subscription added by link.
///
/// One hour is what most providers publish anyway; the scheduler in `lib.rs`
/// refreshes whatever is due, so this is a starting point, not a policy.
const DEFAULT_INTERVAL_SECS: u64 = 3600;

/// Handle one command-line argument or opened URL.
///
/// Returns true when the argument was ours, so the caller can stop looking.
pub fn handle(app: &AppHandle, candidate: &str) -> bool {
    if !candidate.starts_with(&format!("{SCHEME}://")) {
        return false;
    }

    let Ok(url) = tauri::Url::parse(candidate) else {
        return true;
    };

    match url.host_str().unwrap_or_default() {
        "install" => {
            let target = url
                .query_pairs()
                .find(|(key, _)| key == "url")
                .map(|(_, value)| value.into_owned());

            match target {
                Some(target) if target.starts_with("http") => install(app, target),
                _ => notify(app, "链接里没有可用的订阅地址（需要 ?url=）"),
            }
        }
        other => notify(
            app,
            format!("不认识的链接类型 `{other}`；目前只支持 {SCHEME}://install?url=…"),
        ),
    }

    true
}

/// Import a subscription URL in the background.
///
/// Deliberately not awaited by the caller: the OS expects the handler to return
/// promptly, and the fetch can take as long as the provider takes.
fn install(app: &AppHandle, url: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let store = ProfileStore::new(app.state::<AppState>().manager.layout());
        let service = match SubscriptionService::new(&store) {
            Ok(service) => service,
            Err(error) => {
                notify(&app, format!("无法建立订阅客户端：{error}"));
                return;
            }
        };

        let name = subscription_name(&url);
        match service.add(&url, &name, DEFAULT_INTERVAL_SECS).await {
            Ok(stored) => {
                // First profile wins the "active" slot, same rule as the import
                // dialog: an empty library has nothing better to show.
                if store.active().ok().flatten().is_none() {
                    let _ = store.set_active(Some(&stored.id));
                }
                notify(
                    &app,
                    format!("已导入订阅「{}」，共 {} 个节点", stored.name, stored.node_count),
                );
            }
            Err(error) => notify(&app, format!("导入失败：{error}")),
        }

        crate::tray::show_window(&app);
    });
}

/// A readable name for a subscription we only know by URL.
///
/// The host is the one part of a subscription URL that is both stable and
/// meaningful; the path is usually a token.
fn subscription_name(url: &str) -> String {
    tauri::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| "订阅".to_string())
}

fn notify(app: &AppHandle, message: impl Into<String>) {
    let _ = app.emit(crate::tray::NOTICE_EVENT, message.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_arguments_that_are_not_links() {
        // Windows launches us with paths and flags; only one kind of argument is
        // ours, and treating the others as errors would spam the user.
        assert!(!handle_prefix_check("C:\\Program Files\\TriProxy\\TriProxy.exe"));
        assert!(!handle_prefix_check("--minimized"));
        assert!(handle_prefix_check("triproxy://install?url=https%3A%2F%2Fa.example"));
    }

    /// The prefix test is the only part that can run without a Tauri app, so it
    /// is extracted to keep it testable.
    fn handle_prefix_check(candidate: &str) -> bool {
        candidate.starts_with(&format!("{SCHEME}://"))
    }

    #[test]
    fn names_a_subscription_after_its_host() {
        assert_eq!(
            subscription_name("https://sub.example.com/api/v1/client/subscribe?token=abc"),
            "sub.example.com"
        );
        // A URL with no host still needs a name; the import must not fail over it.
        assert_eq!(subscription_name("not a url"), "订阅");
    }
}
