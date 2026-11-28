import { useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { isTauri, type StatsOverviewDto } from "@/lib/api"
import type { ContextSessionRowDto, DayOverviewDto, GatewayStatusDto, RouteLogDto } from "@/lib/gatewayApi"
import type { SessionView } from "@/lib/gatewaySessions"
import { relativeTime } from "@/lib/gatewaySessions"
import { agentLabel } from "@/lib/gatewayPage"
import { sanitizePortInput } from "@/lib/gatewayPort"
import {
  finalAttempts, formatLatency, headerLatency, hourlyTrend, overviewRange, OVERVIEW_RANGES, successRate,
  type OverviewRange,
} from "@/lib/gatewayOverview"
import { activeSessionRows, filterRows, STATUS_FILTERS, statusCell, type ActiveSessionRow, type StatusFilter } from "@/lib/contextSessions"
import { formatResetAt, quotaTitle } from "@/lib/accountDisplay"
import { changeText, formatPercent, formatTokens } from "@/lib/usageStats"
import { PLATFORMS } from "@/lib/platforms"
import { useQuota } from "@/hooks/useQuota"
import { isMemberPaused, strategyText, type RoutingGroupDto } from "@/components/gateway/routingGroups"
import { AgentBadge } from "@/components/agents/AgentBadge"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { Icon } from "@/icons"
import { AnimatedNumber, TactileButton } from "@/motion"
import { cn } from "@/lib/utils"
import {
  DataValue, IconButton, Input, QuotaRing, Select, StatusPill, Switch, Tag, type StatusTone,
} from "@/ui"
import type { Connection } from "@/types"

export interface AttentionItem {
  key: string
  kind: "agent" | "tool" | "rate" | "context"
  title: string
  detail: string
  action: string
  onAction: () => void
}

const HOUR = 3_600_000
/** 请求趋势看最近 12 个小时 */
const TREND_HOURS = 12

/** R1-01 活跃会话 6 列网格（状态列 96px 容得下四字胶囊不溢出；窄屏折 4 列不错位） */
const OVERVIEW_SESSION_GRID =
  "grid-cols-[minmax(144px,1.25fr)_minmax(104px,1fr)_minmax(114px,1fr)_minmax(72px,0.72fr)_96px_18px] @max-[680px]:grid-cols-[minmax(0,1fr)_minmax(108px,0.9fr)_96px_18px]"

/**
 * 模型路由 · 概览 —— 设计稿 R1-01（浅色）/ R1-02（深色，同构，颜色全走令牌）。
 *
 * 四行严格对齐 R1-01：
 * 1. 本机路由大卡（含三层薄荷同心装饰环 + 120px 电源花瓣主按钮）+ 今日请求 + 今日 Token；
 * 2. 默认策略（直连主路由分组）+ 局域网共享 + 中间件链（含真实生效规则数）；
 * 3. 来源额度（84px 含平台 Logo 真实额度环）+ 需要关注 + 响应头耗时（P95 实位高亮）；
 * 4. 活跃会话表（奇数行浅底斑马纹）+ 路由分组（300px）。
 */
export function OverviewPanel(props: {
  status: GatewayStatusDto | null
  running: boolean
  wired: number
  day: DayOverviewDto | null
  todayStats: StatsOverviewDto | null
  activeMiddlewares: number | null
  connections: Connection[]
  defaultConnection: string | null
  range: OverviewRange["id"]
  onRange: (next: OverviewRange["id"]) => void
  sessions: SessionView[]
  inFlightCount?: number
  contexts: ContextSessionRowDto[] | null
  logRows: RouteLogDto[]
  nowMs: number
  updatedAt: number | null
  sources: number
  models: number
  groups: number
  routingGroups: RoutingGroupDto[]
  attention: AttentionItem[]
  busy: boolean
  sharedLan: boolean
  onSharedLan: (on: boolean) => void
  onSelectStrategy: (group: RoutingGroupDto, strategy: "usage" | "order" | "intent") => void
  onToggleRunning: () => void
  onInlinePortCommit: (raw: string) => void | Promise<void>
  onOpenPortDialog: () => void
  onOpenQuota: (connectionId: string | null) => void
  onOpenMiddleware: () => void
  onOpenGroups: () => void
  onRefresh: () => void
  onOpenLog: () => void
  onOpenContext: (platform: string, sessionId: string) => void
}) {
  const { day, todayStats, activeMiddlewares, connections, defaultConnection, range, onRange, sessions, contexts, logRows, nowMs, updatedAt, attention, busy, status } = props
  const [search, setSearch] = useState("")
  const [filter, setFilter] = useState<StatusFilter>("all")
  const partial = day?.partial ?? false
  const identityIncomplete = day?.identity_incomplete ?? false
  const uncertain = partial || identityIncomplete
  const limitNote = identityIncomplete ? "（身份不完整）" : partial ? "（记录不完整）" : ""
  const change = day && !uncertain && day.yesterday > 0 ? day.today - day.yesterday : null
  const rate = successRate(day?.today, day?.errors)
  const runningSessions = sessions.filter((view) => view.running)
  const trendStart = Math.floor(nowMs / HOUR) * HOUR - (TREND_HOURS - 1) * HOUR
  const trend = hourlyTrend(logRows, trendStart, nowMs)
  const peak = Math.max(0, ...trend.map((bucket) => bucket.ok + bucket.failed))
  const p50 = headerLatency(logRows, 0.5)
  const p95 = headerLatency(logRows, 0.95)
  const durations = finalAttempts(logRows).map((row) => row.duration_ms).filter((value): value is number => typeof value === "number" && value >= 0)
  const updated = updatedAt ? new Date(updatedAt).toLocaleTimeString("zh-CN", { hour12: false }) : null
  const rangeText = overviewRange(range)
  const rows = useMemo(() => activeSessionRows(sessions, contexts ?? []), [sessions, contexts])
  const shown = filterRows(rows, search, filter, agentLabel)

  // 今日 Token（真实本机统计，缺数据时保持未知态不伪造）
  const tokenTotal = todayStats ? (todayStats.tokens.total ?? todayStats.token_coverage.known_total) : null
  const tokenPartial = Boolean(todayStats && todayStats.tokens.total === null && tokenTotal !== null)
  const tokenCacheRate = todayStats ? (todayStats.cache_hit_rate ?? todayStats.token_coverage.cache_hit_rate) : null
  const tokenDelta = todayStats && !tokenPartial ? changeText(todayStats.tokens.total, todayStats.previous_total) : null
  const tokenSeries = useMemo(
    () => (todayStats?.trend.points ?? []).map((pt) => {
      const v = pt.total ?? ((pt.fresh_input ?? 0) + (pt.output ?? 0) + (pt.cache_read ?? 0) + (pt.cache_write ?? 0))
      return { value: Math.max(0, v), label: `${pt.label} · ${formatTokens(v)}` }
    }),
    [todayStats],
  )
  const tokenFormatted = tokenTotal !== null ? splitCompact(formatTokens(tokenTotal)) : null
  const tokenTitle = range === "today" ? "今日 Token" : range === "yesterday" ? "昨日 Token" : `${rangeText.label} Token`

  const reqNote = day
    ? [
        change === null ? `较${rangeText.compare} ${day.yesterday}${limitNote}` : `较${rangeText.compare} ${change >= 0 ? "+" : ""}${change}${limitNote}`,
        rate.value !== null ? `成功率 ${(rate.value * 100).toFixed(1)}%` : null,
      ].filter(Boolean).join(" · ")
    : "读取中"

  const primaryGroup = props.routingGroups[0] ?? null

  return (
    <div className="@container flex min-w-0 flex-col gap-3.5">
      {/* 轻量刷新条 */}
      <div className="flex h-8 items-center gap-2">
        <span className="flex items-center gap-1.5 text-[11px] text-jn-text-3">
          <span aria-hidden className={cn("size-1.5 rounded-full", props.running ? "bg-jn-primary" : "bg-jn-text-3")} />
          每 5 秒更新
        </span>
        {updated && <span className="text-[11px] tabular-nums text-jn-text-3">{updated} 更新</span>}
        <span className="min-w-0 flex-1" />
        <div className="w-[116px]">
          <Select variant="filter" width="full" label="统计范围" value={range}
            onChange={(value) => onRange(value as OverviewRange["id"])}
            options={OVERVIEW_RANGES.map((option) => ({ value: option.id, label: option.label }))} />
        </div>
        <IconButton icon="refresh" label="立即刷新" kind="surface" size="sm" disabled={busy} onClick={props.onRefresh} />
      </div>

      {/* 第一行：本机路由（506 比例） / 今日请求 / 今日 Token */}
      <div className="grid grid-cols-1 gap-3.5 @[900px]:grid-cols-[minmax(390px,1.72fr)_minmax(0,1fr)_minmax(0,1fr)]">
        <RouterCard status={status} running={props.running} wired={props.wired} sharedLan={props.sharedLan} day={day} busy={busy}
          handling={props.inFlightCount ?? runningSessions.length} groups={props.groups}
          onToggleRunning={props.onToggleRunning}
          onInlinePortCommit={props.onInlinePortCommit}
          onOpenPortDialog={props.onOpenPortDialog} />
        <TrendCard
          title={rangeText.period}
          icon="activity"
          numericValue={day ? day.today : null}
          formatNumber={(n) => Math.round(n).toLocaleString("zh-CN")}
          unit="次"
          note={reqNote}
          peak={`峰值 ${peak} / 时`}
          series={trend.map((b) => ({ value: b.ok + b.failed, label: `成功 ${b.ok}${b.failed ? ` · 失败 ${b.failed}` : ""}` }))}
        />
        <TrendCard
          title={tokenTitle}
          icon="blocks"
          displayValue={tokenFormatted?.value ?? null}
          unit={tokenFormatted?.unit ?? ""}
          note={
            tokenTotal === null
              ? "等待本机用量统计"
              : tokenPartial
                ? `已知部分小计 · ${todayStats?.token_coverage.known_requests ?? 0}/${todayStats?.requests ?? 0} 次`
                : tokenDelta
                  ? `较${rangeText.compare} ${tokenDelta}`
                  : "本机用量统计口径"
          }
          peak={tokenCacheRate !== null ? `缓存命中 ${formatPercent(tokenCacheRate)}` : "缓存命中 —"}
          series={tokenSeries}
          unknown={tokenTotal === null}
        />
      </div>

      {/* 第二行：默认策略 + 局域网共享 + 中间件链（与第一行 1.72 : 1 : 1 严格对齐） */}
      <div className="grid grid-cols-1 gap-3.5 @[900px]:grid-cols-[minmax(390px,1.72fr)_minmax(0,1fr)_minmax(0,1fr)]">
        <StrategyQuickCard
          group={primaryGroup}
          busy={busy}
          onSelect={(strategy) => primaryGroup && props.onSelectStrategy(primaryGroup, strategy)}
          onOpenGroups={props.onOpenGroups}
        />
        <section className="flex h-[68px] items-center gap-3 rounded-jn-card border border-jn-line bg-jn-surface pr-4 pl-[18px] shadow-jn-surface">
          <Icon name="globe" size="md" className="shrink-0 text-jn-text-2" />
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="truncate text-[14px] font-semibold text-jn-text">局域网共享</span>
            <span className={cn("truncate text-[12px]", props.sharedLan ? "text-jn-primary-ink" : "text-jn-text-3")}>
              {props.sharedLan ? "已开启 · 同网段设备可用 Key 访问" : "已关闭"}
            </span>
          </span>
          <Switch checked={props.sharedLan} disabled={busy} label="局域网共享" onChange={props.onSharedLan} />
        </section>
        <TactileButton variant="tile" type="button" onClick={props.onOpenMiddleware}
          className="jn-interactive flex h-[68px] items-center gap-3 rounded-jn-card border border-jn-line bg-jn-surface pr-4 pl-[18px] text-left shadow-jn-surface">
          <Icon name="sliders" size="md" className="shrink-0 text-jn-text-2" />
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="truncate text-[14px] font-semibold text-jn-text">中间件链</span>
            <span className={cn("truncate text-[12px]", activeMiddlewares && activeMiddlewares > 0 ? "text-jn-primary-ink" : "text-jn-text-3")}>
              {activeMiddlewares === null ? "查看规则与命中" : activeMiddlewares > 0 ? `${activeMiddlewares} 条规则生效` : "未启用规则"}
            </span>
          </span>
          <Icon name="next" size="sm" className="shrink-0 text-jn-text-3" />
        </TactileButton>
      </div>

      {/* 第三行：来源额度（246） / 需要关注（弹性） / 响应头耗时（224） */}
      <div className="grid grid-cols-1 gap-3.5 @[880px]:grid-cols-[246px_minmax(0,1fr)_224px]">
        <QuotaCard connections={connections} defaultConnection={defaultConnection} onOpen={props.onOpenQuota} />
        <AttentionCard attention={attention} />
        <LatencyCard p50={p50} p95={p95} samples={durations} />
      </div>

      {/* 第四行：活跃会话（弹性） / 路由分组（300） */}
      <div className="grid grid-cols-1 gap-3.5 @[880px]:grid-cols-[minmax(0,1fr)_300px]">
        <SessionsCard
          rows={rows} shown={shown} running={runningSessions.length} total={day?.today ?? logRows.length} failed={day?.errors ?? 0}
          search={search} onSearch={setSearch} filter={filter} onFilter={setFilter}
          contextsFailed={contexts === null} nowMs={nowMs} onOpenLog={props.onOpenLog} onOpenContext={props.onOpenContext}
        />
        <GroupsCard groups={props.routingGroups} onOpen={props.onOpenGroups} />
      </div>

      <footer className="flex items-center justify-between gap-4 text-[11px] text-jn-text-3">
        <span>上下文取最近一次请求，非会话累计用量；趋势与响应头分位来自路由流水（只保留最近约 1,000 行）</span>
        <span className="shrink-0">{props.sources} 个来源 · {props.models} 个模型 · {props.groups} 个分组</span>
      </footer>
    </div>
  )
}

function splitCompact(text: string): { value: string; unit: string } {
  const m = text.trim().match(/^([^\sA-Za-z]+)\s*([A-Za-z%]+)?$/)
  if (!m) return { value: text, unit: "" }
  return { value: m[1], unit: m[2] ?? "" }
}

/** 设计稿 R1-01 本机路由花瓣主按钮的精确轮廓路径（viewBox 0 0 120 120）。 */
const PETAL_PATH =
  "M119.36 60l-0.24 2.06-0.69 2.03-1.07 1.94-1.33 1.84-1.46 1.75-1.44 1.67-1.29 1.63-1.04 1.65-0.74 1.7-0.4 1.8-0.12 1.94 0.08 2.08 0.19 2.2 0.15 2.27-0.01 2.28-0.28 2.2-0.63 2.04-1.02 1.81-1.4 1.53-1.75 1.23-2.01 0.95-2.16 0.71-2.2 0.55-2.15 0.51-2.01 0.56-1.81 0.71-1.59 0.95-1.39 1.22-1.24 1.5-1.15 1.73-1.14 1.89-1.22 1.93-1.34 1.83-1.52 1.62-1.71 1.28-1.89 0.86-2.03 0.42-2.14-0.04-2.18-0.41-2.16-0.7-2.12-0.84-2.03-0.86-1.95-0.72-1.89-0.49-1.84-0.17-1.84 0.17-1.89 0.49-1.95 0.72-2.03 0.86-2.12 0.84-2.16 0.7-2.18 0.41-2.14 0.04-2.03-0.42-1.89-0.86-1.71-1.28-1.52-1.62-1.34-1.83-1.22-1.93-1.14-1.89-1.15-1.73-1.24-1.5-1.39-1.22-1.59-0.95-1.81-0.71-2.01-0.56-2.15-0.51-2.2-0.55-2.16-0.71-2.01-0.95-1.75-1.23-1.4-1.53-1.02-1.81-0.63-2.04-0.28-2.2-0.01-2.28 0.15-2.27 0.19-2.2 0.08-2.08-0.12-1.94-0.4-1.8-0.74-1.7-1.04-1.65-1.29-1.63-1.44-1.67-1.46-1.75-1.33-1.84-1.07-1.94-0.69-2.03-0.24-2.06 0.24-2.06 0.69-2.03 1.07-1.94 1.33-1.84 1.46-1.75 1.44-1.67 1.29-1.63 1.04-1.65 0.74-1.7 0.4-1.8 0.12-1.94-0.08-2.08-0.19-2.2-0.15-2.27 0.01-2.28 0.28-2.2 0.63-2.04 1.02-1.81 1.4-1.53 1.75-1.23 2.01-0.95 2.16-0.71 2.2-0.55 2.15-0.51 2.01-0.56 1.81-0.71 1.59-0.95 1.39-1.22 1.24-1.5 1.15-1.73 1.14-1.89 1.22-1.93 1.34-1.83 1.52-1.62 1.71-1.28 1.89-0.86 2.03-0.42 2.14 0.04 2.18 0.41 2.16 0.7 2.12 0.84 2.03 0.86 1.95 0.72 1.89 0.49 1.84 0.17 1.84-0.17 1.89-0.49 1.95-0.72 2.03-0.86 2.12-0.84 2.16-0.7 2.18-0.41 2.14-0.04 2.03 0.42 1.89 0.86 1.71 1.28 1.52 1.62 1.34 1.83 1.22 1.93 1.14 1.89 1.15 1.73 1.24 1.5 1.39 1.22 1.59 0.95 1.81 0.71 2.01 0.56 2.15 0.51 2.2 0.55 2.16 0.71 2.01 0.95 1.75 1.23 1.4 1.53 1.02 1.81 0.63 2.04 0.28 2.2 0.01 2.28-0.15 2.27-0.19 2.2-0.08 2.08 0.12 1.94 0.4 1.8 0.74 1.7 1.04 1.65 1.29 1.63 1.44 1.67 1.46 1.75 1.33 1.84 1.07 1.94 0.69 2.03 0.24 2.06z"

/** 本机路由大卡（R1-01 第 1 行左）：三层薄荷同心装饰环 + 地址/标签/运行数据 + 120px 电源花瓣主开关。 */
function RouterCard({ status, running, wired, sharedLan, day, busy, handling, groups, onToggleRunning, onInlinePortCommit, onOpenPortDialog }: {
  status: GatewayStatusDto | null
  running: boolean
  wired: number
  sharedLan: boolean
  day: DayOverviewDto | null
  busy: boolean
  handling: number
  groups: number
  onToggleRunning: () => void
  onInlinePortCommit: (raw: string) => void | Promise<void>
  onOpenPortDialog: () => void
}) {
  const [editingPort, setEditingPort] = useState(false)
  const [portDraft, setPortDraft] = useState("")
  const inputRef = useRef<HTMLInputElement | null>(null)
  const committedRef = useRef(false)

  useEffect(() => {
    if (!editingPort) return
    const input = inputRef.current
    if (!input) return
    input.focus()
    input.select()
  }, [editingPort])

  const beginPortEdit = () => {
    if (busy || status?.port == null) return
    // 方案 A：已有工具接入时直接打开弹窗提示将先还原工具并停止中转，避免失焦误触
    if (wired > 0) {
      onOpenPortDialog()
      return
    }
    committedRef.current = false
    setPortDraft(String(status.port))
    setEditingPort(true)
  }

  const finishPortEdit = () => {
    if (committedRef.current) return
    committedRef.current = true
    setEditingPort(false)
    const trimmed = portDraft.trim()
    if (!trimmed || trimmed === String(status?.port ?? "")) return
    void onInlinePortCommit(trimmed)
  }

  return (
    <section className="relative flex min-h-[212px] gap-4 overflow-hidden rounded-jn-card border border-jn-line bg-jn-surface px-[22px] py-5 shadow-jn-surface">
      {/* R1-01 标志性同心装饰环（薄荷底 340 + 双环 300 / 250），居中落在右侧花瓣按钮背后 */}
      <div aria-hidden className="pointer-events-none absolute top-1/2 right-[-74px] size-[340px] -translate-y-1/2">
        <span className="absolute inset-0 rounded-full bg-jn-primary-wash" />
        <span className="absolute inset-[20px] rounded-full border border-jn-mint-2" />
        <span className="absolute inset-[45px] rounded-full border-[1.5px] border-jn-mint-2" />
      </div>

      <div className="relative z-10 flex min-w-0 flex-1 flex-col justify-between gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <Icon name="navGateway" size="md" className="text-jn-text-2" />
          <span className="text-[14px] font-semibold text-jn-text">本机路由</span>
          <StatusPill tone={running ? "active" : "neutral"}>{running ? "正在中转" : "已停止"}</StatusPill>
        </div>
        <div className="flex flex-col gap-2.5">
          <div className="flex items-end gap-0.5">
            <span className="tnum text-[30px] leading-none font-bold tracking-[-0.02em] text-jn-text">127.0.0.1</span>
            <span className="tnum pb-0.5 text-[20px] leading-none font-semibold text-jn-text-3">:</span>
            {editingPort ? (
              <input
                ref={inputRef}
                type="text"
                inputMode="numeric"
                aria-label="本机路由监听端口"
                value={portDraft}
                onChange={(event) => setPortDraft(sanitizePortInput(event.target.value))}
                onBlur={finishPortEdit}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault()
                    finishPortEdit()
                  } else if (event.key === "Escape") {
                    event.preventDefault()
                    committedRef.current = true
                    setEditingPort(false)
                  }
                }}
                className="tnum w-[6ch] rounded-[6px] border border-jn-primary bg-jn-well px-1.5 py-0.5 font-mono text-[20px] leading-none font-semibold text-jn-text outline-none"
              />
            ) : (
              <button
                type="button"
                disabled={busy || status?.port == null}
                title={wired > 0 ? "点击修改监听端口（需先停止中转并还原已接入工具）" : "点击修改监听端口"}
                aria-label={`监听端口 ${status?.port ?? "未知"}，点击修改`}
                onClick={beginPortEdit}
                className="tnum -mx-0.5 cursor-pointer rounded-[6px] px-1 pb-0.5 text-[20px] leading-none font-semibold text-jn-text-3 transition-colors hover:bg-jn-well hover:text-jn-text hover:underline hover:decoration-dashed hover:decoration-jn-text-3/60 hover:underline-offset-4 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jn-focus disabled:cursor-default disabled:hover:bg-transparent disabled:hover:no-underline"
              >
                {status?.port ?? "—"}
              </button>
            )}
          </div>
          <div className="flex flex-wrap items-center gap-1.5">
            <Tag>{wired} 个工具已接入</Tag>
            <Tag>{sharedLan ? "局域网共享" : "仅本机访问"}</Tag>
            <Tag>{groups} 个分组</Tag>
          </div>
        </div>
        <div className="flex flex-wrap items-end gap-x-6 gap-y-2">
          <MiniStat label="运行时长" value={<DataValue state="unknown" note="未记录" />} />
          <MiniStat label="正在处理" value={<span className="tnum font-mono text-[14px] font-semibold text-jn-text">{handling} 个请求</span>} />
          <MiniStat label="今日改道" value={day ? <span className="tnum font-mono text-[14px] font-semibold text-jn-text">{day.reroutes} 次</span> : <DataValue state="unknown" />} />
        </div>
      </div>

      <div className="relative z-10 flex w-[138px] shrink-0 flex-col items-center justify-center gap-2.5">
        <TactileButton
          variant="petal"
          type="button"
          aria-label={running ? "停止中转" : "开启中转"}
          disabled={busy}
          onClick={onToggleRunning}
          className="group relative flex size-[120px] items-center justify-center rounded-full outline-none disabled:pointer-events-none disabled:opacity-45 focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-jn-focus"
        >
          <svg
            viewBox="0 0 120 120"
            aria-hidden
            className={cn(
              "size-full",
              running && "jn-petal-spin drop-shadow-[0_10px_24px_var(--jn-glow)]",
            )}
          >
            <path
              d={PETAL_PATH}
              className={cn(
                "transition-colors duration-200",
                running ? "fill-jn-primary group-hover:fill-jn-primary-hover" : "fill-jn-primary-soft group-hover:fill-jn-mint-2",
              )}
            />
          </svg>
          <span className={cn("absolute inset-0 flex items-center justify-center", running ? "text-jn-on-primary" : "text-jn-primary-ink")}>
            <Icon name="power" className="size-8" />
          </span>
        </TactileButton>
        <span className="text-[12px] font-medium text-jn-text-2">{running ? "停止中转" : "开启中转"}</span>
      </div>
    </section>
  )
}

