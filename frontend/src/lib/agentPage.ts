/**
 * Agent 页纯逻辑（Jolu.pen「统一网关 · R2 重设计」V5-1 / V5-2）。
 *
 * 左栏按「已接入 / 可以接入 / 暂时不能接入」分组；右侧详情的摘要、档位提示与
 * 「接入会做什么」都从这里取，便于 node --test 直接验证。
 */
import type { AgentDiscoveryDto, CustomAgentDto, WiringStatusDto } from "./gatewayApi.ts"

/** 自定义工具（没有官方 logo）的首字母小方块；Jolu 支持的工具用官方 logo，见 `AgentBadge` */
export function agentBadge(_agent: string, name: string): { bg: string; glyph: string } {
  return { bg: "#615CED", glyph: (name.trim()[0] ?? "?").toUpperCase() }
}

/** 会话、Skills、自动化里用的平台 id */
export const AGENT_PLATFORM: Readonly<Record<string, string>> = {
  "claude-code": "claude",
  codex: "codex",
  gemini: "gemini",
  antigravity: "antigravity",
  opencode: "opencode",
  zcode: "zcode",
  kimi: "kimi",
  // dsh 的平台 id 就是 "dsh"：后端 `dsh_sessions.rs` 产出的 platform 是 "dsh"，
  // 用量采集与 Skills 也都按这个 id 走。曾经这里写 "deepseek"，导致详情页的
  // 会话数与技能数永远取不到（`platformCatalog.json` 里也没有 "deepseek" 这个 id）。
  dsh: "dsh",
  pi: "pi",
  pencil: "pencil",
  qoder: "qoder",
  "qoder-cn": "qoder-cn",
  qwen: "qwen",
  workbuddy: "workbuddy",
}

export type AgentGroup = "wired" | "available" | "blocked"

