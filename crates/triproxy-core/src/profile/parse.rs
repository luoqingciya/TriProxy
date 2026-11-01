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

//! Subscription payload → `Profile`.
//!
//! Providers hand out either a plain newline-separated list of share links or
//! that list wrapped in base64. Both are accepted, and a single malformed line
//! never aborts the import: it becomes a diagnostic and the rest still loads.

use std::collections::HashSet;

use super::encoding::decode_base64_string;
use super::node::Node;
use super::uri::parse_uri;
use super::validate::{validate_profile, Diagnostic, Severity};
use super::{InputFormat, Profile};

/// Result of importing a subscription payload.
#[derive(Debug, Clone)]
pub struct ParsedProfile {
    pub profile: Profile,
    /// Parse failures (one per rejected line) followed by validation findings.
    pub diagnostics: Vec<Diagnostic>,
}

impl ParsedProfile {
    pub fn node_count(&self) -> usize {
        self.profile.nodes.len()
    }

    pub fn errors(&self) -> usize {
        super::validate::count(&self.diagnostics, Severity::Error)
    }

    pub fn warnings(&self) -> usize {
        super::validate::count(&self.diagnostics, Severity::Warning)
    }
}

/// Parse a subscription payload.
///
/// `name` is the display name for the profile; `source` is a human-readable
/// label (file path, `stdin`, ...) recorded in the metadata.
pub fn parse_payload(text: &str, name: &str, source: &str) -> ParsedProfile {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return ParsedProfile {
            profile: Profile::new(name, Vec::new()).with_metadata(source, InputFormat::UriList),
            diagnostics: vec![Diagnostic {
                severity: Severity::Error,
                node: None,
                node_name: None,
                code: "input.empty".to_string(),
                message: "input is empty".to_string(),
            }],
        };
    }

    let (body, format) = unwrap_payload(trimmed);

    let mut diagnostics = Vec::new();
    let mut nodes: Vec<Node> = Vec::new();
    let mut used_names: HashSet<String> = HashSet::new();

    for (index, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }

        match parse_uri(line) {
            Ok(mut node) => {
                if node.name.trim().is_empty() {
                    node.name = format!("{}-{:02}", node.protocol, nodes.len() + 1);
                }
                node.name = unique_name(&mut used_names, node.name.trim());
                nodes.push(node);
            }
            Err(issue) => diagnostics.push(Diagnostic {
                severity: Severity::Error,
                node: None,
                node_name: None,
                code: format!("line.{}", issue.code),
                message: format!("line {}: {}", index + 1, issue.message),
            }),
        }
    }

    let profile = Profile::new(name, nodes).with_metadata(source, format);
    diagnostics.extend(validate_profile(&profile));

    ParsedProfile {
        profile,
        diagnostics,
    }
}

/// Detect and unwrap a base64-wrapped subscription.
///
/// A payload containing `://` is already a URI list; anything else is assumed to
/// be base64 (that is what providers emit). If decoding fails, the original text
/// is kept so the caller sees a per-line error rather than a vague "bad input".
fn unwrap_payload(text: &str) -> (String, InputFormat) {
    if text.contains("://") {
        return (text.to_string(), InputFormat::UriList);
    }
    match decode_base64_string(text) {
        Some(decoded) if decoded.contains("://") => (decoded, InputFormat::Base64Subscription),
        _ => (text.to_string(), InputFormat::UriList),
    }
}

