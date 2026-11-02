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
 * Wire types. These mirror the serde representations in `triproxy-core`, so the
 * names and casing must stay in sync with the Rust structs.
 */

export type CoreKind = 'mihomo' | 'sing-box' | 'xray'

export type CoreState = 'stopped' | 'starting' | 'running' | 'stopping' | 'failed'

export type Protocol = 'shadowsocks' | 'vmess' | 'vless' | 'trojan' | 'wireguard' | 'hysteria2' | 'tuic' | 'socks' | 'http'

export interface CoreReport {
  kind: CoreKind
  display_name: string
  binary: string | null
  binary_source: string | null
  binary_version: string | null
  state: CoreState
  pid: number | null
  uptime_secs: number | null
  listen_port: number | null
  listening: boolean | null
  latency_ms: number | null
  last_error: string | null
}

/** Result of comparing an installed core with the latest upstream release. */
export interface CoreUpdate {
  core: CoreKind
  installed: string | null
  latest: string | null
  status: 'not-installed' | 'up-to-date' | 'update-available'
}

export interface ManagerStatus {
  active_core: CoreKind | null
  exclusive: boolean
  data_dir: string
  cores: CoreReport[]
}

export interface Subscription {
  url: string
  interval_secs: number
  last_fetch_ms: number
  last_error?: string | null
}

export interface StoredProfile {
  id: string
  name: string
  source: string
  format: 'uri_list' | 'base64_subscription'
  node_count: number
  content_id: string
  updated_at_ms: number
  subscription?: Subscription | null
  /** Node the user picked, or null when the profile order decides. */
  selected_node?: string | null
  /** Starred node ids, in the order they were starred. */
  favorites?: string[]
}

export interface ProfileLibrary {
  active: string | null
  profiles: StoredProfile[]
}

export interface NodeSummary {
  id: string
  name: string
  protocol: Protocol
  server: string
  port: number
  transport: string
  security: string
  udp: boolean
}

export interface CoreEvaluation {
  core: CoreKind
  eligible: boolean
  unmet: string[]
  native_features: string[]
  rank: number
}

export interface Selection {
  selected: CoreKind
  evaluations: CoreEvaluation[]
  reason: string
  overridden: boolean
}

export interface TrafficSample {
  up: number
  down: number
}

export interface ConnectionInfo {
  id: string
  metadata: {
    network: string
    kind: string
    source_ip: string
    destination_ip: string
    destination_port: string
    host: string
    /** Bare name when the core reports one (Mihomo on Windows/macOS). */
    process: string
    /** Full path when the core reports one (sing-box). */
    process_path: string
  }
  upload: number
  download: number
  chains: string[]
  rule: string
  rule_payload: string
  start: string
}

export interface ConnectionsSnapshot {
  upload_total: number
  download_total: number
  connections: ConnectionInfo[]
}

export interface LatencySample {
  node_id: string
  name: string
  endpoint: string
  kind: 'tcp'
  latency_ms: number | null
  error: string | null
  measured_at_ms: number
}

export interface LogEntry {
  seq: number
  ts_ms: number
  core: string
  level: 'debug' | 'info' | 'warn' | 'error'
  text: string
}

export type DnsMode = 'off' | 'split' | 'fake_ip'
export type RuleMode = 'global' | 'split'

export interface AppliedProxy {
  endpoint: string
  backend: string
  applied_at_ms: number
}

export interface Settings {
  theme: 'light' | 'dark' | 'system'
  core_preference: CoreKind | null
  tun: boolean
  rule_set: boolean
  proxy_groups: boolean
  allow_lan: boolean
  /** Install the system proxy automatically when a core starts. */
  system_proxy: boolean
  /** DNS strategy baked into the generated config. */
  dns_mode: DnsMode
  /** Routing strategy baked into the generated config. */
  rule_mode: RuleMode
  log_level: string
  /** Local inbound port for the mixed listener. */
  listen_port: number
  /** Hide to the tray instead of quitting when the window is closed. */
  close_to_tray: boolean
  /** Start a core as soon as the app launches. */
  auto_connect: boolean
  /** Semicolon-separated bypass list; null keeps the built-in one. */
  proxy_bypass: string | null
  /** User-written routing rules, one per line; null means none. */
  custom_rules: string | null
  /** Remote rule-sets, one per line: 目标,类型,URL; null means none. */
  rule_sets: string | null
}

export const CORE_LABEL: Record<CoreKind, string> = {
  mihomo: 'Mihomo',
  'sing-box': 'sing-box',
  xray: 'Xray',
}

export const STATE_LABEL: Record<CoreState, string> = {
  stopped: '已停止',
  starting: '启动中',
  running: '运行中',
  stopping: '停止中',
  failed: '失败',
}
