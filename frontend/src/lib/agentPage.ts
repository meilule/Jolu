/**
 * Agent 页纯逻辑（Jolu.pen「统一网关 · R2 重设计」V5-1 / V5-2）。
 *
 * 左栏按「已接入 / 可以接入 / 暂时不能接入」分组；右侧详情的摘要、档位提示与
 * 「接入会做什么」都从这里取，便于 node --test 直接验证。
 */
import type { AgentDiscoveryDto, CustomAgentDto, WiringStatusDto } from "./gatewayApi.ts"

/** Jolu 能接线的工具（与后端 `wiring::AGENTS` 一致） */
export const WIRABLE_AGENTS = ["claude-code", "codex", "gemini", "opencode", "zcode"] as const

/** 自定义工具（没有官方 logo）的首字母小方块；Jolu 支持的工具用官方 logo，见 `AgentBadge` */
export function agentBadge(_agent: string, name: string): { bg: string; glyph: string } {
  return { bg: "#615CED", glyph: (name.trim()[0] ?? "?").toUpperCase() }
}

/** 会话、Skills、自动化里用的平台 id */
export const AGENT_PLATFORM: Readonly<Record<string, string>> = {
  "claude-code": "claude",
  codex: "codex",
  gemini: "gemini",
  opencode: "opencode",
  zcode: "zcode",
}

export type AgentGroup = "wired" | "available" | "blocked"

export interface AgentRow {
  agent: string
  name: string
  group: AgentGroup
  /** 列表第二行 */
  subtitle: string
  /** 第二行语气：normal / warn（琥珀）/ danger */
  tone: "normal" | "warn" | "danger"
  /** 开关：已接入为开、可接入为关，暂时不能接入时不显示 */
  toggle: "on" | "off" | null
  wiring: WiringStatusDto | null
  discovery: AgentDiscoveryDto | null
}

/** 漂移说明（列表第二行与详情头部共用）。 */
export function driftText(drift: WiringStatusDto["drift"]): string | null {
  switch (drift) {
    case "changed": return "配置被改动，未经路由"
    case "removed": return "配置文件被删除，未经路由"
    case "foreign": return "被另一个 Jolu 接管"
    case "pending": return "上次接入没有完成"
    case "unreachable": return "模型路由已停止，请求会失败"
    case "model_changed": return "主模型已在工具里换过"
    default: return null
  }
}

/**
 * 一个工具在列表里的样子。
 * - 有接线记录且在接管中（wired，或接线后被外部改动）→ 已接入；
 * - 本机检测到（或旧版后端没有检测结果）且有可写的配置 → 可以接入；
 * - CLI 未找到 / 没检测到 / 配置文件还没生成 → 暂时不能接入。
 */
/** 没有 CLI 的桌面客户端：`cli_missing` 不代表卸载，配置在就能接入 */
const DESKTOP_CLIENTS: ReadonlySet<string> = new Set(["zcode", "zed"])

