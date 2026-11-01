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

//! Smart Core Selector — which core should serve this profile.
//!
//! The plan (ch.13) proposes a weighted score: compatibility 50%, feature match
//! 25%, performance 15%, user preference 10%. That formula is deliberately not
//! implemented:
//!
//! - **Compatibility cannot be a weight.** A core that cannot run the profile is
//!   not "90% compatible", it is unusable. Hard elimination runs first, so every
//!   surviving candidate scores 100% and the 50% weight contributes nothing.
//! - **Performance has no measurement behind it.** A 15% weight for numbers
//!   nobody collected would make the output look precise while being arbitrary.
//!
//! What is implemented is a lexicographic ranking over facts we actually have,
//! so the outcome fits in one sentence (plan ch.23):
//!
//! 1. hard compatibility (eliminates)
//! 2. how many of the profile's optional features the core implements natively
//! 3. a documented default order
//!
//! The result is deterministic: the same inputs always select the same core.

use serde::{Deserialize, Serialize};

use crate::capability::{CoreCapability, ProfileRequirement};
use crate::core::CoreKind;
use crate::profile::Profile;

/// Requirements that come from the application, not from the profile.
///
/// A profile of nodes says nothing about whether the user wants TUN, so without
/// this the selector could never eliminate a core on TUN grounds — Xray has no
/// native TUN and would always look equally suitable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkRequirements {
    pub tun: bool,
    pub dns: bool,
    pub rule_set: bool,
    pub proxy_groups: bool,
}

impl NetworkRequirements {
    /// Fold these into a profile requirement.
    pub fn apply_to(self, requirement: &mut ProfileRequirement) {
        requirement.requires_tun |= self.tun;
        requirement.requires_dns |= self.dns;
        requirement.requires_rule_set |= self.rule_set;
        requirement.requires_proxy_groups |= self.proxy_groups;
    }
}

/// The user's core preference (plan ch.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CorePreference {
    /// Let the selector decide.
    #[default]
    Smart,
    /// The user picked a core; it is still checked for compatibility.
    Fixed(CoreKind),
}

/// Default preference order, used only to break ties.
///
/// It follows the plan's own positioning (ch.10) rather than invented benchmark
/// numbers: sing-box is the general-purpose modern core, Mihomo exists for the
/// Clash ecosystem, Xray for the V2Ray/Xray ecosystem. When a profile needs no
/// specific ecosystem, the general-purpose core is the safer default.
const DEFAULT_ORDER: [CoreKind; 3] = [CoreKind::SingBox, CoreKind::Mihomo, CoreKind::Xray];

/// One candidate's evaluation, kept even when it loses so the UI can explain why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreEvaluation {
    pub core: CoreKind,
    pub eligible: bool,
    /// Requirements this core cannot meet. Empty when eligible.
    pub unmet: Vec<String>,
    /// Profile features this core implements natively (not merely adequately).
    pub native_features: Vec<String>,
    /// Position in the default order; lower wins a tie.
    pub rank: usize,
}

impl CoreEvaluation {
    fn new(core: CoreKind, rank: usize) -> Self {
        Self {
            core,
            eligible: true,
            unmet: Vec::new(),
            native_features: Vec::new(),
            rank,
        }
    }

