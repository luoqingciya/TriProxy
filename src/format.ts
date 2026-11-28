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

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value.toFixed(1)} ${units[unit]}`
}

export function formatRate(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`
}

export function formatUptime(seconds: number | null | undefined): string {
  if (!seconds) return '—'
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  if (h > 0) return `${h} 小时 ${m} 分`
  if (m > 0) return `${m} 分钟`
  return `${seconds} 秒`
}

export function formatRelativeTime(tsMs: number): string {
  if (!tsMs) return '尚未更新'
  const diffSecs = Math.max(0, Math.floor((Date.now() - tsMs) / 1000))
  if (diffSecs < 60) return '刚刚'
  const mins = Math.floor(diffSecs / 60)
  if (mins < 60) return `${mins} 分钟前`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours} 小时前`
  const days = Math.floor(hours / 24)
  return `${days} 天前`
}
