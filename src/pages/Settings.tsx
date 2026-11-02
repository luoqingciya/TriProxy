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

import { useEffect, useState } from 'react'

import { PageHeader } from '../components/Sidebar'
import { Button, Card, SectionTitle, SettingRow, Toggle } from '../components/ui'
import { api } from '../services/api'
import {
  checkForUpdate,
  installUpdate,
  type AppUpdateInfo,
} from '../services/updater'
import { useApp } from '../store/app'
import { CORE_LABEL, type CoreKind, type Settings as AppSettings } from '../types'

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
  const autostart = useApp((state) => state.autostart)
  const setAutostart = useApp((state) => state.setAutostart)
  const update = useApp((state) => state.updateSettings)
  const loadSettings = useApp((state) => state.loadSettings)
  const refresh = useApp((state) => state.refresh)
  // Rules are edited as text and saved on blur: validating on every keystroke
  // would flag a half-typed line as an error the user is still writing.
  const [rulesText, setRulesText] = useState(settings?.custom_rules ?? '')
  const [rulesError, setRulesError] = useState<string | null>(null)
  const [ruleSetsText, setRuleSetsText] = useState(settings?.rule_sets ?? '')
  const [ruleSetsError, setRuleSetsError] = useState<string | null>(null)

  useEffect(() => {
    setRulesText(settings?.custom_rules ?? '')
    setRuleSetsText(settings?.rule_sets ?? '')
  }, [settings?.custom_rules, settings?.rule_sets])

  async function commitRules() {
    setRulesError(null)
    try {
      await api.validateRules(rulesText)
    } catch (error) {
      setRulesError(String(error))
      return
    }
    await update({ custom_rules: rulesText.trim() === '' ? null : rulesText })
  }

  async function commitRuleSets() {
    setRuleSetsError(null)
    try {
      await api.validateRuleSets(ruleSetsText)
    } catch (error) {
      setRuleSetsError(String(error))
      return
    }
    await update({ rule_sets: ruleSetsText.trim() === '' ? null : ruleSetsText })
  }

  async function exportBackup() {
    try {
      const path = await api.exportBackup()
      if (path) useApp.setState({ notice: `已导出到 ${path}` })
    } catch (error) {
      useApp.setState({ error: String(error) })
    }
  }

  async function importBackup() {
    try {
      const count = await api.importBackup()
      if (count === null) return
      // Reload both: the backup carries settings and profiles, and the poll
      // alone would leave the form showing the old values.
      await loadSettings()
      await refresh()
      useApp.setState({ notice: `已导入 ${count} 个配置` })
    } catch (error) {
      useApp.setState({ error: String(error) })
    }
  }

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
        <SettingRow
          title="关闭窗口时驻留托盘"
          hint="关闭窗口不退出，代理继续运行；退出请用托盘菜单"
        >
          <Toggle
            checked={settings.close_to_tray}
            onChange={(close_to_tray) => update({ close_to_tray })}
          />
        </SettingRow>
        <SettingRow
          title="开机自启"
          hint="登录时自动启动 TriProxy；Windows 上会请求一次管理员权限"
        >
          <Toggle
            checked={autostart ?? false}
            onChange={(enabled) => void setAutostart(enabled)}
          />
        </SettingRow>
        <SettingRow
          title="启动后自动连接"
          hint="有活动配置时，打开应用就启动核心，不用再点一次"
        >
          <Toggle
            checked={settings.auto_connect}
            onChange={(auto_connect) => update({ auto_connect })}
          />
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
        <SettingRow
          title="监听端口"
          hint="本地混合代理端口；被别的程序占用时核心会启动失败，改完需要重启核心"
        >
          <input
            type="number"
            min={1}
            max={65535}
            value={settings.listen_port}
            onChange={(event) => {
              const port = Number(event.target.value)
              // An empty field parses as 0; saving that would make the next
              // start fail with a port error the user did not ask for.
              if (Number.isInteger(port) && port >= 1 && port <= 65535) {
                update({ listen_port: port })
              }
            }}
            className="w-[104px] rounded-lg border border-line bg-panel-2 px-2.5 py-1.5 text-[13px] text-fg tabular outline-none"
          />
        </SettingRow>
        <SettingRow
          title="代理绕过列表"
          hint="分号分隔；留空用默认（localhost 与内网网段）"
        >
          <input
            value={settings.proxy_bypass ?? ''}
            onChange={(event) =>
              // Empty means "use the built-in list" rather than "bypass nothing":
              // proxying localhost breaks every local dev server.
              update({
                proxy_bypass:
                  event.target.value.trim() === '' ? null : event.target.value,
              })
            }
            placeholder="localhost;*.internal;192.168.*"
            className="w-[240px] rounded-lg border border-line bg-panel-2 px-2.5 py-1.5 text-[12.5px] text-fg outline-none placeholder:text-faint"
          />
        </SettingRow>
      </Card>

      <Card className="mb-4">
        <SectionTitle
          title="自定义规则"
          hint="按顺序匹配，写在前面的优先；命中后不再看后面的规则和预设"
        />
        <textarea
          value={rulesText}
          onChange={(event) => setRulesText(event.target.value)}
          onBlur={() => void commitRules()}
          spellCheck={false}
          rows={6}
          placeholder={
            '# 一行一条，Clash 语法\nDOMAIN-SUFFIX,corp.internal,DIRECT\nDOMAIN-KEYWORD,telemetry,PROXY\nIP-CIDR,10.0.0.0/8,DIRECT'
          }
          className="w-full resize-y rounded-lg border border-line bg-panel-2 px-3 py-2 font-mono text-[12.5px] leading-relaxed text-fg outline-none placeholder:text-faint"
        />
        {rulesError ? (
          <p className="mt-2 text-[12px] text-danger">{rulesError}</p>
        ) : (
          <p className="mt-2 text-[11.5px] text-faint">
            支持 DOMAIN / DOMAIN-SUFFIX / DOMAIN-KEYWORD / IP-CIDR，目标写 PROXY 或 DIRECT。
            改完点空白处保存，改动在下次启动核心时生效。
          </p>
        )}
      </Card>

      <Card className="mb-4">
        <SectionTitle
          title="规则集订阅"
          hint="远程规则集：格式是「目标,类型,URL」。.mrs/.yaml 只对 Mihomo 生效，.srs 只对 sing-box 生效；Xray 不支持"
        />
        <textarea
          value={ruleSetsText}
          onChange={(event) => setRuleSetsText(event.target.value)}
          onBlur={() => void commitRuleSets()}
          spellCheck={false}
          rows={4}
          placeholder={
            '# 目标,类型,URL\nDIRECT,domain,https://example.com/rules/cn.mrs\nPROXY,ipcidr,https://example.com/rules/remote.srs'
          }
          className="w-full resize-y rounded-lg border border-line bg-panel-2 px-3 py-2 font-mono text-[12.5px] leading-relaxed text-fg outline-none placeholder:text-faint"
        />
        {ruleSetsError ? (
          <p className="mt-2 text-[12px] text-danger">{ruleSetsError}</p>
        ) : (
          <p className="mt-2 text-[11.5px] text-faint">
            类型只能是 domain / ipcidr / classical。两个核心的规则集格式不通用，所以一条只对能读它的那个核心生效。
          </p>
        )}
      </Card>

      <Card>
        <SectionTitle title="高级" />
        <SettingRow title="分流模式" hint="global 全部走代理；split 局域网与国内站点直连">
          <select
            value={settings.rule_mode}
            onChange={(event) =>
              update({ rule_mode: event.target.value as AppSettings['rule_mode'] })
            }
            className="rounded-lg border border-line bg-panel-2 px-2.5 py-1.5 text-[13px] text-fg outline-none"
          >
            <option value="global">global</option>
            <option value="split">split</option>
          </select>
        </SettingRow>
        <SettingRow title="DNS 模式" hint="split：走代理解析；fake-ip：TUN 下最快，但部分应用不兼容">
          <select
            value={settings.dns_mode}
            onChange={(event) =>
              update({ dns_mode: event.target.value as AppSettings['dns_mode'] })
            }
            className="rounded-lg border border-line bg-panel-2 px-2.5 py-1.5 text-[13px] text-fg outline-none"
          >
            <option value="off">关闭</option>
            <option value="split">split</option>
            <option value="fake_ip">fake-ip</option>
          </select>
        </SettingRow>
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
        <SettingRow
          title="配置备份"
          hint="导出设置与全部配置到一个 JSON；导入时合并，不会删掉已有的配置"
        >
          <div className="flex gap-2">
            <Button onClick={() => void exportBackup()}>导出</Button>
            <Button onClick={() => void importBackup()}>导入</Button>
          </div>
        </SettingRow>
      </Card>

      <AppUpdate />
    </div>
  )
}

