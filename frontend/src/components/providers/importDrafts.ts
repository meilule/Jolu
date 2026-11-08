/**
 * 从本机导入（P4.3）的纯逻辑：扫描草稿按来源分组、选择集合与完成文案。
 *
 * 后端 `providers_import_scan` 的 drafts 里，单来源失败以 `{source, error}` 条目出现，
 * 不拖垮其它来源；键不出后端，草稿只带掩码，落库由 `providers_import_apply` 重读原文。
 */

export interface ImportDraft {
  /** 稳定引用：`<source>:<条目键>`，apply 原样回传给后端 */
  reference: string
  source: string
  /** 来源里的位置（如 settings.json 的 env、config.toml） */
  from: string
  name: string
  baseUrl: string
  protocol: "openai" | "anthropic"
  keyMasked: string
}

/** 单来源读取失败的错误条目（不可勾选） */
export interface ImportSourceError {
  source: string
  error: string
}

export type ImportScanItem = ImportDraft | ImportSourceError

/** 错误条目没有 reference；草稿一定有 */
export function isImportSourceError(item: ImportScanItem): item is ImportSourceError {
  return !("reference" in item)
}

export const SOURCE_LABELS: Record<string, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  "cc-switch": "CC Switch",
}

export function sourceLabel(source: string): string {
  return SOURCE_LABELS[source] ?? source
}

export const PROTOCOL_LABELS: Record<string, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
}

export function protocolLabel(protocol: string): string {
  return PROTOCOL_LABELS[protocol] ?? protocol
}

export interface ImportGroup {
  source: string
  label: string
  drafts: ImportDraft[]
  errors: string[]
}

/** 按来源分组，保持扫描返回顺序；错误条目并入对应来源 */
export function groupImportDrafts(items: readonly ImportScanItem[]): ImportGroup[] {
  const groups = new Map<string, ImportGroup>()
  for (const item of items) {
    const group = groups.get(item.source) ?? { source: item.source, label: sourceLabel(item.source), drafts: [], errors: [] }
    if (isImportSourceError(item)) group.errors.push(item.error)
    else group.drafts.push(item)
    groups.set(item.source, group)
  }
  return [...groups.values()]
}

/** 可勾选的引用：全部草稿；错误条目不参与 */
export function selectableReferences(groups: readonly ImportGroup[]): string[] {
  return groups.flatMap((group) => group.drafts.map((draft) => draft.reference))
}

export function toggleReference(selected: ReadonlySet<string>, reference: string): Set<string> {
  const next = new Set(selected)
  if (next.has(reference)) next.delete(reference)
  else next.add(reference)
  return next
}

/** 后端返回的创建结果带 id（「名称（id）」）；提示里只显示名称 */
export function connectionDisplayName(created: string): string {
  return created.replace(/（[^（）]*）$/, "")
}

export function importToastTitle(created: readonly string[]): string {
  return `已导入 ${created.length} 个连接`
}

export function importToastDetail(created: readonly string[]): string | undefined {
  const names = created.map(connectionDisplayName)
  return names.length > 0 ? names.join("、") : undefined
}

/** 空状态引导：各来源的配置文件位置；Jolu 只读原文，不改写这些文件 */
export const SOURCE_PATHS: readonly { label: string; path: string }[] = [
  { label: "CC Switch", path: "~/.cc-switch/cc-switch.db（旧版为 config.json）" },
  { label: "Claude Code", path: "~/.claude/settings.json 的 env" },
  { label: "Codex", path: "~/.codex/config.toml 与 auth.json" },
]
