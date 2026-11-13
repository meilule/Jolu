import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { Activity, GitFork, Layers } from "lucide-react"
import { api, invoke, isTauri } from "@/lib/api"
import {
  gatewayApi, type AgentDiscoveryDto, type CatalogModelDto, type DayOverviewDto, type GatewayGroupsDto, type GatewayMode,
  type GatewayStatusDto, type RouteLogDto, type SourceCapabilityDto,
} from "@/lib/gatewayApi"
import { agentLabel, canEscape, wiredCount } from "@/lib/gatewayPage"
import { driftText } from "@/lib/agentPage"
import type { MainPage } from "@/lib/mainNavigation"
import { useConnections } from "@/hooks/useConnections"
import { useGatewaySessions } from "@/hooks/useGatewaySessions"
import { useToast } from "@/components/ui/Toast"
import { ConfirmDialog as ActionConfirmDialog } from "@/components/ui/ConfirmDialog"
import { MiddlewareDialog } from "@/components/gateway/MiddlewareDialog"
import { RouteLogPanel } from "@/components/gateway/RouteLogPanel"
import { RoutingGroupsSection } from "@/components/gateway/RoutingGroupsSection"
import { RoutingObjects, type RoutingObject } from "@/components/gateway/RoutingObjects"
import { RoutingOverview, type AttentionItem } from "@/components/gateway/RoutingOverview"
import { ExternalAccess } from "@/components/gateway/ExternalAccess"
import { ModelPoolPanel } from "@/components/gateway/ModelPoolPanel"
import { CatalogDialog, RenameDialog } from "@/components/gateway/GatewayDialogs"
import { SourceManageDialog } from "@/components/gateway/SourceManageDialog"
import type { Connection } from "@/types"

/**
 * 模型路由（原「统一网关」；Jolu.pen「统一网关 · R2 重设计」V5-3 概览瘦身 / V5-4 对外访问）。
 *
 * 只管请求怎么走：左栏对象清单（网关卡、概览、路由流水、Agent 入口、路由分组、模型池、对外访问、中间件、
 * 紧急切回直连），右侧检查器按选中的对象展示。工具的接入与档位在「Agent」页；模型池只读，
 * 加入 / 移出与前缀在供应商（V5-5）。深色稿待定稿后补。
 */

type Dialog =
  | { kind: "manage" }
  | { kind: "rename"; connection: Connection }
  | { kind: "catalog"; models?: CatalogModelDto[] }
  | { kind: "confirm"; title: string; description: string; label: string; danger?: boolean; work: () => Promise<boolean | void> }
  | null

const EMPTY_GROUPS: GatewayGroupsDto = { sources: [], agents: [] }
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

