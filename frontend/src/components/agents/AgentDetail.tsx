import { useState } from "react"
import {
  Activity, CalendarClock, ChevronDown, Ellipsis, FilePlus2, Info, ListFilter,
  MessageSquareText, Plug, ShieldCheck, Sparkles, TriangleAlert, Undo2,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { ProfileDto, WiringPreviewDto } from "@/lib/gatewayApi"
import type { AgentVersionRow } from "@/lib/settingsApi"
import { driftText, longContextMode, parseTokens, wireExplanation, type AgentRow } from "@/lib/agentPage"
import { EFFORT_LABELS, TIERS, tierEfforts, type AutoChoice, type Tier, type TierDraft } from "@/lib/tierModels"
import { SelectField, type SelectOption } from "@/components/ui/SelectField"
import { JoluButton } from "@/components/ui/JoluButton"
import { AgentBadge } from "./AgentList"
import { DiffView } from "./DiffView"

export interface AgentCounts {
  skills: number | null
  sessions: number | null
  automation: number | null
  routes: number | null
}

export type AgentLink = "skills" | "sessions" | "automation" | "routes"

export interface AgentDetailProps {
  /** V8 列表上的右侧配置抽屉；保留详情内容与列表同时可见。 */
  drawer?: boolean
  row: AgentRow
  version: AgentVersionRow | null
  /** 模型路由是否在运行（中转模式）：没开时接入会先开启它 */
  routing: boolean
  preview: WiringPreviewDto | null
  previewError: string | null
  counts: AgentCounts
  /** 可选模型（`<来源>/<模型>` 与 `group/<分组>`） */
  models: string[]
  /** 模型 id → 来源说明（「工作账号 · 订阅连接」）；缺 = 未知来源 */
  sourceNotes: Record<string, string>
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
  onBack: () => void
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

type DetailTab = "model" | "advanced" | "diff"

/**
 * Agent 页 · 详情态（Jolu.pen「统一网关 · R2 重设计」V6 画板 iTvRW）：
 * 「← 全部工具」返回 + 工具头（配置一致 badge、更多菜单、配置方案下拉）+
 * 「模型设置 / 高级 / 写入改动」三个页签。高级与写入改动没有 V6 画板，沿用 V5 内容重排，标「待确认」。
 * 「/model 菜单」的挑选与排序（F3）和「新会话强度」（F4）后端还没有，标「待确认」。
 */
export function AgentDetail(props: AgentDetailProps) {
  const { row } = props
  const [tab, setTab] = useState<DetailTab>("model")
  const wired = row.group === "wired"
  return (
    <div className={cn("flex min-w-0 flex-1 flex-col gap-5 overflow-y-auto px-7 pb-8 pt-6", props.drawer ? "agent-detail-drawer-content" : "px-8")}>
      <div className="flex items-center gap-2">
        <button type="button" onClick={props.onBack}
          className="flex h-8 items-center gap-1.5 rounded-lg px-2 text-meta text-jolu-muted outline-none hover:bg-jolu-side hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <svg aria-hidden viewBox="0 0 14 14" className="size-3.5" fill="none" stroke="currentColor" strokeWidth="1.5">
            <path d="M9 3L5 7L9 11" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          全部工具
        </button>
        <span className="flex-1" />
        {wired && <ProfileMenu {...props} />}
      </div>

      <Header {...props} />

      {wired ? (
        <>
          <TabBar tab={tab} claude={row.agent === "claude-code"} onTab={setTab} />
          {tab === "model" && (row.agent === "claude-code" ? <ClaudeModelSettings {...props} /> : <GenericModelSettings {...props} />)}
          {tab === "advanced" && row.agent === "claude-code" && <AdvancedBody {...props} />}
          {tab === "diff" && <DiffBody {...props} />}
        </>
      ) : row.group === "available" ? (
        row.supported ? <PreviewBody {...props} /> : <UnsupportedBody row={row} />
      ) : (
        <BlockedBody row={row} />
      )}
    </div>
  )
}

function Header({ row, version, busy, hidden, onUnwire, onToggleHidden }: AgentDetailProps) {
  const [menu, setMenu] = useState(false)
  const drift = row.wiring?.drift ?? null
  const wired = row.group === "wired"
  const status = wired
    ? [driftText(drift) ?? "已接入 · 经模型路由调用模型", row.wiring?.codex_mode === "oauth" ? "保留 ChatGPT 登录" : null].filter(Boolean).join(" · ")
    : row.group === "available"
      ? `现在用它自己的配置${row.wiring?.config_path ? ` · 依据：${row.wiring.config_path}` : ""}`
      : row.subtitle
  return (
    <div className="flex items-center gap-4">
      <AgentBadge agent={row.agent} name={row.name} size={40} />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center gap-2.5">
          <span className="truncate text-[22px] font-semibold leading-7 text-jolu-text">{row.name}</span>
          {version?.version && <span className="text-meta text-jolu-muted">{version.version}</span>}
        </div>
        <span className={cn("truncate text-meta", row.tone === "danger" ? "text-jolu-danger" : "text-jolu-muted")}>{status}</span>
      </div>
      {wired && (
        <span className={cn(
          "rounded-md px-2 py-1 text-note font-semibold",
          drift ? (row.tone === "danger" ? "bg-jolu-status-error-bg text-jolu-danger" : "bg-jolu-gateway-amber-soft text-[#9A5B00]") : "bg-jolu-selected text-jolu-on",
        )}>{drift ? driftText(drift) : "配置一致"}</span>
      )}
      {!wired && <span className="rounded-md bg-jolu-track px-2 py-1 text-note font-semibold text-jolu-muted">{row.group === "available" ? "未接入" : "不能接入"}</span>}
      <div className="relative">
        <button type="button" aria-label="更多操作" aria-expanded={menu} onClick={() => setMenu((value) => !value)}
          className="flex h-9 w-10 items-center justify-center rounded-lg border border-jolu-border outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <Ellipsis aria-hidden className="size-4 text-jolu-muted" />
        </button>
        {menu && (
          <div role="menu" className="absolute right-0 top-10 z-10 w-44 rounded-lg border border-jolu-border bg-jolu-bg p-1 shadow-lg">
            <button type="button" role="menuitem" onClick={() => { setMenu(false); onToggleHidden() }}
              className="w-full rounded-md px-2.5 py-1.5 text-left text-meta text-jolu-text hover:bg-jolu-side">
              {hidden ? "在列表里显示" : "从列表隐藏"}
            </button>
          </div>
        )}
      </div>
      {/* 断开是带确认与还原的动作，不用开关表达（开关像即时启停）；未接入时头部只有「⋯」，接入按钮在下面的「选起点」里 */}
      {row.toggle === "on" && (
        <JoluButton size="md" variant="danger-soft" disabled={busy} onClick={onUnwire}>
          <Undo2 aria-hidden />
          断开并还原
        </JoluButton>
      )}
    </div>
  )
}

/** 配置方案下拉（画板 iTvRW 右上「⌄ 配置方案」；菜单内容无画板，沿用 V5 的方案 chips 功能，标「待确认」）。 */
function ProfileMenu({ profiles, busy, onProfileApply, onProfileSave, onProfileDelete }: AgentDetailProps) {
  const [open, setMenuOpen] = useState(false)
  const [naming, setNaming] = useState<string | null>(null)
  return (
    <div className="relative">
      <button type="button" aria-expanded={open} aria-label="配置方案" disabled={busy}
        onClick={() => { setMenuOpen((value) => !value); setNaming(null) }}
        className="flex h-9 items-center gap-1.5 rounded-lg border border-jolu-border px-3.5 text-meta font-medium text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
        配置方案
        <ChevronDown aria-hidden className="size-3.5 text-jolu-muted" />
      </button>
      {open && (
        <div role="menu" className="absolute right-0 top-10 z-10 w-64 rounded-lg border border-jolu-border bg-jolu-bg p-1.5 shadow-lg">
          <p className="px-2 pb-1.5 text-note text-jolu-muted">把这套方案应用到各已接入的工具</p>
          {profiles.length === 0 && naming === null && <p className="px-2 py-1.5 text-meta text-jolu-muted">还没有保存过方案</p>}
          {profiles.map((profile) => (
            <div key={profile.name} className="flex items-center gap-1 rounded-md px-2 py-1 hover:bg-jolu-side">
              <button type="button" role="menuitem" disabled={busy} onClick={() => { setMenuOpen(false); onProfileApply(profile.name) }}
                className="min-w-0 flex-1 truncate text-left text-meta text-jolu-text outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
                {profile.name}
              </button>
              <button type="button" disabled={busy} aria-label={`删除方案 ${profile.name}`} onClick={() => onProfileDelete(profile.name)}
                className="shrink-0 text-note text-jolu-muted outline-none hover:text-jolu-danger">×</button>
            </div>
          ))}
          {naming === null ? (
            <button type="button" role="menuitem" disabled={busy} onClick={() => setNaming("")}
              className="w-full rounded-md px-2 py-1.5 text-left text-meta text-jolu-on hover:bg-jolu-side">＋ 保存当前配置为新方案</button>
          ) : (
            <form className="flex items-center gap-1.5 px-2 py-1.5" onSubmit={(event) => { event.preventDefault(); if (naming.trim()) { onProfileSave(naming.trim()); setNaming(null); setMenuOpen(false) } }}>
              <input autoFocus value={naming} onChange={(event) => setNaming(event.target.value)} placeholder="方案名称" aria-label="方案名称"
                className="h-7 w-32 rounded-md border border-jolu-border bg-jolu-bg px-2 text-meta text-jolu-text outline-none focus:border-jolu-accent" />
              <button type="submit" className="text-meta font-medium text-jolu-on">保存</button>
              <button type="button" onClick={() => setNaming(null)} className="text-meta text-jolu-muted">取消</button>
            </form>
          )}
        </div>
      )}
    </div>
  )
}

/** 页签（画板 iTvRW）：选中项绿色粗体 + 下划线。 */
function TabBar({ tab, claude, onTab }: { tab: DetailTab; claude: boolean; onTab: (tab: DetailTab) => void }) {
  const items: { id: DetailTab; label: string }[] = [
    { id: "model", label: "模型设置" },
    ...(claude ? [{ id: "advanced" as DetailTab, label: "高级" }] : []),
    { id: "diff", label: "写入改动" },
  ]
  return (
    <div className="flex items-center gap-6 border-b border-jolu-table-divider" role="tablist" aria-label="工具详情">
      {items.map((item) => (
        <button key={item.id} type="button" role="tab" aria-selected={tab === item.id} onClick={() => onTab(item.id)}
          className={cn(
            "-mb-px flex h-9 items-center border-b-2 text-meta outline-none focus-visible:outline-2 focus-visible:-outline-offset-4 focus-visible:outline-jolu-accent",
            tab === item.id ? "border-jolu-accent font-semibold text-jolu-text" : "border-transparent text-jolu-muted hover:text-jolu-text",
          )}>
          {item.label}
        </button>
      ))}
    </div>
  )
}

/* ─────────────────────────── 模型设置（画板 iTvRW） ─────────────────────────── */

function DriftBanners({ row, busy, onRewire, onAccept, onFollowConfig }: AgentDetailProps) {
  const drift = row.wiring?.drift
  const current = row.wiring?.current_resolved || row.wiring?.current_model
  if (!drift || drift === "model_changed") {
    return drift === "model_changed" && current ? (
      <Banner tone="warn" text={`配置里的主模型已在 ${row.name} 里改成 ${row.wiring?.current_model}${row.wiring?.current_resolved ? `（${row.wiring.current_resolved}）` : ""}`}
        actions={[{ label: "以配置为准", onClick: onFollowConfig }]} busy={busy} />
    ) : null
  }
  return (
    <Banner tone="danger" text={`${driftText(drift)}：工具的配置已不指向模型路由，或接入没有完成。`}
      actions={[{ label: "重新接入", onClick: onRewire }, { label: "接受现状", onClick: onAccept }]} busy={busy} />
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

function ClaudeModelSettings(props: AgentDetailProps) {
  const { models, sourceNotes, mainModel, row, counts, onLink, busy } = props
  const [noticeShown, setNoticeShown] = useState(true)
  return (
    <div className="flex flex-col gap-5">
      <DriftBanners {...props} />
      {noticeShown && row.wiring?.notice && (
        <div className="flex items-center gap-2 px-0.5 text-meta text-[#9A5B00]">
          <Info aria-hidden className="size-[13px] shrink-0" />
          <span className="min-w-0 flex-1">重开 Claude Code 后用 <code className="rounded-[5px] bg-jolu-track px-1.5 py-px text-note">/model</code> 选模型；{row.wiring.notice}</span>
          <button type="button" onClick={() => setNoticeShown(false)} className="shrink-0 font-semibold outline-none hover:underline">知道了</button>
        </div>
      )}

      {/* 新会话默认模型 */}
      <section className="flex items-start justify-between gap-6">
        <div className="flex flex-col gap-1.5 pt-1">
          <h3 className="text-[15px] font-semibold text-jolu-text">新会话默认模型</h3>
          <p className="text-meta text-jolu-muted">更改后写入配置；已打开的会话保持原模型。</p>
        </div>
        <div className="flex w-[420px] shrink-0 flex-col gap-1.5">
          <SelectField label="新会话默认模型" value={mainModel} disabled={busy} onValueChange={props.onMainModel}
            placeholder="未选（保留工具自己的选择）" className="h-10 w-full rounded-[10px] px-3 text-meta"
            options={modelOptions(models, sourceNotes)} />
          <span className="truncate text-note text-jolu-muted">{props.sourceNotes[mainModel] ?? (mainModel ? "来源未知（模型不在当前模型池）" : "保留工具自己的选择")}</span>
        </div>
      </section>

      {/* Claude Code 档位 */}
      <section className="flex flex-col gap-3">
        <div className="flex flex-col gap-1.5">
          <h3 className="text-[15px] font-semibold text-jolu-text">Claude Code 档位</h3>
          <p className="text-meta text-jolu-muted">在工具中选择 Opus、Sonnet 等档位时，使用下方对应的模型。</p>
        </div>
        <div className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
          <div className="grid grid-cols-[160px_1fr_180px] items-center gap-4 px-5 py-2.5 text-note text-jolu-muted">
            <span>工具内档位</span><span>调用模型</span><span className="justify-self-end">思考强度</span>
          </div>
          {TIERS.map((tier, index) => <TierRow key={tier} tier={tier} index={index} {...props} />)}
        </div>
      </section>

      {/* /model 菜单 */}
      <section className="flex items-center gap-3 rounded-xl border border-jolu-border px-5 py-3.5">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-jolu-side">
          <ListFilter aria-hidden className="size-4 text-jolu-muted" />
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-[15px] font-semibold text-jolu-text">在工具里输入 /model 可切换 {models.length} 个模型</span>
          <span className="truncate text-meta text-jolu-muted">模型列表来自当前启用的来源与路由分组</span>
        </span>
        <button type="button" onClick={() => onLink("routes")}
          className="flex h-9 shrink-0 items-center gap-1.5 rounded-lg border border-jolu-border px-3.5 text-meta font-medium text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <ArrowUpRightIcon />查看模型
        </button>
      </section>

      <CountBar {...props} counts={counts} />
    </div>
  )
}

/** 档位行（画板 iTvRW）：未选模型 = 「跟随新会话默认模型」；强度在不支持时显示说明文字。 */
function TierRow({ tier, index, models, sourceNotes, mainModel, subagent, draft, busy, onApplyTiers }: AgentDetailProps & { tier: Tier; index: number }) {
  const model = draft.models[tier]
  const efforts = model ? tierEfforts(model) : []
  const setTier = (next: string) => {
    const nextEfforts = tierEfforts(next)
    const effort = draft.efforts[tier]
    onApplyTiers({ ...draft, models: { ...draft.models, [tier]: next }, efforts: { ...draft.efforts, [tier]: effort && nextEfforts.includes(effort) ? effort : null } }, mainModel, subagent)
  }
  const setEffort = (effort: string) => onApplyTiers({ ...draft, efforts: { ...draft.efforts, [tier]: effort || null } }, mainModel, subagent)
  return (
    <div className={cn("grid grid-cols-[160px_1fr_180px] items-center gap-4 px-5 py-3.5", index > 0 && "border-t border-jolu-table-divider")}>
      <span className="text-[15px] text-jolu-text">{tier[0].toUpperCase() + tier.slice(1)}</span>
      <SelectField label={`${tier} 档调用模型`} value={model} disabled={busy} onValueChange={setTier}
        placeholder="跟随新会话默认模型" className="h-10 min-w-0 rounded-[10px] px-3 text-meta"
        options={modelOptions(models, sourceNotes)} />
      {efforts.length === 0 ? (
        <span className="justify-self-end text-meta text-jolu-muted" title={model ? "这个模型不收思考强度" : "跟随主模型时强度由 Claude Code 决定"}>
          {model ? "模型不支持" : "由工具决定"}
        </span>
      ) : (
        <SelectField label={`${tier} 档思考强度`} value={draft.efforts[tier] ?? ""} disabled={busy} onValueChange={setEffort}
          className="h-9 w-[150px] justify-self-end rounded-[10px] px-2.5 text-meta"
          options={[{ value: "", label: "默认" }, ...efforts.map((effort): SelectOption => ({ value: effort, label: EFFORT_LABELS[effort] ?? effort }))]} />
      )}
    </div>
  )
}

/** 非 Claude 工具的模型设置（无 V6 画板，沿用 V5 摘要语义，标「待确认」）。 */
function GenericModelSettings(props: AgentDetailProps) {
  const { row, models, counts, onLink, sourceNotes } = props
  const current = row.wiring?.current_resolved || row.wiring?.current_model
  return (
    <div className="flex flex-col gap-5">
      <section className="flex items-start justify-between gap-6">
        <div className="flex flex-col gap-1.5 pt-1">
          <h3 className="text-[15px] font-semibold text-jolu-text">新会话从</h3>
          <p className="text-meta text-jolu-muted">在 {row.name} 里选择模型；模型路由不替它写默认模型。</p>
        </div>
        <div className="flex w-[420px] shrink-0 flex-col gap-1.5">
          <div className="flex h-10 w-full items-center rounded-lg border border-jolu-border bg-jolu-side px-3 text-meta text-jolu-text">
            <span className="truncate" title={current || undefined}>{current || "工具自己上次的选择"}</span>
          </div>
          <span className="truncate text-note text-jolu-muted">{current ? (sourceNotes[current] ?? "来源未知（模型不在当前模型池）") : `可用模型 ${models.length} 个`}</span>
        </div>
      </section>
      <section className="flex items-center gap-3 rounded-xl border border-jolu-border px-5 py-3.5">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-jolu-side">
          <ListFilter aria-hidden className="size-4 text-jolu-muted" />
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-[15px] font-semibold text-jolu-text">它的模型列表里会出现 {models.length} 个模型</span>
          <span className="truncate text-meta text-jolu-muted">来自当前启用的来源与路由分组</span>
        </span>
        <button type="button" onClick={() => onLink("routes")}
          className="flex h-9 shrink-0 items-center gap-1.5 rounded-lg border border-jolu-border px-3.5 text-meta font-medium text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <ArrowUpRightIcon />查看模型
        </button>
      </section>
      <CountBar {...props} counts={counts} />
    </div>
  )
}

/** 底部统计条（画板 iTvRW）：Skills · 会话 · 自动化 · 今日路由。 */
function CountBar({ counts, onLink }: AgentDetailProps) {
  const items: { key: AgentLink; icon: React.ElementType; label: string; value: number | null }[] = [
    { key: "skills", icon: Sparkles, label: "Skills", value: counts.skills },
    { key: "sessions", icon: MessageSquareText, label: "会话", value: counts.sessions },
    { key: "automation", icon: CalendarClock, label: "自动化", value: counts.automation },
    { key: "routes", icon: Activity, label: "今日路由", value: counts.routes },
  ]
  return (
    <div className="flex items-center gap-8 rounded-xl bg-jolu-side px-5 py-3">
      {items.map(({ key, icon: Icon, label, value }) => (
        <button key={key} type="button" onClick={() => onLink(key)}
          className="flex items-center gap-2 rounded-md px-1 py-0.5 text-left outline-none hover:text-jolu-on focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <Icon aria-hidden className="size-3.5 text-jolu-muted" />
          <span className="text-meta text-jolu-muted">{label}</span>
          <span className="text-meta font-semibold tabular-nums text-jolu-text">{value ?? "—"}</span>
        </button>
      ))}
    </div>
  )
}

/* ─────────────────────────── 高级（无 V6 画板，沿用 V5 内容，标「待确认」） ─────────────────────────── */

function AdvancedBody(props: AgentDetailProps) {
  const { models, sourceNotes, mainModel, subagent, draft, busy, longContext, onApplyTiers, onLongContext } = props
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
  const line = "h-9 min-w-0 rounded-lg border border-jolu-border bg-jolu-bg px-2.5 text-meta text-jolu-text outline-none focus:border-jolu-accent disabled:opacity-50"
  return (
    <div className="flex flex-col gap-5" data-advanced>
      <p className="text-note text-jolu-muted">高级设置直接写入 Claude Code 的配置；改动立即生效（待确认：无 V6 画板，沿用 V5 内容）。</p>
      <section className="flex items-start justify-between gap-6">
        <div className="flex flex-col gap-1.5 pt-1">
          <h3 className="text-[15px] font-semibold text-jolu-text">子代理模型</h3>
          <p className="text-meta text-jolu-muted">Explore 等子代理用的模型；不设就由 Claude Code 自己挑选。</p>
        </div>
        <SelectField label="子代理模型" value={subagent} disabled={busy}
          onValueChange={(value) => onApplyTiers(draft, mainModel, value)}
          placeholder="由 Claude Code 自己挑选" className={cn(line, "w-[420px] shrink-0 rounded-[10px] px-3 text-meta")}
          options={modelOptions(models, sourceNotes)} />
      </section>
      <section className="flex items-start justify-between gap-6">
        <div className="flex flex-col gap-1.5 pt-1">
          <h3 className="text-[15px] font-semibold text-jolu-text">自动模式</h3>
          <p className="text-meta text-jolu-muted">自动模式下检查每个操作用的模型。</p>
        </div>
        <SelectField label="自动模式模型" value={autoValue} disabled={busy} onValueChange={setAuto}
          className={cn(line, "w-[420px] shrink-0 rounded-[10px] px-3 text-meta")}
          options={[
            { value: "default", label: "默认（Claude Code 自己挑）" },
            { value: "subagent", label: "跟随子代理" },
            ...models.map((model): SelectOption => ({ value: `model:${model}`, label: model, description: sourceNotes[model] })),
          ]} />
      </section>
      <section className="flex items-start justify-between gap-6">
        <div className="flex flex-col gap-1.5 pt-1">
          <h3 className="text-[15px] font-semibold text-jolu-text">长对话</h3>
          <p className="text-meta text-jolu-muted">到这个大小时压缩；只对窗口大于 200K 的模型生效，窗口未知的模型不写。</p>
        </div>
        <div className="flex w-[420px] shrink-0 flex-col gap-1.5">
          <div className="flex items-center gap-2">
            <div className="flex items-center gap-0.5 rounded-lg bg-jolu-track p-0.5" role="group" aria-label="长对话压缩">
              {segment("default", "272K 时压缩")}
              {segment("full", "完整窗口")}
            </div>
            <form className="flex items-center gap-1.5" onSubmit={(event) => { event.preventDefault(); if (customValid) onLongContext(custom.trim()) }}>
              <input value={custom} onChange={(event) => setCustom(event.target.value)} placeholder="自定义，如 400k" aria-label="自定义压缩大小"
                className={cn("h-8 w-28 rounded-md border bg-jolu-bg px-2 text-meta text-jolu-text outline-none", mode === "custom" ? "border-jolu-accent" : "border-jolu-border", custom && !customValid && "border-jolu-danger")} />
              <button type="submit" disabled={busy || !customValid} className="text-meta font-medium text-jolu-on disabled:opacity-40">应用</button>
            </form>
          </div>
          <span className="text-note text-jolu-muted">完整窗口每轮更慢、更贵。</span>
        </div>
      </section>
      <section className="flex items-start justify-between gap-6">
        <div className="flex flex-col gap-1.5 pt-1">
          <h3 className="text-[15px] font-semibold text-jolu-text">新会话强度</h3>
          <p className="text-meta text-jolu-muted">新会话默认的思考强度。</p>
        </div>
        <span title="新会话默认强度（F4）待确认" className="w-[420px] shrink-0"><SelectField label="新会话强度" value="default" disabled onValueChange={() => undefined}
          className="w-full rounded-[10px] px-3 text-meta"
          options={[{ value: "default", label: "默认（待确认）" }]} /></span>
      </section>
    </div>
  )
}

/* ─────────────────────────── 写入改动（无 V6 画板，沿用 V5 内容，标「待确认」） ─────────────────────────── */

function DiffBody({ preview, previewError }: AgentDetailProps) {
  const files = preview?.files ?? []
  const keys = files.reduce((sum, file) => sum + file.keys, 0)
  return (
    <div className="flex flex-col gap-4">
      <p className="text-note text-jolu-muted">
        {files.length ? `改了 ${files.length} 个文件 · ${keys} 个键 · 断开时逐键还原（待确认：无 V6 画板，沿用 V5 内容）` : "没有检测到 Jolu 的改动（待确认：无 V6 画板）"}
      </p>
      {previewError && <p className="text-meta text-jolu-danger">{previewError}</p>}
      {!preview && !previewError && <p className="text-meta text-jolu-muted">正在生成写入预览…</p>}
      {files.map((file) => <DiffView key={file.path} file={file} caption="断开时还原成左侧（-）的样子" />)}
    </div>
  )
}

/* ─────────────────────────── 接入前预览（画板 V5-2，无 V6 画板，标「待确认」） ─────────────────────────── */

function PreviewBody({ row, models, sourceNotes, mainModel, onMainModel, routing, preview, previewError, busy, onWire }: AgentDetailProps) {
  const claude = row.agent === "claude-code"
  const files = preview?.mode === "wire" ? preview.files : []
  const icons = { file: FilePlus2, shield: ShieldCheck, undo: Undo2 }
  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col gap-2.5 rounded-xl border border-jolu-border p-4">
        <span className="text-[15px] font-semibold text-jolu-text">{claude ? "接入后，新会话从哪个模型开始" : `接入 ${row.name}`}</span>
        <div className="flex flex-wrap items-center gap-2.5">
          {claude && (
            <SelectField label="起点模型" value={mainModel} disabled={busy} onValueChange={onMainModel}
              placeholder="不选（保留工具自己的选择）" className="h-10 w-[300px] rounded-[10px] px-3 text-meta"
              options={modelOptions(models, sourceNotes)} />
          )}
          <button type="button" disabled={busy} onClick={onWire}
            className="flex h-10 items-center gap-1.5 rounded-lg bg-jolu-accent px-4 text-meta font-semibold text-[#071B10] outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:opacity-45">
            <Plug aria-hidden className="size-4" />
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
        <span className="text-[15px] font-semibold text-jolu-text">接入会做什么</span>
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
    </div>
  )
}

/** 暂不支持（画板 V8 Qoder 卡语义）：探测到但 Jolu 还没有专用接入；指到通用接入。 */
function UnsupportedBody({ row }: { row: AgentRow }) {
  return (
    <div className="flex flex-col gap-2 rounded-xl border border-jolu-border p-4">
      <span className="text-[15px] font-semibold text-jolu-text">等待接入适配</span>
      <span className="text-meta text-jolu-muted">
        Jolu 检测到了 {row.name}，还没有它的专用接入；模型来源与工具接入能力会分别确认。
      </span>
      <span className="text-note text-jolu-muted">{row.wiring?.config_path ? <>配置位置：<code className="font-mono">{row.wiring.config_path}</code></> : row.subtitle}</span>
    </div>
  )
}

function BlockedBody({ row }: { row: AgentRow }) {
  const cliMissing = row.discovery?.state === "cli_missing"
  return (
    <div className="flex flex-col gap-2 rounded-xl border border-jolu-border p-4">
      <span className="text-[15px] font-semibold text-jolu-text">暂时不能接入</span>
      <span className="text-meta text-jolu-muted">
        {cliMissing ? "检测到它的配置，但找不到命令行程序。装回 CLI 后点「重新检测」。" : `${row.subtitle}。装好并运行过一次后，点「重新检测」。`}
      </span>
      {row.wiring?.config_path && <span className="font-mono text-note text-jolu-muted">配置位置：{row.wiring.config_path}</span>}
    </div>
  )
}

/** 模型下拉的公共选项：label = 模型 id，描述 = 来源说明（V6 画板「来源：工作账号 · 订阅连接」）。 */
function modelOptions(models: readonly string[], sourceNotes: Record<string, string>): SelectOption[] {
  return models.map((model): SelectOption => ({ value: model, label: model, description: sourceNotes[model] }))
}

function ArrowUpRightIcon() {
  return (
    <svg aria-hidden viewBox="0 0 14 14" className="size-3 text-jolu-muted" fill="none" stroke="currentColor" strokeWidth="1.5">
      <path d="M4 10L10 4M10 4H5.5M10 4V8.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}
