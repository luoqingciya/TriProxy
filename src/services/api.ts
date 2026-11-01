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
  AppliedProxy,
  CoreKind,
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

  switchCore: (target: string) => call<ManagerStatus>('switch_core', { target }),

  recentLogs: (limit: number) => call<LogEntry[]>('recent_logs', { limit }),

  traffic: () => call<TrafficSample | null>('traffic'),

  connections: () => call<ConnectionsSnapshot | null>('connections'),

  closeConnection: (id: string | null) =>
    call<void>('close_connection', { id }),

  testProxyDelay: (timeoutSecs: number) =>
    call<LatencySample[]>('test_proxy_delay', { timeoutSecs }),

  testLatency: (timeoutSecs: number) =>
    call<LatencySample[]>('test_latency', { timeoutSecs }),

  settings: () => call<Settings>('settings'),

  saveSettings: (settings: Settings) => call<void>('save_settings', { settings }),

  previewSelection: () => call<Selection | null>('preview_selection'),

  systemProxyStatus: () => call<AppliedProxy | null>('system_proxy_status'),

  setSystemProxy: (enabled: boolean) =>
    call<AppliedProxy | null>('set_system_proxy', { enabled }),
}
