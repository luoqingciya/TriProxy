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
    /// Destination address range (IPv4 or IPv6 CIDR).
    IpCidr,
    /// Geosite category tag (for example `cn` or `geolocation-!cn`).
    GeoSite,
    /// GeoIP country/region code (for example `CN` or `LAN`).
    GeoIp,
    /// Executable/process name.
    ProcessName,
    /// Destination TCP/UDP port.
    DstPort,
}

impl RuleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleKind::Domain => "DOMAIN",
            RuleKind::DomainSuffix => "DOMAIN-SUFFIX",
            RuleKind::DomainKeyword => "DOMAIN-KEYWORD",
            RuleKind::IpCidr => "IP-CIDR",
            RuleKind::GeoSite => "GEOSITE",
            RuleKind::GeoIp => "GEOIP",
            RuleKind::ProcessName => "PROCESS-NAME",
            RuleKind::DstPort => "DST-PORT",
        }
    }
}

/// Where a matching request goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleTarget {
    Proxy,
    Direct,
    Reject,
}

impl RuleTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleTarget::Proxy => "PROXY",
            RuleTarget::Direct => "DIRECT",
            RuleTarget::Reject => "REJECT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomRule {
    pub kind: RuleKind,
    pub value: String,
    pub target: RuleTarget,
    /// Optional subscription proxy-group name (used when that group is compiled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
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
                message: "需要三个字段：类型,值,PROXY / DIRECT / REJECT".to_string(),
            });
        }

        let kind = match fields[0].to_ascii_uppercase().as_str() {
            "DOMAIN" => RuleKind::Domain,
            "DOMAIN-SUFFIX" => RuleKind::DomainSuffix,
            "DOMAIN-KEYWORD" => RuleKind::DomainKeyword,
            "IP-CIDR" | "IP-CIDR6" => RuleKind::IpCidr,
            "GEOSITE" => RuleKind::GeoSite,
            "GEOIP" => RuleKind::GeoIp,
            "PROCESS-NAME" => RuleKind::ProcessName,
            "DST-PORT" => RuleKind::DstPort,
            other => {
                return Err(RuleError {
                    line,
                    message: format!(
                        "不支持的类型 `{other}`；可用 DOMAIN / DOMAIN-SUFFIX / DOMAIN-KEYWORD / IP-CIDR / GEOSITE / GEOIP / PROCESS-NAME / DST-PORT"
                    ),
                })
            }
        };

        let target = match fields[2].to_ascii_uppercase().as_str() {
            "PROXY" => RuleTarget::Proxy,
            "DIRECT" => RuleTarget::Direct,
            "REJECT" | "REJECT-DROP" => RuleTarget::Reject,
            other => {
                return Err(RuleError {
                    line,
                    message: format!("目标只能是 PROXY、DIRECT 或 REJECT，收到 `{other}`"),
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

        if kind == RuleKind::DstPort && !valid_port_spec(value) {
            return Err(RuleError {
                line,
                message: format!("`{value}` 不是合法的端口或端口范围"),
            });
        }

        rules.push(CustomRule {
            kind,
            value: value.to_string(),
            target,
            group: None,
        });
    }

    Ok(rules)
}

pub(crate) fn valid_port_spec(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.parse::<u16>().is_ok_and(|p| p > 0) {
        return true;
    }
    if let Some((start, end)) = trimmed.split_once('-') {
        if let (Ok(s), Ok(e)) = (start.trim().parse::<u16>(), end.trim().parse::<u16>()) {
            return s > 0 && s <= e;
        }
    }
    false
}

/// A bare address or `address/prefix`. A bad prefix length is caught here rather
/// than by the core at start time, where the error names a config file the user
/// never wrote.
pub(crate) fn valid_cidr(value: &str) -> bool {
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

// ---------------------------------------------------------------------------
// remote rule-sets
// ---------------------------------------------------------------------------

/// Which core can read a rule-set file.
///
/// The two families do not share a format: Mihomo downloads `.mrs`/`.yaml`
/// providers, sing-box downloads `.srs` binary rule-sets. One URL cannot serve
/// both, which is why the format is derived per entry instead of being a single
/// app-wide setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleSetFormat {
    Clash,
    SingBox,
}

impl RuleSetFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleSetFormat::Clash => "clash",
            RuleSetFormat::SingBox => "sing-box",
        }
    }
}