function MiniStat({ label, value }: { label: string; value: ReactNode }) {
  return (
    <span className="flex flex-col gap-[3px]">
      <span className="text-[12px] text-jn-text-3">{label}</span>
      {value}
    </span>
  )
}

/** R1-01 第 2 行左：默认策略快选条（直连主路由分组 `group/<slug>`，无分组时引导创建）。 */
const QUICK_STRATEGIES: readonly { key: "usage" | "order" | "intent"; label: string; matches: readonly string[] }[] = [
  { key: "usage", label: "额度优先", matches: ["usage", "smart", "pacing"] },
  { key: "order", label: "按顺序", matches: ["order", "round_robin"] },
  { key: "intent", label: "按意图", matches: ["intent"] },
]

function StrategyQuickCard({ group, busy, onSelect, onOpenGroups }: {
  group: RoutingGroupDto | null
  busy: boolean
  onSelect: (strategy: "usage" | "order" | "intent") => void
  onOpenGroups: () => void
}) {
  return (
    <section
      className="flex h-[68px] items-center gap-2 rounded-jn-card border border-jn-line bg-jn-surface pr-2.5 pl-[18px] shadow-jn-surface"
      title={group ? `当前作用于主分组 group/${group.slug}` : "创建路由分组后可快捷切换策略"}
    >
      <Icon name="sliders" size="md" className="shrink-0 text-jn-text-2" />
      <div className="flex shrink-0 flex-col gap-px pr-1.5">
        <span className="text-[14px] font-semibold whitespace-nowrap text-jn-text">默认策略</span>
        {group && <span className="max-w-[96px] truncate font-mono text-[10.5px] text-jn-text-3">group/{group.slug}</span>}
      </div>
      {group ? (
        <div role="radiogroup" aria-label={`group/${group.slug} 路由策略`} className="flex min-w-0 flex-1 items-center gap-1.5">
          {QUICK_STRATEGIES.map((item) => {
            const active = item.matches.includes(group.strategy)
            return (
              <TactileButton
                key={item.key}
                variant="tile"
                type="button"
                role="radio"
                aria-checked={active}
                disabled={busy}
                onClick={() => !active && onSelect(item.key)}
                className={cn(
                  "jn-interactive flex h-[48px] min-w-0 flex-1 items-center justify-center rounded-[18px] px-2.5 text-[13px] font-semibold whitespace-nowrap disabled:pointer-events-none disabled:opacity-45",
                  active
                    ? "bg-jn-primary text-jn-on-primary shadow-[0_4px_10px_var(--jn-glow)]"
                    : "bg-jn-well text-jn-text-2 hover:bg-jn-track/70 hover:text-jn-text",
                )}
              >
                {item.label}
              </TactileButton>
            )
          })}
        </div>
      ) : (
        <TactileButton
          variant="button"
          type="button"
          onClick={onOpenGroups}
          className="jn-interactive ml-auto flex h-9 items-center gap-1.5 rounded-full bg-jn-well px-3.5 text-xs font-medium text-jn-text-2 hover:bg-jn-track hover:text-jn-text"
        >
          <span>去新建路由分组</span>
          <Icon name="next" size="xs" />
        </TactileButton>
      )}
    </section>
  )
}

