/**
 * 隐私脱敏（P5）的纯逻辑：规则清单与开关的乐观显示值。
 * 引擎与规则都在后端 gateway/mask.rs，这里只承载界面如实展示的内容，不做规则扩展。
 */

/** 开关显示值：点了还没保存成功时显示目标态，保存失败回滚到后端保存值 */
export function maskingDisplay(saved: boolean, applied: boolean | null): boolean {
  return applied ?? saved
}

export interface MaskingRule {
  name: string
  hint: string
}

/** 脱敏范围（固定规则，只读展示；后端是唯一事实来源，界面不提供增删入口） */
export const PRIVACY_MASKING_RULES: readonly MaskingRule[] = [
  { name: "API Key", hint: "sk- 等已知前缀的密钥" },
  { name: "私钥 PEM 段", hint: "-----BEGIN … PRIVATE KEY-----" },
  { name: "Bearer 令牌", hint: "Bearer / Basic 后的令牌" },
  { name: "token= 凭证", hint: "token=、api_key= 等键值对" },
  { name: "邮箱", hint: "邮箱地址" },
  { name: "银行卡", hint: "通过 Luhn 校验的卡号" },
  { name: "身份证号", hint: "18 位身份证" },
]

/** 占位符示例，格式与后端一致：{{JOLU_<类型>_<8位hash>}}；同值同占位符，不破坏 prompt cache */
export const MASKING_PLACEHOLDER_SAMPLE = "{{JOLU_APIKEY_1a2b3c4d}}"

/* ───────── 处理方式、豁免与记录（3.7；画板待确认） ───────── */

/** 一类值的处理方式：遮住（默认）、只记录（原样发出）、拦下（整个请求不发） */
export type MaskAction = "mask" | "warn" | "block"

export interface MaskingKind {
  /** 后端 gateway/mask.rs 的种类名 */
  kind: string
  name: string
  hint: string
}

/** 可设置处理方式的种类，与后端 TYPES 一一对应 */
export const PRIVACY_MASKING_KINDS: readonly MaskingKind[] = [
  { kind: "APIKEY", name: "API Key", hint: "sk- 等已知前缀的密钥" },
  { kind: "PRIVATEKEY", name: "私钥", hint: "PEM 私钥段" },
  { kind: "TOKEN", name: "令牌与凭证", hint: "Bearer / Basic 令牌、token= 等键值对" },
  { kind: "EMAIL", name: "邮箱", hint: "邮箱地址" },
  { kind: "CARD", name: "银行卡号", hint: "通过 Luhn 校验的卡号" },
  { kind: "IDCARD", name: "身份证号", hint: "18 位身份证" },
]

export const MASK_ACTIONS: readonly { value: MaskAction; label: string }[] = [
  { value: "mask", label: "遮住" },
  { value: "warn", label: "只记录" },
  { value: "block", label: "拦下" },
]

export function actionOf(actions: Record<string, string> | undefined, kind: string): MaskAction {
  const action = actions?.[kind]
  return action === "warn" || action === "block" ? action : "mask"
}

/** 改一类的处理方式；只保留非默认项（与后端保存口径一致） */
export function withAction(actions: Record<string, string> | undefined, kind: string, action: MaskAction): Record<string, string> {
  const next: Record<string, string> = {}
  for (const [key, value] of Object.entries(actions ?? {})) {
    if (key !== kind && (value === "warn" || value === "block")) next[key] = value
  }
  if (action !== "mask") next[kind] = action
  return next
}

/** 豁免清单输入：按行或逗号切开，去空白、按不分大小写去重 */
export function parseAllow(text: string): string[] {
  const seen = new Set<string>()
  const out: string[] = []
  for (const raw of text.split(/[\n,，]/)) {
    const value = raw.trim()
    const key = value.toLowerCase()
    if (!value || seen.has(key)) continue
    seen.add(key)
    out.push(value)
  }
  return out
}

export interface MaskHitDto {
  kind: string
  action: MaskAction
  length: number
  preview: string
  placeholder: string | null
  left: string
  right: string
}

export interface MaskRecordDto {
  id: number
  at_ms: number
  entry: string
  blocked: boolean
  masked: number
  hits: MaskHitDto[]
}

export function kindName(kind: string): string {
  return PRIVACY_MASKING_KINDS.find((item) => item.kind === kind)?.name ?? kind
}

/** 一条记录的摘要：「遮住 2 处 · 只记录 1 处」；拦下的请求单独说明 */
export function recordSummary(record: MaskRecordDto): string {
  if (record.blocked) return "已拦下，没有发出"
  const count = (action: MaskAction) => record.hits.filter((hit) => hit.action === action).length
  const parts = [`遮住 ${count("mask")} 处`]
  const warned = count("warn")
  if (warned) parts.push(`只记录 ${warned} 处`)
  return parts.join(" · ")
}

/** 对照：发出前（原值只露开头几个字与长度）与发出后（占位符 / 原样） */
export function hitSides(hit: MaskHitDto): { before: string; after: string } {
  const value = `${hit.preview}（${hit.length} 字）`
  const sent = hit.action === "mask" && hit.placeholder ? hit.placeholder : hit.action === "block" ? "（整条请求未发出）" : value
  return { before: `${hit.left}【${value}】${hit.right}`, after: `${hit.left}${sent}${hit.right}` }
}
