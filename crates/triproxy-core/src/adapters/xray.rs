use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adapters::{first_version_token, CoreDialect, ManagedCore};
use crate::bootstrap::{self, BootstrapOptions};
use crate::capability::CoreCapability;
use crate::core::{CoreKind, ProxyCore};
use crate::logging::LogBuffer;

/// V2Ray/Xray lineage: `-config config.json`, `-test` for validation.
pub struct XrayDialect;

impl CoreDialect for XrayDialect {
    fn kind(&self) -> CoreKind {
        CoreKind::Xray
    }

    fn capability(&self) -> CoreCapability {
        CoreCapability::for_core(CoreKind::Xray)
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
            OsString::from("-test"),
            OsString::from("-config"),
            config.as_os_str().to_owned(),
        ]
    }

    fn run_args(&self, config: &Path) -> Vec<OsString> {
        vec![
            OsString::from("-config"),
            config.as_os_str().to_owned(),
        ]
    }

    fn bootstrap(&self, options: &BootstrapOptions) -> String {
        bootstrap::render(CoreKind::Xray, options)
    }
}

pub fn new(
    binary: PathBuf,
    logs: Arc<LogBuffer>,
    log_file: Option<PathBuf>,
) -> Arc<dyn ProxyCore> {
    Arc::new(ManagedCore::new(
        Box::new(XrayDialect),
        binary,
        logs,
        log_file,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xray_uses_flag_style_arguments() {
        let dialect = XrayDialect;
        let config = Path::new("/data/configs/xray/bootstrap.json");

        let check: Vec<String> = dialect
            .check_args(config)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(check[0], "-test");
        assert_eq!(check[1], "-config");

        let run: Vec<String> = dialect
            .run_args(config)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(run, vec!["-config".to_string(), config.to_string_lossy().into_owned()]);
    }
}
