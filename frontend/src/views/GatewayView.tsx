import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { cn } from "@/lib/utils"
import { api, invoke, isTauri, listenEvent, type StatsOverviewDto } from "@/lib/api"
import { dateKey, rangeToMillis, todayRange, type DateRange } from "@/lib/usageStats"
import { EVENTS } from "@/lib/events"
import { useLatestGate } from "@/hooks/useLatestGate"
import {
  gatewayApi, type AgentDiscoveryDto, type CatalogModelDto, type DayOverviewDto,
  type ContextSessionRowDto, type GatewayStatusDto, type RouteLogDto,
} from "@/lib/gatewayApi"
import type { RoutingGroupDto } from "@/components/gateway/routingGroups"
import { agentLabel, canEscape, wiredCount } from "@/lib/gatewayPage"
import { isPortBindError, parseGatewayPort, portChangeSuccessText } from "@/lib/gatewayPort"
import { driftText } from "@/lib/agentPage"
import { rateLimitedSources, type OverviewRange } from "@/lib/gatewayOverview"
import { activeSessionRows, contextRisks } from "@/lib/contextSessions"
import type { MainPage } from "@/lib/mainNavigation"
import { useConnections } from "@/hooks/useConnections"
import { useGatewaySessions } from "@/hooks/useGatewaySessions"
import { useToast } from "@/components/ui/Toast"
import { Button, ConfirmDialog, IconButton, PageActions, PageTabs, StatusPill, Tabs } from "@/ui"
import { Icon, type IconName } from "@/icons"
import { RouteLogPanel } from "@/components/gateway/RouteLogPanel"
import { OverviewPanel, type AttentionItem } from "@/components/gateway/OverviewPanel"
import { SessionObservation } from "@/components/gateway/SessionObservation"
import { GroupsPanel } from "@/components/gateway/GroupsPanel"
import { PoolPanel } from "@/components/gateway/PoolPanel"
import { AccessPanel } from "@/components/gateway/AccessPanel"
import { MiddlewarePanel } from "@/components/gateway/MiddlewarePanel"
import { GatewayPortDialog } from "@/components/gateway/GatewayPortDialog"

/**
 * 模型路由（设计稿 jolu-new.pen R1-01/02 概览、R1-18 路由流水、R1-19 路由分组、
 * R1-20 模型池、R1-21 对外访问、R1-22 中间件）。
 *
 * 外壳按画板拆成三层：
 * 1. **六个页签在页头**（R1 的「页签」槽），内容区不再重复一行页签；
 * 2. **概览**用一张大「本机路由」卡（地址、状态、运行数据、停止 / 开启圆钮）；
 *    其余五个页签共用 56px 紧凑状态条（R1-18～22 顶部的「路由状态条」）；
 * 3. 页面操作（如中间件「添加规则」）通过 `PageActions` 送进页头。
 *
 * 工具的接入与档位在「Agent」页；模型池只读，加入 / 移出与前缀在供应商。
 */

type Tab = "overview" | "log" | "groups" | "pool" | "access" | "middleware"

const TABS: readonly { id: Tab; label: string; icon: IconName }[] = [
  { id: "overview", label: "概览", icon: "activity" },
  { id: "log", label: "路由流水", icon: "file" },
  { id: "groups", label: "路由分组", icon: "blocks" },
  { id: "pool", label: "模型池", icon: "shield" },
  { id: "access", label: "对外访问", icon: "external" },
  { id: "middleware", label: "中间件", icon: "navSettings" },
]

type Dialog =
  | { kind: "confirm"; title: string; description: string; label: string; danger?: boolean; work: () => Promise<boolean | void> }
  | null

type PortModal = {
  reason?: string | null
  initialPort?: string | null
  autoSuggest?: boolean
  retryStart?: boolean
} | null

const RECENT_ROWS = 240
/** 已经提示过的「发现新工具」（只提示一次） */
const SEEN_KEY = "jolu:gateway:seen-tools"

function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key)
    return raw ? (JSON.parse(raw) as T) : fallback
  } catch {
    return fallback
  }
}

function writeJson(key: string, value: unknown) {
  try { localStorage.setItem(key, JSON.stringify(value)) } catch { /* 本地偏好存不进去只影响下次 */ }
}