/** 今日请求 / 今日 Token（R1-01 第 1 行中、右）：大数值 + 对比 + 平滑渐变面积图（600ms 描线）。 */
function TrendCard({ title, icon, numericValue, formatNumber, displayValue, unit, note, peak, series, unknown = false }: {
  title: string
  icon: "activity" | "blocks"
  numericValue?: number | null
  formatNumber?: (n: number) => string
  displayValue?: string | null
  unit: string
  note: string
  peak: string
  series: { value: number; label: string }[]
  unknown?: boolean
}) {
  const [hoverIdx, setHoverIdx] = useState<number | null>(null)
  const totals = series.map((b) => b.value)
  const max = Math.max(1, ...totals)
  const W = 256
  const H = 74
  const padTop = 10
  const padBottom = 6
  const usableH = H - padTop - padBottom

  const pts = totals.map((v, i) => ({
    x: totals.length > 1 ? (i / (totals.length - 1)) * W : W / 2,
    y: padTop + usableH - (v / max) * usableH,
  }))

  let linePath = ""
  if (pts.length > 1) {
    linePath = `M ${pts[0].x.toFixed(1)} ${pts[0].y.toFixed(1)}`
    for (let i = 0; i < pts.length - 1; i++) {
      const p0 = pts[Math.max(0, i - 1)]
      const p1 = pts[i]
      const p2 = pts[i + 1]
      const p3 = pts[Math.min(pts.length - 1, i + 2)]
      const cp1x = p1.x + (p2.x - p0.x) / 6
      const cp1y = Math.max(padTop, Math.min(H - padBottom, p1.y + (p2.y - p0.y) / 6))
      const cp2x = p2.x - (p3.x - p1.x) / 6
      const cp2y = Math.max(padTop, Math.min(H - padBottom, p2.y - (p3.y - p1.y) / 6))
      linePath += ` C ${cp1x.toFixed(1)} ${cp1y.toFixed(1)}, ${cp2x.toFixed(1)} ${cp2y.toFixed(1)}, ${p2.x.toFixed(1)} ${p2.y.toFixed(1)}`
    }
  }
  const areaPath = pts.length > 1 ? `${linePath} L ${W} ${H} L 0 ${H} Z` : ""
  const activeBucket = hoverIdx !== null ? series[hoverIdx] : null
  const gradId = `jn-area-${icon}`

  return (
    <section className="flex h-[212px] flex-col gap-1.5 overflow-hidden rounded-jn-card border border-jn-line bg-jn-surface px-5 pt-5 pb-0 shadow-jn-surface">
      <div className="flex items-center gap-1.5">
        <Icon name={icon} size="sm" className="text-jn-text-2" />
        <span className="shrink-0 text-[14px] font-semibold whitespace-nowrap text-jn-text">{title}</span>
        <span className="min-w-0 flex-1" />
        <span className="truncate text-[11.5px] whitespace-nowrap text-jn-text-3">
          {activeBucket ? activeBucket.label : peak}
        </span>
      </div>
      <div className="flex items-end gap-1 pt-1.5">
        <span className={cn("tnum text-[36px] leading-none font-bold tracking-[-0.025em]", unknown ? "text-jn-text-3" : "text-jn-text")}>
          {unknown
            ? "—"
            : numericValue != null
              ? <AnimatedNumber value={numericValue} format={formatNumber ?? ((n) => Math.round(n).toLocaleString("zh-CN"))} />
              : (displayValue ?? "—")}
        </span>
        {unit && <span className="pb-1 text-[15px] font-semibold text-jn-text-2">{unit}</span>}
      </div>
      <span className="truncate text-[12px] text-jn-text-3">{note}</span>
      <div className="-mx-5 mt-auto flex h-[74px] items-end">
        {unknown || series.length < 2 ? (
          <span className="mx-5 mb-3 flex h-[60px] w-full items-center justify-center rounded-[12px] border border-dashed border-jn-line text-[11px] text-jn-text-3">暂无数据</span>
        ) : (
          <svg
            viewBox={`0 0 ${W} ${H}`}
            preserveAspectRatio="none"
            className="h-full w-full"
            onMouseLeave={() => setHoverIdx(null)}
          >
            <defs>
              <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="var(--jn-area-top)" />
                <stop offset="100%" stopColor="var(--jn-area-end)" />
              </linearGradient>
            </defs>
            {areaPath && <path d={areaPath} fill={`url(#${gradId})`} />}
            {linePath && (
              <path
                d={linePath}
                pathLength={1}
                strokeDasharray="1"
                fill="none"
                stroke="var(--jn-primary)"
                strokeWidth="2.2"
                strokeLinecap="round"
                strokeLinejoin="round"
                vectorEffect="non-scaling-stroke"
                className="jn-stroke-draw"
              />
            )}
            {pts.map((pt, index) => (
              <g key={index} onMouseEnter={() => setHoverIdx(index)}>
                <rect
                  x={index === 0 ? 0 : (pts[index - 1].x + pt.x) / 2}
                  y={0}
                  width={index === 0 || index === pts.length - 1 ? W / Math.max(1, pts.length) : (pts[index + 1].x - pts[index - 1].x) / 2}
                  height={H}
                  fill="transparent"
                />
                {hoverIdx === index && (
                  <circle cx={pt.x} cy={pt.y} r="3.5" fill="var(--jn-surface)" stroke="var(--jn-primary)" strokeWidth="2" vectorEffect="non-scaling-stroke" />
                )}
              </g>
            ))}
          </svg>
        )}
      </div>
    </section>
  )
}

