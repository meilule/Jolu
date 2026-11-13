import { useState } from "react"
import {
  Activity, CalendarClock, ChevronDown, ChevronRight, Ellipsis, FilePenLine, FilePlus2, Info, List, MessageSquareText, Play,
  Plug, ShieldCheck, Sparkles, TriangleAlert, Undo2,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { ProfileDto, WiringPreviewDto } from "@/lib/gatewayApi"
import type { AgentVersionRow } from "@/lib/settingsApi"
import { TIER_HINTS, driftText, longContextMode, longContextText, parseTokens, wireExplanation, type AgentRow } from "@/lib/agentPage"
import { EFFORT_LABELS, TIERS, tierEfforts, type AutoChoice, type Tier, type TierDraft } from "@/lib/tierModels"
import { AgentBadge, Switch } from "./AgentList"
import { DiffView } from "./DiffView"

export interface AgentCounts {
  skills: number | null
  sessions: number | null
  automation: number | null
  routes: number | null
}

export type AgentLink = "skills" | "sessions" | "automation" | "routes"

export interface AgentDetailProps {
  row: AgentRow
  version: AgentVersionRow | null
  /** 模型路由是否在运行（中转模式）：没开时接入会先开启它 */
  routing: boolean
  preview: WiringPreviewDto | null
  previewError: string | null
  counts: AgentCounts
  /** 可选模型（`<来源>/<模型>` 与 `group/<分组>`） */
  models: string[]
  mainModel: string
  onMainModel: (model: string) => void
  subagent: string
  onSubagent: (model: string) => void
  draft: TierDraft
  onDraft: (draft: TierDraft) => void
  longContext: string
  onLongContext: (value: string) => void
  profiles: ProfileDto[]
  hidden: boolean
  busy: boolean
  onWire: () => void
  onUnwire: () => void
  onApplyTiers: (draft: TierDraft, mainModel: string, subagent: string) => void
  onAccept: () => void
  onRewire: () => void
  onFollowConfig: () => void
  onProfileApply: (name: string) => void
  onProfileSave: (name: string) => void
  onProfileDelete: (name: string) => void
  onToggleHidden: () => void
  onLink: (link: AgentLink) => void
}

/**
 * Agent 页右侧详情：已接入（画板 V5-1）与接入前预览（画板 V5-2）。
 * 「/model 菜单」的挑选与排序（F3）和「新会话强度」（F4）后端还没有，按钮置灰并标「待确认」。
 */
export function AgentDetail(props: AgentDetailProps) {
  const { row } = props
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-3.5 overflow-y-auto px-7 py-5">
      <Header {...props} />
      {row.group === "wired" ? <WiredBody {...props} /> : row.group === "available" ? <PreviewBody {...props} /> : <BlockedBody row={row} />}
    </div>
  )
}

