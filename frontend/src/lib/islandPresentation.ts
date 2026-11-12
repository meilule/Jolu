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
