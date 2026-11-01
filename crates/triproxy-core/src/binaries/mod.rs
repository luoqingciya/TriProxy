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

pub mod extract;
pub mod manifest;
pub mod version;

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::adapters::first_version_token;
use crate::core::CoreKind;
use crate::error::{Error, Result};
use crate::logging::now_ms;
use crate::paths::{self, find_on_path, Layout};
use crate::process::run_capture;

pub use manifest::{ReleaseAsset, ReleaseInfo, Target};
pub use version::{compare_versions, is_newer};

const GITHUB_API: &str = "https://api.github.com";
const USER_AGENT: &str = "TriProxy/0.1 (+https://github.com/luoqingciya/TriProxy)";
/// Optional token for GitHub API calls.
///
/// Unauthenticated API access is limited to 60 requests per hour **per IP**, and
/// CI runners share egress addresses, so a release pipeline needs a token or it
/// will start failing with 403 halfway through. `GITHUB_TOKEN` is picked up
/// automatically so the workflow does not need extra wiring.
pub const TOKEN_ENV_VARS: [&str; 2] = ["TRIPROXY_GITHUB_TOKEN", "GITHUB_TOKEN"];
/// Abort a download when the transfer makes no progress for this long.
const STALL_TIMEOUT: Duration = Duration::from_secs(15);
/// Minimum average throughput once the grace period is over.
///
/// A throttled source can trickle a few bytes per second forever: the per-chunk
/// stall timer never fires, so without a throughput floor the fallback chain
/// would sit on one bad mirror for minutes. 40 kB/s still clears a legitimate
/// download at a usable speed while rejecting the ~90 B/s direct route.
const MIN_THROUGHPUT_BYTES_PER_SEC: u64 = 40 * 1024;
/// How long a transfer may run before the throughput floor is enforced.
const THROUGHPUT_GRACE: Duration = Duration::from_secs(20);
/// Backstop ceiling for a single attempt, regardless of progress.
const MAX_DOWNLOAD_TIME: Duration = Duration::from_secs(300);
/// Environment variable that overrides the GitHub release mirror.
pub const MIRROR_ENV: &str = "TRIPROXY_GITHUB_MIRROR";
/// Fallback mirrors, tried only after a direct download fails.
///
/// Mirror use is safe because the SHA256 always comes from `api.github.com`,
/// never from the mirror: a malicious mirror can cause a failure but cannot
/// inject a different binary.
const DEFAULT_MIRRORS: [&str; 2] = ["https://ghfast.top/", "https://gh-proxy.com/"];

/// Where a resolved binary came from. Surfaced in `triproxy cores` so that an
/// unexpected `PATH` hit is immediately visible instead of silently used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinarySource {
    /// `TRIPROXY_*_BIN` environment override.
    Env,
    /// Downloaded and verified into `<data>/bin` (wins over bundled).
    Managed,
    /// Shipped inside the release archive, in `cores/` next to the executable.
    Bundled,
    /// Found on `PATH`.
    Path,
}

