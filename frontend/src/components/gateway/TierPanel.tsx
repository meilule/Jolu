import { useState } from "react"
import { cn } from "@/lib/utils"
import {
  EFFORT_LABELS, TIERS, TIER_LABELS, tierEfforts, tierSummary,
  type AutoChoice, type Tier, type TierDraft,
} from "@/lib/tierModels"

/**
 * Claude Code 档位自选（批 4）：四档各选模型与 effort，外加 Auto 判别档。
 *
 * 待确认：`Jolu.pen` 没有这块画板（语义参考 `docs/plans/2026-11-10-capability-and-correctness-plan.md`
 * 第二部分），按接线弹窗现有的灰底分组与原生下拉做最小版。
 *
 * 「自选保持」：选过的档位存绝对值，换主模型时不跟着变；仍是「跟随主模型」的档位才跟随。
 */
export function TierPanel({ models, mainModel, subagent, draft, onDraft, busy, wired, onApply }: {
  models: { id: string }[]
  mainModel: string
  subagent: string
  draft: TierDraft
  onDraft: (next: TierDraft) => void
  busy: boolean
  /** Claude Code 已接线：可直接「应用档位」；未接线时随「接线」一起写入 */
  wired: boolean
  onApply: () => void
}) {
  const [open, setOpen] = useState(false)
  const chosen = TIERS.filter((tier) => draft.models[tier]).length + (draft.auto.kind === "default" ? 0 : 1)
  const setModel = (tier: Tier, model: string) => {
    const efforts = tierEfforts(model)
    const effort = draft.efforts[tier]
    onDraft({
      ...draft,
      models: { ...draft.models, [tier]: model },
      efforts: { ...draft.efforts, [tier]: effort && efforts.includes(effort) ? effort : null },
    })
  }
  const setEffort = (tier: Tier, effort: string | null) => onDraft({ ...draft, efforts: { ...draft.efforts, [tier]: effort } })
  const autoValue = draft.auto.kind === "model" ? `model:${draft.auto.model}` : draft.auto.kind
  const setAuto = (value: string) => {
    const auto: AutoChoice = value === "default" ? { kind: "default" } : value === "subagent" ? { kind: "subagent" } : { kind: "model", model: value.slice("model:".length) }
    onDraft({ ...draft, auto })
  }
  const select = "h-8 min-w-0 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[11px] text-jolu-text outline-none focus:border-jolu-accent"

  return (
    <div className="mt-1 flex flex-col gap-1 border-t border-jolu-border pt-1.5" data-tier-panel>
      <button type="button" onClick={() => setOpen((value) => !value)} className="flex items-center justify-between text-[9.5px] text-jolu-text outline-none">
        <span>档位（待确认）</span>
        <span className="text-jolu-muted">{open ? "收起 ▲" : `展开 ▼（${chosen ? `已自选 ${chosen} 项` : "全部跟随"}）`}</span>
      </button>
      {open && (
        <div className="flex flex-col gap-1.5">
          <span className="text-[8.5px] text-jolu-muted">选过的档位换主模型时保持不变；仍为「跟随主模型」的档位随主模型变化。effort 只列这个模型在 Claude Code 里收的档。</span>
          {TIERS.map((tier) => {
            const model = draft.models[tier]
            const efforts = model ? tierEfforts(model) : []
            return (
              <div key={tier} className="flex flex-col gap-1">
                <div className="flex items-center gap-1.5">
                  <span className="w-[62px] shrink-0 text-[10px] text-jolu-text">{TIER_LABELS[tier]}</span>
                  <select aria-label={`${TIER_LABELS[tier]}模型`} value={model} disabled={busy} onChange={(event) => setModel(tier, event.target.value)} className={cn(select, "flex-1")}>
                    <option value="">跟随主模型</option>
                    {models.map((entry) => <option key={entry.id} value={entry.id}>{entry.id}</option>)}
                  </select>
                </div>
                {model && (
                  <div className="ml-[68px] flex flex-wrap items-center gap-1" role="radiogroup" aria-label={`${TIER_LABELS[tier]} effort`}>
                    {efforts.length === 0 ? (
                      <span className="text-[8.5px] text-jolu-muted">这个模型不收 effort</span>
                    ) : (
                      [null, ...efforts].map((effort) => (
                        <button
                          key={effort ?? "default"}
                          type="button"
                          role="radio"
                          aria-checked={draft.efforts[tier] === effort}
                          disabled={busy}
                          onClick={() => setEffort(tier, effort)}
                          className={cn(
                            "h-6 rounded-md border px-2 text-[9.5px] outline-none",
                            draft.efforts[tier] === effort ? "border-jolu-accent bg-jolu-selected text-jolu-on" : "border-jolu-border bg-jolu-bg text-jolu-muted",
                          )}
                        >
                          {effort ? EFFORT_LABELS[effort] ?? effort : "默认"}
                        </button>
                      ))
                    )}
                  </div>
                )}
                <span className="ml-[68px] truncate text-[8.5px] text-jolu-muted">{tierSummary(draft, tier, mainModel)}</span>
              </div>
            )
          })}
          <div className="flex items-center gap-1.5">
            <span className="w-[62px] shrink-0 text-[10px] text-jolu-text">Auto 判别</span>
            <select aria-label="Auto 判别模型" value={autoValue} disabled={busy} onChange={(event) => setAuto(event.target.value)} className={cn(select, "flex-1")}>
              <option value="default">交给 Claude Code 自己挑</option>
              <option value="subagent">跟随子代理 · {subagent || `主模型 ${mainModel || "未选"}`}</option>
              {models.map((entry) => <option key={entry.id} value={`model:${entry.id}`}>{entry.id}</option>)}
            </select>
          </div>
          <span className="text-[8.5px] text-jolu-muted">Auto 判别是 Claude Code 每个动作前先跑的分类器，选便宜的模型能省不少。</span>
          {wired && (
            <button type="button" disabled={busy} onClick={onApply}
              className="h-8 self-end rounded-lg bg-jolu-accent px-3 text-[11px] font-medium text-[#071B10] outline-none disabled:opacity-45">
              应用档位
            </button>
          )}
          {!wired && <span className="text-[8.5px] text-jolu-muted">Claude Code 还没接线：点下面的「接线」时一并写入。</span>}
        </div>
      )}
    </div>
  )
}
