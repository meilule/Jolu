import { useMemo, useState } from "react"
import { CalendarDays, ChevronRight, CircleAlert, RefreshCw, Search, SlidersHorizontal, Wrench } from "lucide-react"
import { cn } from "@/lib/utils"
import type { ContextSessionRowDto, DayOverviewDto, RouteLogDto } from "@/lib/gatewayApi"
import type { SessionView } from "@/lib/gatewaySessions"
import { relativeTime } from "@/lib/gatewaySessions"
import { agentLabel } from "@/lib/gatewayPage"
import { formatLatency, headerLatency, hourlyTrend, successRate } from "@/lib/gatewayOverview"
import { activeSessionRows, filterRows, STATUS_FILTERS, statusCell, type ActiveSessionRow, type StatusFilter } from "@/lib/contextSessions"
import { AgentBadge } from "@/components/agents/AgentList"

export interface AttentionItem {
  key: string
  kind: "agent" | "tool" | "rate" | "context"
  title: string
  detail: string
  action: string
  onAction: () => void
}

const HOUR = 3_600_000
/** 请求趋势看最近 12 个小时（V10-01「最近 12 小时」） */
const TREND_HOURS = 12

/**
 * 模型路由 · 概览（Jolu.pen V10-01 浅色 / V10-02 深色「运行概览与活跃会话」）：
 * 六项运行指标、最近 12 小时请求趋势、需要关注、活跃会话（每个会话最近一次请求的上下文与缓存）。
 * 今日请求 / 改道 / 最终失败来自后端全天统计；趋势与响应头分位由路由流水推导（只保留最近约 1,000 行）；
 * 上下文来自 `gateway_context_sessions`，取最近一次请求，不是会话累计用量。
 */
