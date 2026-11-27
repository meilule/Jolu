import type { ContextSessionRowDto, GatewaySessionDto } from "./gatewayApi.ts"
import { compactTokens, sessionKey, sessionTitle, type SessionView } from "./gatewaySessions.ts"
import { PROVIDERS } from "./providerCatalog.ts"

/**
 * 模型路由 · 概览「活跃会话」表的纯逻辑（Jolu.pen V10-01 / V10-02；状态口径见 V10-13；测试见 `contextSessions.test.mjs`）。
 *
 * 纪律（V10-13）：
 * - 上下文取最近一次请求，不拿会话累计用量冒充；
 * - 未知、真实的零、不支持、采集失败分别表达：没有快照写「暂无请求」/「尚未采集」，没有缓存数写「未提供」，
 *   模型窗口未知时写「容量未知」并隐藏百分比与进度；
 * - 输入总量已包含缓存读取，缓存占比的分母就是输入，不再相加。
 */

/** 上下文占窗口达到这个比例时标红并进「需要关注」。画板示例 84% 标红、63% 不标；阈值本身待确认。 */
export const CONTEXT_RISK = 0.8

export type CellTone = "danger" | "normal" | "muted"

export interface ContextCell {
  /** 主文字：「168K / 200K」「631K / 容量未知」「尚未采集」…… */
  text: string
  /** 占窗口比例（0–1）；窗口未知或没有数字时为 null，界面隐藏百分比与进度条 */
  ratio: number | null
  tone: CellTone
  /** 悬停说明：口径、估算还是实测、失败原因 */
  title: string
}

export interface CacheCell {
  text: string
  title: string
}

export interface StatusCell {
  text: string
  tone: "on" | "muted"
  /** 次级补充说明（如「无在途请求」），放在副行以免撑破窄列状态胶囊 */
  note?: string | null
  /** 悬停说明 */
  title?: string
}

export interface ActiveSessionRow {
  key: string
  platform: string
  sessionId: string
  agent: string | null
  title: string
  project: string | null
  running: boolean
  /** 会话生命周期状态（`GatewaySessionDto.thread_status`）；只有快照、没有会话记录时为 null */
  threadStatus: string | null
  /** 最近活动时间：快照、会话线程、经网关流水与在途请求取最新 */
  lastTs: number
  model: string | null
  account: string | null
  context: ContextCell
  cache: CacheCell
  /** 最近一次快照的 id；没有快照为 null（「查看上下文」不可用） */
  snapshotId: number | null
}

/** 输入量：上游实测优先；没有实测时用本地估算并注明 */
function usedTokens(row: ContextSessionRowDto): { value: number | null; estimated: boolean } {
  if (row.input_tokens != null) return { value: row.input_tokens, estimated: false }
  if (row.estimated != null) return { value: row.estimated, estimated: true }
  return { value: null, estimated: false }
}

/**
 * 「最近一次上下文」单元格。
 * `routed`：这个会话有没有经过模型路由（有经网关的流水）——没有快照时据此区分「暂无请求」与「尚未采集」。
 */
export function contextCell(row: ContextSessionRowDto | null, routed: boolean): ContextCell {
  if (!row) {
    return routed
      ? { text: "尚未采集", ratio: null, tone: "muted", title: "已记录路由去向，当前请求没有内容快照" }
      : { text: "暂无请求", ratio: null, tone: "muted", title: "这个会话还没有经过 Jolu 的模型路由" }
  }
  const used = usedTokens(row)
  if (used.value == null) {
    if (row.state === "analyzing") return { text: "正在分析", ratio: null, tone: "muted", title: "已收到请求，正在整理上下文组成" }
    if (row.state === "failed") {
      return { text: "采集失败", ratio: null, tone: "danger", title: row.last_ready_ts ? `上次成功 ${clock(row.last_ready_ts)}` : "请求已完成，但上下文快照没有保存成功" }
    }
    if (row.state === "unsupported") return { text: "暂不支持分析", ratio: null, tone: "muted", title: "此来源暂不支持内容分析，可查看路由与上游用量" }
    return { text: "尚未采集", ratio: null, tone: "muted", title: "已记录路由去向，当前请求没有内容快照" }
  }
  const source = used.estimated ? "本地估算（上游还没回用量）" : "上游实测"
  const window = row.window_tokens
  if (window == null || window <= 0) {
    return { text: `${compactTokens(used.value)} / 容量未知`, ratio: null, tone: "normal", title: `输入 ${used.value.toLocaleString("zh-CN")} · ${source}；模型窗口容量尚未确认` }
  }
  const ratio = used.value / window
  return {
    text: `${compactTokens(used.value)} / ${compactTokens(window)}`,
    ratio,
    tone: ratio >= CONTEXT_RISK ? "danger" : "normal",
    title: `输入 ${used.value.toLocaleString("zh-CN")} / 窗口 ${window.toLocaleString("zh-CN")} · ${source}`,
  }
}