/** 来源额度（R1-01 第 3 行左）：84px 大号额度环内嵌官方平台 Logo + 真实订阅窗口百分比与重置时间。 */
function QuotaCard({ connections, defaultConnection, onOpen }: {
  connections: Connection[]
  defaultConnection: string | null
  onOpen: (connectionId: string | null) => void
}) {
  const target = useMemo(() => {
    const auths = connections.filter((c) => c.kind === "auth" && c.status !== "paused")
    if (defaultConnection) {
      const def = auths.find((c) => c.id === defaultConnection)
      if (def) return def
    }
    return auths.find((c) => c.serve_enabled) ?? auths[0] ?? null
  }, [connections, defaultConnection])

  const quota = useQuota(target?.id ?? null, false, Boolean(isTauri && target))
  const win = quota.state?.state === "ok"
    ? quota.state.windows.find((w) => typeof w.used_percent === "number") ?? null
    : null
  const usedPercent = win && typeof win.used_percent === "number"
    ? Math.max(0, Math.min(100, Math.round(win.used_percent)))
    : null
  const platformName = target ? (PLATFORMS.find((p) => p.id === target.platformId)?.name ?? target.platformId) : null
  const windowLabel = win ? quotaTitle(win.key, win.window_name) : null
  const resetText = win ? formatResetAt(win.resets_at) : null

  return (
    <TactileButton
      variant="tile"
      type="button"
      onClick={() => onOpen(target?.id ?? null)}
      className="jn-interactive flex min-h-[186px] flex-col justify-between gap-3 rounded-jn-card border border-jn-line bg-jn-surface p-[18px] text-left shadow-jn-surface"
    >
      <span className="flex w-full items-center gap-1.5">
        <Icon name="gauge" size="sm" className="text-jn-text-2" />
        <span className="text-[14px] font-semibold text-jn-text">来源额度</span>
        <span className="min-w-0 flex-1" />
        <Icon name="next" size="sm" className="text-jn-text-3" />
      </span>

      <span className="flex items-center gap-3.5">
        <QuotaRing usedPercent={usedPercent} size="lg">
          {target ? <PlatformLogo platform={target.platformId} size={24} /> : undefined}
        </QuotaRing>
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          {usedPercent !== null ? (
            <span className={cn("tnum font-mono text-[24px] leading-none font-bold", usedPercent >= 85 ? "text-jn-warning" : "text-jn-text")}>
              {usedPercent}%
            </span>
          ) : (
            <span className="tnum font-mono text-[22px] leading-none font-bold text-jn-text-3">—</span>
          )}
          <span className="truncate text-[12px] font-semibold text-jn-text">
            {target ? `${platformName} · ${target.baseName}` : "暂无订阅来源"}
          </span>
          <span className="text-[11px] leading-[1.4] text-jn-text-3">
            {usedPercent !== null ? (
              <>
                {windowLabel ?? "主窗口"}
                <br />
                {resetText ?? "重置时间未知"}
              </>
            ) : target ? (
              quota.loading ? "正在读取额度…" : "暂未取得额度读数"
            ) : (
              "供应商页可连接订阅账号"
            )}
          </span>
        </span>
      </span>
    </TactileButton>
  )
}

