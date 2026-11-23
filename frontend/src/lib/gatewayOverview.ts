/**
 * 模型路由概览纯逻辑（Jolu.pen「统一网关 · R2 重设计」V6 概览画板 cb7KY / Cktyu）。
 *
 * 六项指标里的「今日请求 / 改道 / 最终失败」由后端 `gateway_day_overview` 给出（全天口径）；
 * 趋势、来源分布、按工具统计与响应头分位从路由流水行推导——流水只保留最近约 1,000 行，
 * 所以这些聚合都是「当前保留范围内可见的部分」，调用方要如实注明。
 */
import type { RouteLogDto } from "./gatewayApi"

/**
 * 活跃会话表格在窄容器下的三种呈现形态（配合 `@container` 的 `@max-[..]` 断点）。
 *
 * 宽容器用表格；容器变窄时**不隐藏列**（隐藏掉「缓存读取」「状态」「查看上下文」会让用户
 * 无从得知这些信息存在），而是整行折成两层卡片：第一层会话 + 模型，第二层上下文 +
 * 缓存 + 状态 + 入口，所有字段都保留。
 *
 * 阈值来自实测：六列固定宽合计约 718px，加第一列与内边距，容器低于 ~920px 起第一列就会被
 * 压到放不下标题（2026-11-17 尺寸扫描）；两层卡片在 ~620px 以下才开始挤，所以：
 *   - ≥ 920px：六列表格
 *   - 620–919px：四列表格（只收最次要的按钮列，其余照显示）
 *   - < 620px：两层卡片（全部字段显示）
 */
export const SESSION_TABLE_BREAKPOINT = 920
export const SESSION_CARD_BREAKPOINT = 620

/**
 * 表头与数据行共用的列定义（宽档与中档；窄档改用卡片，不用网格）。
 *
 * 注意：类名必须写完整字面量，不能用模板串拼 `@max-[${n}px]`——
 * Tailwind 4 在构建时静态扫描源码文本，拼出来的类名扫不到、样式不会生成。
 * 改断点时要同步改 `SESSION_TABLE_BREAKPOINT` / `SESSION_CARD_BREAKPOINT` 常量。
 */
export const SESSION_GRID = [
  "grid-cols-[minmax(0,1fr)_210px_190px_130px_110px_88px]",
  "@max-[919px]:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)_minmax(0,0.9fr)_130px_110px]",
].join(" ")

/** 同一次请求的最终尝试行：分组成员失败转移会记多行，只有最后一行算最终结果。 */
export function finalAttempts(rows: readonly RouteLogDto[]): RouteLogDto[] {
  const counts = new Map<number, number>()
  const finalId = new Map<number, number>()
  for (const row of rows) {
    if (typeof row.request_uid !== "number") continue
    const count = (counts.get(row.request_uid) ?? 0) + 1
    counts.set(row.request_uid, count)
    const best = finalId.get(row.request_uid)
    // id 自增，同组内 id 最大即最后尝试
    finalId.set(row.request_uid, Math.max(best ?? 0, row.id ?? 0))
  }
  return rows.filter((row) => {
    if (typeof row.request_uid !== "number") return true
    if ((counts.get(row.request_uid) ?? 0) < 2) return true
    return row.id === finalId.get(row.request_uid)
  })
}

/** 成功率：Agent 没收到错误的请求占今日请求的比例（画板「124 / 126 成功」）。 */
export function successRate(today: number | null | undefined, errors: number | null | undefined): { value: number | null; ok: number | null } {
  if (typeof today !== "number" || typeof errors !== "number" || today < 0 || errors < 0 || errors > today) return { value: null, ok: null }
  if (today === 0) return { value: null, ok: 0 }
  const ok = today - errors
  return { value: ok / today, ok }
}

export interface TrendBucket {
  /** 小时起点（毫秒，本地时区整点） */
  at: number
  hour: number
  ok: number
  failed: number
}

/** 每小时请求趋势：今天 00:00 起到当前小时的每一格（没有请求的格子补 0，画板柱子从 00 画到当前小时）。 */
export function hourlyTrend(rows: readonly RouteLogDto[], dayStartMs: number, nowMs: number): TrendBucket[] {
  const lastHour = Math.floor(Math.max(0, nowMs - dayStartMs) / 3_600_000)
  const buckets: TrendBucket[] = []
  const index = new Map<number, TrendBucket>()
  for (let h = 0; h <= lastHour; h += 1) {
    const at = dayStartMs + h * 3_600_000
    const bucket: TrendBucket = { at, hour: new Date(at).getHours(), ok: 0, failed: 0 }
    buckets.push(bucket)
    index.set(h, bucket)
  }
  for (const row of finalAttempts(rows)) {
    if (row.ts < dayStartMs || row.ts > nowMs) continue
    const hour = Math.floor((row.ts - dayStartMs) / 3_600_000)
    const bucket = index.get(hour)
    if (!bucket) continue
    if (row.result === "ok") bucket.ok += 1
    else bucket.failed += 1
  }
  return buckets
}