function Header({ row, version, busy, hidden, onUnwire, onToggleHidden }: AgentDetailProps) {
  const [menu, setMenu] = useState(false)
  const status = row.group === "wired"
    ? [driftText(row.wiring?.drift) ?? "已接入 · 经模型路由调用模型", row.wiring?.codex_mode === "oauth" ? "保留 ChatGPT 登录" : null].filter(Boolean).join(" · ")
    : row.group === "available"
      ? `现在用它自己的配置${row.wiring?.config_path ? ` · 依据：${row.wiring.config_path}` : ""}`
      : row.subtitle
  return (
    <div className="flex items-center gap-3">
      <AgentBadge agent={row.agent} name={row.name} size={40} />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-[17px] font-semibold text-jolu-text">{row.name}</span>
          {version?.version && <span className="text-meta text-jolu-muted">{version.version}</span>}
          {row.group !== "wired" && <span className="rounded-full bg-jolu-track px-2 py-0.5 text-note font-semibold text-jolu-muted">{row.group === "available" ? "未接入" : "不能接入"}</span>}
        </div>
        <span className={cn("truncate text-meta", row.tone === "danger" ? "text-jolu-danger" : "text-jolu-muted")}>{status}</span>
      </div>
      <div className="relative">
        <button type="button" aria-label="更多操作" aria-expanded={menu} onClick={() => setMenu((value) => !value)}
          className="flex size-7 items-center justify-center rounded-md outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <Ellipsis aria-hidden className="size-4 text-jolu-muted" />
        </button>
        {menu && (
          <div role="menu" className="absolute right-0 top-8 z-10 w-36 rounded-lg border border-jolu-border bg-jolu-bg p-1 shadow-lg">
            <button type="button" role="menuitem" onClick={() => { setMenu(false); onToggleHidden() }}
              className="w-full rounded-md px-2.5 py-1.5 text-left text-meta text-jolu-text hover:bg-jolu-side">
              {hidden ? "在列表里显示" : "从列表隐藏"}
            </button>
          </div>
        )}
      </div>
      {/* 画板 V5-2：未接入时头部只有「⋯」，接入按钮在下面的「选起点」里 */}
      {row.toggle === "on" && <Switch on disabled={busy} label={`断开 ${row.name}`} onClick={onUnwire} />}
    </div>
  )
}

/* ─────────────────────────── 已接入（V5-1） ─────────────────────────── */

