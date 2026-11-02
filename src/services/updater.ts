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
 * Application self-update.
 *
 * Separate from the API wrapper because this is not one of our commands: it goes
 * through Tauri's updater plugin, which needs a capability entry and is absent
 * outside the desktop shell. Everything here degrades to "no update available"
 * when running in a browser (the mock/dev path), so the UI never has to know.
 */

import type { Update } from '@tauri-apps/plugin-updater'

export interface AppUpdateInfo {
  version: string
  notes: string | null
  date: string | null
}

/** The pending update, kept so `install` does not have to check again. */
let pending: Update | null = null

function inDesktopShell(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

export async function checkForUpdate(): Promise<AppUpdateInfo | null> {
  if (!inDesktopShell()) return null

  const { check } = await import('@tauri-apps/plugin-updater')
  const update = await check()
  pending = update
  if (!update) return null

  return {
    version: update.version,
    notes: update.body ?? null,
    date: update.date ?? null,
  }
}

/**
 * Download and install the pending update.
 *
 * Progress is reported as a percentage; the updater reports bytes rather than a
 * fraction, and the total is not always known, so `null` means "downloading but
 * size unknown" rather than zero.
 */
export async function installUpdate(
  onProgress: (percent: number | null) => void,
): Promise<void> {
  if (!pending) {
    throw new Error('no update has been checked yet')
  }

  let downloaded = 0
  let total: number | null = null

  await pending.downloadAndInstall((event) => {
    if (event.event === 'Started') {
      total = event.data.contentLength ?? null
      onProgress(total === null ? null : 0)
    } else if (event.event === 'Progress') {
      downloaded += event.data.chunkLength
      onProgress(total === null ? null : Math.min(100, (downloaded / total) * 100))
    } else if (event.event === 'Finished') {
      onProgress(100)
    }
  })

  pending = null
}
