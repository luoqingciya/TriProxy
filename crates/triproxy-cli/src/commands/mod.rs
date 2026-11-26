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

pub mod core;
pub mod inspect;
pub mod lifecycle;
pub mod network;
pub mod profile;

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use triproxy_core::{
    compile, tun_unavailable_reason, CompileOptions, CompiledConfig, ControllerClient, CoreKind,
    CoreManager, DnsMode, NetworkRequirements, ParsedProfile, Profile, ProfileStore, RuleMode,
    Selector, SystemProxyManager,
};

pub fn parse_core(value: &str) -> Result<CoreKind> {
    value.parse::<CoreKind>().map_err(|e| anyhow!(e))
}

pub fn parse_dns_mode(value: Option<&str>) -> Result<DnsMode> {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("off") => Ok(DnsMode::Off),
        Some("split") => Ok(DnsMode::Split),
        Some("fakeip") | Some("fake-ip") => Ok(DnsMode::FakeIp),
        Some(other) => bail!("unknown DNS mode `{other}` (expected off, split or fakeip)"),
    }
}

pub fn parse_rule_mode(value: Option<&str>) -> Result<RuleMode> {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("global") => Ok(RuleMode::Global),
        Some("split") => Ok(RuleMode::Split),
        Some(other) => bail!("unknown rule mode `{other}` (expected global or split)"),
    }
}

/// What the user asked for: let the selector decide, or pin a core.
#[derive(Debug, Clone, Copy)]
pub enum CoreChoice {
    Smart,
    Fixed(CoreKind),
}

pub fn parse_core_choice(value: &str) -> Result<CoreChoice> {
    let lowered = value.trim().to_ascii_lowercase();
    if lowered == "smart" || lowered == "auto" {
        return Ok(CoreChoice::Smart);
    }
    Ok(CoreChoice::Fixed(parse_core(value)?))
}

pub fn controller(manager: &CoreManager) -> Result<ControllerClient> {
    manager
        .controller_client()
        .map_err(|error| anyhow!("{error}"))
}

/// The controller client, but only when a core is actually answering on it.
pub async fn running_controller(manager: &CoreManager) -> Option<ControllerClient> {
    let client = manager.controller_client().ok()?;
    client.version().await.ok()?;
    Some(client)
}

/// Refuse to start TUN when a prerequisite is visibly missing.
pub fn check_tun_ready() -> Result<()> {
    if let Some(reason) = tun_unavailable_reason() {
        bail!("{reason}");
    }
    Ok(())
}

/// Enable the system proxy for whichever core is currently listening.
pub fn enable_system_proxy(manager: &CoreManager) -> Result<()> {
    let proxy = SystemProxyManager::new(manager.layout());
    if !proxy.supported() {
        bail!(
            "system proxy is not supported on this platform (backend `{}`)",
            proxy.backend_name()
        );
    }
    let port = manager
        .persisted_http_proxy_port()
        .ok_or_else(|| anyhow!("no core is running"))?;
    let applied = proxy.enable(&format!("127.0.0.1:{port}"))?;
    println!("  system proxy → {}", applied.endpoint);
    Ok(())
}

/// The node the user picked for the active profile, if any.
pub fn selected_node_of(manager: &CoreManager) -> Option<String> {
    let store = ProfileStore::new(manager.layout());
    store
        .active()
        .ok()
        .flatten()
        .and_then(|(stored, _)| stored.selected_node)
}

pub fn load_profile(path: &Path) -> Result<ParsedProfile> {
    let label = path.display().to_string();
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {label}"))?;
    Ok(triproxy_core::parse_payload(
        &text,
        &profile::profile_name_from_source(&label),
        &label,
    ))
}

pub fn compile_for(
    kind: CoreKind,
    profile: &Profile,
    options: &CompileOptions,
) -> Result<CompiledConfig> {
    let compiled = compile(kind, profile, options).map_err(|error| anyhow!("{error}"))?;
    for skipped in &compiled.skipped {
        eprintln!("skipped `{}`: {}", skipped.name, skipped.reason);
    }
    Ok(compiled)
}

pub fn resolve_target(
    manager: &CoreManager,
    choice: CoreChoice,
    profile: Option<&Path>,
    options: &CompileOptions,
    network: NetworkRequirements,
) -> Result<(CoreKind, Option<CompiledConfig>)> {
    if let Some(path) = profile {
        let parsed = load_profile(path)?;
        return finish_target(choice, parsed.profile, options, network);
    }

    let store = ProfileStore::new(manager.layout());
    if let Some((stored, active)) = store.active()? {
        eprintln!(
            "using active profile `{}` ({} nodes)",
            stored.name,
            active.nodes.len()
        );
        return finish_target(choice, active, options, network);
    }

    match choice {
        CoreChoice::Smart => bail!(
            "`--core smart` needs a profile: import one with `triproxy profile add <file>` or pass `--profile`"
        ),
        CoreChoice::Fixed(kind) => Ok((kind, None)),
    }
}

fn finish_target(
    choice: CoreChoice,
    profile: Profile,
    options: &CompileOptions,
    network: NetworkRequirements,
) -> Result<(CoreKind, Option<CompiledConfig>)> {
    match choice {
        CoreChoice::Smart => {
            let selection = Selector::smart()
                .select(&profile, network)
                .map_err(|error| anyhow!("{error}"))?;
            eprintln!(
                "smart: selected `{}` — {}",
                selection.selected, selection.reason
            );
            let compiled = compile_for(selection.selected, &profile, options)?;
            Ok((selection.selected, Some(compiled)))
        }
        CoreChoice::Fixed(kind) => {
            let compiled = compile_for(kind, &profile, options)?;
            Ok((kind, Some(compiled)))
        }
    }
}
