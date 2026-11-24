import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { EVENTS } from "@/lib/events"
import { cn } from "@/lib/utils"
import { listenEvent } from "@/lib/api"
import { gatewayApi, type RouteLogDto, type RouteLogPageDto } from "@/lib/gatewayApi"
import { agentLabel, attemptLabels } from "@/lib/gatewayPage"
import { modelPart, routeTypeTag } from "@/lib/gatewaySessions"
import { useToast } from "@/components/ui/Toast"
import { AgentBadge } from "@/components/agents/AgentBadge"
import { Icon } from "@/icons"
import { Input, Pagination, Select, StatusPill, Tag, type StatusTone } from "@/ui"
import type { Connection } from "@/types"

const PAGE_SIZE = 10
const COLUMNS = "grid grid-cols-[76px_128px_minmax(0,1.1fr)_minmax(0,1.25fr)_64px_84px_minmax(0,0.9fr)] gap-3"

/**
 * 路由流水 —— 设计稿 R1-18（浅色）。
 *
 * 结构：筛选条（时间 / Agent / 结果 / 搜索 / 暂停刷新）+ 流水表（表头 32 + 行 50，斑马纹）。
 * 分页与查询范围仍由后端提供；筛选与搜索只作用于**当前页**（后端还没有这些过滤参数，
 * 不假装全量筛选——表头会写「当前页」）。
 *
 * - 总数、页数与范围来自后端；一次浏览固定查询上界，翻页期间新增请求不挤动旧页；
 * - 在第 1 页时，定时刷新与新请求会重新建立范围；在其他页时保持当前范围；
 * - 「耗时」是等待上游响应头的时间，不是首字耗时；结果是成功 / 拒绝 / 已取消 / 失败。
 */
