import { useState } from "react"
import type { ProfileDto, WiringPreviewDto } from "@/lib/gatewayApi"
import type { AgentVersionRow } from "@/lib/settingsApi"
import { driftText, type AgentRow } from "@/lib/agentPage"
import type { TierDraft } from "@/lib/tierModels"
import { Icon } from "@/icons"
import { cn } from "@/lib/utils"
import { IconButton, StatusPill, Switch, Tabs, type StatusTone } from "@/ui"
import { AgentBadge } from "./AgentBadge"
import { AdvancedBody, BlockedBody, DiffBody, PreviewBody, ProfileMenu, UnsupportedBody } from "./AgentDetailPanels"
import { ClaudeModelSettings, GenericModelSettings } from "./AgentDetailModel"

export interface AgentDetailProps {
  row: AgentRow
  version: AgentVersionRow | null
  /** 模型路由是否在运行（中转模式）：没开时接入会先开启它 */
  routing: boolean
  preview: WiringPreviewDto | null
  previewError: string | null
  /** 今日经过该工具的路由请求数；null = 查询失败 / 还没拿到 */
  todayRoutes: number | null
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
  /** 进模型路由页看完整的模型清单 */
  onOpenModels: () => void
}

type DetailTab = "model" | "advanced" | "diff"

/**
 * Agent 详情 —— 设计稿 R1-09（模型设置）、R1-32（高级）、R1-33（写入改动）。
 *
 * 布局自顶向下：
 * 1. **工具头**：返回胶囊 + 标识 + 名称/配置位置 + 状态胶囊 + 间距 +「接入模型路由」开关；
 * 2. **页签栏**：模型设置 / 高级（仅 Claude Code）/ 写入改动；
 * 3. **主体**：由子组件按页签渲染（模型设置是左栏 + 右栏 300 的两栏）。
 *
 * 与 V8 的差异：详情从「右侧抽屉」改成**整页切换**——R1-09 带「← 全部工具」
 * 返回，说明详情是一页而不是浮层。列表状态由 `AgentsView` 保留，返回时
 * 不重新探测（探测结果在模块级缓存里）。
 *
 * 画板没有「配置方案」与「从列表隐藏」，两处沿用既有功能并标「待确认」。
 */
export function AgentDetail(props: AgentDetailProps) {
  const { row } = props
  const [tab, setTab] = useState<DetailTab>("model")
  const wired = row.group === "wired"
  const claude = row.agent === "claude-code"
  const tabs: { key: DetailTab; label: string }[] = [
    { key: "model", label: "模型设置" },
    ...(claude ? [{ key: "advanced" as DetailTab, label: "高级" }] : []),
    { key: "diff", label: "写入改动" },
  ]
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3.5 overflow-y-auto px-0.5 pb-2 pt-0.5 [&>*]:shrink-0">
      <Header {...props} />
      {wired ? (
        <>
          <div className="flex flex-wrap items-center gap-2.5">
            <Tabs value={tab} onChange={setTab} items={tabs} />
            <span className="min-w-0 flex-1" />
            <ProfileMenu {...props} />
          </div>
          {tab === "model" && (claude ? <ClaudeModelSettings {...props} /> : <GenericModelSettings {...props} />)}
          {tab === "advanced" && claude && <AdvancedBody {...props} />}
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

/** 工具头（R1-09）：返回、标识、名称与配置位置、状态胶囊、接入开关。 */
function Header({ row, version, busy, hidden, onBack, onWire, onUnwire, onToggleHidden }: AgentDetailProps) {
  const [menu, setMenu] = useState(false)
  const drift = row.wiring?.drift ?? null
  const wired = row.group === "wired"
  const statusTone: StatusTone = wired
    ? row.tone === "danger" ? "danger" : row.tone === "warn" ? "warning" : "success"
    : row.group === "available" ? "neutral" : "muted"
  const statusLabel = wired
    ? drift ? driftText(drift) ?? "已接入" : "已接入 · 配置一致"
    : row.group === "available" ? "未接入" : "不能接入"
  const subtitle = wired
    ? [row.wiring?.config_path, driftText(drift)].filter(Boolean).join(" · ") || "已接入 · 经模型路由调用模型"
    : row.group === "available"
      ? `现在用它自己的配置${row.wiring?.config_path ? ` · 依据：${row.wiring.config_path}` : ""}`
      : row.subtitle
  return (
    <div className="flex items-center gap-3.5">
      <button type="button" onClick={onBack}
        className="jn-interactive inline-flex h-8 shrink-0 items-center gap-1 rounded-full bg-jn-surface pr-3 pl-2 text-[12.5px] font-medium text-jn-text-2 shadow-jn-surface">
        <Icon name="prev" size="sm" />
        全部工具
      </button>
      <AgentBadge agent={row.agent} name={row.name} size={40} />
      <div className="flex min-w-0 max-w-[360px] flex-none flex-col gap-0.5">
        <div className="flex min-w-0 items-baseline gap-2.5">
          <span className="truncate text-[20px] leading-7 font-bold text-jn-text">{row.name}</span>
          {version?.version && <span className="shrink-0 text-[12px] text-jn-text-3">{version.version}</span>}
        </div>
        <span className={cn("truncate text-[12px]", row.tone === "danger" ? "text-jn-danger" : "text-jn-text-3")} title={subtitle}>{subtitle}</span>
      </div>
      <StatusPill tone={statusTone} className="max-w-[240px] overflow-hidden">{statusLabel}</StatusPill>
      <span className="min-w-0 flex-1" />
      <div className="relative">
        <IconButton icon="more" label="更多操作" kind="surface" size="md" aria-expanded={menu} onClick={() => setMenu((value) => !value)} />
        {menu && (
          <div role="menu" className="absolute top-10 right-0 z-20 w-44 rounded-[14px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface">
            <button type="button" role="menuitem" onClick={() => { setMenu(false); onToggleHidden() }}
              className="w-full rounded-[10px] px-2.5 py-1.5 text-left text-[12px] text-jn-text hover:bg-jn-well">
              {hidden ? "在列表里显示" : "从列表隐藏"}
            </button>
          </div>
        )}
      </div>
      {/* 开关表达接入状态：关 = 断开并还原（带确认），开 = 接入。暂不支持的没有开关 */}
      {row.toggle !== null && (
        <span className="flex shrink-0 items-center gap-2">
          <span className="text-[12.5px] font-medium text-jn-text-2">接入模型路由</span>
          <Switch checked={row.toggle === "on"} disabled={busy} label={`${row.toggle === "on" ? "断开" : "接入"} ${row.name}`}
            onChange={(next) => { if (next) onWire(); else onUnwire() }} />
        </span>
      )}
    </div>
  )
}
