import { useState } from "react"
import { ShieldCheck } from "lucide-react"
import { cn } from "@/lib/utils"
import { CAPABILITIES, PLATFORMS, STATE_LABEL, cellOf, cellSummary, stateCounts, type VerifyCell, type VerifyState } from "@/lib/capabilityVerification"
import { SettingsGroup } from "./settingsUi"

const TONE: Record<VerifyState, string> = {
  verified: "bg-jolu-selected text-jolu-on",
  partial: "bg-jolu-pressed/60 text-jolu-text",
  unverified: "bg-jolu-side text-jolu-muted",
  none: "border border-dashed border-jolu-border text-jolu-muted",
  na: "text-jolu-muted/60",
}

/**
 * 平台能力验证（总清单 4.7；无画板，待确认）：每个平台每项能力是「已验证 / 部分验证 / 未验证 / 无记录 / 不适用」，
 * 点格子看依据日期、记录文件、原文依据和还缺的样本。数据来自 `lib/capabilityVerification.ts`。
 */
export function CapabilitySection() {
  const [picked, setPicked] = useState<VerifyCell | null>(null)
  const counts = stateCounts()
  return (
    <SettingsGroup icon={ShieldCheck} title="平台能力验证（待确认）" desc="只认真实账号或真机跑通的记录；合成测试、模拟上游、只跑到初始化成功都不算。点格子看依据。">
      <div className="flex flex-col gap-3 pb-4">
        <div className="flex flex-wrap gap-3 text-[12px] text-jolu-muted">
          {(["verified", "partial", "unverified", "none", "na"] as VerifyState[]).map((state) => (
            <span key={state} className="flex items-center gap-1.5">
              <span className={cn("inline-block h-4 rounded px-1.5 text-[11px] leading-4", TONE[state])}>{STATE_LABEL[state]}</span>
              {counts[state]}
            </span>
          ))}
        </div>
        <div className="overflow-x-auto rounded-lg border border-jolu-border">
          <table className="w-full min-w-[680px] border-collapse text-[12px]">
            <thead>
              <tr className="bg-jolu-side text-jolu-muted">
                <th className="px-3 py-2 text-left font-normal">平台</th>
                {CAPABILITIES.map((cap) => <th key={cap.id} className="px-2 py-2 text-left font-normal">{cap.name}</th>)}
              </tr>
            </thead>
            <tbody>
              {PLATFORMS.map((platform) => (
                <tr key={platform.id} className="border-t border-jolu-table-divider">
                  <td className="px-3 py-1.5 font-medium text-jolu-text">{platform.name}</td>
                  {CAPABILITIES.map((cap) => {
                    const cell = cellOf(platform.id, cap.id)
                    const active = picked?.platform === cell.platform && picked.capability === cell.capability
                    return (
                      <td key={cap.id} className="px-2 py-1.5">
                        <button type="button" onClick={() => setPicked(active ? null : cell)} title={cellSummary(cell)} aria-pressed={active}
                          className={cn("rounded px-1.5 py-0.5 text-[11px] outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent", TONE[cell.state], active && "ring-1 ring-jolu-accent")}>
                          {STATE_LABEL[cell.state]}
                        </button>
                      </td>
                    )
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {picked && (
          <div className="flex flex-col gap-1 rounded-lg bg-jolu-side px-4 py-3 text-[12px]" role="region" aria-label="验证依据">
            <span className="font-medium text-jolu-text">
              {PLATFORMS.find((p) => p.id === picked.platform)?.name} · {CAPABILITIES.find((c) => c.id === picked.capability)?.name} · {STATE_LABEL[picked.state]}
              {picked.date ? ` · ${picked.date}` : ""}
            </span>
            <span className="text-jolu-text">依据：{picked.evidence}</span>
            {picked.record && <span className="font-mono text-jolu-muted">{picked.record}</span>}
            {picked.missing && <span className="text-jolu-muted">还缺：{picked.missing}</span>}
          </div>
        )}
      </div>
    </SettingsGroup>
  )
}
