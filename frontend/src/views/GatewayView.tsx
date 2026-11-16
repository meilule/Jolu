import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { Activity, GitFork, Layers, ListFilter, Network } from "lucide-react"
import { cn } from "@/lib/utils"
import { api, invoke, isTauri, listenEvent } from "@/lib/api"
import { EVENTS } from "@/lib/events"
import { useLatestGate } from "@/hooks/useLatestGate"
import {
  gatewayApi, type AgentDiscoveryDto, type CatalogModelDto, type DayOverviewDto, type GatewayGroupsDto,
  type GatewayStatusDto, type RouteLogDto, type SourceCapabilityDto,
} from "@/lib/gatewayApi"
import type { RoutingGroupDto } from "@/components/gateway/routingGroups"
import { agentLabel, canEscape, wiredCount } from "@/lib/gatewayPage"
import { driftText } from "@/lib/agentPage"
import { rateLimitedSources } from "@/lib/gatewayOverview"
import type { MainPage } from "@/lib/mainNavigation"
import { useConnections } from "@/hooks/useConnections"
import { useGatewaySessions } from "@/hooks/useGatewaySessions"
import { useToast } from "@/components/ui/Toast"
import { ConfirmDialog as ActionConfirmDialog } from "@/components/ui/ConfirmDialog"
import { RouteLogPanel } from "@/components/gateway/RouteLogPanel"
import { PanelPage } from "@/components/panel/PanelPage"
import { OverviewPanel, type AttentionItem } from "@/components/gateway/OverviewPanel"
import { GroupsPanel } from "@/components/gateway/GroupsPanel"
import { PoolPanel } from "@/components/gateway/PoolPanel"
import { AccessPanel } from "@/components/gateway/AccessPanel"
import { MiddlewarePanel } from "@/components/gateway/MiddlewarePanel"
import { CatalogDialog, RenameDialog } from "@/components/gateway/GatewayDialogs"
import { SourceManageDialog } from "@/components/gateway/SourceManageDialog"
import type { Connection } from "@/types"

/**
 * 模型路由（原「统一网关」；Jolu.pen「统一网关 · R2 重设计」V6）：
 * 顶部「本机路由」状态条（地址、运行 pill、N 个工具已接入、停止中转、更多）+
 * 六个页签：概览（cb7KY/Cktyu）、路由流水（无画板，沿用 V5 面板）、路由分组（fgm9F）、
 * 模型池（gaGlT）、对外访问（RMna2）、中间件（wEsPa）。
 * 工具的接入与档位在「Agent」页；模型池只读，加入 / 移出与前缀在供应商（V5-5）。
 */

type Tab = "overview" | "log" | "groups" | "pool" | "access" | "middleware"

const TABS: readonly { id: Tab; label: string; icon: React.ElementType }[] = [
  { id: "overview", label: "概览", icon: Activity },
  { id: "log", label: "路由流水", icon: ListFilter },
  { id: "groups", label: "路由分组", icon: GitFork },
  { id: "pool", label: "模型池", icon: Layers },
  { id: "access", label: "对外访问", icon: Network },
  { id: "middleware", label: "中间件", icon: SlidersIcon },
]

type Dialog =
  | { kind: "manage" }
  | { kind: "rename"; connection: Connection }
  | { kind: "catalog"; models?: CatalogModelDto[] }
  | { kind: "confirm"; title: string; description: string; label: string; danger?: boolean; work: () => Promise<boolean | void> }
  | null

const EMPTY_GROUPS: GatewayGroupsDto = { sources: [], agents: [] }
const RECENT_ROWS = 240
const PIN_KEY = "jolu:gateway:pinned"
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

/** 中间件页签的图标（lucide 没有贴近的滑杆图形，自绘） */
function SlidersIcon({ className }: { className?: string }) {
  return (
    <svg aria-hidden viewBox="0 0 16 16" className={cn("size-4", className)} fill="none" stroke="currentColor" strokeWidth="1.5">
      <path d="M3 5h6M11.5 5H13M3 11h1.5M7 11h6" strokeLinecap="round" />
      <circle cx="10" cy="5" r="1.6" /><circle cx="5.5" cy="11" r="1.6" />
    </svg>
  )
}

