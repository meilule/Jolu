import { test } from "node:test"
import assert from "node:assert/strict"

import {
  KINDS, baseName, cacheCard, callLines, composition, contextCard, countLabel, deviationNote, evidenceOf, historyLabel,
  initialFollow, ioSummary, newSince, onHistory, pause, resume, routeChain, runSummary, tableRows, treemap, visibleRows,
} from "./contextObservation.ts"

const parts = [
  { kind: "system", tokens: 3530, items: [{ name: "prompt", tag: "", tokens: 3500 }, { name: "billing", tag: "", tokens: 30 }] },
  { kind: "tools", tokens: 2710, items: [{ name: "Bash", tag: "", tokens: 900 }, { name: "linear", tag: "mcp", n: 3, tokens: 1800 }, { name: "Read", tag: "", tokens: 10 }] },
  { kind: "skills", tokens: 1660, items: [{ name: "skills", tag: "skills", tokens: 820 }, { name: "surgical-code-change", tag: "loaded", tokens: 840 }] },
  { kind: "memory", tokens: 1420, items: [{ name: "D:/work/Jolu/AGENTS.md", tag: "project", tokens: 1200 }, { name: "reminders", tag: "reminder", tokens: 220 }] },
  { kind: "files", tokens: 1124, items: [{ name: "backend/src/gateway/route.rs", tag: "read", tokens: 1000 }, { name: "src/cart.ts", tag: "shell", tokens: 124 }] },
  { kind: "results", tokens: 2068, items: [{ name: "Bash", tag: "result", n: 299, tokens: 888 }, { name: "Read", tag: "result", tokens: 762 }, { name: "more", tag: "more", n: 3, tokens: 418 }] },
  { kind: "chat", tokens: 1800, items: [{ name: "earlier", tag: "turns", n: 39, tokens: 1000 }, { name: "40", tag: "turn", tokens: 300 }, { name: "62", tag: "turn", tokens: 500 }] },
  { kind: "unknown", tokens: 386, items: [{ name: "hologram", tag: "type", tokens: 386 }] },
]
const TOTAL = parts.reduce((s, p) => s + p.tokens, 0)

const summary = (over = {}) => ({
  id: 9, session_platform: "claude", session_id: "s", request_id: 1, ts: new Date("2026-11-17T14:32:08").getTime(), state: "ready", protocol: "anthropic",
  input_tokens: 631_000, output_tokens: 2_800, cache_read: 602_000, window_tokens: 1_000_000, estimated: 600_000, counted: true, error: null,
  agent: "zcode", requested_model: "group/jolu-dev", upstream_model: "deepseek-v4.1-flash-sg", source: "workbuddy", route_result: "ok",
  status_code: 200, duration_ms: 18_400, connection_id: "c1", connection_name: "个人账号", connection_platform: "workbuddy", ...over,
})

test("八类顺序与颜色固定，组成按固定顺序补齐", () => {
  assert.deepEqual(KINDS.map((k) => k.kind), ["system", "tools", "skills", "memory", "files", "results", "chat", "unknown"])
  assert.equal(new Set(KINDS.map((k) => k.fill)).size, 8, "每类颜色不同")
  const c = composition([{ kind: "files", tokens: 30, items: [] }, { kind: "chat", tokens: 70, items: [] }])
  assert.deepEqual(c.map((x) => x.kind), KINDS.map((k) => k.kind))
  assert.equal(c.find((x) => x.kind === "chat").share, 0.7)
  assert.equal(c.find((x) => x.kind === "system").tokens, 0)
  assert.equal(composition(null)[0].share, null)
})

