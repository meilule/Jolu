import type { Connection } from "../types"

/** 两个平台共用连接规则；只有网关 API 同时提供额度和钱包查询。 */
export function connectionQueries(connection: Connection | null) {
  const none = { quota: null, balance: null, usage: null }
  if (!connection || connection.status === "paused") return none
  if (connection.kind === "auth") return { ...none, quota: connection.id }
  return connection.baseUrl
    ? { quota: connection.id, balance: connection.id, usage: null }
    : { ...none, usage: connection.id }
}

/**
 * 灵动岛生效连接（§7.3，2026-09-19 鼠鼠定版）：明确选择的连接优先；
 * **未选择**连接时，自动启用当前平台第一个连接成功的连接。
 * 仍不回退到本机账号或「唯一候选」；已选择的连接（含已暂停）不被悄悄顶掉，
 * 所选连接被删除时也照旧显示「所选连接不可用」。
 */
export function islandActiveConnection(
  configuredId: string | null | undefined,
  connections: readonly Connection[],
  platform: string,
): Connection | null {
  if (configuredId) {
    return connections.find((c) => c.id === configuredId) ?? null
  }
  return connections.find((c) => c.platformId === platform && c.status === "connected") ?? null
}

/**
 * 收缩态那一行「在跑的是哪条已存连接」（P0 连接归因）。
 *
 * 只有确定档才替换平台名：
 * - `exact`：本应用请求路径写入的连接；
 * - `mapped`：由 provider 端点地址或模型清单**唯一**推出（展示上弱化，与 exact 区分）。
 * `platform` 档、缺 id、缺名字一律返回 null —— 界面继续显示平台名，绝不显示
 * 「未命名连接」这种假归属。三档的语义由后端给出，前端不再二次推断。
 */
export function islandAttribution(
  attribution: { connection_id: string | null; name: string | null; confidence: "exact" | "mapped" | "platform" } | null | undefined,
): { id: string; name: string; confidence: "exact" | "mapped" } | null {
  if (!attribution || attribution.confidence === "platform") return null
  const id = attribution.connection_id?.trim()
  const name = attribution.name?.trim()
  if (!id || !name) return null
  return { id, name, confidence: attribution.confidence }
}

/** 复用真实连接状态；未知选择不能伪装成绿色成功。 */
export function islandConnectionStatus(
  status: Connection["status"] | null,
  localAuth: boolean,
  hasSavedSelection: boolean,
): { tone: "success" | "warn" | "danger"; label: string } {
  if (status === "paused") return { tone: "warn", label: "已断开" }
  if (status === "invalid") return { tone: "danger", label: "凭证无效" }
  if (status === "offline") return { tone: "warn", label: "离线" }
  if (status === "expired") return { tone: "warn", label: "凭证已过期" }
  if (status === "connected" || localAuth) return { tone: "success", label: "已连接" }
  return { tone: "warn", label: hasSavedSelection ? "所选连接不可用" : "未选择连接" }
}

function validCost(value: number | null | undefined): value is number {
  return value != null && Number.isFinite(value) && value >= 0
}

/** 实际账单优先；微小非零值与零、未知值保持区分。 */
export function islandCost(
  actual: number | null | undefined,
  estimate: { amount: number; complete: boolean } | null | undefined,
): { cost: string | null; costEstimated: boolean } {
  const estimated = !validCost(actual) && !!estimate?.complete && validCost(estimate.amount)
  const amount = validCost(actual) ? actual : estimated ? estimate!.amount : null
  if (amount === null) return { cost: null, costEstimated: false }
  const formatted = amount > 0 && amount < 0.0001
    ? "<$0.0001"
    : `$${amount.toFixed(amount > 0 && amount < 0.01 ? 4 : 2)}`
  return { cost: `${estimated ? "≈ " : ""}${formatted}`, costEstimated: estimated }
}

/**
 * 余额是查询瞬间的快照，不是实时值：网关侧每笔请求都在实时扣减，
 * 与 sub2api 面板比对时先对获取时刻再对数值，因此展示时刻本身。
 */
export function balanceSnapshotTime(fetchedAt: Date | null | undefined): string | null {
  if (!fetchedAt) return null
  return `· ${fetchedAt.toLocaleTimeString("zh-CN", { hour12: false })} 获取`
}

/**
 * 4.2「今日按来源」：最多 5 行（按 Token 降序，后端已排好），连接已删的写「已删除的连接」；
 * 归属说明只在有未归属记录时出现（「n/m 条已归属」），全部归属时不打扰。
 * 一条已归属的记录都没有时返回 null，整段不显示。
 */
export function sourceBreakdownView(
  breakdown: { rows: { connection_id: string; name: string | null; requests: number; tokens: number | null }[]; total_requests: number; unattributed_requests: number } | null,
  formatTokens: (value: number) => string,
): { rows: { id: string; name: string; value: string }[]; more: number; coverage: string | null } | null {
  if (!breakdown || breakdown.rows.length === 0) return null
  const rows = breakdown.rows.slice(0, 5).map((row) => ({
    id: row.connection_id,
    name: row.name?.trim() || "已删除的连接",
    value: row.tokens === null ? `${row.requests} 次 · Token 未知` : `${formatTokens(row.tokens)} Token`,
  }))
  const attributed = breakdown.total_requests - breakdown.unattributed_requests
  return {
    rows,
    more: Math.max(0, breakdown.rows.length - rows.length),
    coverage: breakdown.unattributed_requests > 0 ? `${attributed}/${breakdown.total_requests} 条已归属` : null,
  }
}

