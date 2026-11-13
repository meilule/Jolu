import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { api, invoke, isTauri } from "@/lib/api"
import { gatewayApi, type AgentDiscoveryDto, type CustomAgentDto, type CustomAgentInputDto, type GatewayStatusDto, type ProfileDto, type WiringPreviewDto } from "@/lib/gatewayApi"
import { settingsApi, type AgentVersionRow } from "@/lib/settingsApi"
import { skillsApi } from "@/lib/skillsApi"
import { automationApi } from "@/lib/automationApi"
import { AGENT_PLATFORM, WIRABLE_AGENTS, agentRow, customAgentRow, isCustomAgent, readHidden, writeHidden, type AgentRow } from "@/lib/agentPage"
import { emptyTierDraft, tierDraftFrom, tierModelsFrom, type TierDraft } from "@/lib/tierModels"
import type { MainPage } from "@/lib/mainNavigation"
import { useToast } from "@/components/ui/Toast"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { AgentList } from "@/components/agents/AgentList"
import { AgentDetail, type AgentCounts, type AgentLink } from "@/components/agents/AgentDetail"
import { CustomAgentDialog, CustomAgentPanel } from "@/components/agents/CustomAgentPanel"

/**
 * Agent 页（Jolu.pen「统一网关 · R2 重设计」V5-1 已接入工具详情 / V5-2 接入前预览）。
 *
 * 管工具：自动检测本机工具、接入 / 断开、档位、模型菜单、方案、改动与还原。
 * 请求怎么走（路由分组、流水、对外访问）在「模型路由」页。深色稿待定稿后补。
 */

type Applied = { model: string | null; subagent_model: string | null; tiers: import("@/lib/gatewayApi").TierModels | null }
type Confirm = { title: string; description: string; label: string; danger?: boolean; work: () => Promise<boolean | void> }

const EMPTY_COUNTS: AgentCounts = { skills: null, sessions: null, automation: null, routes: null }