export function RouteLogPanel({ desktop, connections, tick }: { desktop: boolean; connections: Connection[]; tick: number }) {
  const toast = useToast()
  const [data, setData] = useState<RouteLogPageDto | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [paused, setPaused] = useState(false)
  const [search, setSearch] = useState("")
  const [agent, setAgent] = useState("all")
  const [result, setResult] = useState("all")
  const lock = useRef(false)
  const current = useRef<RouteLogPageDto | null>(null)
  const pausedRef = useRef(false)
  pausedRef.current = paused

  const load = useCallback(async (page: number, upperId: number | null, quiet = false) => {
    if (!desktop || lock.current) return
    lock.current = true
    setBusy(true)
    try {
      const next = await gatewayApi.routeLogPage(page, PAGE_SIZE, upperId)
      current.current = next
      setData(next)
      setError(null)
    } catch (reason) {
      if (!current.current) setError(String(reason))
      else if (!quiet) toast.danger("读取路由流水失败", `${String(reason)}；仍显示第 ${current.current.page} 页，可稍后重试。`)
    } finally {
      lock.current = false
      setBusy(false)
    }
  }, [desktop, toast])

  // 进入页面与定时刷新：暂停时不刷新；只在第 1 页时重新建立查询范围
  useEffect(() => {
    if (pausedRef.current) return
    const shown = current.current
    if (!shown || shown.page === 1) void load(1, null, shown !== null)
  }, [tick, load])

  // 新请求结束后：正在看第 1 页时刷新（合并 1.5 秒内的多次结束）；暂停时不刷新
  useEffect(() => {
    if (!desktop) return
    let stopped = false
    let timer: number | null = null
    let off: (() => void) | null = null
    void listenEvent(EVENTS.GATEWAY_ROUTE, () => {
      if (pausedRef.current) return
      if (timer) window.clearTimeout(timer)
      timer = window.setTimeout(() => {
        if (!current.current || current.current.page === 1) void load(1, null, true)
      }, 1500)
    }).then((unlisten) => { if (stopped) unlisten(); else off = unlisten }).catch(() => {})
    return () => { stopped = true; off?.(); if (timer) window.clearTimeout(timer) }
  }, [desktop, load])

  const pageRows = data?.rows ?? []
  /** 当前页内的筛选：Agent / 结果 / 搜索（模型、来源、Agent 名） */
  const rows = useMemo(() => {
    const query = search.trim().toLowerCase()
    return pageRows.filter((row) => {
      if (agent !== "all" && row.agent !== agent) return false
      if (result !== "all" && row.result !== result) return false
      if (!query) return true
      const connection = row.connection_id ? connections.find((item) => item.id === row.connection_id) : undefined
      const haystack = [row.agent ? agentLabel(row.agent) : "", row.requested_model, row.upstream_model ?? "", row.source ?? "", connection?.baseName ?? ""]
      return haystack.some((value) => value.toLowerCase().includes(query))
    })
  }, [pageRows, agent, result, search, connections])
  const attempts = attemptLabels(pageRows)
  const from = data && data.total > 0 ? (data.page - 1) * data.page_size + 1 : 0
  const to = data ? from + rows.length - 1 : 0
  const byId = new Map(connections.map((c) => [c.id, c]))
  const bySlug = new Map(connections.filter((c) => c.slug).map((c) => [c.slug as string, c]))
  const agentOptions = useMemo(() => {
    const ids = [...new Set(pageRows.map((row) => row.agent).filter((value): value is string => Boolean(value)))]
    return [{ value: "all", label: "全部 Agent" }, ...ids.map((id) => ({ value: id, label: agentLabel(id) }))]
  }, [pageRows])

  return (
    <section aria-label="路由流水" className="flex min-w-0 flex-col gap-3.5">
      {/* 筛选条（R1-18）：Agent / 结果 / 搜索 / 暂停刷新 */}
      <div className="flex flex-wrap items-center gap-2">
        <div className="w-[130px]">
          <Select variant="filter" width="full" label="时间范围" value="today" onChange={() => undefined}
            options={[{ value: "today", label: "今天" }]} disabled />
        </div>
        <div className="w-[150px]">
          <Select variant="filter" width="full" label="按 Agent 筛选" value={agent} onChange={setAgent} options={agentOptions} />
        </div>
        <div className="w-[140px]">
          <Select variant="filter" width="full" label="按结果筛选" value={result} onChange={setResult}
            options={[{ value: "all", label: "全部结果" }, { value: "ok", label: "成功" }, { value: "rejected", label: "拒绝" }, { value: "failed", label: "失败" }, { value: "cancelled", label: "已取消" }]} />
        </div>
        <Input size="md" icon="search" value={search} onChange={(event) => setSearch(event.target.value)}
          placeholder="搜索模型、来源或 Agent" aria-label="搜索路由流水" className="min-w-[180px] max-w-[260px] flex-1" />
        <span className="min-w-0 flex-1" />
        <button type="button" aria-label={paused ? "恢复刷新" : "暂停刷新"} aria-pressed={paused} onClick={() => setPaused((value) => !value)}
          className={cn("jn-interactive flex h-9 items-center gap-1.5 rounded-full px-3.5 text-[12px] font-medium", paused ? "bg-jn-primary-soft text-jn-primary-ink" : "bg-jn-well text-jn-text-2")}>
          <Icon name={paused ? "play" : "pause"} size="sm" />
          {paused ? "已暂停" : "暂停刷新"}
        </button>
      </div>

      {/* 流水表（R1-18）：表头 32 + 行 50，斑马纹 */}
      <div className="flex min-w-0 flex-col gap-0.5 rounded-jn-card border border-jn-line bg-jn-surface p-3 shadow-jn-surface">
        <div className="flex items-center gap-2 px-1 pb-1 text-[11px] text-jn-text-3">
          <span>{!desktop ? "桌面端可用" : data && data.total > 0 ? `第 ${from}–${to} 条 · 共 ${data.total} 条` : data ? "暂无记录" : error ? "读取失败" : "读取中…"}</span>
          {(agent !== "all" || result !== "all" || search.trim()) && <Tag>筛选仅作用于当前页</Tag>}
        </div>
        {error && !data ? (
          <div className="flex items-center justify-center gap-3 px-4 py-8 text-[12px] text-jn-danger">
            读取路由流水失败：{error}
            <button type="button" disabled={busy} onClick={() => void load(1, null)} className="rounded-full bg-jn-well px-3 py-1 text-[12px] text-jn-text disabled:opacity-45">重试</button>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <div className="flex min-w-[780px] flex-col gap-0.5">
              <div className={cn(COLUMNS, "h-8 items-center px-3 text-[12px] font-medium text-jn-text-3")}>
                <span>时间</span><span>Agent</span><span>请求模型</span><span>路由到</span><span title="等待上游响应头的耗时；流式请求不是首字耗时">耗时</span><span>结果</span><span>说明</span>
              </div>
              {rows.length === 0 && (
                <p className="px-3 py-8 text-center text-[12px] text-jn-text-3">
                  {!pageRows.length ? (desktop ? "网关收到请求后，这里会显示时间、Agent、请求模型、路由到的来源、耗时和结果。" : "浏览器预览不读取路由流水；请在桌面端查看真实数据。") : "当前页没有符合筛选条件的记录。"}
                </p>
              )}
              {rows.map((row, index) => (
                <RouteRow key={row.id ?? `${row.ts}-${index}`} row={row} striped={index % 2 === 0}
                  attempt={typeof row.id === "number" ? attempts.get(row.id) : undefined}
                  connection={(row.connection_id && byId.get(row.connection_id)) || (row.source ? bySlug.get(row.source) : undefined)} />
              ))}
            </div>
          </div>
        )}
        {desktop && data && (
          <div className="pt-1">
            <Pagination page={data.page} pageCount={data.page_count}
              summary={data.page_count > 0 ? `共 ${data.page_count} 页` : "暂无可翻页的记录"}
              onChange={(page) => void load(page, data.upper_id)} />
          </div>
        )}
      </div>
    </section>
  )
}

