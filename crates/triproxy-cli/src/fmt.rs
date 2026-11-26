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

use triproxy_core::{CoreReport, Diagnostic, ManagerStatus, Node, Profile, SwitchReport};

pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if index < widths.len() {
                widths[index] = widths[index].max(cell.chars().count());
            }
        }
    }

    let mut out = String::new();
    let header_line: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(index, header)| format!("{:<width$}", header, width = widths[index]))
        .collect();
    out.push_str("  ");
    out.push_str(header_line.join("  ").trim_end());
    out.push('\n');

    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(index, cell)| format!("{:<width$}", cell, width = widths[index]))
            .collect();
        out.push_str("  ");
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn format_datetime(ts_ms: u64) -> String {
    use time::{OffsetDateTime, UtcOffset};
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    match OffsetDateTime::from_unix_timestamp_nanos(i128::from(ts_ms) * 1_000_000) {
        Ok(datetime) => {
            let local = datetime.to_offset(offset);
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                local.year(),
                u8::from(local.month()),
                local.day(),
                local.hour(),
                local.minute()
            )
        }
        Err(_) => "-".to_string(),
    }
}

pub fn format_ts(ts_ms: u64) -> String {
    use time::{OffsetDateTime, UtcOffset};
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    match OffsetDateTime::from_unix_timestamp_nanos(i128::from(ts_ms) * 1_000_000) {
        Ok(datetime) => {
            let local = datetime.to_offset(offset);
            format!(
                "{:02}:{:02}:{:02}",
                local.hour(),
                local.minute(),
                local.second()
            )
        }
        Err(_) => "--:--:--".to_string(),
    }
}

pub fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_string()
}

pub fn probe_writable(dir: &Path) -> bool {
    let probe = dir.join(".triproxy-write-probe");
    let ok = std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

pub fn print_status(status: &ManagerStatus) {
    println!("TriProxy · core manager");
    println!();
    println!("  data     {}", status.data_dir.display());
    println!(
        "  policy   exclusive — {}",
        if status.exclusive {
            "at most one core runs at a time"
        } else {
            "multiple cores allowed"
        }
    );
    let active = match status.active_core {
        Some(kind) => {
            let pid = status
                .cores
                .iter()
                .find(|c| c.kind == kind)
                .and_then(|c| c.pid)
                .map(|pid| format!("pid {pid}"))
                .unwrap_or_else(|| "pid unknown".to_string());
            format!("{} ({pid})", kind.display_name())
        }
        None => "none".to_string(),
    };
    println!("  active   {active}");
    println!();
    print_core_table(&status.cores);
}

pub fn print_core_table(cores: &[CoreReport]) {
    // "LOOPBACK" rather than "LATENCY": this is the handshake against the local
    // inbound port, which is always under a millisecond. Calling it latency made
    // every running core report `0 ms` and invited the reading "my node is
    // instant" — node latency needs an HTTP request through the core and lives
    // in `profile test --http`.
    let headers = [
        "CORE", "STATE", "PID", "PORT", "LOOPBACK", "MEMORY", "BINARY",
    ];
    let rows: Vec<Vec<String>> = cores
        .iter()
        .map(|core| {
            vec![
                core.kind.as_str().to_string(),
                core.state.to_string(),
                core.pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                core.listen_port
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                core.latency_ms
                    .map(format_loopback)
                    .unwrap_or_else(|| "-".to_string()),
                core.memory_bytes
                    .map(format_bytes)
                    .unwrap_or_else(|| "-".to_string()),
                describe_binary(core),
            ]
        })
        .collect();
    print!("{}", render_table(&headers, &rows));

    for core in cores {
        if let Some(error) = &core.last_error {
            println!();
            println!("  ! {} — {}", core.kind, error);
        }
    }
}

/// Sub-millisecond loopback handshakes are the normal case, so `0 ms` would be
/// technically true and completely useless.
fn format_loopback(ms: u64) -> String {
    if ms == 0 {
        "<1 ms".to_string()
    } else {
        format!("{ms} ms")
    }
}

fn describe_binary(core: &CoreReport) -> String {
    match (&core.binary, &core.binary_version) {
        (Some(_), Some(version)) => format!(
            "{} {}",
            version,
            core.binary_source.as_deref().unwrap_or("unknown")
        ),
        (Some(_), None) => format!(
            "present ({})",
            core.binary_source.as_deref().unwrap_or("unknown")
        ),
        (None, _) => "not installed".to_string(),
    }
}

pub fn print_switch_report(report: &SwitchReport) {
    println!(
        "switch → {} (previous: {})",
        report.target,
        report
            .previous
            .map(|k| k.as_str().to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    println!();
    let width = report
        .steps
        .iter()
        .map(|step| step.step.chars().count())
        .max()
        .unwrap_or(0);
    for (index, step) in report.steps.iter().enumerate() {
        let mark = if step.ok { "ok" } else { "!!" };
        println!(
            "  {mark}  {}. {:<width$}  {}",
            index + 1,
            step.step,
            step.detail,
            width = width
        );
    }
    if let Some(config) = &report.config {
        println!();
        println!("  config  {}", config.display());
    }
}

pub fn security_label(node: &Node) -> String {
    match &node.tls {
        None => "-".to_string(),
        Some(tls) if tls.reality.is_some() => "reality".to_string(),
        Some(_) => "tls".to_string(),
    }
}

pub fn print_diagnostics(diagnostics: &[Diagnostic]) {
    if diagnostics.is_empty() {
        return;
    }
    println!();
    for diagnostic in diagnostics.iter().take(30) {
        let location = match (diagnostic.node, &diagnostic.node_name) {
            (Some(index), Some(name)) => format!("[{:>3} {}]", index + 1, name),
            _ => String::new(),
        };
        println!(
            "  {:<7} {:<26} {} {}",
            diagnostic.severity.as_str(),
            diagnostic.code,
            location,
            diagnostic.message
        );
    }
    if diagnostics.len() > 30 {
        println!("  … {} more", diagnostics.len() - 30);
    }
}

/// Shared node table used by `profile parse`, `show` and `current`.
pub fn print_profile_nodes(profile: &Profile) {
    let headers = ["#", "NAME", "PROTOCOL", "ENDPOINT", "TRANSPORT", "SECURITY"];
    let rows: Vec<Vec<String>> = profile
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            vec![
                (index + 1).to_string(),
                node.name.clone(),
                node.protocol.as_str().to_string(),
                node.endpoint(),
                node.transport.label().to_string(),
                security_label(node),
            ]
        })
        .collect();
    print!("{}", render_table(&headers, &rows));
}
