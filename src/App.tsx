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
import { useEffect } from 'react'

import { Sidebar } from './components/Sidebar'
import { Connections } from './pages/Connections'
import { Dashboard } from './pages/Dashboard'
import { Logs } from './pages/Logs'
import { Profiles } from './pages/Profiles'
import { Proxies } from './pages/Proxies'
import { Settings } from './pages/Settings'
import { useApp } from './store/app'

export function App() {
  const page = useApp((state) => state.page)
  const settings = useApp((state) => state.settings)
  const error = useApp((state) => state.error)
  const clearError = useApp((state) => state.clearError)
  const notice = useApp((state) => state.notice)
  const clearNotice = useApp((state) => state.clearNotice)
  const refresh = useApp((state) => state.refresh)
  const loadSettings = useApp((state) => state.loadSettings)

  // Theme: an explicit class on <html> so both Tailwind and the CSS variables
  // switch together, with "system" tracked live rather than resolved once.
  useEffect(() => {
    const theme = settings?.theme ?? 'dark'
    const media = window.matchMedia('(prefers-color-scheme: dark)')
    const apply = () => {
      const dark = theme === 'dark' || (theme === 'system' && media.matches)
      document.documentElement.classList.toggle('dark', dark)
    }
    apply()
    if (theme === 'system') {
      media.addEventListener('change', apply)
      return () => media.removeEventListener('change', apply)
    }
    return undefined
  }, [settings?.theme])

  useEffect(() => {
    void loadSettings()
    void refresh()
    // Status is polled rather than pushed: the manager is local and the payload
    // is small, so a 2s poll is simpler than an event channel and cannot desync.
    const timer = window.setInterval(() => void refresh(), 2000)
    return () => window.clearInterval(timer)
  }, [loadSettings, refresh])

  return (
    <div className="flex h-full overflow-hidden bg-bg text-fg">
      <Sidebar />
      {/*
        The log page needs a fixed-height inner scroller, so it gets a flex
        container instead of the scrolling one the other pages use.
      */}
      <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
        {page === 'logs' ? (
          <div className="min-h-0 flex-1 px-7 py-6">
            <Logs />
          </div>
        ) : (
          <div className="flex-1 overflow-y-auto px-7 py-6">
            {page === 'dashboard' && <Dashboard />}
            {page === 'proxies' && <Proxies />}
            {page === 'connections' && <Connections />}
            {page === 'profiles' && <Profiles />}
            {page === 'settings' && <Settings />}
          </div>
        )}
      </main>

      {notice && (
        <div className="fixed right-5 bottom-5 z-50 flex max-w-[420px] items-start gap-3 rounded-xl border border-line bg-panel px-4 py-3 shadow-lg">
          <p className="text-[12.5px] leading-relaxed text-muted">{notice}</p>
          <button
            onClick={clearNotice}
            className="mt-0.5 shrink-0 text-faint hover:text-fg"
            aria-label="关闭"
          >
            <X size={14} />
          </button>
        </div>
      )}

      {error && (
        <div className="fixed right-5 bottom-5 z-50 flex max-w-[420px] items-start gap-3 rounded-xl border border-danger/40 bg-panel px-4 py-3 shadow-lg">
          <p className="text-[12.5px] leading-relaxed text-fg">{error}</p>
          <button
            onClick={clearError}
            className="mt-0.5 shrink-0 text-faint hover:text-fg"
            aria-label="关闭"
          >
            <X size={14} />
          </button>
        </div>
      )}
    </div>
  )
}
