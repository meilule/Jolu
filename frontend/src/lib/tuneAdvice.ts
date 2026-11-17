/**
 * 5.10 成本调优建议（后端 `tune.rs`，移植 magpie tune）的类型与文案。无画板，界面待确认。
 */

export interface TunePointDto { value: number; cost: number; compacts: number }

export interface TuneFactsDto {
  calls: number
  streams: number
  compacts: number
  quick: number
  pause: number
  away: number
}

export interface TuneCompactionDto {
  model: string
  current: number
  /** 0 = 模型窗口未知 */
  window: number
  best: number
  cost: number
  low: number
  curve: TunePointDto[]
  facts: TuneFactsDto
}

export interface TuneTtlDto {
  current: string
  current_inferred: boolean
  best: string
  cost: number
  low: number
  facts: TuneFactsDto
}

export interface TuneAgentDto {
  platform: "claude" | "codex" | string
  calls: number
  compact_setting: number | null
  compactions: TuneCompactionDto[]
  cache: TuneTtlDto | null
}

export const kTokens = (n: number) => `${Math.round(n / 1000)}K`

export function savedShare(cost: number, low: number): string {
  if (!(cost > 0)) return "0%"
  return `${Math.round(((cost - low) / cost) * 100)}%`
}

export function agentName(platform: string): string {
  return platform === "claude" ? "Claude Code" : platform === "codex" ? "Codex" : platform
}

/** 一条压缩窗口建议的文案 */
export function compactionLine(c: TuneCompactionDto, setting: number | null): { title: string; detail: string; change: boolean } {
  const now = setting == null ? `现在约 ${kTokens(c.current)}（没设，按压缩点推断）` : `现在 ${kTokens(c.current)}`
  const facts = `${c.facts.calls} 次调用 · ${c.facts.streams} 个会话 · 压缩 ${c.facts.compacts} 次`
  if (c.best === c.current) return { title: `${c.model}：${now}，保持即可`, detail: facts, change: false }
  const unknown = c.window > 0 ? "" : " · 模型窗口未知，建议值可能超过它，改前先核对"
  return { title: `${c.model}：${now} → 建议 ${kTokens(c.best)}，加权 Token 省 ${savedShare(c.cost, c.low)}`, detail: facts + unknown, change: true }
}

/** 提示词缓存建议的文案 */
export function ttlLine(t: TuneTtlDto): { title: string; detail: string; change: boolean } {
  const inferred = t.current_inferred ? "（按 5 分钟推断：记录里没有 1 小时写入的拆分）" : ""
  const gaps = `调用间隔 5 分钟内 ${Math.round(t.facts.quick * 100)}% · 5 分钟到 1 小时 ${Math.round(t.facts.pause * 100)}% · 超过 1 小时 ${Math.round(t.facts.away * 100)}%`
  if (t.best === t.current) return { title: `提示词缓存：${t.current}${inferred}，保持即可`, detail: gaps, change: false }
  return { title: `提示词缓存：${t.current}${inferred} → 建议 ${t.best}，加权 Token 省 ${savedShare(t.cost, t.low)}`, detail: gaps, change: true }
}
