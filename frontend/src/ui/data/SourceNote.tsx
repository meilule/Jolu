/**
 * SourceNote —— 设计稿 R1-C「分区 · 4 · 状态与数据 / 来源脚注 SourceNote」。
 *
 * 产品定位 §5.1 的界面落点：**每项数据保留来源、采集时间和覆盖范围**，
 * 卡片底部统一用这一行，不各写一种「来自…」的文案。
 *
 * ## 四种来源
 *
 * | source | 含义 | 例 |
 * |---|---|---|
 * | official | 官方接口直读 | 官方额度、官方用量 |
 * | native | 本机工具日志 | 本机会话记录 |
 * | routed | 经 Jolu 网关转发时记的 | 网关流水 |
 * | mixed | 多个来源合成（需在 detail 里说清） | 官方额度 + 本机记录 |
 *
 * 来源必须能分辨，因为可信度与新鲜度完全不同：官方额度可能 5 分钟拉一次，
 * 本机记录是实时的，网关流水只覆盖经过 Jolu 的请求。混在一句「统计数据来源」
 * 里，用户无法判断缺的那部分是因为没走网关还是因为采集失败。
 *
 * ## 采集时间与覆盖范围
 *
 * 两个都是可选：没有时间就不要写「刚刚」；没有覆盖范围就不要默认「全部」。
 * 宁可不写，也不给一个会让人误判的默认值。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

const SOURCE_LABEL = {
  official: "官方接口",
  native: "本机记录",
  routed: "网关记录",
  mixed: "多来源",
} as const

export type DataSource = keyof typeof SOURCE_LABEL

export interface SourceNoteProps {
  source: DataSource
  /** 采集时刻，如「2 分钟前」「今天 09:12」。不传则不写时间——不编「刚刚」 */
  collectedAt?: string
  /** 覆盖范围，如「仅本机」「含已清理历史」「3 个账号中 2 个」 */
  coverage?: string
  /** 额外说明（如「无法按账号区分」）。与上面三项并列显示 */
  detail?: ReactNode
  className?: string
}

export function SourceNote({ source, collectedAt, coverage, detail, className }: SourceNoteProps) {
  // 用 `·` 连接，缺失项自动跳过——比手拼字符串少一处「多一个点」的机会
  const parts = [SOURCE_LABEL[source], collectedAt, coverage].filter(Boolean) as string[]
  return (
    <p className={cn("text-[10px] leading-relaxed text-jn-text-3", className)}>
      {parts.join(" · ")}
      {detail && <> · {detail}</>}
    </p>
  )
}
