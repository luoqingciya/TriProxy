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

import { useEffect, useRef, useState } from 'react'

import { PageHeader } from '../components/Sidebar'
import { Button, Card, Empty } from '../components/ui'
import { api } from '../services/api'
import type { LogEntry } from '../types'

const LEVELS = ['all', 'error', 'warn', 'info', 'debug'] as const
type LevelFilter = (typeof LEVELS)[number]

const LEVEL_STYLE: Record<LogEntry['level'], string> = {
  error: 'text-danger',
  warn: 'text-warn',
  info: 'text-muted',
  debug: 'text-faint',
}

/**
 * Per-connection chatter.
 *
 * sing-box logs one INFO line for every accepted inbound connection and xray
 * does the same for `accepted tcp:`/`accepted udp:`. With the local health probe
 * and any app behind the system proxy, that is a line every couple of seconds —
 * it drowns out everything the user actually wants to read. The live connection
 * list on the Connections page already covers this data, so these lines are
 * hidden by default and can be revealed with one click.
 */
const CHATTER = /inbound connection from|accepted (?:tcp|udp):/i

export function Logs() {
  const [entries, setEntries] = useState<LogEntry[]>([])
  const [level, setLevel] = useState<LevelFilter>('all')
  const [follow, setFollow] = useState(true)
  const [showChatter, setShowChatter] = useState(false)
  const bottom = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let cancelled = false
    const load = async () => {
      try {
        const next = await api.recentLogs(300)
        if (!cancelled) setEntries(next)
      } catch {
        // The log panel is read-only; a failed poll is not worth an error banner.
      }
    }
    void load()
    const timer = window.setInterval(load, 2000)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [])

  const matched = entries.filter((entry) => level === 'all' || entry.level === level)
  const chatterCount = matched.filter((entry) => CHATTER.test(entry.text)).length
  const visible = showChatter
    ? matched
    : matched.filter((entry) => !CHATTER.test(entry.text))

  useEffect(() => {
    if (follow) {
      bottom.current?.scrollIntoView({ block: 'end' })
    }
  }, [visible.length, follow])

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title="日志"
        description="核心进程的实时输出"
        actions={
          <>
            <div className="flex items-center gap-1 rounded-lg border border-line bg-panel p-1">
              {LEVELS.map((entry) => (
                <button
                  key={entry}
                  onClick={() => setLevel(entry)}
                  className={`rounded-md px-2 py-1 text-[12px] transition-colors ${
                    level === entry ? 'bg-accent-soft text-accent' : 'text-muted hover:text-fg'
                  }`}
                >
                  {entry === 'all' ? '全部' : entry}
                </button>
              ))}
            </div>
            <Button onClick={() => setShowChatter((value) => !value)}>
              {showChatter ? '连接日志：显示中' : `连接日志：已隐藏 ${chatterCount} 条`}
            </Button>
            <Button onClick={() => setFollow((value) => !value)}>
              {follow ? '自动滚动：开' : '自动滚动：关'}
            </Button>
          </>
        }
      />

      <Card className="min-h-0 flex-1 overflow-auto p-0">
        {visible.length === 0 ? (
          <Empty>
            {chatterCount > 0
              ? `只有 ${chatterCount} 条连接日志，已折叠 — 点右上角可展开`
              : '暂无日志 — 启动核心后这里会实时刷新'}
          </Empty>
        ) : (
          <div className="p-4 font-mono text-[12px] leading-relaxed">
            {visible.map((entry) => (
              <div key={entry.seq} className="flex gap-3">
                <span className="shrink-0 text-faint tabular">{formatTime(entry.ts_ms)}</span>
                <span className="w-16 shrink-0 truncate text-faint">{entry.core}</span>
                <span className={`w-10 shrink-0 uppercase ${LEVEL_STYLE[entry.level]}`}>
                  {entry.level.slice(0, 3)}
                </span>
                <span className="whitespace-pre-wrap break-all text-fg">{entry.text}</span>
              </div>
            ))}
            <div ref={bottom} />
          </div>
        )}
      </Card>
    </div>
  )
}

function formatTime(tsMs: number): string {
  const date = new Date(tsMs)
  return [date.getHours(), date.getMinutes(), date.getSeconds()]
    .map((value) => String(value).padStart(2, '0'))
    .join(':')
}
