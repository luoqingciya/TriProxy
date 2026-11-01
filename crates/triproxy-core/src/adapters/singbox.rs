use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adapters::{first_version_token, CoreDialect, ManagedCore};
use crate::bootstrap::{self, BootstrapOptions};
use crate::capability::CoreCapability;
use crate::core::{CoreKind, ProxyCore};
use crate::logging::LogBuffer;

/// Modern general purpose core: `run -c config.json`, `check -c config.json`.
pub struct SingBoxDialect;

impl CoreDialect for SingBoxDialect {
    fn kind(&self) -> CoreKind {
        CoreKind::SingBox
    }

    fn capability(&self) -> CoreCapability {
        CoreCapability::for_core(CoreKind::SingBox)
    }

    fn version_args(&self) -> Vec<OsString> {
        self.kind()
            .version_probe_args()
            .iter()
            .map(OsString::from)
            .collect()
    }

    fn parse_version(&self, output: &str) -> Option<String> {
        first_version_token(output)
    }

    fn check_args(&self, config: &Path) -> Vec<OsString> {
        vec![
            OsString::from("check"),
            OsString::from("-c"),
            config.as_os_str().to_owned(),
        ]
    }

    fn run_args(&self, config: &Path) -> Vec<OsString> {
        vec![
            OsString::from("run"),
            OsString::from("-c"),
            config.as_os_str().to_owned(),
        ]
    }

    fn bootstrap(&self, options: &BootstrapOptions) -> String {
        bootstrap::render(CoreKind::SingBox, options)
    }
}

pub fn new(
    binary: PathBuf,
    logs: Arc<LogBuffer>,
    log_file: Option<PathBuf>,
) -> Arc<dyn ProxyCore> {
    Arc::new(ManagedCore::new(
        Box::new(SingBoxDialect),
        binary,
        logs,
        log_file,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singbox_uses_subcommands() {
        let dialect = SingBoxDialect;
        let config = Path::new("/data/configs/sing-box/bootstrap.json");

        let check: Vec<String> = dialect
            .check_args(config)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(check[0], "check");

        let run: Vec<String> = dialect
            .run_args(config)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(run[0], "run");
        assert_eq!(run[2], config.to_string_lossy());
    }
}