export interface SourceShare {
  source: string
  total: number
  failed: number
  /** 占聚合范围内请求的比例（0–1）；范围内没有请求时为 null */
  share: number | null
}

/** 来源分布（画板右侧卡）：按最终尝试行聚合；来源为 null 的行（被路由层拒绝）不计入。 */
export function sourceDistribution(rows: readonly RouteLogDto[]): SourceShare[] {
  const counts = new Map<string, { total: number; failed: number }>()
  for (const row of finalAttempts(rows)) {
    if (!row.source) continue
    const entry = counts.get(row.source) ?? { total: 0, failed: 0 }
    entry.total += 1
    if (row.result !== "ok") entry.failed += 1
    counts.set(row.source, entry)
  }
  const total = [...counts.values()].reduce((sum, entry) => sum + entry.total, 0)
  return [...counts.entries()]
    .map(([source, entry]) => ({ source, total: entry.total, failed: entry.failed, share: total > 0 ? entry.total / total : null }))
    .sort((a, b) => b.total - a.total)
}

/** 按工具统计（画板趋势卡底部「Claude Code 88 · Codex 38」），量大的在前。 */
export function byAgent(rows: readonly RouteLogDto[]): { agent: string; total: number }[] {
  const counts = new Map<string, number>()
  for (const row of finalAttempts(rows)) {
    if (!row.agent) continue
    counts.set(row.agent, (counts.get(row.agent) ?? 0) + 1)
  }
  return [...counts.entries()].map(([agent, total]) => ({ agent, total })).sort((a, b) => b.total - a.total)
}

/** 响应头耗时分位（毫秒）：只统计有耗时的最终尝试行；没有样本时为 null。 */
export function headerLatency(rows: readonly RouteLogDto[], p: 0.5 | 0.95): number | null {
  const samples = finalAttempts(rows)
    .map((row) => row.duration_ms)
    .filter((value): value is number => typeof value === "number" && value >= 0)
    .sort((a, b) => a - b)
  if (samples.length === 0) return null
  const at = Math.min(samples.length - 1, Math.max(0, Math.ceil(p * samples.length) - 1))
  return samples[at]
}

/** 触发限流（429）的来源：概览「需要关注」里「个人账号返回限流」一条的来源。 */
export function rateLimitedSources(rows: readonly RouteLogDto[]): { source: string; count: number }[] {
  const counts = new Map<string, number>()
  for (const row of finalAttempts(rows)) {
    if (row.status_code !== 429) continue
    if (!row.source) continue
    counts.set(row.source, (counts.get(row.source) ?? 0) + 1)
  }
  return [...counts.entries()].map(([source, count]) => ({ source, count })).sort((a, b) => b.count - a.count)
}

export function formatLatency(ms: number | null | undefined): string {
  if (typeof ms !== "number" || ms < 0) return "—"
  return ms >= 1000 ? `${(ms / 1000).toFixed(ms >= 10_000 ? 0 : 1)} s` : `${Math.round(ms)} ms`
}

/**
 * 概览页时间范围（V10-01 右上「今天」下拉）。
 * 后端 `Range::parse` 认这些 id；`compare` / `period` 是给文案用的说法：
 * 后端返回的 `today` 字段是**本期**请求数、`yesterday` 是**上期**，所以文案必须跟着范围变，
 * 否则选「昨天」还会显示「今天 0 请求」。
 */
export interface OverviewRange {
  id: "today" | "yesterday" | "7d" | "30d"
  label: string
  /** 本期指标名，如「今日请求」「近 7 天请求」 */
  period: string
  /** 对比文案前缀，如「昨天」「前一天」「前 7 天」 */
  compare: string
}

export const OVERVIEW_RANGES: readonly OverviewRange[] = [
  { id: "today", label: "今天", period: "今日请求", compare: "昨天" },
  { id: "yesterday", label: "昨天", period: "昨日请求", compare: "前一天" },
  { id: "7d", label: "近 7 天", period: "近 7 天请求", compare: "前 7 天" },
  { id: "30d", label: "近 30 天", period: "近 30 天请求", compare: "前 30 天" },
]

export function overviewRange(id: string): OverviewRange {
  return OVERVIEW_RANGES.find((item) => item.id === id) ?? OVERVIEW_RANGES[0]
}
