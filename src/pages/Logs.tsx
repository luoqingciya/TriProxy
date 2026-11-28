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

import { Copy, Search, Trash2, X } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'

import { PageHeader } from '../components/Sidebar'
import { Button, Card, Empty } from '../components/ui'
import { api } from '../services/api'
import { useApp } from '../store/app'
import type { LogEntry } from '../types'

const LEVELS = ['all', 'error', 'warn', 'info', 'debug'] as const
type LevelFilter = (typeof LEVELS)[number]
const MAX_LOG_ENTRIES = 300

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
  const [query, setQuery] = useState('')
  const [follow, setFollow] = useState(true)
  const [showChatter, setShowChatter] = useState(false)
  const [clearedBeforeSeq, setClearedBeforeSeq] = useState<number | null>(null)
  const bottom = useRef<HTMLDivElement>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const lastSeqRef = useRef<number | null>(null)

  useEffect(() => {
    let cancelled = false
    let timer: number | undefined
    const load = async () => {
      try {
        const next = await api.recentLogs(MAX_LOG_ENTRIES, lastSeqRef.current)
        if (cancelled || next.length === 0) return
        lastSeqRef.current = next[next.length - 1].seq
        setEntries((prev) =>
          prev.length === 0 ? next : [...prev, ...next].slice(-MAX_LOG_ENTRIES),
        )
      } catch {
        // The log panel is read-only; a failed poll is not worth an error banner.
      }
    }
    const stopTimer = () => {
      if (timer !== undefined) {
        window.clearInterval(timer)
        timer = undefined
      }
    }
    const startTimer = () => {
      stopTimer()
      if (document.hidden) return
      void load()
      timer = window.setInterval(() => void load(), 2000)
    }
    const onVisibility = () => {
      if (document.hidden) {
        stopTimer()
      } else {
        startTimer()
      }
    }
    startTimer()
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      cancelled = true
      stopTimer()
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [])

  const { chatterCount, visible } = useMemo(() => {
    let chatter = 0
    const needle = query.trim().toLowerCase()
    const out: LogEntry[] = []
    for (const entry of entries) {
      if (clearedBeforeSeq !== null && entry.seq <= clearedBeforeSeq) continue
      if (level !== 'all' && entry.level !== level) continue
      if (
        needle !== '' &&
        !entry.text.toLowerCase().includes(needle) &&
        !entry.core.toLowerCase().includes(needle)
      ) {
        continue
      }
      if (CHATTER.test(entry.text)) {
        chatter += 1
        if (showChatter) out.push(entry)
      } else {
        out.push(entry)
      }
    }
    return { chatterCount: chatter, visible: out }
  }, [entries, level, query, showChatter, clearedBeforeSeq])

  useEffect(() => {
    if (follow) {
      bottom.current?.scrollIntoView({ block: 'end' })
    }
  }, [visible.length, follow])

  const handleScroll = () => {
    const element = scrollRef.current
    if (!element) return
    const distanceFromBottom = element.scrollHeight - element.scrollTop - element.clientHeight
    if (distanceFromBottom > 40 && follow) {
      setFollow(false)
    } else if (distanceFromBottom <= 16 && !follow) {
      setFollow(true)
    }
  }

  const copyLogs = async () => {
    if (visible.length === 0) return
    const text = visible
      .map(
        (entry) =>
          `${formatTime(entry.ts_ms)} [${entry.core}] ${entry.level.toUpperCase()} ${entry.text}`,
      )
      .join('\n')
    try {
      await navigator.clipboard.writeText(text)
      useApp.setState({ notice: `已复制 ${visible.length} 条日志` })
    } catch {
      useApp.setState({ error: '无法访问剪贴板' })
    }
  }

  const clearLogs = () => {
    const latest = entries[entries.length - 1]?.seq
    if (latest !== undefined) {
      setClearedBeforeSeq(latest)
    }
  }

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title="日志"
        description="核心进程的实时输出（上滑可暂停自动滚动）"
        actions={
          <>
            <div className="relative w-[190px]">
              <Search
                size={13}
                className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-faint"
              />
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Escape' && query) setQuery('')
                }}
                placeholder="筛选关键字"
                className="w-full rounded-lg border border-line bg-panel py-1.5 pr-7 pl-8 text-[12.5px] text-fg outline-none placeholder:text-faint focus:border-line-strong"
              />
              {query && (
                <button
                  type="button"
                  onClick={() => setQuery('')}
                  aria-label="清空搜索"
                  className="absolute top-1/2 right-2 -translate-y-1/2 text-faint hover:text-fg"
                >
                  <X size={12} />
                </button>
              )}
            </div>
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
            <Button
              onClick={() => void copyLogs()}
              disabled={visible.length === 0}
              title="复制当前可见日志"
              className="px-2.5"
            >
              <Copy size={14} />
            </Button>
            <Button
              onClick={clearLogs}
              disabled={visible.length === 0}
              title="清屏（仅隐藏当前已显示的日志）"
              className="px-2.5"
            >
              <Trash2 size={14} />
            </Button>
          </>
        }
      />

      <Card className="min-h-0 flex-1 overflow-hidden p-0">
        <div
          ref={scrollRef}
          onScroll={handleScroll}
          className="h-full overflow-auto select-text"
        >
          {visible.length === 0 ? (
            <Empty>
              {chatterCount > 0
                ? `只有 ${chatterCount} 条连接日志，已折叠 — 点右上角可展开`
                : query.trim()
                  ? '没有匹配的日志'
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
        </div>
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
