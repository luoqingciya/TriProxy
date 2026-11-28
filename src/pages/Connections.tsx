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

import { Search, X } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'

import { PageHeader } from '../components/Sidebar'
import { VirtualList } from '../components/VirtualList'
import { Badge, Button, Card, Empty } from '../components/ui'
import { formatBytes } from '../format'
import { useApp } from '../store/app'
import { CORE_LABEL } from '../types'

type ConnectionSort = 'default' | 'download' | 'upload'

/**
 * Bare executable name for a connection.
 *
 * Mihomo fills `process` (a name), sing-box fills `processPath` (a full path);
 * the list wants the same thing from both.
 */
function processName(metadata: {
  process: string
  process_path: string
}): string {
  const raw = metadata.process || metadata.process_path || ''
  return raw.split(/[\\/]/).pop() ?? ''
}

export function Connections() {
  const snapshot = useApp((state) => state.connections)
  const activeKind = useApp((state) => state.status?.active_core ?? null)
  const closeConnection = useApp((state) => state.closeConnection)
  const [query, setQuery] = useState('')
  const [sortBy, setSortBy] = useState<ConnectionSort>('default')
  const [confirmCloseAll, setConfirmCloseAll] = useState(false)

  useEffect(() => {
    if (!confirmCloseAll) return undefined
    const timer = window.setTimeout(() => setConfirmCloseAll(false), 3000)
    return () => window.clearTimeout(timer)
  }, [confirmCloseAll])

  const visible = useMemo(() => {
    const list = snapshot?.connections ?? []
    const needle = query.trim().toLowerCase()
    const filtered =
      needle === ''
        ? [...list]
        : list.filter((connection) => {
            const target =
              (connection.metadata.host || connection.metadata.destination_ip) +
              (connection.metadata.destination_port
                ? `:${connection.metadata.destination_port}`
                : '')
            return (
              target.toLowerCase().includes(needle) ||
              processName(connection.metadata).toLowerCase().includes(needle) ||
              connection.rule.toLowerCase().includes(needle) ||
              connection.rule_payload.toLowerCase().includes(needle) ||
              connection.chains.some((chain) => chain.toLowerCase().includes(needle))
            )
          })
    if (sortBy === 'download') {
      filtered.sort((a, b) => b.download - a.download)
    } else if (sortBy === 'upload') {
      filtered.sort((a, b) => b.upload - a.upload)
    }
    return filtered
  }, [snapshot, query, sortBy])

  const handleCloseAll = () => {
    if (!confirmCloseAll) {
      setConfirmCloseAll(true)
      return
    }
    setConfirmCloseAll(false)
    void closeConnection(null)
  }

  return (
    <div>
      <PageHeader
        title="连接"
        description={
          snapshot
            ? query.trim()
              ? `匹配 ${visible.length} / ${snapshot.connections.length} 条 · 累计 ↑ ${formatBytes(snapshot.upload_total)} / ↓ ${formatBytes(snapshot.download_total)}`
              : `共 ${snapshot.connections.length} 条 · 累计 ↑ ${formatBytes(snapshot.upload_total)} / ↓ ${formatBytes(snapshot.download_total)}`
            : '实时连接列表'
        }
        actions={
          <Button
            variant="danger"
            disabled={!snapshot || snapshot.connections.length === 0}
            onClick={handleCloseAll}
          >
            {confirmCloseAll ? '确认关闭全部？' : '关闭全部'}
          </Button>
        }
      />

      {snapshot && snapshot.connections.length > 0 && (
        <div className="mb-4 flex items-center gap-2">
          <div className="relative flex-1">
            <Search
              size={15}
              className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-faint"
            />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Escape' && query) setQuery('')
              }}
              placeholder="搜索目标域名、IP、进程、规则或链路"
              className="w-full rounded-lg border border-line bg-panel py-2 pr-8 pl-9 text-[13px] text-fg outline-none placeholder:text-faint focus:border-line-strong"
            />
            {query && (
              <button
                type="button"
                onClick={() => setQuery('')}
                aria-label="清空搜索"
                className="absolute top-1/2 right-2.5 -translate-y-1/2 text-faint hover:text-fg"
              >
                <X size={13} />
              </button>
            )}
          </div>
          <div className="flex items-center gap-1 rounded-lg border border-line bg-panel p-1 text-[12px]">
            {(
              [
                ['default', '默认'],
                ['download', '下载最多'],
                ['upload', '上传最多'],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                type="button"
                onClick={() => setSortBy(key)}
                className={`rounded-md px-2 py-1 transition-colors ${
                  sortBy === key ? 'bg-accent-soft text-accent' : 'text-muted hover:text-fg'
                }`}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
      )}

      <Card className="p-0">
        {!snapshot ? (
          <Empty>
            {activeKind
              ? `${CORE_LABEL[activeKind]} 没有控制器 API，无法显示连接。该功能仅支持 Mihomo 与 sing-box。`
              : '核心未运行，或当前核心不支持控制器 API'}
          </Empty>
        ) : visible.length === 0 ? (
          <Empty>{snapshot.connections.length > 0 ? '没有匹配的连接' : '当前没有活动连接'}</Empty>
        ) : (
          <div>
            <div className="grid grid-cols-[1fr_110px_72px_1fr_84px_84px_52px] gap-4 border-b border-line px-5 py-2.5 text-[11px] font-semibold tracking-wide text-faint uppercase">
              <div>目标</div>
              <div>进程</div>
              <div>网络</div>
              <div>链路</div>
              <div className="text-right">上传</div>
              <div className="text-right">下载</div>
              <div />
            </div>
            <VirtualList
              items={visible}
              rowHeight={56}
              maxHeightClass="max-h-[calc(100vh-250px)]"
              renderRow={(connection) => (
                <div
                  key={connection.id}
                  className="grid h-[56px] grid-cols-[1fr_110px_72px_1fr_84px_84px_52px] items-center gap-4 border-b border-line px-5 text-[13px] last:border-b-0 hover:bg-panel-hover"
                >
                  <div className="min-w-0 select-text">
                    <div className="truncate text-fg">
                      {connection.metadata.host || connection.metadata.destination_ip}
                      {connection.metadata.destination_port && (
                        <span className="text-faint">
                          :{connection.metadata.destination_port}
                        </span>
                      )}
                    </div>
                    {/* Which rule caught this is the first thing anyone debugging
                        "why did this site go direct" wants to see. */}
                    {connection.rule && (
                      <div className="truncate text-[11px] text-faint">
                        {connection.rule}
                        {connection.rule_payload ? ` · ${connection.rule_payload}` : ''}
                      </div>
                    )}
                  </div>
                  <div className="truncate text-muted">
                    {processName(connection.metadata) || '—'}
                  </div>
                  <div>
                    <Badge>{connection.metadata.network || '—'}</Badge>
                  </div>
                  <div className="truncate text-muted">
                    {connection.chains.length > 0 ? connection.chains.join(' ← ') : '—'}
                  </div>
                  <div className="text-right text-muted tabular">
                    {formatBytes(connection.upload)}
                  </div>
                  <div className="text-right text-muted tabular">
                    {formatBytes(connection.download)}
                  </div>
                  <div className="text-right">
                    <button
                      onClick={() => closeConnection(connection.id)}
                      className="text-faint hover:text-danger"
                      title="关闭此连接"
                    >
                      <X size={14} />
                    </button>
                  </div>
                </div>
              )}
            />
          </div>
        )}
      </Card>

      <p className="mt-3 text-[12px] text-faint">
        数据来自核心的控制器 API，每 2 秒随状态刷新。Xray 没有这套 API，因此在 Xray 下此页为空。
      </p>
    </div>
  )
}
