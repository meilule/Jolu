/**
 * 用量统计与会话线程的示例数据（静态快照）。
 * 数值与 `mock/usageStats.ts`、`mock/sessionThreads.ts`、画板 30 / 32 / 34 / 40 / 42 一致。
 */
import type { StatsActivityDto, StatsCallPageDto, StatsCallRowDto, StatsOverviewDto } from "@/lib/api"
import { NOW } from "./snapshotData"

const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

export { DAY }

const SERIES = [
  [420, 260, 520, 90], [510, 320, 640, 110], [180, 90, 220, 60], [260, 140, 320, 70],
  [610, 380, 700, 120], [890, 500, 850, 140], [760, 420, 790, 130], [540, 300, 610, 100],
]

function trendPoints(start: number, bucketMs: number) {
  return SERIES.map(([i, o, r, w], index) => ({
    start: start + index * bucketMs,
    end: start + (index + 1) * bucketMs,
    label: new Date(start + index * bucketMs).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false }),
    fresh_input: i! * 1000, output: o! * 1000, cache_read: r! * 1000, cache_write: w! * 1000,
    total: (i! + o! + r! + w!) * 1000,
    requests: Math.round((i! + o!) / 20),
    cost: (i! + o!) / 400,
  }))
}

export function demoStatsOverview(start: number): StatsOverviewDto {
  const d = new Date(start)
  const dayStart = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
  return {
    tokens: { fresh_input: 4_620_000, output: 2_240_000, cache_read: 4_940_000, cache_write: 680_000, total: 12_480_000 },
    token_coverage: { known_total: 12_480_000, known_requests: 846, cache_hit_rate: 0.482, cache_requests: 846 },
    requests: 846,
    failed: 12,
    cancelled: 3,
    unknown: 0,
    cache_hit_rate: 0.482,
    cost: { amount: 18.42, priced_requests: 810, requests: 846, unpriced_no_price: ["grok:grok-4-fast"], unpriced_no_usage: 12 },
    previous_total: 10_523_000,
    trend: { bucket: "2h", points: trendPoints(dayStart, 2 * HOUR) },
    models: [
      { platform: "claude", model: "claude-sonnet-4-5", requests: 400, tokens: 6_200_000, known_tokens: 6_200_000, known_requests: 400, cache_hit_rate: 0.521, avg_duration_ms: 1600, cost: 8.8, price_source: "https://platform.claude.com/docs/en/about-claude/pricing", price_checked: "2026-11-01" },
      { platform: "codex", model: "gpt-5", requests: 126, tokens: 2_160_000, known_tokens: 2_160_000, known_requests: 126, cache_hit_rate: 0.43, avg_duration_ms: 3200, cost: 4, price_source: "https://developers.openai.com/api/docs/pricing", price_checked: "2026-11-01" },
      { platform: "gemini", model: "gemini-2.5-flash", requests: 200, tokens: 2_120_000, known_tokens: 2_120_000, known_requests: 200, cache_hit_rate: 0.452, avg_duration_ms: 1400, cost: 2.72, price_source: "https://ai.google.dev/gemini-api/docs/pricing", price_checked: "2026-11-01" },
      { platform: "claude", model: "claude-haiku-4-5", requests: 80, tokens: 1_200_000, known_tokens: 1_200_000, known_requests: 80, cache_hit_rate: 0.4, avg_duration_ms: 900, cost: 1.8, price_source: null, price_checked: null },
      // 组内有取消记录、缺 Token：tokens=null 但 known_tokens 有值，走「已知」展示
      { platform: "codex", model: "gpt-5-mini", requests: 40, tokens: null, known_tokens: 640_000, known_requests: 36, cache_hit_rate: 0.385, avg_duration_ms: null, cost: null },
    ],
    providers: [
      { platform: "claude", requests: 420, tokens: 6_800_000, known_tokens: 6_800_000, known_requests: 420, cost: { amount: 10, priced_requests: 410, requests: 420 } },
      { platform: "codex", requests: 290, tokens: 3_760_000, known_tokens: 3_760_000, known_requests: 290, cost: { amount: 4, priced_requests: 270, requests: 290 } },
      { platform: "gemini", requests: 86, tokens: 1_120_000, known_tokens: 1_120_000, known_requests: 86, cost: { amount: 2.72, priced_requests: 86, requests: 86 } },
      { platform: "zcode", requests: 30, tokens: 480_000, known_tokens: 480_000, known_requests: 30, cost: { amount: 0, priced_requests: 0, requests: 30, unpriced_no_price: ["zcode:glm-4.7"] } },
      { platform: "opencode", requests: 20, tokens: 320_000, known_tokens: 320_000, known_requests: 20, cost: { amount: 1.1, priced_requests: 14, requests: 20, unpriced_partial: 6 } },
    ],
    attributed: 778,
    sessions: [
      { platform: "claude", session_id: "snap-claude-1", title: "排查缓存 Token 统计", tokens: 2_860_000, cwd: "D:/develop/jolu" },
      { platform: "codex", session_id: "snap-codex-1", title: "优化 Jolu 的会话界面", tokens: 1_920_000, cwd: "D:/develop/jolu" },
      { platform: "gemini", session_id: "snap-gemini-1", title: "梳理登录引导文案", tokens: 680_000, cwd: null },
      { platform: "codex", session_id: "snap-codex-2", title: "接入模型列表接口", tokens: 520_000, cwd: "D:/work/api-proxy" },
      { platform: "claude", session_id: "snap-claude-3", title: null, tokens: 410_000, cwd: null },
    ],
  }
}