impl BinarySource {
    pub fn as_str(self) -> &'static str {
        match self {
            BinarySource::Env => "env",
            BinarySource::Managed => "managed",
            BinarySource::Bundled => "bundled",
            BinarySource::Path => "path",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallRecord {
    pub core: String,
    pub tag: String,
    pub version: String,
    pub asset: String,
    pub sha256: String,
    pub bytes: u64,
    pub installed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BinaryStatus {
    pub core: CoreKind,
    pub path: Option<PathBuf>,
    pub source: Option<BinarySource>,
    pub version: Option<String>,
    pub tag: Option<String>,
}

/// Outcome of comparing the installed version with the latest release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    pub core: CoreKind,
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub status: UpdateStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStatus {
    NotInstalled,
    UpToDate,
    UpdateAvailable,
}

impl UpdateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            UpdateStatus::NotInstalled => "not-installed",
            UpdateStatus::UpToDate => "up-to-date",
            UpdateStatus::UpdateAvailable => "update-available",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// Tag or bare version to install; `None` means "latest release".
    pub version: Option<String>,
    /// Reinstall even when a matching managed binary already exists.
    pub force: bool,
    /// Accept an asset with no published checksum. Off by default: the plan
    /// forbids trusting unknown artifacts (ch.35).
    pub allow_unverified: bool,
    /// Install from a local archive instead of downloading.
    ///
    /// Necessary for users whose network cannot reach GitHub release assets at
    /// all — they fetch the archive by any means and point TriProxy at it.
    pub local_file: Option<PathBuf>,
    /// Override the release mirror prefix (e.g. `https://ghfast.top/`).
    pub mirror: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InstalledBinary {
    pub core: CoreKind,
    pub path: PathBuf,
    pub version: String,
    pub tag: String,
    pub sha256: String,
    pub already_installed: bool,
}

/// Locates, downloads and verifies the three core binaries.
///
/// Binaries are deliberately *not* bundled with the installer: they are fetched
/// on first use, verified against the checksum published by the upstream
/// release, and stored under the application data directory. That keeps the
/// installer small and avoids redistributing third-party GPL/MPL builds.
pub struct BinaryManager {
    layout: Layout,
    client: reqwest::Client,
    mirror: Option<String>,
    /// Applied to `api.github.com` only — never to asset downloads, which may be
    /// routed through a third-party mirror.
    token: Option<String>,
}

impl BinaryManager {
    pub fn new(layout: Layout) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(MAX_DOWNLOAD_TIME)
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| Error::Network(format!("cannot build http client: {e}")))?;
        let mirror = std::env::var(MIRROR_ENV)
            .ok()
            .and_then(|v| normalize_mirror(&v));
        let token = TOKEN_ENV_VARS.iter().find_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
        Ok(Self {
            layout,
            client,
            mirror,
            token,
        })
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Resolve the binary to use.
    ///
    /// Order: environment override → managed install → bundled (`cores/` next to
    /// the executable) → `PATH`. Managed comes before bundled so that a user who
    /// updated a core does not silently fall back to the shipped build.
    pub fn resolve(&self, kind: CoreKind) -> Option<(PathBuf, BinarySource)> {
        if let Some(value) = std::env::var_os(kind.bin_env_var()) {
            if !value.is_empty() {
                let path = PathBuf::from(value);
                if path.is_file() {
                    return Some((path, BinarySource::Env));
                }
            }
        }

        let managed = self.layout.managed_binary(kind.exe_stem());
        if managed.is_file() {
            return Some((managed, BinarySource::Managed));
        }

        if let Some(dir) = paths::bundled_core_dir() {
            let bundled = paths::binary_in(&dir, kind.exe_stem());
            if bundled.is_file() {
                return Some((bundled, BinarySource::Bundled));
            }
        }

        find_on_path(kind.exe_stem()).map(|path| (path, BinarySource::Path))
    }

    pub fn read_record(&self, kind: CoreKind) -> Option<InstallRecord> {
        let managed = self.layout.install_record(kind.exe_stem());
        if let Ok(text) = std::fs::read_to_string(managed) {
            if let Ok(record) = serde_json::from_str(&text) {
                return Some(record);
            }
        }
        // The bundled variant ships the same record files, which is how
        // `triproxy status` can show a version without executing the core.
        let bundled = paths::bundled_core_dir()?.join(format!("{}.json", kind.exe_stem()));
        let text = std::fs::read_to_string(bundled).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn status(&self, kind: CoreKind) -> BinaryStatus {
        let resolved = self.resolve(kind);
        let record = self.read_record(kind);
        BinaryStatus {
            core: kind,
            path: resolved.as_ref().map(|(path, _)| path.clone()),
            source: resolved.map(|(_, source)| source),
            version: record.as_ref().map(|r| r.version.clone()),
            tag: record.map(|r| r.tag),
        }
    }

    /// Download, verify and install a core binary.
    /// Compare the installed version against the latest release.
    ///
    /// Network failures propagate: reporting "up to date" because GitHub could
    /// not be reached would be a lie the user acts on.
    pub async fn check_update(&self, kind: CoreKind) -> Result<UpdateCheck> {
        let installed = self.read_record(kind).map(|record| record.version);
        let release = self.fetch_release(kind, None).await?;
        let latest = release.version();

        let status = match &installed {
            None => UpdateStatus::NotInstalled,
            Some(current) if is_newer(&latest, current) => UpdateStatus::UpdateAvailable,
            Some(_) => UpdateStatus::UpToDate,
        };

        Ok(UpdateCheck {
            core: kind,
            installed,
            latest: Some(latest),
            status,
        })
    }

    pub async fn install(
        &self,
        kind: CoreKind,
        options: &InstallOptions,
    ) -> Result<InstalledBinary> {
        self.layout.ensure()?;

        if !options.force {
            if let Some(record) = self.read_record(kind) {
                let path = self.layout.managed_binary(kind.exe_stem());
                let version_matches = options
                    .version
                    .as_deref()
                    .map(|wanted| wanted.trim_start_matches('v') == record.version)
                    .unwrap_or(true);
                if path.is_file() && version_matches {
                    return Ok(InstalledBinary {
                        core: kind,
                        path,
                        version: record.version,
                        tag: record.tag,
                        sha256: record.sha256,
                        already_installed: true,
                    });
                }
            }
        }

        // Resolve the artifact: either a local archive the user supplied, or a
        // release asset downloaded with mirror fallback.
        let mut verification: Option<(ReleaseInfo, ReleaseAsset)> = None;
        let (asset_name, tag, bytes) = match &options.local_file {
            Some(path) => {
                let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("local-archive")
                    .to_string();
                let tag = match self.fetch_release(kind, options.version.as_deref()).await {
                    Ok(release) => {
                        if let Some(asset) = release
                            .assets
                            .iter()
                            .find(|a| a.name.eq_ignore_ascii_case(&name))
                            .cloned()
                        {
                            let tag = release.tag_name.clone();
                            verification = Some((release, asset));
                            tag
                        } else {
                            "local".to_string()
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            core = kind.as_str(),
                            error = %e,
                            "cannot reach GitHub to look up a checksum for the local file"
                        );
                        "local".to_string()
                    }
                };
                tracing::info!(
                    core = kind.as_str(),
                    path = %path.display(),
                    bytes = bytes.len(),
                    "installing from local archive"
                );
                (name, tag, bytes)
            }
            None => {
                let target = Target::current()?;
                let release = self.fetch_release(kind, options.version.as_deref()).await?;
                let asset = manifest::pick_asset(kind, target, &release.assets)?.clone();
                tracing::info!(
                    core = kind.as_str(),
                    tag = %release.tag_name,
                    asset = %asset.name,
                    size = asset.size,
                    "downloading core binary"
                );
                let bytes = self
                    .download_asset(&asset, options.mirror.as_deref())
                    .await?;
                let tag = release.tag_name.clone();
                verification = Some((release, asset.clone()));
                (asset.name, tag, bytes)
            }
        };

        let actual = sha256_hex(&bytes);
        match &verification {
            Some((release, asset)) => {
                self.verify(kind, release, asset, &actual, options.allow_unverified)
                    .await?
            }
            None => tracing::warn!(
                core = kind.as_str(),
                "installing a local archive without checksum verification"
            ),
        }

        let bundle = extract::extract_bundle(&asset_name, &bytes, kind.exe_stem())?;
        let binary = bundle.binary;
        if binary.is_empty() {
            return Err(Error::Other(format!(
                "`{}` produced an empty binary from {asset_name}",
                kind
            )));
        }

        let final_path = self.layout.managed_binary(kind.exe_stem());
        self.write_binary(&final_path, &binary)?;

        // Runtime data files (geosite.dat and friends) have to sit beside the
        // executable, otherwise geo-based routing fails at load time.
        if let Some(dir) = final_path.parent() {
            for (name, data) in &bundle.companions {
                let path = dir.join(name);
                std::fs::write(&path, data).map_err(|e| Error::io(&path, e))?;
                tracing::info!(file = %path.display(), bytes = data.len(), "installed companion data file");
            }
        }

        // Trust but verify: a binary that cannot even report its version is not
        // usable, and must not be left behind for the manager to pick up.
        let probe_args: Vec<OsString> = kind
            .version_probe_args()
            .iter()
            .map(OsString::from)
            .collect();
        let probe = run_capture(&final_path, &probe_args).await?;
        if !probe.success {
            let _ = std::fs::remove_file(&final_path);
            return Err(Error::Other(format!(
                "`{}` binary failed its version self-check: {}",
                kind,
                probe.combined()
            )));
        }

        let version = first_version_token(&probe.combined()).unwrap_or_else(|| {
            verification
                .as_ref()
                .map(|(release, _)| release.version())
                .unwrap_or_else(|| "unknown".to_string())
        });
        let record = InstallRecord {
            core: kind.as_str().to_string(),
            tag: tag.clone(),
            version: version.clone(),
            asset: asset_name,
            sha256: actual.clone(),
            bytes: bytes.len() as u64,
            installed_at_ms: now_ms(),
        };
        let record_path = self.layout.install_record(kind.exe_stem());
        let text = serde_json::to_string_pretty(&record)?;
        std::fs::write(&record_path, text).map_err(|e| Error::io(&record_path, e))?;

        Ok(InstalledBinary {
            core: kind,
            path: final_path,
            version,
            tag,
            sha256: actual,
            already_installed: false,
        })
    }

    async fn fetch_release(&self, kind: CoreKind, version: Option<&str>) -> Result<ReleaseInfo> {
        let url = match version {
            Some(value) => {
                let tag = if value.starts_with('v') {
                    value.to_string()
                } else {
                    format!("v{value}")
                };
                format!("{GITHUB_API}/repos/{}/releases/tags/{tag}", kind.repo())
            }
            None => format!("{GITHUB_API}/repos/{}/releases/latest", kind.repo()),
        };

        let mut request = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }

        let response = request
            .send()
            .await
            .map_err(|e| Error::Network(format!("GET {url}: {e}")))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| Error::Network(format!("GET {url}: {e}")))?;

        if !status.is_success() {
            let snippet: String = body.chars().take(200).collect();
            let hint = if status == reqwest::StatusCode::FORBIDDEN && self.token.is_none() {
                " (unauthenticated GitHub API calls are limited to 60/hour per IP; \
                 set GITHUB_TOKEN or TRIPROXY_GITHUB_TOKEN)"
            } else {
                ""
            };
            return Err(Error::Network(format!(
                "GitHub API returned {status} for {url}: {snippet}{hint}"
            )));
        }

        serde_json::from_str::<ReleaseInfo>(&body)
            .map_err(|e| Error::Network(format!("unexpected GitHub release payload: {e}")))
    }

    /// Download a release asset, trying the configured mirror, then the direct
    /// URL, then the built-in mirrors.
    async fn download_asset(
        &self,
        asset: &ReleaseAsset,
        mirror_override: Option<&str>,
    ) -> Result<Vec<u8>> {
        let mirror = mirror_override
            .and_then(normalize_mirror)
            .or_else(|| self.mirror.clone());

        let mut candidates: Vec<String> = Vec::new();
        if let Some(mirror) = &mirror {
            candidates.push(format!("{mirror}{}", asset.url));
        }
        candidates.push(asset.url.clone());
        for mirror in DEFAULT_MIRRORS {
            candidates.push(format!("{mirror}{}", asset.url));
        }
        // A configured mirror may also appear in the built-in list.
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|candidate| seen.insert(candidate.clone()));

        let mut errors = Vec::new();
        for url in candidates {
            match self.download_streaming(&url, asset.size).await {
                Ok(bytes) => return Ok(bytes),
                Err(e) => {
                    tracing::warn!(url = %url, error = %e, "download attempt failed");
                    errors.push(format!("{url}: {e}"));
                }
            }
        }

        Err(Error::Network(format!(
            "all download sources failed for `{}` — try `--mirror <prefix>` or `--file <archive>`: {}",
            asset.name,
            errors.join(" | ")
        )))
    }