test("明细表：对话、工具定义、未分类各一行可展开；工具结果每个工具一行；记忆与文件带来源证据", () => {
  const rows = tableRows(parts)
  const chat = rows.find((r) => r.key === "chat")
  assert.equal(chat.countText, "41 轮")
  assert.equal(chat.subtitle, "第 1–62 轮 · 按轮次展开")
  assert.deepEqual(chat.children.map((c) => c.name), ["更早的 39 轮", "第 40 轮", "第 62 轮"])
  const tools = rows.find((r) => r.key === "tools")
  assert.equal(tools.subtitle, "本次实际提供的 5 个工具", "MCP 服务器按它下面的工具数算")
  assert.equal(tools.children.find((c) => c.name === "linear").sub, "MCP 服务器 · 3 个工具")
  const bash = rows.find((r) => r.key === "results:Bash")
  assert.deepEqual([bash.name, bash.countText, bash.expand, bash.tool], ["Bash 工具结果", "299", "calls", "Bash"])
  const more = rows.find((r) => r.key === "results:more")
  assert.equal(more.expand, null)
  const agents = rows.find((r) => r.kind === "memory" && r.title.endsWith("AGENTS.md"))
  assert.equal(agents.name, "AGENTS.md")
  assert.equal(agents.subtitle, "项目指令 · D:/work/Jolu/AGENTS.md")
  assert.equal(rows.find((r) => r.key === "skills:loaded:surgical-code-change").subtitle, "本次加载证据 · Skill 读取结果")
  assert.equal(rows.find((r) => r.key === "unknown").subtitle, "缺少来源标识 · 不推断为技能或文件")
  const sum = rows.reduce((s, r) => s + r.tokens, 0)
  assert.equal(sum, TOTAL, "各行之和等于总量，不重复计算")
  const shares = rows.reduce((s, r) => s + r.share, 0)
  assert.ok(Math.abs(shares - 1) < 1e-9)
})

test("筛选、搜索、排序与计数文字", () => {
  const rows = tableRows(parts)
  const sorted = visibleRows(rows, null, "")
  for (let i = 1; i < sorted.length; i += 1) assert.ok(sorted[i - 1].tokens >= sorted[i].tokens)
  assert.ok(visibleRows(rows, "files", "").every((r) => r.kind === "files"))
  assert.deepEqual(visibleRows(rows, null, "route.rs").map((r) => r.key), ["files:read:backend/src/gateway/route.rs"])
  assert.deepEqual(visibleRows(rows, null, "第 62").map((r) => r.key), ["chat"], "搜索也看展开项")
  const byKind = visibleRows(rows, null, "", "kind")
  assert.equal(byKind[0].kind, "system")
  assert.equal(countLabel(visibleRows(rows, "results", ""), "results"), "工具结果 · 3 类")
  assert.equal(countLabel(visibleRows(rows, "files", ""), "files"), "文件 · 2 项")
  assert.match(countLabel(rows, null), /^已识别 \d+ 项$/)
})

test("来源证据文案与路径名", () => {
  assert.equal(evidenceOf("memory", "user"), "用户全局指令")
  assert.equal(evidenceOf("files", "read"), "读文件工具返回 · 来源证据已识别")
  assert.equal(evidenceOf("skills", "skills"), "技能列表 · 可按需加载的 Skills")
  assert.equal(baseName("D:\\work\\Jolu\\AGENTS.md"), "AGENTS.md")
})

test("单次工具返回：新的在前，默认只显示前几条", () => {
  const calls = [1, 2, 3, 4].map((i) => ({ tool: "Bash", target: `cmd ${i}`, kind: "results", tokens: i * 1000, lines: i, exit_code: 0, masked: i === 2, summary: `退出码 0，输出 ${i} 行`, captured: false }))
  const { lines, total, tokens } = callLines(calls, 3)
  assert.equal(total, 4)
  assert.equal(tokens, 10_000)
  assert.deepEqual(lines.map((l) => l.index), [4, 3, 2])
  assert.equal(lines[0].target, "cmd 4")
  assert.equal(lines[0].tokens, "4K Token")
  assert.equal(callLines(calls, null).lines.length, 4)
})