export function demoStatsActivity(): StatsActivityDto {
  const last = new Date(NOW)
  const pad = (n: number) => String(n).padStart(2, "0")
  const key = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
  const days: StatsActivityDto["days"] = []
  for (let i = 0; i < 140; i++) {
    const date = new Date(last.getFullYear(), last.getMonth(), last.getDate() - i)
    const tokens = [0, 120_000, 480_000, 1_200_000, 2_600_000][(i * 7 + 3) % 5]!
    if (tokens) days.push({ date: key(date), tokens, requests: Math.round(tokens / 15_000) })
  }
  days.reverse()
  const yearAgo = new Date(last.getFullYear() - 1, last.getMonth(), last.getDate())
  return {
    first_date: key(yearAgo), last_date: key(last), days,
    today_active_minutes: 169, week_active_days: 5, month_active_days: 18,
  }
}

const DEMO_PRICES: Record<string, { input: number; cached: number; output: number; write: number | null; source: string }> = {
  "claude-sonnet-4-5": { input: 3, cached: 0.3, output: 15, write: null, source: "https://platform.claude.com/docs/en/about-claude/pricing" },
  "claude-haiku-4-5": { input: 1, cached: 0.1, output: 5, write: null, source: "https://platform.claude.com/docs/en/about-claude/pricing" },
  "gpt-5": { input: 1.25, cached: 0.125, output: 10, write: null, source: "https://developers.openai.com/api/docs/pricing" },
  "gpt-5-mini": { input: 0.25, cached: 0.025, output: 2, write: null, source: "https://developers.openai.com/api/docs/pricing" },
  "gemini-2.5-pro": { input: 1.25, cached: 0.125, output: 10, write: null, source: "https://ai.google.dev/gemini-api/docs/pricing" },
  "gemini-2.5-flash": { input: 0.3, cached: 0.03, output: 2.5, write: null, source: "https://ai.google.dev/gemini-api/docs/pricing" },
}

function demoPrice(model: string, longContext = false) {
  const p = DEMO_PRICES[model]
  return p ? { input: p.input, cached_input: p.cached, output: p.output, cache_write: p.write, source: p.source, long_context: longContext, off_peak: false } : null
}

type CallSeed = Omit<StatsCallRowDto, "id" | "ts">

