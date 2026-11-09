/**
 * 局域网共享与命名客户端 Key 的展示逻辑（纯函数，供 `SharedAccessDialog` 与 node 测试共用）。
 *
 * 后端语义（`backend/src/gateway/access.rs`）：
 * - `requests_per_day` 到量即拒（429，进 provider 之前）；`tokens_per_day` **只存不强制**，
 *   界面上必须如实标注，不能让用户以为 Token 上限也在生效
 * - `today_requests` 只数「当日 result = ok」的流水，被拒与上游失败不计
 * - `last_used_ms` 为 null 表示从未使用
 */

import type { ClientKeySummaryDto } from "@/lib/gatewayApi"

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

/** Token 上限的如实标注（后端只记录不强制，不能让用户误以为生效）。 */
export const TOKENS_NOT_ENFORCED_NOTICE = "每日 Token 上限本期只记录，不做强制拦截；请求数上限会立即生效。"
