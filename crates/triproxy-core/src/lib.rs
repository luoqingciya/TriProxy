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

//! TriProxy core layer.
//!
//! The crate is deliberately UI-agnostic: it knows how to locate, verify,
//! supervise and switch between Mihomo, sing-box and Xray, and nothing about how
//! that is presented. The CLI (`triproxy-cli`) and the future Tauri backend are
//! both thin consumers of this API.
//!
//! Layering (plan ch.4):
//!
//! ```text
//! CoreManager  ── exclusivity, switch flow, crash recovery, persistence
//!      │
//! ProxyCore    ── unified trait: version / check / start / stop / health
//!      │
//! CoreDialect  ── per-core arguments, version parsing, config rendering
//!      │
//! Supervisor   ── process lifecycle, readiness probe, log capture
//!      │
//! Mihomo / sing-box / Xray binaries
//! ```

pub mod adapters;
pub mod binaries;
pub mod bootstrap;
pub mod capability;
pub mod compiler;
pub mod controller;
pub mod core;
pub mod error;
pub mod health;
pub mod latency;
pub mod logging;
pub mod memory;
pub mod network;
pub mod paths;
pub mod platform;
pub mod process;
pub mod profile;
pub mod selection;
pub mod selector;
pub mod settings;
pub mod state;
pub mod store;
pub mod subscription;
pub mod subscription_service;

pub use binaries::{InstallOptions, InstalledBinary, UpdateCheck, UpdateStatus};
pub use capability::{CoreCapability, ProfileRequirement, Protocol};
pub use compiler::{
    build_effective_rules, compile, compile_all, parse_rule_sets, parse_rules, CompileError,
    CompileOptions, CompiledConfig, ConfigCompiler, CustomRule, DnsMode, EffectiveRuleEntry,
    EffectiveRuleSetEntry, EffectiveRulesView, RuleError, RuleKind, RuleMode, RuleSet,
    RuleSetBehavior, RuleSetFormat, RuleTarget, SkippedNode, TunStack, AUTO_GROUP, PROXY_GROUP,
};
pub use controller::{
    ConnectionInfo, ConnectionsSnapshot, ControllerClient, ProxyGroupInfo, TrafficSample,
};
pub use core::{
    CoreKind, CoreManager, CoreReport, ManagerStatus, ProxyCore, RecoveryReport, StartReport,
    SwitchReport, SwitchStep, DEFAULT_API_PORT,
};
pub use error::{Error, Result};
pub use health::CoreHealth;
pub use latency::{
    active_delay, check_outbound_ip, measure_nodes, measure_nodes_streaming, measure_proxy_delay,
    measure_proxy_delay_streaming, tcp_latency, ActiveDelay, LatencyKind, LatencySample,
    OutboundIpInfo, DEFAULT_PROBE_URL,
};
pub use logging::{LogBuffer, LogLevel, LogLine, LogStream};
pub use network::{
    authorize_tun_binaries, binary_has_tun_capability, cleanup_tun_interface, ensure_tun_device,
    is_elevated, is_tun_authorized_for, leftovers, relaunch_elevated, tun_unavailable_reason,
    tun_unavailable_reason_for, AppliedProxy, ProxyBackend, SystemProxyManager, TunAdapter,
    INTERFACE_NAME,
};
pub use paths::Layout;
pub use platform::{Desktop, Distro, PackageFamily, Selinux};
pub use profile::{
    parse_payload, parse_uri, Credentials, Diagnostic, InputFormat, Node, ParsedProfile, Profile,
    Severity, Transport, VlessFlow,
};
pub use selection::{select_node, ApplyOutcome};
pub use selector::{
    CoreEvaluation, CorePreference, NetworkRequirements, SelectError, Selection, Selector,
};
pub use settings::{AppSettings, Theme};
pub use state::CoreState;
pub use store::{ProfileStore, StoredProfile, StoredRecord};
pub use subscription::{FetchOutcome, Subscription, SubscriptionClient};
pub use subscription_service::{RefreshOutcome, RefreshStatus, SubscriptionService};
