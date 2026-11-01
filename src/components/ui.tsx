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

import type { ButtonHTMLAttributes, ReactNode } from 'react'

import type { CoreState } from '../types'

export function Card({
  children,
  className = '',
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <div
      className={`rounded-xl border border-line bg-panel p-5 ${className}`}
    >
      {children}
    </div>
  )
}

export function SectionTitle({
  title,
  hint,
  action,
}: {
  title: string
  hint?: string
  action?: ReactNode
}) {
  return (
    <div className="mb-4 flex items-start justify-between gap-4">
      <div>
        <h2 className="text-[13px] font-semibold tracking-wide text-muted uppercase">
          {title}
        </h2>
        {hint && <p className="mt-1 text-[13px] text-faint">{hint}</p>}
      </div>
      {action}
    </div>
  )
}

type ButtonVariant = 'primary' | 'ghost' | 'danger'

export function Button({
  variant = 'ghost',
  className = '',
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant }) {
  const base =
    'inline-flex items-center justify-center gap-2 rounded-lg px-3.5 py-2 text-[13px] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-45'
  const variants: Record<ButtonVariant, string> = {
    primary: 'bg-accent text-white hover:brightness-110',
    ghost: 'border border-line text-fg hover:bg-panel-hover',
    danger: 'border border-line text-danger hover:bg-panel-hover',
  }
  return (
    <button className={`${base} ${variants[variant]} ${className}`} {...rest}>
      {children}
    </button>
  )
}

export function Toggle({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean
  onChange: (next: boolean) => void
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative h-6 w-11 shrink-0 rounded-full transition-colors disabled:opacity-45 ${
        checked ? 'bg-accent' : 'bg-line-strong'
      }`}
    >
      <span
        className={`absolute top-0.5 h-5 w-5 rounded-full bg-white transition-all ${
          checked ? 'left-[22px]' : 'left-0.5'
        }`}
      />
    </button>
  )
}

export function SettingRow({
  title,
  hint,
  children,
}: {
  title: string
  hint?: string
  children: ReactNode
}) {
  return (
    <div className="flex items-center justify-between gap-6 border-b border-line py-3.5 last:border-b-0 last:pb-0 first:pt-0">
      <div className="min-w-0">
        <div className="text-[14px] text-fg">{title}</div>
        {hint && <div className="mt-0.5 text-[12.5px] text-faint">{hint}</div>}
      </div>
      {children}
    </div>
  )
}

const STATE_COLOR: Record<CoreState, string> = {
  running: 'bg-success',
  starting: 'bg-warn',
  stopping: 'bg-warn',
  stopped: 'bg-faint',
  failed: 'bg-danger',
}

export function StateDot({ state }: { state: CoreState }) {
  const pulse = state === 'running' || state === 'starting'
  return (
    <span className="relative flex h-2 w-2">
      {pulse && (
        <span
          className={`absolute inline-flex h-full w-full animate-ping rounded-full opacity-60 ${STATE_COLOR[state]}`}
        />
      )}
      <span
        className={`relative inline-flex h-2 w-2 rounded-full ${STATE_COLOR[state]}`}
      />
    </span>
  )
}

export function Badge({
  children,
  tone = 'neutral',
}: {
  children: ReactNode
  tone?: 'neutral' | 'accent' | 'success' | 'warn' | 'danger'
}) {
  const tones = {
    neutral: 'border-line text-muted',
    accent: 'border-accent/40 text-accent',
    success: 'border-success/40 text-success',
    warn: 'border-warn/40 text-warn',
    danger: 'border-danger/40 text-danger',
  }
  return (
    <span
      className={`inline-flex items-center rounded-md border px-1.5 py-0.5 text-[11.5px] font-medium ${tones[tone]}`}
    >
      {children}
    </span>
  )
}

export function Empty({ children }: { children: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-14 text-center">
      <p className="text-[13.5px] text-faint">{children}</p>
    </div>
  )
}
