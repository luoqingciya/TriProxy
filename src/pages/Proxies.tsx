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

import { Activity, Search } from 'lucide-react'
import { useMemo, useState } from 'react'

import { PageHeader } from '../components/Sidebar'
import { Badge, Button, Card, Empty } from '../components/ui'
import { useApp } from '../store/app'
import type { LatencySample, Protocol } from '../types'

const PROTOCOL_LABEL: Record<Protocol, string> = {
  shadowsocks: 'SS',
  vmess: 'VMess',
  vless: 'VLESS',
  trojan: 'Trojan',
  wireguard: 'WireGuard',
  hysteria2: 'Hysteria2',
  tuic: 'TUIC',
  socks: 'SOCKS',
  http: 'HTTP',
}

export function Proxies() {
  const nodes = useApp((state) => state.nodes)
  const library = useApp((state) => state.library)
  const latency = useApp((state) => state.latency)
  const testing = useApp((state) => state.testing)
  const testLatency = useApp((state) => state.testLatency)
  const testProxyDelay = useApp((state) => state.testProxyDelay)
  const [query, setQuery] = useState('')
  const [protocol, setProtocol] = useState<Protocol | 'all'>('all')

  const activeProfile = library.profiles.find((entry) => entry.id === library.active)

  const protocols = useMemo(() => {
    const seen = new Set<Protocol>()
    nodes.forEach((node) => seen.add(node.protocol))
    return Array.from(seen)
  }, [nodes])

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return nodes
      .filter((node) => (protocol === 'all' ? true : node.protocol === protocol))
      .filter((node) =>
        needle === ''
          ? true
          : node.name.toLowerCase().includes(needle) ||
            node.server.toLowerCase().includes(needle),
      )
      .sort((a, b) => a.name.localeCompare(b.name))
  }, [nodes, protocol, query])

  return (
    <div>
      <PageHeader
        title="节点"
        description={
          activeProfile
            ? `来自配置「${activeProfile.name}」，共 ${nodes.length} 个节点`
            : '还没有活动配置'
        }
        actions={
          <>
            <Button
              onClick={() => testLatency()}
              disabled={testing || nodes.length === 0}
              title="TCP 握手：只说明端点可达"
            >
              <Activity size={15} strokeWidth={2.2} />
              TCP 延迟
            </Button>
            <Button
              variant="primary"
              onClick={() => testProxyDelay()}
              disabled={testing || nodes.length === 0}
              title="由核心通过节点发起请求：能证明代理真的可用"
            >
              <Activity size={15} strokeWidth={2.2} />
              {testing ? '测试中…' : '真实延迟'}
            </Button>
          </>
        }
      />

      <div className="mb-4 flex items-center gap-2">
        <div className="relative flex-1">
          <Search
            size={15}
            className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-faint"
          />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索节点名称或地址"
            className="w-full rounded-lg border border-line bg-panel py-2 pr-3 pl-9 text-[13px] text-fg outline-none placeholder:text-faint focus:border-line-strong"
          />
        </div>
        <div className="flex items-center gap-1 rounded-lg border border-line bg-panel p-1">
          <FilterChip
            active={protocol === 'all'}
            onClick={() => setProtocol('all')}
            label="全部"
          />
          {protocols.map((entry) => (
            <FilterChip
              key={entry}
              active={protocol === entry}
              onClick={() => setProtocol(entry)}
              label={PROTOCOL_LABEL[entry]}
            />
          ))}
        </div>
      </div>

      <Card className="p-0">
        {visible.length === 0 ? (
          <Empty>
            {nodes.length > 0
              ? '没有匹配的节点'
              : library.profiles.length > 0
                ? '已有配置，但没有启用任何一份。到「配置」页点「设为活动」后，节点会显示在这里'
                : '还没有配置。到「配置」页导入订阅或分享链接'}
          </Empty>
        ) : (
          <div>
            <div className="grid grid-cols-[1fr_88px_1fr_92px_76px] gap-4 border-b border-line px-5 py-2.5 text-[11px] font-semibold tracking-wide text-faint uppercase">
              <div>名称</div>
              <div>协议</div>
              <div>地址</div>
              <div>传输</div>
              <div className="text-right">延迟</div>
            </div>
            {visible.map((node) => (
              <div
                key={node.id}
                className="grid grid-cols-[1fr_88px_1fr_92px_76px] items-center gap-4 border-b border-line px-5 py-3 text-[13px] last:border-b-0 hover:bg-panel-hover"
              >
                <div className="truncate text-fg">{node.name}</div>
                <div>
                  <Badge>{PROTOCOL_LABEL[node.protocol]}</Badge>
                </div>
                <div className="truncate text-muted tabular">
                  {node.server}:{node.port}
                </div>
                <div className="text-muted">
                  {node.transport}
                  {node.security !== '-' && (
                    <span className="ml-1 text-faint">/{node.security}</span>
                  )}
                </div>
                <div className="text-right tabular">
                  <LatencyCell sample={latency[node.id]} />
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>

      <p className="mt-3 text-[12px] text-faint">
        TCP 延迟只说明端点可达（凭据错误也会握手成功）；真实延迟由核心通过节点发起请求，
        能证明代理真的可用，但需要运行中的 Mihomo 或 sing-box（Xray 没有控制器 API）。
      </p>
    </div>
  )
}

/**
 * Latency cell.
 *
 * The colour thresholds are about distance, not quality: under 200 ms is a
 * nearby endpoint, over 500 ms is far away. A failed probe says 超时 and keeps
 * the reason in the tooltip rather than inventing a number.
 */
function LatencyCell({ sample }: { sample?: LatencySample }) {
  if (!sample) return <span className="text-faint">—</span>
  if (sample.latency_ms === null) {
    return (
      <span className="text-danger" title={sample.error ?? undefined}>
        超时
      </span>
    )
  }
  const ms = sample.latency_ms
  const tone = ms < 200 ? 'text-success' : ms < 500 ? 'text-warn' : 'text-danger'
  return <span className={tone}>{ms} ms</span>
}

function FilterChip({
  active,
  onClick,
  label,
}: {
  active: boolean
  onClick: () => void
  label: string
}) {
  return (
    <button
      onClick={onClick}
      className={`rounded-md px-2 py-1 text-[12px] transition-colors ${
        active ? 'bg-accent-soft text-accent' : 'text-muted hover:text-fg'
      }`}
    >
      {label}
    </button>
  )
}
