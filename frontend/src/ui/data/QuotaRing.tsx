/**
 * QuotaRing —— 设计稿 R1-C「分区 · 4 · 状态与数据 / 额度环 QuotaRing」。
 *
 * 规格取自画板：md 56 / sm 40；`level: normal（<85%）/ high（≥85%）/ unknown`。
 *
 * | level | 已用弧 | 何时 |
 * |---|---|---|
 * | normal | `jn-primary` | 已用 < 85% |
 * | high | `jn-warning` | 已用 ≥ 85% |
 * | unknown | **不画弧，只画轨道 + 中心问号** | 没有可用的额度数据 |
 *
 * ## 为什么未知不画 0%
 *
 * 「未知」和「剩 100%」在界面上看起来一样——都是一圈空轨道。所以未知态
 * 必须在中心显示 `?`，否则用户会以为额度还很宽裕。这条与 DataValue 的
 * 七态是同一个原则：不知道就是不知道，不能长得像零。
 *
 * ## 为什么用 SVG 而不是 CSS conic-gradient
 *
 * 环要精确到百分比，且后续要支持悬停与点击（R2 的额度交互都在环上）。
 * SVG 的 `stroke-dasharray` 能精确控制弧长，且命中区域就是路径本身；
 * conic-gradient 无法做圆头端点，也不好挂交互。
 */
import type { CSSProperties, ReactNode } from "react"
import { cn } from "@/lib/utils"

const SIZE = { lg: 84, md: 56, sm: 40 } as const
/** 环宽随尺寸走：84 用 8px（R1-01 对应 innerRadius 0.8），56 用 5px，40 用 4px。 */
const STROKE = { lg: 8, md: 5, sm: 4 } as const

export type QuotaRingLevel = "normal" | "high" | "unknown"
export type QuotaRingSize = keyof typeof SIZE

export interface QuotaRingProps {
  /** 已用百分比 0–100。`level="unknown"` 时忽略 */
  usedPercent?: number | null
  size?: QuotaRingSize
  /** 中心内容。不传则显示百分比数字；unknown 固定显示 `?` */
  children?: ReactNode
  className?: string
}

/** 由已用百分比推出等级。85% 是设计稿给的分界。 */
export function quotaLevel(usedPercent: number | null | undefined): QuotaRingLevel {
  if (usedPercent == null || !Number.isFinite(usedPercent)) return "unknown"
  return usedPercent >= 85 ? "high" : "normal"
}

export function QuotaRing({ usedPercent, size = "md", children, className }: QuotaRingProps) {
  const level = quotaLevel(usedPercent)
  const px = SIZE[size]
  const stroke = STROKE[size]
  const radius = (px - stroke) / 2
  const circumference = 2 * Math.PI * radius
  // 夹到 0–100：来源给超支（>100）或坏数据（<0）时只画到边界，不让弧长变成负数
  const clamped = Math.max(0, Math.min(100, usedPercent ?? 0))
  const dash = (clamped / 100) * circumference

  const center = px / 2
  return (
    <span className={cn("relative inline-flex shrink-0 items-center justify-center", className)} style={{ width: px, height: px }}>
      <svg width={px} height={px} viewBox={`0 0 ${px} ${px}`} aria-hidden className="-rotate-90">
        {/* 轨道 */}
        <circle cx={center} cy={center} r={radius} fill="none" strokeWidth={stroke} className="stroke-jn-track" />
        {/* 已用弧：从 12 点顺时针，首屏 600ms 描线、后续过渡不重画。unknown 时整段不画 */}
        {level !== "unknown" && (
          <circle
            cx={center}
            cy={center}
            r={radius}
            fill="none"
            strokeWidth={stroke}
            strokeLinecap="round"
            strokeDasharray={`${dash} ${circumference - dash}`}
            style={{ "--jn-stroke-start": `${dash}` } as CSSProperties}
            className={cn("jn-meter-fill jn-stroke-draw", level === "high" ? "stroke-jn-warning" : "stroke-jn-primary")}
          />
        )}
      </svg>
      {/* 中心：数字或问号。文字用绝对定位压在 SVG 上，不参与环的布局。 */}
      <span
        className={cn(
          "absolute inset-0 flex items-center justify-center font-mono font-semibold",
          size === "lg" ? "text-[16px]" : size === "md" ? "text-[13px]" : "text-[11px]",
          level === "unknown" ? "text-jn-text-3" : "text-jn-text",
        )}
      >
        {children ?? (level === "unknown" ? "?" : `${Math.round(clamped)}%`)}
      </span>
    </span>
  )
}