/** 「缓存读取 / 占输入」单元格：上游没给写「未提供」（不是 0）；给了 0 就显示 0 */
export function cacheCell(row: ContextSessionRowDto | null): CacheCell {
  if (!row || row.cache_read == null) return { text: "未提供", title: "上游没有返回缓存读取数" }
  const input = row.input_tokens
  if (input == null || input <= 0) return { text: compactTokens(row.cache_read), title: "缓存读取（输入总量未知，不算占比）" }
  const share = (row.cache_read / input) * 100
  return { text: `${compactTokens(row.cache_read)} · ${share.toFixed(1)}%`, title: "缓存读取已包含在输入中" }
}

/**
 * 状态列。在途请求只是「正在生成」的证据；没有在途请求时还要看会话自己的生命周期状态
 * （`thread_status`，灵动岛与会话页用的同一份），否则 Claude Code 在思考、执行本地工具时
 * 这里写「空闲」，而灵动岛和会话页写「运行中」（P4 审计 D12）。
 * 胶囊本身只放简短状态词，避免撑破表格状态列；补充说明放进 `note` / `title`。
 */
export function statusCell(running: boolean, threadStatus: string | null = null): StatusCell {
  if (running) return { text: "正在生成", tone: "on", title: "当前有在途模型路由请求" }
  if (threadStatus === "running") return { text: "运行中", note: "无在途请求", tone: "on", title: "会话仍在运行，当前无在途路由请求" }
  if (threadStatus === "done") return { text: "已完成", tone: "muted", title: "会话已完成" }
  if (threadStatus === "failed") return { text: "失败", tone: "muted", title: "会话以失败结束" }
  return { text: "最近活跃", tone: "muted", title: "最近经过模型路由的会话" }
}

/** 项目名：工作目录的最后一段 */
export function projectOf(cwd: string | null | undefined): string | null {
  if (!cwd) return null
  const parts = cwd.split(/[\\/]+/).filter(Boolean)
  return parts.length > 0 ? parts[parts.length - 1] : null
}

/** 来源账号：「平台 · 账号名」，平台名查不到时只写账号名 */
export function accountOf(row: ContextSessionRowDto | null, fallbackSource: string | null): string | null {
  const name = row?.connection_name ?? null
  const platform = row?.connection_platform ?? null
  const platformName = platform ? PROVIDERS.find((entry) => entry.id === platform)?.name ?? null : null
  if (name && platformName && !name.includes(platformName)) return `${platformName} · ${name}`
  return name ?? row?.source ?? fallbackSource
}

