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

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use triproxy_core::{
    compile, measure_nodes, measure_proxy_delay, parse_payload, CoreManager, ProfileStore,
    RefreshStatus, StoredProfile, SubscriptionService, DEFAULT_PROBE_URL,
};

use super::{controller, parse_core, parse_dns_mode, parse_rule_mode};
use crate::fmt::{format_datetime, print_diagnostics, print_profile_nodes, render_table};

/// Flags for `profile compile`. Grouped so call sites cannot swap tun and dns.
pub struct CompileArgs {
    pub output: Option<PathBuf>,
    pub tun: bool,
    pub dns: Option<String>,
    pub rules: Option<String>,
    pub check: bool,
    pub json: bool,
}

pub fn read_profile_source(source: &str) -> Result<(String, String)> {
    if source == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .context("cannot read stdin")?;
        return Ok((buffer, "stdin".to_string()));
    }

    let path = PathBuf::from(source);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    Ok((text, path.display().to_string()))
}

pub fn profile_name_from_source(source: &str) -> String {
    if source == "-" {
        return "stdin".to_string();
    }
    Path::new(source)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("profile")
        .to_string()
}

pub fn cmd_profile_parse(source: &str, json: bool) -> Result<i32> {
    let (text, label) = read_profile_source(source)?;
    let parsed = parse_payload(&text, &profile_name_from_source(source), &label);

    if json {
        let payload = serde_json::json!({
            "profile": parsed.profile,
            "diagnostics": parsed.diagnostics,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(0);
    }

    let profile = &parsed.profile;
    println!("profile   {}  ({})", profile.name, profile.id);
    println!("source    {}", profile.metadata.source);
    println!("format    {}", profile.metadata.format.as_str());
    println!("nodes     {}", profile.nodes.len());
    if !profile.nodes.is_empty() {
        let protocols = profile
            .protocols()
            .iter()
            .map(|protocol| protocol.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("protocols {protocols}");
    }
    println!();

    print_profile_nodes(profile);

    println!(
        "\n{} errors, {} warnings",
        parsed.errors(),
        parsed.warnings()
    );
    print_diagnostics(&parsed.diagnostics);
    Ok(0)
}

pub fn cmd_profile_check(source: &str, json: bool) -> Result<i32> {
    let (text, label) = read_profile_source(source)?;
    let parsed = parse_payload(&text, &profile_name_from_source(source), &label);
    let errors = parsed.errors();
    let warnings = parsed.warnings();
    let code = if errors > 0 { 1 } else { 0 };

    if json {
        let payload = serde_json::json!({
            "profile": parsed.profile.name,
            "id": parsed.profile.id,
            "nodes": parsed.node_count(),
            "errors": errors,
            "warnings": warnings,
            "diagnostics": parsed.diagnostics,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(code);
    }

    println!(
        "{}: {} nodes, {} errors, {} warnings",
        parsed.profile.name,
        parsed.node_count(),
        errors,
        warnings
    );
    print_diagnostics(&parsed.diagnostics);
    Ok(code)
}

/// Read, parse and compile a profile for one core, reporting skipped nodes.
pub async fn cmd_profile_compile(
    manager: &CoreManager,
    source: &str,
    core: &str,
    args: CompileArgs,
) -> Result<i32> {
    let CompileArgs {
        output,
        tun,
        dns,
        rules,
        check,
        json,
    } = args;
    let kind = parse_core(core)?;
    let (text, label) = read_profile_source(source)?;
    let parsed = parse_payload(&text, &profile_name_from_source(source), &label);
    let options = triproxy_core::CompileOptions {
        tun,
        dns: parse_dns_mode(dns.as_deref())?,
        rules: parse_rule_mode(rules.as_deref())?,
        selected_node: None,
        ..Default::default()
    };
    let compiled = compile(kind, &parsed.profile, &options).map_err(|error| anyhow!("{error}"))?;

    if check {
        manager
            .validate_config(kind, &compiled.text, compiled.file_name)
            .await?;
    }

    if json {
        let payload = serde_json::json!({
            "core": kind.as_str(),
            "file": compiled.file_name,
            "included": compiled.included,
            "skipped": compiled.skipped,
            "checked": check,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(0);
    }

    match &output {
        Some(path) => {
            std::fs::write(path, &compiled.text)
                .with_context(|| format!("cannot write {}", path.display()))?;
            println!(
                "wrote {} — {} nodes for {}{}",
                path.display(),
                compiled.included.len(),
                kind,
                if check { ", validated" } else { "" }
            );
            for skipped in &compiled.skipped {
                println!("  skipped {}: {}", skipped.name, skipped.reason);
            }
        }
        None => {
            // Config on stdout so it can be piped; notes on stderr.
            print!("{}", compiled.text);
            for skipped in &compiled.skipped {
                eprintln!("skipped {}: {}", skipped.name, skipped.reason);
            }
        }
    }

    Ok(0)
}

/// Accept either a record id or a profile name, because ids are not memorable.
pub fn resolve_stored(store: &ProfileStore, key: &str) -> Result<StoredProfile> {
    if let Ok(Some(found)) = store.find(key) {
        return Ok(found);
    }
    if let Some(found) = store.find_by_name(key)? {
        return Ok(found);
    }
    bail!("no profile with id or name `{key}`")
}

pub async fn cmd_profile_add(
    manager: &CoreManager,
    source: &str,
    name: Option<String>,
    use_it: bool,
    interval_hours: u64,
) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());

    // A URL is a subscription, not something to read from disk.
    if source.starts_with("http://") || source.starts_with("https://") {
        let service = SubscriptionService::new(&store)?;
        let name = name.unwrap_or_else(|| "subscription".to_string());
        let saved = service
            .add(source, &name, interval_hours.saturating_mul(3600))
            .await?;

        println!(
            "subscribed `{}` — {} nodes ({})",
            saved.name, saved.node_count, saved.id
        );
        if interval_hours > 0 {
            println!("  auto refresh every {interval_hours}h");
        }
        if use_it {
            store.set_active(Some(&saved.id))?;
            println!("  active profile is now `{}`", saved.name);
        }
        return Ok(0);
    }

    let (text, label) = read_profile_source(source)?;
    let name = name.unwrap_or_else(|| profile_name_from_source(source));
    let mut parsed = parse_payload(&text, &name, &label);
    parsed.profile.name = name.clone();

    let existing = store.find_by_name(&name)?;
    let saved = store.save_activating_if_first(
        &parsed.profile,
        existing.as_ref().map(|entry| entry.id.as_str()),
    )?;

    println!(
        "{} `{}` — {} nodes ({})",
        if existing.is_some() {
            "updated"
        } else {
            "added"
        },
        saved.name,
        saved.node_count,
        saved.id
    );
    if parsed.errors() > 0 {
        println!(
            "  {} errors, {} warnings — run `profile check` for details",
            parsed.errors(),
            parsed.warnings()
        );
    }
    if use_it {
        store.set_active(Some(&saved.id))?;
        println!("  active profile is now `{}`", saved.name);
    }
    Ok(0)
}

pub async fn cmd_profile_test(
    manager: &CoreManager,
    target: Option<&str>,
    timeout_secs: u64,
    http: bool,
    probe_url: Option<&str>,
    json: bool,
) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());
    let profile = match target {
        Some(key) => {
            let stored = resolve_stored(&store, key)?;
            store.load(&stored.id)?
        }
        None => store
            .active()?
            .map(|(_, profile)| profile)
            .ok_or_else(|| anyhow!("no active profile; pass a profile name or use profile use"))?,
    };

    if profile.nodes.is_empty() {
        bail!("the profile has no nodes to test");
    }

    let samples = if http {
        let client = controller(manager)?;
        client.version().await.map_err(|error| {
            anyhow!(
                "proxy delay needs a running Mihomo or sing-box with its controller API ({error})"
            )
        })?;
        measure_proxy_delay(
            &client,
            &profile.nodes,
            probe_url.unwrap_or(DEFAULT_PROBE_URL),
            timeout_secs.max(1) * 1000,
        )
        .await
    } else {
        measure_nodes(
            &profile.nodes,
            std::time::Duration::from_secs(timeout_secs.max(1)),
        )
        .await
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&samples)?);
        return Ok(0);
    }

    let latency_header = if http { "PROXY" } else { "TCP" };
    let headers = ["#", "NAME", "ENDPOINT", latency_header, "STATUS"];
    let rows: Vec<Vec<String>> = samples
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            vec![
                (index + 1).to_string(),
                sample.name.clone(),
                sample.endpoint.clone(),
                sample
                    .latency_ms
                    .map(|ms| format!("{ms} ms"))
                    .unwrap_or_else(|| "-".to_string()),
                sample.error.clone().unwrap_or_else(|| "ok".to_string()),
            ]
        })
        .collect();
    print!("{}", render_table(&headers, &rows));

    let reachable = samples.iter().filter(|sample| sample.reachable()).count();
    if http {
        println!(
            "\n{reachable}/{} working · measured by the core through each node",
            samples.len()
        );
    } else {
        println!(
            "\n{reachable}/{} reachable · TCP handshake only, not proof the proxy works",
            samples.len()
        );
    }
    Ok(0)
}

