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

import { ChevronDown, ChevronRight, Copy, Power, RotateCw, Zap } from 'lucide-react'
import { useEffect, useState } from 'react'

import { useApp } from '../store/app'
import { CORE_LABEL, STATE_LABEL, type Selection } from '../types'
import { Badge, Button, Card, SectionTitle, StateDot, Toggle } from '../components/ui'
import { PageHeader } from '../components/Sidebar'
import { formatMemory, formatRate, formatUptime } from '../format'

/**
 * How often the active node is re-measured.
 *
 * Each measurement is a real HTTP request through the proxy, so this cannot ride
 * on the two-second status poll: that would turn a status indicator into a
 * steady stream of requests and distort the traffic graph it sits next to.
 */
const DELAY_INTERVAL_MS = 15_000

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
  const nodes = useApp((state) => state.nodes)
  const selection = useApp((state) => state.selection)
  const settings = useApp((state) => state.settings)
  const tunStatus = useApp((state) => state.tunStatus)
  const proxy = useApp((state) => state.proxy)
  const traffic = useApp((state) => state.traffic)
  const trafficHistory = useApp((state) => state.trafficHistory)
  const delay = useApp((state) => state.delay)
  const measureDelay = useApp((state) => state.measureDelay)
  const busy = useApp((state) => state.busy)
  const busyAction = useApp((state) => state.busyAction)
  const needsCoreRestart = useApp((state) => state.needsCoreRestart)
  const applySettingsRestart = useApp((state) => state.applySettingsRestart)
  const setPage = useApp((state) => state.setPage)
  const setSystemProxy = useApp((state) => state.setSystemProxy)
  const setTun = useApp((state) => state.setTun)
  const setRuleMode = useApp((state) => state.setRuleMode)
  const start = useApp((state) => state.start)
  const switchTo = useApp((state) => state.switchTo)
  const installCore = useApp((state) => state.installCore)
  const updateCore = useApp((state) => state.updateCore)
  const stop = useApp((state) => state.stop)
  const [showWhy, setShowWhy] = useState(false)

  const running = status?.cores.find((core) => core.state === 'running')
  const connected = Boolean(running)
  const activeProfile = library.profiles.find((entry) => entry.id === library.active)
  const activeNodeName = activeProfile?.selected_node
    ? (nodes.find((node) => node.id === activeProfile.selected_node)?.name ?? '未知节点')
    : '自动选择'

  const copyEndpoint = async (endpoint: string) => {
    try {
      await navigator.clipboard.writeText(endpoint)
      useApp.setState({ notice: `已复制代理地址 ${endpoint}` })
    } catch {
      useApp.setState({ error: '无法访问剪贴板' })
    }
  }

  // Measure only while something is running: a stopped core has no node to
  // measure, and the timer would just produce a stream of "unavailable" results.
  // Depending on the core as well as the boolean means switching cores shows the
  // new node's figure instead of the previous core's for a quarter of a minute.
  const runningKind = running?.kind ?? null
  useEffect(() => {
    if (!runningKind) return undefined
    let timer: number | undefined
    const stopTimer = () => {
      if (timer !== undefined) {
        window.clearInterval(timer)
        timer = undefined
      }
    }
    const startTimer = () => {
      stopTimer()
      if (document.hidden) return
      void measureDelay()
      timer = window.setInterval(() => void measureDelay(), DELAY_INTERVAL_MS)
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
      stopTimer()
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [runningKind, measureDelay])

  return (
    <div>
      <PageHeader
        title="仪表盘"
        description="一眼看清当前连接、核心与配置"
        actions={
          !connected && !activeProfile ? (
            <Button variant="primary" onClick={() => setPage('profiles')}>
              导入配置
            </Button>
          ) : (
            <Button
              variant={connected ? 'ghost' : 'primary'}
              disabled={busy}
              onClick={() => (connected ? stop() : start(null))}
            >
              <Power size={15} strokeWidth={2.2} />
              {busyAction === 'connect'
                ? '连接中…'
                : busyAction === 'disconnect'
                  ? '断开中…'
                  : connected
                    ? '断开'
                    : '连接'}
            </Button>
          )
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
                  <button
                    type="button"
                    onClick={() => setPage('proxies')}
                    title="点击切换节点"
                    className="underline decoration-dotted decoration-line underline-offset-4 hover:text-fg"
                  >
                    节点 {activeNodeName}
                  </button>
                  <span className="text-faint">·</span>
                  <button
                    type="button"
                    onClick={() =>
                      void copyEndpoint(`127.0.0.1:${running!.listen_port ?? 7890}`)
                    }
                    title="点击复制代理地址"
                    className="inline-flex items-center gap-1 hover:text-fg"
                  >
                    <span>127.0.0.1:{running!.listen_port ?? 7890}</span>
                    <Copy size={11} className="text-faint" />
                  </button>
                  <span className="text-faint">·</span>
                  {/*
                    Real proxy delay, not `running.latency_ms`: that field is the
                    handshake against the loopback inbound port, which is always
                    under a millisecond and was rendered as "0 ms" on every
                    connection. A dash with the reason in the tooltip is more
                    useful than a number that cannot be wrong because it never
                    measures anything.
                  */}
                  <button
                    onClick={() => void measureDelay()}
                    title={
                      delay?.error ??
                      (delay?.node
                        ? `当前节点 ${delay.node}，点击重新测量`
                        : '点击测量当前节点延迟')
                    }
                    className="underline decoration-dotted decoration-line underline-offset-4 hover:text-fg"
                  >
                    {delay?.latency_ms != null ? `延迟 ${delay.latency_ms} ms` : '延迟 —'}
                  </button>
                  <span className="text-faint">·</span>
                  <span title="核心进程的常驻内存（RSS）">
                    内存 {formatMemory(running!.memory_bytes)}
                  </span>
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
                <span>
                  {activeProfile
                    ? `就绪 · 节点「${activeNodeName}」，点右上角「连接」即可开始`
                    : '导入并启用配置后即可连接'}
                </span>
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

      {connected && needsCoreRestart && (
        <div className="mb-4 flex items-center justify-between gap-4 rounded-xl border border-accent/30 bg-accent-soft px-4 py-2.5 text-[13px]">
          <span className="text-fg">配置或网络设置已更改，重启核心后生效</span>
          <Button
            variant="primary"
            disabled={busy}
            onClick={() => void applySettingsRestart()}
            className="px-3 py-1.5 text-[12.5px]"
          >
            <RotateCw
              size={13}
              className={busyAction?.startsWith('switch:') ? 'animate-spin' : ''}
            />
            {busyAction?.startsWith('switch:') ? '重启中…' : '立即重启生效'}
          </Button>
        </div>
      )}

      <div className="mb-4 grid grid-cols-3 gap-4">
        <Card>
          <SectionTitle title="当前配置" />
          {activeProfile ? (
            <div className="space-y-1.5 text-[13px]">
              <Row label="名称" value={activeProfile.name} />
              <Row label="节点" value={`${activeProfile.node_count} 个`} />
              <Row
                label="规则"
                value={
                  (activeProfile.rule_count ?? 0) > 0
                    ? `订阅规则 ${activeProfile.rule_count} 条`
                    : '内置分流规则'
                }
              />
              <Row
                label="来源"
                value={
                  activeProfile.source === 'pasted'
                    ? '本地导入'
                    : activeProfile.source || '—'
                }
              />
            </div>
          ) : (
            <p className="text-[13px] text-faint">
              还没有配置，
              <button
                type="button"
                onClick={() => setPage('profiles')}
                className="text-accent hover:underline"
              >
                去「配置」页导入订阅
              </button>
            </p>
          )}
        </Card>

        <Card>
          <SectionTitle title="网络模式" />
          <div className="space-y-2 text-[13px]">
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted">
                系统代理
                {proxy && (
                  <span className="ml-1.5 text-[11px] text-faint">{proxy.endpoint}</span>
                )}
              </span>
              <span title={!connected ? '请先连接核心' : undefined}>
                <Toggle
                  checked={proxy !== null}
                  disabled={!connected || busy}
                  onChange={(enabled) => void setSystemProxy(enabled)}
                />
              </span>
            </div>
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted">
                TUN 模式
                <span className="ml-1.5 text-[11px] text-faint">
                  {busyAction === 'tun'
                    ? '授权中…'
                    : settings?.tun
                      ? `网卡 ${tunStatus?.interface_name ?? 'triproxy'}`
                      : tunStatus?.authorized
                        ? '已授权'
                        : '开启时弹窗提权'}
                </span>
              </span>
              <Toggle
                checked={settings?.tun ?? false}
                disabled={!settings || busy}
                onChange={(enabled) => void setTun(enabled)}
              />
            </div>
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted">代理模式</span>
              <div className="flex items-center gap-0.5 rounded-md border border-line bg-panel-2 p-0.5 text-[11.5px]">
                {(
                  [
                    { key: 'split', label: '规则' },
                    { key: 'global', label: '全局' },
                    { key: 'direct', label: '直连' },
                  ] as const
                ).map((mode) => (
                  <button
                    key={mode.key}
                    type="button"
                    disabled={!settings || busy}
                    onClick={() => void setRuleMode(mode.key)}
                    className={`rounded px-2 py-0.5 transition-colors ${
                      settings?.rule_mode === mode.key
                        ? 'bg-accent-soft text-accent'
                        : 'text-muted hover:text-fg'
                    }`}
                  >
                    {mode.label}
                  </button>
                ))}
              </div>
            </div>
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
              <Sparkline history={trafficHistory} />
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
        <div className="grid grid-cols-[92px_110px_1fr_1fr_88px_96px] gap-3 border-b border-line pb-2 text-[11px] font-semibold tracking-wide text-faint uppercase">
          <div>核心</div>
          <div>状态</div>
          <div>版本</div>
          <div>监听</div>
          <div className="text-right">内存</div>
          <div className="text-right">操作</div>
        </div>
        {status?.cores.map((core) => {
          const running = core.state === 'running'
          return (
            <div
              key={core.kind}
              className="grid grid-cols-[92px_110px_1fr_1fr_88px_96px] items-center gap-3 border-b border-line py-2.5 text-[13px] last:border-b-0"
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
                    {busyAction === `install:${core.kind}` ? '下载中…' : '下载核心'}
                  </button>
                )}
              </div>
              <div className="text-muted tabular">
                {running ? `127.0.0.1:${core.listen_port ?? '—'}` : '—'}
              </div>
              <div className="text-right text-muted tabular">
                {formatMemory(core.memory_bytes)}
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
                      {busyAction === `switch:${core.kind}` ? '切换中…' : '切换'}
                    </button>
                    {core.binary ? (
                      <button
                        disabled={busy}
                        onClick={() => void updateCore(core.kind)}
                        className="text-faint hover:text-accent disabled:opacity-40"
                      >
                        {busyAction === `update:${core.kind}` ? '更新中…' : '更新'}
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
function Sparkline({ history }: { history: { up: number; down: number }[] }) {
  if (history.length < 2) {
    return <div className="h-12 rounded-md bg-panel-2" />
  }

  let max = 1
  for (const sample of history) {
    if (sample.up > max) max = sample.up
    if (sample.down > max) max = sample.down
  }

  const last = history.length - 1
  const upPoints: string[] = []
  const downPoints: string[] = []
  for (let i = 0; i < history.length; i++) {
    const x = ((i / last) * 100).toFixed(2)
    const sample = history[i]
    upPoints.push(`${x},${(100 - (sample.up / max) * 100).toFixed(2)}`)
    downPoints.push(`${x},${(100 - (sample.down / max) * 100).toFixed(2)}`)
  }

  return (
    <svg viewBox="0 0 100 100" preserveAspectRatio="none" className="h-12 w-full">
      <polyline
        points={downPoints.join(' ')}
        fill="none"
        stroke="currentColor"
        strokeWidth={1.5}
        vectorEffect="non-scaling-stroke"
        className="text-accent"
      />
      <polyline
        points={upPoints.join(' ')}
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
