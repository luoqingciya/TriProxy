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

//! User-written routing rules.
//!
//! The two presets (`global` / `split`) answer "do Chinese sites go direct";
//! they cannot answer "my company's internal domain must never leave the
//! machine". That is what this module is for.
//!
//! Rules are written in the Clash syntax everyone already knows:
//!
//! ```text
//! DOMAIN-SUFFIX,corp.example.com,DIRECT
//! DOMAIN-KEYWORD,telemetry,PROXY
//! IP-CIDR,10.0.0.0/8,DIRECT
//! ```
//!
//! Parsing happens once, here, and each compiler renders the neutral form into
//! its own dialect. Keeping one parser is what stops the three cores from
//! disagreeing about what a rule means.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// What part of a request a rule matches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleKind {
    /// Exact host.
    Domain,
    /// Host and every subdomain of it.
    DomainSuffix,
    /// Anywhere in the host.
    DomainKeyword,
    /// Destination address range.
    IpCidr,
}

/// Where a matching request goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleTarget {
    Proxy,
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomRule {
    pub kind: RuleKind,
    pub value: String,
    pub target: RuleTarget,
}

/// A rejected line, with enough context to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    /// 1-based, counting every line including blanks and comments: the user is
    /// looking at a textarea, not at a filtered list.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for RuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "第 {} 行：{}", self.line, self.message)
    }
}

/// Parse the rule text.
///
/// Blank lines and `#` comments are skipped. Extra trailing fields (Clash's
/// `no-resolve`) are accepted and ignored rather than rejected: they are valid
/// in the syntax users copy from elsewhere, and refusing them would only teach
/// people to delete something harmless.
pub fn parse_rules(text: &str) -> Result<Vec<CustomRule>, RuleError> {
    let mut rules = Vec::new();

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }

        let fields: Vec<&str> = trimmed.split(',').map(str::trim).collect();
        if fields.len() < 3 {
            return Err(RuleError {
                line,
                message: "需要三个字段：类型,值,PROXY 或 DIRECT".to_string(),
            });
        }

        let kind = match fields[0].to_ascii_uppercase().as_str() {
            "DOMAIN" => RuleKind::Domain,
            "DOMAIN-SUFFIX" => RuleKind::DomainSuffix,
            "DOMAIN-KEYWORD" => RuleKind::DomainKeyword,
            "IP-CIDR" | "IP-CIDR6" => RuleKind::IpCidr,
            other => {
                return Err(RuleError {
                    line,
                    message: format!(
                        "不支持的类型 `{other}`；可用 DOMAIN / DOMAIN-SUFFIX / DOMAIN-KEYWORD / IP-CIDR"
                    ),
                })
            }
        };

        let target = match fields[2].to_ascii_uppercase().as_str() {
            "PROXY" => RuleTarget::Proxy,
            "DIRECT" => RuleTarget::Direct,
            other => {
                return Err(RuleError {
                    line,
                    message: format!("目标只能是 PROXY 或 DIRECT，收到 `{other}`"),
                })
            }
        };

        let value = fields[1].trim();
        if value.is_empty() {
            return Err(RuleError {
                line,
                message: "值不能为空".to_string(),
            });
        }

        if kind == RuleKind::IpCidr && !valid_cidr(value) {
            return Err(RuleError {
                line,
                message: format!("`{value}` 不是合法的 IP 或 CIDR"),
            });
        }

        rules.push(CustomRule {
            kind,
            value: value.to_string(),
            target,
        });
    }

    Ok(rules)
}

/// A bare address or `address/prefix`. A bad prefix length is caught here rather
/// than by the core at start time, where the error names a config file the user
/// never wrote.
fn valid_cidr(value: &str) -> bool {
    let (address, prefix) = match value.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix)),
        None => (value, None),
    };

    let Ok(address) = address.parse::<IpAddr>() else {
        return false;
    };

    match prefix {
        None => true,
        Some(prefix) => {
            let Ok(prefix) = prefix.parse::<u8>() else {
                return false;
            };
            match address {
                IpAddr::V4(_) => prefix <= 32,
                IpAddr::V6(_) => prefix <= 128,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_field_form() {
        let rules = parse_rules("DOMAIN-SUFFIX,corp.example.com,DIRECT").unwrap();
        assert_eq!(
            rules,
            vec![CustomRule {
                kind: RuleKind::DomainSuffix,
                value: "corp.example.com".to_string(),
                target: RuleTarget::Direct,
            }]
        );
    }

    #[test]
    fn ignores_blanks_comments_and_case() {
        let rules = parse_rules(
            "# 内网直连\n\n  domain,example.com,proxy  \n// 注释\nDOMAIN-KEYWORD,telemetry,DIRECT",
        )
        .unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].kind, RuleKind::Domain);
        assert_eq!(rules[0].target, RuleTarget::Proxy);
        assert_eq!(rules[1].kind, RuleKind::DomainKeyword);
    }

    /// `no-resolve` is copied along with the rule all over the internet; it must
    /// not make an otherwise valid line fail.
    #[test]
    fn accepts_and_ignores_trailing_options() {
        let rules = parse_rules("IP-CIDR,10.0.0.0/8,DIRECT,no-resolve").unwrap();
        assert_eq!(rules[0].kind, RuleKind::IpCidr);
        assert_eq!(rules[0].value, "10.0.0.0/8");
    }

    #[test]
    fn reports_the_line_number_of_a_bad_rule() {
        let error = parse_rules("DOMAIN,ok.com,DIRECT\n\nDOMAIN-BOGUS,x.com,DIRECT").unwrap_err();
        // The blank line counts: the user is looking at a textarea.
        assert_eq!(error.line, 3);
        assert!(error.to_string().contains("第 3 行"));
    }

    #[test]
    fn rejects_a_target_that_is_not_proxy_or_direct() {
        let error = parse_rules("DOMAIN,x.com,REJECT").unwrap_err();
        assert!(error.message.contains("PROXY 或 DIRECT"));
    }

    #[test]
    fn validates_cidr_prefix_lengths() {
        assert!(parse_rules("IP-CIDR,10.0.0.0/33,DIRECT").is_err());
        assert!(parse_rules("IP-CIDR,10.0.0.0/8,DIRECT").is_ok());
        assert!(parse_rules("IP-CIDR,not-an-ip/8,DIRECT").is_err());
        assert!(parse_rules("IP-CIDR,2001:db8::/32,DIRECT").is_ok());
    }
}
