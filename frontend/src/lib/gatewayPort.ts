export const GATEWAY_PORT_MIN = 1024
export const GATEWAY_PORT_MAX = 65535

export type PortParseResult =
  | { ok: true; port: number }
  | { ok: false; error: string }

/** 仅保留数字，最多 5 位（65535 为 5 位）。 */
export function sanitizePortInput(raw: string): string {
  return raw.replace(/\D+/g, "").slice(0, 5)
}

/** 校验用户输入的端口号。 */
export function parseGatewayPort(raw: string): PortParseResult {
  const trimmed = raw.trim()
  if (!trimmed) return { ok: false, error: "请输入端口号" }
  if (!/^\d+$/.test(trimmed)) return { ok: false, error: "端口只能填写数字" }
  const port = Number(trimmed)
  if (!Number.isInteger(port) || port < GATEWAY_PORT_MIN || port > GATEWAY_PORT_MAX) {
    return { ok: false, error: `端口须在 ${GATEWAY_PORT_MIN}–${GATEWAY_PORT_MAX} 之间` }
  }
  return { ok: true, port }
}

/** 判断启动/切换模型路由的错误是否属于端口被占用或被系统保留（如 Windows 10013）。 */
export function isPortBindError(error: unknown): boolean {
  const text = String(error ?? "")
  if (!text) return false
  return (
    text.includes("系统保留或禁止监听") ||
    text.includes("已被占用") ||
    text.includes("10013") ||
    text.includes("10048") ||
    text.includes("访问权限不允许") ||
    text.includes("WSAEACCES") ||
    text.includes("WSAEADDRINUSE") ||
    text.includes("AddrInUse") ||
    text.includes("PermissionDenied") ||
    (/监听\s+[\d.:a-zA-Z-]+\s+失败/.test(text))
  )
}

export interface PortChangeOutcomeSummary {
  port: number
  restored: string[]
  skipped: string[]
  failed: string[]
  stopped: boolean
}

/** 根据后端返回组装端口修改成功的反馈文案。 */
export function portChangeSuccessText(outcome: PortChangeOutcomeSummary): string {
  if (!outcome.stopped) return `监听端口已改为 ${outcome.port}`
  const parts: string[] = []
  if (outcome.restored.length > 0) parts.push(`已还原 ${outcome.restored.length} 个工具`)
  if (outcome.skipped.length > 0) parts.push(`${outcome.skipped.length} 个已由外部修改未覆盖`)
  if (outcome.failed.length > 0) parts.push(`${outcome.failed.length} 个还原失败`)
  const prefix = parts.length > 0 ? `${parts.join("，")}并停止中转，` : "已停止中转，"
  return `${prefix}监听端口已改为 ${outcome.port}`
}
