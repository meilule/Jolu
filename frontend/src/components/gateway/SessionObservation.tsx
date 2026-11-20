import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { ArrowDownWideNarrow, ArrowLeft, ChartColumnStacked, ChevronDown, ChevronRight, History, LayoutGrid, Radio, RefreshCw, Search } from "lucide-react"
import { cn } from "@/lib/utils"
import {
  gatewayApi, type ContextKind, type ContextSnapshotDetailDto, type ContextSnapshotSummaryDto, type ContextToolResultDto,
} from "@/lib/gatewayApi"
import { compactTokens, sessionTitle, type SessionView } from "@/lib/gatewaySessions"
import { agentLabel } from "@/lib/gatewayPage"
import { projectOf } from "@/lib/contextSessions"
import {
  KINDS, cacheCard, callLines, composition, contextCard, countLabel, deviationNote, historyLabel, initialFollow, ioSummary,
  kindMeta, newSince, onHistory, pause, resume, routeChain, runSummary, tableRows, treemap, visibleRows,
  type FollowState, type SortMode, type TableRow,
} from "@/lib/contextObservation"
import { AgentBadge } from "@/components/agents/AgentList"

/** 默认展开时每个工具先显示几条单次返回（V10-05「查看全部 N 条」） */
const CALLS_PREVIEW = 3

/**
 * 模型路由 · 会话观察（Jolu.pen V10-03 / 04 会话观察与上下文组成、05 / 06 工具结果原位展开、07 / 08 历史请求与跟随暂停、
 * 09 / 10 矩形图与筛选联动、11 / 12 Skills 来源证据、13 / 14 采集状态）。
 *
 * 数据：`gateway_context_history`（每 5 秒，判断新请求）+ `gateway_context_snapshot`（所选请求的组成）+
 * `gateway_context_tool_items`（展开某个工具时才取）。默认跟随最新完成的请求；选历史、展开条目、主动筛选时暂停，
 * 只计新请求数、不替换正在看的内容，点「恢复跟随最新」才切换并收起展开（V10-13「阅读不中断」）。
 */
