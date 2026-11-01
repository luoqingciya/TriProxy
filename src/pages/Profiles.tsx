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

import { Check, Plus, RefreshCw, Trash2 } from 'lucide-react'
import { useState } from 'react'

import { PageHeader } from '../components/Sidebar'
import { Badge, Button, Card, Empty } from '../components/ui'
import { useApp } from '../store/app'

export function Profiles() {
  const library = useApp((state) => state.library)
  const busy = useApp((state) => state.busy)
  const importProfile = useApp((state) => state.importProfile)
  const useProfile = useApp((state) => state.useProfile)
  const removeProfile = useApp((state) => state.removeProfile)
  const refreshProfile = useApp((state) => state.refreshProfile)

  const [adding, setAdding] = useState(false)
  const [name, setName] = useState('')
  const [text, setText] = useState('')

  const submit = async () => {
    const trimmed = text.trim()
    if (!trimmed) return
    const stored = await importProfile(trimmed, name.trim() || '未命名配置')
    if (stored) {
      setText('')
      setName('')
      setAdding(false)
    }
  }

  return (
    <div>
      <PageHeader
        title="配置"
        description="导入订阅或分享链接，切换时只需选择活动配置"
        actions={
          <Button variant="primary" onClick={() => setAdding((value) => !value)}>
            <Plus size={15} strokeWidth={2.2} />
            导入配置
          </Button>
        }
      />

      {adding && (
        <Card className="mb-4">
          <div className="space-y-3">
            <div>
              <label className="mb-1.5 block text-[12px] text-muted">名称</label>
              <input
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="例如 jp-sub"
                className="w-full rounded-lg border border-line bg-panel-2 px-3 py-2 text-[13px] text-fg outline-none placeholder:text-faint focus:border-line-strong"
              />
            </div>
            <div>
              <label className="mb-1.5 block text-[12px] text-muted">
                订阅内容（分享链接列表，或 base64 订阅）
              </label>
              <textarea
                value={text}
                onChange={(event) => setText(event.target.value)}
                rows={6}
                spellCheck={false}
                placeholder={'vless://...\ntrojan://...\nss://...'}
                className="w-full resize-y rounded-lg border border-line bg-panel-2 px-3 py-2 font-mono text-[12px] leading-relaxed text-fg outline-none placeholder:text-faint focus:border-line-strong"
              />
              <p className="mt-1.5 text-[12px] text-faint">
                支持 VLESS / VMess / Trojan / Shadowsocks。单行解析失败不会影响其余节点。
              </p>
            </div>
            <div className="flex items-center gap-2">
              <Button
                variant="primary"
                disabled={busy || text.trim().length === 0}
                onClick={submit}
              >
                保存
              </Button>
              <Button onClick={() => setAdding(false)}>取消</Button>
            </div>
          </div>
        </Card>
      )}

      {library.profiles.length === 0 ? (
        <Card>
          <Empty>还没有配置。点「导入配置」粘贴订阅内容即可。</Empty>
        </Card>
      ) : (
        <div className="grid grid-cols-2 gap-4">
          {library.profiles.map((profile) => {
            const active = profile.id === library.active
            return (
              <Card key={profile.id} className={active ? 'ring-1 ring-accent/40' : ''}>
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-[14px] font-medium text-fg">
                        {profile.name}
                      </span>
                      {active && <Badge tone="accent">活动</Badge>}
                      {profile.subscription && <Badge>订阅</Badge>}
                    </div>
                    <div className="mt-1 text-[12.5px] text-muted tabular">
                      {profile.node_count} 个节点 ·{' '}
                      {profile.format === 'base64_subscription' ? 'base64 订阅' : '链接列表'}
                    </div>
                    <div className="mt-1 truncate text-[12px] text-faint">
                      {profile.source || '—'}
                    </div>
                  </div>
                </div>

                <div className="mt-4 flex items-center gap-2">
                  {active ? (
                    <span className="inline-flex items-center gap-1.5 text-[12.5px] text-accent">
                      <Check size={14} strokeWidth={2.4} />
                      当前使用
                    </span>
                  ) : (
                    <Button onClick={() => useProfile(profile.id)}>设为活动</Button>
                  )}
                  {profile.subscription && (
                    <Button
                      className="px-2.5"
                      disabled={busy}
                      onClick={() => refreshProfile(profile.id)}
                      title="立即刷新订阅"
                    >
                      <RefreshCw size={14} />
                    </Button>
                  )}
                  <Button
                    variant="danger"
                    className="ml-auto px-2.5"
                    onClick={() => removeProfile(profile.id)}
                  >
                    <Trash2 size={14} />
                  </Button>
                </div>
              </Card>
            )
          })}
        </div>
      )}
    </div>
  )
}