    /// Stream a download with both a no-progress timeout and a total deadline.
    ///
    /// GitHub release assets are routinely throttled to a few kB/s on mainland
    /// Chinese networks. Without stall detection a single attempt can hang for
    /// minutes before failing, which makes mirror fallback useless.
    async fn download_streaming(&self, url: &str, expected_size: u64) -> Result<Vec<u8>> {
        match tokio::time::timeout(
            MAX_DOWNLOAD_TIME,
            self.download_with_stall_guard(url, expected_size),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(Error::Network(format!(
                "gave up after {}s",
                MAX_DOWNLOAD_TIME.as_secs()
            ))),
        }
    }

    async fn download_with_stall_guard(&self, url: &str, expected_size: u64) -> Result<Vec<u8>> {
        use futures_util::StreamExt;

        let response = self
            .client
            .get(url)
            .header("Accept", "application/octet-stream")
            .send()
            .await
            .map_err(|e| Error::Network(format!("request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(Error::Network(format!("HTTP {status}")));
        }

        let mut buffer: Vec<u8> = Vec::with_capacity(expected_size as usize);
        let mut stream = response.bytes_stream();
        let started = std::time::Instant::now();
        loop {
            match tokio::time::timeout(STALL_TIMEOUT, stream.next()).await {
                Err(_) => {
                    return Err(Error::Network(format!(
                        "stalled after {} bytes (no data for {}s)",
                        buffer.len(),
                        STALL_TIMEOUT.as_secs()
                    )))
                }
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    return Err(Error::Network(format!(
                        "transfer aborted after {} bytes: {e}",
                        buffer.len()
                    )))
                }
                Ok(Some(Ok(chunk))) => buffer.extend_from_slice(&chunk),
            }

            let elapsed = started.elapsed();
            if throughput_too_low(buffer.len(), elapsed) {
                return Err(Error::Network(format!(
                    "too slow: {:.0} kB/s after {}s ({} of {} bytes)",
                    buffer.len() as f64 / elapsed.as_secs_f64() / 1024.0,
                    elapsed.as_secs(),
                    buffer.len(),
                    expected_size
                )));
            }
        }

        if expected_size > 0 && buffer.len() as u64 != expected_size {
            return Err(Error::Network(format!(
                "size mismatch: expected {expected_size} bytes, received {}",
                buffer.len()
            )));
        }

        Ok(buffer)
    }

    async fn verify(
        &self,
        kind: CoreKind,
        release: &ReleaseInfo,
        asset: &ReleaseAsset,
        actual: &str,
        allow_unverified: bool,
    ) -> Result<()> {
        if let Some(digest) = &asset.digest {
            if let Some(expected) = digest.strip_prefix("sha256:") {
                return compare_digest(kind, expected, actual);
            }
        }

        if let Some(expected) = self.fetch_sidecar_checksum(release, &asset.name).await? {
            return compare_digest(kind, &expected, actual);
        }

        if allow_unverified {
            tracing::warn!(
                core = kind.as_str(),
                asset = %asset.name,
                "no published checksum found; installing unverified binary"
            );
            return Ok(());
        }

        Err(Error::ChecksumMismatch {
            core: kind.as_str().to_string(),
            reason: format!(
                "upstream published no checksum for `{}`; re-run with --allow-unverified to accept the risk",
                asset.name
            ),
        })
    }

    /// Look for a checksum sidecar published next to the asset.
    ///
    /// GitHub's per-asset `digest` field is the primary source; older releases
    /// (and Xray, which ships `.dgst` files) need this fallback.
    async fn fetch_sidecar_checksum(
        &self,
        release: &ReleaseInfo,
        asset_name: &str,
    ) -> Result<Option<String>> {
        let candidates = [
            format!("{asset_name}.sha256"),
            format!("{asset_name}.dgst"),
            "checksums.txt".to_string(),
            "sha256sums.txt".to_string(),
        ];

        for candidate in candidates {
            let Some(sidecar) = release
                .assets
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case(&candidate))
            else {
                continue;
            };
            let text = match self.download_streaming(&sidecar.url, sidecar.size).await {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) => {
                    tracing::warn!(sidecar = %sidecar.name, error = %e, "checksum sidecar download failed");
                    continue;
                }
            };
            if let Some(hex) = parse_checksum_text(&text, asset_name) {
                return Ok(Some(hex));
            }
        }