function WiredBody(props: AgentDetailProps) {
  const { row, counts, preview, models, mainModel, busy, profiles, onLink, onAccept, onRewire, onFollowConfig, onUnwire, onProfileApply, onProfileSave, onProfileDelete } = props
  const claude = row.agent === "claude-code"
  const [open, setOpen] = useState<Record<string, boolean>>({ tiers: true })
  const [noticeShown, setNoticeShown] = useState(true)
  const [naming, setNaming] = useState<string | null>(null)
  const toggle = (key: string) => setOpen((current) => ({ ...current, [key]: !current[key] }))
  const files = preview?.mode === "wired" ? preview.files : []
  const keys = files.reduce((sum, file) => sum + file.keys, 0)
  const drift = row.wiring?.drift
  const current = row.wiring?.current_resolved || row.wiring?.current_model
  const link = (key: AgentLink, icon: React.ReactNode, label: string, value: number | null) => (
    <button key={key} type="button" onClick={() => onLink(key)}
      className="flex h-7 items-center gap-1.5 rounded-full border border-jolu-border px-2.5 text-meta text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
      {icon}
      {label}{value === null ? "" : ` ${value}`}
      <ChevronRight aria-hidden className="size-3 text-jolu-muted" />
    </button>
  )
  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-meta text-jolu-muted">这个工具的</span>
        {link("skills", <Sparkles aria-hidden className="size-3 text-jolu-muted" />, "Skills", counts.skills)}
        {link("sessions", <MessageSquareText aria-hidden className="size-3 text-jolu-muted" />, "会话", counts.sessions)}
        {link("automation", <CalendarClock aria-hidden className="size-3 text-jolu-muted" />, "自动化", counts.automation)}
        {link("routes", <Activity aria-hidden className="size-3 text-jolu-muted" />, "今日路由", counts.routes)}
      </div>

      {drift && drift !== "model_changed" && (
        <Banner tone="danger" text={`${driftText(drift)}：工具的配置已不指向模型路由，或接入没有完成。`}
          actions={[{ label: "重新接入", onClick: onRewire }, { label: "接受现状", onClick: onAccept }]} busy={busy} />
      )}
      {drift === "model_changed" && current && (
        <Banner tone="warn" text={`配置里的主模型已在 ${row.name} 里改成 ${row.wiring?.current_model}${row.wiring?.current_resolved ? `（${row.wiring.current_resolved}）` : ""}`}
          actions={[{ label: "以配置为准", onClick: onFollowConfig }]} busy={busy} />
      )}

      <div className="flex gap-2.5">
        <Summary icon={<Play aria-hidden className="size-3.5 text-jolu-muted" />} title="新会话从" value={claude ? (mainModel || "工具自己上次的选择") : "工具自己上次的选择"} />
        <Summary icon={<List aria-hidden className="size-3.5 text-jolu-muted" />} title="/model 菜单里" value={`${models.length} 个模型`} />
        <Summary icon={<FilePenLine aria-hidden className="size-3.5 text-jolu-muted" />} title={`改了 ${files.length} 个文件`}
          value={files.length ? `${files.map((file) => file.path).join("、")} · ${keys} 键` : preview ? "没有检测到改动" : "读取中…"} />
      </div>

      {noticeShown && row.wiring?.notice && (
        <div className="flex items-center gap-2 px-0.5 text-meta text-[#9A5B00]">
          <Info aria-hidden className="size-[13px] shrink-0" />
          <span className="min-w-0 flex-1">{claude ? <>重开 Claude Code 后用 <code className="rounded-[5px] bg-jolu-track px-1.5 py-px text-note">/model</code> 选模型；{row.wiring.notice}</> : row.wiring.notice}</span>
          <button type="button" onClick={() => setNoticeShown(false)} className="shrink-0 font-semibold outline-none hover:underline">知道了</button>
        </div>
      )}

      <div className="flex flex-col overflow-hidden rounded-xl border border-jolu-border bg-jolu-bg">
        {claude && (
          <Section title="档位" summary="主模型 + 4 档 · 未设置的档跟着主模型" open={!!open.tiers} onToggle={() => toggle("tiers")} first>
            <TierRows {...props} />
          </Section>
        )}
        <Section title="模型菜单" summary={`${models.length} 个可用 · 来自模型路由的模型池与路由分组`} open={!!open.menu} onToggle={() => toggle("menu")} first={!claude}
          action={<button type="button" disabled title="按工具挑选与排序（F3）待确认" className="h-7 rounded-md border border-jolu-border px-2.5 text-meta text-jolu-muted opacity-60">挑选与排序（待确认）</button>}>
          {/* shrink-0：ul 是 flex 容器，列表项默认会被压扁（实测 17 项时每行只剩 7.5px，
              文字叠成一团）。列表项必须锁定自身行高，滚动交给 ul。 */}
          <ul className="flex max-h-48 flex-col gap-1 overflow-y-auto">
            {models.map((model) => <li key={model} className="shrink-0 truncate font-mono text-note text-jolu-text">{model}</li>)}
            {models.length === 0 && <li className="shrink-0 text-meta text-jolu-muted">模型池里还没有可用模型</li>}
          </ul>
        </Section>
        {claude && (
          <Section title="高级" summary={`子代理 · 自动模式 · ${longContextText(props.longContext)} · 新会话强度`} open={!!open.advanced} onToggle={() => toggle("advanced")}>
            <AdvancedRows {...props} />
          </Section>
        )}
        <Section title="写入的改动" summary={files.length ? `${files.map((file) => file.path).join("、")} · ${keys} 个键 · 断开时逐键还原` : "没有检测到 Jolu 的改动"}
          open={!!open.diff} onToggle={() => toggle("diff")}
          action={files.length > 0 && <button type="button" onClick={() => setOpen((current) => ({ ...current, diff: true }))} className="h-7 rounded-md border border-jolu-border px-2.5 text-meta text-jolu-text hover:bg-jolu-side">查看差异</button>}>
          <div className="flex flex-col gap-2.5">
            {props.previewError && <p className="text-meta text-jolu-danger">{props.previewError}</p>}
            {files.map((file) => <DiffView key={file.path} file={file} caption="断开时还原成左侧（-）的样子" />)}
          </div>
        </Section>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <span className="text-meta font-medium text-jolu-muted">方案</span>
        {profiles.map((profile) => (
          <span key={profile.name} className="group flex h-[26px] items-center rounded-full border border-jolu-border text-note font-semibold text-jolu-text hover:border-jolu-accent hover:bg-jolu-selected">
            <button type="button" disabled={busy} onClick={() => onProfileApply(profile.name)} title="应用这套方案"
              className="h-full rounded-full pl-2.5 pr-1.5 outline-none group-hover:text-jolu-on focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
              {profile.name}
            </button>
            <button type="button" disabled={busy} aria-label={`删除方案 ${profile.name}`} onClick={() => onProfileDelete(profile.name)}
              className="mr-1 hidden size-4 items-center justify-center rounded-full text-jolu-muted outline-none hover:text-jolu-danger focus-visible:flex group-hover:flex">×</button>
          </span>
        ))}
        {naming === null ? (
          <button type="button" disabled={busy} onClick={() => setNaming("")} className="text-meta text-jolu-muted outline-none hover:text-jolu-text">＋ 保存当前</button>
        ) : (
          <form className="flex items-center gap-1.5" onSubmit={(event) => { event.preventDefault(); if (naming.trim()) { onProfileSave(naming.trim()); setNaming(null) } }}>
            <input autoFocus value={naming} onChange={(event) => setNaming(event.target.value)} placeholder="方案名称" aria-label="方案名称"
              className="h-[26px] w-28 rounded-md border border-jolu-border bg-jolu-bg px-2 text-meta text-jolu-text outline-none focus:border-jolu-accent" />
            <button type="submit" className="text-meta font-medium text-jolu-on">保存</button>
            <button type="button" onClick={() => setNaming(null)} className="text-meta text-jolu-muted">取消</button>
          </form>
        )}
        <span className="flex-1" />
        <button type="button" disabled={busy} onClick={onUnwire}
          className="flex h-8 items-center gap-1.5 rounded-lg border border-jolu-border px-3 text-meta text-jolu-danger outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
          <Undo2 aria-hidden className="size-3.5" />
          断开并还原
        </button>
      </div>
    </>
  )
}

