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

import {
  Activity,
  FileText,
  Gauge,
  GitBranch,
  Layers,
  Network,
  ScrollText,
  Settings as SettingsIcon,
} from 'lucide-react'

import appIcon from '../../src-tauri/icons/128x128.png'
import { useApp, type Page } from '../store/app'
import { CORE_LABEL } from '../types'
import { StateDot } from './ui'

const NAV: { page: Page; label: string; icon: typeof Gauge }[] = [
  { page: 'dashboard', label: '仪表盘', icon: Gauge },
  { page: 'proxies', label: '节点', icon: Activity },
  { page: 'rules', label: '规则', icon: GitBranch },
  { page: 'connections', label: '连接', icon: Network },
  { page: 'profiles', label: '配置', icon: Layers },
  { page: 'logs', label: '日志', icon: ScrollText },
  { page: 'settings', label: '设置', icon: SettingsIcon },
]

export function Sidebar() {
  const page = useApp((state) => state.page)
  const setPage = useApp((state) => state.setPage)
  const activeCore = useApp((state) => state.status?.active_core ?? null)
  const runningPort = useApp((state) => {
    const running = state.status?.cores.find((core) => core.state === 'running')
    return running ? (running.listen_port ?? 7890) : null
  })
  const busy = useApp((state) => state.busy)
  const hasActiveProfile = useApp((state) => state.library.active !== null)
  const nodeCount = useApp((state) => state.nodes.length)
  const profileCount = useApp((state) => state.library.profiles.length)
  const start = useApp((state) => state.start)
  const stop = useApp((state) => state.stop)
  const running = activeCore !== null && runningPort !== null

  const quickToggle = () => {
    if (busy) return
    if (running) {
      void stop()
    } else if (hasActiveProfile) {
      void start(null)
    } else {
      setPage('profiles')
      useApp.setState({ notice: '请先导入并启用一份配置' })
    }
  }

  return (
    <aside className="flex w-[212px] shrink-0 flex-col border-r border-line bg-panel/60">
      <div className="flex items-center gap-2.5 px-4 py-5">
        {/*
          The real application icon, imported from the shell's icon set rather
          than copied into `src/`: two copies of a logo drift apart the first
          time one of them is regenerated, and a sidebar showing last year's
          logo next to a taskbar showing this year's is exactly the kind of
          detail nobody reports and everybody notices.
        */}
        <img
          src={appIcon}
          alt=""
          aria-hidden
          className="h-7 w-7 shrink-0 rounded-[7px] object-cover ring-1 ring-white/10"
        />
        <div className="leading-tight">
          <div className="text-[14px] font-semibold text-fg">TriProxy</div>
          <div className="text-[11px] text-faint">三核代理客户端</div>
        </div>
      </div>

      <nav className="flex flex-col gap-0.5 px-2">
        {NAV.map(({ page: target, label, icon: Icon }, index) => {
          const active = page === target
          const count = target === 'proxies' ? nodeCount : target === 'profiles' ? profileCount : 0
          return (
            <button
              key={target}
              onClick={() => setPage(target)}
              title={`${label} (Ctrl/⌘+${index + 1})`}
              className={`flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-[13.5px] transition-colors ${
                active
                  ? 'bg-panel-hover font-medium text-fg'
                  : 'text-muted hover:bg-panel-hover hover:text-fg'
              }`}
            >
              <Icon size={16} strokeWidth={1.9} className={active ? 'text-accent' : ''} />
              <span className="flex-1 text-left">{label}</span>
              {count > 0 && (
                <span className="text-[11px] text-faint tabular">{count}</span>
              )}
            </button>
          )
        })}
      </nav>

      <div className="mt-auto border-t border-line p-3">
        <button
          type="button"
          disabled={busy}
          onClick={quickToggle}
          title={
            running
              ? '点击断开代理'
              : hasActiveProfile
                ? '点击快速连接'
                : '前往配置页导入订阅'
          }
          className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-panel-hover disabled:opacity-50"
        >
          <StateDot state={running ? 'running' : 'stopped'} />
          <div className="min-w-0 leading-tight">
            <div className="truncate text-[12.5px] text-fg">
              {running ? CORE_LABEL[activeCore] : '未连接'}
            </div>
            <div className="truncate text-[11px] text-faint">
              {busy
                ? '处理中…'
                : running
                  ? `127.0.0.1:${runningPort} · 点击断开`
                  : hasActiveProfile
                    ? '点击快速连接'
                    : '导入配置后连接'}
            </div>
          </div>
        </button>
      </div>
    </aside>
  )
}

export function PageHeader({
  title,
  description,
  actions,
}: {
  title: string
  description?: string
  actions?: React.ReactNode
}) {
  return (
    <header className="mb-6 flex items-end justify-between gap-4">
      <div>
        <h1 className="text-[20px] font-semibold tracking-tight text-fg">{title}</h1>
        {description && (
          <p className="mt-1 text-[13px] text-muted">{description}</p>
        )}
      </div>
      {actions && <div className="flex items-center gap-2">{actions}</div>}
    </header>
  )
}

/** Kept for pages that need a document-ish icon next to a title. */
export const FileIcon = FileText
