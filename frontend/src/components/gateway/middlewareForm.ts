/**
 * 网关中间件表单 ↔ 后端配置 JSON（纯函数，供 `MiddlewareDialog` 与 node 测试共用）。
 *
 * 后端语义见 `backend/src/gateway/middleware.rs`：中间件在选路之前按 sort 顺序改写请求体，
 * 单个失败只跳过自己；保存时后端会在样例请求上试跑，跑不通的配置拒绝保存。
 */

export type MiddlewareKind = "model-map" | "param-override" | "system-prompt" | "word-guard" | "think-tags" | "js-hook"

export const MIDDLEWARE_KINDS: readonly { id: MiddlewareKind; label: string; hint: string }[] = [
  { id: "model-map", label: "模型改名", hint: "把 agent 发来的模型名映射成「来源/模型」，映射后的名字参与选路与记账" },
  { id: "param-override", label: "参数覆盖", hint: "强制设置或删除请求体顶层参数，例如 max_tokens、temperature" },
  { id: "system-prompt", label: "追加系统提示", hint: "在 agent 自己的系统提示之后追加一段，不替换原有内容" },
  { id: "word-guard", label: "敏感词", hint: "拦截：命中整条请求拒发；遮盖：命中的词替换成 ***。只作用于请求" },
  { id: "think-tags", label: "思考段处理", hint: "剥掉 <think>…</think> 与单独返回的思考内容，或把思考内容包进 <think> 放到正文前；流式回复逐事件处理" },
  { id: "js-hook", label: "JS 钩子", hint: "本机 .mjs / .js 脚本的 onRequest（选路前改请求，改模型 / 流式 / 工具会回滚）与 onResponse（拿到完整回复副本）；需要本机装有 Node 或 Bun。待确认：无画板" },
]

/** 表单字段：各类型只用到其中几项。 */
export interface MiddlewareFields {
  mapping: string
  set: string
  remove: string
  prompt: string
  block: string
  mask: string
  mode: "strip" | "wrap"
  path: string
  timeout: string
}

export const emptyFields = (): MiddlewareFields => ({ mapping: "", set: "", remove: "", prompt: "", block: "", mask: "", mode: "strip", path: "", timeout: "" })

export type ConfigResult = { ok: true; config: Record<string, unknown> } | { ok: false; error: string }

const list = (raw: string): string[] => raw.split(/[\n,，、]+/).map((part) => part.trim()).filter(Boolean)

/** 「键 = 值」逐行解析；空行跳过，缺等号或缺键报出行号。 */
function pairs(raw: string, what: string): { ok: true; rows: [string, string][] } | { ok: false; error: string } {
  const rows: [string, string][] = []
  const lines = raw.split("\n")
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index].trim()
    if (!line) continue
    const at = line.indexOf("=")
    const key = at > 0 ? line.slice(0, at).trim() : ""
    const value = at > 0 ? line.slice(at + 1).trim() : ""
    if (!key || !value) return { ok: false, error: `${what}第 ${index + 1} 行需要写成「名称 = 值」` }
    rows.push([key, value])
  }
  return { ok: true, rows }
}

/** 值按 JSON 解析（数字、布尔、对象），解析不了就当字符串。 */
function jsonOrText(value: string): unknown {
  try {
    return JSON.parse(value)
  } catch {
    return value
  }
}