export interface AgentRow {
  agent: string
  name: string
  group: AgentGroup
  /** Jolu 有这条工具的接线实现（可开关接入）；false = 只探测到、暂不支持（画板 V8「暂不支持」卡） */
  supported: boolean
  /** 列表第二行：已接入时是主模型 / 重开提示，未接入时是当前来源说明 */
  subtitle: string
  /**
   * 卡片底部的状态胶囊文案（R1-08）。
   *
   * 与 `subtitle` 分开：已接入时 `subtitle` 放的是主模型（`zcode/glm-5.3`），
   * 那是一个值，不是状态；拿它当状态胶囊会显示成「zcode/glm-5.3」。
   */
  status: string
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

export function agentRow(agent: string, wiring: WiringStatusDto | null, discovery: AgentDiscoveryDto | null, model: string | null | undefined, supported: boolean): AgentRow {
  const name = discovery?.name || wiringName(agent)
  const drift = driftText(wiring?.drift)
  const managed = Boolean(wiring && (wiring.wired || (wiring.state && wiring.state !== "idle" && wiring.drift)))
  if (managed && wiring) {
    const restart = !drift && !model && Boolean(wiring.notice?.includes("重启"))
    // 画板 V5-1：「重开后生效」与主模型被换过用琥珀色，配置被改动用红色
    const tone = wiring.drift === "model_changed" || restart ? "warn" : wiring.drift ? "danger" : "normal"
    const subtitle = drift ?? (model || (restart ? "重开后生效" : "已接入"))
    const status = drift ?? (restart ? "重开后生效" : "已接入 · 配置一致")
    return { agent, name, group: "wired", supported, subtitle, status, tone, toggle: wiring.wired ? "on" : null, wiring, discovery }
  }
  if (discovery && discovery.state === "not_found") {
    return { agent, name, group: "blocked", supported, subtitle: "本机没有检测到", status: "暂不能接入", tone: "normal", toggle: null, wiring, discovery }
  }
  // 对齐 magpie：配置在就算已装、可接入——Jolu 的接线只写配置文件，不调用 CLI。
  // CLI 不在 PATH 只作提示（OpenCode / Kimi / Continue 用 bun 或独立安装器装，命令不在
  // PATH 是常态）；真卸载的残留已由后端 agent_dir_has_content 过滤，不会走到这里。
  // 没有专用接线的仍标「暂不支持」，不给点了会失败的开关。
  if (discovery && discovery.state === "cli_missing") {
    return { agent, name, group: "available", supported, subtitle: "检测到配置 · CLI 未在 PATH（不影响接入）", status: "可接入", tone: "warn", toggle: supported ? "off" : null, wiring, discovery }
  }
  if (wiring?.reason) {
    return { agent, name, group: "blocked", supported, subtitle: wiring.reason, status: "暂不能接入", tone: "normal", toggle: null, wiring, discovery }
  }
  // V8：探测到但 Jolu 没有专用接线的（Qoder / Trae 等 8 家）＝「暂不支持」卡——
  // 在栅格里、开关禁用、详细可点（走通用接入），不算「暂不能接入」（那是没装 / 残留）。
  const subtitle = supported ? AVAILABLE_SUBTITLES[agent] ?? "可以接入" : "已检测到 · 暂无专用接入"
  return { agent, name, group: "available", supported, subtitle, status: supported ? "可接入" : "暂不支持", tone: "normal", toggle: supported ? "off" : null, wiring, discovery }
}

/** 后端状态按注册表返回可接入工具，发现结果补充尚未支持的工具。 */
export function buildAgentRows(wiring: WiringStatusDto[], discovery: AgentDiscoveryDto[], model: string | null): AgentRow[] {
  const supported = new Set(wiring.map((entry) => entry.agent))
  const rows = wiring.map((entry) => agentRow(
    entry.agent, entry, discovery.find((item) => item.agent === entry.agent) ?? null,
    entry.agent === "claude-code" ? model : null, true,
  ))
  for (const entry of discovery) {
    if (!supported.has(entry.agent)) rows.push(agentRow(entry.agent, null, entry, null, false))
  }
  return rows
}

/** 可接入时的第二行：说明它现在用的是什么（画板 V5-1「用它自己的 …」） */
const AVAILABLE_SUBTITLES: Readonly<Record<string, string>> = {
  "claude-code": "用它自己的登录",
  codex: "用它自己的登录",
  gemini: "用它自己的登录",
  antigravity: "用它自己的登录",
  opencode: "用它自己的 API Key",
  zcode: "用 ZCode 自己的账号",
  kimi: "用它自己的账号",
  dsh: "用它自己的配置",
  pi: "用它自己的 provider",
  pencil: "用它自己的模型",
  qoder: "用 Qoder 自己的账号",
  "qoder-cn": "用 Qoder 自己的账号",
  qwen: "用它自己的登录",
  workbuddy: "用它自己的模型",
}

function wiringName(agent: string): string {
  return ({ "claude-code": "Claude Code", antigravity: "Antigravity CLI", codex: "Codex", gemini: "Gemini CLI", opencode: "OpenCode", zcode: "ZCode", kimi: "Kimi CLI",
    pencil: "Pencil", dsh: "DeepSeek Harness", copilot: "Copilot CLI", vscode: "VS Code",
    agy: "Antigravity CLI", atomcode: "AtomCode", ante: "Ante", jcode: "jcode", minimax: "MiniMax Code",
    empryo: "Empryo", commandcode: "Command Code", muse: "Muse", mothx: "Mothx", devin: "Devin",
    codebuddy: "CodeBuddy Code", grok: "Grok Build", hanako: "OpenHanako", snow: "Snow CLI", astrbot: "AstrBot",
    morph: "Mister Morph", openchamber: "OpenChamber", "grok-app": "Grok App", qoder: "Qoder", "qoder-cn": "Qoder CN",
    pi: "Pi", workbuddy: "WorkBuddy", hermes: "Hermes Agent", fx: "fx", stepcode: "StepCode", newmax: "NewMax",
    raycast: "Raycast", penguin: "Penguin", omo: "Omo", prime: "Prime Agent", air: "JetBrains Air", reasonix: "Reasonix",
    openpencil: "OpenPencil", alma: "Alma", cindy: "Cindy", aionui: "AionUi", "vscode-insiders": "VS Code Insiders", vscodium: "VSCodium" } as Record<string, string>)[agent] ?? agent
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

/** V8 接入状态筛选（画板 V8-01）：全部 / 已接入 / 可接入；暂不能接入固定在折叠区，不参与栅格。 */
export type AgentFilter = "all" | "wired" | "available"

export const AGENT_FILTERS: readonly { id: AgentFilter; label: string }[] = [
  { id: "all", label: "全部" },
  { id: "wired", label: "已接入" },
  { id: "available", label: "可接入" },
]

/** 栅格里的工具：应用筛选 + 搜索 + 隐藏；顺序先已接入再可接入（画板示例的排列）。 */
export function agentGridRows(rows: AgentRow[], filter: AgentFilter, search: string, hidden: readonly string[], showHidden: boolean): { rows: AgentRow[]; blocked: AgentRow[]; hiddenCount: number } {
  const query = search.trim().toLowerCase()
  const matched = rows.filter((row) => !query || row.name.toLowerCase().includes(query) || row.agent.includes(query))
  const visible = showHidden ? matched : matched.filter((row) => !hidden.includes(row.agent))
  const inScope = visible.filter((row) => row.group !== "blocked")
  const byFilter = filter === "all" ? inScope : inScope.filter((row) => row.group === filter)
  const order = (["wired", "available"] as const)
  return {
    rows: order.flatMap((group) => byFilter.filter((row) => row.group === group)),
    blocked: visible.filter((row) => row.group === "blocked"),
    hiddenCount: matched.filter((row) => hidden.includes(row.agent)).length,
  }
}

/** 通用接入的自定义工具在列表里的样子（P1-1） */
export function customAgentRow(agent: CustomAgentDto): AgentRow {
  const base = { agent: agent.id, name: agent.name, supported: true, wiring: null, discovery: null }
  if (agent.wired || agent.drift) {
    const drift = agent.drift === "removed" ? "配置文件被删除，未经路由" : agent.drift ? "配置被改动，未经路由" : null
    return { ...base, group: "wired", subtitle: drift ?? (agent.model || "已接入 · 自定义"), status: drift ?? "已接入 · 配置一致", tone: drift ? "danger" : "normal", toggle: agent.wired ? "on" : null }
  }
  if (!agent.installed) return { ...base, group: "blocked", subtitle: "配置所在的目录不存在", status: "暂不能接入", tone: "normal", toggle: null }
  return { ...base, group: "available", subtitle: `自定义 · ${agent.format.toUpperCase()}`, status: "可接入", tone: "normal", toggle: "off" }
}

export function isCustomAgent(id: string): boolean {
  return id.startsWith("custom-agent:")
}

/** 添加自定义工具时每种格式的键名示例 */
export const CUSTOM_EXAMPLES: Readonly<Record<"json" | "toml" | "env", { path: string; base: string; auth: string; model: string; suffix: string }>> = {
  json: { path: "~/.config/tool/settings.json", base: "provider.baseUrl", auth: "provider.apiKey", model: "provider.model", suffix: "/v1" },
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
  // 有专用接线的工具不再给预设：Kimi / Crush（5.11）、Pencil / Qoder / Qoder CN / qwen / workbuddy。
  // Droid（customModels 是数组）与 Zed（密钥在系统凭据库）按 magpie 核对后通用引擎写不了，已删掉，
  // 免得写出工具不认的配置。
  { id: "goose", name: "Goose", format: "env", configPath: "~/.config/goose/.env", baseKey: "OPENAI_BASE_URL", authKey: "OPENAI_API_KEY", modelKey: "OPENAI_MODEL", suffix: "/v1", verified: false },
  { id: "continue", name: "Continue", format: "json", configPath: "~/.continue/config.json", baseKey: "models.0.apiBase", authKey: "models.0.apiKey", modelKey: "models.0.model", suffix: "/v1", verified: false },
]

/** 探测到但还没有专门接线的工具：给用户指到「添加自定义工具」并预填预设 */
export function customPresetFor(agent: string): CustomAgentPreset | null {
  return CUSTOM_AGENT_PRESETS.find((preset) => preset.id === agent) ?? null
}

/**
 * 这家工具能不能在「活跃会话」里留下记录。
 *
 * 为什么要单独声明而不是「没数据就不显示」：`活跃会话` 只列真的读到会话的平台，
 * 用户接了一家工具却发现那里永远没有它，会以为是坏了。把「不支持」明确说出来，
 * 比让它静默消失诚实（AGENTS.md：能力不支持要明确表达）。
 *
 * 三档：
 * - `observable`：有会话标识或用本地会话文件读到了（Claude Code / Codex / OpenCode /
 *   ZCode / Kimi / Pi / dsh / Qoder）。默认就是这档，不需要列出。
 * - `no-record`：**这个工具本身不产生会话记录**。Pencil 只有模型配置文件，没有会话存储。
 * - `not-yet`：理论上能读，但 Jolu 还没接（Qwen / WorkBuddy）。
 *
 * 判据来自实测（2026-11-22）：Pencil 的 `~/.pencil` 只有 `models.json`；
 * Qwen 与 WorkBuddy 本机未安装、无样本。
 */
export type SessionRecordCapability = "observable" | "no-record" | "not-yet"

const SESSION_RECORD: Readonly<Record<string, SessionRecordCapability>> = {
  pencil: "no-record",
  qwen: "not-yet",
  workbuddy: "not-yet",
}

/** 查一家工具的会话记录能力；没列出的都是 `observable`。 */
export function sessionRecordCapability(agent: string): SessionRecordCapability {
  return SESSION_RECORD[agent] ?? "observable"
}

/** 详情页那句说明；`observable` 返回 null（不占位置）。 */
export function sessionRecordNote(agent: string): string | null {
  switch (sessionRecordCapability(agent)) {
    case "no-record":
      return "这个工具只保存模型配置，不产生会话记录，所以它不会出现在模型路由的活跃会话里。"
    case "not-yet":
      return "它自己的会话记录格式还没接入 Jolu，所以暂时不会出现在活跃会话里；这不影响模型路由转发。"
    default:
      return null
  }
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
    case "kimi":
      return [
        { icon: "file", text: "在 Kimi 的 config.toml 里新增 provider「jolu」与一组「jolu/」开头的模型表，默认模型指向网关" },
        { icon: "shield", text: "你原有的 provider 与模型表原样保留；旧版 kimi-cli 会少写它不认识的键" },
        undo,
      ]
    case "pi":
      return [
        { icon: "file", text: "在 Pi 的 models.json 里新增 provider「jolu」（网关模型目录生成模型清单），并把 settings.json 的默认 provider / model 指向它" },
        { icon: "shield", text: "你原有的 provider（如华龙）与其它设置原样保留" },
        undo,
      ]
    case "dsh":
      return [
        { icon: "file", text: "在 DeepSeek Harness 各 profile 的 cordis.patch.yml 里写路由「jolu」（网关模型目录生成模型清单）并把默认模型指向它，密钥写入 .env 与凭据引用" },
        { icon: "shield", text: "你原有的供应商（如 hualong）与其它条目一行不动；你在 dsh 里改过的线协议会保留" },
        undo,
      ]
    case "pencil":
      return [
        { icon: "file", text: "在 ~/.pencil/models.json 里新增 provider「jolu」，模型池里的模型都进 Pencil 的模型选择器" },
        { icon: "shield", text: "你原有的 provider 原样保留；Pencil 启动时读这个文件，接入后要重启它" },
        undo,
      ]
    case "qoder":
    case "qoder-cn":
      return [
        { icon: "file", text: "在 Qoder 的 settings.json 里新增 provider「jolu」，并把当前模型切到「jolu/」开头的网关模型" },
        { icon: "shield", text: "原来选的模型存进快照，断开时切回；Qoder 账号与它自己的密钥不动" },
        undo,
      ]
    case "crush":
      return [
        { icon: "file", text: "在 crush.json 里新增 provider「jolu」，大小两个模型选择都指向网关模型" },
        { icon: "shield", text: "原来的大小模型选择存进快照，断开时放回；你在 Crush 里改选了别的就不动" },
        undo,
      ]
    case "qwen":
      return [
        { icon: "file", text: "在 ~/.qwen/settings.json 的 modelProviders.openai 里追加网关模型，并把默认模型指向它" },
        { icon: "shield", text: "你原有的 provider 与默认模型原样保留（你的条目留在前面）；密钥单独放在 env 里" },
        undo,
      ]
    case "antigravity":
      return [
        { icon: "file", text: "在 ~/.gemini/antigravity-cli/settings.json 里写入 jolu/ 自定义模型并选中，同时在同目录生成带网关地址与令牌的 jolu-agy.cmd 启动脚本" },
        { icon: "shield", text: "你原有的自定义模型、modelProvider 与工作区信任等配置先存快照，原样保留；用 jolu-agy.cmd 启动即可经模型路由转发" },
        { icon: "undo", text: "断开时还原 model / modelProvider / customModelsConfig 并删除 jolu-agy.cmd；接入后你自己改过的值不会被还原" },
      ]
    case "workbuddy":
      return [
        { icon: "file", text: "在 ~/.workbuddy/models.json 里追加网关模型（裸数组与 {models:[…]} 两种形状都支持）" },
        { icon: "shield", text: "你原有的模型、你在 WorkBuddy 里关掉的开关、CodeBuddy 的展示列表都原样保留" },
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