function Banner({ tone, text, actions, busy }: { tone: "danger" | "warn"; text: string; actions: { label: string; onClick: () => void }[]; busy: boolean }) {
  return (
    <div className={cn("flex items-center gap-2.5 rounded-[10px] px-3.5 py-2.5", tone === "danger" ? "bg-jolu-status-error-bg" : "bg-jolu-gateway-amber-soft")}>
      <TriangleAlert aria-hidden className={cn("size-3.5 shrink-0", tone === "danger" ? "text-jolu-danger" : "text-[#9A5B00]")} />
      <span className="min-w-0 flex-1 text-meta text-jolu-text">{text}</span>
      {actions.map((action) => (
        <button key={action.label} type="button" disabled={busy} onClick={action.onClick}
          className="h-7 shrink-0 rounded-md border border-jolu-border bg-jolu-bg px-2.5 text-meta text-jolu-text outline-none hover:bg-jolu-side disabled:opacity-45">
          {action.label}
        </button>
      ))}
    </div>
  )
}

function Summary({ icon, title, value }: { icon: React.ReactNode; title: string; value: string }) {
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2.5 rounded-[10px] bg-jolu-side px-3 py-2.5">
      {icon}
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="text-note text-jolu-muted">{title}</span>
        <span className="truncate text-meta font-medium text-jolu-text" title={value}>{value}</span>
      </span>
    </div>
  )
}

function Section({ title, summary, open, onToggle, first, action, children }: {
  title: string; summary: string; open: boolean; onToggle: () => void; first?: boolean; action?: React.ReactNode; children: React.ReactNode
}) {
  return (
    <div className={cn(!first && "border-t border-jolu-table-divider")}>
      <div className={cn("flex h-[46px] items-center gap-2.5 px-3.5", open && "bg-jolu-side")}>
        <button type="button" aria-expanded={open} onClick={onToggle} className="flex min-w-0 flex-1 items-center gap-2.5 text-left outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
          {open ? <ChevronDown aria-hidden className="size-3.5 shrink-0 text-jolu-muted" /> : <ChevronRight aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />}
          <span className="shrink-0 text-[13px] font-semibold text-jolu-text">{title}</span>
          <span className="min-w-0 truncate text-meta text-jolu-muted">{summary}</span>
        </button>
        {action}
      </div>
      {open && <div className="px-3.5 pb-3.5 pl-[38px] pt-2.5">{children}</div>}
    </div>
  )
}

