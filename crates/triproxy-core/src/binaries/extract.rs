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

use std::io::{Cursor, Read};

use crate::error::{Error, Result};

/// Data files a core loads at runtime, if the release archive carries them.
///
/// Xray's `geosite:cn` / `geoip:cn` routing only works when `geosite.dat` and
/// `geoip.dat` sit next to the executable. They are inside the official archive,
/// so dropping them during install silently disables a whole feature — the config
/// is accepted, then fails at load time with "failed to open geosite.dat".
const COMPANION_FILES: [&str; 4] = [
    "geosite.dat",
    "geoip.dat",
    "geoip-only-cn-private.dat",
    "country.mmdb",
];

/// A core executable plus the data files it expects to find beside it.
#[derive(Debug, Clone)]
pub struct ExtractedBundle {
    pub binary: Vec<u8>,
    pub companions: Vec<(String, Vec<u8>)>,
}

/// Extract the core executable out of a release archive.
///
/// The three projects ship three different container formats:
/// - Mihomo on Windows: `.zip`, on Linux/macOS: a gzipped raw binary (`.gz`)
/// - sing-box: `.zip` on Windows, `.tar.gz` elsewhere
/// - Xray: `.zip` everywhere
pub fn extract_binary(asset_name: &str, bytes: &[u8], stem: &str) -> Result<Vec<u8>> {
    Ok(extract_bundle(asset_name, bytes, stem)?.binary)
}

/// Extract the executable and any runtime data files from a release archive.
pub fn extract_bundle(asset_name: &str, bytes: &[u8], stem: &str) -> Result<ExtractedBundle> {
    let name = asset_name.to_ascii_lowercase();
    if name.ends_with(".zip") {
        extract_from_zip(bytes, stem)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_from_tar_gz(bytes, stem)
    } else if name.ends_with(".gz") {
        Ok(ExtractedBundle {
            binary: extract_from_gz(bytes)?,
            companions: Vec::new(),
        })
    } else {
        Ok(ExtractedBundle {
            binary: bytes.to_vec(),
            companions: Vec::new(),
        })
    }
}

fn is_companion(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    COMPANION_FILES.contains(&lower.as_str())
}

/// Does this archive entry look like the core executable?
///
/// Exact stems are not enough: releases ship platform-prefixed names such as
/// `mihomo-windows-amd64-compatible.exe` and
/// `sing-box-1.11.15-windows-amd64.exe` inside their archives.
fn is_target_file(file_name: &str, stem: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let stem = stem.to_ascii_lowercase();

    let (base, extension) = match lower.rsplit_once('.') {
        Some((base, extension)) => (base, Some(extension)),
        None => (lower.as_str(), None),
    };
    if !matches!(extension, None | Some("exe")) {
        return false;
    }

    base == stem || base.starts_with(&format!("{stem}-"))
}

fn extract_from_zip(bytes: &[u8], stem: &str) -> Result<ExtractedBundle> {
    let reader = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| Error::Other(format!("invalid zip archive: {e}")))?;

    let names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();
    let target = names
        .iter()
        .find(|name| {
            name.rsplit(['/', '\\'])
                .next()
                .map(|file_name| is_target_file(file_name, stem))
                .unwrap_or(false)
        })
        .cloned()
        .ok_or_else(|| {
            Error::Other(format!(
                "`{stem}` not found in zip archive (contains: {})",
                names.iter().take(8).cloned().collect::<Vec<_>>().join(", ")
            ))
        })?;

    let binary = read_zip_entry(&mut archive, &target)?;

    // Runtime data files live in the same archive; collect them while it is open.
    let mut companions = Vec::new();
    let companion_names: Vec<String> = names
        .iter()
        .filter(|name| {
            name.rsplit(['/', '\\'])
                .next()
                .map(is_companion)
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    for name in companion_names {
        let file_name = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_string();
        let data = read_zip_entry(&mut archive, &name)?;
        companions.push((file_name, data));
    }

    Ok(ExtractedBundle { binary, companions })
}

fn read_zip_entry(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let mut file = archive
        .by_name(name)
        .map_err(|e| Error::Other(format!("cannot read `{name}` from zip: {e}")))?;
    let mut buffer = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut buffer)
        .map_err(|e| Error::Other(format!("cannot extract `{name}`: {e}")))?;
    Ok(buffer)
}