    fn summary(&self) -> String {
        if !self.eligible {
            format!("missing {}", self.unmet.join(", "))
        } else if self.native_features.is_empty() {
            "no specialisation needed".to_string()
        } else {
            format!("native {}", self.native_features.join(", "))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub selected: CoreKind,
    /// All three cores, best first.
    pub evaluations: Vec<CoreEvaluation>,
    /// One-sentence justification, safe to show in the UI.
    pub reason: String,
    /// True when the user picked the core and it was merely validated.
    pub overridden: bool,
}

impl Selection {
    pub fn evaluation(&self, core: CoreKind) -> Option<&CoreEvaluation> {
        self.evaluations.iter().find(|entry| entry.core == core)
    }

    /// Multi-line "why this core" report (plan ch.23).
    pub fn explain(&self) -> String {
        let mut out = String::new();
        out.push_str("Smart Engine Analysis\n\n");
        for evaluation in &self.evaluations {
            let marker = if evaluation.core == self.selected {
                "selected"
            } else if evaluation.eligible {
                "eligible"
            } else {
                "ineligible"
            };
            out.push_str(&format!(
                "  {:<9} {:<11} {}\n",
                evaluation.core.as_str(),
                marker,
                evaluation.summary()
            ));
        }
        out.push_str(&format!("\nReason: {}\n", self.reason));
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectError {
    /// No core can serve the profile.
    NoEligibleCore { evaluations: Vec<CoreEvaluation> },
    /// The user's explicit choice cannot serve the profile.
    OverrideIneligible { core: CoreKind, unmet: Vec<String> },
}

impl std::fmt::Display for SelectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectError::NoEligibleCore { evaluations } => {
                let detail = evaluations
                    .iter()
                    .filter(|evaluation| !evaluation.eligible)
                    .map(|evaluation| {
                        format!("{} ({})", evaluation.core, evaluation.unmet.join(", "))
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "no core can serve this profile: {detail}")
            }
            SelectError::OverrideIneligible { core, unmet } => write!(
                f,
                "`{core}` cannot serve this profile: {}",
                unmet.join(", ")
            ),
        }
    }
}

impl std::error::Error for SelectError {}

/// The selector itself. Stateless apart from the user's preference.
#[derive(Debug, Clone, Copy, Default)]
pub struct Selector {
    preference: CorePreference,
}

impl Selector {
    pub fn smart() -> Self {
        Self {
            preference: CorePreference::Smart,
        }
    }

    pub fn with_preference(preference: CorePreference) -> Self {
        Self { preference }
    }

    pub fn preference(&self) -> CorePreference {
        self.preference
    }

    /// Requirements for this profile, with application settings folded in.
    pub fn requirement(profile: &Profile, network: NetworkRequirements) -> ProfileRequirement {
        let mut requirement = profile.requirement();
        network.apply_to(&mut requirement);
        requirement
    }

    /// Evaluate every core for this profile.
    ///
    /// Pure and I/O-free: no binaries are inspected and no config is compiled.
    /// Whether the winner can actually produce a usable config is a separate
    /// question, answered by `compiler::compile`.
    pub fn evaluate(&self, profile: &Profile, network: NetworkRequirements) -> Vec<CoreEvaluation> {
        let requirement = Self::requirement(profile, network);

        let mut evaluations: Vec<CoreEvaluation> = DEFAULT_ORDER
            .iter()
            .enumerate()
            .map(|(rank, core)| {
                let capability = CoreCapability::for_core(*core);
                let mut evaluation = CoreEvaluation::new(*core, rank);

                evaluation.unmet = requirement.unmet_by(&capability);
                evaluation.eligible = evaluation.unmet.is_empty();
                evaluation.native_features = native_features(&requirement, &capability);

                evaluation
            })
            .collect();

        // Stable, explicit ordering: eligible first, then native feature count,
        // then the documented default order.
        evaluations.sort_by(|a, b| {
            b.eligible
                .cmp(&a.eligible)
                .then(b.native_features.len().cmp(&a.native_features.len()))
                .then(a.rank.cmp(&b.rank))
        });
        evaluations
    }

    /// Pick a core, honouring an explicit user choice.
    pub fn select(
        &self,
        profile: &Profile,
        network: NetworkRequirements,
    ) -> Result<Selection, SelectError> {
        let evaluations = self.evaluate(profile, network);

        if let CorePreference::Fixed(core) = self.preference {
            let evaluation = evaluations
                .iter()
                .find(|entry| entry.core == core)
                .expect("every core is evaluated");
            if !evaluation.eligible {
                return Err(SelectError::OverrideIneligible {
                    core,
                    unmet: evaluation.unmet.clone(),
                });
            }
            return Ok(Selection {
                selected: core,
                evaluations,
                reason: format!("`{core}` was chosen manually and can serve this profile"),
                overridden: true,
            });
        }

        let winner = evaluations
            .iter()
            .find(|entry| entry.eligible)
            .ok_or_else(|| SelectError::NoEligibleCore {
                evaluations: evaluations.clone(),
            })?;

        let mut reason = if winner.native_features.is_empty() {
            format!(
                "{} is the default general-purpose core; no candidate needed a specialisation",
                winner.core
            )
        } else {
            format!(
                "{} implements {} natively and outranks the alternatives",
                winner.core,
                winner.native_features.join(" + ")
            )
        };

        // Say who was ruled out, not just who won: "xray" quietly disappearing
        // from the list is exactly the kind of decision that needs explaining.
        let eliminated: Vec<String> = evaluations
            .iter()
            .filter(|entry| !entry.eligible)
            .map(|entry| format!("{} ({})", entry.core, entry.unmet.join(", ")))
            .collect();
        if !eliminated.is_empty() {
            reason.push_str(&format!("; eliminated: {}", eliminated.join(", ")));
        }

        Ok(Selection {
            selected: winner.core,
            evaluations,
            reason,
            overridden: false,
        })
    }
}

/// Features the profile asks for that this core has a first-class concept for.
///
/// Only differentiating features are counted. Reality and Vision are supported by
/// all three cores, so they would add the same constant to every candidate and
/// carry no information.
fn native_features(requirement: &ProfileRequirement, capability: &CoreCapability) -> Vec<String> {
    let mut features = Vec::new();
    if requirement.requires_rule_set && capability.supports_rule_set {
        features.push("rule-set".to_string());
    }
    if requirement.requires_proxy_groups && capability.supports_proxy_groups {
        features.push("proxy-groups".to_string());
    }
    features
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Protocol;
    use crate::profile::{Credentials, Node, TlsConfig, Transport};

    fn trojan(server: &str) -> Node {
        let mut node = Node {
            id: String::new(),
            name: format!("t-{server}"),
            protocol: Protocol::Trojan,
            server: server.to_string(),
            port: 443,
            credentials: Credentials::Trojan {
                password: "pw".to_string(),
            },
            transport: Transport::Tcp,
            tls: Some(TlsConfig::default()),
            udp: true,
            plugin: None,
        };
        node.refresh_id();
        node
    }

    fn profile() -> Profile {
        Profile::new("p", vec![trojan("a.example.com")])
    }

    const PLAIN: NetworkRequirements = NetworkRequirements {
        tun: false,
        dns: false,
        rule_set: false,
        proxy_groups: false,
    };

    #[test]
    fn plain_profile_selects_the_general_purpose_core() {
        let selection = Selector::smart().select(&profile(), PLAIN).unwrap();
        assert_eq!(selection.selected, CoreKind::SingBox);
        assert!(!selection.overridden);
        assert!(selection.reason.contains("sing-box"));
    }

    #[test]
    fn every_core_is_evaluated_so_the_ui_can_explain_losses() {
        let selection = Selector::smart().select(&profile(), PLAIN).unwrap();
        assert_eq!(selection.evaluations.len(), 3);
        assert!(selection.evaluations.iter().all(|entry| entry.eligible));
        assert!(selection.evaluation(CoreKind::Xray).is_some());
    }

    #[test]
    fn selection_is_deterministic() {
        let first = Selector::smart().select(&profile(), PLAIN).unwrap();
        for _ in 0..10 {
            assert_eq!(Selector::smart().select(&profile(), PLAIN).unwrap(), first);
        }
    }

    #[test]
    fn tun_requirement_eliminates_xray() {
        let network = NetworkRequirements {
            tun: true,
            ..Default::default()
        };
        let selection = Selector::smart().select(&profile(), network).unwrap();

        let xray = selection.evaluation(CoreKind::Xray).unwrap();
        assert!(!xray.eligible);
        assert_eq!(xray.unmet, vec!["TUN".to_string()]);
        assert_ne!(selection.selected, CoreKind::Xray);
        assert!(selection.explain().contains("ineligible"));
    }

    #[test]
    fn reason_names_the_eliminated_cores() {
        let network = NetworkRequirements {
            tun: true,
            ..Default::default()
        };
        let selection = Selector::smart().select(&profile(), network).unwrap();
        assert!(
            selection.reason.contains("eliminated: xray (TUN)"),
            "{}",
            selection.reason
        );
    }

    #[test]
    fn tun_requirement_leaves_singbox_and_mihomo() {
        let network = NetworkRequirements {
            tun: true,
            ..Default::default()
        };
        let evaluations = Selector::smart().evaluate(&profile(), network);
        let eligible: Vec<CoreKind> = evaluations
            .iter()
            .filter(|entry| entry.eligible)
            .map(|entry| entry.core)
            .collect();
        assert_eq!(eligible, vec![CoreKind::SingBox, CoreKind::Mihomo]);
    }

    #[test]
    fn rule_set_requirement_favours_singbox() {
        let network = NetworkRequirements {
            rule_set: true,
            proxy_groups: true,
            ..Default::default()
        };
        let selection = Selector::smart().select(&profile(), network).unwrap();

        assert_eq!(selection.selected, CoreKind::SingBox);
        assert_eq!(
            selection
                .evaluation(CoreKind::SingBox)
                .unwrap()
                .native_features,
            vec!["rule-set".to_string(), "proxy-groups".to_string()]
        );
        assert!(selection.reason.contains("rule-set"));
    }

    #[test]
    fn fixed_preference_wins_when_compatible() {
        let selector = Selector::with_preference(CorePreference::Fixed(CoreKind::Xray));
        let selection = selector.select(&profile(), PLAIN).unwrap();
        assert_eq!(selection.selected, CoreKind::Xray);
        assert!(selection.overridden);
    }

    #[test]
    fn fixed_preference_is_rejected_when_incompatible() {
        let selector = Selector::with_preference(CorePreference::Fixed(CoreKind::Xray));
        let network = NetworkRequirements {
            tun: true,
            ..Default::default()
        };
        let error = selector.select(&profile(), network).unwrap_err();
        match error {
            SelectError::OverrideIneligible { core, unmet } => {
                assert_eq!(core, CoreKind::Xray);
                assert_eq!(unmet, vec!["TUN".to_string()]);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn no_eligible_core_is_reported_with_every_gap() {
        // TUN + WireGuard-style requirement that no core satisfies is hard to
        // build today, so drive the error through a synthetic evaluation list.
        let evaluations = vec![CoreEvaluation {
            core: CoreKind::Xray,
            eligible: false,
            unmet: vec!["TUN".to_string()],
            native_features: Vec::new(),
            rank: 2,
        }];
        let error = SelectError::NoEligibleCore { evaluations };
        assert!(error.to_string().contains("xray"));
        assert!(error.to_string().contains("TUN"));
    }

    #[test]
    fn native_features_break_ties() {
        let requirement = ProfileRequirement {
            requires_rule_set: true,
            requires_proxy_groups: true,
            ..Default::default()
        };
        let singbox = native_features(&requirement, &CoreCapability::for_core(CoreKind::SingBox));
        let mihomo = native_features(&requirement, &CoreCapability::for_core(CoreKind::Mihomo));
        let xray = native_features(&requirement, &CoreCapability::for_core(CoreKind::Xray));

        assert_eq!(singbox, vec!["rule-set", "proxy-groups"]);
        assert_eq!(mihomo, vec!["proxy-groups"]);
        assert!(xray.is_empty());
    }

    #[test]
    fn explain_report_lists_every_core_and_the_reason() {
        let selection = Selector::smart().select(&profile(), PLAIN).unwrap();
        let report = selection.explain();
        assert!(report.contains("Smart Engine Analysis"));
        assert!(report.contains("sing-box"));
        assert!(report.contains("mihomo"));
        assert!(report.contains("xray"));
        assert!(report.contains("Reason:"));
        assert!(report.contains("selected"));
    }

    #[test]
    fn vision_and_reality_do_not_count_as_native_features() {
        // All three cores support them, so counting them would add the same
        // constant to every candidate and carry no information.
        let mut node = trojan("a.example.com");
        node.protocol = Protocol::Vless;
        node.credentials = Credentials::Vless {
            uuid: "11111111-2222-3333-4444-555555555555".to_string(),
            flow: crate::profile::VlessFlow::Vision,
        };
        let profile = Profile::new("p", vec![node]);
        assert!(profile.requirement().requires_vision);

        let selection = Selector::smart().select(&profile, PLAIN).unwrap();
        assert!(selection
            .evaluations
            .iter()
            .all(|entry| entry.native_features.is_empty()));
    }
}