/// What a Mihomo rule-provider contains.
///
/// sing-box infers this from the file, so the field is ignored there — but
/// Mihomo requires it to be declared, and guessing wrong makes it reject the
/// config at load time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleSetBehavior {
    Domain,
    IpCidr,
    Classical,
}

impl RuleSetBehavior {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleSetBehavior::Domain => "domain",
            RuleSetBehavior::IpCidr => "ipcidr",
            RuleSetBehavior::Classical => "classical",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSet {
    /// Identifier used as the provider key / rule-set tag. Derived from the URL.
    pub name: String,
    pub url: String,
    pub target: RuleTarget,
    pub behavior: RuleSetBehavior,
    pub format: RuleSetFormat,
    /// Optional subscription proxy-group name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// Parse rule-set lines: `目标,类型,URL`.
///
/// The type is spelled out rather than guessed from the file name because Mihomo
/// needs it and a wrong guess is a config the core refuses to load.
pub fn parse_rule_sets(text: &str) -> Result<Vec<RuleSet>, RuleError> {
    let mut sets: Vec<RuleSet> = Vec::new();

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }

        let fields: Vec<&str> = trimmed.splitn(3, ',').map(str::trim).collect();
        if fields.len() < 3 {
            return Err(RuleError {
                line,
                message: "需要三个字段：PROXY 或 DIRECT,类型(domain/ipcidr/classical),URL"
                    .to_string(),
            });
        }

        let target = match fields[0].to_ascii_uppercase().as_str() {
            "PROXY" => RuleTarget::Proxy,
            "DIRECT" => RuleTarget::Direct,
            "REJECT" | "REJECT-DROP" => RuleTarget::Reject,
            other => {
                return Err(RuleError {
                    line,
                    message: format!("目标只能是 PROXY、DIRECT 或 REJECT，收到 `{other}`"),
                })
            }
        };

        let behavior = match fields[1].to_ascii_lowercase().as_str() {
            "domain" => RuleSetBehavior::Domain,
            "ipcidr" => RuleSetBehavior::IpCidr,
            "classical" => RuleSetBehavior::Classical,
            other => {
                return Err(RuleError {
                    line,
                    message: format!("类型只能是 domain / ipcidr / classical，收到 `{other}`"),
                })
            }
        };

        let url = fields[2].to_string();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(RuleError {
                line,
                message: "URL 必须以 http:// 或 https:// 开头".to_string(),
            });
        }

        let format = match format_of(&url) {
            Some(format) => format,
            None => {
                return Err(RuleError {
                    line,
                    message: "只认 .mrs / .yaml（Mihomo）与 .srs（sing-box）结尾的规则集文件"
                        .to_string(),
                })
            }
        };

        let base = name_from(&url);
        let name = unique_name(&sets, &base, format);
        sets.push(RuleSet {
            name,
            url,
            target,
            behavior,
            format,
            group: None,
        });
    }

    Ok(sets)
}

/// The format a URL implies, from its extension.
///
/// Extensions rather than content sniffing: this runs while the user is typing,
/// and a HEAD request per keystroke is not an option.
fn format_of(url: &str) -> Option<RuleSetFormat> {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if path.ends_with(".mrs") || path.ends_with(".yaml") || path.ends_with(".yml") {
        Some(RuleSetFormat::Clash)
    } else if path.ends_with(".srs") {
        Some(RuleSetFormat::SingBox)
    } else {
        None
    }
}

/// A safe identifier from the URL's file stem.
fn name_from(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or("rules")
        .split('.')
        .next()
        .unwrap_or("rules");

    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();

    if cleaned.is_empty() {
        "rules".to_string()
    } else {
        cleaned
    }
}

/// Tags reserved by the built-in split mode rule-sets in sing-box.
const RESERVED_RULE_SET_NAMES: [&str; 2] = ["geosite-cn", "geoip-cn"];

