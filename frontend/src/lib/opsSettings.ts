/**
 * 批次 G 的运维项展示逻辑（纯函数，供设置页与 node 测试共用）。
 *
 * 对应后端：
 * - `agent_version::AgentVersion`：版本检测只读，不自动升级
 * - `otlp::metrics_url`：端点在保存前就补好 `/v1/metrics`
 * - `warmup::should_fire`：每天一次，失败不算已发
 */

export interface AgentVersionRow {
  agent: string
  name: string
  found: boolean
  version: string | null
  raw: string | null
  update_command: string | null
  update_note: string
}

/**
 * 一个字幕化版本摘要：命令在不在、版本读没读到，必须分开说。
 *
 * 「未安装」和「装了但读出版本」是两回事；都不能笼统写成「未知」。
 */
export function versionLabel(row: Pick<AgentVersionRow, "found" | "version">): string {
  if (!row.found) return "未安装"
  return row.version ? `v${row.version}` : "已安装 · 版本未能读取"
}

/** 更新命令是不是可以复制执行的完整命令（不是说明文字）。 */
export function hasRunnableUpdate(row: Pick<AgentVersionRow, "update_command">): boolean {
  return typeof row.update_command === "string" && row.update_command.trim() !== ""
}

/**
 * warm-up 的当前状态文案。
 *
 * 优先级：未开启 → 今天已跑 → 等待今天的时间点 → 到点待跑。
 * `lastDate` 用本地日期字符串比较，不做时区换算（后端也用本地日期）。
 */
export function warmupStateText(
  enabled: boolean, lastDate: string | null | undefined, today: string,
): string {
  if (!enabled) return "未开启（开启后会真实消耗额度）"
  if (lastDate === today) return "今天已完成"
  return "等待今天的时间点"
}

/** 该小时是否合法（后端按 0–23 截断）。 */
export function normalizeWarmupHour(value: number): number {
  if (!Number.isFinite(value)) return 9
  return Math.min(23, Math.max(0, Math.trunc(value)))
}

/**
 * OTLP 端点的展示与校验提示。
 *
 * 后端保存时会补 `/v1/metrics`；这里给用户看补好之后的样子，避免「我填的」和
 * 「实际请求的」不一致。空串表示未开启导出。
 */
export function otlpEndpointHint(endpoint: string | null | undefined): string {
  const value = (endpoint ?? "").trim()
  if (value === "") return "未配置：用量不会离开本机"
  if (!/^https?:\/\//i.test(value)) return "端点必须以 http:// 或 https:// 开头"
  return value.endsWith("/v1/metrics") ? value : `${value.replace(/\/+$/, "")}/v1/metrics`
}

/** OTLP 导出只包含聚合值，这一点必须在界面上写明，避免用户以为是全量日志。 */
export const OTLP_SCOPE_NOTICE =
  "只导出按「平台 + 模型」聚合的 token 计数与请求数，不含会话 ID、连接标识、Key 或任何请求正文。"