/// Clash and sing-box both key proxies by name, so names must be unique within a
/// profile. Collisions are resolved here instead of failing the import.
fn unique_name(used: &mut HashSet<String>, candidate: &str) -> String {
    let base = if candidate.is_empty() {
        "node".to_string()
    } else {
        candidate.to_string()
    };
    if used.insert(base.clone()) {
        return base;
    }
    for suffix in 2.. {
        let candidate = format!("{base} ({suffix})");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("suffix loop is unbounded")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TROJAN: &str = "trojan://pw@a.example.com:443#A";
    const VLESS: &str =
        "vless://11111111-2222-3333-4444-555555555555@b.example.com:443?security=tls#B";

    #[test]
    fn plain_uri_list_is_parsed() {
        let parsed = parse_payload(&format!("{TROJAN}\n{VLESS}\n"), "sub", "stdin");
        assert_eq!(parsed.node_count(), 2);
        assert_eq!(parsed.errors(), 0);
        assert_eq!(parsed.profile.metadata.format, InputFormat::UriList);
        assert_eq!(parsed.profile.metadata.source, "stdin");
        assert_eq!(parsed.profile.nodes[0].name, "A");
        assert_eq!(parsed.profile.nodes[1].name, "B");
    }

    #[test]
    fn base64_wrapped_subscription_is_detected_and_decoded() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;

        let payload = format!("{TROJAN}\n{VLESS}");
        let encoded = STANDARD.encode(payload.as_bytes());
        let parsed = parse_payload(&encoded, "sub", "file.txt");

        assert_eq!(parsed.node_count(), 2);
        assert_eq!(
            parsed.profile.metadata.format,
            InputFormat::Base64Subscription
        );
    }

    #[test]
    fn one_bad_line_does_not_lose_the_others() {
        let payload = format!("{TROJAN}\nhysteria2://x@y:1#C\n{VLESS}");
        let parsed = parse_payload(&payload, "sub", "stdin");

        assert_eq!(parsed.node_count(), 2);
        assert_eq!(parsed.errors(), 1);
        let error = parsed
            .diagnostics
            .iter()
            .find(|d| d.code.starts_with("line."))
            .expect("a per-line diagnostic");
        assert!(error.message.contains("line 2"), "{}", error.message);
        assert!(error.message.contains("hysteria2"));
    }

    #[test]
    fn blank_lines_and_comments_are_ignored() {
        let payload = format!("# provider: example\n\n{TROJAN}\n\n// note\n");
        let parsed = parse_payload(&payload, "sub", "stdin");
        assert_eq!(parsed.node_count(), 1);
        assert_eq!(parsed.errors(), 0);
    }

    #[test]
    fn unnamed_nodes_get_a_generated_name() {
        let parsed = parse_payload("trojan://pw@a.example.com:443", "sub", "stdin");
        assert_eq!(parsed.profile.nodes[0].name, "trojan-01");
    }

    #[test]
    fn duplicate_names_are_made_unique() {
        let payload = "trojan://pw@a.example.com:443#Same\ntrojan://pw@b.example.com:443#Same";
        let parsed = parse_payload(payload, "sub", "stdin");
        assert_eq!(parsed.profile.nodes[0].name, "Same");
        assert_eq!(parsed.profile.nodes[1].name, "Same (2)");
    }

    #[test]
    fn empty_input_reports_an_error_instead_of_panicking() {
        let parsed = parse_payload("   \n\n", "sub", "stdin");
        assert_eq!(parsed.node_count(), 0);
        assert_eq!(parsed.errors(), 1);
        assert_eq!(parsed.diagnostics[0].code, "input.empty");
    }

    #[test]
    fn payload_without_any_share_link_reports_no_nodes() {
        let parsed = parse_payload("this is not a subscription", "sub", "stdin");
        assert_eq!(parsed.node_count(), 0);
        assert!(parsed
            .diagnostics
            .iter()
            .any(|d| d.code == "profile.no_nodes"));
    }

    #[test]
    fn validation_findings_are_merged_with_parse_findings() {
        // allowInsecure produces a warning; the node itself is still usable.
        let payload = "trojan://pw@a.example.com:443?security=tls&allowInsecure=1#A";
        let parsed = parse_payload(payload, "sub", "stdin");
        assert_eq!(parsed.node_count(), 1);
        assert_eq!(parsed.warnings(), 1);
        assert!(parsed.diagnostics.iter().any(|d| d.code == "tls.insecure"));
    }
}