const bareModel = (model: string | null | undefined) => (model ? model.replace(/^[^/]+\//, "") : null)

/** 判断这条会话是否确实经过模型路由（在途请求、路由流水、网关时间戳或上下文快照任一成立）。 */
function isRoutedView(view: SessionView, ctx: ContextSessionRowDto | null): boolean {
  return view.running || view.session.links.length > 0 || view.session.gateway_last_ts != null || ctx != null
}

/**
 * 合并「经模型路由的会话」（标题、项目、运行状态）与每个会话最近一次快照。
 * 未经过模型路由的纯本机会话会被过滤掉；顺序：运行中在前，其余按最近活动倒序。
 */
export function activeSessionRows(views: SessionView[], contexts: ContextSessionRowDto[]): ActiveSessionRow[] {
  const byKey = new Map(contexts.map((row) => [sessionKey(row.session_platform, row.session_id), row]))
  const seen = new Set<string>()
  const rows: ActiveSessionRow[] = []
  const push = (
    key: string,
    session: GatewaySessionDto | null,
    running: boolean,
    activity: number,
    ctx: ContextSessionRowDto | null,
    liveModel: string | null = null,
    liveSource: string | null = null,
  ) => {
    seen.add(key)
    const link = session?.links[0] ?? null
    const routed = running || (session?.links.length ?? 0) > 0 || session?.gateway_last_ts != null || ctx != null
    rows.push({
      key,
      platform: session?.platform ?? ctx?.session_platform ?? "",
      sessionId: session?.session_id ?? ctx?.session_id ?? "",
      agent: session?.agent ?? ctx?.agent ?? link?.agent ?? null,
      title: session ? sessionTitle(session) : sessionTitle({ title: null, session_id: ctx?.session_id ?? "" }),
      project: projectOf(session?.cwd),
      running,
      threadStatus: session?.thread_status ?? null,
      lastTs: Math.max(activity, ctx?.ts ?? 0),
      model: ctx?.upstream_model ?? bareModel(ctx?.requested_model) ?? link?.upstream_model ?? bareModel(link?.requested_model) ?? bareModel(liveModel),
      account: accountOf(ctx, link?.source ?? liveSource ?? null),
      context: contextCell(ctx, routed),
      cache: cacheCell(ctx),
      snapshotId: ctx?.id ?? null,
    })
  }
  for (const view of views) {
    const ctx = byKey.get(view.key) ?? null
    if (!isRoutedView(view, ctx)) continue
    push(view.key, view.session, view.running, view.activity, ctx, view.liveModel ?? null, view.liveSource ?? null)
  }
  for (const [key, ctx] of byKey) if (!seen.has(key)) push(key, null, false, 0, ctx)
  return rows.sort((a, b) => Number(b.running) - Number(a.running) || b.lastTs - a.lastTs)
}

export type StatusFilter = "all" | "running" | "risk" | "uncollected"

export const STATUS_FILTERS: readonly { id: StatusFilter; label: string }[] = [
  { id: "all", label: "全部状态" },
  { id: "running", label: "正在生成" },
  { id: "risk", label: "接近上限" },
  { id: "uncollected", label: "尚未采集" },
]

/** 搜索会话标题、项目、Agent、模型（不区分大小写）与状态筛选 */
export function filterRows(rows: ActiveSessionRow[], search: string, status: StatusFilter, agentName: (agent: string) => string = (a) => a): ActiveSessionRow[] {
  const needle = search.trim().toLowerCase()
  return rows.filter((row) => {
    if (status === "running" && !row.running) return false
    if (status === "risk" && !(row.context.ratio != null && row.context.ratio >= CONTEXT_RISK)) return false
    if (status === "uncollected" && row.snapshotId != null) return false
    if (!needle) return true
    const hay = [row.title, row.project, row.agent ? agentName(row.agent) : null, row.agent, row.model].filter(Boolean).join("\n").toLowerCase()
    return hay.includes(needle)
  })
}

export interface ContextRisk {
  key: string
  platform: string
  sessionId: string
  title: string
  detail: string
}

/** 「需要关注」：最近一次上下文已接近模型窗口的会话，占比高的在前 */
export function contextRisks(rows: ActiveSessionRow[], agentName: (agent: string) => string = (a) => a): ContextRisk[] {
  return rows
    .filter((row) => row.context.ratio != null && row.context.ratio >= CONTEXT_RISK)
    .sort((a, b) => (b.context.ratio ?? 0) - (a.context.ratio ?? 0))
    .map((row) => ({
      key: `context:${row.key}`,
      platform: row.platform,
      sessionId: row.sessionId,
      title: `${row.agent ? agentName(row.agent) : row.title} 上下文已用 ${((row.context.ratio ?? 0) * 100).toFixed(0)}%`,
      detail: `最近一次请求 · ${row.context.text}`,
    }))
}

function clock(ts: number): string {
  return new Date(ts).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false })
}