        Ok(None)
    }

    fn write_binary(&self, path: &std::path::Path, bytes: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }

        // Stage next to the destination, then rename: a half-written binary must
        // never be visible to `resolve()`.
        let staging = path.with_extension("staging");
        std::fs::write(&staging, bytes).map_err(|e| Error::io(&staging, e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&staging, permissions).map_err(|e| Error::io(&staging, e))?;
        }

        if path.exists() {
            std::fs::remove_file(path).map_err(|e| {
                Error::Other(format!(
                    "cannot replace {}: {e} (is the core still running?)",
                    path.display()
                ))
            })?;
        }
        std::fs::rename(&staging, path).map_err(|e| Error::io(path, e))?;
        Ok(())
    }
}

/// Should this transfer be abandoned as unusably slow?
///
/// Judged on average throughput rather than instantaneous progress: a throttled
/// route can keep dribbling bytes forever, which keeps a "no data" timer happy.
fn throughput_too_low(bytes: usize, elapsed: Duration) -> bool {
    if elapsed <= THROUGHPUT_GRACE {
        return false;
    }
    let rate = bytes as f64 / elapsed.as_secs_f64();
    rate < MIN_THROUGHPUT_BYTES_PER_SEC as f64
}

/// Accept `ghfast.top`, `https://ghfast.top` and `https://ghfast.top/` alike.
fn normalize_mirror(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    Some(if with_scheme.ends_with('/') {
        with_scheme
    } else {
        format!("{with_scheme}/")
    })
}

