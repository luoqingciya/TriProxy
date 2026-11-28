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

import { useEffect, useRef, useState } from 'react'

interface VirtualListProps<T> {
  items: T[]
  rowHeight: number
  maxHeightClass: string
  overscan?: number
  threshold?: number
  renderRow: (item: T, index: number) => React.ReactNode
}

/**
 * Lightweight fixed-height windowing for large node and connection tables.
 *
 * Small lists keep normal document flow with no nested scroller; once `items`
 * exceeds `threshold`, only the visible slice plus `overscan` rows is mounted.
 */
export function VirtualList<T>({
  items,
  rowHeight,
  maxHeightClass,
  overscan = 12,
  threshold = 60,
  renderRow,
}: VirtualListProps<T>) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(600)

  const virtualized = items.length > threshold

  useEffect(() => {
    const element = containerRef.current
    if (!virtualized || !element) return undefined

    const updateHeight = () => {
      if (element.clientHeight > 0) {
        setViewportHeight(element.clientHeight)
      }
    }
    updateHeight()

    if (typeof ResizeObserver === 'undefined') return undefined
    const observer = new ResizeObserver(updateHeight)
    observer.observe(element)
    return () => observer.disconnect()
  }, [virtualized])

  if (!virtualized) {
    return <>{items.map((item, index) => renderRow(item, index))}</>
  }

  const totalHeight = items.length * rowHeight
  const clampedScroll = Math.min(scrollTop, Math.max(0, totalHeight - viewportHeight))
  const start = Math.max(0, Math.floor(clampedScroll / rowHeight) - overscan)
  const visibleCount = Math.ceil(viewportHeight / rowHeight) + overscan * 2
  const end = Math.min(items.length, start + visibleCount)

  return (
    <div
      ref={containerRef}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      className={`overflow-y-auto ${maxHeightClass}`}
    >
      <div style={{ height: totalHeight, position: 'relative' }}>
        <div style={{ transform: `translateY(${start * rowHeight}px)` }}>
          {items.slice(start, end).map((item, offset) => renderRow(item, start + offset))}
        </div>
      </div>
    </div>
  )
}