export function AgentsView({ onNavigate }: { onNavigate: (page: MainPage) => void }) {
  const toast = useToast()
  const [status, setStatus] = useState<GatewayStatusDto | null>(null)
  const [discovery, setDiscovery] = useState<AgentDiscoveryDto[] | null>(null)
  const [detectedAt, setDetectedAt] = useState<number | null>(null)
  const [versions, setVersions] = useState<AgentVersionRow[]>([])
  const [models, setModels] = useState<string[]>([])
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
  const [counts, setCounts] = useState<AgentCounts>(EMPTY_COUNTS)
  const [applied, setApplied] = useState<Applied | null>(null)
  const [mainModel, setMainModel] = useState("")
  const [subagent, setSubagent] = useState("")
  const [draft, setDraft] = useState<TierDraft>(emptyTierDraft)
  const [confirm, setConfirm] = useState<Confirm | null>(null)
  const [customs, setCustoms] = useState<CustomAgentDto[]>([])
  const [customModel, setCustomModel] = useState("")
  const [adding, setAdding] = useState(false)
  const lock = useRef(false)

  const loadCore = useCallback(async () => {
    if (!isTauri) return
    try {
      void gatewayApi.customAgents().then(setCustoms).catch(() => setCustoms([]))
      const [nextStatus, catalog, groups, nextProfiles, overview, settings] = await Promise.all([
        gatewayApi.status(), gatewayApi.catalog(), gatewayApi.groupList().catch(() => []), gatewayApi.profilesList().catch(() => []),
        gatewayApi.dayOverview().catch(() => null), invoke<{ claude_long_context?: string }>("get_settings").catch(() => null),
      ])
      setStatus(nextStatus)
      const ids = new Set<string>()
      for (const group of groups) ids.add(`group/${group.slug}`)
      for (const model of catalog) if (model.slug) ids.add(`${model.slug}/${model.model}`)
      setModels([...ids])
      setProfiles(nextProfiles)
      setRoutesByAgent(overview?.by_agent ?? null)
      setLongContext(settings?.claude_long_context ?? "")
    } catch (error) {
      toast.danger("读取 Agent 状态失败", String(error))
    }
  }, [toast])

  const detect = useCallback(async () => {
    if (!isTauri) return
    const [found, nextVersions] = await Promise.all([
      gatewayApi.agentDiscovery().catch(() => null),
      settingsApi.agentVersions().catch(() => [] as AgentVersionRow[]),
    ])
    setDiscovery(found)
    setVersions(nextVersions)
    setDetectedAt(Date.now())
  }, [])

  useEffect(() => {
    void loadCore()
    void detect()
    const timer = window.setInterval(() => { if (!document.hidden) void loadCore() }, 15_000)
    return () => window.clearInterval(timer)
  }, [loadCore, detect])

  const rows: AgentRow[] = useMemo(() => {
    // 列表按「后端探测到谁」来建，而不是硬编码白名单：
    // 只遍历 WIRABLE_AGENTS 时，agent_discovery 新增的 8 家永远不显示（界面上只有 5 个）。
    // 顺序：先已接线能力的 5 家（有专门接线），再其余探测到的（只能走通用接入）。
    const seen = new Set<string>()
    const built: AgentRow[] = []
    const push = (agent: string) => {
      if (seen.has(agent)) return
      seen.add(agent)
      built.push(agentRow(
        agent,
        status?.wiring.find((entry) => entry.agent === agent) ?? null,
        discovery?.find((entry) => entry.agent === agent) ?? null,
        agent === "claude-code" ? (applied?.model ?? null) : null,
      ))
    }
    WIRABLE_AGENTS.forEach(push)
    ;(discovery ?? []).forEach((entry) => push(entry.agent))
    return [...built, ...customs.map(customAgentRow)]
  }, [status, discovery, applied, customs])
  const custom = selected && isCustomAgent(selected) ? customs.find((entry) => entry.id === selected) ?? null : null

  // 默认选中第一个已接入的工具，没有就选第一个可接入的
  useEffect(() => {
    if (selected && rows.some((row) => row.agent === selected)) return
    const first = rows.find((row) => row.group === "wired") ?? rows.find((row) => row.group === "available") ?? rows[0]
    if (first) setSelected(first.agent)
  }, [rows, selected])

  const row = rows.find((entry) => entry.agent === selected) ?? null
  const wiredState = row?.group === "wired"

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
    if (!isTauri || !selected || isCustomAgent(selected)) return
    const platform = AGENT_PLATFORM[selected]
    let alive = true
    setCounts({ ...EMPTY_COUNTS, routes: routesByAgent ? routesByAgent[selected] ?? 0 : null })
    void Promise.all([
      skillsApi.overview(platform).then((value) => value.items.filter((item) => item.platform === platform && item.enabled).length).catch(() => null),
      api.sessionsList({ platform, connection: null, project: null, since: null, search: null }).then((value) => value.sessions.length).catch(() => null),
      automationApi.overview().then((value) => value.tasks.filter((task) => task.platform === platform).length).catch(() => null),
    ]).then(([skills, sessions, automation]) => {
      if (alive) setCounts({ skills, sessions, automation, routes: routesByAgent ? routesByAgent[selected] ?? 0 : null })
    })
    return () => { alive = false }
  }, [selected, routesByAgent])

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

  const toggle = (target: AgentRow) => {
    if (isCustomAgent(target.agent)) return target.toggle === "on" ? customUnwire(target.agent) : customWire(target.agent)
    return target.toggle === "on" ? unwire(target.agent) : wire(target.agent)
  }

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

  const link = (target: AgentLink) => onNavigate(target === "routes" ? "gateway" : target)

  const redetect = () => {
    setBusy(true)
    void Promise.all([detect(), loadCore()]).finally(() => setBusy(false))
  }

  if (!isTauri) {
    return <div className="flex h-full items-center justify-center text-meta text-jolu-muted">Agent 页需要在桌面端使用</div>
  }

  return (
    <div className="flex h-full min-h-0 overflow-hidden bg-jolu-bg">
      <AgentList rows={rows} selected={selected} detectedAt={detectedAt} busy={busy} search={search} onSearch={setSearch}
        hidden={hidden} showHidden={showHidden} onShowHidden={setShowHidden} onSelect={setSelected} onToggle={toggle} onRedetect={redetect}
        onAddCustom={() => setAdding(true)} />
      {custom ? (
        <CustomAgentPanel agent={custom} models={models} routing={routing} preview={preview} previewError={previewError} busy={busy}
          model={customModel} onModel={setCustomModel} onWire={() => customWire(custom.id)} onUnwire={() => customUnwire(custom.id)} onRemove={() => customRemove(custom.id)} />
      ) : row ? (
        <AgentDetail
          row={row}
          version={versions.find((entry) => entry.agent === row.agent && entry.found) ?? null}
          routing={routing}
          preview={preview}
          previewError={previewError}
          counts={counts}
          models={models}
          mainModel={mainModel}
          onMainModel={setMainModel}
          subagent={subagent}
          onSubagent={setSubagent}
          draft={draft}
          onDraft={setDraft}
          longContext={longContext}
          onLongContext={saveLongContext}
          profiles={profiles}
          hidden={hidden.includes(row.agent)}
          busy={busy}
          onWire={() => wire(row.agent)}
          onUnwire={() => unwire(row.agent)}
          onApplyTiers={applyTiers}
          onAccept={accept}
          onRewire={rewire}
          onFollowConfig={followConfig}
          onProfileApply={profileApply}
          onProfileSave={profileSave}
          onProfileDelete={profileDelete}
          onToggleHidden={toggleHidden}
          onLink={link}
        />
      ) : (
        <div className="flex flex-1 items-center justify-center text-meta text-jolu-muted">正在检测本机工具…</div>
      )}
      {adding && <CustomAgentDialog busy={busy} onClose={() => setAdding(false)} onSave={customAdd} />}
      {confirm && <ConfirmDialog title={confirm.title} description={confirm.description} confirmLabel={confirm.label} danger={confirm.danger} onClose={() => setConfirm(null)} onConfirm={confirm.work} />}
    </div>
  )
}
