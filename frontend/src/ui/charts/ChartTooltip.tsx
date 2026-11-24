/**
 * ChartTooltip —— 设计稿 R1-C「分区 · 6 · 图表 / 图表提示 ChartTooltip」。
 *
 * 规格取自画板：宽 200 左右、`jn-surface`、圆角 14、1px `jn-line`、柔阴影、
 * 内边距 10×12，标题 12 semibold + 多行色点/名称/等宽数值 + 可选合计底行。
 */
import type { CSSProperties, ReactNode } from "react"
import { cn } from "@/lib/utils"
import { VIZ_BG, type VizSeries } from "./Legend"

export interface ChartTooltipRow {
  label: ReactNode
  value: ReactNode
  series?: VizSeries
}

export function ChartTooltip({
  title,
  rows,
  footer,
  style,
  className,
}: {
  title: ReactNode
  rows: ChartTooltipRow[]
  footer?: ReactNode
  style?: CSSProperties
  className?: string
}) {
  return (
    <div
      style={style}
      className={cn(
        "pointer-events-none flex min-w-[184px] flex-col gap-1.5 rounded-[14px] border border-jn-line bg-jn-surface px-3 py-2.5 shadow-[0_8px_20px_var(--jn-shadow)]",
        className,
      )}
    >
      <span className="text-xs font-semibold text-jn-text">{title}</span>
      <div className="flex flex-col gap-1">
        {rows.map((row, idx) => (
          <div key={idx} className="flex items-center gap-1.5 text-[11.5px]">
            {row.series !== undefined && (
              <span aria-hidden className={cn("size-2 shrink-0 rounded-[2px]", VIZ_BG[row.series])} />
            )}
            <span className="min-w-0 flex-1 truncate text-jn-text-2">{row.label}</span>
            <span className="tnum shrink-0 font-mono font-semibold text-jn-text">{row.value}</span>
          </div>
        ))}
      </div>
      {footer && (
        <div className="border-t border-jn-line pt-1 text-[11px] text-jn-text-3">{footer}</div>
      )}
    </div>
  )
}
