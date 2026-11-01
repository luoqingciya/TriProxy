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

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

pub const APP_DIR_NAME: &str = "TriProxy";

/// Filesystem layout owned by TriProxy.
///
/// Everything the application writes lives under a single root so that
/// uninstalling is a matter of deleting one directory:
///
/// ```text
/// <root>/
/// ├── bin/                 managed core binaries (+ <core>.json install record)
/// ├── configs/<core>/      generated configs (bootstrap in Phase 1)
/// ├── logs/                rolling application logs
/// ├── downloads/           transient download staging
/// └── state.json           active core + network state for crash recovery
/// ```
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolve the default layout.
    ///
    /// Precedence: `TRIPROXY_HOME` env var, then the platform data directory
    /// (`%APPDATA%\TriProxy` on Windows, `~/Library/Application Support/TriProxy`
    /// on macOS, `~/.local/share/TriProxy` on Linux).
    pub fn discover() -> Result<Self> {
        if let Some(custom) = std::env::var_os("TRIPROXY_HOME") {
            if !custom.is_empty() {
                return Ok(Self::new(PathBuf::from(custom)));
            }
        }
        let base = dirs::data_dir()
            .ok_or_else(|| Error::Other("cannot determine platform data directory".to_string()))?;
        Ok(Self::new(base.join(APP_DIR_NAME)))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }

    pub fn configs_dir(&self) -> PathBuf {
        self.root.join("configs")
    }

    pub fn core_config_dir(&self, core: &str) -> PathBuf {
        self.configs_dir().join(core)
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Append-only log file for one core, so that logs survive between CLI
    /// invocations (the in-memory ring buffer is per process).
    pub fn core_log_file(&self, core: &str) -> PathBuf {
        self.logs_dir().join(format!("{core}.log"))
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.json")
    }

    pub fn ensure(&self) -> Result<()> {
        for dir in [
            self.root.clone(),
            self.bin_dir(),
            self.configs_dir(),
            self.logs_dir(),
            self.downloads_dir(),
        ] {
            std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        }
        Ok(())
    }

    /// Path of a managed binary inside `bin/`.
    pub fn managed_binary(&self, stem: &str) -> PathBuf {
        binary_in(&self.bin_dir(), stem)
    }

    pub fn install_record(&self, stem: &str) -> PathBuf {
        self.bin_dir().join(format!("{stem}.json"))
    }
}

/// Path of a core executable inside an arbitrary directory.
pub fn binary_in(dir: &Path, stem: &str) -> PathBuf {
    dir.join(format!("{stem}{}", std::env::consts::EXE_SUFFIX))
}

/// Directory next to the executable that ships pre-bundled cores.
///
/// The "with cores" release archive places verified cores (plus their install
/// records) in `cores/` beside the binary, so that build works offline with no
/// first-run download. Nothing is ever written here at runtime — updates always
/// go to the managed `bin/` directory, which takes precedence.
pub fn bundled_core_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("cores"))
}

/// Look up an executable on `PATH` without pulling in an extra dependency.
pub fn find_on_path(stem: &str) -> Option<PathBuf> {
    let file_name = format!("{stem}{}", std::env::consts::EXE_SUFFIX);
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_fully_derived_from_root() {
        let layout = Layout::new("/tmp/triproxy-test");
        assert_eq!(layout.bin_dir(), Path::new("/tmp/triproxy-test/bin"));
        assert_eq!(
            layout.core_config_dir("sing-box"),
            Path::new("/tmp/triproxy-test/configs/sing-box")
        );
        assert_eq!(
            layout.state_file(),
            Path::new("/tmp/triproxy-test/state.json")
        );
    }

    #[test]
    fn managed_binary_carries_platform_suffix() {
        let layout = Layout::new("/tmp/triproxy-test");
        let expected = format!("xray{}", std::env::consts::EXE_SUFFIX);
        assert_eq!(
            layout.managed_binary("xray").file_name().unwrap(),
            expected.as_str()
        );
    }
}