function statsRangeForOverview(id: OverviewRange["id"]): DateRange {
  const now = new Date()
  if (id === "today") return todayRange(now)
  const shift = (days: number) => {
    const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() + days)
    return dateKey(d)
  }
  if (id === "yesterday") {
    const y = shift(-1)
    return { start: y, end: y }
  }
  return { start: shift(id === "7d" ? -6 : -29), end: dateKey(now) }
}

export interface MoreItem {
  label: string
  danger?: boolean
  onClick: () => void
}

export function GatewayView({ onNavigate, onOpenConnection }: { onNavigate?: (page: MainPage) => void; onOpenConnection?: (connectionId?: string) => void }) {
  const toast = useToast()
  const { connections, reload } = useConnections()
  const sessions = useGatewaySessions()
  const [tab, setTab] = useState<Tab>("overview")
  const [status, setStatus] = useState<GatewayStatusDto | null>(null)
  const [routingGroups, setRoutingGroups] = useState<RoutingGroupDto[]>([])
  const [catalog, setCatalog] = useState<CatalogModelDto[]>([])
  const [day, setDay] = useState<DayOverviewDto | null>(null)
  const [todayStats, setTodayStats] = useState<StatsOverviewDto | null>(null)
  const [activeMiddlewares, setActiveMiddlewares] = useState<number | null>(null)
  const [logRows, setLogRows] = useState<RouteLogDto[]>([])
  /** 每个会话最近一次上下文快照；读取失败为 null，与「还没有快照」的空数组分开 */
  const [contexts, setContexts] = useState<ContextSessionRowDto[] | null>([])
  /** 概览里正在观察的会话；null = 看运行概览 */
  const [observing, setObserving] = useState<{ platform: string; sessionId: string } | null>(null)
  const [sharedLan, setSharedLan] = useState(false)
  const [busy, setBusy] = useState(false)
  const [tick, setTick] = useState(0)
  const [updatedAt, setUpdatedAt] = useState<number | null>(null)
  const [seenTools, setSeenTools] = useState<string[]>(() => readJson<string[]>(SEEN_KEY, []))
  const [discovery, setDiscovery] = useState<AgentDiscoveryDto[]>([])
  const [dialog, setDialog] = useState<Dialog>(null)
  const [portModal, setPortModal] = useState<PortModal>(null)
  /** 中间件「添加规则」在页头；用序号通知面板打开新建表单 */
  const [middlewareAdd, setMiddlewareAdd] = useState(0)
  const operationLock = useRef(false)
  /** 概览页的时间范围：切了就重新拉日概览，轮询沿用同一个值 */
  const [dayRange, setDayRange] = useState<OverviewRange["id"]>("today")
  const dayRangeRef = useRef<OverviewRange["id"]>("today")
  dayRangeRef.current = dayRange

  // 挂载、5 秒轮询、目录 / 连接事件、操作、切换范围后都会调 refresh：只认最新一次的结果
  const refreshGate = useLatestGate()
  const refresh = useCallback(async () => {
    if (!isTauri) return
    const isLatest = refreshGate.begin()
    const range = dayRangeRef.current
    try {
      const statsWindow = rangeToMillis(statsRangeForOverview(range))
      const [nextStatus, nextCatalog, nextSettings, nextDay, nextRows, nextContexts, nextRouting, nextMiddlewares, nextStats] = await Promise.all([
        gatewayApi.status(), gatewayApi.catalog(),
        invoke<{ gateway_shared_lan?: boolean }>("get_settings").catch(() => null),
        gatewayApi.dayOverview(range).catch(() => null),
        gatewayApi.routeLogPage(1, RECENT_ROWS, null).catch(() => null),
        gatewayApi.contextSessions().catch(() => null),
        gatewayApi.groupList().catch(() => [] as RoutingGroupDto[]),
        gatewayApi.middlewares().catch(() => null),
        api.statsOverview({ ...statsWindow, platform: null, connections: [] }).catch(() => null),
      ])
      if (!isLatest()) return
      if (nextSettings) setSharedLan(Boolean(nextSettings.gateway_shared_lan))
      setStatus(nextStatus)
      setCatalog(nextCatalog)
      setDay(nextDay)
      if (nextRows) setLogRows(nextRows.rows)
      setContexts(nextContexts)
      setUpdatedAt(Date.now())
      setRoutingGroups(nextRouting)
      setActiveMiddlewares(nextMiddlewares ? nextMiddlewares.filter((item) => item.enabled).length : null)
      setTodayStats(nextStats)
    } catch (error) {
      if (isLatest()) toast.danger("读取模型路由失败", String(error))
    }
  }, [toast, refreshGate])

  useEffect(() => {
    void refresh()
    if (isTauri) void gatewayApi.agentDiscovery().then(setDiscovery).catch(() => setDiscovery([]))
    const timer = window.setInterval(() => {
      if (!isTauri || document.hidden) return
      void refresh()
      setTick((value) => value + 1)
    }, 5_000)
    // 模型目录、连接或路由请求变了立即重读；5 秒轮询保留作兜底
    let disposed = false
    let routeTimer: number | null = null
    const offs: Array<() => void> = []
    const keep = (off: () => void) => { if (disposed) off(); else offs.push(off) }
    const onReload = () => { void refresh() }
    const onRouteActivity = () => {
      if (routeTimer !== null) return
      routeTimer = window.setTimeout(() => {
        routeTimer = null
        if (!disposed && !document.hidden) void refresh()
      }, 400)
    }
    void listenEvent(EVENTS.GATEWAY_CATALOG_CHANGED, onReload).then(keep)
    void listenEvent(EVENTS.CONNECTIONS_CHANGED, onReload).then(keep)
    void listenEvent(EVENTS.GATEWAY_ROUTE_START, onRouteActivity).then(keep)
    void listenEvent(EVENTS.GATEWAY_ROUTE, onRouteActivity).then(keep)
    return () => {
      window.clearInterval(timer)
      if (routeTimer !== null) window.clearTimeout(routeTimer)
      disposed = true
      offs.forEach((off) => off())
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const run = async (work: () => Promise<unknown>, ok?: string, throwOnFailure = false) => {
    if (operationLock.current) return false
    operationLock.current = true
    setBusy(true)
    try {
      await work()
      if (ok) toast.success(ok)
      await Promise.all([refresh(), reload()])
      return true
    } catch (error) {
      toast.danger("操作失败", String(error))
      if (throwOnFailure) throw error
      return false
    } finally {
      operationLock.current = false
      setBusy(false)
    }
  }

  const agents = status?.wiring ?? []
  const wired = wiredCount(status)
  const running = Boolean(status?.enabled && status.mode === "relay")
  const serving = connections.filter((connection) => connection.serve_enabled)
  const servingSlugs = new Set(serving.map((connection) => connection.slug).filter(Boolean))
  const poolModels = catalog.filter((model) => model.slug && servingSlugs.has(model.slug)).length
  const modelIds = useMemo(() => {
    const ids = routingGroups.map((group) => `group/${group.slug}`)
    for (const model of catalog) if (model.slug && servingSlugs.has(model.slug)) ids.push(`${model.slug}/${model.model}`)
    return [...new Set(ids)]
  }, [routingGroups, catalog, servingSlugs])

  const goAgents = () => onNavigate?.("agents")

  /** 需要关注：工具不再经过模型路由、新装的工具、来源返回限流（R1-01「需要关注」卡） */
  const attention: AttentionItem[] = useMemo(() => {
    const items: AttentionItem[] = []
    for (const entry of agents) {
      if (!entry.drift || entry.drift === "unreachable") continue
      items.push({
        key: `agent:${entry.agent}`,
        kind: "agent",
        title: entry.drift === "model_changed" ? `${agentLabel(entry.agent)} 配置有变化` : `${agentLabel(entry.agent)} 已不经过模型路由`,
        detail: entry.drift === "model_changed" ? `本机模型设置与保存方案不一致（现在是 ${entry.current_model ?? "未知"}）` : `${entry.config_path ?? "配置文件"} · ${driftText(entry.drift)}`,
        action: entry.drift === "model_changed" ? "查看差异" : "去 Agent 页处理",
        onAction: goAgents,
      })
    }
    for (const tool of discovery) {
      const entry = agents.find((item) => item.agent === tool.agent)
      const managed = entry?.wired || (entry?.state && entry.state !== "idle")
      if (tool.state !== "installed" || managed || seenTools.includes(tool.agent)) continue
      items.push({
        key: `tool:${tool.agent}`,
        kind: "tool",
        title: `发现可接入的工具：${tool.name}`,
        detail: `${tool.cli_path ?? "已检测到安装"} · 接入后可用模型池里的 ${modelIds.length} 个模型`,
        action: "去接入",
        onAction: () => {
          const next = [...seenTools, tool.agent]
          setSeenTools(next)
          writeJson(SEEN_KEY, next)
          goAgents()
        },
      })
    }
    const slugNames = new Map(connections.filter((connection) => connection.slug).map((connection) => [connection.slug as string, connection.baseName]))
    for (const limited of rateLimitedSources(logRows)) {
      items.push({
        key: `rate:${limited.source}`,
        kind: "rate",
        title: `${slugNames.get(limited.source) ?? limited.source} 返回限流`,
        detail: `429 · 保留范围内最终失败 ${limited.count} 次`,
        action: "查看流水",
        onAction: () => setTab("log"),
      })
    }
    const risks = contextRisks(activeSessionRows(sessions.views, contexts ?? []), agentLabel).map((risk): AttentionItem => ({
      ...risk,
      kind: "context",
      action: "查看会话上下文",
      onAction: () => { setTab("overview"); setObserving({ platform: risk.platform, sessionId: risk.sessionId }) },
    }))
    return [...risks, ...items]
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agents, discovery, seenTools, modelIds.length, logRows, connections, sessions.views, contexts])

  const toggleRunning = (on: boolean) => {
    const startRelay = async () => {
      if (operationLock.current) return false
      operationLock.current = true
      setBusy(true)
      try {
        await gatewayApi.setMode("relay", status?.default_connection ?? null)
        toast.success("模型路由已开启")
        await Promise.all([refresh(), reload()])
        return true
      } catch (error) {
        if (isPortBindError(error)) {
          setPortModal({ reason: String(error), autoSuggest: true, retryStart: true })
        } else {
          toast.danger("操作失败", String(error))
        }
        return false
      } finally {
        operationLock.current = false
        setBusy(false)
      }
    }
    const stopRelay = (throwOnFailure = false) =>
      run(() => gatewayApi.setMode("observe", status?.default_connection ?? null), "模型路由已停止，接入的工具已恢复原连接", throwOnFailure)
    if (on) {
      void startRelay()
    } else if (wired > 0) {
      setDialog({ kind: "confirm", title: "停止模型路由？", description: `将停止转发，并把 ${wired} 个已接入的工具恢复到原连接。正在进行的调用可能中断。`, label: "停止", danger: true, work: () => stopRelay(true) })
    } else {
      void stopRelay()
    }
  }

  const commitInlinePort = async (raw: string) => {
    const parsed = parseGatewayPort(raw)
    if (!parsed.ok) {
      toast.danger("端口无效", parsed.error)
      setPortModal({ reason: parsed.error, initialPort: raw, autoSuggest: false })
      return
    }
    if (status?.port === parsed.port) return
    if (operationLock.current) return
    operationLock.current = true
    setBusy(true)
    try {
      const outcome = await gatewayApi.setPort(parsed.port, false)
      toast.success(portChangeSuccessText(outcome))
      await Promise.all([refresh(), reload()])
    } catch (error) {
      const text = String(error)
      toast.danger("修改端口失败", text)
      setPortModal({ reason: text, initialPort: raw, autoSuggest: isPortBindError(error) })
    } finally {
      operationLock.current = false
      setBusy(false)
    }
  }

  const escape = () => setDialog({
    kind: "confirm",
    title: "紧急切回直连？",
    description: "还原全部已接入工具的配置并停止模型路由；账号、Key 与历史记录不会删除。",
    label: "切回直连",
    danger: true,
    work: () => run(() => gatewayApi.escape(), "已切回直连", true),
  })

  const setShared = (on: boolean) => {
    setSharedLan(on)
    void run(() => gatewayApi.setSharedLan(on), on ? "已开启局域网共享（重启接入后监听生效）" : "已关闭局域网共享", true).then((ok) => { if (!ok) setSharedLan(!on) }).catch(() => setSharedLan(!on))
  }

  const openSources = () => {
    if (onOpenConnection) onOpenConnection()
    else onNavigate?.("providers")
  }

  if (!isTauri) {
    return <div className="flex h-full items-center justify-center text-[12px] text-jn-text-3">模型路由需要在桌面端使用</div>
  }

  const moreItems: MoreItem[] = [
    { label: "修改监听端口", onClick: () => setPortModal({}) },
    { label: "管理来源", onClick: openSources },
    { label: "去 Agent 页接入工具", onClick: goAgents },
    ...(canEscape(status) ? [{ label: "紧急切回直连", danger: true, onClick: escape }] : []),
  ]

  return (
    <>
      <PageTabs wide>
        <Tabs
          value={tab}
          onChange={setTab}
          iconMode="active"
          items={TABS.map(({ id, label, icon }) => ({ key: id, label, icon }))}
        />
      </PageTabs>
      {tab === "middleware" && (
        <PageActions>
          <Button kind="primary" size="sm" icon="add" disabled={busy} onClick={() => setMiddlewareAdd((value) => value + 1)}>添加规则</Button>
        </PageActions>
      )}

      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3.5 overflow-y-auto px-0.5 pb-2 pt-0.5 [&>*]:shrink-0">
        {tab === "overview" && observing && (
          <SessionObservation
            key={`${observing.platform}:${observing.sessionId}`}
            platform={observing.platform}
            sessionId={observing.sessionId}
            view={sessions.views.find((item) => item.session.platform === observing.platform && item.session.session_id === observing.sessionId) ?? null}
            groupName={(slug) => routingGroups.find((group) => group.slug === slug)?.name ?? null}
            onBack={() => setObserving(null)}
          />
        )}
        {tab === "overview" && !observing && (
          <OverviewPanel
            status={status}
            running={running}
            wired={wired}
            day={day}
            todayStats={todayStats}
            activeMiddlewares={activeMiddlewares}
            connections={connections}
            defaultConnection={status?.default_connection ?? null}
            range={dayRange}
            onRange={(next) => {
              setDayRange(next)
              dayRangeRef.current = next
              void refresh()
            }}
            sessions={sessions.views}
            inFlightCount={sessions.inFlightCount}
            contexts={contexts}
            logRows={logRows}
            nowMs={Date.now()}
            updatedAt={updatedAt}
            sources={serving.length}
            models={poolModels}
            groups={routingGroups.length}
            routingGroups={routingGroups}
            attention={attention}
            busy={busy}
            sharedLan={sharedLan}
            onSharedLan={setShared}
            onSelectStrategy={(group, nextStrategy) => {
              if (nextStrategy === "intent" && !group.intent?.classifier) {
                setTab("groups")
                return
              }
              void run(
                () => gatewayApi.groupUpdate(group.slug, { strategy: nextStrategy, members: group.members }),
                `已将 group/${group.slug} 切换为「${nextStrategy === "usage" ? "额度优先" : nextStrategy === "order" ? "按顺序" : "按意图"}」`,
              )
            }}
            onToggleRunning={() => toggleRunning(!running)}
            onInlinePortCommit={commitInlinePort}
            onOpenPortDialog={() => setPortModal({})}
            onOpenQuota={(connectionId) => {
              if (connectionId && onOpenConnection) onOpenConnection(connectionId)
              else onNavigate?.("providers")
            }}
            onOpenMiddleware={() => setTab("middleware")}
            onOpenGroups={() => setTab("groups")}
            onRefresh={() => void refresh()}
            onOpenLog={() => setTab("log")}
            onOpenContext={(platform, sessionId) => setObserving({ platform, sessionId })}
          />
        )}
        {tab !== "overview" && (
          <GatewayStatusBar
            port={status?.port ?? null}
            running={running}
            wired={wired}
            busy={busy}
            onToggleRunning={() => toggleRunning(!running)}
            onOpenPortDialog={() => setPortModal({})}
            moreItems={moreItems}
          />
        )}
        {tab === "log" && <RouteLogPanel desktop={isTauri} connections={connections} tick={tick} />}
        {tab === "groups" && (
          <GroupsPanel catalog={catalog} connections={connections} busy={busy} run={(work, title, throwOnFailure) => run(work, title, throwOnFailure)} onChanged={() => void refresh()} />
        )}
        {tab === "pool" && (
          <PoolPanel
            connections={connections}
            catalog={catalog}
            defaultConnection={status?.default_connection ?? null}
            busy={busy}
            run={run}
            onOpenConnection={(id) => onOpenConnection?.(id)}
            onManage={openSources}
          />
        )}
        {tab === "access" && (
          <AccessPanel port={status?.port ?? 0} shared={sharedLan} busy={busy} onSharedChange={setShared} run={(work, title) => run(work, title)} models={modelIds} />
        )}
        {tab === "middleware" && (
          <MiddlewarePanel busy={busy} addSignal={middlewareAdd} run={(work, title) => run(work, title)} onOpenPrivacy={() => onNavigate?.("settings")} />
        )}
      </div>

      {portModal && (
        <GatewayPortDialog
          open
          currentPort={status?.port ?? null}
          wired={wired}
          reason={portModal.reason}
          initialPort={portModal.initialPort}
          autoSuggest={portModal.autoSuggest}
          confirmLabel={portModal.retryStart ? "保存并开启中转" : undefined}
          onOpenChange={(open) => { if (!open) setPortModal(null) }}
          onSaved={async (outcome) => {
            const shouldStart = Boolean(portModal.retryStart)
            toast.success(portChangeSuccessText(outcome))
            await Promise.all([refresh(), reload()])
            if (shouldStart) toggleRunning(true)
          }}
        />
      )}

      {dialog?.kind === "confirm" && (
        <ConfirmDialog
          open
          onOpenChange={(open) => { if (!open) setDialog(null) }}
          title={dialog.title}
          description={dialog.description}
          confirmText={dialog.label}
          kind={dialog.danger ? "danger" : "primary"}
          loading={busy}
          onConfirm={() => { const current = dialog; setDialog(null); void current.work() }}
        />
      )}
    </>
  )
}

/** 五页签共用的紧凑状态条（R1-18～22「路由状态条」：56 高、圆角 20）。 */
function GatewayStatusBar({ port, running, wired, busy, onToggleRunning, onOpenPortDialog, moreItems }: {
  port: number | null
  running: boolean
  wired: number
  busy: boolean
  onToggleRunning: () => void
  onOpenPortDialog: () => void
  moreItems: MoreItem[]
}) {
  return (
    <section className="flex min-h-14 shrink-0 flex-wrap items-center gap-x-3 gap-y-2 rounded-[20px] border border-jn-line bg-jn-surface py-2 pr-3.5 pl-2.5 shadow-jn-surface">
      <span className="flex size-[34px] shrink-0 items-center justify-center rounded-[12px] bg-jn-primary-soft">
        <Icon name="navGateway" size="md" className="text-jn-primary" />
      </span>
      <span className="shrink-0 text-[14px] font-bold text-jn-text">本机路由</span>
      <button
        type="button"
        disabled={busy || port == null}
        title="点击修改监听端口"
        onClick={onOpenPortDialog}
        className="shrink-0 cursor-pointer rounded-[6px] px-1.5 py-0.5 font-mono text-[12.5px] font-medium text-jn-text-2 transition-colors hover:bg-jn-well hover:text-jn-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jn-focus disabled:cursor-default disabled:hover:bg-transparent"
      >
        127.0.0.1:{port ?? "—"}
      </button>
      <StatusPill tone={running ? "success" : "neutral"}>{running ? "正在中转" : "已停止"}</StatusPill>
      <span className="min-w-0 flex-1" />
      <span className="shrink-0 text-[12px] text-jn-text-3">{wired} 个工具已接入</span>
      <Button kind={running ? "ghost" : "primary"} icon={running ? "pause" : "play"} disabled={busy} onClick={onToggleRunning}>
        {running ? "停止中转" : "开启中转"}
      </Button>
      <MoreMenu items={moreItems} />
    </section>
  )
}

/** 「更多」菜单：点外 / Escape 关闭，不使用旧 `.jolu-menu` 与 `gw-pop`。 */
export function MoreMenu({ items }: { items: MoreItem[] }) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    if (!open) return
    const onDown = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") setOpen(false) }
    document.addEventListener("mousedown", onDown)
    document.addEventListener("keydown", onKey)
    return () => {
      document.removeEventListener("mousedown", onDown)
      document.removeEventListener("keydown", onKey)
    }
  }, [open])
  return (
    <div ref={rootRef} className="relative shrink-0" data-open={open}>
      <IconButton icon="more" label="更多操作" kind="surface" aria-expanded={open} onClick={() => setOpen((value) => !value)} />
      {open && (
        <div role="menu" className="absolute top-10 right-0 z-20 w-48 rounded-[14px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface">
          {items.map((item) => (
            <button key={item.label} type="button" role="menuitem" onClick={() => { setOpen(false); item.onClick() }}
              className={cn("w-full rounded-[10px] px-2.5 py-1.5 text-left text-[12px] hover:bg-jn-well", item.danger ? "text-jn-danger" : "text-jn-text")}>
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

export type { GatewayMode } from "@/lib/gatewayApi"