const CALL_SEEDS: CallSeed[] = [
  { call_id: "call_demo_0138", platform: "claude", model: "claude-sonnet-4-5", session_id: "snap-claude-1", fresh_input: 8_400, cache_read: 12_000, cache_write: 0, output: 1_200, total: 21_600, price: demoPrice("claude-sonnet-4-5"), cost: 0.026, effort: "high", duration_ms: 1600, first_token_ms: 400, status_code: null, failed: false, attribution: { connection_id: "conn-claude-momo", name: "Momo", confidence: "exact" } },
  { call_id: "call_demo_0137", platform: "codex", model: "gpt-5", session_id: "snap-codex-1", fresh_input: 12_000, cache_read: 18_000, cache_write: null, output: 4_200, total: 34_200, price: demoPrice("gpt-5"), cost: 0.082, effort: "high", duration_ms: 3200, first_token_ms: 800, status_code: 200, failed: false, attribution: { connection_id: "conn-codex-alex", name: "Alex", confidence: "exact" } },
  { call_id: "call_demo_0136", platform: "codex", model: "gpt-5-mini", session_id: "snap-codex-2", fresh_input: 3_000, cache_read: 4_800, cache_write: null, output: 600, total: 8_400, price: demoPrice("gpt-5-mini"), cost: 0.009, effort: null, duration_ms: 800, first_token_ms: 200, status_code: 200, failed: false },
  { call_id: "call_demo_0135", platform: "gemini", model: "gemini-2.5-pro", session_id: "snap-gemini-1", fresh_input: 5_200, cache_read: 6_400, cache_write: null, output: 1_200, total: 12_800, price: demoPrice("gemini-2.5-pro"), cost: null, effort: "medium", duration_ms: 2100, first_token_ms: 500, status_code: null, failed: false },
  { call_id: "call_demo_0134", platform: "grok", model: "grok-4", session_id: null, fresh_input: null, cache_read: null, cache_write: null, output: null, total: null, price: demoPrice("grok-4"), cost: null, effort: null, duration_ms: 400, first_token_ms: null, status_code: 429, failed: true },
  { call_id: "call_demo_0133", platform: "claude", model: "claude-haiku-4-5", session_id: "snap-claude-3", fresh_input: 6_000, cache_read: 9_200, cache_write: 0, output: 1_200, total: 16_400, price: demoPrice("claude-haiku-4-5"), cost: 0.02, effort: "medium", duration_ms: 1500, first_token_ms: 300, status_code: null, failed: false },
  { call_id: "call_demo_0132", platform: "claude", model: "claude-sonnet-4-5", session_id: "snap-claude-1", fresh_input: 7_500, cache_read: 9_600, cache_write: 0, output: 1_800, total: 18_900, price: demoPrice("claude-sonnet-4-5"), cost: 0.041, effort: "high", duration_ms: 2600, first_token_ms: 600, status_code: null, failed: false },
  { call_id: "call_demo_0131", platform: "codex", model: "gpt-5-mini", session_id: "snap-codex-2", fresh_input: 2_400, cache_read: 4_000, cache_write: null, output: 400, total: 6_800, price: demoPrice("gpt-5-mini"), cost: 0.008, effort: "low", duration_ms: 900, first_token_ms: 200, status_code: 200, failed: false },
  { call_id: "call_demo_0130", platform: "gemini", model: "gemini-2.5-flash", session_id: "snap-gemini-1", fresh_input: 4_000, cache_read: 5_400, cache_write: null, output: 800, total: 10_200, price: demoPrice("gemini-2.5-flash"), cost: 0.012, effort: null, duration_ms: 1100, first_token_ms: 300, status_code: null, failed: false },
  { call_id: "call_demo_0129", platform: "codex", model: "gpt-5", session_id: null, fresh_input: null, cache_read: null, cache_write: null, output: null, total: null, price: demoPrice("gpt-5"), cost: null, effort: null, duration_ms: 30_000, first_token_ms: null, status_code: 504, failed: true },
  // ZCode 原生 cancelled：演示「已取消」胶囊，不能画成失败
  { call_id: "call_demo_0128", platform: "zcode", model: "glm-4.7", session_id: "snap-zcode-1", fresh_input: 5_800, cache_read: 2_100, cache_write: 0, output: 0, total: 7_900, price: null, cost: null, effort: null, duration_ms: 6_400, first_token_ms: 900, status_code: null, failed: false, cancelled: true },
]

export function demoStatsCalls(page: number): StatsCallPageDto {
  return {
    rows: CALL_SEEDS.map((row, index) => ({ ...row, id: 1_000 - index, ts: NOW - index * 66_000 - (page - 1) * 660_000 })),
    total_count: 846, unpriced_count: 36, page, page_size: 10, page_count: 85,
    models: ["claude-haiku-4-5", "claude-sonnet-4-5", "gemini-2.5-flash", "gemini-2.5-pro", "gpt-5", "gpt-5-mini", "grok-4"],
  }
}

/* ─────────────────────────── 会话线程 ─────────────────────────── */

