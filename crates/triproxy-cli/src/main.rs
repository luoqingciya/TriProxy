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

use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use triproxy_core::binaries::{InstallOptions, UpdateStatus};
use triproxy_core::health::port_is_free;
use triproxy_core::process::run_capture;
use triproxy_core::{
    compile, is_elevated, leftovers, measure_nodes, measure_proxy_delay, parse_payload,
    AppSettings, CompileOptions, CompiledConfig, CoreKind, CoreManager, CoreReport, Diagnostic,
    DnsMode, Layout, ManagerStatus, NetworkRequirements, Node, ParsedProfile, Profile,
    ProfileStore, RefreshStatus, RuleMode, Selector, StoredProfile, SubscriptionService,
    SwitchReport, SystemProxyManager, DEFAULT_API_PORT, DEFAULT_PROBE_URL,
};
use triproxy_core::{select_node, ApplyOutcome, ControllerClient};

#[derive(Parser)]
#[command(
    name = "triproxy",
    version,
    long_version = concat!(
        env!("CARGO_PKG_VERSION"),
        "\nCopyright (C) 2026 luoqingci\n",
        "License GPLv3+: GNU GPL version 3 or later <https://gnu.org/licenses/gpl.html>.\n",
        "This is free software: you are free to change and redistribute it.\n",
        "There is NO WARRANTY, to the extent permitted by law.\n\n",
        "Additional terms under GPLv3 section 7 apply; see ADDITIONAL-TERMS.md."
    ),
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
        /// Compile this profile for the core instead of using the bootstrap config
        #[arg(long, value_name = "PATH")]
        profile: Option<PathBuf>,
        /// Also point the system proxy at the core once it is running
        #[arg(long)]
        proxy: bool,
        /// Run in TUN mode (requires administrator privileges)
        #[arg(long)]
        tun: bool,
        /// DNS strategy: off | split | fakeip
        #[arg(long, value_name = "MODE")]
        dns: Option<String>,
        /// Routing strategy: global | split
        #[arg(long, value_name = "MODE")]
        rules: Option<String>,
    },
    /// Stop whatever core is running
    Stop,
    /// Restart the active core
    Restart,
    /// Stop the current core and start another one
    Switch {
        /// mihomo | sing-box | xray
        core: String,
        /// Compile this profile for the core instead of using the bootstrap config
        #[arg(long, value_name = "PATH")]
        profile: Option<PathBuf>,
        /// Also point the system proxy at the core once it is running
        #[arg(long)]
        proxy: bool,
        /// Run in TUN mode (requires administrator privileges)
        #[arg(long)]
        tun: bool,
        /// DNS strategy: off | split | fakeip
        #[arg(long, value_name = "MODE")]
        dns: Option<String>,
        /// Routing strategy: global | split
        #[arg(long, value_name = "MODE")]
        rules: Option<String>,
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
    /// List live connections from the running core (Mihomo / sing-box only)
    Connections {
        /// Close this connection id
        #[arg(long, value_name = "ID")]
        close: Option<String>,
        /// Close every connection
        #[arg(long = "close-all")]
        close_all: bool,
        /// Print as JSON
        #[arg(long)]
        json: bool,
    },
    /// Read one traffic sample from the running core
    Traffic {
        /// Print as JSON
        #[arg(long)]
        json: bool,
    },
    /// Choose which node the proxy uses
    Node {
        #[command(subcommand)]
        command: NodeCommand,
    },
    /// Inspect TUN mode prerequisites and leftovers
    Tun {
        #[command(subcommand)]
        command: TunCommand,
    },
    /// Manage the system proxy
    Proxy {
        #[command(subcommand)]
        command: ProxyCommand,
    },
    /// Inspect a profile (Phase 2: URI lists and base64 subscriptions)
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Check the environment: binaries, versions, ports, data directory
    Doctor,
}

#[derive(Subcommand)]
enum TunCommand {
    /// Show whether TUN can run here and whether an adapter was left behind
    Status,
}

