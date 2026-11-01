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

//! Version comparison for core updates.
//!
//! The three projects do not share a versioning scheme: Mihomo is `1.19.33`,
//! sing-box `1.14.4`, Xray `26.10.31`, and some tags carry a `v` or a suffix
//! like `-beta.1`. A full semver parser would reject half of them, so this
//! compares the leading numeric components and ignores the rest. That is enough
//! to answer "is the release newer than what is installed", which is the only
//! question being asked.

use std::cmp::Ordering;

/// Compare two version strings, ignoring a leading `v` and any suffix.
///
/// `1.19.33` vs `1.19.4` compares component by component, not as strings — the
/// string comparison would call `1.19.4` newer.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let left = parse(a);
    let right = parse(b);
    let width = left.parts.len().max(right.parts.len());

    for index in 0..width {
        let l = left.parts.get(index).copied().unwrap_or(0);
        let r = right.parts.get(index).copied().unwrap_or(0);
        match l.cmp(&r) {
            Ordering::Equal => continue,
            other => return other,
        }
    }

    // Same numbers: a pre-release (`1.15.0-beta.1`) sorts *below* the release
    // (`1.15.0`), which is what semver says and what users expect — otherwise
    // every beta would look like an update over a stable build.
    match (left.prerelease, right.prerelease) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

struct Parsed {
    parts: Vec<u64>,
    /// The version carried a non-numeric suffix.
    prerelease: bool,
}

fn parse(version: &str) -> Parsed {
    let mut parts = Vec::new();
    let mut prerelease = false;

    for part in version.trim().trim_start_matches('v').split('.') {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        // A component that is not purely numeric ends the release version:
        // everything after it is a pre-release/build suffix, not a new field.
        if digits.len() != part.len() || digits.is_empty() {
            prerelease = true;
            break;
        }
        parts.push(digits.parse().unwrap_or(0));
    }

    Parsed { parts, prerelease }
}

/// Is `candidate` newer than `installed`?
pub fn is_newer(candidate: &str, installed: &str) -> bool {
    compare_versions(candidate, installed) == Ordering::Greater
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_numerically_not_lexicographically() {
        // The whole point: as strings, "1.19.4" > "1.19.33".
        assert!(is_newer("1.19.33", "1.19.4"));
        assert!(!is_newer("1.19.4", "1.19.33"));
    }

    #[test]
    fn handles_a_leading_v() {
        assert!(is_newer("v1.14.5", "1.14.4"));
        assert!(!is_newer("v1.14.4", "1.14.4"));
    }

    #[test]
    fn handles_different_component_counts() {
        assert!(is_newer("2.0", "1.9.9"));
        assert!(!is_newer("1.9", "1.9.0"));
        assert!(is_newer("1.9.1", "1.9"));
    }

    #[test]
    fn ignores_suffixes() {
        assert!(is_newer("1.15.0-beta.1", "1.14.9"));
        assert!(!is_newer("1.15.0-beta.1", "1.15.0"));
    }

    #[test]
    fn xray_style_dates_work() {
        assert!(is_newer("26.10.31", "26.9.1"));
        assert!(!is_newer("26.9.1", "26.10.31"));
    }

    #[test]
    fn equal_versions_are_not_newer() {
        assert_eq!(compare_versions("1.2.3", "v1.2.3"), Ordering::Equal);
        assert!(!is_newer("1.2.3", "1.2.3"));
    }
}
