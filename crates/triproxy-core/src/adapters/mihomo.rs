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

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adapters::{first_version_token, CoreDialect, ManagedCore};
use crate::bootstrap::{self, BootstrapOptions};
use crate::capability::CoreCapability;
use crate::core::{CoreKind, ProxyCore};
use crate::logging::LogBuffer;

/// Clash.Meta lineage: `-f config.yaml`, `-t` for validation, `-d` for the
/// working directory that holds geo databases.
pub struct MihomoDialect;

impl CoreDialect for MihomoDialect {
    fn kind(&self) -> CoreKind {
        CoreKind::Mihomo
    }

    fn capability(&self) -> CoreCapability {
        CoreCapability::for_core(CoreKind::Mihomo)
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
            OsString::from("-t"),
            OsString::from("-f"),
            config.as_os_str().to_owned(),
        ]
    }

    fn run_args(&self, config: &Path) -> Vec<OsString> {
        let mut args = vec![OsString::from("-f"), config.as_os_str().to_owned()];
        if let Some(dir) = config.parent() {
            args.push(OsString::from("-d"));
            args.push(dir.as_os_str().to_owned());
        }
        args
    }

    fn bootstrap(&self, options: &BootstrapOptions) -> String {
        bootstrap::render(CoreKind::Mihomo, options)
    }
}

pub fn new(binary: PathBuf, logs: Arc<LogBuffer>, log_file: Option<PathBuf>) -> Arc<dyn ProxyCore> {
    Arc::new(ManagedCore::new(
        Box::new(MihomoDialect),
        binary,
        logs,
        log_file,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_and_run_args_carry_the_config() {
        let dialect = MihomoDialect;
        let config = Path::new("/data/configs/mihomo/bootstrap.yaml");

        let check: Vec<String> = dialect
            .check_args(config)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(check[0], "-t");
        assert_eq!(check[1], "-f");
        assert_eq!(check[2], config.to_string_lossy());

        let run: Vec<String> = dialect
            .run_args(config)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(run.contains(&"-d".to_string()));
        assert!(run.contains(&config.to_string_lossy().into_owned()));
    }
}
