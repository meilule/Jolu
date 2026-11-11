/**
 * 联网搜索设置的纯逻辑：后端配置 ⇄ 界面草稿、保存前校验、搜索插件状态文案。
 *
 * Key 只以掩码到达前端（后端序列化默认掩码）。保存时把掩码原样交回，后端据此保留
 * 原 Key；新填的 Key 以原文提交一次，之后界面上也只见掩码。
 */

/** 后端 `SearchApiConfig`（Key 为掩码） */
export interface SearchApiDto {
  vendor: string
  keys: string[]
  url?: string | null
  off: boolean
}

/** 后端 `VendorInfo` */
export interface SearchVendorDto {
  id: string
  name: string
  key_optional: boolean
  needs_url: boolean
}

/** 后端 `SearchHit` */
export interface SearchHitDto {
  title: string
  url: string
  snippet: string
}

/** 界面草稿：Key 以多行文本编辑（每行或逗号分隔一把） */
export interface SearchDraft {
  vendor: string
  keysText: string
  url: string
  off: boolean
}

export function toDrafts(apis: readonly SearchApiDto[] | undefined): SearchDraft[] {
  return (apis ?? []).map((api) => ({ vendor: api.vendor, keysText: api.keys.join("\n"), url: api.url ?? "", off: api.off }))
}

export function splitKeys(text: string): string[] {
  const out: string[] = []
  for (const piece of text.split(/[\s,;]+/)) {
    if (piece && !out.includes(piece)) out.push(piece)
  }
  return out
}

/** 提交给 `search_apis_set` 的结构 */
export function toInputs(drafts: readonly SearchDraft[]): SearchApiDto[] {
  return drafts.map((draft) => ({ vendor: draft.vendor, keys: splitKeys(draft.keysText), url: draft.url.trim() || null, off: draft.off }))
}

/** 保存前的本地校验；后端还会再校验一次（掩码对不上等只有后端知道） */
export function validateDrafts(drafts: readonly SearchDraft[], vendors: readonly SearchVendorDto[]): string | null {
  for (const draft of drafts) {
    const vendor = vendors.find((item) => item.id === draft.vendor)
    if (!vendor) return `未知搜索后端「${draft.vendor}」`
    if (vendor.needs_url && !draft.url.trim()) return `${vendor.name} 需要填写实例地址`
    if (!vendor.key_optional && splitKeys(draft.keysText).length === 0) return `${vendor.name} 至少要填一把 Key`
    const url = draft.url.trim()
    if (url && !/^https?:\/\//i.test(url)) return `${vendor.name} 的地址要以 http:// 或 https:// 开头`
  }
  return null
}

/** 新增一行时默认选第一个还没用过的后端 */
export function nextVendor(drafts: readonly SearchDraft[], vendors: readonly SearchVendorDto[]): string {
  return (vendors.find((vendor) => !drafts.some((draft) => draft.vendor === vendor.id)) ?? vendors[0])?.id ?? "tavily"
}

/** 草稿与已保存的配置是否不同（决定「保存」按钮是否可点） */
export function draftsChanged(drafts: readonly SearchDraft[], saved: readonly SearchApiDto[] | undefined): boolean {
  return JSON.stringify(toInputs(drafts)) !== JSON.stringify(toInputs(toDrafts(saved)))
}

export function isMaskedKey(key: string): boolean {
  return key.includes("•")
}

/* ---------------------------- 搜索插件（MCP） ---------------------------- */

/** 后端 `LibraryAgentStatus` */
export interface LibraryAgentDto {
  agent: string
  installed: string[]
  config_present: boolean
  supported: boolean
  note: string | null
}

export const SEARCH_PLUGIN_NAME = "jolu-search"

const AGENT_LABELS: Record<string, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  gemini: "Gemini CLI",
  opencode: "OpenCode",
  zcode: "ZCode",
  qoder: "Qoder CLI",
  "qoder-cn": "Qoder CLI 国内版",
  workbuddy: "WorkBuddy",
  trae: "Trae",
}

export function agentLabel(agent: string): string {
  return AGENT_LABELS[agent] ?? agent
}

export type PluginState = "installed" | "absent" | "unsupported"

export function pluginState(row: LibraryAgentDto): PluginState {
  if (!row.supported) return "unsupported"
  return row.installed.includes(SEARCH_PLUGIN_NAME) ? "installed" : "absent"
}

export function pluginStateText(row: LibraryAgentDto): string {
  switch (pluginState(row)) {
    case "installed": return "已安装"
    case "unsupported": return "不支持"
    default: return row.config_present ? "未安装" : "未安装 · 配置文件尚未创建，安装时会新建"
  }
}
