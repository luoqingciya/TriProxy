use serde::Deserialize;

use crate::core::CoreKind;
use crate::error::{Error, Result};

/// Platform identity used for release asset selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// `windows` | `linux` | `macos`
    pub os: &'static str,
    /// `x86_64` | `aarch64`
    pub arch: &'static str,
}

impl Target {
    pub fn current() -> Result<Self> {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let target = match (os, arch) {
            ("windows", "x86_64") => Target {
                os: "windows",
                arch: "x86_64",
            },
            ("windows", "aarch64") => Target {
                os: "windows",
                arch: "aarch64",
            },
            ("linux", "x86_64") => Target {
                os: "linux",
                arch: "x86_64",
            },
            ("linux", "aarch64") => Target {
                os: "linux",
                arch: "aarch64",
            },
            ("macos", "x86_64") => Target {
                os: "macos",
                arch: "x86_64",
            },
            ("macos", "aarch64") => Target {
                os: "macos",
                arch: "aarch64",
            },
            _ => {
                return Err(Error::Other(format!(
                    "unsupported host platform: {os}/{arch}"
                )))
            }
        };
        Ok(target)
    }

    pub fn label(&self) -> String {
        format!("{}-{}", self.os, self.arch)
    }
}

/// A downloadable release asset as reported by the GitHub releases API.
#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub url: String,
    #[serde(default)]
    pub size: u64,
    /// `sha256:<hex>` on releases published after GitHub added asset digests.
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

impl ReleaseInfo {
    /// Version without the leading `v` (sing-box and Xray asset names use this form).
    pub fn version(&self) -> String {
        self.tag_name.trim_start_matches('v').to_string()
    }
}

/// Score a release asset for a given core and target.
///
/// The three projects name their assets completely differently and the naming
/// drifts between releases, so instead of exact-match templates this scores
/// candidates on required tokens plus a preference bonus. `None` means "not a
/// candidate for this target".
///
/// Required tokens per core (verified against current releases):
/// - Mihomo: `mihomo-{os}-{arch}[-compatible|-v3]-{tag}.{zip|gz}`
/// - sing-box: `sing-box-{version}-{os}-{arch}.{zip|tar.gz}`
/// - Xray: `Xray-{windows|linux|macos}-{64|arm64-v8a}.zip`
pub fn score_asset(kind: CoreKind, target: Target, name: &str) -> Option<i32> {
    let n = name.to_ascii_lowercase();
    let arch_token = match target.arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => return None,
    };

    match kind {
        CoreKind::Mihomo => {
            if !n.starts_with("mihomo") {
                return None;
            }
            let os_token = match target.os {
                "windows" => "windows",
                "linux" => "linux",
                "macos" => "darwin",
                _ => return None,
            };
            if !n.contains(os_token) || !n.contains(arch_token) {
                return None;
            }
            match target.os {
                "windows" => {
                    if !n.ends_with(".zip") {
                        return None;
                    }
                }
                _ => {
                    if !n.ends_with(".gz") || n.ends_with(".tar.gz") {
                        return None;
                    }
                }
            }
            let mut score = 0;
            // Prefer the `compatible` build: it runs on older CPUs, and a client
            // should never fail to start because the user lacks AVX2.
            if n.contains("compatible") {
                score += 30;
            }
            if n.contains("-v3") {
                score += 20;
            }
            if n.contains("-v2") {
                score += 10;
            }
            Some(score)
        }
        CoreKind::SingBox => {
            if !n.starts_with("sing-box") {
                return None;
            }
            // sing-box publishes a `legacy-windows-7` variant next to the normal
            // build. It sorts first in some releases and would otherwise win the
            // tie, silently shipping a Windows 7 compatibility build to everyone.
            if n.contains("legacy") {
                return None;
            }
            let os_token = match target.os {
                "windows" => "windows",
                "linux" => "linux",
                "macos" => "darwin",
                _ => return None,
            };
            if !n.contains(&format!("{os_token}-{arch_token}")) {
                return None;
            }
            match target.os {
                "windows" => {
                    if !n.ends_with(".zip") {
                        return None;
                    }
                }
                _ => {
                    if !n.ends_with(".tar.gz") {
                        return None;
                    }
                }
            }
            Some(0)
        }
        CoreKind::Xray => {
            if !n.starts_with("xray-") || !n.ends_with(".zip") {
                return None;
            }
            let os_token = match target.os {
                "windows" => "windows",
                "linux" => "linux",
                "macos" => "macos",
                _ => return None,
            };
            if !n.contains(os_token) {
                return None;
            }
            if target.arch == "aarch64" {
                if !n.contains("arm64") {
                    return None;
                }
            } else {
                if n.contains("arm64") {
                    return None;
                }
                if !(n.contains("-64") || n.contains("amd64")) {
                    return None;
                }
            }
            Some(0)
        }
    }
}

