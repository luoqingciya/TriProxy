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

import { create } from 'zustand'

import { api } from '../services/api'
import { CORE_LABEL } from '../types'
import type {
  ActiveDelay,
  AppliedProxy,
  ConnectionsSnapshot,
  LatencySample,
  TrafficSample,
  CoreKind,
  ManagerStatus,
  NodeSummary,
  ProfileLibrary,
  Selection,
  Settings,
  StoredProfile,
} from '../types'

/** 90 samples at the 2s poll interval: three minutes of history. */
const TRAFFIC_HISTORY_POINTS = 90

export type Page =
  | 'dashboard'
  | 'proxies'
  | 'connections'
  | 'profiles'
  | 'logs'
  | 'settings'

const PAGES: Page[] = [
  'dashboard',
  'proxies',
  'connections',
  'profiles',
  'logs',
  'settings',
]

/**
 * The active page lives in the URL hash.
 *
 * The shell has no router (a desktop window has no address bar), but keeping the
 * page in the hash means a dev-server reload — or a crash the webview recovers
 * from — lands on the page the user was on instead of resetting to the dashboard.
 */
function initialPage(): Page {
  const hash = window.location.hash.replace('#', '')
  return PAGES.includes(hash as Page) ? (hash as Page) : 'dashboard'
}

interface AppState {
  page: Page
  status: ManagerStatus | null
  library: ProfileLibrary
  nodes: NodeSummary[]
  selection: Selection | null
  settings: Settings | null
  /** Whether the OS will launch the app at login; `null` until read. */
  autostart: boolean | null
  proxy: AppliedProxy | null
  latency: Record<string, LatencySample>
  traffic: TrafficSample | null
  /**
   * Delay of the node the PROXY group is using, measured through the core.
   *
   * Not part of : each measurement is a real HTTP request through
   * the proxy, so it runs on its own slower timer instead of every two seconds.
   */
  delay: ActiveDelay | null
  /**
   * Recent traffic samples, oldest first, one per poll.
   *
   * Kept in memory only: the chart answers "what has my connection been doing
   * for the last few minutes", which is a question about the session, not about
   * history. Persisting it would mean a schema, a retention policy and a file
   * to corrupt, for a graph nobody looks at after closing the window.
   */
  trafficHistory: { up: number; down: number }[]
  connections: ConnectionsSnapshot | null
  testing: boolean
  busy: boolean
  error: string | null
  /** Transient informational message (not an error). */
  notice: string | null

  setPage: (page: Page) => void
  refresh: () => Promise<void>
  loadSettings: () => Promise<void>

  start: (core: CoreKind | null) => Promise<void>
  stop: () => Promise<void>
  switchTo: (target: string) => Promise<void>
  installCore: (core: CoreKind) => Promise<void>
  updateCore: (core: CoreKind) => Promise<void>

  importProfile: (text: string, name: string) => Promise<StoredProfile | null>
  useProfile: (id: string) => Promise<void>
  removeProfile: (id: string) => Promise<void>
  refreshProfile: (id: string) => Promise<void>

  setSystemProxy: (enabled: boolean) => Promise<void>
  testLatency: () => Promise<void>
  testProxyDelay: () => Promise<void>
  measureDelay: () => Promise<void>
  closeConnection: (id: string | null) => Promise<void>
  selectNode: (nodeId: string | null) => Promise<void>
  toggleFavorite: (nodeId: string) => Promise<void>
  updateSettings: (patch: Partial<Settings>) => Promise<void>
  setAutostart: (enabled: boolean) => Promise<void>
  clearError: () => void
  clearNotice: () => void
}

const emptyLibrary: ProfileLibrary = { active: null, profiles: [] }

