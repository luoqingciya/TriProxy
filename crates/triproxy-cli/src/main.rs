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

mod commands;
mod fmt;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use triproxy_core::{CoreManager, Layout, SystemProxyManager};

use commands::core::{cmd_core_install, cmd_core_path, cmd_core_select, cmd_core_update};
use commands::inspect::{cmd_connections, cmd_delay, cmd_doctor, cmd_logs, cmd_node, cmd_traffic};
use commands::lifecycle::{cmd_cores, cmd_restart, cmd_start, cmd_status, cmd_stop, cmd_switch};
use commands::network::{cmd_proxy_off, cmd_proxy_on, cmd_proxy_status, cmd_tun_status};
use commands::profile::{
    cmd_profile_add, cmd_profile_check, cmd_profile_compile, cmd_profile_current, cmd_profile_list,
    cmd_profile_parse, cmd_profile_refresh, cmd_profile_remove, cmd_profile_show, cmd_profile_test,
    cmd_profile_use, CompileArgs,
};

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
    /// Measure the delay of the node the proxy is using right now
    Delay {
        /// Timeout in seconds
        #[arg(long, value_name = "SECS", default_value_t = 3)]
        timeout: u64,
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
pub enum NodeCommand {
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
    manager.recover().await?;

    let system_proxy = SystemProxyManager::new(manager.layout());
    if system_proxy.status().is_some() && !manager.has_running_core().await {
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
        Command::Delay { timeout, json } => cmd_delay(&manager, timeout, json).await,
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