export function demoSessions() {
  const row = (platform: string, id: string, title: string | null, cwd: string | null, last: number, status: string, tokens: number[]) => ({
    platform, session_id: id, title, cwd, first_ts: last - 24 * MIN, last_ts: last, requests: 12,
    fresh_input: tokens[0]!, cache_read: tokens[1]!, cache_write: tokens[2]!, output: tokens[3]!,
    connection_id: null, status, has_source: status !== "usage_only",
  })
  return {
    sessions: [
      row("claude", "snap-claude-1", "排查缓存 Token 统计", "D:/develop/jolu", NOW - 3 * MIN, "running", [9_000, 21_000, 1_400, 4_100]),
      row("codex", "snap-codex-1", "优化 Jolu 的会话界面", "D:/develop/jolu", NOW - 22 * MIN, "running", [12_400, 8_600, 0, 5_200]),
      row("claude", "snap-claude-2", "整理 Skills 管理方案", "D:/develop/jolu", NOW - 3 * HOUR, "done", [3_200, 7_400, 600, 1_900]),
      row("codex", "snap-codex-2", "接入模型列表接口", "D:/work/api-proxy", NOW - 8 * HOUR, "done", [6_000, 4_000, 0, 2_000]),
      row("gemini", "snap-gemini-1", "梳理登录引导文案", "D:/develop/jolu-docs", NOW - 11 * HOUR, "recent", [2_400, 0, 0, 900]),
      row("claude", "snap-claude-3", "缓存命中率统计口径", "D:/develop/jolu", NOW - 26 * HOUR, "done", [12_400, 8_600, 1_400, 5_200]),
      row("codex", "snap-codex-3", "调整供应商卡片布局", "D:/develop/jolu", NOW - 2 * DAY, "done", [4_100, 2_200, 0, 1_300]),
      row("claude", "snap-claude-4", "会话缓存数据补全", "D:/develop/jolu-docs", NOW - 2 * DAY - 3 * HOUR, "usage_only", [8_800, 3_200, 400, 2_100]),
      row("codex", "snap-codex-4", "补充安装步骤", "D:/develop/jolu-docs", NOW - 3 * DAY, "done", [1_800, 900, 0, 700]),
      row("claude", "snap-claude-5", null, null, NOW - 5 * DAY, "done", [700, 300, 0, 200]),
    ],
    files: 9,
  }
}

export function demoMessages(platform: string, sessionId: string) {
  const usageOnly = sessionId === "snap-claude-4" || platform === "gemini"
  const base = NOW - HOUR
  return {
    available: !usageOnly,
    total: usageOnly ? 0 : 24,
    offset: usageOnly ? 0 : 22,
    messages: usageOnly ? [] : [
      { role: "user" as const, ts: base, text: "把会话列表和详情放在同一页，切换会话时不要丢失筛选条件。", tools: [] },
      {
        role: "assistant" as const, ts: base + 6 * MIN,
        text: "已整理为左右分栏：左侧保留筛选和会话列表，右侧展示当前会话。切换会话时保留列表位置。\n\n• 按供应商、项目和时间快速定位\n• 账号无法确认归属时单独标注\n• 从详情返回原工具继续会话",
        tools: [{ name: "查看文件", summary: "SessionList.tsx", output: "已读取 128 行\n输出按原记录展示\n长内容可展开或复制" }],
      },
      { role: "user" as const, ts: base + 12 * MIN, text: "压缩后的会话要能导出成 Markdown 交给下一个工具。", tools: [] },
      {
        role: "assistant" as const, ts: base + 18 * MIN,
        text: "压缩流程分三步：生成交接摘要、就地修订、确认导出。\n\n1. 摘要包含目标、已完成、下一步与关键文件\n2. 可以先在对话里让模型补充遗漏\n3. 导出前预览 Markdown，确认再写入文件",
        tools: [
          { name: "写入文件", summary: "docs/handoff.md", output: "写入 42 行" },
          { name: "运行命令", summary: "node --test frontend/src/lib/*.test.mjs", output: "通过 128 项" },
        ],
      },
    ],
    models: [platform === "codex" ? "gpt-5" : "claude-sonnet-4-5"],
    cost: 0.42, priced_requests: 12, requests: 12, note: null,
  }
}

export const DEMO_CONTINUE_CHECK = { supported: true, tool_found: true, cwd: "D:/develop/jolu", cwd_accessible: true }
