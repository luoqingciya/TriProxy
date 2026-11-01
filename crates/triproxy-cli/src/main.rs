use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use clap::{Parser, Subcommand};
use triproxy_core::binaries::InstallOptions;
use triproxy_core::health::port_is_free;
use triproxy_core::process::run_capture;
use triproxy_core::{CoreKind, CoreManager, CoreReport, Layout, ManagerStatus, SwitchReport};

#[derive(Parser)]
#[command(
    name = "triproxy",
    version,
    about = "TriProxy — one client, three cores (Mihomo / sing-box / Xray)"
)]
struct Cli {
    /// Override the application data directory (default: platform data dir)
    #[arg(long, global = true, env = "TRIPROXY_HOME", value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Emit machine-readable JSON where supported
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the manager status of all three cores
    Status,
    /// List cores and their binary installation state
    Cores,
    /// Start a core (defaults to the persisted active core)
    Start {
        /// mihomo | sing-box | xray
        core: Option<String>,
    },
    /// Stop whatever core is running
    Stop,
    /// Restart the active core
    Restart,
    /// Stop the current core and start another one
    Switch {
        /// mihomo | sing-box | xray
        core: String,
    },
    /// Show captured core logs
    Logs {
        /// Number of trailing lines to show
        #[arg(short = 'n', long, default_value_t = 40)]
        lines: usize,
        /// Only this core
        #[arg(long)]
        core: Option<String>,
        /// Keep streaming new lines
        #[arg(short = 'f', long)]
        follow: bool,
    },
    /// Manage core binaries
    Core {
        #[command(subcommand)]
        command: CoreCommand,
    },
    /// Check the environment: binaries, versions, ports, data directory
    Doctor,
}

#[derive(Subcommand)]
enum CoreCommand {
    /// Download, verify and install a core binary
    Install {
        /// mihomo | sing-box | xray
        core: String,
        /// Release tag or bare version (default: latest)
        #[arg(long)]
        version: Option<String>,
        /// Reinstall even if a matching version is present
        #[arg(long)]
        force: bool,
        /// Accept an asset that publishes no checksum (not recommended)
        #[arg(long)]
        allow_unverified: bool,
        /// Install from a locally downloaded archive instead of the network
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
        /// GitHub release mirror prefix, e.g. https://ghfast.top/
        #[arg(long, value_name = "PREFIX")]
        mirror: Option<String>,
    },
    /// Print the resolved binary path for a core
    Path {
        /// mihomo | sing-box | xray
        core: String,
    },
}

#[tokio::main]
async fn main() {
    init_tracing();
    let cli = Cli::parse();
    let code = match run(cli).await {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err:#}");
            1
        }
    };
    exit_now(code);
}

/// Terminate without tearing down the tokio runtime.
///
/// Starting a core is the whole point of this CLI, and that core is deliberately
/// left running. A normal return would block in runtime teardown, because the
/// process driver keeps a waiter for every child it spawned — so `triproxy start`
/// would never hand the prompt back.
fn exit_now(code: i32) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(code);
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,triproxy_core=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .with_writer(std::io::stderr)
        .try_init();
}

async fn run(cli: Cli) -> Result<()> {
    let layout = match cli.data_dir {
        Some(path) => Layout::new(path),
        None => Layout::discover()?,
    };
    let manager = CoreManager::new(layout)?;
    // Adoption is logged by the core layer; a stderr note here would repeat on
    // every single invocation, because every invocation adopts the running core.
    manager.recover().await?;

    match cli.command {
        Command::Status => cmd_status(&manager, cli.json).await,
        Command::Cores => cmd_cores(&manager, cli.json).await,
        Command::Start { core } => cmd_start(&manager, core.as_deref()).await,
        Command::Stop => cmd_stop(&manager).await,
        Command::Restart => cmd_restart(&manager).await,
        Command::Switch { core } => cmd_switch(&manager, &core).await,
        Command::Logs {
            lines,
            core,
            follow,
        } => cmd_logs(&manager, lines, core.as_deref(), follow).await,
        Command::Core { command } => match command {
            CoreCommand::Install {
                core,
                version,
                force,
                allow_unverified,
                file,
                mirror,
            } => {
                cmd_core_install(&manager, &core, version, force, allow_unverified, file, mirror)
                    .await
            }
            CoreCommand::Path { core } => cmd_core_path(&manager, &core),
        },
        Command::Doctor => cmd_doctor(&manager).await,
    }
}

