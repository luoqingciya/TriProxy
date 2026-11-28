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

import { RotateCw, Search, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'

import { PageHeader } from '../components/Sidebar'
import { Badge, Button, Card, Empty, FilterChip, SectionTitle } from '../components/ui'
import { api } from '../services/api'
import { useApp } from '../store/app'
import type { EffectiveRulesView, RuleMode } from '../types'

const PAGE_LIMIT = 300

const MODES: { key: RuleMode; label: string }[] = [
  { key: 'split', label: '规则分流' },
  { key: 'global', label: '全局代理' },
  { key: 'direct', label: '全局直连' },
]

function targetTone(target: string): 'accent' | 'neutral' | 'danger' {
  if (target === 'DIRECT') return 'neutral'
  if (target === 'REJECT') return 'danger'
  return 'accent'
}

export function Rules() {
  const settings = useApp((state) => state.settings)
  const setRuleMode = useApp((state) => state.setRuleMode)
  const busy = useApp((state) => state.busy)
  const activeId = useApp((state) => state.library.active)
  const [data, setData] = useState<EffectiveRulesView | null>(null)
  const [loading, setLoading] = useState(false)
  const [query, setQuery] = useState('')
  const [targetFilter, setTargetFilter] = useState<string>('all')
  const [limit, setLimit] = useState(PAGE_LIMIT)

  const loadRules = useCallback(async () => {
    setLoading(true)
    try {
      const view = await api.effectiveRules()
      setData(view)
    } catch (error) {
      useApp.setState({ error: error instanceof Error ? error.message : String(error) })
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void loadRules()
  }, [loadRules, activeId, settings?.rule_mode, settings?.custom_rules, settings?.rule_sets])

  const allRules = useMemo(() => {
    if (!data) return []
    return [...data.custom_rules, ...data.profile_rules]
  }, [data])

  const targets = useMemo(() => {
    const seen = new Set<string>(['PROXY', 'DIRECT', 'REJECT'])
    for (const rule of allRules) {
      seen.add(rule.group ?? rule.target)
    }
    return Array.from(seen)
  }, [allRules])

  const filteredRules = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return allRules.filter((rule) => {
      const effectiveTarget = rule.group ?? rule.target
      if (targetFilter !== 'all' && effectiveTarget !== targetFilter) return false
      if (
        needle !== '' &&
        !rule.value.toLowerCase().includes(needle) &&
        !rule.kind.toLowerCase().includes(needle) &&
        !effectiveTarget.toLowerCase().includes(needle)
      ) {
        return false
      }
      return true
    })
  }, [allRules, query, targetFilter])

  const visibleRules = filteredRules.slice(0, limit)
  const currentMode = settings?.rule_mode ?? data?.mode ?? 'split'

  return (
    <div>
      <PageHeader
        title="规则"
        description={
          data
            ? data.source === 'subscription'
              ? `当前使用订阅自带规则（${data.profile_rules.length} 条）`
              : `订阅未携带规则，使用软件内置分流规则（${data.profile_rules.length} 条）`
            : '查看当前生效的自定义规则、订阅规则与内置分流规则'
        }
        actions={
          <>
            <div className="flex items-center gap-0.5 rounded-lg border border-line bg-panel p-1 text-[12px]">
              {MODES.map((mode) => (
                <button
                  key={mode.key}
                  type="button"
                  disabled={!settings || busy}
                  onClick={() => void setRuleMode(mode.key)}
                  className={`rounded-md px-2.5 py-1 transition-colors ${
                    currentMode === mode.key
                      ? 'bg-accent-soft font-medium text-accent'
                      : 'text-muted hover:text-fg'
                  }`}
                >
                  {mode.label}
                </button>
              ))}
            </div>
            <Button onClick={() => void loadRules()} disabled={loading}>
              <RotateCw size={13} className={loading ? 'animate-spin' : ''} />
              刷新
            </Button>
          </>
        }
      />

      <div className="mb-4 flex flex-wrap items-center gap-2 text-[12px] text-muted">
        <Badge tone={data?.source === 'subscription' ? 'accent' : 'neutral'}>
          {data?.source === 'subscription' ? '订阅规则优先' : '内置分流规则'}
        </Badge>
        {data && (
          <>
            <span>自定义规则 {data.custom_rules.length} 条</span>
            <span>·</span>
            <span>规则集 {data.rule_sets.length} 个</span>
            <span>·</span>
            <span>兜底策略 MATCH → {data.default_target}</span>
            {data.groups.length > 0 && (
              <>
                <span>·</span>
                <span>分流策略组 {data.groups.length} 个</span>
              </>
            )}
          </>
        )}
      </div>

      {currentMode !== 'split' && (
        <Card className="mb-4 border-accent/30 bg-accent-soft/40 py-3">
          <p className="text-[12.5px] text-muted">
            当前处于
            <strong className="mx-1 text-fg">
              {currentMode === 'global' ? '全局代理' : '全局直连'}
            </strong>
            模式，所有流量将跳过分流列表统一转发；切换到「规则分流」后会按下方顺序匹配。
          </p>
        </Card>
      )}

      {data && data.rule_sets.length > 0 && (
        <Card className="mb-4">
          <SectionTitle title={`规则集 (${data.rule_sets.length})`} />
          <div className="overflow-x-auto">
            <table className="w-full text-left text-[12.5px]">
              <thead>
                <tr className="border-b border-line text-[11px] text-faint uppercase">
                  <th className="pb-2 font-medium">名称</th>
                  <th className="pb-2 font-medium">格式</th>
                  <th className="pb-2 font-medium">类型</th>
                  <th className="pb-2 font-medium">目标</th>
                  <th className="pb-2 font-medium">来源</th>
                  <th className="pb-2 font-medium">URL</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-line">
                {data.rule_sets.map((set) => (
                  <tr key={`${set.source}-${set.name}-${set.format}`}>
                    <td className="py-2 pr-3 font-medium text-fg">{set.name}</td>
                    <td className="py-2 pr-3 text-muted">{set.format}</td>
                    <td className="py-2 pr-3 text-muted">{set.behavior}</td>
                    <td className="py-2 pr-3">
                      <Badge tone={targetTone(set.target)}>{set.group ?? set.target}</Badge>
                    </td>
                    <td className="py-2 pr-3 text-muted">
                      {set.source === 'user'
                        ? '自定义'
                        : set.source === 'subscription'
                          ? '订阅'
                          : '内置'}
                    </td>
                    <td className="max-w-[280px] truncate py-2 text-faint" title={set.url}>
                      {set.url}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}

      <div className="mb-3 flex flex-wrap items-center gap-2.5">
        <div className="relative min-w-[220px] flex-1">
          <Search
            size={14}
            className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-faint"
          />
          <input
            type="text"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Escape' && query) setQuery('')
            }}
            placeholder="搜索域名、IP CIDR、规则类型或策略组"
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
        <div className="flex flex-wrap items-center gap-1 rounded-lg border border-line bg-panel p-1">
          <FilterChip
            active={targetFilter === 'all'}
            onClick={() => setTargetFilter('all')}
            label="全部"
          />
          {targets.slice(0, 8).map((target) => (
            <FilterChip
              key={target}
              active={targetFilter === target}
              onClick={() => setTargetFilter(target)}
              label={target}
            />
          ))}
        </div>
      </div>

      <Card className="overflow-hidden p-0">
        {visibleRules.length === 0 ? (
          <Empty>{loading ? '正在加载规则…' : '没有匹配的规则'}</Empty>
        ) : (
          <>
            <div className="divide-y divide-line text-[12.5px]">
              {visibleRules.map((rule, index) => (
                <div
                  key={`${rule.source}-${rule.kind}-${rule.value}-${index}`}
                  className="grid grid-cols-[130px_1fr_140px_72px] items-center gap-3 px-4 py-2.5"
                >
                  <span className="truncate font-mono text-[11.5px] text-muted">{rule.kind}</span>
                  <span className="truncate select-text font-mono text-fg" title={rule.value}>
                    {rule.value}
                  </span>
                  <div className="flex justify-end">
                    <Badge tone={targetTone(rule.target)}>{rule.group ?? rule.target}</Badge>
                  </div>
                  <span className="text-right text-[11px] text-faint">
                    {rule.source === 'custom'
                      ? '自定义'
                      : rule.source === 'subscription'
                        ? '订阅'
                        : '内置'}
                  </span>
                </div>
              ))}
            </div>
            {filteredRules.length > visibleRules.length && (
              <div className="flex items-center justify-between border-t border-line px-4 py-2.5 text-[12px] text-muted">
                <span>
                  已显示 {visibleRules.length} / {filteredRules.length} 条
                </span>
                <Button onClick={() => setLimit((value) => value + PAGE_LIMIT)}>
                  加载更多
                </Button>
              </div>
            )}
          </>
        )}
      </Card>
    </div>
  )
}