fn compare_digest(kind: CoreKind, expected: &str, actual: &str) -> Result<()> {
    if expected.eq_ignore_ascii_case(actual) {
        return Ok(());
    }
    Err(Error::ChecksumMismatch {
        core: kind.as_str().to_string(),
        reason: format!("expected sha256 {expected}, got {actual}"),
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Extract the sha256 of `asset_name` from a checksum file.
///
/// Handles the three formats seen in the wild:
/// - `sha256sum` style: `<hex>  <file>`
/// - Xray `.dgst` style: `SHA256 = <hex>`
/// - single-hash files: just `<hex>`
fn parse_checksum_text(text: &str, asset_name: &str) -> Option<String> {
    let mut fallback = None;
    for line in text.lines() {
        let Some(hex) = line
            .split(|c: char| !c.is_ascii_hexdigit())
            .find(|token| token.len() == 64)
        else {
            continue;
        };
        if line.contains(asset_name) {
            return Some(hex.to_ascii_lowercase());
        }
        fallback.get_or_insert_with(|| hex.to_ascii_lowercase());
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256sum_style_checksum_is_parsed() {
        let text = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  other.zip
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  Xray-windows-64.zip
";
        assert_eq!(
            parse_checksum_text(text, "Xray-windows-64.zip").as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
    }

    #[test]
    fn xray_dgst_style_checksum_is_parsed() {
        let text = "\
MD5 = 0123456789abcdef0123456789abcdef
SHA1 = 0123456789abcdef0123456789abcdef01234567
SHA256 = cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
SHA512 = 0123456789abcdef
";
        assert_eq!(
            parse_checksum_text(text, "whatever.zip").as_deref(),
            Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
        );
    }

    #[test]
    fn digest_comparison_is_case_insensitive() {
        let expected = "ABCdef0000000000000000000000000000000000000000000000000000000000";
        assert!(compare_digest(CoreKind::Xray, expected, &expected.to_ascii_lowercase()).is_ok());
        assert!(compare_digest(CoreKind::Xray, expected, "deadbeef").is_err());
    }

    #[test]
    fn throughput_guard_rejects_a_trickling_source() {
        // The measured direct-route behaviour: ~90 B/s for a minute.
        assert!(throughput_too_low(90 * 60, Duration::from_secs(60)));
        // A usable mirror: ~200 kB/s.
        assert!(!throughput_too_low(
            200 * 1024 * 60,
            Duration::from_secs(60)
        ));
        // Inside the grace period nothing is judged yet.
        assert!(!throughput_too_low(1024, Duration::from_secs(5)));
    }

    #[test]
    fn mirror_prefix_is_normalized() {
        assert_eq!(
            normalize_mirror("ghfast.top").as_deref(),
            Some("https://ghfast.top/")
        );
        assert_eq!(
            normalize_mirror("https://gh-proxy.com").as_deref(),
            Some("https://gh-proxy.com/")
        );
        assert_eq!(
            normalize_mirror("  http://127.0.0.1:8080/  ").as_deref(),
            Some("http://127.0.0.1:8080/")
        );
        assert_eq!(normalize_mirror("   "), None);
    }

    #[test]
    fn sha256_of_known_input() {
        // sha256("abc")
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