/// Pick the best matching asset for a core and target.
pub fn pick_asset(
    kind: CoreKind,
    target: Target,
    assets: &[ReleaseAsset],
) -> Result<&ReleaseAsset> {
    let mut best: Option<(i32, &ReleaseAsset)> = None;
    for asset in assets {
        if let Some(score) = score_asset(kind, target, &asset.name) {
            match best {
                Some((best_score, _)) if best_score >= score => {}
                _ => best = Some((score, asset)),
            }
        }
    }
    best.map(|(_, asset)| asset).ok_or_else(|| {
        let available = assets
            .iter()
            .map(|a| a.name.as_str())
            .take(10)
            .collect::<Vec<_>>()
            .join(", ");
        Error::NoMatchingAsset {
            core: kind.as_str().to_string(),
            target: target.label(),
            available,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.to_string(),
            url: format!("https://example.invalid/{name}"),
            size: 1,
            digest: None,
        }
    }

    const WIN64: Target = Target {
        os: "windows",
        arch: "x86_64",
    };

    #[test]
    fn mihomo_prefers_compatible_build() {
        let assets = vec![
            asset("mihomo-windows-amd64-v3-v1.19.12.zip"),
            asset("mihomo-windows-amd64-compatible-v1.19.12.zip"),
            asset("mihomo-windows-arm64-v1.19.12.zip"),
            asset("mihomo-darwin-arm64-v1.19.12.gz"),
        ];
        let picked = pick_asset(CoreKind::Mihomo, WIN64, &assets).unwrap();
        assert_eq!(picked.name, "mihomo-windows-amd64-compatible-v1.19.12.zip");
    }

    #[test]
    fn singbox_matches_platform_and_arch() {
        let assets = vec![
            asset("sing-box-1.11.15-windows-amd64.zip"),
            asset("sing-box-1.11.15-windows-arm64.zip"),
            asset("sing-box-1.11.15-linux-amd64.tar.gz"),
        ];
        let picked = pick_asset(CoreKind::SingBox, WIN64, &assets).unwrap();
        assert_eq!(picked.name, "sing-box-1.11.15-windows-amd64.zip");
    }

    #[test]
    fn singbox_rejects_the_windows7_legacy_build() {
        let assets = vec![
            asset("sing-box-1.14.4-windows-amd64-legacy-windows-7.zip"),
            asset("sing-box-1.14.4-windows-amd64.zip"),
        ];
        let picked = pick_asset(CoreKind::SingBox, WIN64, &assets).unwrap();
        assert_eq!(picked.name, "sing-box-1.14.4-windows-amd64.zip");
    }

    #[test]
    fn xray_does_not_confuse_arm64_with_64() {
        let assets = vec![
            asset("Xray-windows-64.zip"),
            asset("Xray-windows-arm64-v8a.zip"),
        ];
        let picked = pick_asset(CoreKind::Xray, WIN64, &assets).unwrap();
        assert_eq!(picked.name, "Xray-windows-64.zip");

        let arm = Target {
            os: "windows",
            arch: "aarch64",
        };
        let picked = pick_asset(CoreKind::Xray, arm, &assets).unwrap();
        assert_eq!(picked.name, "Xray-windows-arm64-v8a.zip");
    }

    #[test]
    fn missing_target_reports_available_assets() {
        let assets = vec![asset("sing-box-1.11.15-linux-amd64.tar.gz")];
        let err = pick_asset(CoreKind::SingBox, WIN64, &assets).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("windows-x86_64"));
        assert!(text.contains("linux-amd64"));
    }

    #[test]
    fn digests_and_sidecars_are_optional_fields() {
        let json = r#"{"tag_name":"v1.2.3","assets":[{"name":"a.zip","browser_download_url":"u","size":7,"digest":"sha256:abc"}]}"#;
        let release: ReleaseInfo = serde_json::from_str(json).unwrap();
        assert_eq!(release.version(), "1.2.3");
        assert_eq!(release.assets[0].digest.as_deref(), Some("sha256:abc"));

        let json = r#"{"tag_name":"v1.2.3","assets":[{"name":"a.zip","browser_download_url":"u"}]}"#;
        let release: ReleaseInfo = serde_json::from_str(json).unwrap();
        assert!(release.assets[0].digest.is_none());
    }
}
