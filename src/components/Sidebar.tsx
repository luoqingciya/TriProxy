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
  Layers,
  ScrollText,
  Settings as SettingsIcon,
} from 'lucide-react'

import { useApp, type Page } from '../store/app'
import { CORE_LABEL, type CoreKind } from '../types'
import { StateDot } from './ui'

const NAV: { page: Page; label: string; icon: typeof Gauge }[] = [
  { page: 'dashboard', label: '仪表盘', icon: Gauge },
  { page: 'proxies', label: '节点', icon: Activity },
  { page: 'profiles', label: '配置', icon: Layers },
  { page: 'logs', label: '日志', icon: ScrollText },
  { page: 'settings', label: '设置', icon: SettingsIcon },
]

export function Sidebar() {
  const page = useApp((state) => state.page)
  const setPage = useApp((state) => state.setPage)
  const status = useApp((state) => state.status)

  const running = status?.cores.find((core) => core.state === 'running')
  const activeCore = status?.active_core as CoreKind | null | undefined

  return (
    <aside className="flex w-[212px] shrink-0 flex-col border-r border-line bg-panel/60">
      <div className="flex items-center gap-2.5 px-4 py-5">
        <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-accent text-[13px] font-bold text-white">
          T
        </div>
        <div className="leading-tight">
          <div className="text-[14px] font-semibold text-fg">TriProxy</div>
          <div className="text-[11px] text-faint">三核代理客户端</div>
        </div>
      </div>

      <nav className="flex flex-col gap-0.5 px-2">
        {NAV.map(({ page: target, label, icon: Icon }) => {
          const active = page === target
          return (
            <button
              key={target}
              onClick={() => setPage(target)}
              className={`flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-[13.5px] transition-colors ${
                active
                  ? 'bg-panel-hover font-medium text-fg'
                  : 'text-muted hover:bg-panel-hover hover:text-fg'
              }`}
            >
              <Icon size={16} strokeWidth={1.9} className={active ? 'text-accent' : ''} />
              {label}
            </button>
          )
        })}
      </nav>

      <div className="mt-auto border-t border-line p-3">
        <div className="flex items-center gap-2 rounded-lg px-1.5 py-1.5">
          <StateDot state={running ? 'running' : 'stopped'} />
          <div className="min-w-0 leading-tight">
            <div className="truncate text-[12.5px] text-fg">
              {running ? CORE_LABEL[running.kind] : '未连接'}
            </div>
            <div className="text-[11px] text-faint">
              {running && activeCore ? `127.0.0.1:${running.listen_port ?? 7890}` : '点击开始连接'}
            </div>
          </div>
        </div>
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