export function OverviewPanel({ day, sessions, contexts, logRows, nowMs, updatedAt, sources, models, groups, attention, busy, onRefresh, onOpenLog, onOpenContext }: {
  day: DayOverviewDto | null
  sessions: SessionView[]
  /** 每个会话最近一次上下文快照；读取失败为 null（表格照常列出会话，上下文列写「读取失败」） */
  contexts: ContextSessionRowDto[] | null
  /** 最近的路由流水行（用于趋势 / 分位推导） */
  logRows: RouteLogDto[]
  nowMs: number
  updatedAt: number | null
  sources: number
  models: number
  groups: number
  attention: AttentionItem[]
  busy: boolean
  onRefresh: () => void
  onOpenLog: () => void
  /** 「查看上下文」：进入这个会话的会话观察（V10-03） */
  onOpenContext: (platform: string, sessionId: string) => void
}) {
  const [search, setSearch] = useState("")
  const [status, setStatus] = useState<StatusFilter>("all")
  const partial = day?.partial ?? false
  const limitNote = partial ? "（下限）" : ""
  const change = day && day.yesterday > 0 ? day.today - day.yesterday : null
  const running = sessions.filter((view) => view.running)
  const rate = successRate(day?.today, day?.errors)
  const trendStart = Math.floor(nowMs / HOUR) * HOUR - (TREND_HOURS - 1) * HOUR
  const trend = hourlyTrend(logRows, trendStart, nowMs)
  const trendOk = trend.reduce((sum, bucket) => sum + bucket.ok, 0)
  const trendFailed = trend.reduce((sum, bucket) => sum + bucket.failed, 0)
  const p50 = headerLatency(logRows, 0.5)
  const p95 = headerLatency(logRows, 0.95)
  const updated = updatedAt ? new Date(updatedAt).toLocaleTimeString("zh-CN", { hour12: false }) : null
  const rows = useMemo(() => activeSessionRows(sessions, contexts ?? []), [sessions, contexts])
  const shown = filterRows(rows, search, status, agentLabel)
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 px-8 pb-8 pt-2">
      <div className="flex items-center justify-between">
        <div className="flex items-baseline gap-2.5">
          <h2 className="text-[20px] font-semibold text-jolu-text">运行概览</h2>
          <span className="text-meta text-jolu-muted">先看全局，再进入会话</span>
        </div>
        <div className="flex items-center gap-2.5">
          <span className="flex items-center gap-1.5 text-note text-jolu-muted">
            <span aria-hidden className="size-1.5 rounded-full bg-jolu-on" />
            每 5 秒更新
          </span>
          {updated && <span className="text-note tabular-nums text-jolu-muted">{updated} 更新</span>}
          <span className="flex h-9 items-center gap-1.5 rounded-lg border border-jolu-border px-3 text-meta text-jolu-text" title="历史范围还没有后端支持（待确认）">
            <CalendarDays aria-hidden className="size-3.5 text-jolu-muted" />
            今天
            <ChevronRight aria-hidden className="size-3 rotate-90 text-jolu-muted" />
          </span>
          <button type="button" aria-label="立即刷新" disabled={busy} onClick={onRefresh}
            className="flex size-9 items-center justify-center rounded-lg border border-jolu-border outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
            <RefreshCw aria-hidden className={cn("size-3.5 text-jolu-muted", busy && "animate-spin")} />
          </button>
        </div>
      </div>

      <div className="grid grid-cols-6 overflow-hidden rounded-xl border border-jolu-border">
        <Metric label="今日请求" value={day ? day.today.toLocaleString("zh-CN") : "—"} note={day ? `已结束 · ${change === null ? `昨天 ${day.yesterday}${limitNote}` : `较昨日 ${change >= 0 ? "+" : ""}${change}${limitNote}`}` : "读取中"} />
        <Metric label="成功率" value={rate.value === null ? "—" : `${(rate.value * 100).toFixed(1)}%`} tone={rate.value !== null && rate.value < 1 ? "muted" : "ok"} note={rate.ok !== null && day ? `${rate.ok} / ${day.today} 成功${limitNote}` : "读取中"} />
        <Metric label="正在处理" value={String(running.length)} tone={running.length > 0 ? "ok" : undefined} note={running.length > 0 ? `来自 ${new Set(running.map((view) => view.session.session_id)).size} 个会话` : "现在没有进行中的请求"} />
        <Metric label="发生改道" value={day ? String(day.reroutes) : "—"} note={day && day.today > 0 ? `占今日请求 ${((day.reroutes / day.today) * 100).toFixed(1)}%${limitNote}` : "失败后切换成员"} />
        <Metric label="最终失败" value={day ? String(day.errors) : "—"} tone={day && day.errors > 0 ? "danger" : undefined} note={day && day.errors > 0 ? undefined : "今天没有"} action={day && day.errors > 0 ? "查看失败流水 →" : undefined} onAction={onOpenLog} />
        <Metric label="响应头 P50" value={formatLatency(p50)} note={p95 !== null ? `P95 ${formatLatency(p95)}` : "流水里还没有耗时样本"} />
      </div>

      <div className="grid grid-cols-[1fr_340px] gap-4">
        <section className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
          <header className="flex items-center gap-2.5 px-5 py-3">
            <span className="text-[15px] font-semibold text-jolu-text">请求趋势</span>
            <span className="text-note text-jolu-muted">最近 {TREND_HOURS} 小时</span>
            <span className="flex-1" />
            <span className="text-note tabular-nums text-jolu-muted">成功 {trendOk}　/　失败 {trendFailed}</span>
          </header>
          <div className="flex min-h-[150px] flex-1 items-end gap-[9px] px-5">
            {trendOk + trendFailed === 0 && <p className="w-full pb-6 text-center text-meta text-jolu-muted">最近 {TREND_HOURS} 小时还没有请求；收到请求后这里按小时画柱。</p>}
            {trendOk + trendFailed > 0 && trend.map((bucket) => {
              const max = Math.max(...trend.map((entry) => entry.ok + entry.failed), 1)
              const height = (value: number) => `${Math.round((value / max) * 100)}%`
              return (
                <div key={bucket.at} className="flex h-[110px] min-w-0 flex-1 flex-col justify-end" title={`${String(bucket.hour).padStart(2, "0")} 时 · 成功 ${bucket.ok} · 失败 ${bucket.failed}`}>
                  <div className="flex w-full flex-col justify-end" style={{ height: height(bucket.ok + bucket.failed) }}>
                    {bucket.failed > 0 && <div className="w-full rounded-t-[3px] bg-jolu-danger" style={{ height: height(bucket.failed) }} />}
                    <div className="w-full flex-1 rounded-t-[3px] bg-jolu-accent" />
                  </div>
                </div>
              )
            })}
          </div>
          <footer className="flex items-center px-5 py-2.5 text-note tabular-nums text-jolu-muted">
            <span>{clock(trendStart)}</span>
            <span className="flex-1" />
            <span>{clock(trendStart + Math.floor(TREND_HOURS / 2) * HOUR)}</span>
            <span className="flex-1" />
            <span>{clock(nowMs)}</span>
          </footer>
        </section>

        <section className="flex flex-col rounded-xl bg-jolu-side">
          <header className="flex items-center gap-2.5 px-5 py-3">
            <CircleAlert aria-hidden className="size-4 text-jolu-danger" />
            <span className="text-[15px] font-semibold text-jolu-text">需要关注</span>
            <span className="rounded-md bg-jolu-stage px-1.5 py-0.5 text-note font-medium text-jolu-muted">{attention.length} 项</span>
          </header>
          <div className="flex flex-1 flex-col px-5 pb-4">
            {attention.length === 0 && <p className="py-2 text-meta text-jolu-muted">一切正常：没有接近上限的上下文、没有漂移的接入、没有集中限流。</p>}
            {attention.slice(0, 3).map((item) => (
              <div key={item.key} className="flex flex-col gap-1 border-b border-jolu-table-divider py-2.5 first:pt-0 last:border-b-0">
                <span className="flex items-center gap-2">
                  {item.kind === "tool" || item.kind === "agent"
                    ? <Wrench aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />
                    : <CircleAlert aria-hidden className="size-3.5 shrink-0 text-jolu-danger" />}
                  <span className="min-w-0 flex-1 truncate text-meta font-medium text-jolu-text">{item.title}</span>
                </span>
                <span className="pl-5 text-note leading-relaxed text-jolu-muted">{item.detail}</span>
                <button type="button" onClick={item.onAction}
                  className="self-start pl-5 text-note font-medium text-jolu-on outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">
                  {item.action} →
                </button>
              </div>
            ))}
          </div>
        </section>
      </div>

      <section id="active-sessions" className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
        <header className="flex items-center gap-2.5 px-5 py-3">
          <span className="text-[15px] font-semibold text-jolu-text">活跃会话</span>
          <span className="text-note text-jolu-on">{running.length} 个处理中</span>
          <span className="flex-1" />
          <label className="flex h-9 w-56 items-center gap-1.5 rounded-lg border border-jolu-border px-2.5 focus-within:outline-2 focus-within:outline-jolu-accent">
            <Search aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />
            <input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索会话或项目" aria-label="搜索会话或项目"
              className="min-w-0 flex-1 bg-transparent text-meta text-jolu-text outline-none placeholder:text-jolu-muted" />
          </label>
          <label className="relative flex h-9 items-center gap-1.5 rounded-lg border border-jolu-border pl-2.5 pr-2 text-meta text-jolu-text" title="筛选项待确认">
            <SlidersHorizontal aria-hidden className="size-3.5 text-jolu-muted" />
            <select value={status} onChange={(event) => setStatus(event.target.value as StatusFilter)} aria-label="按状态筛选"
              className="appearance-none bg-transparent pr-4 text-meta text-jolu-text outline-none">
              {STATUS_FILTERS.map((option) => <option key={option.id} value={option.id}>{option.label}</option>)}
            </select>
            <ChevronRight aria-hidden className="pointer-events-none absolute right-2 size-3 rotate-90 text-jolu-muted" />
          </label>
        </header>
        <div className="grid grid-cols-[minmax(0,1fr)_210px_190px_130px_110px_88px] gap-3 bg-jolu-side px-5 py-2 text-note text-jolu-muted">
          <span>Agent / 会话 / 项目</span><span>实际模型 / 来源账号</span><span>最近一次上下文</span><span>缓存读取 / 占输入</span><span>状态 / 最近请求</span><span />
        </div>
        {shown.length === 0 && (
          <p className="border-t border-jolu-table-divider px-5 py-6 text-center text-meta text-jolu-muted">
            {rows.length === 0 ? "还没有经过模型路由的会话。" : "没有符合条件的会话。"}
          </p>
        )}
        {shown.slice(0, 8).map((row) => <SessionLine key={row.key} row={row} nowMs={nowMs} contextsFailed={contexts === null} onOpen={() => onOpenContext(row.platform, row.sessionId)} />)}
        <footer className="border-t border-jolu-table-divider px-5 py-2 text-note text-jolu-muted">
          缓存读取已包含在输入中；未采集与 0 分开显示。{shown.length > 8 ? `　共 ${shown.length} 个会话，显示最近 8 个。` : ""}
        </footer>
      </section>

      <footer className="flex items-center justify-between gap-4 text-note text-jolu-muted">
        <span>上下文取最近一次请求，非会话累计用量；趋势与响应头分位来自路由流水（只保留最近约 1,000 行）</span>
        <span className="shrink-0">{sources} 个来源　{models} 个模型　{groups} 个分组</span>
      </footer>
    </div>
  )
}

