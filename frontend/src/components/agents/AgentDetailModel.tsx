import type { ReactNode } from "react"
import { Banner, Button, CapabilityTag, DataValue, Select, SourceNote, type SelectOption } from "@/ui"
import { cn } from "@/lib/utils"
import { Icon } from "@/icons"
import { AGENT_PLATFORM, driftText, type AgentRow } from "@/lib/agentPage"
import { capLevel, capabilityRows, hasCapabilityPlatform } from "@/lib/capabilityVerification"
import { EFFORT_LABELS, TIERS, tierEfforts, type Tier } from "@/lib/tierModels"
import type { AgentDetailProps } from "./AgentDetail"

/**
 * Agent 详情 · 模型设置页 —— 设计稿 R1-09「Agent · 详情（Claude Code）」。
 *
 * 两栏：左栏（弹性）是「新会话默认模型 / 档位 / /model 菜单」三张卡，
 * 右栏（固定 300）是「今天经过路由 / 能力验证」。
 *
 * 右栏的数据边界要写清楚：路由流水只按工具统计**请求数**，Token / 失败 /
 * 改道没有按工具聚合的数据，因此用 `DataValue` 的 unknown 态显示「—」，
 * 不拿全站合计冒充单工具数据。
 */

/** 模型下拉的公共选项：label = 模型 id，hint = 来源说明（画板「来源：工作账号 · 订阅连接」）。 */
export function modelOptions(models: readonly string[], sourceNotes: Record<string, string>): SelectOption[] {
  return models.map((model) => ({ value: model, label: model, hint: sourceNotes[model] }))
}

/** Claude Code 的模型设置：默认模型 + 四档 + 子代理 + /model 菜单。 */
export function ClaudeModelSettings(props: AgentDetailProps) {
  const { models, sourceNotes, mainModel, row, busy, onMainModel, onOpenModels } = props
  return (
    <div className="flex items-start gap-3.5">
      <div className="flex min-w-0 flex-1 flex-col gap-3.5">
        <DriftBanners {...props} />
        <Notice row={row} />
        <section className="flex items-center justify-between gap-4 rounded-jn-card border border-jn-line bg-jn-surface px-5 pt-[18px] pb-4 shadow-jn-surface">
          <div className="flex min-w-0 flex-col gap-[3px]">
            <h3 className="text-[14px] font-semibold text-jn-text">新会话默认模型</h3>
            <span className="truncate text-[12px] text-jn-text-3">
              {sourceNotes[mainModel] ?? (mainModel ? "来源未知（模型不在当前模型池）" : "不选则保留工具自己的选择")}
            </span>
          </div>
          <div className="w-[320px] shrink-0">
            <Select
              width="full"
              label="新会话默认模型"
              value={mainModel || null}
              disabled={busy}
              onChange={onMainModel}
              placeholder="保留工具自己的选择"
              options={modelOptions(models, sourceNotes)}
            />
          </div>
        </section>

        <TierCard {...props} />

        <section className="flex items-center gap-3 rounded-jn-card border border-jn-line bg-jn-surface px-[18px] py-3.5 shadow-jn-surface">
          <span className="flex size-9 shrink-0 items-center justify-center rounded-[12px] bg-jn-well">
            <Icon name="blocks" size="md" className="text-jn-text-2" />
          </span>
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="truncate text-[14px] font-semibold text-jn-text">/model 菜单有 {models.length} 个模型</span>
            <span className="truncate text-[12px] text-jn-text-3">来自已启用来源与路由分组</span>
          </span>
          <Button kind="ghost" size="sm" icon="external" onClick={onOpenModels}>查看模型</Button>
        </section>
      </div>
      <AgentSideColumn row={row} todayRoutes={props.todayRoutes} />
    </div>
  )
}

/** 非 Claude 工具的模型设置（画板未画，标「待确认」）：只读当前模型 + 模型清单入口。 */
export function GenericModelSettings(props: AgentDetailProps) {
  const { row, models, sourceNotes, onOpenModels } = props
  const current = row.wiring?.current_resolved || row.wiring?.current_model
  return (
    <div className="flex items-start gap-3.5">
      <div className="flex min-w-0 flex-1 flex-col gap-3.5">
        <DriftBanners {...props} />
        <Notice row={row} />
        <section className="flex items-center justify-between gap-4 rounded-jn-card border border-jn-line bg-jn-surface px-5 pt-[18px] pb-4 shadow-jn-surface">
          <div className="flex min-w-0 flex-col gap-[3px]">
            <h3 className="text-[14px] font-semibold text-jn-text">当前模型</h3>
            <span className="truncate text-[12px] text-jn-text-3">
              {current ? (sourceNotes[current] ?? "来源未知（模型不在当前模型池）") : `可用模型 ${models.length} 个`}
            </span>
          </div>
          <div className="flex h-9 w-[320px] shrink-0 items-center rounded-full bg-jn-well px-3.5 text-[13px] text-jn-text">
            <span className="truncate" title={current || undefined}>{current || "工具自己上次的选择"}</span>
          </div>
        </section>
        <section className="flex items-center gap-3 rounded-jn-card border border-jn-line bg-jn-surface px-[18px] py-3.5 shadow-jn-surface">
          <span className="flex size-9 shrink-0 items-center justify-center rounded-[12px] bg-jn-well">
            <Icon name="blocks" size="md" className="text-jn-text-2" />
          </span>
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="truncate text-[14px] font-semibold text-jn-text">模型列表有 {models.length} 个模型</span>
            <span className="truncate text-[12px] text-jn-text-3">来自已启用来源与路由分组</span>
          </span>
          <Button kind="ghost" size="sm" icon="external" onClick={onOpenModels}>查看模型</Button>
        </section>
      </div>
      <AgentSideColumn row={row} todayRoutes={props.todayRoutes} />
    </div>
  )
}