export function GatewayView({ onNavigate, onOpenConnection }: { onNavigate?: (page: MainPage) => void; onOpenConnection?: (connectionId: string) => void }) {
  const toast = useToast()
  const { connections, reload } = useConnections()
  const sessions = useGatewaySessions()
  const [status, setStatus] = useState<GatewayStatusDto | null>(null)
  const [groups, setGroups] = useState<GatewayGroupsDto>(EMPTY_GROUPS)
  const [routingGroups, setRoutingGroups] = useState<{ slug: string }[]>([])
  const [capabilities, setCapabilities] = useState<SourceCapabilityDto[]>([])
  const [catalog, setCatalog] = useState<CatalogModelDto[]>([])
  const [day, setDay] = useState<DayOverviewDto | null>(null)
  const [routes, setRoutes] = useState<RouteLogDto[]>([])
  const [middlewares, setMiddlewares] = useState<number | null>(null)
  const [keyCount, setKeyCount] = useState<number | null>(null)
  const [discovery, setDiscovery] = useState<AgentDiscoveryDto[]>([])
  const [sharedLan, setSharedLan] = useState(false)
  const [busy, setBusy] = useState(false)
  const [tick, setTick] = useState(0)
  const [selected, setSelected] = useState<RoutingObject>({ kind: "overview" })
  const [createNonce, setCreateNonce] = useState(0)
  const [pinned, setPinned] = useState<string[]>(() => readJson<string[]>(PIN_KEY, []))
  const [seenTools, setSeenTools] = useState<string[]>(() => readJson<string[]>(SEEN_KEY, []))
  const [dialog, setDialog] = useState<Dialog>(null)
  const operationLock = useRef(false)

  const refresh = useCallback(async () => {
    if (!isTauri) return
    try {
      const [nextStatus, nextGroups, nextCatalog, nextCapabilities, nextSettings, nextDay, nextRoutes, nextMiddlewares, nextKeys, nextRouting] = await Promise.all([
        gatewayApi.status(), gatewayApi.groups(), gatewayApi.catalog(), gatewayApi.sourceCapabilities(),
        invoke<{ gateway_shared_lan?: boolean }>("get_settings").catch(() => null),
        gatewayApi.dayOverview().catch(() => null),
        gatewayApi.routeLog().catch(() => [] as RouteLogDto[]),
        gatewayApi.middlewares().catch(() => null),
        gatewayApi.clientKeys().catch(() => null),
        gatewayApi.groupList().catch(() => []),
      ])
      if (nextSettings) setSharedLan(Boolean(nextSettings.gateway_shared_lan))
      setStatus(nextStatus)
      setGroups(nextGroups)
      setCatalog(nextCatalog)
      setCapabilities(nextCapabilities)
      setDay(nextDay)
      setRoutes(nextRoutes)
      setMiddlewares(nextMiddlewares ? nextMiddlewares.filter((row) => row.enabled).length : null)
      setKeyCount(nextKeys ? nextKeys.length : null)
      setRoutingGroups(nextRouting.map((group) => ({ slug: group.slug })))
    } catch (error) {
      toast.danger("读取模型路由失败", String(error))
    }
  }, [toast])

  useEffect(() => {
    void refresh()
    if (isTauri) void gatewayApi.agentDiscovery().then(setDiscovery).catch(() => setDiscovery([]))
    const timer = window.setInterval(() => {
      if (!isTauri || document.hidden) return
      void refresh()
      setTick((value) => value + 1)
    }, 15_000)
    return () => window.clearInterval(timer)
  }, [refresh])

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

  /** 需要你处理（没有异常时整块不出现）：工具不再经过模型路由、新装的工具 */
  const attention: AttentionItem[] = useMemo(() => {
    const items: AttentionItem[] = []
    for (const entry of agents) {
      if (!entry.drift || entry.drift === "unreachable") continue
      items.push({
        key: `agent:${entry.agent}`,
        kind: "agent",
        title: entry.drift === "model_changed" ? `${agentLabel(entry.agent)} 的主模型在工具里换过` : `${agentLabel(entry.agent)} 已不经过模型路由`,
        detail: entry.drift === "model_changed" ? `现在是 ${entry.current_model ?? "未知"}；跟随档位还停在旧模型` : `${entry.config_path ?? "配置文件"} · ${driftText(entry.drift)}`,
        action: "去 Agent 页处理",
        onAction: goAgents,
      })
    }
    for (const tool of discovery) {
      const entry = agents.find((item) => item.agent === tool.agent)
      // 接入过的（含被外部改动、等用户处理的）不算「新发现」
      const managed = entry?.wired || (entry?.state && entry.state !== "idle")
      if (tool.state !== "installed" || managed || seenTools.includes(tool.agent)) continue
      items.push({
        key: `tool:${tool.agent}`,
        kind: "tool",
        title: `发现可接入的工具：${tool.name}`,
        detail: `${tool.cli_path ?? "已检测到安装"} · 接入后可用模型池里的 ${modelIds.length} 个模型`,
        action: "去 Agent 页接入",
        onAction: () => {
          const next = [...seenTools, tool.agent]
          setSeenTools(next)
          writeJson(SEEN_KEY, next)
          goAgents()
        },
      })
    }
    return items
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agents, discovery, seenTools, modelIds.length])

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

  const header = (icon: React.ElementType, title: string, note: string) => {
    const Icon = icon
    return (
      <div className="flex items-center gap-3">
        <span className="flex size-10 shrink-0 items-center justify-center rounded-[10px] bg-jolu-side"><Icon aria-hidden className="size-4 text-jolu-muted" /></span>
        <span className="flex min-w-0 flex-col gap-1">
          <span className="text-[17px] font-semibold text-jolu-text">{title}</span>
          <span className="text-meta text-jolu-muted">{note}</span>
        </span>
      </div>
    )
  }

  const inspector = () => {
    switch (selected.kind) {
      case "overview":
        return (
          <RoutingOverview day={day} agents={wired} middlewares={middlewares} groups={routingGroups.length} sources={serving.length} models={poolModels}
            attention={attention} sessions={sessions.views} routes={routes} onOpenAgents={goAgents} onOpenSessions={() => onNavigate?.("sessions")}
            onOpenLog={() => setSelected({ kind: "log" })} />
        )
      case "log":
        return (
          <div className="flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto px-7 py-5">
            {header(Activity, "路由流水", "每次请求选了哪个来源、改道几次、结果如何")}
            <RouteLogPanel desktop={isTauri} connections={connections} tick={tick} />
          </div>
        )
      case "groups":
        return (
          <div className="flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto px-7 py-5">
            {header(GitFork, selected.slug ? `group/${selected.slug}` : "路由分组", "多个「来源/模型」组成一个模型 id，按策略分摊与失败转移")}
            <RoutingGroupsSection embedded catalog={catalog} busy={busy} run={run} focus={selected.slug} createNonce={createNonce} onChanged={() => void refresh()} />
          </div>
        )
      case "pool":
        return (
          <ModelPoolPanel connections={connections} catalog={catalog} defaultConnection={status?.default_connection ?? null}
            onOpenConnection={(id) => onOpenConnection?.(id)} onManage={() => setDialog({ kind: "manage" })} />
        )
      case "access":
        return <ExternalAccess port={status?.port ?? 0} shared={sharedLan} busy={busy} onSharedChange={setShared} run={(work, title) => run(work, title)} models={modelIds} />
      case "middleware":
        return (
          <div className="flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto px-7 py-5">
            {header(Layers, "中间件", "选路前改写请求：模型改名、参数覆盖、系统提示、敏感词、思考段")}
            <MiddlewareDialog inline busy={busy} onClose={() => undefined} run={(work, title) => run(work, title)} />
          </div>
        )
    }
  }

  if (!isTauri) {
    return <div className="flex h-full items-center justify-center text-meta text-jolu-muted">模型路由需要在桌面端使用</div>
  }

  return (
    <div className="flex h-full min-h-0 overflow-hidden bg-jolu-bg">
      <RoutingObjects
        selected={selected}
        onSelect={setSelected}
        running={running}
        address={`127.0.0.1:${status?.port ?? "—"}`}
        shared={sharedLan}
        keyCount={keyCount}
        busy={busy}
        onToggleRunning={toggleRunning}
        todayRequests={day?.today ?? null}
        agentsInUse={wired}
        onOpenAgents={goAgents}
        groups={routingGroups.map((group) => ({ slug: group.slug, problem: false }))}
        sources={serving.length}
        models={poolModels}
        middlewares={middlewares}
        onAddGroup={() => { setSelected({ kind: "groups" }); setCreateNonce(Date.now()) }}
        canEscape={canEscape(status)}
        onEscape={escape}
      />
      {inspector()}

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
    </div>
  )
}

export type { GatewayMode }
