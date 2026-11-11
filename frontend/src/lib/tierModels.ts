/**
 * Claude Code 档位自选（批 4）的纯逻辑：档位值的拆分与拼接、各模型可选的 effort、
 * 草稿 ⇄ 后端 `TierModels`。
 *
 * effort 表与后端 `gateway::wiring_store::tier_efforts` 一致（按 Claude Code 实际对各模型
 * 发送的范围）；后端保存前还会再校验一次，这里只用来决定界面上给哪些选项。
 */
import type { TierModels } from "./gatewayApi"

export const TIERS = ["opus", "sonnet", "haiku", "fable"] as const
export type Tier = (typeof TIERS)[number]

export const TIER_LABELS: Record<Tier, string> = { opus: "Opus 档", sonnet: "Sonnet 档", haiku: "Haiku 档", fable: "Fable 档" }

export const CLAUDE_EFFORTS = ["low", "medium", "high", "xhigh", "max"] as const
const EFFORT_WORDS = ["none", "minimal", "low", "medium", "high", "xhigh", "max"]

export const EFFORT_LABELS: Record<string, string> = { low: "低", medium: "中", high: "高", xhigh: "很高", max: "最高" }

/** `来源/模型[1m]:effort` → 模型与 effort；冒号后不是 effort 词的属于模型名（`qwen3:32b`） */
export function parseTierValue(value: string): { model: string; effort: string | null } {
  const at = value.lastIndexOf(":")
  if (at > 0) {
    const effort = value.slice(at + 1).trim().toLowerCase()
    if (EFFORT_WORDS.includes(effort)) return { model: value.slice(0, at), effort }
  }
  return { model: value, effort: null }
}

export function composeTierValue(model: string, effort: string | null): string {
  return effort ? `${model}:${effort}` : model
}

/** 某个模型在 Claude Code 里可设的 effort；空数组 = 不收 effort */
export function tierEfforts(model: string): readonly string[] {
  const id = (model.split("/").pop() ?? model).replace(/\[1m\]$/i, "").toLowerCase().replace(/\./g, "-")
  const at = id.indexOf("claude-")
  if (at < 0) return CLAUDE_EFFORTS
  const rest = id.slice(at + "claude-".length)
  if (/^(3|2|instant)/.test(rest)) return []
  const [family = "", major = "", minorRaw = ""] = rest.split("-")
  const minor = /^\d{1,2}$/.test(minorRaw) ? minorRaw : ""
  const name = minor ? `${family}-${major}-${minor}` : `${family}-${major}`
  if (name === "opus-4-5") return ["low", "medium", "high"]
  if (name === "opus-4-6" || name === "sonnet-4-6") return ["low", "medium", "high", "max"]
  if (["haiku-4-5", "sonnet-4-5", "opus-4", "sonnet-4", "opus-4-1", "opus-4-0", "sonnet-4-0"].includes(name)) return []
  return CLAUDE_EFFORTS
}

/** Auto 判别档的三种选法：交给 Claude Code 自己挑、跟随子代理、指定模型 */
export type AutoChoice = { kind: "default" } | { kind: "subagent" } | { kind: "model"; model: string }

export interface TierDraft {
  /** 每档：空串 = 跟随主模型 */
  models: Record<Tier, string>
  efforts: Record<Tier, string | null>
  auto: AutoChoice
}

export function emptyTierDraft(): TierDraft {
  return {
    models: { opus: "", sonnet: "", haiku: "", fable: "" },
    efforts: { opus: null, sonnet: null, haiku: null, fable: null },
    auto: { kind: "default" },
  }
}

/** 后端存的绝对值 → 草稿。Auto 档的值等于当时的子代理值时视为「跟随子代理」。 */
export function tierDraftFrom(tiers: TierModels | null | undefined, subagent: string | null | undefined): TierDraft {
  const draft = emptyTierDraft()
  for (const tier of TIERS) {
    const value = tiers?.[tier]
    if (!value) continue
    const { model, effort } = parseTierValue(value)
    draft.models[tier] = model
    draft.efforts[tier] = effort
  }
  const auto = tiers?.autoMode
  draft.auto = !auto ? { kind: "default" } : subagent && auto === subagent ? { kind: "subagent" } : { kind: "model", model: auto }
  return draft
}

/**
 * 草稿 → 后端 `TierModels`。「跟随子代理」由前端填入子代理的值（后端不知道这个语义）；
 * 子代理本身跟随主模型时填主模型。全部跟随时返回 null（后端清掉自选与 Auto 档）。
 */
export function tierModelsFrom(draft: TierDraft, mainModel: string, subagent: string): TierModels | null {
  const out: TierModels = {}
  for (const tier of TIERS) {
    const model = draft.models[tier]
    if (!model) continue
    const allowed = tierEfforts(model)
    const effort = draft.efforts[tier]
    out[tier] = composeTierValue(model, effort && allowed.includes(effort) ? effort : null)
  }
  if (draft.auto.kind === "subagent") out.autoMode = subagent || mainModel || null
  if (draft.auto.kind === "model") out.autoMode = draft.auto.model
  return Object.values(out).some(Boolean) ? out : null
}

/** 档位行的说明：跟随时写清跟随的是谁，自选时写实际值 */
export function tierSummary(draft: TierDraft, tier: Tier, mainModel: string): string {
  const model = draft.models[tier]
  if (!model) return `跟随主模型 · ${mainModel || "未选"}`
  const effort = draft.efforts[tier]
  return `自选 · ${composeTierValue(model, effort && tierEfforts(model).includes(effort) ? effort : null)}`
}