/**
 * 4.2「请求进入时额度环切到被调用来源」：网关正在转发（或刚转发完、结果还在显示）的请求
 * 用的是另一条有额度可查的订阅连接时，返回那条连接，岛的额度环临时显示它的额度。
 *
 * 不切换的情况：本岛是分身（分身本来就绑定了一条连接）、请求没有连接 id、就是本岛的连接、
 * 连接不在列表里或已暂停、不是订阅连接（API Key 连接没有额度环）。
 */
export function relayQuotaConnection(
  relay: { connectionId?: string | null; phase: string } | null,
  activeId: string | null | undefined,
  connections: Connection[],
  isClone: boolean,
): Connection | null {
  if (isClone || !relay?.connectionId || relay.phase === "rejected") return null
  if (relay.connectionId === activeId) return null
  const target = connections.find((item) => item.id === relay.connectionId)
  if (!target || target.status === "paused" || target.kind !== "auth") return null
  return target
}

/**
 * 从灵动岛跳到用量页时带哪条连接筛选：只有后端按请求事实**确定**归到这条连接（`exact`）时才带。
 *
 * 灵动岛的今日数字是整个平台的；以前跳转总带「本岛选中的连接」，而直接用 Claude Code、
 * 不经网关的记录没有连接 id，用量页一筛就是 0 次调用（P4 审计 D7）。`mapped` / `platform`
 * 档只预筛平台，不猜连接。
 */
export function usageJumpConnection(attribution: { connection_id: string | null; confidence: string } | null | undefined): string | null {
  return attribution?.confidence === "exact" ? attribution.connection_id : null
}

/**
 * P1-4「未知不是零」在灵动岛上的落点：今日 Token 的三态。
 *
 * 后端 `today_tokens` 是 `Option<i64>`，`None` 有两种完全不同的含义：
 * - **今天没有任何记录**（`known = 0 && unknown = 0`）→ 沿用 `today_tokens` 自己的判断
 *   （有值即真零，null 即未知，因为原生来源可能只是还没扫到）；
 * - **有记录但都没给 Token**（`known = 0 && unknown > 0`）→ 未知，写 `—`。
 *
 * 单看 `today_tokens` 分不出这两者（都是 null），所以后端另外返回
 * `today_token_coverage`（共享契约 `data_contract::Coverage`，与用量页
 * `token_coverage`、会话卡片 `SessionTokens` 同一套规则）。这里把它翻成展示值。
 *
 * 三态之外还有「部分覆盖」：有值也有缺口，主数字是已知小计，`note` 说明它覆盖了多少条，
 * 避免被读成整日总量——与用量页 `providerTokenCell` 的口径一致。
 *
 * 老后端不返回该字段时退化：有数按 known，没数按 missing（保守，不把「拿不到」写成 0）。
 */
export function todayTokenView(args: {
  todayTokens: number | null
  /** 后端 `today_token_coverage`；老后端缺失时为 undefined */
  coverage?: { total?: number | null; known: number; unknown: number } | null
  /** 数值格式化（岛里传 `compactTokens`，测试可传原值） */
  format: (value: number) => string
  /** 未知态占位符，默认 `—` */
  dash?: string
}): { text: string; note: string | null; state: "known" | "partial" | "missing" | "empty" } {
  const { todayTokens, coverage, format } = args
  const dash = args.dash ?? "—"
  // 总量与覆盖分开查询时两边都可能是唯一来源（覆盖里的 `total` 是后端用同一口径补的），
  // 取先有值的那个；都为 null 才是真「拿不到数」。
  const sum = coverage?.total ?? todayTokens
  if (!coverage) {
    return todayTokens != null
      ? { text: format(todayTokens), note: null, state: "known" }
      : { text: dash, note: null, state: "missing" }
  }
  if (coverage.known === 0 && coverage.unknown === 0) {
    // 今天没有记录。这**不等于**真实的零：原生来源（zcode 等）是先扫库再落表，
    // 还没扫到也是 0 行，所以后端对它们返回 null。沿用后端的判断——
    // 它给了数值就是真零（claude / codex 的采集器知道自己扫过），是 null 就不冒充零。
    return sum != null
      ? { text: format(sum), note: null, state: "empty" }
      : { text: dash, note: null, state: "missing" }
  }
  if (coverage.known === 0) return { text: dash, note: `${coverage.unknown} 条记录没有 Token`, state: "missing" }
  const text = format(sum ?? 0)
  if (coverage.unknown > 0) {
    return { text, note: `${coverage.known} / ${coverage.known + coverage.unknown} 条有 Token`, state: "partial" }
  }
  return { text, note: null, state: "known" }
}
