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

export interface ManagerStatus {
  active_core: CoreKind | null
  exclusive: boolean
  data_dir: string
  cores: CoreReport[]
}

export interface StoredProfile {
  id: string
  name: string
  source: string
  format: 'uri_list' | 'base64_subscription'
  node_count: number
  content_id: string
  updated_at_ms: number
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

export interface LogEntry {
  seq: number
  ts_ms: number
  core: string
  level: 'debug' | 'info' | 'warn' | 'error'
  text: string
}

export interface Settings {
  theme: 'light' | 'dark' | 'system'
  core_preference: CoreKind | null
  tun: boolean
  rule_set: boolean
  proxy_groups: boolean
  allow_lan: boolean
  log_level: string
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