fn extract_from_tar_gz(bytes: &[u8], stem: &str) -> Result<ExtractedBundle> {
    let decoder = flate2::read::GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|e| Error::Other(format!("invalid tar archive: {e}")))?;

    let mut binary: Option<Vec<u8>> = None;
    let mut companions = Vec::new();

    for entry in entries {
        let mut entry = entry.map_err(|e| Error::Other(format!("invalid tar entry: {e}")))?;
        let path = entry
            .path()
            .map_err(|e| Error::Other(format!("invalid tar path: {e}")))?
            .to_path_buf();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let file_name = file_name.to_string();

        // Keep scanning after the binary is found: the data files can come
        // after it in the archive, and stopping early would silently drop them.
        if binary.is_none() && is_target_file(&file_name, stem) {
            let mut buffer = Vec::new();
            entry
                .read_to_end(&mut buffer)
                .map_err(|e| Error::Other(format!("cannot extract `{stem}`: {e}")))?;
            binary = Some(buffer);
        } else if is_companion(&file_name) {
            let mut buffer = Vec::new();
            entry
                .read_to_end(&mut buffer)
                .map_err(|e| Error::Other(format!("cannot extract `{file_name}`: {e}")))?;
            companions.push((file_name, buffer));
        }
    }

    match binary {
        Some(binary) => Ok(ExtractedBundle { binary, companions }),
        None => Err(Error::Other(format!(
            "`{stem}` not found in tar.gz archive"
        ))),
    }
}

fn extract_from_gz(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = flate2::read::GzDecoder::new(Cursor::new(bytes));
    let mut buffer = Vec::new();
    decoder
        .read_to_end(&mut buffer)
        .map_err(|e| Error::Other(format!("invalid gzip stream: {e}")))?;
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            for (name, content) in files {
                writer.start_file(*name, options).unwrap();
                writer.write_all(content).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer.into_inner()
    }

    #[test]
    fn zip_extraction_finds_nested_binary() {
        let archive = build_zip(&[
            ("LICENSE", b"license text".as_slice()),
            ("nested/xray.exe", b"binary bytes".as_slice()),
        ]);
        let extracted = extract_binary("Xray-windows-64.zip", &archive, "xray").unwrap();
        assert_eq!(extracted, b"binary bytes");
    }

    #[test]
    fn zip_extraction_reports_missing_binary() {
        let archive = build_zip(&[("readme.txt", b"hi".as_slice())]);
        let err = extract_binary("Xray-windows-64.zip", &archive, "xray").unwrap_err();
        assert!(err.to_string().contains("not found in zip archive"));
    }

    #[test]
    fn gz_extraction_returns_raw_binary() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"mihomo binary").unwrap();
        let compressed = encoder.finish().unwrap();

        let extracted =
            extract_binary("mihomo-linux-amd64-v1.19.12.gz", &compressed, "mihomo").unwrap();
        assert_eq!(extracted, b"mihomo binary");
    }

    #[test]
    fn zip_extraction_matches_platform_prefixed_names() {
        // The real mihomo archive ships exactly this name.
        let archive = build_zip(&[
            ("LICENSE", b"license".as_slice()),
            (
                "mihomo-windows-amd64-compatible.exe",
                b"meta binary".as_slice(),
            ),
        ]);
        let extracted = extract_binary(
            "mihomo-windows-amd64-compatible-v1.19.33.zip",
            &archive,
            "mihomo",
        )
        .unwrap();
        assert_eq!(extracted, b"meta binary");
    }

    #[test]
    fn zip_extraction_ignores_non_executables_with_similar_names() {
        let archive = build_zip(&[
            ("xray-notes.txt", b"notes".as_slice()),
            ("xray.exe", b"real".as_slice()),
        ]);
        let extracted = extract_binary("Xray-windows-64.zip", &archive, "xray").unwrap();
        assert_eq!(extracted, b"real");
    }

    #[test]
    fn plain_bytes_pass_through() {
        let extracted = extract_binary("sing-box", b"raw".as_slice(), "sing-box").unwrap();
        assert_eq!(extracted, b"raw");
    }
}
