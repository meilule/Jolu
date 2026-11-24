import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { invoke, isTauri, listenEvent } from "@/lib/api"
import { EVENTS } from "@/lib/events"
import { useLatestGate } from "@/hooks/useLatestGate"
import { gatewayApi, type AgentDiscoveryDto, type CatalogModelDto, type CustomAgentDto, type CustomAgentInputDto, type GatewayStatusDto, type ProfileDto, type WiringPreviewDto } from "@/lib/gatewayApi"
import type { RoutingGroupDto } from "@/components/gateway/routingGroups"
import { settingsApi, type AgentVersionRow } from "@/lib/settingsApi"
import { buildAgentRows, customAgentRow, isCustomAgent, readHidden, writeHidden, type AgentRow } from "@/lib/agentPage"
import { emptyTierDraft, tierDraftFrom, tierModelsFrom, type TierDraft } from "@/lib/tierModels"
import type { MainPage } from "@/lib/mainNavigation"
import { useToast } from "@/components/ui/Toast"
import { useContentMotion } from "@/lib/motion"
import { Button, ConfirmDialog, PageActions } from "@/ui"
import { AgentDirectory } from "@/components/agents/AgentList"
import { AgentDetail } from "@/components/agents/AgentDetail"
import { CustomAgentDialog, CustomAgentPanel } from "@/components/agents/CustomAgentPanel"

/**
 * Agent 页（设计稿 jolu-new.pen R1-08 列表 / R1-09 详情 / R1-32 高级 / R1-33 写入改动）。
 *
 * 管工具：自动检测本机工具、接入 / 断开、档位、模型菜单、方案、改动与还原。
 * 列表与详情是**整页切换**（R1-09 带「← 全部工具」）；请求怎么走（路由分组、
 * 流水、对外访问）在「模型路由」页。
 *
 * 页头的「重新检测 / 添加工具」通过 `PageActions` 送进页头——这是 R1-08 的
 * 「页面操作」槽，不在页面内容里多占一行。
 */

type Applied = { model: string | null; subagent_model: string | null; tiers: import("@/lib/gatewayApi").TierModels | null }
type Confirm = { title: string; description: string; label: string; danger?: boolean; work: () => Promise<boolean | void> }

/**
 * 跨挂载记住上次的探测与版本结果（T7）：切走页面会卸载组件，切回来先显示上次的结果，
 * 再在后台刷新一次（后端命令查找按目录戳缓存，单次约几毫秒），不节流——别处刚装好的工具
 * 切回来就能看到。查询失败时清掉，不拿过期结果冒充当前状态。
 */
let lastStatus: GatewayStatusDto | null = null
let lastDiscovery: AgentDiscoveryDto[] | null = null
let lastVersions: AgentVersionRow[] = []

