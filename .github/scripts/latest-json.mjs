#!/usr/bin/env node
/*
 * TriProxy — a modern multi-core proxy client.
 * Copyright (C) 2026 luoqingci
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License as published by the Free Software
 * Foundation, either version 3 of the License, or (at your option) any later
 * version. It is distributed in the hope that it will be useful, but WITHOUT ANY
 * WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
 * PARTICULAR PURPOSE. See LICENSE for the full text.
 *
 * Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.
 */

/**
 * Assemble `latest.json` for the Tauri updater.
 *
 * The updater fetches this file, compares `version` with its own, and downloads
 * the payload for its platform — verifying it against `signature`, which is why
 * a missing `.sig` must fail the release rather than silently produce an entry
 * no client can install.
 *
 * Only the `lite` payloads are advertised. The updater replaces the application,
 * not the bundled cores: a `full` install updated with a lite package keeps its
 * `cores/` directory, because the NSIS installer overwrites the files it carries
 * and leaves everything else alone. Advertising `full` would mean a 50 MB
 * download on every update for a payload the user already has.
 *
 * Usage: node latest-json.mjs <artifacts-dir> <version> <repo> <output>
 */

import { readFileSync, readdirSync, writeFileSync, existsSync } from 'node:fs'
import { join } from 'node:path'

const [dir, version, repo, output] = process.argv.slice(2)

if (!dir || !version || !repo || !output) {
  console.error('usage: latest-json.mjs <artifacts-dir> <version> <repo> <output>')
  process.exit(1)
}

const files = readdirSync(dir)
const base = `https://github.com/${repo}/releases/download/v${version}`

/**
 * Platform key the updater asks for → payload file name.
 *
 * The names mirror the packaging step in .github/workflows/release.yml:
 * `TriProxy-<version>-<os>-<arch>-<variant>.<ext>`. They are spelled out rather
 * than globbed so that a rename in the workflow fails here — with the missing
 * name in the error — instead of producing a latest.json that points at a file
 * that does not exist.
 */
const wanted = {
  'windows-x86_64': `TriProxy-${version}-windows-x64-lite.exe`,
  'darwin-aarch64': `TriProxy-${version}-macos-arm64-lite.app.tar.gz`,
  'linux-x86_64': `TriProxy-${version}-linux-x64-lite.AppImage`,
}

const platforms = {}
const missing = []

for (const [platform, payload] of Object.entries(wanted)) {
  if (!files.includes(payload)) {
    missing.push(`${platform}: ${payload}`)
    continue
  }
  const sigPath = join(dir, `${payload}.sig`)
  if (!existsSync(sigPath)) {
    missing.push(`${platform}: ${payload}.sig`)
    continue
  }

  const signature = readFileSync(sigPath, 'utf8').trim()
  // An empty or truncated signature produces a latest.json that every client
  // rejects at install time — long after the release looks successful. The
  // minimum length is a cheap sanity check, not a cryptographic one.
  if (signature.length < 100) {
    missing.push(`${platform}: ${payload}.sig is empty or truncated (${signature.length} bytes)`)
    continue
  }

  platforms[platform] = {
    signature,
    url: `${base}/${encodeURIComponent(payload)}`,
  }
}

if (missing.length > 0) {
  console.error('missing update artifacts:')
  for (const item of missing) console.error(`  ${item}`)
  console.error(`present: ${files.join(', ')}`)
  process.exit(1)
}

const latest = {
  version,
  notes: 'See the release page for details.',
  pub_date: new Date().toISOString(),
  platforms,
}

writeFileSync(output, `${JSON.stringify(latest, null, 2)}\n`)
console.log(`wrote ${output} for ${Object.keys(platforms).join(', ')}`)