export function GatewayView({ onNavigate, onOpenConnection }: { onNavigate?: (page: MainPage) => void; onOpenConnection?: (connectionId: string) => void }) {
  const toast = useToast()
  const { connections, reload } = useConnections()
  const sessions = useGatewaySessions()
  const [tab, setTab] = useState<Tab>("overview")
  const [status, setStatus] = useState<GatewayStatusDto | null>(null)
  const [groups, setGroups] = useState<GatewayGroupsDto>(EMPTY_GROUPS)
  const [routingGroups, setRoutingGroups] = useState<RoutingGroupDto[]>([])
  const [capabilities, setCapabilities] = useState<SourceCapabilityDto[]>([])
  const [catalog, setCatalog] = useState<CatalogModelDto[]>([])
  const [day, setDay] = useState<DayOverviewDto | null>(null)
  const [logRows, setLogRows] = useState<RouteLogDto[]>([])
  const [middlewaresCount, setMiddlewaresCount] = useState<number | null>(null)
  const [sharedLan, setSharedLan] = useState(false)
  const [busy, setBusy] = useState(false)
  const [tick, setTick] = useState(0)
  const [updatedAt, setUpdatedAt] = useState<number | null>(null)
  const [pinned, setPinned] = useState<string[]>(() => readJson<string[]>(PIN_KEY, []))
  const [seenTools, setSeenTools] = useState<string[]>(() => readJson<string[]>(SEEN_KEY, []))
  const [discovery, setDiscovery] = useState<AgentDiscoveryDto[]>([])
  const [dialog, setDialog] = useState<Dialog>(null)
  const operationLock = useRef(false)

  // 挂载、5 秒轮询、目录 / 连接事件、操作后都会调 refresh：只认最新一次的结果
  const refreshGate = useLatestGate()
  const refresh = useCallback(async () => {
    if (!isTauri) return
    const isLatest = refreshGate.begin()
    try {
      const [nextStatus, nextGroups, nextCatalog, nextCapabilities, nextSettings, nextDay, nextRows, nextMiddlewares, nextRouting] = await Promise.all([
        gatewayApi.status(), gatewayApi.groups(), gatewayApi.catalog(), gatewayApi.sourceCapabilities(),
        invoke<{ gateway_shared_lan?: boolean }>("get_settings").catch(() => null),
        gatewayApi.dayOverview().catch(() => null),
        gatewayApi.routeLogPage(1, RECENT_ROWS, null).catch(() => null),
        gatewayApi.middlewares().catch(() => null),
        gatewayApi.groupList().catch(() => [] as RoutingGroupDto[]),
      ])
      if (!isLatest()) return
      if (nextSettings) setSharedLan(Boolean(nextSettings.gateway_shared_lan))
      setStatus(nextStatus)
      setGroups(nextGroups)
      setCatalog(nextCatalog)
      setCapabilities(nextCapabilities)
      setDay(nextDay)
      if (nextRows) setLogRows(nextRows.rows)
      setMiddlewaresCount(nextMiddlewares ? nextMiddlewares.filter((row) => row.enabled).length : null)
      setUpdatedAt(Date.now())
      setRoutingGroups(nextRouting)
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
    // 模型目录或连接变了立即重读；5 秒轮询保留作兜底
    let disposed = false
    const offs: Array<() => void> = []
    const keep = (off: () => void) => { if (disposed) off(); else offs.push(off) }
    const reload = () => { void refresh() }
    void listenEvent(EVENTS.GATEWAY_CATALOG_CHANGED, reload).then(keep)
    void listenEvent(EVENTS.CONNECTIONS_CHANGED, reload).then(keep)
    return () => { window.clearInterval(timer); disposed = true; offs.forEach((off) => off()) }
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

  /** 需要关注：工具不再经过模型路由、新装的工具、来源返回限流（V6 概览画板「需要关注」卡） */
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
    return items
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agents, discovery, seenTools, modelIds.length, logRows, connections])

  const toggleRunning = (on: boolean) => {
    const work = (throwOnFailure = false) => run(() => gatewayApi.setMode(on ? "relay" : "observe", status?.default_connection ?? null), on ? "模型路由已开启" : "模型路由已停止，接入的工具已恢复原连接", throwOnFailure)
    if (!on && wired > 0) setDialog({ kind: "confirm", title: "停止模型路由？", description: `将停止转发，并把 ${wired} 个已接入的工具恢复到原连接。正在进行的调用可能中断。`, label: "停止", danger: true, work: () => work(true) })
    else void work()
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

  const togglePin = (key: string) => {
    setPinned((current) => {
      const next = current.includes(key) ? current.filter((item) => item !== key) : [...current, key]
      writeJson(PIN_KEY, next)
      return next
    })
  }

  const dayStart = useMemo(() => {
    const now = new Date()
    return new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  }, [])

  if (!isTauri) {
    return <div className="flex h-full items-center justify-center text-meta text-jolu-muted">模型路由需要在桌面端使用</div>
  }

  return (
    <PanelPage className="flex-col">
      {/* 状态条（V6：所有页签共用） */}
      <div className="flex shrink-0 flex-col gap-3 px-8 pt-5">
        <div className="flex items-center gap-3.5">
          <span className={cn("flex size-11 shrink-0 items-center justify-center rounded-[11px]", running ? "bg-jolu-selected" : "bg-jolu-side")}>
            <RouteGlyph className={running ? "text-jolu-on" : "text-jolu-muted"} />
          </span>
          <span className="text-[15px] font-semibold text-jolu-text">本机路由</span>
          <span className="font-mono text-meta text-jolu-muted">{`127.0.0.1:${status?.port ?? "—"}`}</span>
          <span className={cn(
            "flex h-7 items-center gap-1.5 rounded-full px-2.5 text-note font-semibold",
            running ? "bg-jolu-selected text-jolu-on" : "bg-jolu-track text-jolu-muted",
          )}>
            <span aria-hidden className={cn("size-1.5 rounded-full", running ? "bg-jolu-on" : "bg-jolu-muted")} />
            {running ? "正在中转" : "已停止"}
          </span>
          <span className="flex-1" />
          <span className="shrink-0 text-meta text-jolu-muted">{wired} 个工具已接入</span>
          <button type="button" disabled={busy} onClick={() => toggleRunning(!running)}
            className="flex h-10 shrink-0 items-center gap-2 rounded-lg border border-jolu-border px-4 text-meta font-medium text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
            <StopGlyph on={running} />
            {running ? "停止中转" : "开启中转"}
          </button>
          <div className="relative shrink-0">
            <MoreMenu
              items={[
                { label: "管理来源", onClick: () => setDialog({ kind: "manage" }) },
                { label: "去 Agent 页接入工具", onClick: goAgents },
                ...(canEscape(status) ? [{ label: "紧急切回直连", danger: true, onClick: escape }] : []),
              ]}
            />
          </div>
        </div>
        <TabBar tab={tab} onTab={setTab} />
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto [&>*]:shrink-0">
        {tab === "overview" && (
          <OverviewPanel
            day={day}
            sessions={sessions.views}
            logRows={logRows}
            dayStartMs={dayStart}
            nowMs={Date.now()}
            updatedAt={updatedAt}
            sources={serving.length}
            models={poolModels}
            groups={routingGroups.length}
            middlewares={middlewaresCount}
            attention={attention}
            busy={busy}
            onRefresh={() => void refresh()}
            onOpenLog={() => setTab("log")}
          />
        )}
        {tab === "log" && (
          <div className="flex min-w-0 flex-1 flex-col gap-4 px-8 pb-8 pt-2">
            <div className="flex flex-col gap-1.5">
              <h2 className="text-[20px] font-semibold text-jolu-text">路由流水</h2>
              <p className="text-meta text-jolu-muted">每次请求选了哪个来源、改道几次、结果如何（本页签暂无 V6 画板，沿用 V5 面板）</p>
            </div>
            <RouteLogPanel desktop={isTauri} connections={connections} tick={tick} />
          </div>
        )}
        {tab === "groups" && (
          <GroupsPanel catalog={catalog} busy={busy} run={(work, title, throwOnFailure) => run(work, title, throwOnFailure)} onChanged={() => void refresh()} />
        )}
        {tab === "pool" && (
          <PoolPanel connections={connections} catalog={catalog} defaultConnection={status?.default_connection ?? null}
            onOpenConnection={(id) => onOpenConnection?.(id)} onManage={() => setDialog({ kind: "manage" })} />
        )}
        {tab === "access" && (
          <AccessPanel port={status?.port ?? 0} shared={sharedLan} busy={busy} onSharedChange={setShared} run={(work, title) => run(work, title)} models={modelIds} />
        )}
        {tab === "middleware" && (
          <MiddlewarePanel busy={busy} run={(work, title) => run(work, title)} onOpenPrivacy={() => onNavigate?.("settings")} />
        )}
      </div>

      {dialog?.kind === "manage" && (
        <SourceManageDialog connections={connections} capabilities={capabilities} catalog={catalog} groups={groups} agents={agents}
          pinned={pinned} busy={busy} run={run} onClose={() => setDialog(null)}
          onRename={(connection) => setDialog({ kind: "rename", connection })}
          onShowModels={(models) => setDialog({ kind: "catalog", models })}
          onTogglePin={togglePin}
          onWire={goAgents} onUnwire={goAgents} onAccept={goAgents} />
      )}
      {dialog?.kind === "rename" && (
        <RenameDialog connection={dialog.connection} busy={busy} onClose={() => setDialog({ kind: "manage" })} onSave={async (name, slug) => {
          const ok = await run(async () => {
            if (name !== dialog.connection.baseName) await api.renameConnection(dialog.connection.id, name)
            if (slug && slug !== dialog.connection.slug) await gatewayApi.setSlug(dialog.connection.id, slug)
          }, "已保存")
          if (ok) setDialog({ kind: "manage" })
        }} />
      )}
      {dialog?.kind === "catalog" && <CatalogDialog models={dialog.models ?? catalog} onClose={() => setDialog({ kind: "manage" })} />}
      {dialog?.kind === "confirm" && <ActionConfirmDialog title={dialog.title} description={dialog.description} confirmLabel={dialog.label} danger={dialog.danger} onClose={() => setDialog(null)} onConfirm={dialog.work} />}
    </PanelPage>
  )
}

/** 页签条（V6 画板：浅底圆角容器，选中项白底描边 + 品牌色文字）。 */
function TabBar({ tab, onTab }: { tab: Tab; onTab: (tab: Tab) => void }) {
  return (
    <div className="flex items-center gap-1 rounded-xl border border-jolu-border bg-jolu-side p-1" role="tablist" aria-label="模型路由">
      {TABS.map(({ id, label, icon: Icon }) => {
        const active = tab === id
        return (
          <button key={id} type="button" role="tab" aria-selected={active} onClick={() => onTab(id)}
            className={cn(
              "flex h-10 flex-1 items-center justify-center gap-2 rounded-[9px] text-meta outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
              active ? "border border-jolu-border bg-jolu-bg font-semibold text-jolu-on shadow-sm" : "text-jolu-muted hover:text-jolu-text",
            )}>
            {id === "middleware" ? <SlidersIcon className="size-3.5" /> : <Icon aria-hidden className="size-3.5" />}
            {label}
          </button>
        )
      })}
    </div>
  )
}

function RouteGlyph({ className }: { className?: string }) {
  return (
    <svg aria-hidden viewBox="0 0 20 20" className={cn("size-5", className)} fill="none" stroke="currentColor" strokeWidth="1.5">
      <circle cx="5" cy="5" r="2.2" /><circle cx="15" cy="10" r="2.2" /><circle cx="5" cy="15" r="2.2" />
      <path d="M7 6.2L12.9 9M7 13.8L12.9 11" strokeLinecap="round" />
    </svg>
  )
}

function StopGlyph({ on }: { on: boolean }) {
  return on ? (
    <svg aria-hidden viewBox="0 0 14 14" className="size-3.5 text-jolu-muted" fill="none" stroke="currentColor" strokeWidth="1.5">
      <rect x="3" y="3" width="8" height="8" rx="1.5" />
    </svg>
  ) : (
    <svg aria-hidden viewBox="0 0 14 14" className="size-3.5 text-jolu-muted" fill="none" stroke="currentColor" strokeWidth="1.5">
      <path d="M4 2.5L11.5 7L4 11.5V2.5z" strokeLinejoin="round" />
    </svg>
  )
}

interface MoreMenuItem {
  label: string
  danger?: boolean
  onClick: () => void
}

function MoreMenu({ items }: { items: MoreMenuItem[] }) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button type="button" aria-label="更多操作" aria-expanded={open} onClick={() => setOpen((value) => !value)}
        className="flex h-10 w-11 items-center justify-center gap-1 rounded-lg border border-jolu-border outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
        <span className="text-meta font-semibold text-jolu-muted">···</span>
        <span className="text-meta font-medium text-jolu-text">更多</span>
      </button>
      {open && (
        <div role="menu" className="absolute right-0 top-12 z-20 w-48 rounded-lg border border-jolu-border bg-jolu-bg p-1 shadow-lg">
          {items.map((item) => (
            <button key={item.label} type="button" role="menuitem" onClick={() => { setOpen(false); item.onClick() }}
              className={cn("w-full rounded-md px-2.5 py-1.5 text-left text-meta hover:bg-jolu-side", item.danger ? "text-jolu-danger" : "text-jolu-text")}>
              {item.label}
            </button>
          ))}
        </div>
      )}
    </>
  )
}

export type { GatewayMode } from "@/lib/gatewayApi"