fn parse_core(value: &str) -> Result<CoreKind> {
    value.parse::<CoreKind>().map_err(|e| anyhow!(e))
}

async fn cmd_status(manager: &CoreManager, json: bool) -> Result<()> {
    let status = manager.status().await;
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }
    print_status(&status);
    Ok(())
}

async fn cmd_cores(manager: &CoreManager, json: bool) -> Result<()> {
    let status = manager.status().await;
    if json {
        println!("{}", serde_json::to_string_pretty(&status.cores)?);
        return Ok(());
    }
    print_core_table(&status.cores);
    Ok(())
}

async fn cmd_start(manager: &CoreManager, core: Option<&str>) -> Result<()> {
    let kind = match core {
        Some(name) => parse_core(name)?,
        None => resolve_default_core(manager).await?,
    };

    let report = manager.start(kind).await?;
    if report.already_running {
        println!("{} is already running", kind);
        return Ok(());
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
    println!("  listen  127.0.0.1:{}", kind.default_listen_port());
    Ok(())
}

async fn cmd_stop(manager: &CoreManager) -> Result<()> {
    let stopped = manager.stop().await?;
    if stopped.is_empty() {
        println!("nothing to stop");
    } else {
        for kind in stopped {
            println!("{kind} stopped");
        }
    }
    Ok(())
}

async fn cmd_restart(manager: &CoreManager) -> Result<()> {
    let report = manager.restart().await?;
    println!(
        "{} restarted{}",
        report.kind,
        report
            .pid
            .map(|pid| format!(" (pid {pid})"))
            .unwrap_or_default()
    );
    Ok(())
}

async fn cmd_switch(manager: &CoreManager, core: &str) -> Result<()> {
    let kind = parse_core(core)?;
    let report = manager.switch(kind).await?;
    print_switch_report(&report);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_core_install(
    manager: &CoreManager,
    core: &str,
    version: Option<String>,
    force: bool,
    allow_unverified: bool,
    file: Option<PathBuf>,
    mirror: Option<String>,
) -> Result<()> {
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
        println!(
            "{} {} is already installed",
            kind, installed.version
        );
    } else {
        println!("{} {} installed", kind, installed.version);
    }
    println!("  tag      {}", installed.tag);
    println!("  path     {}", installed.path.display());
    println!("  sha256   {}", installed.sha256);
    Ok(())
}

fn cmd_core_path(manager: &CoreManager, core: &str) -> Result<()> {
    let kind = parse_core(core)?;
    match manager.binaries().resolve(kind) {
        Some((path, source)) => {
            println!("{}", path.display());
            eprintln!("source: {}", source.as_str());
            Ok(())
        }
        None => {
            bail!(
                "no binary found for `{kind}`; run `triproxy core install {kind}` or set {}",
                kind.bin_env_var()
            )
        }
    }
}

async fn cmd_doctor(manager: &CoreManager) -> Result<()> {
    println!("TriProxy doctor");
    println!();

    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    println!("  platform   {platform}");

    let data_dir = manager.layout().root();
    let writable = probe_writable(data_dir);
    println!(
        "  data dir   {} ({})",
        data_dir.display(),
        if writable { "writable" } else { "NOT writable" }
    );

    println!();
    for kind in CoreKind::ALL {
        match manager.binaries().resolve(kind) {
            Some((path, source)) => {
                let args: Vec<OsString> = kind
                    .version_probe_args()
                    .iter()
                    .map(OsString::from)
                    .collect();
                let probe = run_capture(&path, &args).await;
                let detail = match probe {
                    Ok(output) if output.success => {
                        format!("ok — {}", first_line(&output.combined()))
                    }
                    Ok(output) => format!("FAILED to report version (exit {:?})", output.code),
                    Err(e) => format!("FAILED to execute: {e}"),
                };
                println!(
                    "  {:<9} {:<8} {} — {detail}",
                    kind.as_str(),
                    source.as_str(),
                    path.display()
                );
            }
            None => println!(
                "  {:<9} {:<8} not installed (triproxy core install {kind})",
                kind.as_str(),
                "-"
            ),
        }
    }

    println!();
    for port in [7890u16, 7891, 9090] {
        println!(
            "  port {port:<5} {}",
            if port_is_free(port) { "free" } else { "IN USE" }
        );
    }

    Ok(())
}

fn probe_writable(dir: &Path) -> bool {
    let probe = dir.join(".triproxy-write-probe");
    let ok = std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
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

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

fn print_status(status: &ManagerStatus) {
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

fn print_core_table(cores: &[CoreReport]) {
    let headers = ["CORE", "STATE", "PID", "PORT", "LATENCY", "BINARY"];
    let rows: Vec<Vec<String>> = cores
        .iter()
        .map(|core| {
            vec![
                core.kind.as_str().to_string(),
                core.state.to_string(),
                core.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string()),
                core.listen_port
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                core.latency_ms
                    .map(|ms| format!("{ms} ms"))
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

fn print_switch_report(report: &SwitchReport) {
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

fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
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

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

struct LogEntry {
    ts_ms: u64,
    core: CoreKind,
    level: char,
    text: String,
}

fn read_log_file(path: &Path, core: CoreKind) -> Vec<LogEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let ts_ms = parts.next()?.parse::<u64>().ok()?;
            let level = parts.next()?.chars().next().unwrap_or('I');
            let text = parts.next().unwrap_or("").to_string();
            Some(LogEntry {
                ts_ms,
                core,
                level,
                text,
            })
        })
        .collect()
}

fn format_ts(ts_ms: u64) -> String {
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

fn print_entries(entries: &[LogEntry], show_core: bool) {
    for entry in entries {
        let text = entry.text.trim_end();
        if show_core {
            println!(
                "{}  {:<8} {} {}",
                format_ts(entry.ts_ms),
                entry.core.as_str(),
                entry.level,
                text
            );
        } else {
            println!("{}  {} {}", format_ts(entry.ts_ms), entry.level, text);
        }
    }
}

async fn cmd_logs(
    manager: &CoreManager,
    lines: usize,
    core: Option<&str>,
    follow: bool,
) -> Result<()> {
    let kinds: Vec<CoreKind> = match core {
        Some(name) => vec![parse_core(name)?],
        None => CoreKind::ALL.to_vec(),
    };
    let show_core = kinds.len() > 1;

    let mut entries: Vec<LogEntry> = kinds
        .iter()
        .flat_map(|kind| read_log_file(&manager.layout().core_log_file(kind.as_str()), *kind))
        .collect();

    if entries.is_empty() {
        eprintln!("no logs captured yet — start a core first");
        if !follow {
            return Ok(());
        }
    }

    entries.sort_by_key(|entry| entry.ts_ms);
    let skip = entries.len().saturating_sub(lines);
    print_entries(&entries[skip..], show_core);

    if !follow {
        return Ok(());
    }

    // Follow mode polls the per-core log files. Polling (rather than an in-memory
    // channel) is what makes this work across processes: the writing process is a
    // different CLI invocation.
    let mut offsets: Vec<(CoreKind, PathBuf, u64)> = kinds
        .iter()
        .map(|kind| {
            let path = manager.layout().core_log_file(kind.as_str());
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            (*kind, path, len)
        })
        .collect();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = tokio::time::sleep(Duration::from_millis(300)) => {
                for (kind, path, offset) in offsets.iter_mut() {
                    let Ok(meta) = std::fs::metadata(&*path) else { continue };
                    if meta.len() < *offset {
                        // File was rotated or truncated; start over.
                        *offset = 0;
                    }
                    if meta.len() == *offset {
                        continue;
                    }
                    if let Ok(mut file) = std::fs::File::open(&*path) {
                        if file.seek(SeekFrom::Start(*offset)).is_ok() {
                            let mut chunk = String::new();
                            if file.read_to_string(&mut chunk).is_ok() {
                                *offset = meta.len();
                                for line in chunk.lines() {
                                    if let Some(entry) = parse_log_line(line, *kind) {
                                        print_entries(&[entry], show_core);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn parse_log_line(line: &str, core: CoreKind) -> Option<LogEntry> {
    let mut parts = line.splitn(3, '\t');
    let ts_ms = parts.next()?.parse::<u64>().ok()?;
    let level = parts.next()?.chars().next().unwrap_or('I');
    let text = parts.next().unwrap_or("").to_string();
    Some(LogEntry {
        ts_ms,
        core,
        level,
        text,
    })
}