export function AgentsView({ onNavigate }: { onNavigate: (page: MainPage) => void }) {
  const toast = useToast()
  const [status, setStatus] = useState<GatewayStatusDto | null>(() => lastStatus)
  const [discovery, setDiscovery] = useState<AgentDiscoveryDto[] | null>(() => lastDiscovery)
  const [versions, setVersions] = useState<AgentVersionRow[]>(() => lastVersions)
  const [catalog, setCatalog] = useState<CatalogModelDto[]>([])
  const [groups, setGroups] = useState<RoutingGroupDto[]>([])
  const [profiles, setProfiles] = useState<ProfileDto[]>([])
  const [routesByAgent, setRoutesByAgent] = useState<Record<string, number> | null>(null)
  const [longContext, setLongContext] = useState("")
  const [selected, setSelected] = useState<string | null>(null)
  const [search, setSearch] = useState("")
  const [hidden, setHidden] = useState<string[]>(() => readHidden())
  const [showHidden, setShowHidden] = useState(false)
  const [busy, setBusy] = useState(false)
  const [preview, setPreview] = useState<WiringPreviewDto | null>(null)
  const [previewError, setPreviewError] = useState<string | null>(null)
  const [applied, setApplied] = useState<Applied | null>(null)
  const [mainModel, setMainModel] = useState("")
  const [subagent, setSubagent] = useState("")
  const [draft, setDraft] = useState<TierDraft>(emptyTierDraft)
  const [confirm, setConfirm] = useState<Confirm | null>(null)
  const [customs, setCustoms] = useState<CustomAgentDto[]>([])
  const [customModel, setCustomModel] = useState("")
  const [adding, setAdding] = useState(false)
  const lock = useRef(false)

  // loadCore 由挂载、15 秒轮询、目录 / 连接事件、重新检测共同触发；detect 由挂载与重新检测触发。
  // 前一次没回来又发了一次时，只认最新一次的结果
  const coreGate = useLatestGate()
  const detectGate = useLatestGate()
  const loadCore = useCallback(async () => {
    if (!isTauri) return
    const isLatest = coreGate.begin()
    try {
      void gatewayApi.customAgents().then((next) => { if (isLatest()) setCustoms(next) }).catch(() => { if (isLatest()) setCustoms([]) })
      const [, nextCatalog, nextGroups, nextProfiles, overview, settings] = await Promise.all([
        gatewayApi.status().then((next) => {
          if (isLatest()) {
            lastStatus = next
            setStatus(next)
          }
          return next
        }),
        gatewayApi.catalog(), gatewayApi.groupList().catch(() => [] as RoutingGroupDto[]), gatewayApi.profilesList().catch(() => []),
        gatewayApi.dayOverview().catch(() => null), invoke<{ claude_long_context?: string }>("get_settings").catch(() => null),
      ])
      if (!isLatest()) return
      setCatalog(nextCatalog)
      setGroups(nextGroups)
      setProfiles(nextProfiles)
      setRoutesByAgent(overview?.by_agent ?? null)
      setLongContext(settings?.claude_long_context ?? "")
    } catch (error) {
      if (isLatest()) toast.danger("读取 Agent 状态失败", String(error))
    }
  }, [toast, coreGate])

  // 探测与版本查询分开落地：探测几毫秒就回来，不再等 `--version`（启动后第一次最多 3 秒）
  const detect = useCallback(async (force = false) => {
    if (!isTauri) return
    const isLatest = detectGate.begin()
    // 过期的结果既不显示，也不写进跨挂载的缓存
    const found = gatewayApi.agentDiscovery(force).catch(() => null).then((next) => {
      if (!isLatest()) return
      lastDiscovery = next
      setDiscovery(next)
    })
    const versioned = settingsApi.agentVersions().catch(() => [] as AgentVersionRow[]).then((next) => {
      if (!isLatest()) return
      lastVersions = next
      setVersions(next)
    })
    await Promise.all([found, versioned])
  }, [detectGate])

  useEffect(() => {
    void loadCore()
    void detect()
    const timer = window.setInterval(() => { if (!document.hidden) void loadCore() }, 15_000)
    // 模型目录或连接变了立即重读（模型菜单不用等下一次 15 秒轮询）；轮询保留作兜底
    let disposed = false
    const offs: Array<() => void> = []
    const keep = (off: () => void) => { if (disposed) off(); else offs.push(off) }
    const reload = () => { void loadCore() }
    void listenEvent(EVENTS.GATEWAY_CATALOG_CHANGED, reload).then(keep)
    void listenEvent(EVENTS.CONNECTIONS_CHANGED, reload).then(keep)
    return () => { window.clearInterval(timer); disposed = true; offs.forEach((off) => off()) }
  }, [loadCore, detect])

  const rows: AgentRow[] = useMemo(() => {
    if (!status) return [...customs.map(customAgentRow)]
    return [...buildAgentRows(status.wiring, discovery ?? [], applied?.model ?? null), ...customs.map(customAgentRow)]
  }, [status, discovery, applied, customs])
  const row = rows.find((entry) => entry.agent === selected) ?? null
  const wiredState = row?.group === "wired"
  /** 详情是整页切换：选中自定义工具走通用接入面板，其余走内置工具详情 */
  const detailCustom = selected && isCustomAgent(selected) ? customs.find((entry) => entry.id === selected) ?? null : null
  const detailRow = selected && !detailCustom ? row : null
  const detailReady = Boolean(detailCustom || detailRow)
  /** 列表 ↔ 详情的整页切换淡入（不位移，避免固定弹窗定位参照变化） */
  const contentMotion = useContentMotion(selected ?? "list", 220)

  /** 模型 id → 来源说明（画板 iTvRW「来源：工作账号 · 订阅连接」） */
  const sourceNotes = useMemo(() => {
    const notes: Record<string, string> = {}
    for (const model of catalog) {
      if (!model.slug) continue
      notes[`${model.slug}/${model.model}`] = `${model.source} · ${model.kind === "auth" ? "订阅连接" : "API Key"}`
    }
    for (const group of groups) notes[`group/${group.slug}`] = `路由分组 · ${group.name}`
    return notes
  }, [catalog, groups])

  // Claude Code 的接线回填：主模型、子代理、档位
  useEffect(() => {
    if (!isTauri) return
    let alive = true
    gatewayApi.wiringApplied("claude-code").then((value) => {
      if (!alive) return
      setApplied(value)
      if (value) {
        setMainModel((value.model ?? "").replace(/\[1m\]$/i, ""))
        setSubagent(value.subagent_model ?? "")
        setDraft(tierDraftFrom(value.tiers, value.subagent_model))
      }
    }).catch(() => { /* 回填失败不影响接入 */ })
    return () => { alive = false }
  }, [status])

  // 选中工具变化：读改动预览与计数
  const previewModel = selected === "claude-code" && !wiredState ? mainModel : ""
  useEffect(() => {
    if (!isTauri || !selected || !row || row.group === "blocked") {
      setPreview(null)
      return
    }
    let alive = true
    setPreviewError(null)
    const request = isCustomAgent(selected)
      ? gatewayApi.customAgentPreview(selected, row.group === "wired" ? null : customModel || null)
      : gatewayApi.wiringPreview(selected, previewModel || null)
    request.then((value) => { if (alive) setPreview(value) })
      .catch((error) => { if (alive) { setPreview(null); setPreviewError(String(error)) } })
    return () => { alive = false }
  }, [selected, row?.group, previewModel, customModel, status, customs])

  useEffect(() => {
    if (!selected) return
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setSelected(null)
    }
    window.addEventListener("keydown", closeOnEscape)
    return () => window.removeEventListener("keydown", closeOnEscape)
  }, [selected])

  const run = async (work: () => Promise<unknown>, ok?: string, rethrow = false) => {
    if (lock.current) return false
    lock.current = true
    setBusy(true)
    try {
      const message = await work()
      if (ok) toast.success(typeof message === "string" && message ? message : ok)
      await loadCore()
      return true
    } catch (error) {
      toast.danger("操作失败", String(error))
      if (rethrow) throw error
      return false
    } finally {
      lock.current = false
      setBusy(false)
    }
  }

  const routing = Boolean(status?.enabled && status.mode === "relay")
  /** 模型路由没开时先开启（V5-2「点接入会先开启模型路由」） */
  const ensureRouting = async () => {
    if (!routing) await gatewayApi.setMode("relay", status?.default_connection ?? null)
  }

  const tiersFor = (nextDraft: TierDraft, main: string, sub: string) => tierModelsFrom(nextDraft, main, sub) ?? undefined

  const wire = (agent: string) => {
    const name = rows.find((entry) => entry.agent === agent)?.name ?? agent
    const claude = agent === "claude-code"
    void run(async () => {
      await ensureRouting()
      return gatewayApi.setWiring(agent, true, claude ? mainModel || null : null, claude ? subagent || null : null, claude ? tiersFor(draft, mainModel, subagent) : undefined)
    }, `已接入 ${name}`)
  }

  const unwire = (agent: string) => {
    const name = rows.find((entry) => entry.agent === agent)?.name ?? agent
    setConfirm({
      title: `断开 ${name} 并还原？`,
      description: "只删除 Jolu 写入的键，把你原来的值放回去；接入后你自己改过的值不会被还原。之后它的请求不再经过模型路由。",
      label: "断开并还原",
      danger: true,
      work: () => run(() => gatewayApi.setWiring(agent, false), `已断开 ${name}`, true),
    })
  }

  const customName = (id: string) => customs.find((entry) => entry.id === id)?.name ?? id
  const reloadCustoms = async () => setCustoms(await gatewayApi.customAgents())
  const customWire = (id: string) => void run(async () => {
    await ensureRouting()
    await gatewayApi.customAgentWire(id, customModel || null)
    await reloadCustoms()
  }, `已接入 ${customName(id)}`)
  const customUnwire = (id: string) => setConfirm({
    title: `断开 ${customName(id)} 并还原？`,
    description: "只还原 Jolu 写下的地址、令牌与模型键；接入后你自己改过的值不会被还原。",
    label: "断开并还原",
    danger: true,
    work: () => run(async () => { await gatewayApi.customAgentUnwire(id); await reloadCustoms() }, `已断开 ${customName(id)}`, true),
  })
  const customRemove = (id: string) => setConfirm({
    title: `删除自定义工具「${customName(id)}」？`,
    description: "只删除 Jolu 里的这条接入说明，不改动工具的配置文件。",
    label: "删除",
    danger: true,
    work: () => run(async () => { await gatewayApi.customAgentRemove(id); await reloadCustoms(); setSelected(null) }, "已删除", true),
  })
  const customAdd = (input: CustomAgentInputDto) => run(async () => {
    const added = await gatewayApi.customAgentAdd(input)
    await reloadCustoms()
    setSelected(added.id)
  }, `已添加 ${input.name}`)

  /** 档位 / 主模型 / 子代理 / 自动模式改动立即写入（已接入时）；未接入时只更新草稿，接入时一并写入 */
  const applyTiers = (nextDraft: TierDraft, main: string, sub: string) => {
    setDraft(nextDraft)
    setMainModel(main)
    setSubagent(sub)
    if (!wiredState || selected !== "claude-code") return
    void run(() => gatewayApi.setWiring("claude-code", true, main || null, sub || null, tiersFor(nextDraft, main, sub)), "已写入 Claude Code")
  }

  const followConfig = () => {
    const current = row?.wiring?.current_resolved || row?.wiring?.current_model
    if (!current) return
    const model = current.replace(/\[1m\]$/i, "")
    setMainModel(model)
    void run(() => gatewayApi.setWiring("claude-code", true, model, subagent || null, tiersFor(draft, model, subagent)), "已按配置里的主模型更新跟随档位")
  }

  const rewire = () => {
    if (!row) return
    const claude = row.agent === "claude-code"
    void run(async () => {
      await ensureRouting()
      return gatewayApi.setWiring(row.agent, true, claude ? mainModel || null : null, claude ? subagent || null : null, claude ? tiersFor(draft, mainModel, subagent) : undefined)
    }, `已重新接入 ${row.name}`)
  }

  const accept = () => {
    if (!row) return
    setConfirm({
      title: `接受 ${row.name} 的现有配置？`,
      description: "以工具当前的配置为准，结束 Jolu 对这份接线记录的管理；不会改动配置文件。",
      label: "接受现状",
      work: () => run(() => gatewayApi.acceptWiring(row.agent), "已接受现状", true),
    })
  }

  const profileApply = (name: string) => setConfirm({
    title: `应用方案「${name}」？`,
    description: "把这套方案里各工具的主模型、档位与子代理重新写入已接入的工具；保存了默认来源的会一并切换。",
    label: "应用",
    work: () => run(async () => (await gatewayApi.profilesApply(name)).message, name, true),
  })
  const profileDelete = (name: string) => setConfirm({
    title: `删除方案「${name}」？`,
    description: "只删除这份方案，不影响工具当前的配置。",
    label: "删除",
    danger: true,
    work: () => run(() => gatewayApi.profilesDelete(name), "已删除", true),
  })
  const profileSave = (name: string) => void run(async () => { await gatewayApi.profilesSave(name); return `已保存方案「${name}」` }, "已保存")

  const saveLongContext = (value: string) => void run(() => gatewayApi.longContextSet(value), "已保存")

  const toggleHidden = () => {
    if (!selected) return
    const next = hidden.includes(selected) ? hidden.filter((agent) => agent !== selected) : [...hidden, selected]
    setHidden(next)
    writeHidden(next)
  }

  const redetect = () => {
    setBusy(true)
    void Promise.all([detect(true), loadCore()]).finally(() => setBusy(false))
  }

  if (!isTauri) {
    return <div className="flex h-full items-center justify-center text-[12px] text-jn-text-3">Agent 页需要在桌面端使用</div>
  }

  const modelIds = modelsOf(catalog, groups)
  /** 选中的工具在刷新后可能已经消失（自定义工具被删）：此时退回列表，不留空白页 */
  const showList = !selected || !detailReady
  return (
    <>
      {/* 只在列表态显示页头操作：详情里页面操作换成了工具头（R1-09） */}
      {showList && (
        <PageActions>
          <Button kind="ghost" size="sm" icon="refresh" disabled={busy} onClick={redetect}>重新检测</Button>
          <Button kind="primary" size="sm" icon="add" disabled={busy} onClick={() => setAdding(true)}>添加工具</Button>
        </PageActions>
      )}
      <div ref={contentMotion} className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        {showList ? (
          <AgentDirectory rows={rows} selected={selected} modelsCount={modelIds.length} tierCount={6} busy={busy} search={search} onSearch={setSearch}
            hidden={hidden} showHidden={showHidden} onShowHidden={setShowHidden} onSelect={setSelected}
            onToggle={(target) => target.toggle === "on" ? unwire(target.agent) : wire(target.agent)} />
        ) : detailCustom ? (
          <CustomAgentPanel agent={detailCustom} models={modelIds} routing={routing} preview={preview} previewError={previewError} busy={busy}
            model={customModel} onModel={setCustomModel} onBack={() => setSelected(null)}
            onWire={() => customWire(detailCustom.id)} onUnwire={() => customUnwire(detailCustom.id)} onRemove={() => customRemove(detailCustom.id)} />
        ) : detailReady && detailRow ? (
          <AgentDetail
            row={detailRow}
            version={versions.find((entry) => entry.agent === detailRow.agent && entry.found) ?? null}
            routing={routing}
            preview={preview}
            previewError={previewError}
            todayRoutes={routesByAgent ? (routesByAgent[detailRow.agent] ?? 0) : null}
            models={modelIds}
            sourceNotes={sourceNotes}
            mainModel={mainModel}
            onMainModel={setMainModel}
            subagent={subagent}
            onSubagent={setSubagent}
            draft={draft}
            onDraft={setDraft}
            longContext={longContext}
            onLongContext={saveLongContext}
            profiles={profiles}
            hidden={hidden.includes(detailRow.agent)}
            busy={busy}
            onBack={() => setSelected(null)}
            onWire={() => wire(detailRow.agent)}
            onUnwire={() => unwire(detailRow.agent)}
            onApplyTiers={applyTiers}
            onAccept={accept}
            onRewire={rewire}
            onFollowConfig={followConfig}
            onProfileApply={profileApply}
            onProfileSave={profileSave}
            onProfileDelete={profileDelete}
            onToggleHidden={toggleHidden}
            onOpenModels={() => onNavigate("gateway")}
          />
        ) : null}
      </div>
      {adding && <CustomAgentDialog busy={busy} onClose={() => setAdding(false)} onSave={customAdd} />}
      {confirm && (
        <ConfirmDialog
          open
          onOpenChange={(open) => { if (!open) setConfirm(null) }}
          title={confirm.title}
          description={confirm.description}
          confirmText={confirm.label}
          kind={confirm.danger ? "danger" : "primary"}
          loading={busy}
          onConfirm={() => { const current = confirm; setConfirm(null); void current.work() }}
        />
      )}
    </>
  )
}

/** 可选模型 id：`<来源>/<模型>` 与 `group/<分组>`（去重、保序） */
function modelsOf(catalog: CatalogModelDto[], groups: RoutingGroupDto[]): string[] {
  const ids: string[] = []
  for (const group of groups) ids.push(`group/${group.slug}`)
  for (const model of catalog) if (model.slug) ids.push(`${model.slug}/${model.model}`)
  return [...new Set(ids)]
}