const selectClass = "h-8 min-w-0 rounded-lg border border-jolu-border bg-jolu-bg px-2.5 text-meta text-jolu-text outline-none focus:border-jolu-accent disabled:opacity-50"

/** 档位区（画板 V5-1）：主模型 + 四档；改动立即写入 Claude Code。 */
function TierRows({ models, mainModel, subagent, draft, busy, onApplyTiers }: AgentDetailProps) {
  const setTier = (tier: Tier, model: string) => {
    const efforts = tierEfforts(model)
    const effort = draft.efforts[tier]
    onApplyTiers({ ...draft, models: { ...draft.models, [tier]: model }, efforts: { ...draft.efforts, [tier]: effort && efforts.includes(effort) ? effort : null } }, mainModel, subagent)
  }
  const setEffort = (tier: Tier, effort: string) => onApplyTiers({ ...draft, efforts: { ...draft.efforts, [tier]: effort || null } }, mainModel, subagent)
  return (
    <div className="flex flex-col gap-1.5">
      <TierLine label="主模型" hint={TIER_HINTS.main}>
        <select aria-label="主模型" value={mainModel} disabled={busy} onChange={(event) => onApplyTiers(draft, event.target.value, subagent)} className={cn(selectClass, "w-[240px]")}>
          {!mainModel && <option value="">未选（保留工具自己的选择）</option>}
          {models.map((model) => <option key={model} value={model}>{model}</option>)}
        </select>
        <select aria-label="主模型强度" disabled title="新会话强度（F4）待确认" className={cn(selectClass, "w-[110px]")}><option>默认</option></select>
      </TierLine>
      {TIERS.map((tier) => {
        const model = draft.models[tier]
        const efforts = model ? tierEfforts(model) : []
        return (
          <TierLine key={tier} label={tier[0].toUpperCase() + tier.slice(1)} hint={TIER_HINTS[tier]}>
            <select aria-label={`${tier} 档模型`} value={model} disabled={busy} onChange={(event) => setTier(tier, event.target.value)} className={cn(selectClass, "w-[240px]")}>
              <option value="">同主模型</option>
              {models.map((entry) => <option key={entry} value={entry}>{entry}</option>)}
            </select>
            <select aria-label={`${tier} 档强度`} value={draft.efforts[tier] ?? ""} disabled={busy || !model || efforts.length === 0}
              title={!model ? "跟随主模型时强度由 Claude Code 决定" : efforts.length === 0 ? "这个模型不收 effort" : undefined}
              onChange={(event) => setEffort(tier, event.target.value)} className={cn(selectClass, "w-[110px]")}>
              <option value="">默认</option>
              {efforts.map((effort) => <option key={effort} value={effort}>{EFFORT_LABELS[effort] ?? effort}</option>)}
            </select>
          </TierLine>
        )
      })}
    </div>
  )
}

function TierLine({ label, hint, children }: { label: string; hint?: { text: string; warn?: boolean }; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2">
      <span className="w-14 shrink-0 text-meta text-jolu-text">{label}</span>
      {children}
      {hint && (
        <span className={cn("flex min-w-0 items-center gap-1 truncate text-note", hint.warn ? "text-[#9A5B00]" : "text-jolu-muted")}>
          {hint.warn && <TriangleAlert aria-hidden className="size-3 shrink-0" />}
          {hint.warn ? hint.text : <code className="font-mono">{hint.text}</code>}
        </span>
      )}
    </div>
  )
}

