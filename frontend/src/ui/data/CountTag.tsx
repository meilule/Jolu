/**
 * Count 与 Tag —— 设计稿 R1-C「分区 · 4 · 状态与数据 / 计数 Count / 标签 Tag」。
 *
 * 两者外观相近但用途不同，放在一个文件里因为它们是同一组「小文字装饰」：
 *
 * - **Count** 跟在标题后面表示数量。`tone` 可选，用来在数量异常时染色
 *   （例如错误数 > 0 用 danger）。它是**可变的数**，所以支持 tone。
 * - **Tag** 只做描述（「已暂停」「只读」「实验」），**不可点、不随数据变化**，
 *   所以固定一种底色，不给 tone —— 给了就会有人拿它当状态胶囊用，
 *   而状态该用 `StatusPill`。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

const COUNT_TONE = {
  neutral: "bg-jn-well text-jn-text-2",
  primary: "bg-jn-primary-soft text-jn-primary-ink",
  danger: "bg-jn-danger-soft text-jn-danger",
} as const

export type CountTone = keyof typeof COUNT_TONE

/** 标题后的计数。最小宽度保证个位数也是圆形，不会忽宽忽窄。 */
export function Count({
  children,
  tone = "neutral",
  className,
}: {
  children: ReactNode
  tone?: CountTone
  className?: string
}) {
  return (
    <span
      className={cn(
        "tnum inline-flex h-5 min-w-5 items-center justify-center rounded-full px-1.5 font-mono text-[11px] font-semibold",
        COUNT_TONE[tone],
        className,
      )}
    >
      {children}
    </span>
  )
}

/** 描述性小标签。不可点、不表状态。 */
export function Tag({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center rounded-[6px] bg-jn-well px-1.5 text-[10px] font-medium text-jn-text-3 whitespace-nowrap",
        className,
      )}
    >
      {children}
    </span>
  )
}
