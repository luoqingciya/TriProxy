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
 * Browser preview data.
 *
 * `npm run dev` in a plain browser has no Rust backend, so the UI would be
 * empty. This mock answers the same commands with plausible data so the
 * interface can be developed and reviewed without building the desktop shell.
 * It is never bundled into the Tauri build for use: `isTauri` in `api.ts`
 * decides which path runs, and the desktop app always has the real one.
 */

import type {
  CoreReport,
  CoreState,
  LogEntry,
  ManagerStatus,
  NodeSummary,
  ProfileLibrary,
  Selection,
  Settings,
  StoredProfile,
} from '../types'

const now = Date.now()

let library: ProfileLibrary = {
  active: 'a1b2c3d4e5f6',
  profiles: [
    {
      id: 'a1b2c3d4e5f6',
      name: 'jp-sub',
      source: 'sub.txt',
      format: 'uri_list',
      node_count: 4,
      content_id: '837dc357d6deeb27',
      updated_at_ms: now - 1000 * 60 * 12,
      selected_node: 'n2',
      subscription: {
        url: 'https://example.com/api/v1/client/subscribe?token=demo',
        interval_secs: 86400,
        last_fetch_ms: now - 1000 * 60 * 12,
        last_error: null,
      },
    },
    {
      id: 'b2c3d4e5f6a7',
      name: 'backup',
      source: 'https://example.com/sub',
      format: 'base64_subscription',
      node_count: 12,
      content_id: 'cc11aa22bb33cc44',
      updated_at_ms: now - 1000 * 60 * 60 * 30,
    },
  ],
}

let proxyEnabled = false

let activeCore: CoreReport['kind'] | null = 'sing-box'

let settings: Settings = {
  theme: 'dark',
  core_preference: null,
  tun: false,
  rule_set: false,
  proxy_groups: false,
  allow_lan: false,
  system_proxy: false,
  dns_mode: 'off',
  rule_mode: 'global',
  log_level: 'info',
  listen_port: 7890,
  close_to_tray: true,
  auto_connect: false,
  proxy_bypass: null,
  custom_rules: null,
  rule_sets: null,
}

const nodes: NodeSummary[] = [
  { id: 'n1', name: 'Tokyo Reality', protocol: 'vless', server: 'jp.example.com', port: 443, transport: 'tcp', security: 'reality', udp: true },
  { id: 'n2', name: 'Singapore gRPC', protocol: 'trojan', server: 'sg.example.com', port: 443, transport: 'grpc', security: 'tls', udp: true },
  { id: 'n3', name: 'US West', protocol: 'shadowsocks', server: 'us.example.com', port: 8388, transport: 'tcp', security: '-', udp: true },
  { id: 'n4', name: 'HK Relay', protocol: 'vmess', server: 'hk.example.com', port: 443, transport: 'ws', security: 'tls', udp: true },
]

const logs: LogEntry[] = Array.from({ length: 40 }).map((_, index) => {
  const cores = ['mihomo', 'sing-box', 'xray']
  const samples = [
    'startup: loading configuration',
    'inbound mixed listening at 127.0.0.1:7890',
    'route: final outbound PROXY',
    'health check connection closed',
    'provider updated',
  ]
  return {
    seq: index,
    ts_ms: now - (40 - index) * 1000,
    core: cores[index % cores.length],
    level: index % 9 === 0 ? 'warn' : 'info',
    text: samples[index % samples.length],
  }
})

function status(): ManagerStatus {
  const states: Record<string, CoreState> = {
    mihomo: 'stopped',
    'sing-box': activeCore === 'sing-box' ? 'running' : 'stopped',
    xray: activeCore === 'xray' ? 'running' : 'stopped',
  }
  if (activeCore === 'mihomo') states.mihomo = 'running'

  const cores: CoreReport[] = (['mihomo', 'sing-box', 'xray'] as const).map((kind, index) => ({
    kind,
    display_name: kind === 'sing-box' ? 'sing-box' : kind === 'mihomo' ? 'Mihomo' : 'Xray',
    binary: `/data/bin/${kind}`,
    binary_source: 'managed',
    binary_version: ['1.19.33', '1.14.4', '26.10.31'][index],
    state: states[kind],
    pid: states[kind] === 'running' ? 12000 + index : null,
    uptime_secs: states[kind] === 'running' ? 320 + index * 7 : null,
    listen_port: 7890,
    listening: states[kind] === 'running',
    latency_ms: states[kind] === 'running' ? 1 : null,
    last_error: null,
  }))

  return {
    active_core: activeCore,
    exclusive: true,
    data_dir: 'C:\\Users\\you\\AppData\\Roaming\\TriProxy',
    cores,
  }
}

