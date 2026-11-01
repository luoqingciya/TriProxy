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
pub mod core;
pub mod error;
pub mod health;
pub mod logging;
pub mod paths;
pub mod process;
pub mod profile;
pub mod state;

pub use capability::{CoreCapability, ProfileRequirement, Protocol};
pub use compiler::{
    compile, compile_all, CompileError, CompileOptions, CompiledConfig, ConfigCompiler, SkippedNode,
};
pub use core::{
    CoreKind, CoreManager, CoreReport, ManagerStatus, ProxyCore, RecoveryReport, StartReport,
    SwitchReport, SwitchStep,
};
pub use error::{Error, Result};
pub use health::CoreHealth;
pub use logging::{LogBuffer, LogLevel, LogLine, LogStream};
pub use paths::Layout;
pub use profile::{
    parse_payload, parse_uri, Credentials, Diagnostic, InputFormat, Node, ParsedProfile, Profile,
    Severity, Transport, VlessFlow,
};
pub use state::CoreState;
