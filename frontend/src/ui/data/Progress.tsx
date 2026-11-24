/**
 * Progress —— 设计稿 R1-C「分区 · 4 · 状态与数据 / 进度条 Progress」。
 *
 * 上下文占用、额度、存储共用同一个进度条。规格取自画板：轨道高 6、圆角 3。
 *
 * | 状态 | 外观 |
 * |---|---|
 * | normal（<80%） | 主色实心段 |
 * | high（≥80%） | 红色实心段 |
 * | unknown | **只有 1.5px 描边的空轨道**（不画任何已用段） |
 *
 * ## 为什么未知画描边轨道而不是空轨道
 *
 * 空轨道（只有底色）和「已用 0%」长得一样。加一圈描边，视觉上明确是
 * 「这条进度条现在没有数据」，而不是「已经用完了/还没开始」。
 *
 * ## 为什么阈值是 80% 而不是 100%
 *
 * 上下文与额度都是「接近满就开始出问题」：上下文到 100% 会被压缩，
 * 额度到 100% 会被拒绝。80% 是留出反应余量的位置，与画板一致。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

export type ProgressLevel = "normal" | "high" | "unknown"

export interface ProgressProps {
  /** 已用百分比 0–100。传 null / undefined 表示未知 */
  percent?: number | null
  /** 轨道上方的数值行（如「12.4 / 200K」）。不传则不画 */
  label?: ReactNode
  /** 自定义阈值，默认 80 */
  highAt?: number
  className?: string
}

export function Progress({ percent, label, highAt = 80, className }: ProgressProps) {
  const known = percent != null && Number.isFinite(percent)
  const clamped = known ? Math.max(0, Math.min(100, percent)) : 0
  const level: ProgressLevel = !known ? "unknown" : clamped >= highAt ? "high" : "normal"

  return (
    <div className={cn("flex w-full flex-col gap-1.5", className)}>
      {label && <div className="flex w-full items-center justify-between text-[11px] text-jn-text-3">{label}</div>}
      <div
        role="progressbar"
        aria-valuenow={known ? Math.round(clamped) : undefined}
        aria-valuemin={0}
        aria-valuemax={100}
        className={cn(
          "relative h-1.5 w-full overflow-hidden rounded-full",
          // 未知：不填底，只留一圈描边；已知：填底色
          // 未知用**虚线**描边（画板组说明写的是「只画虚线轨道」）：实线空轨道仍会被读成「已用 0%」
          level === "unknown" ? "border-[1.5px] border-dashed border-jn-track bg-transparent" : "bg-jn-track",
        )}
      >
        {level !== "unknown" && (
          <span
            className={cn("jn-meter-fill absolute inset-y-0 left-0 rounded-full", level === "high" ? "bg-jn-danger" : "bg-jn-primary")}
            style={{ width: `${clamped}%` }}
          />
        )}
      </div>
    </div>
  )
}
