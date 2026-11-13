import { ArrowRight, ArrowUpRight, Boxes, GitFork, Layers, Sparkles, Terminal, TriangleAlert } from "lucide-react"
import { cn } from "@/lib/utils"
import type { DayOverviewDto, RouteLogDto } from "@/lib/gatewayApi"
import type { SessionView } from "@/lib/gatewaySessions"
import { agentLabel } from "@/lib/gatewayPage"

export interface AttentionItem {
  key: string
  kind: "agent" | "group" | "tool"
  title: string
  detail: string
  action: string
  onAction: () => void
}

/**
 * 模型路由 · 概览（画板 V5-3「瘦身」）：四项指标 → 请求怎么走 → 需要你处理（没有异常时不出现）→
 * 正在进行的会话 / 最近路由。Token 指标的数据源还没有按「是否经模型路由」归账，标「待确认」。
 */
export function RoutingOverview({ day, agents, middlewares, groups, sources, models, attention, sessions, routes, onOpenAgents, onOpenSessions, onOpenLog }: {
  day: DayOverviewDto | null
  agents: number
  middlewares: number | null
  groups: number
  sources: number
  models: number
  attention: AttentionItem[]
  sessions: SessionView[]
  routes: RouteLogDto[]
  onOpenAgents: () => void
  onOpenSessions: () => void
  onOpenLog: () => void
}) {
  const lower = day?.partial ? "（下限）" : ""
  const change = day && day.yesterday > 0 ? Math.round(((day.today - day.yesterday) / day.yesterday) * 100) : null
  const running = sessions.filter((view) => view.running).slice(0, 3)
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto px-7 py-5">
      <div className="flex items-baseline gap-2">
        <h2 className="text-[17px] font-semibold text-jolu-text">概览</h2>
        <span className="text-meta text-jolu-muted">今天 · 本机统计</span>
      </div>

      <div className="flex border-y border-jolu-table-divider py-3.5">
        <Metric label="今日请求" value={day ? day.today.toLocaleString("zh-CN") : "—"} note={day ? (change === null ? `昨天 ${day.yesterday}${lower}` : `较昨天 ${change >= 0 ? "+" : ""}${change}%${lower}`) : "读取中"} />
        <Metric label="改道" value={day ? String(day.reroutes) : "—"} note="失败后换下一个成员" />
        <Metric label="Agent 收到的错误" value={day ? String(day.errors) : "—"} tone={day && day.errors > 0 ? "danger" : undefined}
          note={day?.last_error ? `最近：${clock(day.last_error.ts)} ${day.last_error.requested_model} · ${day.last_error.status_code ?? day.last_error.result}` : "今天没有"} />
        <Metric label="Token" value="—" note="经模型路由的用量未单独归账（待确认）" />
      </div>

      <section className="flex flex-col gap-2.5">
        <div className="flex items-center gap-2">
          <span className="text-[13px] font-semibold text-jolu-text">请求怎么走</span>
        </div>
        <div className="flex items-center rounded-xl border border-jolu-border bg-jolu-bg px-3.5 py-3">
          <Station icon={Terminal} title={`${agents} 个 Agent`} note="在 Agent 页管理 ›" onClick={onOpenAgents} />
          <ArrowRight aria-hidden className="size-3.5 shrink-0 text-jolu-border" />
          <Station icon={Layers} title={`中间件 ${middlewares ?? "—"}`} note="选路前改写请求" />
          <ArrowRight aria-hidden className="size-3.5 shrink-0 text-jolu-border" />
          <Station icon={GitFork} title={`${groups} 个分组`} note="+ 来源/模型直连" />
          <ArrowRight aria-hidden className="size-3.5 shrink-0 text-jolu-border" />
          <Station icon={Boxes} title={`${sources} 个来源`} note={`${models} 个模型`} />
        </div>
      </section>

      {attention.length > 0 && (
        <section className="flex flex-col gap-2.5">
          <div className="flex items-center gap-2">
            <span className="text-[13px] font-semibold text-jolu-text">需要你处理</span>
            <span className="rounded-full bg-jolu-danger-soft px-1.5 text-note font-semibold text-jolu-danger">{attention.length}</span>
          </div>
          <div className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
            {attention.map((item, index) => {
              const Icon = item.kind === "tool" ? Sparkles : item.kind === "group" ? GitFork : TriangleAlert
              return (
                <div key={item.key} className={cn("flex items-center gap-3 px-3.5 py-3", index > 0 && "border-t border-jolu-table-divider")}>
                  <span className={cn("flex size-[30px] shrink-0 items-center justify-center rounded-lg", item.kind === "tool" ? "bg-jolu-selected" : "bg-jolu-danger-soft")}>
                    <Icon aria-hidden className={cn("size-3.5", item.kind === "tool" ? "text-jolu-on" : "text-jolu-danger")} />
                  </span>
                  <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className="truncate text-meta font-medium text-jolu-text">{item.title}</span>
                    <span className="truncate text-note text-jolu-muted">{item.detail}</span>
                  </span>
                  <button type="button" onClick={item.onAction}
                    className="flex h-7 shrink-0 items-center gap-1 rounded-md border border-jolu-border px-2.5 text-meta text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
                    {item.action}
                    {item.kind !== "group" && <ArrowUpRight aria-hidden className="size-3 text-jolu-muted" />}
                  </button>
                </div>
              )
            })}
          </div>
        </section>
      )}

      <div className="grid grid-cols-2 gap-4">
        <section className="flex min-w-0 flex-col gap-2">
          <div className="flex items-center justify-between">
            <span className="text-[13px] font-semibold text-jolu-text">正在进行的会话</span>
            <button type="button" onClick={onOpenSessions} className="text-meta text-jolu-muted outline-none hover:text-jolu-text">会话线程 ›</button>
          </div>
          {running.length === 0 && <p className="rounded-lg bg-jolu-side px-3 py-2.5 text-meta text-jolu-muted">现在没有经模型路由的会话在跑</p>}
          {running.map((view) => (
            <div key={view.key} className="flex items-center gap-2.5 rounded-lg border border-jolu-border px-3 py-2">
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="truncate text-meta text-jolu-text">{view.session.title || view.session.session_id}</span>
                <span className="truncate text-note text-jolu-muted">{agentLabel(view.session.agent ?? view.session.platform)}{view.session.links[0]?.source ? ` · ${view.session.links[0].source}` : ""}</span>
              </span>
              <span className="shrink-0 text-note font-medium text-jolu-on">进行中</span>
            </div>
          ))}
        </section>
        <section className="flex min-w-0 flex-col gap-2">
          <div className="flex items-center justify-between">
            <span className="text-[13px] font-semibold text-jolu-text">最近路由</span>
            <button type="button" onClick={onOpenLog} className="text-meta text-jolu-muted outline-none hover:text-jolu-text">路由流水 ›</button>
          </div>
          {routes.length === 0 && <p className="rounded-lg bg-jolu-side px-3 py-2.5 text-meta text-jolu-muted">还没有路由记录</p>}
          {routes.slice(0, 3).map((route) => (
            <div key={route.id ?? route.ts} className="flex items-center gap-2.5 rounded-lg border border-jolu-border px-3 py-2 text-meta">
              <span className="shrink-0 text-jolu-muted">{clock(route.ts)}</span>
              <span className="min-w-0 flex-1 truncate font-mono text-note text-jolu-text">{route.requested_model}</span>
              <span className="max-w-[96px] shrink-0 truncate text-jolu-muted">{route.source ?? "—"}</span>
              <span className={cn("shrink-0", route.result === "ok" ? "text-jolu-muted" : "text-jolu-danger")}>
                {route.status_code ?? route.result}{route.duration_ms != null ? ` · ${(route.duration_ms / 1000).toFixed(1)}s` : ""}
              </span>
            </div>
          ))}
        </section>
      </div>
    </div>
  )
}

