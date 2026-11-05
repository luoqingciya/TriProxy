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

//! Resident memory of a running process.
//!
//! **What the number means.** It is the resident set size: the pages currently
//! mapped into physical RAM, which is the figure Task Manager and Activity
//! Monitor show for a process. It is not the virtual size — on macOS that is
//! routinely hundreds of gigabytes for every process, and quoting it would make
//! a healthy core look like a catastrophe.
//!
//! **Why it is not cached.** A proxy core's footprint moves with the number of
//! live connections and the size of the rule data it loaded; a cached number
//! would keep reporting the value from the moment the core started. Reading one
//! process is a single `OpenProcess` + `GetProcessMemoryInfo` on Windows and one
//! `/proc/<pid>/statm` read on Linux, which is cheap enough to do on every status
//! poll (every two seconds) without a cache to invalidate.
//!
//! Platform differences are handled by `sysinfo` rather than by hand-written
//! `#[cfg]` branches: the macOS path (`proc_pidinfo`) has a struct layout that
//! cannot be verified on the platforms this project is developed on, and a
//! wrong offset would produce a plausible-looking wrong number — the worst
//! possible failure for a statistic.

use sysinfo::{Pid, ProcessesToUpdate, System};

/// Resident memory of `pid` in bytes, or `None` when the process is gone or the
/// platform refused the query.
///
/// `None` is a legitimate answer: a core that exited between the status poll and
/// this call has no memory figure, and inventing a zero would make the UI claim
/// the process is alive and using nothing.
pub fn resident_bytes(pid: u32) -> Option<u64> {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    // Only the one process: refreshing everything would walk the whole process
    // table on every poll, which on Windows is far more expensive than the
    // single query this is here for.
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(|process| process.memory())
}

/// Resident memory of the calling process.
pub fn self_resident_bytes() -> Option<u64> {
    resident_bytes(std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_plausible_figure_for_this_process() {
        let bytes = self_resident_bytes().expect("the test process exists");
        // A test binary is at least a few hundred kilobytes; anything smaller
        // means the unit is wrong (KiB read as bytes, or a bitfield).
        assert!(bytes > 128 * 1024, "suspiciously small: {bytes} bytes");
        assert!(
            bytes < 64 * 1024 * 1024 * 1024,
            "suspiciously large: {bytes}"
        );
    }

    #[test]
    fn a_dead_process_reports_nothing() {
        // `u32::MAX` is not a valid PID on any platform this ships for. The
        // contract that matters is `None`, not `Some(0)`: a zero would render
        // as a live process using no memory.
        assert_eq!(resident_bytes(u32::MAX), None);
    }
}
