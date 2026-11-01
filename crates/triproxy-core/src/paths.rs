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
        let base = dirs::data_dir().ok_or_else(|| {
            Error::Other("cannot determine platform data directory".to_string())
        })?;
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
        self.bin_dir().join(format!("{stem}{}", std::env::consts::EXE_SUFFIX))
    }

    pub fn install_record(&self, stem: &str) -> PathBuf {
        self.bin_dir().join(format!("{stem}.json"))
    }
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
        assert_eq!(layout.state_file(), Path::new("/tmp/triproxy-test/state.json"));
    }

    #[test]
    fn managed_binary_carries_platform_suffix() {
        let layout = Layout::new("/tmp/triproxy-test");
        let expected = format!("xray{}", std::env::consts::EXE_SUFFIX);
        assert_eq!(layout.managed_binary("xray").file_name().unwrap(), expected.as_str());
    }
}
