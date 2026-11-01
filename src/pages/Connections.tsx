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

import { X } from 'lucide-react'

import { PageHeader } from '../components/Sidebar'
import { Badge, Button, Card, Empty } from '../components/ui'
import { useApp } from '../store/app'
import { CORE_LABEL } from '../types'

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB']
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value.toFixed(1)} ${units[unit]}`
}

export function Connections() {
  const snapshot = useApp((state) => state.connections)
  const status = useApp((state) => state.status)
  const closeConnection = useApp((state) => state.closeConnection)

  const active = status?.cores.find((core) => core.state === 'running')

  return (
    <div>
      <PageHeader
        title="连接"
        description={
          snapshot
            ? `共 ${snapshot.connections.length} 条 · 累计 ↑ ${formatBytes(snapshot.upload_total)} / ↓ ${formatBytes(snapshot.download_total)}`
            : '实时连接列表'
        }
        actions={
          <Button
            variant="danger"
            disabled={!snapshot || snapshot.connections.length === 0}
            onClick={() => closeConnection(null)}
          >
            关闭全部
          </Button>
        }
      />

      <Card className="p-0">
        {!snapshot ? (
          <Empty>
            {active
              ? `${CORE_LABEL[active.kind]} 没有控制器 API，无法显示连接。该功能仅支持 Mihomo 与 sing-box。`
              : '核心未运行，或当前核心不支持控制器 API'}
          </Empty>
        ) : snapshot.connections.length === 0 ? (
          <Empty>当前没有活动连接</Empty>
        ) : (
          <div>
            <div className="grid grid-cols-[1fr_72px_1fr_84px_84px_52px] gap-4 border-b border-line px-5 py-2.5 text-[11px] font-semibold tracking-wide text-faint uppercase">
              <div>目标</div>
              <div>网络</div>
              <div>链路</div>
              <div className="text-right">上传</div>
              <div className="text-right">下载</div>
              <div />
            </div>
            {snapshot.connections.map((connection) => (
              <div
                key={connection.id}
                className="grid grid-cols-[1fr_72px_1fr_84px_84px_52px] items-center gap-4 border-b border-line px-5 py-2.5 text-[13px] last:border-b-0 hover:bg-panel-hover"
              >
                <div className="truncate text-fg">
                  {connection.metadata.host || connection.metadata.destination_ip}
                  {connection.metadata.destination_port && (
                    <span className="text-faint">
                      :{connection.metadata.destination_port}
                    </span>
                  )}
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
            ))}
          </div>
        )}
      </Card>

      <p className="mt-3 text-[12px] text-faint">
        数据来自核心的控制器 API，每秒随状态刷新。Xray 没有这套 API，因此在 Xray 下此页为空。
      </p>
    </div>
  )
}