export function toConfig(kind: MiddlewareKind, fields: MiddlewareFields): ConfigResult {
  switch (kind) {
    case "model-map": {
      const parsed = pairs(fields.mapping, "改名表")
      if (!parsed.ok) return parsed
      if (parsed.rows.length === 0) return { ok: false, error: "至少写一条改名，例如 fast = zcode/GLM-5.2" }
      return { ok: true, config: { mapping: Object.fromEntries(parsed.rows) } }
    }
    case "param-override": {
      const parsed = pairs(fields.set, "设置项")
      if (!parsed.ok) return parsed
      const remove = list(fields.remove)
      if (parsed.rows.length === 0 && remove.length === 0) return { ok: false, error: "至少设置或删除一个参数" }
      const config: Record<string, unknown> = {}
      if (parsed.rows.length > 0) config.set = Object.fromEntries(parsed.rows.map(([key, value]) => [key, jsonOrText(value)]))
      if (remove.length > 0) config.delete = remove
      return { ok: true, config }
    }
    case "system-prompt": {
      const prompt = fields.prompt.trim()
      return prompt ? { ok: true, config: { prompt } } : { ok: false, error: "请填写要追加的系统提示" }
    }
    case "word-guard": {
      const block = list(fields.block)
      const mask = list(fields.mask)
      if (block.length === 0 && mask.length === 0) return { ok: false, error: "拦截词与遮盖词至少填一项" }
      const config: Record<string, unknown> = {}
      if (block.length > 0) config.block = block
      if (mask.length > 0) config.mask = mask
      return { ok: true, config }
    }
    case "think-tags":
      return { ok: true, config: { mode: fields.mode } }
    case "js-hook": {
      const path = fields.path.trim()
      if (!path) return { ok: false, error: "请填写钩子脚本的完整路径" }
      const config: Record<string, unknown> = { path }
      if (fields.timeout.trim()) {
        const ms = Number(fields.timeout.trim())
        if (!Number.isInteger(ms) || ms <= 0) return { ok: false, error: "超时请填正整数毫秒" }
        config.timeoutMs = ms
      }
      return { ok: true, config }
    }
  }
}

const strings = (value: unknown): string[] => (Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [])
const record = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {}

/** 已保存的配置回填表单（编辑用）。 */
export function fromConfig(kind: MiddlewareKind, config: unknown): MiddlewareFields {
  const source = record(config)
  const fields = emptyFields()
  if (kind === "model-map") fields.mapping = Object.entries(record(source.mapping)).map(([key, value]) => `${key} = ${String(value)}`).join("\n")
  if (kind === "param-override") {
    fields.set = Object.entries(record(source.set)).map(([key, value]) => `${key} = ${typeof value === "string" ? value : JSON.stringify(value)}`).join("\n")
    fields.remove = strings(source.delete).join(", ")
  }
  if (kind === "system-prompt") fields.prompt = typeof source.prompt === "string" ? source.prompt : ""
  if (kind === "word-guard") {
    fields.block = strings(source.block).join(", ")
    fields.mask = strings(source.mask).join(", ")
  }
  if (kind === "think-tags") fields.mode = source.mode === "wrap" ? "wrap" : "strip"
  if (kind === "js-hook") {
    fields.path = typeof source.path === "string" ? source.path : ""
    fields.timeout = typeof source.timeoutMs === "number" ? String(source.timeoutMs) : ""
  }
  return fields
}

/** 列表里的一行概要。 */
export function summary(kind: string, config: unknown): string {
  const source = record(config)
  switch (kind) {
    case "model-map":
      return `${Object.keys(record(source.mapping)).length} 条改名`
    case "param-override": {
      const parts = [`设置 ${Object.keys(record(source.set)).length} 项`, `删除 ${strings(source.delete).length} 项`]
      return parts.join(" · ")
    }
    case "system-prompt": {
      const prompt = typeof source.prompt === "string" ? source.prompt : ""
      return prompt.length > 24 ? `${prompt.slice(0, 24)}…` : prompt
    }
    case "word-guard":
      return [`拦截 ${strings(source.block).length} 个词`, `遮盖 ${strings(source.mask).length} 个词`].join(" · ")
    case "think-tags":
      return source.mode === "wrap" ? "包进思考标签" : "剥掉思考段"
    case "js-hook":
      return typeof source.path === "string" ? source.path.split(/[\\/]/).pop() ?? "" : "未设置脚本"
    default:
      return "未知类型"
  }
}

/** 上移（-1）/ 下移（+1）一项并按新顺序重排 sort；越界不动。 */
export function reorder<T extends { name: string; sort: number }>(rows: readonly T[], name: string, delta: -1 | 1): T[] {
  const ordered = [...rows].sort((a, b) => a.sort - b.sort)
  const from = ordered.findIndex((row) => row.name === name)
  const to = from + delta
  if (from < 0 || to < 0 || to >= ordered.length) return ordered
  const [moved] = ordered.splice(from, 1)
  ordered.splice(to, 0, moved)
  return ordered.map((row, index) => ({ ...row, sort: index }))
}