export function agentRow(agent: string, wiring: WiringStatusDto | null, discovery: AgentDiscoveryDto | null, model?: string | null): AgentRow {
  const name = discovery?.name || wiringName(agent)
  const drift = driftText(wiring?.drift)
  const managed = Boolean(wiring && (wiring.wired || (wiring.state && wiring.state !== "idle" && wiring.drift)))
  if (managed && wiring) {
    const restart = !drift && !model && Boolean(wiring.notice?.includes("重启"))
    // 画板 V5-1：「重开后生效」与主模型被换过用琥珀色，配置被改动用红色
    const tone = wiring.drift === "model_changed" || restart ? "warn" : wiring.drift ? "danger" : "normal"
    const subtitle = drift ?? (model || (restart ? "重开后生效" : "已接入"))
    return { agent, name, group: "wired", subtitle, tone, toggle: wiring.wired ? "on" : null, wiring, discovery }
  }
  if (discovery && discovery.state === "not_found") {
    return { agent, name, group: "blocked", subtitle: "本机没有检测到", tone: "normal", toggle: null, wiring, discovery }
  }
  // CLI 不在 PATH：区分两种情况（这条容易一刀切错）
  // - 桌面客户端（ZCode）：本来就没有 CLI，配置在就能接入 → 留在「可以接入」
  // - 真 CLI 工具：CLI 不在说明已经卸载、只剩配置残留 → 归「暂时不能接入」，
  //   否则列表里会堆一堆用户没装的工具，分不清哪些真能用
  if (discovery && discovery.state === "cli_missing" && !DESKTOP_CLIENTS.has(agent)) {
    return { agent, name, group: "blocked", subtitle: "检测到配置 · CLI 未找到", tone: "normal", toggle: null, wiring, discovery }
  }
  if (wiring?.reason) {
    return { agent, name, group: "blocked", subtitle: wiring.reason, tone: "normal", toggle: null, wiring, discovery }
  }
  const subtitle = AVAILABLE_SUBTITLES[agent] ?? "可以接入"
  return { agent, name, group: "available", subtitle, tone: "normal", toggle: "off", wiring, discovery }
}

/** 可接入时的第二行：说明它现在用的是什么（画板 V5-1「用它自己的 …」） */
const AVAILABLE_SUBTITLES: Readonly<Record<string, string>> = {
  "claude-code": "用它自己的登录",
  codex: "用它自己的登录",
  gemini: "用它自己的登录",
  opencode: "用它自己的 API Key",
  zcode: "用 ZCode 自己的账号",
}

function wiringName(agent: string): string {
  return ({ "claude-code": "Claude Code", codex: "Codex", gemini: "Gemini CLI", opencode: "OpenCode", zcode: "ZCode" } as Record<string, string>)[agent] ?? agent
}

export const GROUP_LABELS: Record<AgentGroup, string> = { wired: "已接入", available: "可以接入", blocked: "暂时不能接入" }

/** 列表：按组排好，组内按 Jolu 支持的顺序；搜索按名称与 id 不分大小写；隐藏的单独计数。 */
export function agentRows(rows: AgentRow[], search: string, hidden: readonly string[], showHidden: boolean): { groups: { group: AgentGroup; rows: AgentRow[] }[]; hiddenCount: number } {
  const query = search.trim().toLowerCase()
  const matched = rows.filter((row) => !query || row.name.toLowerCase().includes(query) || row.agent.includes(query))
  const visible = showHidden ? matched : matched.filter((row) => !hidden.includes(row.agent))
  const hiddenCount = matched.filter((row) => hidden.includes(row.agent)).length
  const order = (["wired", "available", "blocked"] as const)
  return {
    groups: order.map((group) => ({ group, rows: visible.filter((row) => row.group === group) })).filter((entry) => entry.rows.length > 0),
    hiddenCount,
  }
}

/** 通用接入的自定义工具在列表里的样子（P1-1） */
export function customAgentRow(agent: CustomAgentDto): AgentRow {
  const base = { agent: agent.id, name: agent.name, wiring: null, discovery: null }
  if (agent.wired || agent.drift) {
    const drift = agent.drift === "removed" ? "配置文件被删除，未经路由" : agent.drift ? "配置被改动，未经路由" : null
    return { ...base, group: "wired", subtitle: drift ?? (agent.model || "已接入 · 自定义"), tone: drift ? "danger" : "normal", toggle: agent.wired ? "on" : null }
  }
  if (!agent.installed) return { ...base, group: "blocked", subtitle: "配置所在的目录不存在", tone: "normal", toggle: null }
  return { ...base, group: "available", subtitle: `自定义 · ${agent.format.toUpperCase()}`, tone: "normal", toggle: "off" }
}

export function isCustomAgent(id: string): boolean {
  return id.startsWith("custom-agent:")
}

