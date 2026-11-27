/**
 * StatusPill —— 设计稿 R1-C「分区 · 4 · 状态与数据 / 状态胶囊 StatusPill」。
 *
 * 一个 6px 圆点 + 一行文字，胶囊形容器。规格取自画板：
 * 高 24、圆角 12（= 高度一半）、点 6px、文字 12px/600、左内边距 9 / 右 10。
 *
 * ## 七种 tone
 *
 * 浅底 + 同色文字（`-soft` 底 + 主色文字），只有 `neutral` / `muted` 用 well 底。
 *
 * | tone | 用于 | 点 |
 * |---|---|---|
 * | success | 已完成、可用 | 主色实心 |
 * | active | 正在运行 | 主色实心 + 呼吸 |
 * | warning | 将尽、需注意 | 橙 |
 * | danger | 失败、断开 | 红 |
 * | info | 中性提示 | 蓝 |
 * | neutral | 未开始、已暂停 | 次级灰 |
 * | muted | 不适用、已忽略 | 更浅的灰 |
 *
 * `active` 的点带呼吸动画：它是唯一表示「此刻正在进行」的状态，静止的点
 * 与 `success` 看不出区别。减少动效时呼吸自动停（动画在 motion.css 之外用
 * Tailwind 的 animate-pulse，受 prefers-reduced-motion 影响）。
 *
 * ## 为什么不接受自定义颜色
 *
 * 状态色只有这七种。允许传 color 就会有页面传品牌色当状态色，
 * 于是「这个绿是成功还是品牌」再也分不清。
 */
import type { HTMLAttributes } from "react"
import { cn } from "@/lib/utils"

const TONE = {
  success: "bg-jn-primary-soft text-jn-primary-ink",
  active: "bg-jn-primary-soft text-jn-primary-ink",
  warning: "bg-jn-warning-soft text-jn-warning",
  danger: "bg-jn-danger-soft text-jn-danger",
  info: "bg-jn-info-soft text-jn-info",
  neutral: "bg-jn-well text-jn-text-2",
  muted: "bg-jn-well text-jn-text-3",
} as const

/** 圆点颜色。与胶囊文字同色系，但 success/active 用更饱和的主色。 */
const DOT = {
  success: "bg-jn-primary",
  active: "bg-jn-primary",
  warning: "bg-jn-warning",
  danger: "bg-jn-danger",
  info: "bg-jn-info",
  neutral: "bg-jn-text-3",
  muted: "bg-jn-text-3",
} as const

export type StatusTone = keyof typeof TONE

export interface StatusPillProps extends Omit<HTMLAttributes<HTMLSpanElement>, "children"> {
  tone: StatusTone
  children: string
}

export function StatusPill({ tone, children, className, ...rest }: StatusPillProps) {
  return (
    <span
      className={cn(
        "inline-flex h-6 max-w-full shrink-0 items-center gap-[6px] rounded-full pl-[9px] pr-[10px] text-xs font-semibold whitespace-nowrap",
        TONE[tone],
        className,
      )}
      {...rest}
    >
      <span
        aria-hidden
        className={cn("size-1.5 shrink-0 rounded-full", DOT[tone], tone === "active" && "animate-pulse")}
      />
      <span className="min-w-0 truncate">{children}</span>
    </span>
  )
}