function SessionLine({ row, nowMs, contextsFailed, onOpen }: { row: ActiveSessionRow; nowMs: number; contextsFailed: boolean; onOpen: () => void }) {
  const status = statusCell(row.running)
  const context = contextsFailed && row.snapshotId === null ? { text: "读取失败", ratio: null, tone: "danger" as const, title: "上下文快照读取失败，稍后自动重试" } : row.context
  const agentName = row.agent ? agentLabel(row.agent) : null
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_210px_190px_130px_110px_88px] items-center gap-3 border-t border-jolu-table-divider px-5 py-2.5 text-meta">
      <span className="flex min-w-0 items-center gap-2.5">
        {row.agent ? <AgentBadge agent={row.agent} name={agentName ?? row.agent} size={24} /> : <span aria-hidden className="size-6 shrink-0 rounded-[7px] bg-jolu-side" />}
        <span className="flex min-w-0 flex-col">
          <span className="truncate font-medium text-jolu-text" title={row.title}>{row.title}</span>
          <span className="truncate text-note text-jolu-muted">{[agentName, row.project].filter(Boolean).join(" · ") || "—"}</span>
        </span>
      </span>
      <span className="flex min-w-0 flex-col">
        <span className="truncate font-mono text-note text-jolu-text" title={row.model ?? undefined}>{row.model ?? "—"}</span>
        <span className="truncate text-note text-jolu-muted" title={row.account ?? undefined}>{row.account ?? "—"}</span>
      </span>
      <span className="flex min-w-0 flex-col gap-1.5" title={context.title}>
        <span className="flex items-baseline gap-1.5">
          <span className={cn("truncate text-note font-medium tabular-nums", context.tone === "danger" ? "text-jolu-danger" : context.tone === "muted" ? "text-jolu-muted" : "text-jolu-text")}>{context.text}</span>
          <span className="flex-1" />
          {context.ratio != null && <span className="shrink-0 text-note tabular-nums text-jolu-muted">{(context.ratio * 100).toFixed(1)}%</span>}
        </span>
        {context.ratio != null && (
          <span className="h-1 overflow-hidden rounded-full bg-jolu-track">
            <span className={cn("block h-full rounded-full", context.tone === "danger" ? "bg-jolu-danger" : "bg-jolu-accent")} style={{ width: `${Math.min(100, Math.round(context.ratio * 100))}%` }} />
          </span>
        )}
      </span>
      <span className="truncate text-note tabular-nums text-jolu-muted" title={row.cache.title}>{row.cache.text}</span>
      <span className="flex min-w-0 flex-col">
        <span className={cn("text-note", status.tone === "on" ? "text-jolu-on" : "text-jolu-muted")}>{status.text}</span>
        <span className="text-note tabular-nums text-jolu-muted">{relativeTime(row.lastTs, nowMs)}</span>
      </span>
      <span className="text-right">
        <button type="button" onClick={onOpen}
          className="text-note font-medium text-jolu-on outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">
          查看上下文
        </button>
      </span>
    </div>
  )
}

function Metric({ label, value, note, tone, action, onAction }: {
  label: string
  value: string
  note?: string
  tone?: "ok" | "danger" | "muted"
  action?: string
  onAction?: () => void
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1 border-l border-jolu-table-divider px-4 py-3.5 first:border-l-0">
      <span className="text-note text-jolu-muted">{label}</span>
      <span className={cn(
        "text-[24px] font-semibold leading-8",
        tone === "danger" ? "text-jolu-danger" : tone === "ok" ? "text-jolu-on" : "text-jolu-text",
      )}>{value}</span>
      {action && onAction ? (
        <button type="button" onClick={onAction} className="self-start text-note font-medium text-jolu-danger outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">{action}</button>
      ) : (
        <span className="truncate text-note text-jolu-muted" title={note}>{note}</span>
      )}
    </div>
  )
}

function clock(ts: number): string {
  return new Date(ts).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false })
}
