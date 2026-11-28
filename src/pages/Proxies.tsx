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

import { Activity, Check, LayoutGrid, List, Search, Star, X, Zap } from 'lucide-react'
import { useDeferredValue, useEffect, useMemo, useState } from 'react'

import { PageHeader } from '../components/Sidebar'
import { VirtualList } from '../components/VirtualList'
import { Badge, Button, Card, Empty, FilterChip } from '../components/ui'
import { useApp } from '../store/app'
import type { LatencySample, Protocol } from '../types'

const EMPTY_FAVORITES: string[] = []
const NAME_COLLATOR = new Intl.Collator('zh-CN', { numeric: true, sensitivity: 'base' })
const VIEW_STORAGE_KEY = 'triproxy.proxies.view'
type SortMode = 'name' | 'latency' | 'favorite'
type ViewMode = 'grid' | 'list'

function initialViewMode(): ViewMode {
  try {
    return window.localStorage.getItem(VIEW_STORAGE_KEY) === 'list' ? 'list' : 'grid'
  } catch {
    return 'grid'
  }
}

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
  const testingProgress = useApp((state) => state.testingProgress)
  const testingNodeIds = useApp((state) => state.testingNodeIds)
  const setPage = useApp((state) => state.setPage)
  const testLatency = useApp((state) => state.testLatency)
  const testProxyDelay = useApp((state) => state.testProxyDelay)
  const testNodeDelay = useApp((state) => state.testNodeDelay)
  const selectNode = useApp((state) => state.selectNode)
  const toggleFavorite = useApp((state) => state.toggleFavorite)
  const activeCore = useApp((state) => state.status?.active_core ?? null)
  const proxyGroups = useApp((state) => state.proxyGroups)
  const refreshProxyGroups = useApp((state) => state.refreshProxyGroups)
  const selectGroupProxy = useApp((state) => state.selectGroupProxy)
  const [query, setQuery] = useState('')
  const [protocol, setProtocol] = useState<Protocol | 'all'>('all')
  const [onlyFavorites, setOnlyFavorites] = useState(false)
  const [hideTimeout, setHideTimeout] = useState(false)
  const [selectedGroup, setSelectedGroup] = useState<string>('PROXY')
  const [sortBy, setSortBy] = useState<SortMode>('name')
  const [viewMode, setViewMode] = useState<ViewMode>(initialViewMode)

  useEffect(() => {
    if (activeCore && activeCore !== 'xray') {
      void refreshProxyGroups()
    }
  }, [activeCore, refreshProxyGroups])

  const changeViewMode = (next: ViewMode) => {
    setViewMode(next)
    try {
      window.localStorage.setItem(VIEW_STORAGE_KEY, next)
    } catch {
      // Ignore storage failures.
    }
  }

  const activeProfile = library.profiles.find((entry) => entry.id === library.active)
  const selectedNodeId = activeProfile?.selected_node ?? null
  const favorites = activeProfile?.favorites ?? EMPTY_FAVORITES
  const favoriteSet = useMemo(() => new Set(favorites), [favorites])
  const deferredQuery = useDeferredValue(query)
  // No stored pick means the compiled config defaults to the automatic group.
  const autoSelected = selectedNodeId === null && nodes.length > 0

  const protocols = useMemo(() => {
    const seen = new Set<Protocol>()
    nodes.forEach((node) => seen.add(node.protocol))
    return Array.from(seen)
  }, [nodes])

  const visible = useMemo(() => {
    const needle = deferredQuery.trim().toLowerCase()
    const filtered = nodes.filter((node) => {
      if (protocol !== 'all' && node.protocol !== protocol) return false
      if (onlyFavorites && !favoriteSet.has(node.id)) return false
      if (hideTimeout) {
        const sample = latency[node.id]
        if (sample && sample.latency_ms === null) return false
      }
      if (
        needle !== '' &&
        !node.name.toLowerCase().includes(needle) &&
        !node.server.toLowerCase().includes(needle)
      ) {
        return false
      }
      return true
    })

    const latencyRank = (nodeId: string) => {
      const sample = latency[nodeId]
      if (!sample) return Number.MAX_SAFE_INTEGER - 1
      if (sample.latency_ms === null) return Number.MAX_SAFE_INTEGER
      return sample.latency_ms
    }

    filtered.sort((a, b) => {
      if (sortBy === 'latency') {
        const diff = latencyRank(a.id) - latencyRank(b.id)
        if (diff !== 0) return diff
      } else if (sortBy === 'favorite') {
        const favDiff = Number(favoriteSet.has(b.id)) - Number(favoriteSet.has(a.id))
        if (favDiff !== 0) return favDiff
      }
      return NAME_COLLATOR.compare(a.name, b.name)
    })
    return filtered
  }, [nodes, protocol, deferredQuery, onlyFavorites, hideTimeout, favoriteSet, sortBy, latency])

  const filteredActive =
    query.trim() !== '' || protocol !== 'all' || onlyFavorites || hideTimeout
  const visibleGroups = useMemo(
    () => proxyGroups.filter((group) => group.name !== 'GLOBAL'),
    [proxyGroups],
  )
  const activeGroupObj = useMemo(() => {
    if (visibleGroups.length === 0) return null
    return (
      visibleGroups.find((group) => group.name === selectedGroup) ??
      visibleGroups.find((group) => group.name === 'PROXY') ??
      visibleGroups[0]
    )
  }, [visibleGroups, selectedGroup])
  const nodesByName = useMemo(() => new Map(nodes.map((node) => [node.name, node])), [nodes])
  const showCustomGroup = Boolean(activeGroupObj && activeGroupObj.name !== 'PROXY')

  return (
    <div>
      <PageHeader
        title="节点"
        description={
          activeProfile
            ? `${
                filteredActive
                  ? `来自配置「${activeProfile.name}」，显示 ${visible.length} / ${nodes.length} 个节点`
                  : `来自配置「${activeProfile.name}」，共 ${nodes.length} 个节点`
              } · ${
                (activeProfile.rule_count ?? 0) > 0
                  ? `订阅规则 ${activeProfile.rule_count} 条`
                  : '内置分流规则'
              }`
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
              {testing
                ? testingProgress && testingProgress.total > 0
                  ? `测试中 ${testingProgress.done}/${testingProgress.total}…`
                  : '测试中…'
                : '真实延迟'}
            </Button>
          </>
        }
      />

      <div className="mb-4 flex flex-wrap items-center gap-2">
        <div className="relative min-w-[200px] flex-1">
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
            placeholder="搜索节点名称或地址"
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
        <div className="flex items-center gap-1 rounded-lg border border-line bg-panel p-1">
          <FilterChip
            active={sortBy === 'name'}
            onClick={() => setSortBy('name')}
            label="按名称"
          />
          <FilterChip
            active={sortBy === 'latency'}
            onClick={() => setSortBy('latency')}
            label="按延迟"
          />
          <FilterChip
            active={sortBy === 'favorite'}
            onClick={() => setSortBy('favorite')}
            label="收藏优先"
          />
        </div>
        {/* Starring is only useful if you can then look at just those. */}
        <FilterChip
          active={onlyFavorites}
          onClick={() => setOnlyFavorites((value) => !value)}
          label={`收藏 ${favorites.length}`}
        />
        <FilterChip
          active={hideTimeout}
          onClick={() => setHideTimeout((value) => !value)}
          label="隐藏超时"
        />
        <div className="flex items-center gap-1 rounded-lg border border-line bg-panel p-1">
          <button
            type="button"
            onClick={() => changeViewMode('grid')}
            title="卡片网格视图"
            className={`rounded-md p-1 transition-colors ${
              viewMode === 'grid' ? 'bg-accent-soft text-accent' : 'text-muted hover:text-fg'
            }`}
          >
            <LayoutGrid size={14} />
          </button>
          <button
            type="button"
            onClick={() => changeViewMode('list')}
            title="紧凑列表视图"
            className={`rounded-md p-1 transition-colors ${
              viewMode === 'list' ? 'bg-accent-soft text-accent' : 'text-muted hover:text-fg'
            }`}
          >
            <List size={14} />
          </button>
        </div>
      </div>

      {visibleGroups.length > 0 && (
        <div className="mb-4 flex flex-wrap items-center gap-1.5 rounded-xl border border-line bg-panel p-1.5">
          {visibleGroups.map((group) => {
            const active = activeGroupObj?.name === group.name
            const label =
              group.name === 'PROXY' ? '手动选择' : group.name === 'AUTO' ? '自动选择' : group.name
            return (
              <button
                key={group.name}
                type="button"
                onClick={() => setSelectedGroup(group.name)}
                className={`flex items-center gap-2 rounded-lg px-3 py-1.5 text-[12.5px] transition-colors ${
                  active
                    ? 'bg-accent-soft font-medium text-accent'
                    : 'text-muted hover:bg-panel-hover hover:text-fg'
                }`}
              >
                <span>{label}</span>
                {group.now && (
                  <span className="max-w-[140px] truncate text-[11px] opacity-75">
                    · {group.now}
                  </span>
                )}
              </button>
            )
          })}
        </div>
      )}

      {showCustomGroup && activeGroupObj ? (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {activeGroupObj.all
            .filter((member) => {
              const needle = deferredQuery.trim().toLowerCase()
              if (needle !== '' && !member.toLowerCase().includes(needle)) return false
              const matchedNode = nodesByName.get(member)
              if (hideTimeout && matchedNode) {
                const sample = latency[matchedNode.id]
                if (sample && sample.latency_ms === null) return false
              }
              return true
            })
            .map((member) => {
              const matchedNode = nodesByName.get(member)
              const sample = matchedNode ? latency[matchedNode.id] : undefined
              const isSelected = activeGroupObj.now === member
              const selectable = activeGroupObj.group_type.toLowerCase() !== 'urltest'
              return (
                <button
                  key={`${activeGroupObj.name}-${member}`}
                  type="button"
                  disabled={!selectable}
                  onClick={() => {
                    if (selectable && !isSelected) {
                      void selectGroupProxy(activeGroupObj.name, member)
                    }
                  }}
                  className={`flex flex-col justify-between rounded-xl border p-3.5 text-left transition-colors ${
                    isSelected
                      ? 'border-accent/50 bg-accent-soft'
                      : 'border-line bg-panel hover:bg-panel-hover'
                  } ${!selectable ? 'cursor-default' : ''}`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="truncate text-[13.5px] font-medium text-fg">{member}</span>
                    {isSelected && (
                      <Check size={14} strokeWidth={2.6} className="shrink-0 text-accent" />
                    )}
                  </div>
                  <div className="mt-2.5 flex items-center justify-between text-[11.5px] text-muted">
                    {matchedNode ? (
                      <>
                        <Badge tone="neutral">{PROTOCOL_LABEL[matchedNode.protocol]}</Badge>
                        <LatencyCell sample={sample} />
                      </>
                    ) : (
                      <>
                        <Badge tone="neutral">{activeGroupObj.group_type}</Badge>
                        <span className="text-faint">策略成员</span>
                      </>
                    )}
                  </div>
                </button>
              )
            })}
        </div>
      ) : visible.length === 0 ? (
        <Card className="p-0">
          <Empty>
            {nodes.length > 0 ? (
              '没有匹配的节点'
            ) : library.profiles.length > 0 ? (
              <>
                已有配置，但没有启用任何一份。
                <button
                  type="button"
                  onClick={() => setPage('profiles')}
                  className="ml-1 text-accent hover:underline"
                >
                  前往「配置」页启用
                </button>
              </>
            ) : (
              <>
                还没有配置。
                <button
                  type="button"
                  onClick={() => setPage('profiles')}
                  className="ml-1 text-accent hover:underline"
                >
                  前往「配置」页导入订阅
                </button>
              </>
            )}
          </Empty>
        </Card>
      ) : viewMode === 'grid' ? (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          <button
            type="button"
            onClick={() => {
              if (!autoSelected) void selectNode(null)
            }}
            title="按延迟自动挑选，每 5 分钟重新测速"
            className={`flex flex-col justify-between rounded-xl border p-3.5 text-left transition-colors ${
              autoSelected
                ? 'border-accent/50 bg-accent-soft'
                : 'border-line bg-panel hover:bg-panel-hover'
            }`}
          >
            <div className="flex items-center justify-between gap-2">
              <div className="flex min-w-0 items-center gap-2">
                <Zap size={14} strokeWidth={2.2} className="shrink-0 text-accent" />
                <span className="truncate text-[13.5px] font-medium text-fg">自动选择</span>
              </div>
              {autoSelected && <Check size={14} strokeWidth={2.6} className="shrink-0 text-accent" />}
            </div>
            <div className="mt-2.5 flex items-center justify-between text-[11.5px] text-muted">
              <span>AUTO · url-test</span>
              <span className="text-faint">延迟最低优先</span>
            </div>
          </button>

          {visible.map((node) => {
            const isSelected = node.id === selectedNodeId
            const starred = favoriteSet.has(node.id)
            const probing = Boolean(testingNodeIds[node.id])
            return (
              <div
                key={node.id}
                onClick={() => {
                  if (window.getSelection()?.toString()) return
                  if (!isSelected) void selectNode(node.id)
                }}
                title={isSelected ? '当前已选中此节点' : '点击切换到此节点'}
                className={`flex cursor-pointer flex-col justify-between rounded-xl border p-3.5 transition-colors ${
                  isSelected
                    ? 'border-accent/50 bg-accent-soft'
                    : 'border-line bg-panel hover:bg-panel-hover'
                }`}
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-1.5">
                    <button
                      type="button"
                      onClick={(event) => {
                        event.stopPropagation()
                        void toggleFavorite(node.id)
                      }}
                      title={starred ? '取消收藏' : '收藏'}
                      className={starred ? 'shrink-0 text-accent' : 'shrink-0 text-faint hover:text-accent'}
                    >
                      <Star size={13} strokeWidth={2.2} fill={starred ? 'currentColor' : 'none'} />
                    </button>
                    <span className="truncate text-[13.5px] font-medium text-fg">{node.name}</span>
                  </div>
                  {isSelected && <Check size={14} strokeWidth={2.6} className="shrink-0 text-accent" />}
                </div>
                <div className="mt-2.5 flex items-center justify-between gap-2 text-[11.5px]">
                  <span className="truncate text-muted">
                    {PROTOCOL_LABEL[node.protocol]}
                    {node.transport !== 'tcp' && ` · ${node.transport}`}
                    {node.security !== '-' && ` · ${node.security}`}
                  </span>
                  <button
                    type="button"
                    disabled={probing}
                    onClick={(event) => {
                      event.stopPropagation()
                      void testNodeDelay(node.id)
                    }}
                    title="点击单独测速"
                    className="shrink-0 rounded-md border border-line/70 bg-panel-2 px-1.5 py-0.5 text-[11px] tabular hover:border-line-strong"
                  >
                    {probing ? (
                      <span className="text-muted">测速中…</span>
                    ) : latency[node.id] ? (
                      <LatencyCell sample={latency[node.id]} />
                    ) : (
                      <span className="text-faint">测速</span>
                    )}
                  </button>
                </div>
              </div>
            )
          })}
        </div>
      ) : (
        <Card className="p-0">
          <div>
            <div className="grid grid-cols-[1fr_88px_1fr_92px_76px] gap-4 border-b border-line px-5 py-2.5 text-[11px] font-semibold tracking-wide text-faint uppercase">
              <div>名称</div>
              <div>协议</div>
              <div>地址</div>
              <div>传输</div>
              <div className="text-right">延迟</div>
            </div>
            <button
              onClick={() => {
                if (!autoSelected) void selectNode(null)
              }}
              title="按延迟自动挑选，每 5 分钟重新测速"
              className={
                autoSelected
                  ? 'grid w-full cursor-pointer grid-cols-[1fr_88px_1fr_92px_76px] items-center gap-4 border-b border-line bg-accent-soft px-5 py-3 text-left text-[13px]'
                  : 'grid w-full cursor-pointer grid-cols-[1fr_88px_1fr_92px_76px] items-center gap-4 border-b border-line px-5 py-3 text-left text-[13px] hover:bg-panel-hover'
              }
            >
              <div className="flex min-w-0 items-center gap-2">
                <span className={autoSelected ? 'text-accent' : 'text-transparent'}>
                  <Check size={14} strokeWidth={2.6} />
                </span>
                <Zap size={14} strokeWidth={2.2} className="shrink-0 text-accent" />
                <span className="truncate text-fg">自动选择</span>
              </div>
              <div>
                <Badge>自动</Badge>
              </div>
              <div className="truncate text-muted">延迟最低的节点</div>
              <div className="text-muted">—</div>
              <div className="text-right text-muted tabular">—</div>
            </button>
            <VirtualList
              items={visible}
              rowHeight={46}
              maxHeightClass="max-h-[calc(100vh-300px)]"
              renderRow={(node) => {
                const isSelected = node.id === selectedNodeId
                const starred = favoriteSet.has(node.id)
                const probing = Boolean(testingNodeIds[node.id])
                return (
                  <div
                    key={node.id}
                    onClick={() => {
                      if (window.getSelection()?.toString()) return
                      if (!isSelected) void selectNode(node.id)
                    }}
                    title={isSelected ? '当前已选中此节点' : '点击切换到此节点'}
                    className={
                      isSelected
                        ? 'grid h-[46px] cursor-pointer grid-cols-[1fr_88px_1fr_92px_76px] items-center gap-4 border-b border-line bg-accent-soft px-5 text-[13px]'
                        : 'grid h-[46px] cursor-pointer grid-cols-[1fr_88px_1fr_92px_76px] items-center gap-4 border-b border-line px-5 text-[13px] last:border-b-0 hover:bg-panel-hover'
                    }
                  >
                    <div className="flex min-w-0 items-center gap-2">
                      <span className={isSelected ? 'text-accent' : 'text-transparent'}>
                        <Check size={14} strokeWidth={2.6} />
                      </span>
                      <button
                        onClick={(event) => {
                          event.stopPropagation()
                          void toggleFavorite(node.id)
                        }}
                        title={starred ? '取消收藏' : '收藏'}
                        className={starred ? 'shrink-0 text-accent' : 'shrink-0 text-faint hover:text-accent'}
                      >
                        <Star size={14} strokeWidth={2.2} fill={starred ? 'currentColor' : 'none'} />
                      </button>
                      <span className="truncate text-fg">{node.name}</span>
                    </div>
                    <div>
                      <Badge>{PROTOCOL_LABEL[node.protocol]}</Badge>
                    </div>
                    <div className="truncate select-text text-muted tabular">
                      {node.server}:{node.port}
                    </div>
                    <div className="text-muted">
                      {node.transport}
                      {node.security !== '-' && (
                        <span className="ml-1 text-faint">/{node.security}</span>
                      )}
                    </div>
                    <div className="text-right tabular">
                      <button
                        type="button"
                        disabled={probing}
                        onClick={(event) => {
                          event.stopPropagation()
                          void testNodeDelay(node.id)
                        }}
                        title="点击单独测速"
                        className="hover:underline"
                      >
                        {probing ? (
                          <span className="text-muted">…</span>
                        ) : (
                          <LatencyCell sample={latency[node.id]} />
                        )}
                      </button>
                    </div>
                  </div>
                )
              }}
            />
          </div>
        </Card>
      )}

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