function clock(ts: number): string {
  return new Date(ts).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })
}

function Metric({ label, value, note, tone }: { label: string; value: string; note: string; tone?: "danger" }) {
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-[3px] px-1">
      <span className="text-note text-jolu-muted">{label}</span>
      <span className={cn("text-[22px] font-semibold", tone === "danger" ? "text-jolu-danger" : "text-jolu-text")}>{value}</span>
      <span className="truncate text-note text-jolu-muted" title={note}>{note}</span>
    </div>
  )
}

function Station({ icon: Icon, title, note, onClick }: { icon: React.ElementType; title: string; note: string; onClick?: () => void }) {
  const body = (
    <>
      <span className="flex size-[30px] shrink-0 items-center justify-center rounded-lg bg-jolu-side"><Icon aria-hidden className="size-3.5 text-jolu-muted" /></span>
      <span className="flex min-w-0 flex-col gap-0.5 text-left">
        <span className="truncate text-meta font-medium text-jolu-text">{title}</span>
        <span className="truncate text-note text-jolu-muted">{note}</span>
      </span>
    </>
  )
  return onClick
    ? <button type="button" onClick={onClick} className="flex min-w-0 flex-1 items-center gap-2.5 rounded-lg px-2 py-1.5 outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">{body}</button>
    : <div className="flex min-w-0 flex-1 items-center gap-2.5 px-2 py-1.5">{body}</div>
}
