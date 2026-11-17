import { test } from "node:test"
import assert from "node:assert/strict"

import {
  finalAttempts, successRate, hourlyTrend, sourceDistribution, byAgent, headerLatency, rateLimitedSources, formatLatency,
} from "./gatewayOverview.ts"

const DAY_START = new Date("2026-11-15T00:00:00").getTime()
const NOW = new Date("2026-11-15T14:32:00").getTime()
const HOUR = 3_600_000

const row = (over = {}) => ({
  id: undefined, ts: NOW, agent: null, requested_model: "m", source: null, upstream_model: null,
  status_code: null, result: "ok", duration_ms: null, request_id: null, ...over,
})

test("finalAttempts 保留无分组的行，分组只留最后一次尝试", () => {
  const rows = [
    row({ id: 1, request_id: 7, result: "failed", source: "a" }),
    row({ id: 2, request_id: 7, result: "ok", source: "b" }),
    row({ id: 3, source: "c" }),
  ]
  const final = finalAttempts(rows)
  assert.deepEqual(final.map((r) => r.id), [2, 3])
})

test("successRate 是「没有收到错误的比例」，0 请求与异常输入都如实为 null", () => {
  assert.deepEqual(successRate(126, 2), { value: 124 / 126, ok: 124 })
  assert.deepEqual(successRate(0, 0), { value: null, ok: 0 })
  assert.equal(successRate(null, 2).value, null)
  assert.equal(successRate(10, 20).value, null)
})

test("hourlyTrend 从 00 点补齐到当前小时，失败行计入红柱", () => {
  const rows = [
    row({ ts: DAY_START + 2 * HOUR, result: "ok" }),
    row({ ts: DAY_START + 8 * HOUR + 60_000, result: "failed" }),
    row({ ts: DAY_START - HOUR, result: "ok" }), // 昨天的行不计入
    row({ ts: NOW + HOUR, result: "ok" }), // 超出现在不计入
  ]
  const buckets = hourlyTrend(rows, DAY_START, NOW)
  assert.equal(buckets.length, 15) // 00:00–14:32 → 15 格
  assert.equal(buckets[2].ok, 1)
  assert.equal(buckets[8].failed, 1)
  assert.equal(buckets[0].ok, 0)
  assert.equal(buckets[14].ok, 0)
})

test("sourceDistribution 按最终尝试聚合并算占比，拒绝层的行（无来源）不计入", () => {
  const rows = [
    row({ source: "work", result: "ok" }),
    row({ source: "work", result: "failed" }),
    row({ source: "personal", result: "ok" }),
    row({ source: null, result: "rejected" }),
  ]
  const shares = sourceDistribution(rows)
  assert.deepEqual(shares.map((s) => [s.source, s.total, s.failed]), [["work", 2, 1], ["personal", 1, 0]])
  assert.equal(shares[0].share, 2 / 3)
})

test("byAgent 只统计带 agent 的最终行", () => {
  const rows = [row({ agent: "claude-code" }), row({ agent: "claude-code" }), row({ agent: "codex" }), row({})]
  assert.deepEqual(byAgent(rows), [{ agent: "claude-code", total: 2 }, { agent: "codex", total: 1 }])
})

test("headerLatency P50 / P95 与无样本", () => {
  const rows = [100, 200, 300, 400, 500].map((ms, index) => row({ id: index, duration_ms: ms }))
  assert.equal(headerLatency(rows, 0.5), 300)
  assert.equal(headerLatency(rows, 0.95), 500)
  assert.equal(headerLatency([row({ duration_ms: null })], 0.5), null)
})

test("rateLimitedSources 只看最终失败的 429 行", () => {
  const rows = [
    row({ source: "personal", status_code: 429, result: "failed" }),
    row({ source: "personal", status_code: 429, result: "failed" }),
    row({ source: "work", status_code: 200, result: "ok" }),
  ]
  assert.deepEqual(rateLimitedSources(rows), [{ source: "personal", count: 2 }])
})

test("formatLatency 区分毫秒与秒、无数据", () => {
  assert.equal(formatLatency(842), "842 ms")
  assert.equal(formatLatency(1800), "1.8 s")
  assert.equal(formatLatency(18000), "18 s")
  assert.equal(formatLatency(null), "—")
})

// 活跃会话窄容器改用卡片、不隐藏字段（鼠鼠 2026-11-17：收起来的信息没法看）
test("活跃会话：窄档改卡片而非隐藏列，断点与网格类名一致", async () => {
  const { SESSION_GRID, SESSION_TABLE_BREAKPOINT, SESSION_CARD_BREAKPOINT } = await import("./gatewayOverview.ts")
  // 宽档六列、中档五列（收掉按钮列），窄档靠卡片、不再收列
  assert.match(SESSION_GRID, /grid-cols-\[minmax\(0,1fr\)_210px_190px_130px_110px_88px\]/)
  const five = SESSION_GRID.match(/@max-\[(\d+)px\]:grid-cols-\[[^\]]+\]/)
  assert.ok(five, "中档缺少容器查询列定义")
  assert.equal(Number(five[1]) + 1, SESSION_TABLE_BREAKPOINT, "中档断点要与表常量对齐")
  assert.ok(SESSION_CARD_BREAKPOINT < SESSION_TABLE_BREAKPOINT, "卡片档应比表格档更窄")
  // 类名必须写死字面量：Tailwind 4 静态扫描，模板串拼的类不会生成
  assert.doesNotMatch(SESSION_GRID, /\$\{/, "列定义不能含模板占位")
})

// 概览范围下拉：后端 today/yesterday 是「本期/上期」，文案必须跟着范围变
test("概览范围：四档齐全，未知值退回今天", async () => {
  const { OVERVIEW_RANGES, overviewRange } = await import("./gatewayOverview.ts")
  assert.deepEqual(OVERVIEW_RANGES.map((r) => r.id), ["today", "yesterday", "7d", "30d"])
  // 每档的指标名与对比文案都要不同，否则选「昨天」会显示「今日请求」
  assert.equal(overviewRange("today").period, "今日请求")
  assert.equal(overviewRange("today").compare, "昨天")
  assert.equal(overviewRange("yesterday").period, "昨日请求")
  assert.equal(overviewRange("7d").compare, "前 7 天")
  assert.equal(overviewRange("30d").period, "近 30 天请求")
  // 拼错不能把页面变空
  assert.equal(overviewRange("week").id, "today")
})
