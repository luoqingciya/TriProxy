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

import { ChevronDown, ChevronRight, Power, Zap } from 'lucide-react'
import { useState } from 'react'

import { useApp } from '../store/app'
import { CORE_LABEL, STATE_LABEL, type Selection } from '../types'
import { Badge, Button, Card, SectionTitle, StateDot } from '../components/ui'
import { PageHeader } from '../components/Sidebar'

function formatRate(bytesPerSecond: number): string {
  if (bytesPerSecond < 1024) return `${bytesPerSecond} B/s`
  const units = ['KB/s', 'MB/s', 'GB/s']
  let value = bytesPerSecond / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value.toFixed(1)} ${units[unit]}`
}

function formatUptime(seconds: number | null | undefined): string {
  if (!seconds) return '—'
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  if (h > 0) return `${h} 小时 ${m} 分`
  if (m > 0) return `${m} 分钟`
  return `${seconds} 秒`
}

/**
 * Render the selection rationale in the UI language.
 *
 * The backend's `reason` is a canonical English sentence intended for logs and
 * `--json`; showing it verbatim in a Chinese interface reads like a leaked
 * debug string. The structured evaluations carry everything needed to phrase it
 * locally.
 */
function describeSelection(selection: Selection): string {
  const winner = selection.evaluations.find((entry) => entry.core === selection.selected)
  const parts: string[] = []

  if (winner && winner.native_features.length > 0) {
    parts.push(
      `${CORE_LABEL[selection.selected]} 原生支持 ${winner.native_features.join(' + ')}`,
    )
  } else {
    parts.push(`${CORE_LABEL[selection.selected]} 是默认的通用核心，没有候选需要特殊能力`)
  }

  const eliminated = selection.evaluations.filter((entry) => !entry.eligible)
  if (eliminated.length > 0) {
    parts.push(
      `已排除 ${eliminated
        .map((entry) => `${CORE_LABEL[entry.core]}（缺少 ${entry.unmet.join('、')}）`)
        .join('、')}`,
    )
  }

  return parts.join('；')
}

export function Dashboard() {
  const status = useApp((state) => state.status)
  const library = useApp((state) => state.library)
  const selection = useApp((state) => state.selection)
  const settings = useApp((state) => state.settings)
  const proxy = useApp((state) => state.proxy)
  const traffic = useApp((state) => state.traffic)
  const trafficHistory = useApp((state) => state.trafficHistory)
  const busy = useApp((state) => state.busy)
  const start = useApp((state) => state.start)
  const switchTo = useApp((state) => state.switchTo)
  const installCore = useApp((state) => state.installCore)
  const updateCore = useApp((state) => state.updateCore)
  const stop = useApp((state) => state.stop)
  const [showWhy, setShowWhy] = useState(false)

  const running = status?.cores.find((core) => core.state === 'running')
  const connected = Boolean(running)
  const activeProfile = library.profiles.find((entry) => entry.id === library.active)

  return (
    <div>
      <PageHeader
        title="仪表盘"
        description="一眼看清当前连接、核心与配置"
        actions={
          <Button
            variant={connected ? 'ghost' : 'primary'}
            disabled={busy}
            onClick={() => (connected ? stop() : start(null))}
          >
            <Power size={15} strokeWidth={2.2} />
            {connected ? '断开' : '连接'}
          </Button>
        }
      />

      {/* Connection hero */}
      <Card className="mb-4">
        <div className="flex items-start justify-between gap-6">
          <div>
            <div className="flex items-center gap-2">
              <StateDot state={running?.state ?? 'stopped'} />
              <span className="text-[13px] font-medium text-muted">
                {connected ? '已连接' : '未连接'}
              </span>
            </div>
            <div className="mt-3 text-[26px] leading-none font-semibold tracking-tight text-fg">
              {activeProfile?.name ?? '未选择配置'}
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[13px] text-muted tabular">
              {connected ? (
                <>
                  <span>{CORE_LABEL[running!.kind]}</span>
                  <span className="text-faint">·</span>
                  <span>127.0.0.1:{running!.listen_port ?? 7890}</span>
                  <span className="text-faint">·</span>
                  <span>{running!.latency_ms ?? '—'} ms</span>
                  <span className="text-faint">·</span>
                  <span>已运行 {formatUptime(running!.uptime_secs)}</span>
                  {traffic && (
                    <>
                      <span className="text-faint">·</span>
                      <span>
                        ↑ {formatRate(traffic.up)} · ↓ {formatRate(traffic.down)}
                      </span>
                    </>
                  )}
                </>
              ) : (
                <span>选择一个配置后即可连接</span>
              )}
            </div>
          </div>

          {running && (
            <div className="flex flex-col items-end gap-2">
              <Badge tone="success">{STATE_LABEL[running.state]}</Badge>
              {running.binary_version && (
                <span className="text-[11.5px] text-faint tabular">
                  v{running.binary_version}
                </span>
              )}
            </div>
          )}
        </div>
      </Card>

      <div className="mb-4 grid grid-cols-3 gap-4">
        <Card>
          <SectionTitle title="当前配置" />
          {activeProfile ? (
            <div className="space-y-1.5 text-[13px]">
              <Row label="名称" value={activeProfile.name} />
              <Row label="节点" value={`${activeProfile.node_count}`} />
              <Row label="来源" value={activeProfile.source || '—'} />
            </div>
          ) : (
            <p className="text-[13px] text-faint">还没有配置，去「配置」页导入订阅</p>
          )}
        </Card>

        <Card>
          <SectionTitle title="网络模式" />
          <div className="space-y-1.5 text-[13px]">
            <Row
              label="系统代理"
              value={proxy ? proxy.endpoint : "未启用"}
            />
            <Row
              label="TUN"
              value={settings?.tun ? "已启用" : "未启用"}
              hint={settings?.tun ? "需要管理员" : undefined}
            />
            <Row label="分流规则" value={settings?.rule_set ? '已启用' : '未启用'} />
          </div>
        </Card>

        <Card>
          <SectionTitle title="流量" />
          {traffic ? (
            <>
              <div className="mb-2 flex items-baseline justify-between text-[13px] tabular">
                <span className="text-muted">↑ {formatRate(traffic.up)}</span>
                <span className="text-muted">↓ {formatRate(traffic.down)}</span>
              </div>
              <Sparkline
                up={trafficHistory.map((sample) => sample.up)}
                down={trafficHistory.map((sample) => sample.down)}
              />
              <div className="mt-1 text-[11px] text-faint">最近 3 分钟</div>
            </>
          ) : (
            <p className="text-[13px] text-faint">核心未运行</p>
          )}
        </Card>
      </div>

      {/* Smart Engine */}
      <Card>
        <div className="flex items-start justify-between gap-4">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 flex h-7 w-7 items-center justify-center rounded-lg bg-accent-soft">
              <Zap size={15} className="text-accent" strokeWidth={2.2} />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <span className="text-[14px] font-medium text-fg">Smart Engine</span>
                {selection && <Badge tone="accent">{CORE_LABEL[selection.selected]}</Badge>}
              </div>
              <p className="mt-1 max-w-[560px] text-[12.5px] leading-relaxed text-muted">
                {selection ? describeSelection(selection) : '没有可用的配置，无法评估核心'}
              </p>
            </div>
          </div>

          {selection && (
            <button
              onClick={() => setShowWhy((value) => !value)}
              className="flex shrink-0 items-center gap-1 text-[12.5px] text-accent hover:underline"
            >
              {showWhy ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              为什么是这个核心？
            </button>
          )}
        </div>

        {showWhy && selection && (
          <div className="mt-4 border-t border-line pt-4">
            <div className="grid grid-cols-[88px_92px_1fr] gap-y-2 text-[12.5px]">
              <div className="text-[11px] font-semibold tracking-wide text-faint uppercase">核心</div>
              <div className="text-[11px] font-semibold tracking-wide text-faint uppercase">结论</div>
              <div className="text-[11px] font-semibold tracking-wide text-faint uppercase">说明</div>
              {selection.evaluations.map((evaluation) => (
                <EvaluationRow key={evaluation.core} evaluation={evaluation} />
              ))}
            </div>
          </div>
        )}
      </Card>

      {/* Core roster: this is a multi-core client, so the state of all three
          cores is first-class information, and switching is one click away. */}
      <Card className="mt-4">
        <SectionTitle title="核心" hint="同一时刻只有一个核心运行" />
        <div className="grid grid-cols-[92px_110px_1fr_1fr_96px] gap-3 border-b border-line pb-2 text-[11px] font-semibold tracking-wide text-faint uppercase">
          <div>核心</div>
          <div>状态</div>
          <div>版本</div>
          <div>监听</div>
          <div className="text-right">操作</div>
        </div>
        {status?.cores.map((core) => {
          const running = core.state === 'running'
          return (
            <div
              key={core.kind}
              className="grid grid-cols-[92px_110px_1fr_1fr_96px] items-center gap-3 border-b border-line py-2.5 text-[13px] last:border-b-0"
            >
              <div className={running ? 'font-medium text-fg' : 'text-muted'}>
                {CORE_LABEL[core.kind]}
              </div>
              <div className="flex items-center gap-2">
                <StateDot state={core.state} />
                <span className="text-[12.5px] text-muted">{STATE_LABEL[core.state]}</span>
              </div>
              <div className="text-muted tabular">
                {core.binary_version ? (
                  `v${core.binary_version}`
                ) : (
                  // The lite installer ships no cores, so this cell is the whole
                  // first-run experience for anyone who picked it.
                  <button
                    disabled={busy}
                    onClick={() => void installCore(core.kind)}
                    className="text-[12px] text-accent hover:underline disabled:opacity-40"
                  >
                    {busy ? '下载中…' : '下载核心'}
                  </button>
                )}
              </div>
              <div className="text-muted tabular">
                {running ? `127.0.0.1:${core.listen_port ?? '—'}` : '—'}
              </div>
              <div className="flex justify-end gap-2 text-[12px]">
                {running ? (
                  <span className="text-accent">当前</span>
                ) : (
                  <>
                    <button
                      disabled={busy || !core.binary}
                      onClick={() => void switchTo(core.kind)}
                      className="text-muted hover:text-accent disabled:opacity-40"
                    >
                      切换
                    </button>
                    {core.binary ? (
                      <button
                        disabled={busy}
                        onClick={() => void updateCore(core.kind)}
                        className="text-faint hover:text-accent disabled:opacity-40"
                      >
                        更新
                      </button>
                    ) : null}
                  </>
                )}
              </div>
            </div>
          )
        })}
      </Card>
    </div>
  )
}

/**
 * Two-line traffic sparkline.
 *
 * Normalised to the busiest sample in the window rather than to an absolute
 * scale: a quiet connection still shows its shape, where a fixed scale would
 * flatten everything except the one burst that set the maximum.
 */
function Sparkline({ up, down }: { up: number[]; down: number[] }) {
  if (up.length < 2) {
    return <div className="h-12 rounded-md bg-panel-2" />
  }

  const max = Math.max(...up, ...down, 1)
  const points = (values: number[]) =>
    values
      .map((value, index) => {
        const x = (index / (values.length - 1)) * 100
        const y = 100 - (value / max) * 100
        return `${x.toFixed(2)},${y.toFixed(2)}`
      })
      .join(' ')

  return (
    <svg viewBox="0 0 100 100" preserveAspectRatio="none" className="h-12 w-full">
      <polyline
        points={points(down)}
        fill="none"
        stroke="currentColor"
        strokeWidth={1.5}
        vectorEffect="non-scaling-stroke"
        className="text-accent"
      />
      <polyline
        points={points(up)}
        fill="none"
        stroke="currentColor"
        strokeWidth={1.5}
        vectorEffect="non-scaling-stroke"
        className="text-muted"
      />
    </svg>
  )
}

function Row({
  label,
  value,
  hint,
}: {
  label: string
  value: string
  hint?: string
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <span className="text-muted">
        {label}
        {hint && <span className="ml-1.5 text-[11px] text-faint">{hint}</span>}
      </span>
      <span className="truncate text-fg">{value}</span>
    </div>
  )
}

function EvaluationRow({
  evaluation,
}: {
  evaluation: {
    core: 'mihomo' | 'sing-box' | 'xray'
    eligible: boolean
    unmet: string[]
    native_features: string[]
  }
}) {
  const selected = useApp((state) => state.selection?.selected === evaluation.core)
  const verdict = selected ? '已选中' : evaluation.eligible ? '可用' : '不可用'
  const detail = evaluation.eligible
    ? evaluation.native_features.length > 0
      ? `原生支持 ${evaluation.native_features.join(' + ')}`
      : '无需特殊能力'
    : `缺少 ${evaluation.unmet.join('、')}`

  return (
    <>
      <div className={selected ? 'font-medium text-fg' : 'text-muted'}>
        {CORE_LABEL[evaluation.core]}
      </div>
      <div>
        <Badge tone={selected ? 'accent' : evaluation.eligible ? 'neutral' : 'danger'}>
          {verdict}
        </Badge>
      </div>
      <div className="text-muted">{detail}</div>
    </>
  )
}