/** 添加自定义工具时每种格式的键名示例 */
export const CUSTOM_EXAMPLES: Readonly<Record<"json" | "toml" | "env", { path: string; base: string; auth: string; model: string; suffix: string }>> = {
  json: { path: "~/.config/crush/crush.json", base: "providers.jolu.base_url", auth: "providers.jolu.api_key", model: "models.large.model", suffix: "/v1" },
  toml: { path: "~/.config/tool/config.toml", base: "provider.base_url", auth: "provider.api_key", model: "model", suffix: "/v1" },
  env: { path: "~/.tool/.env", base: "OPENAI_BASE_URL", auth: "OPENAI_API_KEY", model: "OPENAI_MODEL", suffix: "/v1" },
}

/**
 * 通用接入的工具预设（扩展 agent 广度用）：路径与键名取自各工具官方配置格式。
 * 用户在「添加自定义工具」里选一个，就能自动填好格式与三个键；不用自己查文档。
 * 只收录键名可从工具文档确认的；不确定的留空由用户填（AGENTS.md：不猜）。
 */
export interface CustomAgentPreset {
  /** 与 agent_discovery 的 SPECS id 一致，用于和探测结果对上 */
  id: string
  name: string
  format: "json" | "toml" | "env"
  configPath: string
  baseKey: string
  authKey: string
  modelKey: string
  /** 地址要不要带 /v1 这类后缀 */
  suffix: string
  /**
   * 键名是否已有代码或文档依据。
   * false = 按工具常见格式推测，界面必须提示用户核对后再接入（AGENTS.md：未知值不冒充）。
   */
  verified: boolean
}

export const CUSTOM_AGENT_PRESETS: readonly CustomAgentPreset[] = [
  { id: "qwen", name: "Qwen Code", format: "json", configPath: "~/.qwen/settings.json", baseKey: "env.OPENAI_BASE_URL", authKey: "env.OPENAI_API_KEY", modelKey: "env.OPENAI_MODEL", suffix: "/v1", verified: false },
  { id: "kimi", name: "Kimi CLI", format: "toml", configPath: "~/.kimi/config.toml", baseKey: "provider.base_url", authKey: "provider.api_key", modelKey: "provider.model", suffix: "/v1", verified: false },
  { id: "droid", name: "Droid", format: "json", configPath: "~/.factory/settings.json", baseKey: "customModels[0].baseUrl", authKey: "customModels[0].apiKey", modelKey: "customModels[0].model", suffix: "/v1", verified: false },
  { id: "zed", name: "Zed", format: "json", configPath: "~/.config/zed/settings.json", baseKey: "language_models.openai.api_url", authKey: "language_models.openai.api_key", modelKey: "language_models.openai.available_models", suffix: "/v1", verified: false },
  { id: "crush", name: "Crush", format: "json", configPath: "~/.config/crush/crush.json", baseKey: "providers.jolu.base_url", authKey: "providers.jolu.api_key", modelKey: "models.large.model", suffix: "/v1", verified: true },
  { id: "goose", name: "Goose", format: "env", configPath: "~/.config/goose/.env", baseKey: "OPENAI_BASE_URL", authKey: "OPENAI_API_KEY", modelKey: "OPENAI_MODEL", suffix: "/v1", verified: false },
  { id: "continue", name: "Continue", format: "json", configPath: "~/.continue/config.json", baseKey: "models.0.apiBase", authKey: "models.0.apiKey", modelKey: "models.0.model", suffix: "/v1", verified: false },
]

/** 探测到但还没有专门接线的工具：给用户指到「添加自定义工具」并预填预设 */
export function customPresetFor(agent: string): CustomAgentPreset | null {
  return CUSTOM_AGENT_PRESETS.find((preset) => preset.id === agent) ?? null
}

