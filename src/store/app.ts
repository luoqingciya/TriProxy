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

import { api, isTauri } from '../services/api'
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
  RuleMode,
  Selection,
  Settings,
  StoredProfile,
  TunStatus,
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
  tunStatus: TunStatus | null
  latency: Record<string, LatencySample>
  testingNodeIds: Record<string, boolean>
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
  testingProgress: { done: number; total: number } | null
  busy: boolean
  busyAction: string | null
  needsCoreRestart: boolean
  error: string | null
  /** Transient informational message (not an error). */
  notice: string | null

  setPage: (page: Page) => void
  refresh: () => Promise<void>
  refreshLive: () => Promise<void>
  refreshLibrary: () => Promise<void>
  loadSettings: () => Promise<void>
  pushTraffic: (sample: TrafficSample) => void
  mergeLatencySamples: (samples: LatencySample[]) => void

  start: (core: CoreKind | null) => Promise<void>
  stop: () => Promise<void>
  switchTo: (target: string) => Promise<void>
  applySettingsRestart: () => Promise<void>
  installCore: (core: CoreKind) => Promise<void>
  updateCore: (core: CoreKind) => Promise<void>

  importProfile: (text: string, name: string) => Promise<StoredProfile | null>
  useProfile: (id: string) => Promise<void>
  removeProfile: (id: string) => Promise<void>
  refreshProfile: (id: string) => Promise<void>

  setSystemProxy: (enabled: boolean) => Promise<void>
  setTun: (enabled: boolean) => Promise<void>
  authorizeTun: () => Promise<void>
  setRuleMode: (mode: RuleMode) => Promise<void>
  testLatency: () => Promise<void>
  testProxyDelay: () => Promise<void>
  testNodeDelay: (nodeId: string) => Promise<void>
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
let liveInFlight = false

const CORE_RESTART_FIELDS = new Set<keyof Settings>([
  'core_preference',
  'tun',
  'rule_set',
  'proxy_groups',
  'allow_lan',
  'dns_mode',
  'log_level',
  'listen_port',
  'api_port',
  'custom_rules',
  'rule_sets',
])

const RULE_MODE_LABEL: Record<RuleMode, string> = {
  split: '规则分流',
  global: '全局代理',
  direct: '全局直连',
}