/** 高级（画板 V5-1）：子代理、自动模式、长对话；新会话强度待确认。 */
function AdvancedRows({ models, mainModel, subagent, draft, busy, longContext, onApplyTiers, onLongContext }: AgentDetailProps) {
  const mode = longContextMode(longContext)
  const [custom, setCustom] = useState(mode === "custom" ? longContext : "")
  const autoValue = draft.auto.kind === "model" ? `model:${draft.auto.model}` : draft.auto.kind
  const setAuto = (value: string) => {
    const auto: AutoChoice = value === "default" ? { kind: "default" } : value === "subagent" ? { kind: "subagent" } : { kind: "model", model: value.slice("model:".length) }
    onApplyTiers({ ...draft, auto }, mainModel, subagent)
  }
  const segment = (value: "default" | "full", label: string) => (
    <button type="button" aria-pressed={mode === value} disabled={busy} onClick={() => onLongContext(value === "default" ? "" : "full")}
      className={cn("h-7 rounded-md px-2.5 text-meta outline-none", mode === value ? "bg-jolu-bg text-jolu-text shadow-sm" : "text-jolu-muted hover:text-jolu-text")}>
      {label}
    </button>
  )
  const customValid = parseTokens(custom) !== null
  return (
    <div className="flex flex-col gap-2">
      <TierLine label="子代理">
        <select aria-label="子代理模型" value={subagent} disabled={busy} onChange={(event) => onApplyTiers(draft, mainModel, event.target.value)} className={cn(selectClass, "w-[240px]")}>
          <option value="">由 Claude Code 自己挑选</option>
          {models.map((model) => <option key={model} value={model}>{model}</option>)}
        </select>
      </TierLine>
      <TierLine label="自动模式" hint={{ text: "自动模式下检查每个操作的模型" }}>
        <select aria-label="自动模式模型" value={autoValue} disabled={busy} onChange={(event) => setAuto(event.target.value)} className={cn(selectClass, "w-[240px]")}>
          <option value="default">默认（Claude Code 自己挑）</option>
          <option value="subagent">跟随子代理</option>
          {models.map((model) => <option key={model} value={`model:${model}`}>{model}</option>)}
        </select>
      </TierLine>
      <div className="flex items-start gap-2">
        <span className="w-14 shrink-0 pt-1.5 text-meta text-jolu-text">长对话</span>
        <div className="flex min-w-0 flex-col gap-1.5">
          <div className="flex flex-wrap items-center gap-2">
            <div className="flex items-center gap-0.5 rounded-lg bg-jolu-track p-0.5" role="group" aria-label="长对话压缩">
              {segment("default", "272K 时压缩")}
              {segment("full", "完整窗口")}
            </div>
            <form className="flex items-center gap-1.5" onSubmit={(event) => { event.preventDefault(); if (customValid) onLongContext(custom.trim()) }}>
              <input value={custom} onChange={(event) => setCustom(event.target.value)} placeholder="自定义，如 400k" aria-label="自定义压缩大小"
                className={cn("h-7 w-28 rounded-md border bg-jolu-bg px-2 text-meta text-jolu-text outline-none", mode === "custom" ? "border-jolu-accent" : "border-jolu-border", custom && !customValid && "border-jolu-danger")} />
              <button type="submit" disabled={busy || !customValid} className="text-meta font-medium text-jolu-on disabled:opacity-40">应用</button>
            </form>
          </div>
          <span className="text-note text-jolu-muted">到这个大小时压缩；完整窗口每轮更慢、更贵。只对窗口大于 200K 的模型生效，窗口未知的模型不写。</span>
        </div>
      </div>
      <TierLine label="新会话强度">
        <select aria-label="新会话强度" disabled title="新会话默认强度（F4）待确认" className={cn(selectClass, "w-[240px]")}><option>默认（待确认）</option></select>
      </TierLine>
    </div>
  )
}

/* ─────────────────────────── 接入前预览（V5-2） ─────────────────────────── */

