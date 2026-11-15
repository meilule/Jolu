/**
 * 模型路由概览纯逻辑（Jolu.pen「统一网关 · R2 重设计」V6 概览画板 cb7KY / Cktyu）。
 *
 * 六项指标里的「今日请求 / 改道 / 最终失败」由后端 `gateway_day_overview` 给出（全天口径）；
 * 趋势、来源分布、按工具统计与响应头分位从路由流水行推导——流水只保留最近约 1,000 行，
 * 所以这些聚合都是「当前保留范围内可见的部分」，调用方要如实注明。
 */
import type { RouteLogDto } from "./gatewayApi"

/** 同一次请求的最终尝试行：分组成员失败转移会记多行，只有最后一行算最终结果。 */
export function finalAttempts(rows: readonly RouteLogDto[]): RouteLogDto[] {
  const counts = new Map<number, number>()
  const finalId = new Map<number, number>()
  for (const row of rows) {
    if (typeof row.request_id !== "number") continue
    const count = (counts.get(row.request_id) ?? 0) + 1
    counts.set(row.request_id, count)
    const best = finalId.get(row.request_id)
    // id 自增，同组内 id 最大即最后尝试
    finalId.set(row.request_id, Math.max(best ?? 0, row.id ?? 0))
  }
  return rows.filter((row) => {
    if (typeof row.request_id !== "number") return true
    if ((counts.get(row.request_id) ?? 0) < 2) return true
    return row.id === finalId.get(row.request_id)
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