#[derive(Subcommand)]
enum ProxyCommand {
    /// Point the system proxy at a core's local inbound
    On {
        /// Endpoint to use (default: the running core's port on 127.0.0.1)
        #[arg(long)]
        endpoint: Option<String>,
    },
    /// Restore the proxy settings captured before TriProxy changed them
    Off,
    /// Show whether TriProxy has the system proxy enabled
    Status,
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// Parse a profile and list its nodes
    Parse {
        /// File path, or `-` to read stdin
        source: String,
        /// Print the parsed profile as JSON
        #[arg(long)]
        json: bool,
    },
    /// Compile a profile into a core-native config
    Compile {
        /// File path, or `-` to read stdin
        source: String,
        /// Target core: mihomo | sing-box | xray
        #[arg(long)]
        core: String,
        /// Write the config to this file instead of stdout
        #[arg(short = 'o', long, value_name = "PATH")]
        output: Option<PathBuf>,
        /// Generate a TUN config instead of a local mixed port
        #[arg(long)]
        tun: bool,
        /// DNS strategy: off | split | fakeip
        #[arg(long, value_name = "MODE")]
        dns: Option<String>,
        /// Routing strategy: global | split
        #[arg(long, value_name = "MODE")]
        rules: Option<String>,
        /// Let the core validate the generated config before returning
        #[arg(long)]
        check: bool,
        /// Print a summary as JSON instead of the config
        #[arg(long)]
        json: bool,
    },
    /// Validate a profile and report problems; exits 1 when any node is unusable
    Check {
        /// File path, or `-` to read stdin
        source: String,
        /// Print the diagnostics as JSON
        #[arg(long)]
        json: bool,
    },
    /// Import a profile into the local library
    Add {
        /// File path, or `-` to read stdin
        source: String,
        /// Display name (defaults to the file name)
        #[arg(long)]
        name: Option<String>,
        /// Make it the active profile
        #[arg(long = "use")]
        use_it: bool,
        /// Auto-refresh interval in hours for a URL subscription (0 = never)
        #[arg(long, value_name = "HOURS", default_value_t = 24)]
        interval: u64,
    },
    /// List stored profiles
    List {
        /// Print the listing as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show a stored profile's nodes
    Show {
        /// Profile id or name
        target: String,
        /// Print the profile as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete a stored profile
    Remove {
        /// Profile id or name
        target: String,
    },
    /// Mark a stored profile as active
    Use {
        /// Profile id or name
        target: String,
    },
    /// Measure TCP latency for the nodes of a profile
    Test {
        /// Profile id or name (defaults to the active profile)
        target: Option<String>,
        /// Per-node timeout in seconds
        #[arg(long, default_value_t = 3)]
        timeout: u64,
        /// Measure real proxy delay through the running core instead of a TCP
        /// handshake (needs Mihomo or sing-box; Xray has no controller API)
        #[arg(long)]
        http: bool,
        /// URL the core requests when measuring proxy delay
        #[arg(long, value_name = "URL")]
        url: Option<String>,
        /// Print the results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Refresh URL subscriptions
    Refresh {
        /// Profile id or name; omit with --all
        target: Option<String>,
        /// Refresh every subscription, due or not
        #[arg(long)]
        all: bool,
        /// Print the results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show the active profile
    Current {
        /// Print the profile as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum NodeCommand {
    /// Use a node (by name, or id prefix) from the active profile
    Use { target: String },
    /// Show the current selection
    Current,
    /// Clear the selection and fall back to the profile order
    Clear,
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
    /// Check for newer core releases and install them
    Update {
        /// mihomo | sing-box | xray; omit with --all
        core: Option<String>,
        /// Check every core
        #[arg(long)]
        all: bool,
        /// Only report, do not install
        #[arg(long)]
        check: bool,
        /// Print the results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Print the resolved binary path for a core
    Path {
        /// mihomo | sing-box | xray
        core: String,
    },
    /// Evaluate which core should serve a profile (Smart Engine)
    Select {
        /// File path, or `-` to read stdin
        source: String,
        /// Treat TUN as required (eliminates cores without native TUN)
        #[arg(long)]
        tun: bool,
        /// Print the evaluation as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Switch the Windows console to UTF-8.
///
/// Rust writes UTF-8 bytes to stdout, but a Chinese-locale console defaults to
/// code page 936 and renders them as mojibake. The CLI prints Chinese status
/// text, so without this the output is unreadable on exactly the machines this
/// project targets.
#[cfg(windows)]
fn use_utf8_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleOutputCP(code_page: u32) -> i32;
    }
    // SAFETY: one integer argument, no pointers, no lifetime concerns.
    unsafe {
        SetConsoleOutputCP(65001);
    }
}

#[cfg(not(windows))]
fn use_utf8_console() {}

#[tokio::main]
async fn main() {
    use_utf8_console();
    init_tracing();
    let cli = Cli::parse();
    let code = match run(cli).await {
        Ok(code) => code,
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

async fn run(cli: Cli) -> Result<i32> {
    let layout = match cli.data_dir {
        Some(path) => Layout::new(path),
        None => Layout::discover()?,
    };
    let manager = CoreManager::new(layout)?;
    // Adoption is logged by the core layer; a stderr note here would repeat on
    // every single invocation, because every invocation adopts the running core.
    manager.recover().await?;

    // A leftover proxy snapshot with no running core means a previous session
    // died before restoring it. Undo that before doing anything else — leaving
    // the machine pointing at a dead port is the worst outcome of a crash.
    let system_proxy = SystemProxyManager::new(manager.layout());
    if system_proxy.status().is_some() && manager.status().await.active_core.is_none() {
        if let Some(applied) = system_proxy.recover()? {
            eprintln!(
                "note: restored the system proxy left behind by a previous session ({})",
                applied.endpoint
            );
        }
    }

    match cli.command {
        Command::Status => cmd_status(&manager, cli.json).await,
        Command::Cores => cmd_cores(&manager, cli.json).await,
        Command::Start {
            core,
            profile,
            proxy,
            tun,
            dns,
            rules,
        } => {
            cmd_start(
                &manager,
                core.as_deref(),
                profile.as_deref(),
                proxy,
                tun,
                dns,
                rules,
            )
            .await
        }
        Command::Stop => cmd_stop(&manager).await,
        Command::Restart => cmd_restart(&manager).await,
        Command::Switch {
            core,
            profile,
            proxy,
            tun,
            dns,
            rules,
        } => cmd_switch(&manager, &core, profile.as_deref(), proxy, tun, dns, rules).await,
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
                cmd_core_install(
                    &manager,
                    &core,
                    version,
                    force,
                    allow_unverified,
                    file,
                    mirror,
                )
                .await
            }
            CoreCommand::Update {
                core,
                all,
                check,
                json,
            } => cmd_core_update(&manager, core.as_deref(), all, check, json).await,
            CoreCommand::Path { core } => cmd_core_path(&manager, &core),
            CoreCommand::Select { source, tun, json } => cmd_core_select(&source, tun, json),
        },
        Command::Connections {
            close,
            close_all,
            json,
        } => cmd_connections(&manager, close.as_deref(), close_all, json).await,
        Command::Traffic { json } => cmd_traffic(&manager, json).await,
        Command::Node { command } => cmd_node(&manager, command).await,
        Command::Tun { command } => match command {
            TunCommand::Status => cmd_tun_status(&manager),
        },
        Command::Proxy { command } => match command {
            ProxyCommand::On { endpoint } => cmd_proxy_on(&manager, endpoint.as_deref()).await,
            ProxyCommand::Off => cmd_proxy_off(&manager),
            ProxyCommand::Status => cmd_proxy_status(&manager),
        },
        Command::Profile { command } => match command {
            ProfileCommand::Parse { source, json } => cmd_profile_parse(&source, json),
            ProfileCommand::Compile {
                source,
                core,
                output,
                tun,
                dns,
                rules,
                check,
                json,
            } => {
                let args = CompileArgs {
                    output,
                    tun,
                    dns,
                    rules,
                    check,
                    json,
                };
                cmd_profile_compile(&manager, &source, &core, args).await
            }
            ProfileCommand::Check { source, json } => cmd_profile_check(&source, json),
            ProfileCommand::Add {
                source,
                name,
                use_it,
                interval,
            } => cmd_profile_add(&manager, &source, name, use_it, interval).await,
            ProfileCommand::List { json } => cmd_profile_list(&manager, json),
            ProfileCommand::Show { target, json } => cmd_profile_show(&manager, &target, json),
            ProfileCommand::Remove { target } => cmd_profile_remove(&manager, &target),
            ProfileCommand::Use { target } => cmd_profile_use(&manager, &target),
            ProfileCommand::Test {
                target,
                timeout,
                http,
                url,
                json,
            } => {
                cmd_profile_test(
                    &manager,
                    target.as_deref(),
                    timeout,
                    http,
                    url.as_deref(),
                    json,
                )
                .await
            }
            ProfileCommand::Refresh { target, all, json } => {
                cmd_profile_refresh(&manager, target.as_deref(), all, json).await
            }
            ProfileCommand::Current { json } => cmd_profile_current(&manager, json),
        },
        Command::Doctor => cmd_doctor(&manager).await,
    }
}

fn parse_core(value: &str) -> Result<CoreKind> {
    value.parse::<CoreKind>().map_err(|e| anyhow!(e))
}

async fn cmd_status(manager: &CoreManager, json: bool) -> Result<i32> {
    let status = manager.status().await;
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(0);
    }
    print_status(&status);
    Ok(0)
}

async fn cmd_cores(manager: &CoreManager, json: bool) -> Result<i32> {
    let status = manager.status().await;
    if json {
        println!("{}", serde_json::to_string_pretty(&status.cores)?);
        return Ok(0);
    }
    print_core_table(&status.cores);
    Ok(0)
}

async fn cmd_start(
    manager: &CoreManager,
    core: Option<&str>,
    profile: Option<&Path>,
    proxy: bool,
    tun: bool,
    dns: Option<String>,
    rules: Option<String>,
) -> Result<i32> {
    if tun && !is_elevated() {
        bail!("TUN 模式需要管理员权限：请以管理员身份重新运行终端");
    }

    let choice = match core {
        Some(name) => parse_core_choice(name)?,
        None => CoreChoice::Fixed(resolve_default_core(manager).await?),
    };
    // Start from the saved settings so the port, custom rules and any other
    // compile-time knob mean the same thing here as they do in the GUI; the
    // flags below only override what this command was told to change.
    let saved = AppSettings::load(manager.layout());
    let mut options = saved.compile_options(saved.listen_port, DEFAULT_API_PORT);
    options.tun = tun;
    options.dns = parse_dns_mode(dns.as_deref())?;
    options.rules = parse_rule_mode(rules.as_deref())?;
    // Only when running from the stored active profile: an explicit --profile
    // path is a different profile and its selection is unknown.
    options.selected_node = if profile.is_none() {
        selected_node_of(manager)
    } else {
        None
    };
    let options = options;
    let (kind, compiled) = resolve_target(manager, choice, profile, &options)?;

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
    println!(
        "  listen  127.0.0.1:{}",
        AppSettings::load(manager.layout()).listen_port
    );
    if proxy && tun {
        eprintln!("note: TUN 模式已接管全部流量，跳过系统代理");
    } else if proxy {
        enable_system_proxy(manager)?;
    }
    Ok(0)
}

async fn cmd_stop(manager: &CoreManager) -> Result<i32> {
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

async fn cmd_restart(manager: &CoreManager) -> Result<i32> {
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

async fn cmd_switch(
    manager: &CoreManager,
    core: &str,
    profile: Option<&Path>,
    proxy: bool,
    tun: bool,
    dns: Option<String>,
    rules: Option<String>,
) -> Result<i32> {
    if tun && !is_elevated() {
        bail!("TUN 模式需要管理员权限：请以管理员身份重新运行终端");
    }

    let choice = parse_core_choice(core)?;
    // Start from the saved settings so the port, custom rules and any other
    // compile-time knob mean the same thing here as they do in the GUI; the
    // flags below only override what this command was told to change.
    let saved = AppSettings::load(manager.layout());
    let mut options = saved.compile_options(saved.listen_port, DEFAULT_API_PORT);
    options.tun = tun;
    options.dns = parse_dns_mode(dns.as_deref())?;
    options.rules = parse_rule_mode(rules.as_deref())?;
    // Only when running from the stored active profile: an explicit --profile
    // path is a different profile and its selection is unknown.
    options.selected_node = if profile.is_none() {
        selected_node_of(manager)
    } else {
        None
    };
    let options = options;
    let (kind, compiled) = resolve_target(manager, choice, profile, &options)?;

    let report = match compiled {
        Some(compiled) => {
            manager
                .switch_with_config(kind, &compiled.text, compiled.file_name)
                .await?
        }
        None => manager.switch(kind).await?,
    };
    print_switch_report(&report);
    if proxy && tun {
        eprintln!("note: TUN 模式已接管全部流量，跳过系统代理");
    } else if proxy {
        enable_system_proxy(manager)?;
    }
    Ok(0)
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

async fn cmd_core_update(
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

fn cmd_core_path(manager: &CoreManager, core: &str) -> Result<i32> {
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

// ---------------------------------------------------------------------------
// profile (Phase 2)
// ---------------------------------------------------------------------------

fn read_profile_source(source: &str) -> Result<(String, String)> {
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

fn profile_name_from_source(source: &str) -> String {
    if source == "-" {
        return "stdin".to_string();
    }
    Path::new(source)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("profile")
        .to_string()
}

fn security_label(node: &Node) -> String {
    match &node.tls {
        None => "-".to_string(),
        Some(tls) if tls.reality.is_some() => "reality".to_string(),
        Some(_) => "tls".to_string(),
    }
}

fn print_diagnostics(diagnostics: &[Diagnostic]) {
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
fn print_profile_nodes(profile: &Profile) {
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

fn cmd_profile_parse(source: &str, json: bool) -> Result<i32> {
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

fn cmd_profile_check(source: &str, json: bool) -> Result<i32> {
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
async fn cmd_profile_compile(
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
    let options = CompileOptions {
        tun,
        dns: parse_dns_mode(dns.as_deref())?,
        rules: parse_rule_mode(rules.as_deref())?,
        // Compiling a file has no stored selection to apply; use `node use` for
        // the active profile.
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

/// Read and parse a profile from disk.
fn load_profile(path: &Path) -> Result<ParsedProfile> {
    let label = path.display().to_string();
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {label}"))?;
    Ok(parse_payload(
        &text,
        &profile_name_from_source(&label),
        &label,
    ))
}

/// Compile for one core, reporting skipped nodes on stderr.
fn compile_for(
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

/// What the user asked for: let the selector decide, or pin a core.
#[derive(Debug, Clone, Copy)]
enum CoreChoice {
    Smart,
    Fixed(CoreKind),
}

fn parse_dns_mode(value: Option<&str>) -> Result<DnsMode> {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("off") => Ok(DnsMode::Off),
        Some("split") => Ok(DnsMode::Split),
        Some("fakeip") | Some("fake-ip") => Ok(DnsMode::FakeIp),
        Some(other) => bail!("unknown DNS mode `{other}` (expected off, split or fakeip)"),
    }
}

fn parse_rule_mode(value: Option<&str>) -> Result<RuleMode> {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("global") => Ok(RuleMode::Global),
        Some("split") => Ok(RuleMode::Split),
        Some(other) => bail!("unknown rule mode `{other}` (expected global or split)"),
    }
}

fn parse_core_choice(value: &str) -> Result<CoreChoice> {
    let lowered = value.trim().to_ascii_lowercase();
    if lowered == "smart" || lowered == "auto" {
        return Ok(CoreChoice::Smart);
    }
    Ok(CoreChoice::Fixed(parse_core(value)?))
}

/// Resolve the core to run and, when a profile is available, its compiled config.
///
/// Precedence: explicit `--profile` file → stored active profile → bootstrap
/// config (Phase 1 behaviour, kept for the case where no profile exists yet).
fn resolve_target(
    manager: &CoreManager,
    choice: CoreChoice,
    profile: Option<&Path>,
    options: &CompileOptions,
) -> Result<(CoreKind, Option<CompiledConfig>)> {
    if let Some(path) = profile {
        let parsed = load_profile(path)?;
        return finish_target(choice, parsed.profile, options);
    }

    let store = ProfileStore::new(manager.layout());
    if let Some((stored, active)) = store.active()? {
        eprintln!(
            "using active profile `{}` ({} nodes)",
            stored.name,
            active.nodes.len()
        );
        return finish_target(choice, active, options);
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
) -> Result<(CoreKind, Option<CompiledConfig>)> {
    match choice {
        CoreChoice::Smart => {
            let selection = Selector::smart()
                .select(&profile, NetworkRequirements::default())
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

/// Flags for `profile compile`. Grouped so call sites cannot swap tun and dns.
struct CompileArgs {
    output: Option<PathBuf>,
    tun: bool,
    dns: Option<String>,
    rules: Option<String>,
    check: bool,
    json: bool,
}

fn cmd_core_select(source: &str, tun: bool, json: bool) -> Result<i32> {
    let (text, label) = read_profile_source(source)?;
    let parsed = parse_payload(&text, &profile_name_from_source(source), &label);
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

// ---------------------------------------------------------------------------
// profile library (storage)
// ---------------------------------------------------------------------------

fn format_datetime(ts_ms: u64) -> String {
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

/// Accept either a record id or a profile name, because ids are not memorable.
fn resolve_stored(store: &ProfileStore, key: &str) -> Result<StoredProfile> {
    if let Some(found) = store.find(key)? {
        return Ok(found);
    }
    if let Some(found) = store.find_by_name(key)? {
        return Ok(found);
    }
    bail!("no profile with id or name `{key}`")
}

async fn cmd_profile_add(
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

    // Re-importing under an existing name updates that record instead of
    // creating a second entry with the same title.
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

async fn cmd_profile_test(
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
        // Fail early with one useful message instead of N identical errors.
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
            "
{reachable}/{} working · measured by the core through each node",
            samples.len()
        );
    } else {
        println!(
            "
{reachable}/{} reachable · TCP handshake only, not proof the proxy works",
            samples.len()
        );
    }
    Ok(0)
}

async fn cmd_profile_refresh(
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

    // A failed refresh is worth a non-zero exit code so scripts can notice,
    // even though the library is intact.
    let failed = outcomes
        .iter()
        .any(|outcome| matches!(outcome.status, RefreshStatus::Failed { .. }));
    Ok(if failed { 1 } else { 0 })
}

fn cmd_profile_list(manager: &CoreManager, json: bool) -> Result<i32> {
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

fn cmd_profile_show(manager: &CoreManager, target: &str, json: bool) -> Result<i32> {
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

fn cmd_profile_remove(manager: &CoreManager, target: &str) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());
    let stored = resolve_stored(&store, target)?;
    store.delete(&stored.id)?;
    println!("removed `{}` ({})", stored.name, stored.id);
    Ok(0)
}

fn cmd_profile_use(manager: &CoreManager, target: &str) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());
    let stored = resolve_stored(&store, target)?;
    store.set_active(Some(&stored.id))?;
    println!(
        "active profile: `{}` ({} nodes)",
        stored.name, stored.node_count
    );
    Ok(0)
}

fn cmd_profile_current(manager: &CoreManager, json: bool) -> Result<i32> {
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

// ---------------------------------------------------------------------------
// system proxy (Phase 6)
// ---------------------------------------------------------------------------

/// Enable the system proxy for whichever core is currently listening.
fn enable_system_proxy(manager: &CoreManager) -> Result<()> {
    let proxy = SystemProxyManager::new(manager.layout());
    if !proxy.supported() {
        bail!(
            "system proxy is not supported on this platform (backend `{}`)",
            proxy.backend_name()
        );
    }
    let port = running_port(manager).ok_or_else(|| anyhow!("no core is running"))?;
    let applied = proxy.enable(&format!("127.0.0.1:{port}"))?;
    println!("  system proxy → {}", applied.endpoint);
    Ok(())
}

fn running_port(manager: &CoreManager) -> Option<u16> {
    // `status()` is async; the CLI calls this from a sync helper, so use the
    // persisted listen port recorded when the core was started.
    manager.persisted_listen_port()
}

/// The node the user picked for the active profile, if any.
fn selected_node_of(manager: &CoreManager) -> Option<String> {
    let store = ProfileStore::new(manager.layout());
    store
        .active()
        .ok()
        .flatten()
        .and_then(|(stored, _)| stored.selected_node)
}

fn controller(manager: &CoreManager) -> Result<ControllerClient> {
    ControllerClient::new(manager.api_port()).map_err(|error| anyhow!("{error}"))
}

async fn cmd_connections(
    manager: &CoreManager,
    close: Option<&str>,
    close_all: bool,
    json: bool,
) -> Result<i32> {
    let client = controller(manager)?;

    if close_all {
        client.close_all().await?;
        println!("all connections closed");
        return Ok(0);
    }
    if let Some(id) = close {
        client.close_connection(id).await?;
        println!("closed {id}");
        return Ok(0);
    }

    let snapshot = client.connections().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(0);
    }

    if snapshot.connections.is_empty() {
        println!("no live connections");
        return Ok(0);
    }

    let headers = ["ID", "TARGET", "NET", "CHAIN", "UP", "DOWN"];
    let rows: Vec<Vec<String>> = snapshot
        .connections
        .iter()
        .map(|connection| {
            vec![
                connection.id.chars().take(8).collect::<String>(),
                connection.target(),
                connection.metadata.network.clone(),
                connection.chains.join(" ← "),
                format_bytes(connection.upload),
                format_bytes(connection.download),
            ]
        })
        .collect();
    print!("{}", render_table(&headers, &rows));
    println!(
        "
{} live · total up {} / down {}",
        snapshot.connections.len(),
        format_bytes(snapshot.upload_total),
        format_bytes(snapshot.download_total)
    );
    Ok(0)
}

async fn cmd_traffic(manager: &CoreManager, json: bool) -> Result<i32> {
    let client = controller(manager)?;
    let sample = client.traffic_once().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&sample)?);
        return Ok(0);
    }

    println!(
        "↑ {} /s    ↓ {} /s",
        format_bytes(sample.up),
        format_bytes(sample.down)
    );
    Ok(0)
}

fn format_bytes(bytes: u64) -> String {
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

async fn cmd_node(manager: &CoreManager, command: NodeCommand) -> Result<i32> {
    let store = ProfileStore::new(manager.layout());

    let Some((stored, profile)) = store.active()? else {
        bail!("no active profile; choose one with profile use first");
    };

    match command {
        NodeCommand::Current => {
            match stored.selected_node.as_deref() {
                Some(id) => {
                    let name = profile
                        .nodes
                        .iter()
                        .find(|node| node.id == id)
                        .map(|node| node.name.clone())
                        .unwrap_or_else(|| "(missing)".to_string());
                    println!("{name}");
                }
                None => println!("(none — the profile order decides)"),
            }
            Ok(0)
        }
        NodeCommand::Clear => {
            let client = running_controller(manager).await;
            let outcome = select_node(&store, client.as_ref(), None).await?;
            match outcome {
                ApplyOutcome::Applied { .. } => println!("selection cleared and applied"),
                ApplyOutcome::SavedOnly { reason, .. } => {
                    println!("selection cleared; takes effect on next start ({reason})")
                }
            }
            Ok(0)
        }
        NodeCommand::Use { target } => {
            let node = profile
                .nodes
                .iter()
                .find(|node| node.name.eq_ignore_ascii_case(&target))
                .or_else(|| {
                    profile
                        .nodes
                        .iter()
                        .find(|node| node.id.starts_with(&target))
                })
                .ok_or_else(|| {
                    anyhow!("no node matching {target} in the active profile; run profile show to list them")
                })?;

            let client = running_controller(manager).await;
            let outcome = select_node(&store, client.as_ref(), Some(&node.id)).await?;
            match outcome {
                ApplyOutcome::Applied { node } => {
                    println!("using {node} (applied to the running core)")
                }
                ApplyOutcome::SavedOnly { node, reason } => {
                    let name = node.unwrap_or_default();
                    println!("using {name}");
                    println!("  saved; takes effect on next start ({reason})");
                }
            }
            Ok(0)
        }
    }
}

/// The controller client, but only when a core is actually answering on it.
async fn running_controller(manager: &CoreManager) -> Option<ControllerClient> {
    let client = ControllerClient::new(manager.api_port()).ok()?;
    client.version().await.ok()?;
    Some(client)
}

fn cmd_tun_status(manager: &CoreManager) -> Result<i32> {
    println!("TUN 模式");
    if is_elevated() {
        println!("  权限      管理员");
    } else {
        println!("  权限      普通用户 — TUN 需要以管理员身份运行");
    }

    match manager.persisted_listen_port() {
        Some(port) => println!("  当前核心  127.0.0.1:{port}"),
        None => println!("  当前核心  未运行"),
    }

    let found = leftovers()?;
    if found.is_empty() {
        println!("  残留网卡  无");
    } else {
        println!("  残留网卡  {} 个（可能是上次异常退出留下的）", found.len());
        for adapter in &found {
            println!("    {}  [{}]", adapter.name, adapter.id);
            println!("      {}", adapter.cleanup_hint());
        }
    }
    Ok(0)
}

async fn cmd_proxy_on(manager: &CoreManager, endpoint: Option<&str>) -> Result<i32> {
    let proxy = SystemProxyManager::new(manager.layout());
    if !proxy.supported() {
        bail!(
            "system proxy is not supported on this platform (backend `{}`)",
            proxy.backend_name()
        );
    }

    let endpoint = match endpoint {
        Some(endpoint) => endpoint.to_string(),
        None => {
            let port = running_port(manager).ok_or_else(|| {
                anyhow!("no core is running; pass --endpoint or start a core first")
            })?;
            format!("127.0.0.1:{port}")
        }
    };

    let applied = proxy.enable(&endpoint)?;
    println!("system proxy enabled → {}", applied.endpoint);
    println!("  backend  {}", applied.backend);
    println!("  previous settings saved; `triproxy proxy off` restores them");
    Ok(0)
}

fn cmd_proxy_off(manager: &CoreManager) -> Result<i32> {
    let proxy = SystemProxyManager::new(manager.layout());
    if proxy.disable()? {
        println!("system proxy restored");
    } else {
        println!("system proxy was not enabled by TriProxy");
    }
    Ok(0)
}

fn cmd_proxy_status(manager: &CoreManager) -> Result<i32> {
    let proxy = SystemProxyManager::new(manager.layout());
    match proxy.status() {
        Some(applied) => {
            println!("system proxy  enabled → {}", applied.endpoint);
            println!("  backend      {}", applied.backend);
            println!("  applied at   {}", format_datetime(applied.applied_at_ms));
        }
        None => println!("system proxy  not managed by TriProxy"),
    }
    Ok(0)
}

async fn cmd_doctor(manager: &CoreManager) -> Result<i32> {
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

    Ok(0)
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
                core.pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string()),
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
    text.lines().next().unwrap_or("").trim().to_string()
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
) -> Result<i32> {
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
            return Ok(0);
        }
    }

    entries.sort_by_key(|entry| entry.ts_ms);
    let skip = entries.len().saturating_sub(lines);
    print_entries(&entries[skip..], show_core);

    if !follow {
        return Ok(0);
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

    Ok(0)
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
