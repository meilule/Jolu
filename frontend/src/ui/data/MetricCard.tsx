/**
 * MetricCard —— 设计稿 R1-C「分区 · 4 · 状态与数据 / 指标卡 MetricCard」。
 *
 * 页面上**所有指标都用它**，不各写一套「标题 + 大数字 + 一行说明」。
 *
 * 规格取自画板：
 * | 元素 | 规格 |
 * |---|---|
 * | 容器 | `jn-surface`、圆角 24、1px `jn-line`、内边距 上18 右18 下14 左18、间距 6 |
 * | 标题 | 14 / 600 / `jn-text`；右侧可放范围文字（12 / `jn-text-3`） |
 * | 数值 | 34 / 700 + 单位 15 / 600 / `jn-text-2`，与标题间上 6 |
 * | 对比 | 12 / `jn-text-3` |
 * | 迷你图 | 可选插槽，高 8 |
 *
 * ## 下内边距更小
 *
 * 上 18 下 14：底部留白比顶部少，因为底部常跟一条来源脚注或迷你图，
 * 视觉重量已经在下半部分。等边距会让卡片显得头重脚轻。
 *
 * ## 数值为什么要走 DataValue
 *
 * 「12.48M」和「—」在信任度上完全不同。指标卡的数字位是 `children` 插槽，
 * 调用方传 `DataValue`（七态）而不是裸字符串——这样「未知」不会被写成 `0`，
 * 「估算」不会被当成实测。组件本身不解析数字，只负责排版。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

export interface MetricCardProps {
  title: ReactNode
  /** 右上角范围说明，如「今天」「近 7 天」 */
  scope?: ReactNode
  /** 数值区。传 `DataValue` 或组合好的「数字 + 单位」 */
  value: ReactNode
  /** 对比说明，如「较昨天同时段 +18.6%」 */
  compare?: ReactNode
  /** 迷你图插槽 */
  spark?: ReactNode
  /** 底部来源脚注。传 `SourceNote` */
  source?: ReactNode
  className?: string
}

export function MetricCard({ title, scope, value, compare, spark, source, className }: MetricCardProps) {
  return (
    <div
      className={cn(
        "flex flex-col gap-1.5 rounded-jn-card border border-jn-line bg-jn-surface px-[18px] pt-[18px] pb-3.5 shadow-jn-surface",
        className,
      )}
    >
      <div className="flex w-full items-center gap-[7px]">
        <span className="text-sm font-semibold text-jn-text">{title}</span>
        {/* 弹性间隔把范围文字推到右侧，标题长短不影响它的位置 */}
        <span className="flex-1" />
        {scope && <span className="shrink-0 text-xs text-jn-text-3">{scope}</span>}
      </div>
      <div className="flex items-baseline gap-1 pt-1.5">{value}</div>
      {compare && <p className="text-xs text-jn-text-3">{compare}</p>}
      {spark && <div className="h-2 w-full">{spark}</div>}
      {source}
    </div>
  )
}
