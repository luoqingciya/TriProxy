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
pub mod core;
pub mod error;
pub mod health;
pub mod logging;
pub mod paths;
pub mod process;
pub mod state;

pub use capability::{CoreCapability, ProfileRequirement, Protocol};
pub use core::{
    CoreKind, CoreManager, CoreReport, ManagerStatus, ProxyCore, RecoveryReport, StartReport,
    SwitchReport, SwitchStep,
};
pub use error::{Error, Result};
pub use health::CoreHealth;
pub use logging::{LogBuffer, LogLevel, LogLine, LogStream};
pub use paths::Layout;
pub use state::CoreState;