pub async fn cmd_profile_refresh(
    manager: &CoreManager,
    target: Option<&str>,
    all: bool,
    json: bool,
) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());
    let service = SubscriptionService::new(&store)?;

    let outcomes = if all {
        let mut outcomes = Vec::new();
        for record in store.records()? {
            if record.subscription.is_some() {
                outcomes.push(service.refresh(&record.id).await?);
            }
        }
        outcomes
    } else {
        let Some(target) = target else {
            bail!("specify a profile id/name, or pass --all to refresh every subscription");
        };
        let stored = resolve_stored(&store, target)?;
        vec![service.refresh(&stored.id).await?]
    };

    if outcomes.is_empty() {
        println!("no subscriptions to refresh");
        return Ok(0);
    }

    if json {
        let payload = serde_json::json!({
            "outcomes": outcomes.iter().map(|outcome| serde_json::json!({
                "id": outcome.id,
                "name": outcome.name,
                "status": match &outcome.status {
                    RefreshStatus::Updated { node_count } => serde_json::json!({
                        "kind": "updated", "node_count": node_count
                    }),
                    RefreshStatus::NotModified => serde_json::json!({ "kind": "not_modified" }),
                    RefreshStatus::Failed { error } => serde_json::json!({
                        "kind": "failed", "error": error
                    }),
                },
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        for outcome in &outcomes {
            match &outcome.status {
                RefreshStatus::Updated { node_count } => {
                    println!("updated `{}` — {node_count} nodes", outcome.name)
                }
                RefreshStatus::NotModified => {
                    println!("`{}` — unchanged (provider returned 304)", outcome.name)
                }
                RefreshStatus::Failed { error } => {
                    println!("`{}` — refresh failed: {error}", outcome.name);
                    println!("  the stored profile is unchanged");
                }
            }
        }
    }

    let failed = outcomes
        .iter()
        .any(|outcome| matches!(outcome.status, RefreshStatus::Failed { .. }));
    Ok(if failed { 1 } else { 0 })
}

pub fn cmd_profile_list(manager: &CoreManager, json: bool) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());
    let profiles = store.list()?;
    let active = store.active_id()?;

    if json {
        let payload = serde_json::json!({ "active": active, "profiles": profiles });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(0);
    }

    if profiles.is_empty() {
        println!("no stored profiles — import one with `triproxy profile add <file>`");
        return Ok(0);
    }

    let headers = ["ID", "NAME", "NODES", "FORMAT", "UPDATED", "ACTIVE"];
    let rows: Vec<Vec<String>> = profiles
        .iter()
        .map(|stored| {
            vec![
                stored.id.clone(),
                stored.name.clone(),
                stored.node_count.to_string(),
                stored.format.as_str().to_string(),
                format_datetime(stored.updated_at_ms),
                if active.as_deref() == Some(stored.id.as_str()) {
                    "*".to_string()
                } else {
                    String::new()
                },
            ]
        })
        .collect();
    print!("{}", render_table(&headers, &rows));
    Ok(0)
}

pub fn cmd_profile_show(manager: &CoreManager, target: &str, json: bool) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());
    let stored = resolve_stored(&store, target)?;
    let profile = store.load(&stored.id)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&profile)?);
        return Ok(0);
    }

    println!("profile   {}  ({})", profile.name, stored.id);
    println!("source    {}", profile.metadata.source);
    println!("format    {}", profile.metadata.format.as_str());
    println!("nodes     {}", profile.nodes.len());
    println!();
    print_profile_nodes(&profile);
    Ok(0)
}

pub fn cmd_profile_remove(manager: &CoreManager, target: &str) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());
    let stored = resolve_stored(&store, target)?;
    store.delete(&stored.id)?;
    println!("removed `{}` ({})", stored.name, stored.id);
    Ok(0)
}

pub fn cmd_profile_use(manager: &CoreManager, target: &str) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());
    let stored = resolve_stored(&store, target)?;
    store.set_active(Some(&stored.id))?;
    println!(
        "active profile: `{}` ({} nodes)",
        stored.name, stored.node_count
    );
    Ok(0)
}

pub fn cmd_profile_current(manager: &CoreManager, json: bool) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());
    let Some((stored, profile)) = store.active()? else {
        if json {
            println!("null");
        } else {
            println!("no active profile — pick one with `triproxy profile use <id|name>`");
        }
        return Ok(0);
    };

    if json {
        let payload = serde_json::json!({
            "id": stored.id,
            "name": stored.name,
            "profile": profile,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(0);
    }

    println!("active    {}  ({})", profile.name, stored.id);
    println!("source    {}", profile.metadata.source);
    println!("nodes     {}", profile.nodes.len());
    println!();
    print_profile_nodes(&profile);
    Ok(0)
}