/** Claude Code 档位表（R1-09）：表头 130 + 弹性 + 150；行 52、偶数行 well 底。 */
function TierCard(props: AgentDetailProps) {
  return (
    <section className="flex flex-col gap-1.5 rounded-jn-card border border-jn-line bg-jn-surface px-5 pt-[18px] pb-3.5 shadow-jn-surface">
      <div className="flex h-10 items-center gap-2">
        <Icon name="activity" size="md" className="text-jn-text-2" />
        <h3 className="text-[14px] font-semibold text-jn-text">Claude Code 档位</h3>
        <span className="min-w-0 flex-1" />
        <span className="text-[12px] text-jn-text-3">{TIERS.length} 个档位</span>
      </div>
      <div className="flex h-7 items-center px-2.5 text-[11.5px] font-medium text-jn-text-3">
        <span className="w-[118px] shrink-0">工具内档位</span>
        <span className="min-w-0 flex-1">调用模型</span>
        <span className="w-[150px] shrink-0">思考强度</span>
      </div>
      {TIERS.map((tier, index) => (
        <TierRow key={tier} tier={tier} striped={index % 2 === 0} {...props} />
      ))}
    </section>
  )
}

function TierRow({ tier, striped, models, sourceNotes, mainModel, subagent, draft, busy, onApplyTiers }: AgentDetailProps & { tier: Tier; striped: boolean }) {
  const model = draft.models[tier]
  const efforts = model ? tierEfforts(model) : []
  const setTier = (next: string) => {
    const nextEfforts = tierEfforts(next)
    const effort = draft.efforts[tier]
    onApplyTiers(
      {
        ...draft,
        models: { ...draft.models, [tier]: next },
        efforts: { ...draft.efforts, [tier]: effort && nextEfforts.includes(effort) ? effort : null },
      },
      mainModel,
      subagent,
    )
  }
  const setEffort = (effort: string) => onApplyTiers({ ...draft, efforts: { ...draft.efforts, [tier]: effort || null } }, mainModel, subagent)
  return (
    <div className={cn("flex h-[52px] items-center gap-3 px-2.5", striped && "rounded-[14px] bg-jn-well")}>
      <span className="w-[118px] shrink-0 truncate text-[13.5px] font-semibold text-jn-text">{tier[0].toUpperCase() + tier.slice(1)}</span>
      <div className="min-w-0 flex-1">
        <Select
          width="full"
          label={`${tier} 档调用模型`}
          value={model || null}
          disabled={busy}
          onChange={setTier}
          placeholder="跟随新会话默认模型"
          options={modelOptions(models, sourceNotes)}
        />
      </div>
      {efforts.length === 0 ? (
        <span className="w-[150px] shrink-0 truncate text-[12px] text-jn-text-3" title={model ? "这个模型不收思考强度" : "跟随主模型时强度由 Claude Code 决定"}>
          {model ? "模型不支持" : "由工具决定"}
        </span>
      ) : (
        <div className="w-[150px] shrink-0">
          <Select
            width="full"
            label={`${tier} 档思考强度`}
            value={draft.efforts[tier] ?? ""}
            disabled={busy}
            onChange={setEffort}
            options={[{ value: "", label: "默认" }, ...efforts.map((effort): SelectOption => ({ value: effort, label: EFFORT_LABELS[effort] ?? effort }))]}
          />
        </div>
      )}
    </div>
  )
}

