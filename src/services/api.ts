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

import { invoke } from '@tauri-apps/api/core'

import type {
  ActiveDelay,
  AppliedProxy,
  CoreKind,
  CoreUpdate,
  ConnectionsSnapshot,
  LatencySample,
  TrafficSample,
  LogEntry,
  ManagerStatus,
  NodeSummary,
  ProfileLibrary,
  Selection,
  Settings,
  StoredProfile,
} from '../types'
import { mockCall } from './mock'

/**
 * True when running inside the Tauri shell.
 *
 * The frontend is also served by `vite dev` in a plain browser for UI work, in
 * which case there is no Rust backend and the mock answers instead.
 */
export const isTauri =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) {
    return mockCall<T>(command, args)
  }
  return invoke<T>(command, args)
}

export const api = {
  status: () => call<ManagerStatus>('status'),

  profiles: () => call<ProfileLibrary>('profiles'),

  activeNodes: () => call<NodeSummary[]>('active_nodes'),

  importProfile: (text: string, name: string) =>
    call<StoredProfile>('import_profile', { text, name }),

  setActiveProfile: (id: string) => call<void>('set_active_profile', { id }),

  deleteProfile: (id: string) => call<void>('delete_profile', { id }),

  refreshProfile: (id: string) =>
    call<{ kind: string; node_count?: number; error?: string }>('refresh_profile', { id }),

  startCore: (core: CoreKind | null) => call<ManagerStatus>('start_core', { core }),

  stopCore: () => call<void>('stop_core'),

  /** Download and install a core binary (the lite installer ships none). */
  installCore: (core: CoreKind, mirror?: string) =>
    call<ManagerStatus>('install_core', { core, mirror: mirror ?? null }),

  checkCoreUpdate: (core: CoreKind) =>
    call<CoreUpdate>('check_core_update', { core }),

  autostartEnabled: () => call<boolean>('autostart_enabled'),

  setAutostart: (enabled: boolean) =>
    call<boolean>('set_autostart', { enabled }),

  switchCore: (target: string) => call<ManagerStatus>('switch_core', { target }),

  recentLogs: (limit: number) => call<LogEntry[]>('recent_logs', { limit }),

  traffic: () => call<TrafficSample | null>('traffic'),

  connections: () => call<ConnectionsSnapshot | null>('connections'),

  closeConnection: (id: string | null) =>
    call<void>('close_connection', { id }),

  selectNode: (nodeId: string | null) =>
    call<{ applied: boolean; node?: string; reason?: string }>('select_node', {
      nodeId,
    }),

  setFavorite: (nodeId: string, favorite: boolean) =>
    call<boolean>('set_favorite', { nodeId, favorite }),

  testProxyDelay: (timeoutSecs: number) =>
    call<LatencySample[]>('test_proxy_delay', { timeoutSecs }),

  /**
   * Delay of the node the PROXY group is using right now.
   *
   * Separate from `status()` on purpose: it makes one HTTP request through the
   * proxy, so it belongs on a slower timer than the two-second status poll.
   */
  activeDelay: () => call<ActiveDelay>('active_delay'),

  testLatency: (timeoutSecs: number) =>
    call<LatencySample[]>('test_latency', { timeoutSecs }),

  settings: () => call<Settings>('settings'),

  saveSettings: (settings: Settings) => call<void>('save_settings', { settings }),

  /** Parse rule text without saving it; rejects with the offending line. */
  validateRules: (text: string) => call<void>('validate_rules', { text }),

  /** Same for rule-sets: rejects with the offending line. */
  validateRuleSets: (text: string) => call<void>('validate_rule_sets', { text }),

  /** Write a backup; null when the save dialog was dismissed. */
  exportBackup: () => call<string | null>('export_backup'),

  /** Merge a backup; null when the open dialog was dismissed. */
  importBackup: () => call<number | null>('import_backup'),

  previewSelection: () => call<Selection | null>('preview_selection'),

  systemProxyStatus: () => call<AppliedProxy | null>('system_proxy_status'),

  setSystemProxy: (enabled: boolean) =>
    call<AppliedProxy | null>('set_system_proxy', { enabled }),
}
