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

use std::path::Path;

use anyhow::{bail, Result};
use triproxy_core::binaries::MIRROR_ENV;
use triproxy_core::{
    AppSettings, CompileOptions, CoreKind, CoreManager, NetworkRequirements, SystemProxyManager,
};

use super::{
    check_tun_ready, enable_system_proxy, parse_core_choice, parse_dns_mode, parse_rule_mode,
    resolve_target, selected_node_of, CoreChoice,
};
use crate::fmt::{print_core_table, print_status, print_switch_report};

pub async fn cmd_status(manager: &CoreManager, json: bool) -> Result<i32> {
    let status = manager.status().await;
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(0);
    }
    print_status(&status);
    Ok(0)
}

pub async fn cmd_cores(manager: &CoreManager, json: bool) -> Result<i32> {
    let status = manager.status().await;
    if json {
        println!("{}", serde_json::to_string_pretty(&status.cores)?);
        return Ok(0);
    }
    print_core_table(&status.cores);
    Ok(0)
}

fn build_compile_options(
    manager: &CoreManager,
    saved: &AppSettings,
    profile: Option<&Path>,
    tun: bool,
    dns: Option<&str>,
    rules: Option<&str>,
) -> Result<(CompileOptions, NetworkRequirements)> {
    let mut options = saved.compile_options(saved.listen_port, saved.api_port);
    options.tun = tun || saved.tun;
    if dns.is_some() {
        options.dns = parse_dns_mode(dns)?;
    }
    if rules.is_some() {
        options.rules = parse_rule_mode(rules)?;
    }
    options.selected_node = if profile.is_none() {
        selected_node_of(manager)
    } else {
        None
    };
    options.api_secret = Some(manager.ensure_api_secret());
    options.rule_set_mirror = std::env::var(MIRROR_ENV).ok();

    let mut network = saved.network();
    network.tun = options.tun;
    Ok((options, network))
}

pub async fn cmd_start(
    manager: &CoreManager,
    core: Option<&str>,
    profile: Option<&Path>,
    proxy: bool,
    tun: bool,
    dns: Option<String>,
    rules: Option<String>,
) -> Result<i32> {
    let saved = AppSettings::load(manager.layout());
    let (options, network) = build_compile_options(
        manager,
        &saved,
        profile,
        tun,
        dns.as_deref(),
        rules.as_deref(),
    )?;

    let choice = match core {
        Some(name) => parse_core_choice(name)?,
        None => CoreChoice::Fixed(resolve_default_core(manager).await?),
    };
    let (kind, compiled) = resolve_target(manager, choice, profile, &options, network)?;

    if options.tun {
        check_tun_ready(manager, kind)?;
    }

    let report = match compiled {
        Some(compiled) => {
            manager
                .start_with_config(kind, &compiled.text, compiled.file_name)
                .await?
        }
        None => manager.start(kind).await?,
    };

    if report.already_running {
        println!("{} is already running", kind);
        return Ok(0);
    }

    println!(
        "{} started{}",
        kind,
        report
            .pid
            .map(|pid| format!(" (pid {pid})"))
            .unwrap_or_default()
    );
    println!("  config  {}", report.config.display());
    println!("  listen  127.0.0.1:{}", saved.listen_port);

    let enable_proxy = proxy || saved.system_proxy;
    if enable_proxy && options.tun {
        eprintln!("note: TUN 模式已接管全部流量，跳过系统代理");
    } else if enable_proxy {
        enable_system_proxy(manager)?;
    }
    Ok(0)
}

pub async fn cmd_stop(manager: &CoreManager) -> Result<i32> {
    let stopped = manager.stop().await?;
    if stopped.is_empty() {
        println!("nothing to stop");
    } else {
        for kind in stopped {
            println!("{kind} stopped");
        }
    }

    // The proxy is useless without a core, and leaving it on would break the
    // machine's connectivity. Restore before returning.
    if SystemProxyManager::new(manager.layout()).disable()? {
        println!("system proxy restored");
    }
    Ok(0)
}

pub async fn cmd_restart(manager: &CoreManager) -> Result<i32> {
    let report = manager.restart().await?;
    println!(
        "{} restarted{}",
        report.kind,
        report
            .pid
            .map(|pid| format!(" (pid {pid})"))
            .unwrap_or_default()
    );
    Ok(0)
}

pub async fn cmd_switch(
    manager: &CoreManager,
    core: &str,
    profile: Option<&Path>,
    proxy: bool,
    tun: bool,
    dns: Option<String>,
    rules: Option<String>,
) -> Result<i32> {
    let saved = AppSettings::load(manager.layout());
    let (options, network) = build_compile_options(
        manager,
        &saved,
        profile,
        tun,
        dns.as_deref(),
        rules.as_deref(),
    )?;

    let choice = parse_core_choice(core)?;
    let (kind, compiled) = resolve_target(manager, choice, profile, &options, network)?;

    if options.tun {
        check_tun_ready(manager, kind)?;
    }

    let report = match compiled {
        Some(compiled) => {
            manager
                .switch_with_config(kind, &compiled.text, compiled.file_name)
                .await?
        }
        None => manager.switch(kind).await?,
    };
    print_switch_report(&report);

    let enable_proxy = proxy || saved.system_proxy;
    if enable_proxy && options.tun {
        eprintln!("note: TUN 模式已接管全部流量，跳过系统代理");
    } else if enable_proxy {
        enable_system_proxy(manager)?;
    }
    Ok(0)
}

async fn resolve_default_core(manager: &CoreManager) -> Result<CoreKind> {
    if let Some(active) = manager.persisted_active().await {
        return Ok(active);
    }
    let installed: Vec<CoreKind> = manager
        .status()
        .await
        .cores
        .iter()
        .filter(|c| c.binary.is_some())
        .map(|c| c.kind)
        .collect();
    match installed.len() {
        0 => bail!("no core binary installed; run `triproxy core install <mihomo|sing-box|xray>`"),
        1 => Ok(installed[0]),
        _ => {
            let names = installed
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("multiple cores installed ({names}); specify one: triproxy start <core>")
        }
    }
}
