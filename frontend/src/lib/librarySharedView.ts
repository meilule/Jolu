/**
 * Library 5.1 第二批：共享指令与子代理（无画板，界面待确认）的纯逻辑。
 * 后端 `library/shared.rs`：Jolu 写下的每一处都记哈希，改过的算用户的、不再动。
 */

export interface TargetStatusDto {
  agent: string
  path: string | null
  /** Jolu 写过 */
  written: boolean
  /** 写过，但现在的内容和写的不一样（用户改过）或已被删掉 */
  changed: boolean
}

export interface SubagentDto {
  name: string
  description: string
  prompt: string
  model: string | null
  agents: string[]
}

export interface LibrarySharedStateDto {
  instructions: { text: string; agents: string[] }
  instruction_targets: TargetStatusDto[]
  subagents: (SubagentDto & { targets: TargetStatusDto[] })[]
  subagent_agents: string[]
}

export interface FileOutcomeDto {
  agent: string
  path: string | null
  action: "written" | "removed" | "unchanged" | "skipped"
  reason: string | null
}

export const SHARED_AGENT_LABELS: Readonly<Record<string, string>> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  gemini: "Gemini CLI",
  opencode: "OpenCode",
  qwen: "Qwen Code",
  droid: "Droid",
}

export function sharedAgentLabel(agent: string): string {
  return SHARED_AGENT_LABELS[agent] ?? agent
}

/** 一处目标的状态文字与色调 */
export function targetText(target: TargetStatusDto): { text: string; tone: "ok" | "warn" | "muted" } {
  if (!target.written) return { text: "未写入", tone: "muted" }
  if (target.changed) return { text: "已被改动，Jolu 不再动它", tone: "warn" }
  return { text: "已写入", tone: "ok" }
}

/** 保存结果汇总成一句提示；有跳过的单独列出原因 */
export function outcomeSummary(outcomes: FileOutcomeDto[]): { text: string; warn: boolean } {
  if (outcomes.length === 0) return { text: "没有需要改动的工具", warn: false }
  const count = (action: FileOutcomeDto["action"]) => outcomes.filter((o) => o.action === action).length
  const parts: string[] = []
  if (count("written")) parts.push(`写入 ${count("written")} 处`)
  if (count("removed")) parts.push(`移除 ${count("removed")} 处`)
  if (count("unchanged")) parts.push(`${count("unchanged")} 处无变化`)
  const skipped = outcomes.filter((o) => o.action === "skipped")
  if (skipped.length) {
    parts.push(`跳过 ${skipped.length} 处：${skipped.map((o) => `${sharedAgentLabel(o.agent)}（${o.reason ?? "原因未知"}）`).join("；")}`)
  }
  return { text: parts.join("，"), warn: skipped.length > 0 }
}

/** 子代理名称：前端先查一遍，后端还会再查（规则同 magpie：2–50 个小写字母、数字或 -） */
export function subagentNameError(name: string): string | null {
  if (!/^[a-z0-9][a-z0-9-]{0,48}[a-z0-9]$/.test(name)) return "名称用 2–50 个小写字母、数字或 -，不能以 - 开头或结尾"
  if (["self", "system", "user", "model", "tool", "config", "default", "main"].includes(name)) return `「${name}」是工具自己保留的名字`
  return null
}
