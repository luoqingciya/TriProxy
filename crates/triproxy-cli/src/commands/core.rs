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

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use triproxy_core::binaries::{InstallOptions, UpdateStatus};
use triproxy_core::{CoreKind, CoreManager, NetworkRequirements, Selector};

use super::parse_core;
use super::profile::read_profile_source;
use crate::fmt::render_table;

pub async fn cmd_core_install(
    manager: &CoreManager,
    core: &str,
    version: Option<String>,
    force: bool,
    allow_unverified: bool,
    file: Option<PathBuf>,
    mirror: Option<String>,
) -> Result<i32> {
    let kind = parse_core(core)?;
    let options = InstallOptions {
        version,
        force,
        allow_unverified,
        local_file: file,
        mirror,
    };
    let installed = manager.install(kind, &options).await?;

    if installed.already_installed {
        println!("{} {} is already installed", kind, installed.version);
    } else {
        println!("{} {} installed", kind, installed.version);
    }
    println!("  tag      {}", installed.tag);
    println!("  path     {}", installed.path.display());
    println!("  sha256   {}", installed.sha256);
    Ok(0)
}

pub async fn cmd_core_update(
    manager: &CoreManager,
    core: Option<&str>,
    all: bool,
    check_only: bool,
    json: bool,
) -> Result<i32> {
    let kinds: Vec<CoreKind> = if all {
        CoreKind::ALL.to_vec()
    } else {
        let name = core.ok_or_else(|| anyhow!("specify a core, or pass --all"))?;
        vec![parse_core(name)?]
    };

    let mut results = Vec::new();
    for kind in kinds {
        match manager.binaries().check_update(kind).await {
            Ok(outcome) => results.push(outcome),
            Err(error) => {
                // A network failure must not be reported as "up to date".
                eprintln!("{kind}: cannot check for updates — {error}");
            }
        }
    }

    if json {
        let payload = serde_json::json!({
            "checks": results.iter().map(|outcome| serde_json::json!({
                "core": outcome.core.as_str(),
                "installed": outcome.installed,
                "latest": outcome.latest,
                "status": outcome.status.as_str(),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(0);
    }

    for outcome in &results {
        let installed = outcome.installed.as_deref().unwrap_or("not installed");
        let latest = outcome.latest.as_deref().unwrap_or("?");
        match outcome.status {
            UpdateStatus::UpToDate => {
                println!("{:<9} up to date ({installed})", outcome.core.as_str())
            }
            UpdateStatus::NotInstalled => println!(
                "{:<9} not installed — latest is {latest}, run `core install`",
                outcome.core.as_str()
            ),
            UpdateStatus::UpdateAvailable => {
                println!("{:<9} {installed} → {latest}", outcome.core.as_str());
                if !check_only {
                    let options = InstallOptions {
                        version: Some(latest.to_string()),
                        force: true,
                        ..Default::default()
                    };
                    match manager.binaries().install(outcome.core, &options).await {
                        Ok(binary) => println!("          installed {}", binary.version),
                        Err(error) => eprintln!("          install failed: {error}"),
                    }
                }
            }
        }
    }

    Ok(0)
}

pub fn cmd_core_path(manager: &CoreManager, core: &str) -> Result<i32> {
    let kind = parse_core(core)?;
    match manager.binaries().resolve(kind) {
        Some((path, source)) => {
            println!("{}", path.display());
            eprintln!("source: {}", source.as_str());
            Ok(0)
        }
        None => {
            bail!(
                "no binary found for `{kind}`; run `triproxy core install {kind}` or set {}",
                kind.bin_env_var()
            )
        }
    }
}

pub fn cmd_core_select(source: &str, tun: bool, json: bool) -> Result<i32> {
    let (text, label) = read_profile_source(source)?;
    let parsed = triproxy_core::parse_payload(
        &text,
        &super::profile::profile_name_from_source(source),
        &label,
    );
    let network = NetworkRequirements {
        tun,
        ..Default::default()
    };

    let selection = match Selector::smart().select(&parsed.profile, network) {
        Ok(selection) => selection,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(1);
        }
    };

    if json {
        let payload = serde_json::json!({
            "profile": parsed.profile.name,
            "nodes": parsed.profile.nodes.len(),
            "network": { "tun": network.tun },
            "selected": selection.selected.as_str(),
            "overridden": selection.overridden,
            "reason": selection.reason,
            "evaluations": selection.evaluations,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(0);
    }

    let requirement = Selector::requirement(&parsed.profile, network);
    let protocols = requirement
        .protocols
        .iter()
        .map(|protocol| protocol.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut needs = Vec::new();
    if requirement.requires_reality {
        needs.push("reality");
    }
    if requirement.requires_vision {
        needs.push("vision");
    }
    if requirement.requires_tun {
        needs.push("tun");
    }
    if requirement.requires_rule_set {
        needs.push("rule-set");
    }
    if requirement.requires_proxy_groups {
        needs.push("proxy-groups");
    }

    println!(
        "profile   {}  ({} nodes)",
        parsed.profile.name,
        parsed.profile.nodes.len()
    );
    println!("protocols {protocols}");
    if !needs.is_empty() {
        println!("needs     {}", needs.join(", "));
    }
    println!();

    let headers = ["CORE", "VERDICT", "DETAIL"];
    let rows: Vec<Vec<String>> = selection
        .evaluations
        .iter()
        .map(|evaluation| {
            let verdict = if evaluation.core == selection.selected {
                "selected"
            } else if evaluation.eligible {
                "eligible"
            } else {
                "ineligible"
            };
            let detail = if evaluation.eligible {
                if evaluation.native_features.is_empty() {
                    "-".to_string()
                } else {
                    format!("native {}", evaluation.native_features.join(" + "))
                }
            } else {
                format!("missing {}", evaluation.unmet.join(", "))
            };
            vec![
                evaluation.core.as_str().to_string(),
                verdict.to_string(),
                detail,
            ]
        })
        .collect();
    print!("{}", render_table(&headers, &rows));

    println!("\nReason: {}", selection.reason);
    Ok(0)
}
