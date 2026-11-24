/**
 * Card —— 设计稿 R1-C「分区 · 5 · 容器与反馈 / 卡片 Card」。
 *
 * 页面里**唯一的卡片外框**。以前 `.jolu-card` 与几十处手写 `rounded border bg-*`
 * 并存，圆角与阴影各不相同；这里把它们收成一个组件。
 *
 * ## 规格（取自画板）
 *
 * | 项 | 值 | 来源 |
 * |---|---|---|
 * | 底色 | `jn-surface` | 画板 fill |
 * | 圆角 | 24 | `jn-r-card` |
 * | 描边 | 1px `jn-line` | 画板 stroke |
 * | 投影 | `0 1px 3px jn-shadow` | 画板 shadow effect（全稿 223 次引用） |
 * | 内边距 | 18 | 画板 padding |
 * | 元素间距 | 12 | 画板 gap |
 *
 * ## 两种 tone
 *
 * - `default`：白底。绝大多数卡片。
 * - `wash`：主色淡底（`jn-primary-wash`）。用于「提示 / 说明」类卡片，
 *   让它在视觉上退到内容后面，不与数据卡抢注意力。
 *
 * ## 为什么标题行是插槽而不是 `title` 字符串
 *
 * 标题行右侧经常还要放计数、状态胶囊或一个图标按钮。传字符串就只能再开一个
 * `extra` 参数，然后各页各写一套对齐方式。给 `header` 插槽，对齐由 Card 保证。
 */
import type { HTMLAttributes, ReactNode } from "react"
import { cn } from "@/lib/utils"

const TONE = {
  default: "bg-jn-surface",
  /** 提示类：主色淡底，视觉上退后一层 */
  wash: "bg-jn-primary-wash",
} as const

export type CardTone = keyof typeof TONE

export interface CardProps extends HTMLAttributes<HTMLDivElement> {
  tone?: CardTone
  /**
   * 标题行。放标题文字，以及右侧的计数 / 状态 / 图标按钮。
   * 不传则整块都是内容区。
   */
  header?: ReactNode
  /** 内容区。多个子元素之间自动 12px 间距。 */
  children?: ReactNode
  /** 去掉内边距与标题行间距，用于表格、列表这类自己管贴边的卡片。 */
  flush?: boolean
}

export function Card({ tone = "default", header, children, flush = false, className, ...rest }: CardProps) {
  return (
    <div
      className={cn(
        "flex flex-col rounded-jn-card border border-jn-line shadow-jn-surface",
        TONE[tone],
        flush ? "gap-0" : "gap-3 p-[18px]",
        className,
      )}
      {...rest}
    >
      {header && (
        // 标题行：左标题右操作，垂直居中。间距 7px 取自画板。
        <div className={cn("flex w-full items-center gap-[7px]", flush && "px-[18px] pt-[18px]")}>{header}</div>
      )}
      {children}
    </div>
  )
}

/**
 * 卡片标题文字。与 `Card` 的 `header` 插槽配合使用。
 *
 * 规格取自画板：14px / 600 / `jn-text`。单独成一个组件是为了让字号与字重
 * 只有一处定义——页面不自己挑字号。
 *
 * `hint` 是画板外的扩展（副标题在画板上没画）：现有界面不少卡片需要「主标题 +
 * 一行说明」，没有这个参数各页会自己拼，字号立刻分叉。等画板补了副标题样式
 * 再按画板改，现在先按 11px / `jn-text-3` 与列表行的副文字保持一致。
 */
export function CardTitle({ children, hint }: { children: ReactNode; hint?: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <span className="truncate text-sm font-semibold text-jn-text">{children}</span>
      {hint && <span className="truncate text-[11px] text-jn-text-3">{hint}</span>}
    </div>
  )
}