/** 「接入会做什么」三条说明（画板 V5-2）：写哪里、原有的怎么保留、断开时怎么还原。 */
export function wireExplanation(agent: string): { icon: "file" | "shield" | "undo"; text: string }[] {
  const undo = { icon: "undo" as const, text: "断开时只删除 Jolu 写入的键；接入后你自己改过的值不会被还原" }
  switch (agent) {
    case "claude-code":
      return [
        { icon: "file", text: "在 ~/.claude/settings.json 的 env 里写入模型路由地址与令牌，并写入主模型与各档位" },
        { icon: "shield", text: "你原有的设置先存快照；选了「保留 claude.ai 登录」时不会把你登出" },
        undo,
      ]
    case "codex":
      return [
        { icon: "file", text: "在 ~/.codex/config.toml 里把请求地址指向本机模型路由" },
        { icon: "shield", text: "已登录 ChatGPT 时保留登录，只改请求地址；否则新增一个 Jolu provider" },
        undo,
      ]
    case "gemini":
      return [
        { icon: "file", text: "在 ~/.gemini/.env 写入模型路由地址与 Key，并把登录方式临时切到 API Key" },
        { icon: "shield", text: "原来的登录方式存进快照，断开时切回" },
        undo,
      ]
    case "opencode":
      return [
        { icon: "file", text: "在 opencode.json 里新增一个 provider「jolu」，指向本机模型路由" },
        { icon: "shield", text: "你原有的 provider 和 API Key 原样保留，不会被覆盖" },
        undo,
      ]
    case "zcode":
      return [
        { icon: "file", text: "在 ZCode 的模型配置里新增一个指向本机模型路由的 provider" },
        { icon: "shield", text: "你原有的 provider 原样保留；新版与旧版 ZCode 的两份配置都会写" },
        undo,
      ]
    default:
      return [undo]
  }
}

/** 档位行最右侧的提示（F1，画板 V5-1）：在 Claude Code 里怎么做。 */
export const TIER_HINTS: Readonly<Record<string, { text: string; warn?: boolean }>> = {
  main: { text: "/model" },
  opus: { text: "/model opus" },
  sonnet: { text: "/model sonnet · 默认档" },
  haiku: { text: "也跑标题、Explore 等后台小任务", warn: true },
  fable: { text: "/model fable" },
}

/** 长对话设置的三态（B5）：默认 272K、完整窗口、自定义。 */
export type LongContextMode = "default" | "full" | "custom"

export function longContextMode(value: string | null | undefined): LongContextMode {
  const text = (value ?? "").trim().toLowerCase()
  if (!text || text === "default") return "default"
  if (text === "full") return "full"
  return "custom"
}

export function longContextText(value: string | null | undefined): string {
  const mode = longContextMode(value)
  if (mode === "default") return "长对话 272K 压缩"
  if (mode === "full") return "长对话用满窗口"
  return `长对话 ${value!.trim()} 压缩`
}

/** 自定义大小：`272k` / `1M` / `400000` / `1.5m`，至少 1000 token。与后端 `escape::parse_tokens` 同一口径。 */
export function parseTokens(text: string): number | null {
  const clean = text.trim().toLowerCase().replace(/[,_\s]/g, "")
  const match = /^(\d+(?:\.\d+)?)([km]?)$/.exec(clean)
  if (!match) return null
  const unit = match[2] === "k" ? 1_000 : match[2] === "m" ? 1_000_000 : 1
  const tokens = Math.round(Number(match[1]) * unit)
  return tokens >= 1_000 ? tokens : null
}

/** 本地隐藏的工具（只影响列表，不改配置） */
export const HIDDEN_KEY = "jolu:agents:hidden"

export function readHidden(storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage): string[] {
  try {
    const raw = storage?.getItem(HIDDEN_KEY)
    const list = raw ? JSON.parse(raw) : []
    return Array.isArray(list) ? list.filter((item): item is string => typeof item === "string") : []
  } catch {
    return []
  }
}

export function writeHidden(list: string[], storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage): void {
  try {
    storage?.setItem(HIDDEN_KEY, JSON.stringify(list))
  } catch {
    /* 存不进去只影响下次打开 */
  }
}