/** 需要关注（R1-01 第 3 行中）：上下文接近窗口、工具漂移、来源限流。 */
function AttentionCard({ attention }: { attention: AttentionItem[] }) {
  const tone = (kind: AttentionItem["kind"]): { box: string; icon: "danger" | "warning" | "info" } =>
    kind === "context" ? { box: "bg-jn-danger-soft text-jn-danger", icon: "danger" }
      : kind === "rate" ? { box: "bg-jn-warning-soft text-jn-warning", icon: "warning" }
        : { box: "bg-jn-well text-jn-text-2", icon: "info" }
  return (
    <section className="flex min-h-[186px] flex-col gap-2.5 rounded-jn-card border border-jn-line bg-jn-surface p-[18px] shadow-jn-surface">
      <div className="flex items-center gap-1.5">
        <Icon name="danger" size="sm" className="text-jn-text-2" />
        <span className="text-[14px] font-semibold text-jn-text">需要关注</span>
        <span className="min-w-0 flex-1" />
        <span className="flex h-5 items-center rounded-full bg-jn-well px-2 text-[11px] font-bold text-jn-text-2">{attention.length}</span>
      </div>
      {attention.length === 0 && <p className="py-2 text-[12px] text-jn-text-3">一切正常：没有接近上限的上下文、没有漂移的接入、没有集中限流。</p>}
      {attention.slice(0, 3).map((item) => {
        const spec = tone(item.kind)
        return (
          <TactileButton key={item.key} variant="row" type="button" onClick={item.onAction} className="jn-interactive flex items-center gap-2.5 rounded-[12px] p-1 text-left hover:bg-jn-well">
            <span className={cn("flex size-[30px] shrink-0 items-center justify-center rounded-full", spec.box)}>
              <Icon name={spec.icon} size="sm" />
            </span>
            <span className="flex min-w-0 flex-1 flex-col gap-px">
              <span className="truncate text-[13px] font-semibold text-jn-text">{item.title}</span>
              <span className="truncate text-[11px] text-jn-text-3">{item.detail}</span>
            </span>
            <Icon name="next" size="sm" className="shrink-0 text-jn-text-3" />
          </TactileButton>
        )
      })}
    </section>
  )
}