export const useApp = create<AppState>((set, get) => ({
  page: initialPage(),
  status: null,
  library: emptyLibrary,
  nodes: [],
  selection: null,
  settings: null,
  autostart: null,
  proxy: null,
  latency: {},
  traffic: null,
  delay: null,
  trafficHistory: [],
  connections: null,
  testing: false,
  busy: false,
  error: null,
  notice: null,

  setPage: (page) => {
    window.history.replaceState(null, '', `#${page}`)
    set({ page })
  },

  /**
   * One round trip for everything the shell shows.
   *
   * The backend is a single process with an in-memory manager, so a combined
   * refresh is cheaper and keeps every panel consistent with each other.
   */
  refresh: async () => {
    try {
      const [status, library, nodes, selection, proxy, traffic, connections] =
        await Promise.all([
          api.status(),
          api.profiles(),
          api.activeNodes(),
          api.previewSelection(),
          api.systemProxyStatus(),
          api.traffic(),
          api.connections(),
        ])
      set({
        status,
        library,
        nodes,
        selection,
        proxy,
        traffic,
        trafficHistory: traffic
          ? [...get().trafficHistory, { up: traffic.up, down: traffic.down }].slice(
              -TRAFFIC_HISTORY_POINTS,
            )
          : [],
        connections,
        error: null,
      })
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  loadSettings: async () => {
    try {
      const [settings, autostart] = await Promise.all([
        api.settings(),
        api.autostartEnabled().catch(() => null),
      ])
      set({ settings, autostart })
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  start: async (core) => {
    set({ busy: true })
    try {
      const status = await api.startCore(core)
      set({ status, error: null })
      await get().refresh()
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false })
    }
  },

  stop: async () => {
    set({ busy: true })
    try {
      await api.stopCore()
      await get().refresh()
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false })
    }
  },

  switchTo: async (target) => {
    set({ busy: true })
    try {
      const status = await api.switchCore(target)
      set({ status, error: null })
      await get().refresh()
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false })
    }
  },

  /**
   * Download a core binary.
   *
   * The download runs in Rust and can take a while, so the busy flag is what
   * tells the button to say "下载中" instead of looking dead.
   */
  installCore: async (core) => {
    set({ busy: true, error: null })
    try {
      const status = await api.installCore(core)
      set({ status, notice: `${CORE_LABEL[core]} 已安装` })
      await get().refresh()
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false })
    }
  },

  /** Check upstream for a newer build of one core, then install it. */
  updateCore: async (core) => {
    set({ busy: true, error: null })
    try {
      const check = await api.checkCoreUpdate(core)
      if (check.status === 'up-to-date') {
        set({ notice: `${CORE_LABEL[core]} 已是最新版本` })
        return
      }
      const status = await api.installCore(core)
      set({
        status,
        notice:
          check.latest !== null
            ? `${CORE_LABEL[core]} 已更新到 ${check.latest}`
            : `${CORE_LABEL[core]} 已更新`,
      })
      await get().refresh()
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false })
    }
  },

  importProfile: async (text, name) => {
    set({ busy: true })
    try {
      const stored = await api.importProfile(text, name)
      await get().refresh()
      return stored
    } catch (error) {
      set({ error: describe(error) })
      return null
    } finally {
      set({ busy: false })
    }
  },

  useProfile: async (id) => {
    try {
      await api.setActiveProfile(id)
      await get().refresh()
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  removeProfile: async (id) => {
    try {
      await api.deleteProfile(id)
      await get().refresh()
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  setSystemProxy: async (enabled) => {
    try {
      const proxy = await api.setSystemProxy(enabled)
      set({ proxy })
      // Remember the choice so starting a core re-applies it.
      await get().updateSettings({ system_proxy: enabled })
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  testLatency: async () => {
    set({ testing: true })
    try {
      const samples = await api.testLatency(3)
      const map: Record<string, LatencySample> = {}
      for (const sample of samples) map[sample.node_id] = sample
      set({ latency: map, error: null })
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ testing: false })
    }
  },

  selectNode: async (nodeId) => {
    try {
      const result = await api.selectNode(nodeId)
      await get().refresh()
      // Saved but not applied is the normal case on Xray and on a stopped core;
      // saying so beats leaving the user wondering why nothing changed.
      if (!result.applied && result.reason) {
        set({ notice: `已保存，下次启动生效（${result.reason}）` })
      }
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  toggleFavorite: async (nodeId) => {
    const profile = get().library.profiles.find(
      (entry) => entry.id === get().library.active,
    )
    const starred = profile?.favorites?.includes(nodeId) ?? false
    try {
      await api.setFavorite(nodeId, !starred)
      await get().refresh()
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  closeConnection: async (id) => {
    try {
      await api.closeConnection(id)
      await get().refresh()
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  testProxyDelay: async () => {
    set({ testing: true })
    try {
      const samples = await api.testProxyDelay(3)
      const map: Record<string, LatencySample> = {}
      for (const sample of samples) map[sample.node_id] = sample
      set({ latency: map, error: null })
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ testing: false })
    }
  },

  /**
   * Measure the delay of whatever the group is using right now.
   *
   * Failures land in `delay.error` rather than the global banner: "Xray has no
   * controller API" is a permanent property of the running core, not an error
   * the user can act on, and a toast every ten seconds would be noise.
   */
  measureDelay: async () => {
    try {
      const delay = await api.activeDelay()
      set({ delay })
    } catch (error) {
      set({ delay: { node: null, latency_ms: null, error: describe(error) } })
    }
  },

  refreshProfile: async (id) => {
    set({ busy: true })
    try {
      await api.refreshProfile(id)
      await get().refresh()
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false })
    }
  },

  updateSettings: async (patch) => {
    const current = get().settings
    if (!current) return
    const next = { ...current, ...patch }
    // Optimistic: toggles should feel instant, and a failure rolls back below.
    set({ settings: next })
    try {
      await api.saveSettings(next)
    } catch (error) {
      set({ settings: current, error: describe(error) })
    }
  },

  /**
   * Toggle the login entry.
   *
   * The OS is the source of truth, so the value comes back from the backend
   * rather than being assumed from the click.
   */
  setAutostart: async (enabled) => {
    try {
      const state = await api.setAutostart(enabled)
      set({ autostart: state, error: null })
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  clearError: () => set({ error: null }),

  clearNotice: () => set({ notice: null }),
}))

function describe(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  return String(error)
}
