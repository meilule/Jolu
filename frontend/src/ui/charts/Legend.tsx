/**
 * Chart 系列色与 Legend —— 设计稿 R1-C「分区 · 6 · 图表 / 图例项 Legend」。
 *
 * 调色板固定顺序：绿（jn-viz-1）→ 蓝（jn-viz-2）→ 橙（jn-viz-3）→ 紫（jn-viz-4），
 * 超过 4 项归入「其他」（jn-viz-other）。浅深两套均已过亮度与色盲区分度检查。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

export type VizSeries = 1 | 2 | 3 | 4 | "other"

export const VIZ_BG: Record<VizSeries, string> = {
  1: "bg-jn-viz-1",
  2: "bg-jn-viz-2",
  3: "bg-jn-viz-3",
  4: "bg-jn-viz-4",
  other: "bg-jn-viz-other",
}

export const VIZ_VAR: Record<VizSeries, string> = {
  1: "var(--jn-viz-1)",
  2: "var(--jn-viz-2)",
  3: "var(--jn-viz-3)",
  4: "var(--jn-viz-4)",
  other: "var(--jn-viz-other)",
}

/** 按位置下标取系列色（0..3 对应 1..4，>=4 归为 other）。 */
export function vizSeriesAt(index: number): VizSeries {
  if (index === 0) return 1
  if (index === 1) return 2
  if (index === 2) return 3
  if (index === 3) return 4
  return "other"
}

export interface LegendItemData {
  key: string
  label: ReactNode
  series: VizSeries
  value?: ReactNode
  dimmed?: boolean
}

export function Legend({
  items,
  onHover,
  className,
}: {
  items: LegendItemData[]
  onHover?: (key: string | null) => void
  className?: string
}) {
  return (
    <div className={cn("flex flex-wrap items-center gap-x-4 gap-y-1.5", className)}>
      {items.map((item) => (
        <span
          key={item.key}
          onMouseEnter={onHover ? () => onHover(item.key) : undefined}
          onMouseLeave={onHover ? () => onHover(null) : undefined}
          className={cn(
            "inline-flex items-center gap-1.5 text-xs font-medium text-jn-text-2 transition-opacity",
            item.dimmed && "opacity-40",
          )}
        >
          <span aria-hidden className={cn("size-2.5 shrink-0 rounded-[3px]", VIZ_BG[item.series])} />
          <span>{item.label}</span>
          {item.value !== undefined && (
            <span className="tnum font-mono text-[11.5px] text-jn-text-3">{item.value}</span>
          )}
        </span>
      ))}
    </div>
  )
}