/** 漂移提示（R1-09 的顶部提示条）：配置被改 / 主模型被换。 */
function DriftBanners({ row, busy, onRewire, onAccept, onFollowConfig }: AgentDetailProps) {
  const drift = row.wiring?.drift
  const current = row.wiring?.current_resolved || row.wiring?.current_model
  if (!drift) return null
  if (drift === "model_changed") {
    if (!current) return null
    return (
      <Banner tone="warning" className="gap-2.5 rounded-[16px] px-3.5 py-[11px] text-[12.5px] font-medium"
        action={<Button kind="ghost" size="sm" disabled={busy} onClick={onFollowConfig}>以配置为准</Button>}>
        <span className="text-jn-text">
          配置里的主模型已在 {row.name} 里改成 {row.wiring?.current_model}
          {row.wiring?.current_resolved ? `（${row.wiring.current_resolved}）` : ""}
        </span>
      </Banner>
    )
  }
  return (
    <Banner tone="danger" className="gap-2.5 rounded-[16px] px-3.5 py-[11px] text-[12.5px] font-medium"
      action={
        <span className="flex items-center gap-2">
          <Button kind="ghost" size="sm" disabled={busy} onClick={onRewire}>重新接入</Button>
          <Button kind="ghost" size="sm" disabled={busy} onClick={onAccept}>接受现状</Button>
        </span>
      }>
      <span className="text-jn-text">{driftText(drift)}：工具的配置已不指向模型路由，或接入没有完成。</span>
    </Banner>
  )
}

/** `wiring.notice` 的提示（重开工具后生效等）。可关掉，但只在本次详情里生效。 */
function Notice({ row }: { row: AgentRow }) {
  if (!row.wiring?.notice) return null
  return (
    <Banner tone="info" className="gap-2.5 rounded-[16px] px-3.5 py-[11px] text-[12.5px] font-medium">
      <span className="text-jn-text">
        {row.agent === "antigravity" ? (
          row.wiring.notice
        ) : (
          <>
            重开 {row.name} 后用 <code className="rounded-[5px] bg-jn-track px-1.5 py-px text-[11px]">/model</code> 选模型；{row.wiring.notice}
          </>
        )}
      </span>
    </Banner>
  )
}

/**
 * 右栏（R1-09）：今天经过路由 + 能力验证。
 *
 * 「今天经过路由」只显示请求数——路由流水没有按工具聚合的 Token / 失败 /
 * 改道；三行用 unknown 态显示「—」，并在来源脚注里写明缺口，
 * 不拿全站合计冒充单个工具的数据。
 */
function AgentSideColumn({ row, todayRoutes }: { row: AgentRow; todayRoutes: number | null }) {
  const platform = AGENT_PLATFORM[row.agent] ?? null
  const rows = platform && hasCapabilityPlatform(platform) ? capabilityRows(platform) : []
  return (
    <aside className="flex w-[300px] shrink-0 flex-col gap-3.5">
      <section className="flex flex-col gap-2.5 rounded-jn-card border border-jn-line bg-jn-surface px-5 pt-[18px] pb-4 shadow-jn-surface">
        <div className="flex items-center gap-2">
          <Icon name="activity" size="md" className="text-jn-text-2" />
          <h3 className="text-[14px] font-semibold text-jn-text">今天经过路由</h3>
        </div>
        <Metric label="请求">
          {todayRoutes === null ? <DataValue state="unknown" /> : <DataValue state={todayRoutes === 0 ? "zero" : "value"} value={todayRoutes} unit="次" />}
        </Metric>
        <Metric label="Token"><DataValue state="unknown" note="未按工具统计" /></Metric>
        <Metric label="失败"><DataValue state="unknown" note="未按工具统计" /></Metric>
        <Metric label="改道"><DataValue state="unknown" note="未按工具统计" /></Metric>
        <SourceNote source="routed" coverage="仅经过模型路由的请求" detail="Token / 失败 / 改道暂无按工具聚合" />
      </section>

      <section className="flex min-h-[220px] flex-1 flex-col gap-2.5 rounded-jn-card border border-jn-line bg-jn-surface px-5 pt-[18px] pb-4 shadow-jn-surface">
        <div className="flex items-center gap-2">
          <Icon name="shield" size="md" className="text-jn-text-2" />
          <h3 className="text-[14px] font-semibold text-jn-text">能力验证</h3>
        </div>
        {rows.length === 0 ? (
          <div className="flex flex-col gap-1 py-2">
            <span className="text-[12.5px] text-jn-text-3">能力矩阵还没有这家工具的记录。</span>
            <span className="text-[11.5px] text-jn-text-3">没有记录不等于不支持。</span>
          </div>
        ) : (
          rows.map((item) => (
            <div key={item.capability} className="flex items-center gap-2">
              <span className="text-[12.5px] font-medium text-jn-text">{item.name}</span>
              <span className="min-w-0 flex-1" />
              <CapabilityTag level={item.cell ? capLevel(item.cell.state) : "unknown"} />
            </div>
          ))
        )}
        <p className="mt-auto text-[10px] leading-relaxed text-jn-text-3">依据 docs/verification 下的验收记录；合成测试不算真实跑通。</p>
      </section>
    </aside>
  )
}

/** 右栏的指标行：左标签、右数值。 */
function Metric({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-[12.5px] text-jn-text-2">{label}</span>
      <span className="min-w-0 flex-1" />
      {children}
    </div>
  )
}
