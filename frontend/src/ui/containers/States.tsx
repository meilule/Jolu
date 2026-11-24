/**
 * EmptyState / Skeleton / Banner —— 设计稿 R1-C「分区 · 5 · 容器与反馈」。
 *
 * 三者都是「页面没有正常内容时」的表达，放在一个文件因为它们必须成套使用、
 * 口径一致：
 *
 * | 组件 | 用在哪 | 设计稿要求 |
 * |---|---|---|
 * | EmptyState | 空列表、没有连接、没有记录 | 说明原因并给一个下一步 |
 * | Skeleton | 加载中 | 骨架块，不用转圈占满页面 |
 * | Banner | 页面顶部 | 持续状态（降级、只读、实验功能） |
 *
 * ## 为什么空状态必须给下一步
 *
 * 只写「暂无数据」等于把问题丢回给用户：是他没配置，还是加载失败，还是真的
 * 没有？`action` 参数强制调用方想清楚「用户接下来该做什么」，想不清楚时
 * 至少说明原因。这两项都不给会被类型检查拦下（`reason` 必填）。
 *
 * ## 为什么加载用骨架而不是转圈
 *
 * 转圈不告诉用户「将要出现什么」。骨架保留布局，内容到位时不会跳一下。
 * 只有「不知道会有什么形状」的场景才用转圈（比如整页首次加载）。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"
import { Icon, type IconName } from "@/icons"

/* ─────────────────────────── EmptyState ─────────────────────────── */

export interface EmptyStateProps {
  /** 标题。一句话说清「这里为什么是空的」 */
  title: string
  /** 原因说明。为什么空、缺什么——必填，不允许只写「暂无数据」 */
  reason: string
  /** 下一步动作。按钮或链接。确实没有可做的动作时传 null 并确认原因已写清 */
  action?: ReactNode
  icon?: IconName
  className?: string
}

export function EmptyState({ title, reason, action, icon = "info", className }: EmptyStateProps) {
  return (
    <div className={cn("flex flex-col items-center justify-center gap-2 py-10 text-center", className)}>
      <span className="flex size-11 items-center justify-center rounded-full bg-jn-well text-jn-text-3">
        <Icon name={icon} size="lg" />
      </span>
      <p className="mt-1 text-[13px] font-semibold text-jn-text">{title}</p>
      <p className="max-w-[320px] text-[11px] leading-relaxed text-jn-text-3">{reason}</p>
      {action && <div className="mt-2">{action}</div>}
    </div>
  )
}

/* ──────────────────────────── Skeleton ──────────────────────────── */

/**
 * 骨架块。`width` 用类名控制（`w-1/2` 这类），不传像素值——
 * 骨架的意义是保留布局结构，宽度应该跟着容器走。
 */
export function Skeleton({ className }: { className?: string }) {
  return <span aria-hidden className={cn("block animate-pulse rounded-[8px] bg-jn-well", className)} />
}

/** 多行骨架。`lines` 控制行数，最后一行的宽度缩短以模拟真实文本。 */
export function SkeletonLines({ lines = 3, className }: { lines?: number; className?: string }) {
  return (
    <div className={cn("flex flex-col gap-2", className)}>
      {Array.from({ length: lines }, (_, index) => (
        <Skeleton key={index} className={index === lines - 1 ? "h-3 w-2/3" : "h-3 w-full"} />
      ))}
    </div>
  )
}

/* ───────────────────────────── Banner ───────────────────────────── */

const BANNER_TONE = {
  warning: "bg-jn-warning-soft text-jn-warning",
  danger: "bg-jn-danger-soft text-jn-danger",
  info: "bg-jn-info-soft text-jn-info",
} as const

const BANNER_ICON = { warning: "warning", danger: "danger", info: "info" } as const

export type BannerTone = keyof typeof BANNER_TONE

/**
 * 页面顶部的持续状态条。
 *
 * 与 Toast 的区别：Toast 是「刚发生了什么」的一次性通知，会自动消失；
 * Banner 是「现在处于什么状态」（降级运行、只读模式、实验功能），
 * 只要状态还在就一直显示。
 */
export function Banner({
  tone = "info",
  children,
  action,
  className,
}: {
  tone?: BannerTone
  children: ReactNode
  action?: ReactNode
  className?: string
}) {
  return (
    <div
      role="status"
      className={cn(
        "flex w-full items-center gap-2 rounded-[14px] px-3 py-2 text-[11px] leading-relaxed",
        BANNER_TONE[tone],
        className,
      )}
    >
      <Icon name={BANNER_ICON[tone]} size="sm" className="shrink-0" />
      <span className="min-w-0 flex-1">{children}</span>
      {action && <span className="shrink-0">{action}</span>}
    </div>
  )
}
