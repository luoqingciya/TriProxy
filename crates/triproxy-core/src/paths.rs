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
        let root = root.into();
        // A relative root is a trap, not a convenience: cores are launched with
        // `-d <directory of the config>` and mihomo resolves a relative `-f`
        // against it, so `--data-dir ./data` made the core look for
        // `./data/configs/mihomo/./data/configs/mihomo/config.yaml`, find
        // nothing, and silently start from its own default config — profile
        // ignored, no controller API, no explanation. Anchoring the root to the
        // working directory once, here, keeps every derived path meaningful no
        // matter what a child process does with its cwd.
        let root = match std::env::current_dir() {
            Ok(cwd) if !root.is_absolute() => cwd.join(root),
            _ => root,
        };
        Self { root }
    }

    /// Resolve the default layout.
    ///
    /// Precedence:
    /// 1. `TRIPROXY_HOME`, for tests and for pointing several builds at one root.
    /// 2. `<directory of the executable>/data` — the portable layout.
    /// 3. The platform data directory (`%APPDATA%\TriProxy`, `~/Library/...`).
    ///
    /// Portable by default because that is what an installed Windows application
    /// is expected to do here: everything in one folder, so uninstalling or
    /// moving the program moves its state too. The data directory has to be
    /// writable — the installer grants the Users group modify rights on it, and
    /// the app itself always runs elevated.
    ///
    /// Step 3 exists for the cases where the portable layout cannot work: a
    /// binary in a read-only location, or one installed before this changed.
    pub fn discover() -> Result<Self> {
        if let Some(custom) = std::env::var_os("TRIPROXY_HOME") {
            if !custom.is_empty() {
                return Ok(Self::new(PathBuf::from(custom)));
            }
        }

        if let Some(root) = portable_root() {
            return Ok(Self::new(root));
        }

        let base = dirs::data_dir()
            .ok_or_else(|| Error::Other("cannot determine platform data directory".to_string()))?;
        Ok(Self::new(base.join(APP_DIR_NAME)))
    }

    /// The old platform data directory, if it exists.
    ///
    /// Used once, on first run after the portable change, to bring existing
    /// profiles across instead of presenting the user with an empty library.
    pub fn legacy_root() -> Option<PathBuf> {
        let base = dirs::data_dir()?;
        let root = base.join(APP_DIR_NAME);
        root.is_dir().then_some(root)
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

    /// Stored profiles (one JSON file per profile plus an index).
    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
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
            self.profiles_dir(),
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
/// Adopt `<exe dir>/data` if it exists or can be created.
///
/// **Windows only.** An installed Windows application is expected to keep
/// everything in one folder, and the installer grants the Users group write
/// access so the bundled CLI can use it from an ordinary terminal.
///
/// macOS and Linux deliberately do not do this. A `.app` is a sealed bundle and
/// an AppImage is a read-only mount; writing application state inside either is
/// wrong on those platforms, and the platform data directory is what users and
/// backup tools expect.
fn portable_root() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }

    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let root = dir.join("data");

    if root.is_dir() {
        return Some(root);
    }

    match std::fs::create_dir_all(&root) {
        Ok(()) => {
            migrate_from_legacy(&root);
            Some(root)
        }
        Err(_) => None,
    }
}

/// Bring the previous platform data directory across, once.
///
/// Without this the first launch after switching to the portable layout would
/// look like a fresh install: an empty profile library and no active profile,
/// for no reason the user can see.
fn migrate_from_legacy(root: &Path) {
    let Some(legacy) = Layout::legacy_root() else {
        return;
    };
    if legacy == root {
        return;
    }

    match copy_tree(&legacy, root) {
        Ok(()) => tracing::info!(
            from = %legacy.display(),
            to = %root.display(),
            "migrated the previous data directory"
        ),
        // Not fatal: the app works with an empty root, and the old data is still
        // where it was.
        Err(error) => {
            tracing::warn!(error = %error, "could not migrate the previous data directory")
        }
    }
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if !target.exists() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// first-run download. Nothing is ever written here at runtime — updates always
/// go to the managed `bin/` directory, which takes precedence.
pub fn bundled_core_dir() -> Option<PathBuf> {
    // Set by the desktop shell from its own resource directory. Needed because
    // the location differs per platform and cannot be derived from the
    // executable path alone: on macOS the cores live in
    // `TriProxy.app/Contents/Resources/cores`, and a Linux `.deb` installs them
    // under `/usr/lib/triproxy/cores`.
    if let Some(explicit) = std::env::var_os("TRIPROXY_BUNDLED_CORES") {
        if !explicit.is_empty() {
            let dir = PathBuf::from(explicit);
            if dir.is_dir() {
                return Some(dir);
            }
        }
    }

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
        // `temp_dir()` rather than a literal "/tmp": on Windows that string has
        // no drive letter and is therefore *relative*, so the layout anchors it
        // to the working directory and every assertion below would shift.
        let root = std::env::temp_dir().join("triproxy-test");
        let layout = Layout::new(&root);
        assert_eq!(layout.bin_dir(), root.join("bin"));
        assert_eq!(
            layout.core_config_dir("sing-box"),
            root.join("configs/sing-box")
        );
        assert_eq!(layout.state_file(), root.join("state.json"));
    }

    #[test]
    fn a_relative_root_is_anchored_to_the_working_directory() {
        let layout = Layout::new("some-relative-dir");
        assert!(
            layout.root().is_absolute(),
            "a relative root breaks core launch: {}",
            layout.root().display()
        );
        assert!(layout.root().ends_with("some-relative-dir"));
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