export function SessionObservation({ platform, sessionId, view, groupName, onBack }: {
  platform: string
  sessionId: string
  /** 最近会话里的这一项（标题、项目、请求数）；只有快照、不在最近会话里时为 null */
  view: SessionView | null
  groupName: (slug: string) => string | null
  onBack: () => void
}) {
  const [history, setHistory] = useState<ContextSnapshotSummaryDto[] | null>(null)
  const [historyFailed, setHistoryFailed] = useState(false)
  const [follow, setFollow] = useState<FollowState>(initialFollow)
  const [detail, setDetail] = useState<ContextSnapshotDetailDto | null | undefined>(undefined)
  const [detailFailed, setDetailFailed] = useState(false)
  const [kind, setKind] = useState<ContextKind | null>(null)
  const [search, setSearch] = useState("")
  const [sort, setSort] = useState<SortMode>("tokens")
  const [chart, setChart] = useState<"bar" | "treemap">("bar")
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [picker, setPicker] = useState(false)
  const historyRef = useRef<ContextSnapshotSummaryDto[]>([])

  const loadHistory = useCallback(async () => {
    try {
      const rows = await gatewayApi.contextHistory(platform, sessionId)
      historyRef.current = rows
      setHistory(rows)
      setHistoryFailed(false)
      setFollow((state) => onHistory(state, rows))
    } catch {
      setHistoryFailed(true)
    }
  }, [platform, sessionId])

  useEffect(() => {
    void loadHistory()
    const timer = window.setInterval(() => { if (!document.hidden) void loadHistory() }, 5_000)
    return () => window.clearInterval(timer)
  }, [loadHistory])

  const selectedId = follow.selectedId
  const loadDetail = useCallback(async () => {
    if (selectedId == null) return
    try {
      const next = await gatewayApi.contextSnapshot(platform, sessionId, selectedId)
      setDetail(next)
      setDetailFailed(false)
    } catch {
      setDetailFailed(true)
    }
  }, [platform, sessionId, selectedId])

  // 所选请求变了重读；跟随中、还在分析或刚完成（用量可能晚到）时随历史轮询一起刷新
  const selectedSummary = history?.find((row) => row.id === selectedId) ?? null
  const freshness = selectedSummary ? `${selectedSummary.state}:${selectedSummary.counted}:${selectedSummary.input_tokens}` : ""
  useEffect(() => { void loadDetail() }, [loadDetail, freshness])

  useEffect(() => {
    if (!picker) return
    const close = (event: KeyboardEvent) => { if (event.key === "Escape") setPicker(false) }
    window.addEventListener("keydown", close)
    return () => window.removeEventListener("keydown", close)
  }, [picker])

  const paused = (select?: number) => setFollow((state) => pause(state, historyRef.current, select))
  const resumeFollow = () => {
    setFollow(resume(historyRef.current))
    setExpanded(new Set())
    setPicker(false)
  }
  const toggleRow = (key: string) => {
    paused()
    setExpanded((current) => {
      const next = new Set(current)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }
  const chooseKind = (next: ContextKind | null) => {
    if (next !== null) paused()
    setKind(next)
  }

  const latestId = history?.[0]?.id ?? null
  const newCount = history ? newSince(follow, history) : 0
  const summary: ContextSnapshotSummaryDto | null = detail ?? selectedSummary
  const rows = useMemo(() => tableRows(detail?.parts ?? null), [detail])
  const shown = visibleRows(rows, kind, search, sort)
  const parts = composition(detail?.parts ?? null)
  const total = parts.reduce((sum, part) => sum + part.tokens, 0)
  const requests = view?.session.links.reduce((sum, link) => sum + link.requests, 0) ?? null
  const agent = view?.session.agent ?? summary?.agent ?? null
  const title = view ? sessionTitle(view.session) : sessionTitle({ title: null, session_id: sessionId })
  const project = projectOf(view?.session.cwd)
  const lastReady = history?.find((row) => row.state === "ready")?.ts ?? null

  return (
    // @container：本组件的窄窗降级全部按这个容器的宽度判断（右侧详情面板宽，不随窗口直接变化）
    <div className="@container flex min-w-0 flex-1 flex-col gap-3 px-8 pb-8 pt-2">
      <div className="flex items-center gap-2">
        <button type="button" onClick={onBack}
          className="flex items-center gap-1.5 text-meta text-jolu-muted outline-none hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <ArrowLeft aria-hidden className="size-4" />运行概览
        </button>
        <span className="flex-1" />
        <span className="text-note text-jolu-muted">当前会话 · {requests ?? history?.length ?? "—"} 次请求</span>
      </div>

      <div className="flex items-center gap-3">
        {agent ? <AgentBadge agent={agent} name={agentLabel(agent)} size={34} /> : <span aria-hidden className="size-[34px] shrink-0 rounded-[9px] bg-jolu-side" />}
        <div className="flex min-w-0 flex-col gap-0.5">
          <h2 className="truncate text-[21px] font-semibold text-jolu-text" title={title}>{title}</h2>
          <span className="truncate text-note text-jolu-muted" title={view?.session.cwd ?? undefined}>
            {[agent ? agentLabel(agent) : null, project ? `${project} 项目` : null, view?.session.cwd].filter(Boolean).join(" · ") || sessionId}
          </span>
        </div>
        <span className="flex-1" />
        <div className="relative">
          <button type="button" aria-haspopup="listbox" aria-expanded={picker} onClick={() => setPicker((open) => !open)} disabled={!history || history.length === 0}
            className="flex h-9 items-center gap-1.5 rounded-lg border border-jolu-border px-3 text-meta font-medium text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
            <History aria-hidden className="size-3.5 text-jolu-muted" />
            {selectedId == null ? "还没有请求" : `${selectedId === latestId ? "最近一次请求" : "历史请求"} · #${selectedId}`}
            <ChevronDown aria-hidden className="size-3 text-jolu-muted" />
          </button>
          {picker && history && (
            <div role="listbox" aria-label="选择本次会话的一次请求"
              className="absolute right-0 top-11 z-20 flex w-72 flex-col rounded-xl border border-jolu-border bg-jolu-bg p-1.5 shadow-lg">
              <span className="px-2.5 py-1.5 text-note text-jolu-muted">选择本次会话的一次请求</span>
              {history.map((row) => (
                <button key={row.id} type="button" role="option" aria-selected={row.id === selectedId}
                  onClick={() => { setPicker(false); if (row.id === latestId && follow.following) return; paused(row.id) }}
                  className={cn("flex flex-col items-start gap-0.5 rounded-lg px-2.5 py-2 text-left outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent", row.id === selectedId && "bg-jolu-selected")}>
                  <span className="flex w-full items-center gap-2 text-meta">
                    <span className="font-medium text-jolu-text">#{row.id}</span>
                    <span className="flex-1" />
                    <span className="tabular-nums text-note text-jolu-muted">{new Date(row.ts).toLocaleTimeString("zh-CN", { hour12: false })}</span>
                  </span>
                  <span className={cn("text-note", row.route_result && row.route_result !== "ok" ? "text-jolu-danger" : "text-jolu-muted")}>
                    {historyLabel(row, row.id === latestId, row.id === selectedId)}
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>
        <button type="button" onClick={() => (follow.following ? paused() : resumeFollow())} aria-pressed={follow.following}
          className={cn("flex h-9 items-center gap-1.5 rounded-lg px-3 text-meta font-medium outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
            follow.following ? "bg-jolu-selected text-jolu-on" : "border border-jolu-border text-jolu-muted hover:bg-jolu-side")}>
          <Radio aria-hidden className="size-3.5" />
          {follow.following ? "跟随最新" : "跟随已暂停"}
        </button>
      </div>

      {!follow.following && (
        <div className="flex items-center gap-3 rounded-lg bg-jolu-selected/60 px-4 py-2 text-meta">
          <span className="text-jolu-text">
            {selectedId !== latestId && selectedId != null ? `正在查看历史请求 #${selectedId}。` : "正在查看条目，已暂停跟随。"}
            {newCount > 0 ? `已有 ${newCount} 次新请求，内容保持不变。` : "有新请求时只计数，不替换正在看的内容。"}
          </span>
          <span className="flex-1" />
          <button type="button" onClick={resumeFollow} className="shrink-0 font-medium text-jolu-on outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">
            恢复跟随最新 →
          </button>
        </div>
      )}

      {history === null && !historyFailed && <Skeleton lines={6} label="正在读取会话快照" />}
      {historyFailed && history === null && (
        <StateCard title="读取失败" detail="上下文快照读取失败，稍后自动重试。" action={{ label: "重试读取", run: () => void loadHistory() }} />
      )}
      {history && history.length === 0 && (
        <StateCard title="暂无请求" detail="这个会话还没有经过 Jolu 的模型路由。等待下一次请求；不显示历史累计用量作为上下文。" />
      )}

      {summary && <Overview summary={summary} groupName={groupName} />}

      {history && history.length > 0 && (
        <section className="flex flex-col gap-3">
          <div className="flex items-center gap-2.5">
            <span className="text-[15px] font-semibold text-jolu-text">上下文组成</span>
            <span className="text-note text-jolu-muted">{summary?.counted ? "本地估算 · 已按上游实测换算" : "本地估算"}</span>
            <span className="flex-1" />
            <ChartToggle active={chart === "bar"} onClick={() => setChart("bar")} icon={ChartColumnStacked} label="分类堆叠条" />
            <ChartToggle active={chart === "treemap"} onClick={() => setChart("treemap")} icon={LayoutGrid} label="矩形图" />
          </div>
          <CompositionArea detail={detail} detailFailed={detailFailed} lastReady={lastReady} onRetry={() => void loadDetail()}
            chart={chart} parts={parts} total={total} kind={kind} onKind={chooseKind} />
          {detail?.parts && (
            <>
              <p className="text-note text-jolu-muted">
                分类为本地估算；Skills、记忆与文件仅计入有本次来源证据的内容，其余保留为未分类。{deviationNote(detail) ?? ""}
              </p>
              <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="按类别筛选">
                <Chip active={kind === null} onClick={() => chooseKind(null)} label="全部" />
                {KINDS.map((meta) => <Chip key={meta.kind} active={kind === meta.kind} onClick={() => chooseKind(meta.kind)} label={meta.chip} />)}
              </div>
              <div className="flex flex-wrap items-center gap-x-2.5 gap-y-2">
                <label className="flex h-9 w-full min-w-0 max-w-72 flex-1 items-center gap-1.5 rounded-lg border border-jolu-border px-2.5 focus-within:outline-2 focus-within:outline-jolu-accent">
                  <Search aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />
                  <input value={search} onChange={(event) => { if (event.target.value) paused(); setSearch(event.target.value) }}
                    placeholder="搜索名称、路径或摘要" aria-label="搜索名称、路径或摘要"
                    className="min-w-0 flex-1 bg-transparent text-meta text-jolu-text outline-none placeholder:text-jolu-muted" />
                </label>
                <span className="hidden flex-1 @[640px]:block" />
                <span className="whitespace-nowrap text-note text-jolu-muted">{countLabel(shown, kind)}</span>
                <button type="button" onClick={() => setSort((mode) => (mode === "tokens" ? "kind" : "tokens"))}
                  className="flex h-9 items-center gap-1.5 rounded-lg border border-jolu-border px-3 text-meta font-medium text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
                  <ArrowDownWideNarrow aria-hidden className="size-3.5 text-jolu-muted" />
                  {sort === "tokens" ? "按占用排序" : "按类别排序"}
                </button>
              </div>
              <ItemTable rows={shown} counted={detail.counted} expanded={expanded} onToggle={toggleRow} snapshotId={detail.id} />
            </>
          )}
        </section>
      )}

      <footer className="text-note text-jolu-muted">
        展开、筛选或选择旧请求时暂停跟随；摘要先脱敏，正文未采集时不补写预览。Skills、记忆与文件必须有本次请求的来源证据，没有证据则保留为未分类。
      </footer>
    </div>
  )
}

function Overview({ summary, groupName }: { summary: ContextSnapshotSummaryDto; groupName: (slug: string) => string | null }) {
  const chain = routeChain(summary, groupName)
  const run = runSummary(summary)
  const ctx = contextCard(summary)
  const cache = cacheCard(summary)
  return (
    <>
      {/* 链路行窄容器折行（三个格子 w-52 固定，不折行会互相挤压截断） */}
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl bg-jolu-side px-4 py-3">
        <ChainCell label="请求模型" value={chain.requested} mono />
        <ChevronRight aria-hidden className="size-4 shrink-0 text-jolu-muted" />
        <ChainCell label="路由分组" value={chain.group ?? "未经分组"} muted={!chain.group} />
        <ChevronRight aria-hidden className="size-4 shrink-0 text-jolu-muted" />
        <ChainCell label="实际模型 / 来源账号" value={chain.actual} grow />
      </div>
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-note text-jolu-muted">
        <span className={cn("font-medium whitespace-nowrap", run.tone === "on" ? "text-jolu-on" : run.tone === "danger" ? "text-jolu-danger" : "text-jolu-muted")}>● {run.status}</span>
        {run.parts.map((part) => <span key={part} className="whitespace-nowrap tabular-nums">{part}</span>)}
        <span className="hidden flex-1 @[640px]:block" />
        <span className="whitespace-nowrap tabular-nums">{ioSummary(summary)}</span>
      </div>
      {/* 双栏在窄容器降为上下堆叠（提示词缓存列 360px 固定，容器不足时被裁出面板） */}
      <div className="grid gap-4 py-1 @[640px]:grid-cols-[1fr_360px] @[640px]:gap-6">
        <UsageBlock title="上下文占用" basis={ctx.basis} value={ctx.value} ratio={ctx.ratio} ratioText={ctx.ratio != null ? `${(ctx.ratio * 100).toFixed(1)}%` : null} note={ctx.note} danger={ctx.ratio != null && ctx.ratio >= 0.8} />
        <UsageBlock title="提示词缓存" basis="" value={cache.value} ratio={cache.ratio} ratioText={cache.ratio != null ? `占输入 ${(cache.ratio * 100).toFixed(1)}%` : null} note={cache.note} />
      </div>
    </>
  )
}

function ChainCell({ label, value, mono, muted, grow }: { label: string; value: string; mono?: boolean; muted?: boolean; grow?: boolean }) {
  return (
    <span className={cn("flex min-w-0 flex-col gap-1", grow ? "min-w-[180px] flex-1" : "w-52 shrink-0")}>
      <span className="text-note text-jolu-muted">{label}</span>
      <span className={cn("truncate text-meta font-medium", mono && "font-mono", muted ? "text-jolu-muted" : "text-jolu-text")} title={value}>{value}</span>
    </span>
  )
}

function UsageBlock({ title, basis, value, ratio, ratioText, note, danger }: { title: string; basis: string; value: string; ratio: number | null; ratioText: string | null; note: string; danger?: boolean }) {
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <span className="flex items-center gap-2">
        <span className="text-meta font-semibold text-jolu-text">{title}</span>
        {basis && <span className={cn("text-note", basis === "上游实测" ? "text-jolu-on" : "text-jolu-muted")}>{basis}</span>}
      </span>
      <span className="flex items-baseline gap-2">
        <span className={cn("text-[25px] font-semibold tabular-nums", danger ? "text-jolu-danger" : "text-jolu-text")}>{value}</span>
        <span className="flex-1" />
        {ratioText && <span className="text-meta tabular-nums text-jolu-muted">{ratioText}</span>}
      </span>
      {ratio != null && (
        <span className="h-[7px] overflow-hidden rounded-full bg-jolu-track">
          <span className={cn("block h-full rounded-full motion-safe:transition-[width] motion-safe:duration-150", danger ? "bg-jolu-danger" : "bg-jolu-accent")} style={{ width: `${Math.min(100, ratio * 100)}%` }} />
        </span>
      )}
      <span className="text-note text-jolu-muted">{note}</span>
    </div>
  )
}

function ChartToggle({ active, onClick, icon: Icon, label }: { active: boolean; onClick: () => void; icon: React.ElementType; label: string }) {
  return (
    <button type="button" onClick={onClick} aria-pressed={active}
      className={cn("flex h-8 items-center gap-1.5 rounded-lg px-2.5 text-meta font-medium outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
        active ? "bg-jolu-selected text-jolu-on" : "border border-jolu-border text-jolu-text hover:bg-jolu-side")}>
      <Icon aria-hidden className={cn("size-3.5", active ? "text-jolu-on" : "text-jolu-muted")} />{label}
    </button>
  )
}

function Chip({ active, onClick, label }: { active: boolean; onClick: () => void; label: string }) {
  return (
    <button type="button" onClick={onClick} aria-pressed={active}
      className={cn("h-7 rounded-md px-2.5 text-note outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
        active ? "bg-jolu-selected font-semibold text-jolu-on" : "text-jolu-muted hover:bg-jolu-side hover:text-jolu-text")}>
      {label}
    </button>
  )
}

function CompositionArea({ detail, detailFailed, lastReady, onRetry, chart, parts, total, kind, onKind }: {
  detail: ContextSnapshotDetailDto | null | undefined
  detailFailed: boolean
  lastReady: number | null
  onRetry: () => void
  chart: "bar" | "treemap"
  parts: { kind: ContextKind; tokens: number; share: number | null }[]
  total: number
  kind: ContextKind | null
  onKind: (kind: ContextKind | null) => void
}) {
  if (detailFailed && !detail) return <StateCard title="读取失败" detail="这次请求的上下文读取失败。" action={{ label: "重试读取", run: onRetry }} />
  if (detail === undefined) return <Skeleton lines={2} label="正在读取上下文组成" />
  if (detail === null) return <StateCard title="暂无请求" detail="这次请求的快照已被清理（每个会话只保留最近 20 次）。" />
  if (detail.state === "analyzing" && !detail.parts) return <Skeleton lines={2} label="正在分析请求快照：总量可用时先展示，组成显示骨架，不以 0 占位。" />
  if (detail.state === "no_body") return <StateCard title="尚未采集上下文" detail="已记录路由去向，这次请求没有内容快照。" />
  if (detail.state === "unsupported") return <StateCard title="此来源暂不支持内容分析" detail="可查看路由与上游用量，无法拆分内容组成。" />
  if (detail.state === "failed" || !detail.parts) {
    return (
      <StateCard title="采集失败" detail={`请求已完成，但上下文快照没有保存成功${lastReady ? `；上次成功 ${new Date(lastReady).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false })}` : ""}。`}
        action={{ label: "重试读取", run: onRetry }} />
    )
  }
  const selected = kind ? parts.find((part) => part.kind === kind) : null
  return (
    <div className="flex flex-col gap-2.5">
      {chart === "bar" ? (
        <div className="flex h-8 gap-0.5 overflow-hidden rounded-md" onClick={(event) => { if (event.target === event.currentTarget) onKind(null) }}>
          {parts.filter((part) => part.tokens > 0).map((part) => {
            const meta = kindMeta(part.kind)
            return (
              <button key={part.kind} type="button" aria-label={`${meta.label} ${compactTokens(part.tokens)}`} title={`${meta.label} · ${compactTokens(part.tokens)}（${((part.share ?? 0) * 100).toFixed(1)}%）`}
                onClick={() => onKind(kind === part.kind ? null : part.kind)}
                className={cn("h-full min-w-[3px] outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent motion-safe:transition-opacity motion-safe:duration-150", meta.fill, kind && kind !== part.kind && "opacity-35")}
                style={{ flexGrow: part.tokens, flexBasis: 0 }} />
            )
          })}
        </div>
      ) : (
        <div className="relative h-[180px] overflow-hidden rounded-md" onClick={(event) => { if (event.target === event.currentTarget) onKind(null) }}>
          {treemap(parts).map((tile) => {
            const meta = kindMeta(tile.kind)
            const pct = total > 0 ? ((tile.tokens / total) * 100).toFixed(1) : null
            return (
              <button key={tile.kind} type="button" onClick={() => onKind(kind === tile.kind ? null : tile.kind)} title={`${meta.label} · ${compactTokens(tile.tokens)}`}
                className={cn("absolute flex flex-col items-start gap-0.5 overflow-hidden border-2 border-jolu-bg p-2 text-left outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent motion-safe:transition-opacity motion-safe:duration-150",
                  meta.fill, meta.ink, kind && kind !== tile.kind && "opacity-35")}
                style={{ left: `${tile.x * 100}%`, top: `${tile.y * 100}%`, width: `${tile.w * 100}%`, height: `${tile.h * 100}%` }}>
                <span className="truncate text-note font-semibold">{meta.label}</span>
                <span className="truncate text-note tabular-nums">{compactTokens(tile.tokens)}{pct && tile.w > 0.18 ? ` · ${pct}%` : ""}</span>
              </button>
            )
          })}
        </div>
      )}
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5">
        {parts.map((part) => {
          const meta = kindMeta(part.kind)
          return (
            <span key={part.kind} className="flex items-center gap-1.5 text-note text-jolu-muted">
              <span aria-hidden className={cn("size-[7px] rounded-[2px]", meta.fill)} />
              {meta.label} {compactTokens(part.tokens)}
            </span>
          )
        })}
      </div>
      {selected && (
        <p className="text-note text-jolu-on">
          已选：{kindMeta(selected.kind).label} · {compactTokens(selected.tokens)}{selected.share != null ? `（${(selected.share * 100).toFixed(1)}%）` : ""}　/　点击空白或「全部」清除筛选
        </p>
      )}
    </div>
  )
}

/**
 * 明细表列定义：按容器宽度三档降列（表头 / 数据行 / 展开子行三处必须用同一套，否则错位）。
 * 五列固定宽合计约 425px + 间距与内边距，右侧详情面板窄于 ~560px 时 Token 列被裁出面板
 * （2026-11-17 尺寸扫描）。降级顺序按可推断性：「占输入」占比可由 Token 与总数推断先收，
 * 「类别」已在筛选 chips 里表达次收；名称 / 次数 / Token 三列始终保留。
 * 注意类名写死字面量——Tailwind 4 静态扫描，模板串拼接的类不会生成。
 */
const ITEM_GRID = [
  "grid-cols-[minmax(0,1fr)_110px_60px_100px_155px]",
  "@max-[619px]:grid-cols-[minmax(0,1fr)_110px_60px_100px]",
  "@max-[459px]:grid-cols-[minmax(0,1fr)_60px_100px]",
].join(" ")

function ItemTable({ rows, counted, expanded, onToggle, snapshotId }: {
  rows: TableRow[]
  counted: boolean
  expanded: Set<string>
  onToggle: (key: string) => void
  snapshotId: number
}) {
  return (
    <div className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
      <div className={cn("grid gap-3.5 bg-jolu-side px-4 py-2 text-note text-jolu-muted", ITEM_GRID)}>
        <span className="whitespace-nowrap">名称 / 来源</span>
        <span className="whitespace-nowrap @max-[459px]:hidden">类别</span>
        <span className="whitespace-nowrap">次数</span>
        <span className="whitespace-nowrap">{counted ? "Token · 换算" : "Token · 估算"}</span>
        <span className="whitespace-nowrap @max-[619px]:hidden">占输入</span>
      </div>
      {rows.length === 0 && <p className="border-t border-jolu-table-divider px-4 py-6 text-center text-meta text-jolu-muted">没有符合条件的条目。</p>}
      {rows.map((row) => {
        const open = expanded.has(row.key)
        const meta = kindMeta(row.kind)
        return (
          <div key={row.key} className="border-t border-jolu-table-divider">
            <button type="button" disabled={!row.expand} onClick={() => onToggle(row.key)} aria-expanded={row.expand ? open : undefined}
              className={cn("grid w-full items-center gap-3.5 px-4 py-2 text-left outline-none enabled:hover:bg-jolu-side/60 focus-visible:outline-2 focus-visible:outline-jolu-accent", ITEM_GRID)}>
              <span className="flex min-w-0 items-center gap-2">
                {row.expand
                  ? <ChevronRight aria-hidden className={cn("size-3 shrink-0 text-jolu-muted motion-safe:transition-transform motion-safe:duration-150", open && "rotate-90")} />
                  : <span aria-hidden className="size-3 shrink-0" />}
                <span className="flex min-w-0 flex-col gap-0.5">
                  <span className="truncate text-meta font-medium text-jolu-text" title={row.title}>{row.name}</span>
                  <span className="truncate text-note text-jolu-muted" title={row.subtitle}>{row.subtitle}</span>
                </span>
              </span>
              <span className="text-note text-jolu-muted @max-[459px]:hidden">{meta.label}</span>
              <span className="text-note tabular-nums text-jolu-muted">{row.countText}</span>
              <span className="text-note tabular-nums text-jolu-muted">{compactTokens(row.tokens)}</span>
              <span className="@max-[619px]:hidden"><ShareCell share={row.share} fill={meta.fill} /></span>
            </button>
            {open && row.expand === "calls" && row.tool && <CallsPanel snapshotId={snapshotId} tool={row.tool} counted={counted} />}
            {open && row.expand !== "calls" && row.children.length > 0 && (
              <div className="flex flex-col bg-jolu-side/40 py-1">
                {row.children.map((child) => (
                  <div key={child.key} className={cn("grid items-center gap-3.5 py-1.5 pl-[42px] pr-4 text-note", ITEM_GRID)}>
                    <span className="flex min-w-0 flex-col">
                      <span className="truncate text-jolu-text" title={child.name}>{child.name}</span>
                      {child.sub && <span className="truncate text-jolu-muted">{child.sub}</span>}
                    </span>
                    <span className="@max-[459px]:hidden" />
                    <span />
                    <span className="tabular-nums text-jolu-muted">{compactTokens(child.tokens)}</span>
                    <span className="@max-[619px]:hidden"><ShareCell share={child.share} fill={meta.fill} /></span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}

function ShareCell({ share, fill }: { share: number | null; fill: string }) {
  return (
    <span className="flex items-center gap-2">
      <span className="w-11 shrink-0 text-note tabular-nums text-jolu-muted">{share == null ? "—" : `${(share * 100).toFixed(1)}%`}</span>
      <span className="h-1 flex-1 overflow-hidden rounded-full bg-jolu-track">
        <span className={cn("block h-full rounded-full", fill === "bg-jolu-track" || fill === "bg-jolu-stage" || fill === "bg-jolu-selected" || fill === "bg-jolu-pressed" ? "bg-jolu-accent" : fill)} style={{ width: `${Math.min(100, (share ?? 0) * 100)}%` }} />
      </span>
    </span>
  )
}

/** 某个工具的单次返回（V10-05）：只有脱敏摘要，正文未采集 */
function CallsPanel({ snapshotId, tool, counted }: { snapshotId: number; tool: string; counted: boolean }) {
  const [calls, setCalls] = useState<ContextToolResultDto[] | null | undefined>(undefined)
  const [all, setAll] = useState(false)
  useEffect(() => {
    let alive = true
    gatewayApi.contextToolItems(snapshotId, tool).then((rows) => { if (alive) setCalls(rows) }).catch(() => { if (alive) setCalls(null) })
    return () => { alive = false }
  }, [snapshotId, tool])
  if (calls === undefined) return <div className="px-[42px] py-3 text-note text-jolu-muted">正在读取单次返回…</div>
  if (calls === null) return <div className="px-[42px] py-3 text-note text-jolu-danger">单次返回读取失败。</div>
  const { lines, total, tokens } = callLines(calls, all ? null : CALLS_PREVIEW)
  if (total === 0) return <div className="px-[42px] py-3 text-note text-jolu-muted">这次请求没有记下 {tool} 的单次返回（每次请求最多记 200 条）。</div>
  return (
    <div className="flex flex-col gap-2 bg-jolu-side/40 py-3 pl-[42px] pr-4">
      <div className="flex items-center gap-3 text-note text-jolu-muted">
        <span>本次输入保留 {total} 条返回 · 合计 {compactTokens(tokens)} Token（{counted ? "按实测换算" : "估算"}）</span>
        <span className="flex-1" />
        {total > CALLS_PREVIEW && (
          <button type="button" onClick={() => setAll((value) => !value)} className="font-medium text-jolu-on outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">
            {all ? "只看最近几条" : `查看全部 ${total} 条 →`}
          </button>
        )}
      </div>
      {lines.map((line) => (
        <div key={line.key} className="flex flex-col gap-1 rounded-lg bg-jolu-bg px-3 py-2">
          <span className="flex min-w-0 items-center gap-2 text-note">
            <span className="shrink-0 tabular-nums text-jolu-muted">#{line.index}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-jolu-text" title={line.target}>{line.target}</span>
            <span className="shrink-0 tabular-nums text-jolu-muted">{line.tokens}</span>
          </span>
          <span className="text-note text-jolu-text">{line.summary}</span>
          <span className="text-note text-jolu-muted">{line.masked ? "已脱敏摘要（本机路径、用户名或凭证已替换）" : "摘要"}　/　正文未采集</span>
        </div>
      ))}
    </div>
  )
}

function StateCard({ title, detail, action }: { title: string; detail: string; action?: { label: string; run: () => void } }) {
  return (
    <div className="flex items-center gap-4 rounded-xl border border-jolu-border px-5 py-4">
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="text-meta font-semibold text-jolu-text">{title}</span>
        <span className="text-note text-jolu-muted">{detail}</span>
      </span>
      {action && (
        <button type="button" onClick={action.run}
          className="flex h-9 shrink-0 items-center gap-1.5 rounded-lg border border-jolu-border px-3 text-meta font-medium text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <RefreshCw aria-hidden className="size-3.5 text-jolu-muted" />{action.label}
        </button>
      )}
    </div>
  )
}

function Skeleton({ lines, label }: { lines: number; label: string }) {
  return (
    <div role="status" aria-label={label} className="flex flex-col gap-2 rounded-xl border border-jolu-border p-4">
      <span className="text-note text-jolu-muted">{label}</span>
      {Array.from({ length: lines }, (_, i) => <span key={i} className="h-3 rounded bg-jolu-side motion-safe:animate-pulse" style={{ width: `${90 - i * 9}%` }} />)}
    </div>
  )
}