/**
 * Application self-update.
 *
 * Deliberately manual: checking on every launch costs a network round trip and
 * surprises people mid-task. The button is the whole feature.
 */
function AppUpdate() {
  const [checking, setChecking] = useState(false)
  const [update, setUpdate] = useState<AppUpdateInfo | null>(null)
  const [checked, setChecked] = useState(false)
  const [installing, setInstalling] = useState(false)
  const [percent, setPercent] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)

  async function check() {
    setChecking(true)
    setError(null)
    try {
      setUpdate(await checkForUpdate())
      setChecked(true)
    } catch (problem) {
      setError(String(problem))
    } finally {
      setChecking(false)
    }
  }

  async function install() {
    setInstalling(true)
    setError(null)
    try {
      await installUpdate(setPercent)
      // The installer restarts the app; if we are still here, say so instead of
      // pretending nothing happened.
      setError('安装完成，正在重启…')
    } catch (problem) {
      setError(String(problem))
      setInstalling(false)
    }
  }

  return (
    <Card className="mt-4">
      <SectionTitle title="应用更新" hint="更新程序本身；核心的更新在命令行 `core update`" />
      <SettingRow title="版本" hint="当前安装的 TriProxy 版本">
        <div className="flex items-center gap-3">
          {checked && !update && (
            <span className="text-[12.5px] text-muted">已是最新版本</span>
          )}
          {update && !installing && (
            <span className="text-[12.5px] text-accent">可更新到 {update.version}</span>
          )}
          {installing && (
            <span className="text-[12.5px] text-muted">
              {percent === null ? '正在下载…' : `正在下载 ${percent.toFixed(0)}%`}
            </span>
          )}
          {update && !installing ? (
            <Button variant="primary" onClick={install}>
              下载并安装
            </Button>
          ) : (
            <Button onClick={check} disabled={checking || installing}>
              {checking ? '检查中…' : '检查更新'}
            </Button>
          )}
        </div>
      </SettingRow>
      {error && (
        <SettingRow title="结果">
          <span className="max-w-[380px] text-[12.5px] text-muted">{error}</span>
        </SettingRow>
      )}
    </Card>
  )
}
