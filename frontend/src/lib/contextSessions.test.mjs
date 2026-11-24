import { test } from "node:test"
import assert from "node:assert/strict"

import {
  CONTEXT_RISK, accountOf, activeSessionRows, cacheCell, contextCell, contextRisks, filterRows, projectOf, statusCell,
} from "./contextSessions.ts"

const win = (...parts) => parts.join(String.fromCharCode(92))

const ctx = (over = {}) => ({
  id: 1, session_platform: "claude", session_id: "s1", request_id: 7, ts: 1_000, state: "ready", protocol: "anthropic",
  input_tokens: 168_000, output_tokens: 20, cache_read: 162_000, window_tokens: 200_000, estimated: 150_000, counted: true, error: null,
  agent: "claude-code", requested_model: "claude/claude-sonnet-4-5", upstream_model: "claude-sonnet-4-5", source: "claude",
  route_result: "ok", status_code: 200, duration_ms: 900, connection_id: "c1", connection_name: "工作账号", connection_platform: "claude",
  last_ready_ts: 1_000, snapshots: 3, ...over,
})

const session = (over = {}) => ({
  platform: "claude", session_id: "s1", agent: "claude-code", title: "接线修复", cwd: win("D:", "work", "Jolu"), thread_last_ts: 500, gateway_last_ts: 600,
  thread_status: null, tokens: {}, links: [{ session_platform: "claude", session_id: "s1", connection_id: "c1", source: "claude", requested_model: "claude/x", upstream_model: "x", agent: "claude-code", last_ts: 600, requests: 2 }],
  ...over,
})

const view = (s, running = false, activity = 600) => ({ key: `${s.platform}\u001f${s.session_id}`, session: s, running, liveConnections: [], activity, prioritized: false })

test("上下文单元格：实测量 / 窗口，达到阈值标红", () => {
  const cell = contextCell(ctx(), true)
  assert.equal(cell.text, "168K / 200K")
  assert.equal(cell.ratio, 0.84)
  assert.equal(cell.tone, "danger")
  assert.equal(contextCell(ctx({ input_tokens: 74_200, window_tokens: 400_000 }), true).tone, "normal")
  assert.ok(CONTEXT_RISK > 0 && CONTEXT_RISK < 1)
})

test("上下文单元格：窗口未知写「容量未知」并隐藏比例；没实测时用估算并注明", () => {
  const unknown = contextCell(ctx({ input_tokens: 631_000, window_tokens: null }), true)
  assert.equal(unknown.text, "631K / 容量未知")
  assert.equal(unknown.ratio, null)
  const estimated = contextCell(ctx({ input_tokens: null, counted: false, estimated: 50_000 }), true)
  assert.equal(estimated.text, "50K / 200K")
  assert.match(estimated.title, /估算/)
})

test("上下文单元格：没有快照、分析中、失败、不支持分别表达，不写 0", () => {
  assert.equal(contextCell(null, false).text, "暂无请求")
  assert.equal(contextCell(null, true).text, "尚未采集")
  const empty = { input_tokens: null, estimated: null, cache_read: null }
  assert.equal(contextCell(ctx({ ...empty, state: "analyzing" }), true).text, "正在分析")
  const failed = contextCell(ctx({ ...empty, state: "failed", last_ready_ts: new Date("2026-11-17T14:28:00").getTime() }), true)
  assert.equal(failed.text, "采集失败")
  assert.match(failed.title, /上次成功 14:28/)
  assert.equal(contextCell(ctx({ ...empty, state: "unsupported" }), true).text, "暂不支持分析")
  assert.equal(contextCell(ctx({ ...empty, state: "no_body" }), true).text, "尚未采集")
})

test("缓存列：没给写「未提供」，给了 0 就是 0，占比的分母是输入", () => {
  assert.equal(cacheCell(null).text, "未提供")
  assert.equal(cacheCell(ctx({ cache_read: null })).text, "未提供")
  assert.equal(cacheCell(ctx({ cache_read: 0 })).text, "0 · 0.0%")
  assert.equal(cacheCell(ctx()).text, "162K · 96.4%")
  assert.equal(cacheCell(ctx({ input_tokens: null })).text, "162K")
})

test("状态、项目与来源账号", () => {
  assert.deepEqual(statusCell(true), { text: "正在生成", tone: "on" })
  assert.deepEqual(statusCell(false), { text: "最近活跃", tone: "muted" }, "没有证据时不断言空闲")
  // P4 审计 D12：没有在途请求但会话仍在运行（思考、执行本地工具）时不能写「空闲」
  assert.deepEqual(statusCell(false, "running"), { text: "运行中（无在途请求）", tone: "on" })
  assert.equal(statusCell(false, "done").text, "已完成")
  assert.equal(projectOf(win("D:", "work", "Jolu", "")), "Jolu")
  assert.equal(projectOf("/home/dev/proj"), "proj")
  assert.equal(projectOf(null), null)
  assert.equal(accountOf(ctx(), null), "Claude Code · 工作账号", "平台名取供应商目录")
  assert.equal(accountOf(ctx({ connection_name: null, connection_platform: null }), null), "claude")
  assert.equal(accountOf(null, "kimi"), "kimi")
})

test("合并最近会话与快照：运行中在前，只有快照的会话也列出", () => {
  const a = session()
  const b = session({ session_id: "s2", title: null, links: [] })
  const rows = activeSessionRows(
    [view(a, false, 600), view(b, true, 100)],
    [ctx(), ctx({ id: 9, session_id: "s3", ts: 5_000 })],
  )
  assert.deepEqual(rows.map((r) => r.sessionId), ["s2", "s3", "s1"])
  const first = rows.find((r) => r.sessionId === "s1")
  assert.equal(first.title, "接线修复")
  assert.equal(first.project, "Jolu")
  assert.equal(first.model, "claude-sonnet-4-5")
  assert.equal(first.lastTs, 1_000)
  assert.equal(first.snapshotId, 1)
  const noRoute = rows.find((r) => r.sessionId === "s2")
  assert.equal(noRoute.context.text, "暂无请求")
  assert.equal(noRoute.cache.text, "未提供")
  assert.equal(noRoute.snapshotId, null)
})

test("筛选与需要关注", () => {
  const rows = activeSessionRows(
    [view(session(), true), view(session({ session_id: "s2", title: "文档与验收", cwd: "/p/docs" }))],
    [ctx(), ctx({ id: 2, session_id: "s2", input_tokens: 74_200, window_tokens: 400_000 })],
  )
  assert.deepEqual(filterRows(rows, "", "running").map((r) => r.sessionId), ["s1"])
  assert.deepEqual(filterRows(rows, "", "risk").map((r) => r.sessionId), ["s1"])
  assert.deepEqual(filterRows(rows, "DOCS", "all").map((r) => r.sessionId), ["s2"])
  assert.deepEqual(filterRows(rows, "Claude Code", "all", (a) => (a === "claude-code" ? "Claude Code" : a)).map((r) => r.sessionId), ["s1", "s2"])
  assert.equal(filterRows(rows, "", "uncollected").length, 0)
  const risks = contextRisks(rows, () => "Claude Code")
  assert.equal(risks.length, 1)
  assert.equal(risks[0].title, "Claude Code 上下文已用 84%")
  assert.equal(risks[0].detail, "最近一次请求 · 168K / 200K")
})
