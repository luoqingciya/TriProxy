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

/// Unified error type for the core layer.
///
/// Every variant that describes a core failure carries the core name so that
/// the CLI / GUI can render an actionable message without extra plumbing.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("binary for `{core}` not found ({hint})")]
    BinaryNotFound { core: String, hint: String },

    #[error("binary for `{core}` failed verification: {reason}")]
    ChecksumMismatch { core: String, reason: String },

    #[error("`{core}` is {state}; expected one of: {expected}")]
    InvalidTransition {
        core: String,
        state: String,
        expected: String,
    },

    #[error("`{core}` failed to start: {reason}")]
    StartupFailed { core: String, reason: String },

    #[error("`{core}` rejected the generated config: {reason}")]
    ConfigInvalid { core: String, reason: String },

    #[error("no active core")]
    NoActiveCore,

    #[error("`{core}` does not support target {target}")]
    UnsupportedPlatform { core: String, target: String },

    #[error("no release asset matches target {target} for `{core}` (available: {available})")]
    NoMatchingAsset {
        core: String,
        target: String,
        available: String,
    },

    #[error("port {port} is already in use, `{core}` cannot bind")]
    PortInUse { core: String, port: u16 },

    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    IoPlain(#[from] std::io::Error),

    #[error("network error: {0}")]
    Network(String),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