function selection(): Selection {
  return {
    selected: 'sing-box',
    overridden: false,
    reason:
      'sing-box is the default general-purpose core; no candidate needed a specialisation; eliminated: xray (TUN)',
    evaluations: [
      { core: 'sing-box', eligible: true, unmet: [], native_features: [], rank: 0 },
      { core: 'mihomo', eligible: true, unmet: [], native_features: [], rank: 1 },
      { core: 'xray', eligible: false, unmet: ['TUN'], native_features: [], rank: 2 },
    ],
  }
}

export async function mockCall<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  // A touch of latency keeps loading states honest during development.
  await new Promise((resolve) => setTimeout(resolve, 80))

  switch (command) {
    case 'status':
      return status() as T
    case 'profiles':
      return library as T
    case 'active_nodes':
      return (library.active ? nodes : []) as T
    case 'import_profile': {
      const name = String(args?.name ?? 'imported')
      const existing = library.profiles.find((entry) => entry.name === name)
      const record: StoredProfile = {
        id: existing?.id ?? Math.random().toString(16).slice(2, 14),
        name,
        source: 'pasted',
        format: 'uri_list',
        node_count: nodes.length,
        content_id: 'mock',
        updated_at_ms: Date.now(),
      }
      library = {
        active: library.active,
        profiles: existing
          ? library.profiles.map((entry) => (entry.id === existing.id ? record : entry))
          : [record, ...library.profiles],
      }
      return record as T
    }
    case 'set_active_profile':
      library = { ...library, active: String(args?.id) }
      return undefined as T
    case 'refresh_profile':
      return { kind: 'not_modified' } as T
    case 'install_core':
      // Pretend the download worked: the browser mock has no cores at all.
      return status() as T
    case 'check_core_update':
      return {
        core: args?.core,
        installed: null,
        latest: null,
        status: 'not-installed',
      } as T
    case 'autostart_enabled':
      return false as T
    case 'set_autostart':
      return Boolean(args?.enabled) as T
    case 'validate_rules':
      return undefined as T
    case 'validate_rule_sets':
      return undefined as T
    case 'export_backup':
      return null as T
    case 'import_backup':
      return null as T
    case 'delete_profile':
      library = {
        active: library.active === args?.id ? null : library.active,
        profiles: library.profiles.filter((entry) => entry.id !== args?.id),
      }
      return undefined as T
    case 'start_core':
      activeCore = (args?.core as CoreReport['kind']) ?? 'sing-box'
      return status() as T
    case 'stop_core':
      activeCore = null
      return undefined as T
    case 'switch_core': {
      const target = String(args?.target)
      activeCore = target === 'smart' ? 'sing-box' : (target as CoreReport['kind'])
      return status() as T
    }
    case 'traffic':
      return { up: 24_576, down: 512_000 } as T
    case 'connections':
      return {
        upload_total: 1024 * 512,
        download_total: 1024 * 1024 * 12,
        connections: [
          {
            id: 'a1b2c3d4e5f6',
            metadata: { network: 'tcp', kind: 'HTTPS', source_ip: '127.0.0.1', destination_ip: '93.184.216.34', destination_port: '443', host: 'www.google.com' },
            upload: 2048,
            download: 65536,
            chains: ['PROXY', 'Tokyo Reality'],
            rule: 'Match',
            rule_payload: '',
            start: '2026-11-01T22:40:00Z',
          },
        ],
      } as T
    case 'close_connection':
      return undefined as T
    case 'select_node':
      return { applied: false, node: 'mock', reason: 'mock mode' } as T
    case 'set_favorite':
      return Boolean(args?.favorite) as T
    case 'test_proxy_delay':
      return nodes.map((n, i) => ({
        node_id: n.id,
        name: n.name,
        endpoint: n.server + ':' + n.port,
        kind: 'http',
        latency_ms: 120 + i * 90,
        error: null,
        measured_at_ms: now,
      })) as T
    case 'test_latency':
      return nodes.map((n, i) => ({
        node_id: n.id,
        name: n.name,
        endpoint: n.server + ':' + n.port,
        kind: 'tcp',
        latency_ms: 40 + i * 37,
        error: null,
        measured_at_ms: now,
      })) as T
    case 'recent_logs':
      return logs.slice(-Number(args?.limit ?? 200)) as T
    case 'settings':
      return settings as T
    case 'save_settings':
      settings = args?.settings as Settings
      return undefined as T
    case 'preview_selection':
      return selection() as T
    case 'system_proxy_status':
      return (proxyEnabled ? { endpoint: '127.0.0.1:7890', backend: 'windows-registry', applied_at_ms: now } : null) as T
    case 'set_system_proxy':
      proxyEnabled = Boolean(args?.enabled)
      return (proxyEnabled ? { endpoint: '127.0.0.1:7890', backend: 'windows-registry', applied_at_ms: now } : null) as T
    default:
      throw new Error(`unknown command: ${command}`)
  }
}
