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

use serde::{Deserialize, Serialize};

/// Lifecycle state machine of a single proxy core.
///
/// ```text
/// Stopped --start--> Starting --ready--> Running --stop--> Stopping --> Stopped
///                        |                                     |
///                        +------------- Failed <---------------+
/// ```
///
/// `Failed` is reachable from `Starting` (config rejected, bind failure, early
/// exit) and from `Running` (unexpected process exit detected by the supervisor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

impl CoreState {
    pub fn as_str(self) -> &'static str {
        match self {
            CoreState::Stopped => "stopped",
            CoreState::Starting => "starting",
            CoreState::Running => "running",
            CoreState::Stopping => "stopping",
            CoreState::Failed => "failed",
        }
    }

    pub fn is_running(self) -> bool {
        matches!(self, CoreState::Running)
    }

    /// A core may only be started from a terminal state.
    pub fn can_start(self) -> bool {
        matches!(self, CoreState::Stopped | CoreState::Failed)
    }
}

impl std::fmt::Display for CoreState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_terminal_states_can_start() {
        assert!(CoreState::Stopped.can_start());
        assert!(CoreState::Failed.can_start());
        assert!(!CoreState::Running.can_start());
        assert!(!CoreState::Starting.can_start());
        assert!(!CoreState::Stopping.can_start());
    }

    #[test]
    fn state_roundtrips_through_json() {
        for state in [
            CoreState::Stopped,
            CoreState::Starting,
            CoreState::Running,
            CoreState::Stopping,
            CoreState::Failed,
        ] {
            let encoded = serde_json::to_string(&state).unwrap();
            let decoded: CoreState = serde_json::from_str(&encoded).unwrap();
            assert_eq!(state, decoded);
        }
    }
}
