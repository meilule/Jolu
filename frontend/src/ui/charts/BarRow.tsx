/**
 * BarRow —— 设计稿 R1-C「分区 · 6 · 图表 / 横条行 BarRow」。
 *
 * 用于模型分布、供应商费用排行：顶部名称 + 数值 + 占比，底部 6px 圆角进度条，
 * 系列色走 `jn-viz-1..4` / `jn-viz-other`。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"
import { VIZ_BG, type VizSeries } from "./Legend"

export interface BarRowProps {
  label: ReactNode
  value: ReactNode
  percentText?: ReactNode
  /** 0–1 比例，null 表示未知（不画条） */
  ratio: number | null
  series: VizSeries
  title?: string
  note?: ReactNode
  onClick?: () => void
  className?: string
}

export function BarRow({
  label,
  value,
  percentText,
  ratio,
  series,
  title,
  note,
  onClick,
  className,
}: BarRowProps) {
  const widthPct = ratio === null ? 0 : Math.min(100, Math.max(ratio > 0 ? 2 : 0, ratio * 100))
  const content = (
    <>
      <span className="flex w-full items-center gap-2 text-[12.5px]">
        <span className="min-w-0 flex-1 truncate font-medium text-jn-text">{label}</span>
        <span className="tnum shrink-0 font-mono text-xs font-semibold text-jn-text">{value}</span>
        {percentText !== undefined && (
          <span className="tnum w-11 shrink-0 text-right font-mono text-[11.5px] text-jn-text-3">
            {percentText}
          </span>
        )}
      </span>
      <span className="block h-1.5 w-full overflow-hidden rounded-full bg-jn-track">
        {ratio !== null && (
          <span
            className={cn("jn-meter-fill block h-full rounded-full", VIZ_BG[series])}
            style={{ width: `${widthPct}%` }}
          />
        )}
      </span>
      {note && <span className="text-[11px] text-jn-text-3">{note}</span>}
    </>
  )

  if (onClick) {
    return (
      <button
        type="button"
        title={title}
        onClick={onClick}
        className={cn(
          "jn-interactive -mx-1.5 flex w-[calc(100%+12px)] flex-col gap-1.5 rounded-xl px-1.5 py-1 text-left hover:bg-jn-well",
          className,
        )}
      >
        {content}
      </button>
    )
  }

  return (
    <div title={title} className={cn("flex w-full flex-col gap-1.5 py-0.5", className)}>
      {content}
    </div>
  )
}