/// Two rule-sets with the same file name must not collide: Mihomo keys providers
/// by name, and a duplicate silently replaces the first. Also avoid colliding
/// with sing-box's built-in split-mode rule-set tags.
fn unique_name(existing: &[RuleSet], base: &str, format: RuleSetFormat) -> String {
    let is_taken = |candidate: &str| {
        (format == RuleSetFormat::SingBox && RESERVED_RULE_SET_NAMES.contains(&candidate))
            || existing.iter().any(|set| set.name == candidate)
    };
    if !is_taken(base) {
        return base.to_string();
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !is_taken(&candidate) {
            return candidate;
        }
    }
    unreachable!("the loop only exits by returning")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveRuleEntry {
    pub kind: String,
    pub value: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveRuleSetEntry {
    pub name: String,
    pub format: String,
    pub behavior: String,
    pub url: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveRulesView {
    pub mode: super::RuleMode,
    pub source: String,
    pub default_target: String,
    pub custom_rules: Vec<EffectiveRuleEntry>,
    pub profile_rules: Vec<EffectiveRuleEntry>,
    pub rule_sets: Vec<EffectiveRuleSetEntry>,
    pub groups: Vec<String>,
}

fn rule_entry(rule: &CustomRule, source: &str) -> EffectiveRuleEntry {
    EffectiveRuleEntry {
        kind: rule.kind.as_str().to_string(),
        value: rule.value.clone(),
        target: rule.target.as_str().to_string(),
        group: rule.group.clone(),
        source: source.to_string(),
    }
}

fn rule_set_entry(set: &RuleSet, source: &str) -> EffectiveRuleSetEntry {
    EffectiveRuleSetEntry {
        name: set.name.clone(),
        format: set.format.as_str().to_string(),
        behavior: set.behavior.as_str().to_string(),
        url: set.url.clone(),
        target: set.target.as_str().to_string(),
        group: set.group.clone(),
        source: source.to_string(),
    }
}

/// Inspect the routing rules and rule-sets that will be compiled for `profile` + `settings`.
pub fn build_effective_rules(
    profile: Option<&crate::Profile>,
    settings: &crate::AppSettings,
) -> EffectiveRulesView {
    let options = settings.compile_options(settings.listen_port, settings.api_port);
    let custom_rules = options
        .custom_rules
        .iter()
        .map(|rule| rule_entry(rule, "custom"))
        .collect();

    let mut rule_sets: Vec<EffectiveRuleSetEntry> = options
        .rule_sets
        .iter()
        .map(|set| rule_set_entry(set, "user"))
        .collect();

    let has_sub_rules = profile.is_some_and(crate::Profile::has_subscription_rules);
    let source = if has_sub_rules {
        "subscription"
    } else {
        "builtin"
    }
    .to_string();

    let default_target = if has_sub_rules {
        profile
            .and_then(|p| p.default_target)
            .unwrap_or(RuleTarget::Proxy)
            .as_str()
    } else {
        RuleTarget::Proxy.as_str()
    }
    .to_string();

    let mut profile_rules = Vec::new();
    if let Some(p) = profile.filter(|p| p.has_subscription_rules()) {
        for set in &p.rule_sets {
            rule_sets.push(rule_set_entry(set, "subscription"));
        }
        profile_rules.extend(p.rules.iter().map(|rule| rule_entry(rule, "subscription")));
    } else {
        profile_rules.extend(
            super::builtin_rules::builtin_split_rules()
                .iter()
                .map(|rule| rule_entry(rule, "builtin")),
        );
    }

    let groups = profile
        .map(|p| {
            let included: Vec<String> = p.nodes.iter().map(|n| n.name.clone()).collect();
            super::resolved_subscription_groups(p, &included, true)
                .into_iter()
                .map(|g| g.name)
                .collect()
        })
        .unwrap_or_default();

    EffectiveRulesView {
        mode: settings.rule_mode,
        source,
        default_target,
        custom_rules,
        profile_rules,
        rule_sets,
        groups,
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
                group: None,
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
    fn supports_reject_and_extended_rule_kinds() {
        let rules = parse_rules(
            "DOMAIN,ads.example.com,REJECT\nGEOSITE,cn,DIRECT\nGEOIP,CN,DIRECT\nPROCESS-NAME,curl,DIRECT\nDST-PORT,443,PROXY",
        )
        .unwrap();
        assert_eq!(rules.len(), 5);
        assert_eq!(rules[0].target, RuleTarget::Reject);
        assert_eq!(rules[1].kind, RuleKind::GeoSite);
        assert_eq!(rules[2].kind, RuleKind::GeoIp);
        assert_eq!(rules[3].kind, RuleKind::ProcessName);
        assert_eq!(rules[4].kind, RuleKind::DstPort);
    }

    #[test]
    fn rejects_an_unknown_target() {
        let error = parse_rules("DOMAIN,x.com,UNKNOWN").unwrap_err();
        assert!(error.message.contains("PROXY"));
    }

    #[test]
    fn validates_cidr_prefix_lengths() {
        assert!(parse_rules("IP-CIDR,10.0.0.0/33,DIRECT").is_err());
        assert!(parse_rules("IP-CIDR,10.0.0.0/8,DIRECT").is_ok());
        assert!(parse_rules("IP-CIDR,not-an-ip/8,DIRECT").is_err());
        assert!(parse_rules("IP-CIDR,2001:db8::/32,DIRECT").is_ok());
    }

    #[test]
    fn rule_sets_carry_their_target_type_and_format() {
        let sets = parse_rule_sets(
            "# 国内直连\nDIRECT,domain,https://example.com/rules/geosite-cn.mrs\nPROXY,ipcidr,https://example.com/rules/geoip-us.srs",
        )
        .unwrap();

        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].name, "geosite-cn");
        assert_eq!(sets[0].target, RuleTarget::Direct);
        assert_eq!(sets[0].behavior, RuleSetBehavior::Domain);
        assert_eq!(sets[0].format, RuleSetFormat::Clash);
        assert_eq!(sets[1].name, "geoip-us");
        assert_eq!(sets[1].format, RuleSetFormat::SingBox);
    }

    /// The two families use different file formats; a URL we cannot classify is
    /// an error rather than a silent skip, because the user would otherwise
    /// wonder why their rule-set never took effect.
    #[test]
    fn rule_sets_reject_an_unknown_file_type() {
        let error = parse_rule_sets("DIRECT,domain,https://example.com/rules.txt").unwrap_err();
        assert!(error.message.contains(".mrs"), "{}", error.message);
    }

    #[test]
    fn duplicate_file_names_get_a_suffix() {
        let sets = parse_rule_sets(
            "DIRECT,domain,https://a.example.com/cn.mrs\nPROXY,domain,https://b.example.com/cn.mrs",
        )
        .unwrap();
        assert_eq!(sets[0].name, "cn");
        assert_eq!(sets[1].name, "cn-2");
    }

    #[test]
    fn rule_set_urls_must_be_http() {
        let error = parse_rule_sets("DIRECT,domain,file:///tmp/cn.mrs").unwrap_err();
        assert!(error.message.contains("http"), "{}", error.message);
    }

    #[test]
    fn built_in_split_rule_set_names_are_reserved() {
        let sets = parse_rule_sets(
            "DIRECT,domain,https://example.com/geosite-cn.srs\nDIRECT,ipcidr,https://example.com/geoip-cn.srs",
        )
        .unwrap();
        assert_eq!(sets[0].name, "geosite-cn-2");
        assert_eq!(sets[1].name, "geoip-cn-2");
    }

    #[test]
    fn effective_rules_reports_subscription_precedence_and_builtin_fallback() {
        let settings = crate::AppSettings {
            custom_rules: Some("DOMAIN-SUFFIX,corp.example.com,DIRECT".to_string()),
            ..Default::default()
        };

        let bare = crate::Profile::new("bare", Vec::new());
        let builtin_view = build_effective_rules(Some(&bare), &settings);
        assert_eq!(builtin_view.source, "builtin");
        assert_eq!(builtin_view.custom_rules.len(), 1);
        assert!(!builtin_view.profile_rules.is_empty());
        assert!(builtin_view.rule_sets.is_empty());

        let mut with_sub = bare;
        with_sub.rules.push(CustomRule {
            kind: RuleKind::DomainSuffix,
            value: "netflix.com".to_string(),
            target: RuleTarget::Proxy,
            group: Some("Streaming".to_string()),
        });
        let sub_view = build_effective_rules(Some(&with_sub), &settings);
        assert_eq!(sub_view.source, "subscription");
        assert_eq!(sub_view.profile_rules.len(), 1);
        assert_eq!(
            sub_view.profile_rules[0].group.as_deref(),
            Some("Streaming")
        );
        assert!(!sub_view.rule_sets.iter().any(|set| set.source == "builtin"));
    }
}