function PreviewBody({ row, models, mainModel, onMainModel, routing, preview, previewError, busy, onWire }: AgentDetailProps) {
  const claude = row.agent === "claude-code"
  const files = preview?.mode === "wire" ? preview.files : []
  const icons = { file: FilePlus2, shield: ShieldCheck, undo: Undo2 }
  return (
    <>
      <div className="flex flex-col gap-2.5 rounded-xl border border-jolu-border p-4">
        <span className="text-[13px] font-semibold text-jolu-text">{claude ? "接入后，新会话从哪个模型开始" : `接入 ${row.name}`}</span>
        <div className="flex flex-wrap items-center gap-2.5">
          {claude && (
            <>
              <select aria-label="起点模型" value={mainModel} disabled={busy} onChange={(event) => onMainModel(event.target.value)} className={cn(selectClass, "w-[260px]")}>
                <option value="">不选（保留工具自己的选择）</option>
                {models.map((model) => <option key={model} value={model}>{model}</option>)}
              </select>
              <select aria-label="起点强度" disabled title="新会话强度（F4）待确认" className={cn(selectClass, "w-[120px]")}><option>默认强度</option></select>
            </>
          )}
          <button type="button" disabled={busy} onClick={onWire}
            className="flex h-8 items-center gap-1.5 rounded-lg bg-jolu-accent px-3.5 text-meta font-semibold text-[#071B10] outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:opacity-45">
            <Plug aria-hidden className="size-3.5" />
            接入 {row.name}
          </button>
        </div>
        <span className="text-meta text-jolu-muted">
          {claude
            ? `不选也能接入：${row.name} 会保持它自己上次的选择，模型路由里的 ${models.length} 个模型出现在它的模型列表里。`
            : `接入后 ${row.name} 的请求经模型路由转发，可以用模型路由里的 ${models.length} 个模型。`}
        </span>
      </div>

      <div className="flex flex-col gap-2.5">
        <span className="text-[13px] font-semibold text-jolu-text">接入会做什么</span>
        {wireExplanation(row.agent).map((item) => {
          const Icon = icons[item.icon]
          return (
            <div key={item.text} className="flex items-center gap-2.5">
              <span className="flex size-[26px] shrink-0 items-center justify-center rounded-[7px] bg-jolu-selected"><Icon aria-hidden className="size-3.5 text-jolu-on" /></span>
              <span className="text-meta text-jolu-text">{item.text}</span>
            </div>
          )
        })}
      </div>

      {previewError && <p className="text-meta text-jolu-danger">写入预览读取失败：{previewError}</p>}
      {!preview && !previewError && <p className="text-meta text-jolu-muted">正在生成写入预览…</p>}
      {files.map((file) => <DiffView key={file.path} file={file} caption={file.existed ? "写入预览 · 还没有改动" : "写入预览 · 会新建这个文件"} />)}

      {!routing && (
        <div className="flex items-center gap-2.5 rounded-[10px] bg-jolu-side px-3.5 py-2.5">
          <Info aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />
          <span className="text-meta text-jolu-muted">模型路由现在没开，点「接入」会先开启模型路由，再写入配置。</span>
        </div>
      )}
    </>
  )
}

function BlockedBody({ row }: { row: AgentRow }) {
  const cliMissing = row.discovery?.state === "cli_missing"
  return (
    <div className="flex flex-col gap-2 rounded-xl border border-jolu-border p-4">
      <span className="text-[13px] font-semibold text-jolu-text">暂时不能接入</span>
      <span className="text-meta text-jolu-muted">
        {cliMissing ? "检测到它的配置，但找不到命令行程序。装回 CLI 后点左上角「重新检测」。" : `${row.subtitle}。装好并运行过一次后，点左上角「重新检测」。`}
      </span>
      {row.wiring?.config_path && <span className="font-mono text-note text-jolu-muted">配置位置：{row.wiring.config_path}</span>}
    </div>
  )
}
