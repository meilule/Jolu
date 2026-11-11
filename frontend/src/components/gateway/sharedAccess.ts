/**
 * 局域网共享与命名客户端 Key 的展示逻辑（纯函数，供 `SharedAccessDialog` 与 node 测试共用）。
 *
 * 后端语义（`backend/src/gateway/access.rs`）：
 * - `requests_per_day` 到量即拒（429，进 provider 之前）；`tokens_per_day` 由 `gateway/budget.rs`
 * 预扣 + 回冲强制：请求前按估算预扣，响应结束按实际用量记账，超量 429、次日零点重置，
 *   界面上必须如实标注，不能让用户以为 Token 上限也在生效
 * - `today_requests` 只数「当日 result = ok」的流水，被拒与上游失败不计
 * - `last_used_ms` 为 null 表示从未使用
 */

import type { ClientKeySummaryDto, KeyBudgetDto } from "@/lib/gatewayApi"

/** 每日上限的展示文案：未设置时写「不限」，而不是 0。 */
export function limitText(value: number | null | undefined): string {
  return typeof value === "number" && value > 0 ? value.toLocaleString("zh-CN") : "不限"
}

/** 今日用量的展示：有限额时带进度感（`12 / 100`），无限额只给数字。 */
export function usageText(row: Pick<ClientKeySummaryDto, "today_requests" | "requests_per_day">): string {
  const used = row.today_requests ?? 0
  const cap = row.requests_per_day
  return typeof cap === "number" && cap > 0 ? `${used} / ${cap}` : String(used)
}

/** 是否已用满当日请求上限（满额时行内要给提示，因为后续请求会被 429）。 */
export function isExhausted(row: Pick<ClientKeySummaryDto, "today_requests" | "requests_per_day">): boolean {
  const cap = row.requests_per_day
  return typeof cap === "number" && cap > 0 && (row.today_requests ?? 0) >= cap
}

/** 最近使用的相对时间；从未使用给明确文案，不显示 `1970`。 */
export function lastUsedText(lastUsedMs: number | null | undefined, now: number): string {
  if (!lastUsedMs) return "从未使用"
  const diff = Math.max(0, now - lastUsedMs)
  const minute = 60_000
  const hour = 60 * minute
  const day = 24 * hour
  if (diff < minute) return "刚刚"
  if (diff < hour) return `${Math.floor(diff / minute)} 分钟前`
  if (diff < day) return `${Math.floor(diff / hour)} 小时前`
  if (diff < 30 * day) return `${Math.floor(diff / day)} 天前`
  return new Date(lastUsedMs).toLocaleDateString("zh-CN")
}

/**
 * 共享开关的危险态：已开启共享但没有任何客户端 Key。
 *
 * 这时局域网设备无法通过严格校验（共享下 relaxed 被关掉），用户会以为「开了就能用」。
 * 界面必须高亮提示先签发 Key。
 */
export function sharingWithoutKeys(shared: boolean, keyCount: number): boolean {
  return shared && keyCount === 0
}

/** 签发结果页的警示文案：Key 原文只显示一次，这是后端的行为，不能承诺「以后还能看」。 */
export const KEY_ONCE_NOTICE = "这个 Key 只显示这一次，关闭后无法再查看；请现在复制并保存到客户端。"

/** Token 上限的口径说明：请求前按估算预扣，可能比实际用量更早拒绝。 */
export const TOKENS_LIMIT_NOTICE = "每日 Token 上限按「输入 + 输出」计，请求前先按估算预扣、结束后按实际用量回冲，临近上限时可能提前拒绝；两项上限都在次日零点重置。"

/* ───── 签发约束：有效期、模型白名单、只读（后端 `issue_client_key` 的可选参数） ───── */

const DAY_MS = 24 * 60 * 60 * 1000

/** 有效期选项：首项永不过期；后端要求到期时间在将来，所以只给正天数。 */
export const EXPIRY_CHOICES: readonly { days: number | null; label: string }[] = [
  { days: null, label: "永不过期" },
  { days: 1, label: "1 天" },
  { days: 7, label: "7 天" },
  { days: 30, label: "30 天" },
  { days: 90, label: "90 天" },
]

/** 有效期 → 到期时间（毫秒）；null = 永不过期。 */
export function expiresAtFrom(days: number | null, now: number): number | null {
  return days === null ? null : now + days * DAY_MS
}

/**
 * 模型白名单输入 → 列表：逗号、空白或换行分隔，去空去重；什么都没填返回 null（= 不限）。
 * 后端同样把「全空白」当未设，这里提前归一，免得用户以为设了一个空白名单。
 */
export function parseModelList(raw: string): string[] | null {
  const models = [...new Set(raw.split(/[\s,，]+/).map((part) => part.trim()).filter(Boolean))]
  return models.length > 0 ? models : null
}