export const useApp = create<AppState>((set, get) => ({
  page: initialPage(),
  status: null,
  library: emptyLibrary,
  nodes: [],
  selection: null,
  settings: null,
  autostart: null,
  proxy: null,
  tunStatus: null,
  latency: {},
  testingNodeIds: {},
  traffic: null,
  delay: null,
  trafficHistory: [],
  connections: null,
  testing: false,
  testingProgress: null,
  busy: false,
  busyAction: null,
  needsCoreRestart: false,
  error: null,
  notice: null,

  setPage: (page) => {
    window.history.replaceState(null, '', `#${page}`)
    set({ page })
    if (page === 'connections') {
      void get().refreshLive()
    }
  },

  refreshLive: async () => {
    if (liveInFlight) return
    liveInFlight = true
    const wantConnections = get().page === 'connections'
    const needTrafficFetch =
      !isTauri || (get().status?.active_core !== null && get().traffic === null)
    try {
      const [status, traffic, connections] = await Promise.all([
        api.status(),
        needTrafficFetch ? api.traffic() : Promise.resolve(get().traffic),
        wantConnections ? api.connections() : Promise.resolve(get().connections),
      ])
      set((state) => {
        const stopped = status.active_core === null
        const nextTraffic = stopped ? null : traffic
        return {
          status,
          traffic: nextTraffic,
          trafficHistory: stopped
            ? []
            : !isTauri && nextTraffic
              ? [...state.trafficHistory, { up: nextTraffic.up, down: nextTraffic.down }].slice(
                  -TRAFFIC_HISTORY_POINTS,
                )
              : state.trafficHistory,
          ...(wantConnections ? { connections: stopped ? null : connections } : {}),
        }
      })
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      liveInFlight = false
    }
  },

  refreshLibrary: async () => {
    try {
      const snapshot = await api.librarySnapshot()
      set({
        library: snapshot.library,
        nodes: snapshot.nodes,
        selection: snapshot.selection,
      })
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  refresh: async () => {
    await Promise.all([get().refreshLive(), get().refreshLibrary()])
  },

  loadSettings: async () => {
    try {
      const [settings, autostart, proxy, tunStatus] = await Promise.all([
        api.settings(),
        api.autostartEnabled().catch(() => null),
        api.systemProxyStatus().catch(() => null),
        api.tunStatus().catch(() => null),
      ])
      set({ settings, autostart, proxy, tunStatus })
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  pushTraffic: (sample) =>
    set((state) => ({
      traffic: sample,
      trafficHistory: [...state.trafficHistory, { up: sample.up, down: sample.down }].slice(
        -TRAFFIC_HISTORY_POINTS,
      ),
    })),

  mergeLatencySamples: (samples) => {
    if (samples.length === 0) return
    set((state) => {
      const next = { ...state.latency }
      for (const sample of samples) next[sample.node_id] = sample
      const progress = state.testingProgress
        ? {
            total: state.testingProgress.total,
            done: Math.min(
              state.testingProgress.total,
              state.testingProgress.done + samples.length,
            ),
          }
        : null
      return { latency: next, testingProgress: progress }
    })
  },

  start: async (core) => {
    set({ busy: true, busyAction: 'connect' })
    try {
      const status = await api.startCore(core)
      const proxy = await api.systemProxyStatus().catch(() => get().proxy)
      set({ status, proxy, needsCoreRestart: false, error: null })
      if (get().page === 'connections') {
        void get().refreshLive()
      }
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false, busyAction: null })
    }
  },

  stop: async () => {
    set({ busy: true, busyAction: 'disconnect' })
    try {
      await api.stopCore()
      const status = await api.status()
      set({
        status,
        proxy: null,
        traffic: null,
        trafficHistory: [],
        connections: null,
        delay: null,
        needsCoreRestart: false,
      })
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false, busyAction: null })
    }
  },

  switchTo: async (target) => {
    set({ busy: true, busyAction: `switch:${target}` })
    try {
      const status = await api.switchCore(target)
      const proxy = await api.systemProxyStatus().catch(() => get().proxy)
      set({ status, proxy, needsCoreRestart: false, error: null })
      if (get().page === 'connections') {
        void get().refreshLive()
      }
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false, busyAction: null })
    }
  },

  applySettingsRestart: async () => {
    const target = get().settings?.core_preference ?? 'smart'
    await get().switchTo(target)
    if (!get().error) {
      set({ notice: '核心已按最新配置重启生效' })
    }
  },

  /**
   * Download a core binary.
   *
   * The download runs in Rust and can take a while, so the busy flag is what
   * tells the button to say "下载中" instead of looking dead.
   */
  installCore: async (core) => {
    set({ busy: true, busyAction: `install:${core}`, error: null })
    try {
      const status = await api.installCore(core)
      set({ status, notice: `${CORE_LABEL[core]} 已安装` })
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false, busyAction: null })
    }
  },

  /** Check upstream for a newer build of one core, then install it. */
  updateCore: async (core) => {
    set({ busy: true, busyAction: `update:${core}`, error: null })
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
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false, busyAction: null })
    }
  },

  importProfile: async (text, name) => {
    set({ busy: true, busyAction: 'import' })
    try {
      const stored = await api.importProfile(text, name)
      await get().refreshLibrary()
      set({
        notice: `已导入配置「${stored.name}」（${stored.node_count} 个节点）`,
        error: null,
      })
      return stored
    } catch (error) {
      set({ error: describe(error) })
      return null
    } finally {
      set({ busy: false, busyAction: null })
    }
  },

  useProfile: async (id) => {
    try {
      await api.setActiveProfile(id)
      await get().refreshLibrary()
      const running = get().status?.active_core !== null
      if (running) {
        await get().switchTo(get().settings?.core_preference ?? 'smart')
        if (!get().error) {
          set({ notice: '已切换配置并立即应用' })
        }
      } else {
        set({ notice: '已切换活动配置' })
      }
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  removeProfile: async (id) => {
    try {
      await api.deleteProfile(id)
      await get().refreshLibrary()
      set({ notice: '已删除配置' })
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  setSystemProxy: async (enabled) => {
    try {
      const proxy = await api.setSystemProxy(enabled)
      set({
        proxy,
        notice: enabled ? '系统代理已开启' : '系统代理已关闭',
        error: null,
      })
      // Remember the choice so starting a core re-applies it.
      await get().updateSettings({ system_proxy: enabled })
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  setTun: async (enabled) => {
    const prevSettings = get().settings
    set({ busy: true, busyAction: 'tun', error: null })
    try {
      const status = await api.setTun(enabled)
      const [settings, tunStatus, proxy, selection] = await Promise.all([
        api.settings().catch(() => (prevSettings ? { ...prevSettings, tun: enabled } : null)),
        api.tunStatus().catch(() => get().tunStatus),
        api.systemProxyStatus().catch(() => (enabled ? null : get().proxy)),
        api.previewSelection().catch(() => get().selection),
      ])
      set({
        status,
        settings,
        tunStatus,
        proxy,
        selection,
        needsCoreRestart: false,
        notice: enabled
          ? `TUN 模式已开启（网卡 ${tunStatus?.interface_name ?? 'triproxy'}）`
          : 'TUN 模式已关闭',
        error: null,
      })
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false, busyAction: null })
    }
  },

  authorizeTun: async () => {
    set({ busy: true, busyAction: 'authorize-tun', error: null })
    try {
      const tunStatus = await api.authorizeTun()
      set({ tunStatus, notice: 'TUN 权限已授权，可直接开启 TUN 模式', error: null })
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false, busyAction: null })
    }
  },

  setRuleMode: async (mode) => {
    const prevSettings = get().settings
    if (prevSettings) {
      set({ settings: { ...prevSettings, rule_mode: mode }, error: null })
    }
    try {
      const appliedLive = await api.setRuleMode(mode)
      set({
        notice: appliedLive
          ? `已切换为${RULE_MODE_LABEL[mode]}（已即时生效）`
          : `已切换为${RULE_MODE_LABEL[mode]}`,
        error: null,
      })
    } catch (error) {
      set({ settings: prevSettings, error: describe(error) })
    }
  },

  testLatency: async () => {
    set({ testing: true, testingProgress: { done: 0, total: get().nodes.length } })
    try {
      const samples = await api.testLatency(3)
      const map: Record<string, LatencySample> = {}
      for (const sample of samples) map[sample.node_id] = sample
      set({ latency: map, error: null })
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ testing: false, testingProgress: null })
    }
  },

  selectNode: async (nodeId) => {
    try {
      const result = await api.selectNode(nodeId)
      await get().refreshLibrary()
      // Saved but not applied is the normal case on Xray and on a stopped core;
      // saying so beats leaving the user wondering why nothing changed.
      if (result.applied) {
        set({
          notice: nodeId === null ? '已切换到自动选择' : `已切换到「${result.node ?? '所选节点'}」`,
        })
      } else if (result.reason) {
        set({ notice: `已保存，下次启动生效（${result.reason}）` })
      }
    } catch (error) {
      set({ error: describe(error) })
    }
  },

  toggleFavorite: async (nodeId) => {
    const currentLibrary = get().library
    const activeId = currentLibrary.active
    const profile = currentLibrary.profiles.find((entry) => entry.id === activeId)
    const starred = profile?.favorites?.includes(nodeId) ?? false
    const nextStarred = !starred
    if (activeId) {
      set({
        library: {
          ...currentLibrary,
          profiles: currentLibrary.profiles.map((entry) => {
            if (entry.id !== activeId) return entry
            const prevFavs = entry.favorites ?? []
            const nextFavs = nextStarred
              ? prevFavs.includes(nodeId)
                ? prevFavs
                : [...prevFavs, nodeId]
              : prevFavs.filter((id) => id !== nodeId)
            return { ...entry, favorites: nextFavs }
          }),
        },
      })
    }
    try {
      await api.setFavorite(nodeId, nextStarred)
    } catch (error) {
      set({ library: currentLibrary, error: describe(error) })
    }
  },

  closeConnection: async (id) => {
    const prev = get().connections
    if (prev) {
      set({
        connections: {
          ...prev,
          connections: id === null ? [] : prev.connections.filter((c) => c.id !== id),
        },
      })
    }
    try {
      await api.closeConnection(id)
      if (get().page === 'connections') {
        const connections = await api.connections()
        set({ connections })
      }
    } catch (error) {
      set({ connections: prev, error: describe(error) })
    }
  },

  testProxyDelay: async () => {
    set({ testing: true, testingProgress: { done: 0, total: get().nodes.length } })
    try {
      const samples = await api.testProxyDelay(3)
      const map: Record<string, LatencySample> = {}
      for (const sample of samples) map[sample.node_id] = sample
      set({ latency: map, error: null })
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ testing: false, testingProgress: null })
    }
  },

  testNodeDelay: async (nodeId) => {
    set((state) => ({
      testingNodeIds: { ...state.testingNodeIds, [nodeId]: true },
    }))
    try {
      const sample = await api.testNodeDelay(nodeId, 3)
      set((state) => ({
        latency: { ...state.latency, [nodeId]: sample },
        error: null,
      }))
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set((state) => {
        const next = { ...state.testingNodeIds }
        delete next[nodeId]
        return { testingNodeIds: next }
      })
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
    set({ busy: true, busyAction: `refresh:${id}` })
    try {
      const outcome = await api.refreshProfile(id)
      await get().refreshLibrary()
      if (outcome.kind === 'updated') {
        set({ notice: `订阅已更新（共 ${outcome.node_count ?? 0} 个节点）` })
      } else if (outcome.kind === 'not_modified') {
        set({ notice: '订阅已是最新，无变更' })
      } else if (outcome.kind === 'failed') {
        set({ error: outcome.error ?? '订阅刷新失败' })
      }
    } catch (error) {
      set({ error: describe(error) })
    } finally {
      set({ busy: false, busyAction: null })
    }
  },

  updateSettings: async (patch) => {
    const current = get().settings
    if (!current) return
    const next = { ...current, ...patch }
    const touchesCore = (Object.keys(patch) as (keyof Settings)[]).some(
      (key) => CORE_RESTART_FIELDS.has(key) && patch[key] !== current[key],
    )
    const running = get().status?.active_core !== null
    // Optimistic: toggles should feel instant, and a failure rolls back below.
    set({
      settings: next,
      needsCoreRestart: touchesCore && running ? true : get().needsCoreRestart,
    })
    try {
      await api.saveSettings(next)
      const selection = await api.previewSelection().catch(() => get().selection)
      set({ selection })
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