/** 响应头耗时（R1-01 第 3 行右）：P50 + 12 柱分布直方图（峰值主色、P95 实位警示橙）+ P95 / 最慢。 */
function LatencyCard({ p50, p95, samples }: { p50: number | null; p95: number | null; samples: number[] }) {
  const edges = [100, 200, 350, 500, 800, 1200, 2000, 3000, 5000, 8000, 12000]
  const binOf = (v: number) => {
    const at = edges.findIndex((edge) => v < edge)
    return at === -1 ? edges.length : at
  }
  const bins = new Array(edges.length + 1).fill(0) as number[]
  for (const value of samples) bins[binOf(value)] += 1
  const max = Math.max(1, ...bins)
  const slowest = samples.length ? Math.max(...samples) : null
  const busiest = bins.indexOf(Math.max(...bins))
  const p95Bin = p95 !== null ? binOf(p95) : -1
  const p50Text = p50 === null ? "—" : formatLatency(p50)
  const p50Unit = p50 === null ? "无样本" : p50Text.endsWith("ms") ? "ms · P50" : "s · P50"
  const p50Value = p50Text.replace(/ms$/, "").replace(/s$/, "")

  return (
    <section className="flex min-h-[186px] flex-col gap-2.5 rounded-jn-card border border-jn-line bg-jn-surface p-[18px] shadow-jn-surface">
      <div className="flex items-center gap-1.5">
        <Icon name="activity" size="sm" className="text-jn-text-2" />
        <span className="text-[14px] font-semibold text-jn-text">响应头耗时</span>
      </div>
      <div className="flex items-end gap-1">
        <span className="tnum text-[30px] leading-none font-bold tracking-[-0.02em] text-jn-text">{p50Value}</span>
        <span className="pb-0.5 text-[13px] font-medium text-jn-text-2">{p50Unit}</span>
      </div>
      <div className="flex min-h-[58px] flex-1 items-end gap-1">
        {samples.length === 0 && <span className="w-full pb-2 text-center text-[11px] text-jn-text-3">流水里还没有耗时样本</span>}
        {samples.length > 0 && bins.map((count, index) => (
          <span key={index} className="min-w-0 flex-1" style={{ height: `${Math.max(8, Math.round((count / max) * 100))}%` }}>
            <span
              className={cn(
                "block h-full w-full rounded-t-[4px] rounded-b-[2px]",
                index === busiest
                  ? "bg-jn-primary"
                  : index === p95Bin && count > 0
                    ? "bg-jn-warning"
                    : "bg-jn-mint-2",
              )}
            />
          </span>
        ))}
      </div>
      <div className="flex items-center text-[11px] font-medium text-jn-text-3">
        <span>{p95 === null ? "P95 —" : `P95 ${formatLatency(p95)}`}</span>
        <span className="min-w-0 flex-1" />
        <span>{slowest === null ? "最慢 —" : `最慢 ${formatLatency(slowest)}`}</span>
      </div>
    </section>
  )
}