/** 是否已过期（到点即过期，与后端鉴权一致）；null 永不过期。 */
export function isExpired(row: Pick<ClientKeySummaryDto, "expires_ms">, now: number): boolean {
  return typeof row.expires_ms === "number" && row.expires_ms <= now
}

/** 列表行的约束标签：只列出确实设置了的约束，过期标红。 */
export function constraintTags(
  row: Pick<ClientKeySummaryDto, "expires_ms" | "models" | "read_only">,
  now: number,
): { text: string; danger: boolean }[] {
  const tags: { text: string; danger: boolean }[] = []
  if (typeof row.expires_ms === "number") {
    if (isExpired(row, now)) tags.push({ text: "已过期", danger: true })
    else {
      const days = Math.floor((row.expires_ms - now) / DAY_MS)
      tags.push({ text: days < 1 ? "今天过期" : `${days} 天后过期`, danger: false })
    }
  }
  if (row.models && row.models.length > 0) tags.push({ text: `限 ${row.models.length} 个模型`, danger: false })
  if (row.read_only) tags.push({ text: "只读", danger: false })
  return tags
}

/** 同时请求数的常用选项（magpie 同款）；0 表示不限 */
export const CONCURRENCY_CHOICES = [0, 1, 2, 3, 5, 10] as const

/** 下拉选项：常用值，加上当前已设但不在常用里的值（比如经 CLI 设的 7） */
export function concurrencyOptions(current: number | null | undefined): { value: number; label: string }[] {
  const values: number[] = [...CONCURRENCY_CHOICES]
  if (typeof current === "number" && current > 0 && !values.includes(current)) values.push(current)
  return values.sort((a, b) => a - b).map((value) => ({ value, label: value === 0 ? "不限" : `${value} 个` }))
}

/** 按 Key 出账：今日 / 本周 / 本月的 Token 与费用。没有数据（旧版后端）时为空。 */
export function spendLines(row: Pick<ClientKeySummaryDto, "spend">): { label: string; tokens: string; cost: string }[] {
  const labels = ["今日", "本周", "本月"]
  return (row.spend ?? []).slice(0, 3).map(([tokens, cost], index) => ({
    label: labels[index],
    tokens: compactTokens(tokens),
    cost: cost > 0 ? `$${cost < 0.01 ? cost.toFixed(4) : cost.toFixed(2)}` : "$0",
  }))
}

function compactTokens(n: number): string {
  const trim = (text: string) => (text.endsWith(".0") ? text.slice(0, -2) : text)
  if (n >= 1_000_000) return `${trim((n / 1_000_000).toFixed(1))}M`
  if (n >= 1_000) return `${trim((n / 1_000).toFixed(1))}K`
  return String(n)
}

/** 上限编辑表单：六个数字框（空串 = 不限）+ 账号范围（勾选的连接 id） */
export const BUDGET_FIELDS: readonly { key: keyof KeyBudgetDto; label: string; money: boolean }[] = [
  { key: "tokens_per_day", label: "每日 Token", money: false },
  { key: "tokens_per_week", label: "每周 Token", money: false },
  { key: "tokens_per_month", label: "每月 Token", money: false },
  { key: "cost_per_day", label: "每日费用（$）", money: true },
  { key: "cost_per_week", label: "每周费用（$）", money: true },
  { key: "cost_per_month", label: "每月费用（$）", money: true },
]

export type BudgetForm = Record<keyof KeyBudgetDto, string>

export function budgetForm(budget: KeyBudgetDto | undefined): BudgetForm {
  const out = {} as BudgetForm
  for (const field of BUDGET_FIELDS) {
    const value = budget?.[field.key]
    out[field.key] = typeof value === "number" ? String(value) : ""
  }
  return out
}

/** 表单 → 提交值；有非法值时返回错误文案 */
export function budgetFromForm(form: BudgetForm): { budget: KeyBudgetDto; error?: undefined } | { error: string; budget?: undefined } {
  const budget: KeyBudgetDto = {}
  for (const field of BUDGET_FIELDS) {
    const text = form[field.key].trim()
    if (!text) { budget[field.key] = null; continue }
    const value = Number(text)
    if (!Number.isFinite(value) || value <= 0) return { error: `${field.label}只能留空或填正数` }
    if (!field.money && !Number.isInteger(value)) return { error: `${field.label}只能填整数` }
    budget[field.key] = value
  }
  return { budget }
}

/** 列表里的上限说明：只列设了的（每日 Token 已在「每日上限」那一栏的口径之外，单独列出） */
export function budgetTags(row: Pick<ClientKeySummaryDto, "budget" | "accounts">): string[] {
  const tags: string[] = []
  for (const field of BUDGET_FIELDS) {
    const value = row.budget?.[field.key]
    if (typeof value === "number") tags.push(field.money ? `${field.label.replace("（$）", "")} $${value}` : `${field.label} ${compactTokens(value)}`)
  }
  if (row.accounts && row.accounts.length > 0) tags.push(`限 ${row.accounts.length} 个账号`)
  return tags
}