function duration(ms: number | null): string {
  if (ms === null) return "—"
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`
}

function RouteRow({ row, connection, attempt, striped }: { row: RouteLogDto; connection: Connection | undefined; attempt?: string; striped: boolean }) {
  const model = row.upstream_model ?? (row.source ? modelPart(row.requested_model, row.source) : null)
  const name = connection?.baseName ?? row.source
  // P2-4：把「模型名怎么解析的」与「为什么选这个成员」合成一个 tooltip
  const modelTitle = [
    row.resolved_from === "bare"
      ? `${row.requested_model}（裸名：按模型目录第一个来源或默认来源解析）`
      : row.requested_model,
    row.decision ? `路由决策：${row.decision}` : null,
  ].filter(Boolean).join("\n")
  const tone: StatusTone = row.result === "ok" ? "success" : row.result === "rejected" ? "neutral" : "danger"
  const resultText = row.result === "ok" ? "成功" : row.result === "rejected" ? "拒绝" : row.result === "cancelled" ? "已取消" : "失败"
  return (
    <div className={cn(COLUMNS, "h-[50px] items-center rounded-[14px] px-3", striped && "bg-jn-well")}>
      <span className="tabular-nums text-[12px] font-medium text-jn-text-2">{new Date(row.ts).toLocaleTimeString("zh-CN", { hour12: false })}</span>
      <span className="flex min-w-0 items-center gap-1.5">
        {row.agent ? <AgentBadge agent={row.agent} name={agentLabel(row.agent)} size={30} /> : <span aria-hidden className="size-[30px] shrink-0 rounded-[10px] bg-jn-well" />}
        <span className="min-w-0 truncate text-[12.5px] font-semibold text-jn-text">{row.agent ? agentLabel(row.agent) : "—"}</span>
      </span>
      <span className="flex min-w-0 items-center gap-1" title={modelTitle}>
        <span className="truncate text-[12px] text-jn-text">{row.requested_model}</span>
        {row.resolved_from === "bare" && <Tag>裸名</Tag>}
        {row.profile && <Tag>Profile</Tag>}
        {row.decision && <Tag>分组</Tag>}
      </span>
      <span className="flex min-w-0 items-center gap-1.5">
        <span className="truncate text-[12.5px] text-jn-text" title={name ? `${name}${model ? ` · ${model}` : ""}` : undefined}>{name ? `${name}${model ? ` · ${model}` : ""}` : "—"}</span>
        {connection && <Tag>{routeTypeTag(connection.kind, connection.platformId)}</Tag>}
      </span>
      <span className="tabular-nums text-[12px] font-semibold text-jn-text">{duration(row.duration_ms)}</span>
      <span className="flex min-w-0 flex-col items-start gap-0.5">
        <StatusPill tone={tone}>{resultText}</StatusPill>
        {attempt && <span className="text-[10px] text-jn-text-3">{attempt}</span>}
      </span>
      <span className="truncate text-[11.5px] text-jn-text-3" title={row.result === "cancelled" ? "客户端在流式响应中途断开，这次请求没有完成" : attempt ? "同一次请求在分组成员间转移的第几次尝试" : undefined}>
        {row.result === "cancelled" ? "客户端中途断开" : "—"}
      </span>
    </div>
  )
}
