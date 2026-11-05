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
 * Memory, in the unit a person would use to describe it.
 *
 * One decimal below 100 MB and none above: "86.4 MB" is informative, "86.43 MB"
 * is noise, and "1024.0 MB" should have been "1.00 GB". A missing figure renders
 * as a dash rather than "0 MB" — a process that exited has no memory use, and
 * zero would read as "running but idle".
 */
export function formatMemory(bytes: number | null | undefined): string {
  if (bytes == null) return '—'
  const mb = bytes / (1024 * 1024)
  if (mb >= 1024) return `${(mb / 1024).toFixed(2)} GB`
  if (mb >= 100) return `${Math.round(mb)} MB`
  return `${mb.toFixed(1)} MB`
}
