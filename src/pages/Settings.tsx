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

import { PageHeader } from '../components/Sidebar'
import { Card, SectionTitle, SettingRow, Toggle } from '../components/ui'
import { useApp } from '../store/app'
import { CORE_LABEL, type CoreKind } from '../types'

const CORES: (CoreKind | null)[] = [null, 'mihomo', 'sing-box', 'xray']
const THEMES = [
  { value: 'light', label: '浅色' },
  { value: 'dark', label: '深色' },
  { value: 'system', label: '跟随系统' },
] as const

export function Settings() {
  const settings = useApp((state) => state.settings)
  const status = useApp((state) => state.status)
  const proxy = useApp((state) => state.proxy)
  const setSystemProxy = useApp((state) => state.setSystemProxy)
  const update = useApp((state) => state.updateSettings)

  if (!settings) {
    return (
      <div>
        <PageHeader title="设置" />
        <Card>正在加载…</Card>
      </div>
    )
  }

  return (
    <div className="max-w-[720px]">
      <PageHeader title="设置" description="所有设置立即生效并保存在本地" />

      <Card className="mb-4">
        <SectionTitle title="通用" />
        <SettingRow title="主题" hint="浅色 / 深色 / 跟随系统">
          <div className="flex items-center gap-1 rounded-lg border border-line bg-panel-2 p-1">
            {THEMES.map((theme) => (
              <button
                key={theme.value}
                onClick={() => update({ theme: theme.value })}
                className={`rounded-md px-2.5 py-1 text-[12.5px] transition-colors ${
                  settings.theme === theme.value
                    ? 'bg-accent-soft text-accent'
                    : 'text-muted hover:text-fg'
                }`}
              >
                {theme.label}
              </button>
            ))}
          </div>
        </SettingRow>
        <SettingRow
          title="核心选择"
          hint="Smart 由选核引擎决定；固定核心会在不兼容时直接报错"
        >
          <select
            value={settings.core_preference ?? 'smart'}
            onChange={(event) =>
              update({
                core_preference:
                  event.target.value === 'smart'
                    ? null
                    : (event.target.value as CoreKind),
              })
            }
            className="rounded-lg border border-line bg-panel-2 px-2.5 py-1.5 text-[13px] text-fg outline-none"
          >
            {CORES.map((core) => (
              <option key={core ?? 'smart'} value={core ?? 'smart'}>
                {core === null ? 'Smart（推荐）' : CORE_LABEL[core]}
              </option>
            ))}
          </select>
        </SettingRow>
      </Card>

      <Card className="mb-4">
        <SectionTitle
          title="网络"
          hint="这些是应用侧需求：打开后，不具备该能力的核心会在选核时被淘汰"
        />
        <SettingRow
          title="系统代理"
          hint={
            proxy
              ? `当前指向 ${proxy.endpoint}（${proxy.backend}）`
              : '接管系统的 HTTP/HTTPS 代理，关闭核心时自动还原'
          }
        >
          <Toggle checked={proxy !== null} onChange={setSystemProxy} />
        </SettingRow>
        <SettingRow
          title="TUN 模式"
          hint="接管全局流量，需要以管理员身份运行；启用后 Xray 会被排除"
        >
          <Toggle checked={settings.tun} onChange={(tun) => update({ tun })} />
        </SettingRow>
        <SettingRow title="原生 rule-set" hint="仅 sing-box 原生支持；开启后 mihomo/xray 会被淘汰">
          <Toggle
            checked={settings.rule_set}
            onChange={(rule_set) => update({ rule_set })}
          />
        </SettingRow>
        <SettingRow title="代理组" hint="需要 select / urltest 之类的分组能力">
          <Toggle
            checked={settings.proxy_groups}
            onChange={(proxy_groups) => update({ proxy_groups })}
          />
        </SettingRow>
        <SettingRow title="允许局域网连接" hint="监听 0.0.0.0 而不是 127.0.0.1">
          <Toggle
            checked={settings.allow_lan}
            onChange={(allow_lan) => update({ allow_lan })}
          />
        </SettingRow>
      </Card>

      <Card>
        <SectionTitle title="高级" />
        <SettingRow title="日志级别">
          <select
            value={settings.log_level}
            onChange={(event) => update({ log_level: event.target.value })}
            className="rounded-lg border border-line bg-panel-2 px-2.5 py-1.5 text-[13px] text-fg outline-none"
          >
            {['debug', 'info', 'warn', 'error'].map((level) => (
              <option key={level} value={level}>
                {level}
              </option>
            ))}
          </select>
        </SettingRow>
        <SettingRow title="数据目录" hint="配置、核心二进制与日志都存放在这里">
          <span className="max-w-[380px] truncate text-[12.5px] text-muted tabular">
            {status?.data_dir ?? '—'}
          </span>
        </SettingRow>
      </Card>
    </div>
  )
}