test("占用卡：实测 / 估算 / 窗口未知 / 缓存未提供", () => {
  const c = contextCard(summary())
  assert.deepEqual([c.value, c.basis], ["631K / 1M", "上游实测"])
  assert.equal(c.ratio, 0.631)
  assert.equal(c.note, "剩余上下文空间 369K · 窗口容量由模型元数据提供")
  const unknown = contextCard(summary({ window_tokens: null }))
  assert.deepEqual([unknown.value, unknown.ratio], ["631K / 容量未知", null])
  assert.equal(contextCard(summary({ input_tokens: null, counted: false })).basis, "本地估算")
  assert.equal(contextCard(summary({ input_tokens: null, estimated: null })).value, "尚未采集")
  const cache = cacheCard(summary())
  assert.equal(cache.value, "602K")
  assert.ok(Math.abs(cache.ratio - 602 / 631) < 1e-9)
  assert.equal(cacheCard(summary({ cache_read: null })).value, "未提供")
  assert.equal(cacheCard(summary({ cache_read: 0 })).value, "0")
})

test("偏差提示、运行摘要与链路", () => {
  assert.equal(deviationNote(summary()), "本地估算 600K，比上游实测少 4.9%；组成已按实测等比例换算")
  assert.equal(deviationNote(summary({ counted: false })), null)
  const run = runSummary(summary())
  assert.deepEqual([run.status, run.tone], ["已完成", "on"])
  assert.deepEqual(run.parts, ["14:32:08", "完整用时 18.4s"])
  assert.equal(runSummary(summary({ route_result: "failed" })).status, "请求失败")
  assert.equal(ioSummary(summary({ output_tokens: null })), "输入 631K　/　输出 未提供 Token")
  assert.deepEqual(routeChain(summary(), (slug) => (slug === "jolu-dev" ? "Jolu 开发" : null)), {
    requested: "group/jolu-dev", group: "Jolu 开发", actual: "deepseek-v4.1-flash-sg · 个人账号",
  })
  assert.equal(routeChain(summary({ requested_model: "kimi/kimi-k2" }), () => null).group, null)
})

test("跟随最新：选旧请求、展开或筛选暂停；新请求只计数不替换；恢复才切到最新", () => {
  const h1 = [{ id: 126 }, { id: 125 }]
  let s = onHistory(initialFollow, h1)
  assert.deepEqual(s, { following: true, selectedId: 126, pausedAt: null })
  s = pause(s, h1, 125)
  assert.deepEqual(s, { following: false, selectedId: 125, pausedAt: 126 })
  const h2 = [{ id: 128 }, { id: 127 }, ...h1]
  s = onHistory(s, h2)
  assert.equal(s.selectedId, 125, "暂停时不替换已选快照")
  assert.equal(newSince(s, h2), 2)
  s = pause(s, h2)
  assert.equal(s.pausedAt, 126, "再次暂停不重置起点")
  s = resume(h2)
  assert.deepEqual(s, { following: true, selectedId: 128, pausedAt: null })
  assert.equal(newSince(s, h2), 0)
  assert.equal(onHistory(pause(initialFollow, h1, 1), h1).selectedId, 126, "已选的被清理时退回最新")
})

test("历史下拉文字", () => {
  assert.equal(historyLabel(summary(), true, false), "最新 · 已完成 · 输入 631K")
  assert.equal(historyLabel(summary({ route_result: "failed", input_tokens: null, estimated: null }), false, true), "已选 · 请求失败 · 用量未提供")
})

test("矩形图：面积按 token，铺满且不重叠", () => {
  const tiles = treemap([{ kind: "results", tokens: 50 }, { kind: "chat", tokens: 30 }, { kind: "files", tokens: 20 }, { kind: "system", tokens: 0 }])
  assert.equal(tiles.length, 3)
  const area = tiles.reduce((s, t) => s + t.w * t.h, 0)
  assert.ok(Math.abs(area - 1) < 1e-9)
  for (const t of tiles) {
    assert.ok(Math.abs(t.w * t.h - t.tokens / 100) < 1e-9, `${t.kind} 面积`)
    assert.ok(t.x >= -1e-9 && t.y >= -1e-9 && t.x + t.w <= 1 + 1e-9 && t.y + t.h <= 1 + 1e-9)
  }
  assert.deepEqual(treemap([{ kind: "chat", tokens: 0 }]), [])
})