/** 活跃会话（R1-01 第 4 行左）：胶囊页签 + 搜索 + 奇数行浅底斑马纹表格。 */
function SessionsCard({ rows, shown, running, total, failed, search, onSearch, filter, onFilter, contextsFailed, nowMs, onOpenLog, onOpenContext }: {
  rows: ActiveSessionRow[]
  shown: ActiveSessionRow[]
  running: number
  total: number
  failed: number
  search: string
  onSearch: (value: string) => void
  filter: StatusFilter
  onFilter: (value: StatusFilter) => void
  contextsFailed: boolean
  nowMs: number
  onOpenLog: () => void
  onOpenContext: (platform: string, sessionId: string) => void
}) {
  return (
    <section className="@container flex min-w-0 flex-col gap-2.5 rounded-jn-card border border-jn-line bg-jn-surface px-3.5 pt-3.5 pb-2.5 shadow-jn-surface">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="flex h-8 items-center gap-1.5 rounded-full bg-jn-primary px-3.5 text-[13px] font-semibold text-jn-on-primary">
          <span className="tabular-nums font-bold">{running}</span> 活跃会话
        </span>
        <TactileButton variant="button" type="button" onClick={onOpenLog} className="jn-interactive flex h-8 items-center gap-1.5 rounded-full bg-jn-well px-3.5 text-[13px] font-medium text-jn-text-2 hover:bg-jn-track hover:text-jn-text">
          <span className="tabular-nums font-bold text-jn-text">{total}</span> 路由流水
        </TactileButton>
        <TactileButton variant="button" type="button" onClick={onOpenLog} className="jn-interactive flex h-8 items-center gap-1.5 rounded-full bg-jn-well px-3.5 text-[13px] font-medium text-jn-text-2 hover:bg-jn-track hover:text-jn-text">
          <span className="tabular-nums font-bold text-jn-text">{failed}</span> 失败请求
        </TactileButton>
        <span className="min-w-0 flex-1" />
        <Input size="md" icon="search" value={search} onChange={(event) => onSearch(event.target.value)} placeholder="搜索会话或项目" aria-label="搜索会话" className="h-8 w-[176px] shrink-0" />
        <div className="w-[106px] shrink-0">
          <Select variant="filter" width="full" label="按状态筛选" value={filter}
            onChange={(value) => onFilter(value as StatusFilter)}
            options={STATUS_FILTERS.map((option) => ({ value: option.id, label: option.label }))} />
        </div>
      </div>

      <div className={cn("grid h-[30px] items-center gap-3 px-2.5 text-[12px] font-medium text-jn-text-3", OVERVIEW_SESSION_GRID)}>
        <span>Agent / 会话</span>
        <span className="@max-[680px]:hidden">实际模型</span>
        <span>上下文</span>
        <span className="@max-[680px]:hidden">缓存读取</span>
        <span>状态</span>
        <span />
      </div>

      {shown.length === 0 && (
        <p className="rounded-[14px] bg-jn-well px-2.5 py-7 text-center text-[12px] text-jn-text-3">
          {rows.length === 0 ? "还没有经过模型路由的会话。" : "没有符合条件的会话。"}
        </p>
      )}
      <div className="flex flex-col gap-1">
        {shown.slice(0, 8).map((row) => (
          <SessionLine key={row.key} row={row} nowMs={nowMs} contextsFailed={contextsFailed} onOpen={() => onOpenContext(row.platform, row.sessionId)} />
        ))}
      </div>
      <p className="px-2.5 pt-0.5 text-[11px] text-jn-text-3">
        缓存读取已包含在输入中；未采集与 0 分开显示。{shown.length > 8 ? `　共 ${shown.length} 个会话，显示最近 8 个。` : ""}
      </p>
    </section>
  )
}

