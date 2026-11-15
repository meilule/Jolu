import { CalendarDays, ChevronRight, CircleAlert, RefreshCw, Wrench } from "lucide-react"
import { cn } from "@/lib/utils"
import type { DayOverviewDto, RouteLogDto } from "@/lib/gatewayApi"
import type { SessionView } from "@/lib/gatewaySessions"
import { agentLabel } from "@/lib/gatewayPage"
import {
  byAgent, formatLatency, headerLatency, hourlyTrend, sourceDistribution, successRate,
} from "@/lib/gatewayOverview"

export interface AttentionItem {
  key: string
  kind: "agent" | "tool" | "rate"
  title: string
  detail: string
  action: string
  onAction: () => void
}

/**
 * 模型路由 · 概览（Jolu.pen「统一网关 · R2 重设计」V6 画板 cb7KY 浅色 / Cktyu 深色）：
 * 运行监控六指标、每小时请求趋势、来源分布、请求活动、需要关注。
 * 今日请求 / 改道 / 最终失败来自后端全天统计；趋势、来源分布、按工具统计与响应头分位
 * 由路由流水推导，流水只保留最近约 1,000 行——页脚如实注明。
 */
export function OverviewPanel({ day, sessions, logRows, dayStartMs, nowMs, updatedAt, sources, models, groups, middlewares, attention, busy, onRefresh, onOpenLog }: {
  day: DayOverviewDto | null
  sessions: SessionView[]
  /** 最近的路由流水行（用于趋势 / 来源分布 / 分位推导） */
  logRows: RouteLogDto[]
  dayStartMs: number
  nowMs: number
  updatedAt: number | null
  sources: number
  models: number
  groups: number
  middlewares: number | null
  attention: AttentionItem[]
  busy: boolean
  onRefresh: () => void
  onOpenLog: () => void
}) {
  const partial = day?.partial ?? false
  const limitNote = partial ? "（下限）" : ""
  const change = day && day.yesterday > 0 ? day.today - day.yesterday : null
  const running = sessions.filter((view) => view.running)
  const rate = successRate(day?.today, day?.errors)
  const trend = hourlyTrend(logRows, dayStartMs, nowMs)
  const distribution = sourceDistribution(logRows)
  const agents = byAgent(logRows)
  const p50 = headerLatency(logRows, 0.5)
  const p95 = headerLatency(logRows, 0.95)
  const trendTotal = trend.reduce((sum, bucket) => sum + bucket.ok + bucket.failed, 0)
  const recent = [...logRows].sort((a, b) => b.ts - a.ts).slice(0, 4)
  const updated = updatedAt ? new Date(updatedAt).toLocaleTimeString("zh-CN", { hour12: false }) : null
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 px-8 pb-8 pt-2">
      <div className="flex items-center justify-between">
        <div className="flex items-baseline gap-2.5">
          <h2 className="text-[20px] font-semibold text-jolu-text">运行监控</h2>
          <span className="text-meta text-jolu-muted">本机记录</span>
        </div>
        <div className="flex items-center gap-2.5">
          <span className="flex items-center gap-1.5 text-note text-jolu-muted">
            <span aria-hidden className="size-1.5 rounded-full bg-jolu-on" />
            每 5 秒刷新
            <button type="button" aria-label="暂停刷新（待确认）" disabled title="暂停自动刷新（待确认）" className="text-jolu-muted">
              <svg aria-hidden viewBox="0 0 14 14" className="size-3" fill="currentColor"><path d="M4 2.5h2v9H4zM8 2.5h2v9H8z" /></svg>
            </button>
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
        <Metric label="发生改道" value={day ? String(day.reroutes) : "—"} note={day && day.today > 0 ? `占今日请求 ${((day.reroutes / day.today) * 100).toFixed(1)}%${limitNote}` : "失败后换下一个成员"} />
        <Metric label="最终失败" value={day ? String(day.errors) : "—"} tone={day && day.errors > 0 ? "danger" : undefined} note={day && day.errors > 0 ? undefined : "今天没有"} action={day && day.errors > 0 ? "查看失败流水 →" : undefined} onAction={onOpenLog} />
        <Metric label="响应头 P50" value={formatLatency(p50)} note={p95 !== null ? `P95 ${formatLatency(p95)}` : "流水里还没有耗时样本"} />
      </div>

      <div className="grid grid-cols-[1fr_380px] gap-4">
        <section className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
          <header className="flex items-center gap-2.5 px-5 py-3">
            <span className="text-[15px] font-semibold text-jolu-text">请求趋势</span>
            <span className="text-note text-jolu-muted">{dayRangeLabel(nowMs)}</span>
            <span className="flex-1" />
            <Legend colorClass="bg-jolu-on" label="成功" />
            <Legend colorClass="bg-jolu-danger" label="最终失败" />
          </header>
          <div className="flex min-h-[210px] flex-1 items-end gap-[6px] px-5 pb-2">
            {trendTotal === 0 && <p className="w-full pb-6 text-center text-meta text-jolu-muted">今天还没有请求；收到请求后这里按小时画柱。</p>}
            {trendTotal > 0 && trend.map((bucket) => {
              const max = Math.max(...trend.map((entry) => entry.ok + entry.failed), 1)
              const height = (value: number) => `${Math.round((value / max) * 100)}%`
              return (
                <div key={bucket.at} className="flex h-[150px] min-w-0 flex-1 flex-col justify-end" title={`${String(bucket.hour).padStart(2, "0")} 时 · 成功 ${bucket.ok} · 最终失败 ${bucket.failed}`}>
                  <div className="flex w-full flex-col justify-end" style={{ height: height(bucket.ok + bucket.failed) }}>
                    {bucket.failed > 0 && <div className="w-full rounded-t-[3px] bg-jolu-danger" style={{ height: height(bucket.failed) }} />}
                    <div className="w-full flex-1 rounded-t-[3px] bg-jolu-on" />
                  </div>
                </div>
              )
            })}
          </div>
          <footer className="flex items-center gap-2 border-t border-jolu-table-divider px-5 py-2.5 text-note text-jolu-muted">
            <span className="truncate">{agents.slice(0, 3).map((entry) => `${agentLabel(entry.agent)} ${entry.total}`).join("　·　") || "还没有按工具的请求"}</span>
            <span className="flex-1" />
            <span>每小时 · {trend.length} 时统计中</span>
          </footer>
        </section>

        <section className="flex flex-col rounded-xl border border-jolu-border">
          <header className="flex items-center justify-between px-5 py-3">
            <span className="text-[15px] font-semibold text-jolu-text">来源分布</span>
            <span className="text-note text-jolu-muted">今日请求</span>
          </header>
          <div className="flex flex-1 flex-col gap-3.5 px-5 pb-4">
            {distribution.length === 0 && <p className="py-4 text-meta text-jolu-muted">流水里还没有带来源的请求。</p>}
            {distribution.slice(0, 5).map((entry) => (
              <div key={entry.source} className="flex flex-col gap-1.5">
                <div className="flex items-center gap-2 text-meta">
                  <span className="min-w-0 truncate font-medium text-jolu-text" title={entry.source}>{entry.source}</span>
                  <span className={cn("shrink-0 text-note", entry.failed > 0 ? "text-jolu-danger" : "text-jolu-muted")}>{entry.failed > 0 ? `${entry.failed} 失败` : "0 失败"}</span>
                  <span className="flex-1" />
                  <span className="shrink-0 tabular-nums text-jolu-text">{entry.total}</span>
                  <span className="w-11 shrink-0 text-right tabular-nums text-jolu-muted">{entry.share === null ? "—" : `${(entry.share * 100).toFixed(1)}%`}</span>
                </div>
                <div className="h-1.5 overflow-hidden rounded-full bg-jolu-track">
                  <div className="h-full rounded-full bg-jolu-on" style={{ width: `${Math.round((entry.share ?? 0) * 100)}%` }} />
                </div>
              </div>
            ))}
          </div>
        </section>
      </div>

      <div className="grid grid-cols-[1fr_380px] gap-4">
        <section className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
          <header className="flex items-center gap-2.5 px-5 py-3">
            <span className="text-[15px] font-semibold text-jolu-text">请求活动</span>
            {running.length > 0 && <span className="rounded-md bg-jolu-selected px-1.5 py-0.5 text-note font-semibold text-jolu-on">{running.length} 进行中</span>}
            <span className="flex-1" />
            <button type="button" onClick={onOpenLog} className="text-meta text-jolu-muted outline-none hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent">
              全部流水 →
            </button>
          </header>
          <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)_76px_88px] gap-3 bg-jolu-side px-5 py-2 text-note text-jolu-muted">
            <span>工具 / 会话</span><span>请求模型 / 实际去向</span><span>结果</span><span>用时</span>
          </div>
          {running.length === 0 && recent.length === 0 && (
            <p className="border-t border-jolu-table-divider px-5 py-6 text-center text-meta text-jolu-muted">还没有路由请求。</p>
          )}
          {running.slice(0, 2).map((view) => (
            <div key={view.key} className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)_76px_88px] items-center gap-3 border-t border-jolu-table-divider bg-jolu-selected/40 px-5 py-2.5 text-meta">
              <span className="flex min-w-0 flex-col">
                <span className="truncate text-jolu-text">{agentLabel(view.session.agent ?? view.session.platform)}</span>
                <span className="truncate text-note text-jolu-muted">{view.session.title || view.session.session_id}</span>
              </span>
              <span className="flex min-w-0 flex-col">
                <span className="truncate font-mono text-note text-jolu-text">{view.session.links[0]?.requested_model ?? "—"}</span>
                <span className="truncate text-note text-jolu-muted">{view.session.links[0]?.upstream_model ?? view.session.links[0]?.source ?? "路由中"}</span>
              </span>
              <span className="text-note font-medium text-jolu-on">处理中</span>
              <span className="text-note tabular-nums text-jolu-muted">已持续</span>
            </div>
          ))}
          {recent.map((route) => (
            <div key={route.id ?? `${route.ts}-${route.requested_model}`} className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)_76px_88px] items-center gap-3 border-t border-jolu-table-divider px-5 py-2.5 text-meta">
              <span className="flex min-w-0 flex-col">
                <span className="truncate text-jolu-text">{route.agent ? agentLabel(route.agent) : "—"}</span>
                <span className="truncate text-note tabular-nums text-jolu-muted">{clock(route.ts)}</span>
              </span>
              <span className="flex min-w-0 flex-col">
                <span className="truncate font-mono text-note text-jolu-text" title={route.requested_model}>{route.requested_model}</span>
                <span className="truncate text-note text-jolu-muted">{route.source ?? "未路由"}</span>
              </span>
              <span className={cn("text-note", route.result === "ok" ? "text-jolu-muted" : route.result === "rejected" ? "text-jolu-muted" : "text-jolu-danger")}>
                {resultText(route)}
              </span>
              <span className="text-note tabular-nums text-jolu-muted">{route.duration_ms != null ? formatLatency(route.duration_ms) : "—"}</span>
            </div>
          ))}
        </section>

        <section className="flex flex-col rounded-xl border border-jolu-border">
          <header className="flex items-center gap-2.5 px-5 py-3">
            <span className="text-[15px] font-semibold text-jolu-text">需要关注</span>
            {attention.length > 0 && <span className="rounded-md bg-jolu-danger-soft px-1.5 py-0.5 text-note font-semibold text-jolu-danger">{attention.length} 项</span>}
          </header>
          <div className="flex flex-1 flex-col px-5 pb-4">
            {attention.length === 0 && <p className="py-4 text-meta text-jolu-muted">一切正常：没有漂移的接入、没有新装的工具、没有集中限流。</p>}
            {attention.slice(0, 3).map((item) => (
              <div key={item.key} className="flex flex-col gap-1 border-b border-jolu-table-divider py-2.5 last:border-b-0">
                <span className="flex items-center gap-2">
                  {item.kind === "rate"
                    ? <CircleAlert aria-hidden className="size-3.5 shrink-0 text-jolu-danger" />
                    : <Wrench aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />}
                  <span className="min-w-0 flex-1 truncate text-meta font-medium text-jolu-text">{item.title}</span>
                </span>
                <span className="flex items-end gap-2 pl-5">
                  <span className="min-w-0 flex-1 text-note leading-relaxed text-jolu-muted">{item.detail}</span>
                  <button type="button" onClick={item.onAction} className="shrink-0 text-note font-medium text-jolu-on outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">
                    {item.action} →
                  </button>
                </span>
              </div>
            ))}
          </div>
        </section>
      </div>

      <footer className="flex items-center justify-between text-note text-jolu-muted">
        <span>统计覆盖今日 00:00 起；趋势、来源分布与响应头分位来自路由流水（只保留最近约 1,000 行，是其中可见的部分）</span>
        <span className="shrink-0">{sources} 个来源　{models} 个模型　{groups} 个分组　{middlewares ?? "—"} 个中间件</span>
      </footer>
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
        tone === "danger" ? "text-jolu-danger" : tone === "ok" ? "text-jolu-on" : tone === "muted" ? "text-jolu-text" : "text-jolu-text",
      )}>{value}</span>
      {action && onAction ? (
        <button type="button" onClick={onAction} className="self-start text-note font-medium text-jolu-danger outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">{action}</button>
      ) : (
        <span className="truncate text-note text-jolu-muted" title={note}>{note}</span>
      )}
    </div>
  )
}

function Legend({ colorClass, label }: { colorClass: string; label: string }) {
  return (
    <span className="flex items-center gap-1.5 text-note text-jolu-muted">
      <span aria-hidden className={cn("size-2.5 rounded-[2px]", colorClass)} />
      {label}
    </span>
  )
}

function resultText(route: RouteLogDto): string {
  if (route.result === "ok") return "成功"
  if (route.result === "rejected") return "拒绝"
  return route.status_code ? `${route.status_code} 失败` : "失败"
}

function dayRangeLabel(nowMs: number): string {
  const format = (ms: number) => new Date(ms).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false })
  return `00:00 ~ ${format(nowMs)}`
}

function clock(ts: number): string {
  return new Date(ts).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false })
}