/** 路由分组（R1-01 第 4 行右 300px）：名称、策略 + 当前首成员（或已暂停灰态）、成员数。 */
function GroupsCard({ groups, onOpen }: { groups: RoutingGroupDto[]; onOpen: () => void }) {
  const now = Date.now()
  return (
    <section className="flex flex-col gap-2.5 rounded-jn-card border border-jn-line bg-jn-surface px-4 pt-[18px] pb-4 shadow-jn-surface">
      <div className="flex items-center gap-1.5">
        <Icon name="blocks" size="sm" className="text-jn-text-2" />
        <span className="text-[14px] font-semibold text-jn-text">路由分组</span>
        <span className="flex h-5 items-center rounded-full bg-jn-well px-2 text-[11px] font-bold text-jn-text-2">{groups.length}</span>
        <span className="min-w-0 flex-1" />
        <IconButton icon="next" label="打开路由分组" kind="ghost" size="sm" onClick={onOpen} />
      </div>
      {groups.length === 0 && <p className="py-2 text-[12px] text-jn-text-3">还没有路由分组。</p>}
      {groups.map((group) => {
        const activeMember = group.members.find((m) => !isMemberPaused(m, now))
        const allPaused = group.members.length > 0 && !activeMember
        const activeHint = allPaused
          ? "已暂停"
          : activeMember
            ? `正在用 ${activeMember.slug}`
            : "暂无成员"
        return (
          <TactileButton
            key={group.slug}
            variant="tile"
            type="button"
            onClick={onOpen}
            className="jn-interactive flex items-center gap-3 rounded-[18px] bg-jn-well p-3 text-left hover:bg-jn-primary-wash/60"
          >
            <span className={cn(
              "flex size-[38px] shrink-0 items-center justify-center rounded-[13px] text-[15px] font-bold",
              allPaused ? "bg-jn-track text-jn-text-3" : "bg-jn-primary-soft text-jn-primary-ink",
            )}>
              {(group.name.trim()[0] ?? "?").toUpperCase()}
            </span>
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className={cn("truncate font-mono text-[13px] font-semibold", allPaused ? "text-jn-text-2" : "text-jn-text")}>
                group/{group.slug}
              </span>
              <span className="truncate text-[11px] text-jn-text-3">
                {strategyText(group.strategy)} · {activeHint}
              </span>
            </span>
            <span className="shrink-0 text-[11px] font-medium text-jn-text-3">{group.members.length} 个</span>
          </TactileButton>
        )
      })}
    </section>
  )
}

/** 活跃会话一行（R1-01 奇数行浅底圆角 14px，图标白底反差，带平滑上下文进度条）。 */
function SessionLine({ row, nowMs, contextsFailed, onOpen }: { row: ActiveSessionRow; nowMs: number; contextsFailed: boolean; onOpen: () => void }) {
  const status = statusCell(row.running, row.threadStatus)
  const context = contextsFailed && row.snapshotId === null ? { text: "读取失败", ratio: null, tone: "danger" as const, title: "上下文快照读取失败，稍后自动重试" } : row.context
  const agentName = row.agent ? agentLabel(row.agent) : null
  const tone: StatusTone = row.running || row.threadStatus === "running"
    ? "active"
    : row.threadStatus === "failed"
      ? "danger"
      : row.threadStatus === "done"
        ? "success"
        : "neutral"
  const highCtx = context.tone === "danger" || (context.ratio != null && context.ratio >= 0.8)
  const statusSub = [status.note, relativeTime(row.lastTs, nowMs)].filter(Boolean).join(" · ")

  return (
    <TactileButton
      variant="row"
      role="button"
      type="button"
      onClick={onOpen}
      className={cn(
        "jn-interactive grid h-[52px] w-full min-w-0 items-center gap-3 overflow-hidden rounded-[14px] px-2.5 text-left odd:bg-jn-well hover:bg-jn-primary-wash/50",
        OVERVIEW_SESSION_GRID,
      )}
    >
      <span className="flex min-w-0 items-center gap-2.5">
        <span className="flex size-[30px] shrink-0 items-center justify-center rounded-[10px] bg-jn-surface">
          {row.agent ? <AgentBadge agent={row.agent} name={agentName ?? row.agent} size={24} /> : <Icon name="navSessions" size="sm" className="text-jn-text-3" />}
        </span>
        <span className="flex min-w-0 flex-col gap-px">
          <span className="truncate text-[13px] font-semibold text-jn-text" title={row.title}>{row.title}</span>
          <span className="truncate text-[11px] text-jn-text-3">{[agentName, row.project].filter(Boolean).join(" · ") || "—"}</span>
        </span>
      </span>

      <span className="min-w-0 flex-col gap-px @max-[680px]:hidden">
        <span className="block truncate font-mono text-[11.5px] font-medium text-jn-text" title={row.model ?? undefined}>{row.model ?? "—"}</span>
        <span className="block truncate text-[11px] text-jn-text-3" title={row.account ?? undefined}>{row.account ?? "—"}</span>
      </span>

      <span className="flex min-w-0 flex-col gap-1.5 pr-2" title={context.title}>
        <span className="flex items-baseline gap-1.5">
          <span className={cn("truncate text-[11.5px] font-semibold tabular-nums", highCtx ? "text-jn-danger" : context.tone === "muted" ? "text-jn-text-3" : "text-jn-text")}>
            {context.text}
          </span>
          <span className="min-w-0 flex-1" />
          {context.ratio != null && <span className="shrink-0 text-[11px] tabular-nums text-jn-text-3">{Math.round(context.ratio * 100)}%</span>}
        </span>
        <span className={cn("h-1.5 w-full overflow-hidden rounded-full", context.ratio != null ? "bg-jn-track" : "border-[1.5px] border-dashed border-jn-track")}>
          {context.ratio != null && (
            <span
              className={cn("jn-meter-fill block h-full rounded-full", highCtx ? "bg-jn-danger" : "bg-jn-primary")}
              style={{ width: `${Math.min(100, Math.round(context.ratio * 100))}%` }}
            />
          )}
        </span>
      </span>

      <span className="truncate text-[12px] font-medium tabular-nums text-jn-text-2 @max-[680px]:hidden" title={row.cache.title}>{row.cache.text}</span>

      <span className="flex min-w-0 flex-col items-start overflow-hidden" title={status.title ?? status.text}>
        <StatusPill tone={tone} className="max-w-full">{status.text}</StatusPill>
        <span className="mt-0.5 w-full truncate text-[10.5px] tabular-nums text-jn-text-3" title={statusSub}>{statusSub}</span>
      </span>

      <span className="justify-self-end text-jn-text-3"><Icon name="next" size="sm" /></span>
    </TactileButton>
  )
}
