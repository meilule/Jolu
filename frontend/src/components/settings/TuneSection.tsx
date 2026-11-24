import { useEffect, useState } from "react"
import { Gauge } from "lucide-react"
import { isTauri } from "@/lib/api"
import { settingsApi } from "@/lib/settingsApi"
import { cn } from "@/lib/utils"
import { agentName, compactionLine, ttlLine, type TuneAgentDto } from "@/lib/tuneAdvice"
import { ActionButton, Notice, SelectButton, SettingsGroup } from "./settingsUi"

/**
 * 5.10 成本调优建议（后端 `tune.rs`，移植 magpie tune）：只给建议，不改任何工具配置。
 * 待确认：`Jolu.pen` 没有这块的画板，按 `SettingsGroup` 挂「数据与存储」页尾；「一键应用」不做。
 */
const DAYS = [
  { value: "7", label: "最近 7 天" },
  { value: "30", label: "最近 30 天" },
  { value: "90", label: "最近 90 天" },
]

export function TuneSection() {
  const [days, setDays] = useState("30")
  const [advice, setAdvice] = useState<TuneAgentDto[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  const load = async (value: string) => {
    if (!isTauri) return
    setLoading(true)
    try { setAdvice(await settingsApi.tuneAdvice(Number(value))); setError(null) } catch (e) { setError(String(e)) } finally { setLoading(false) }
  }
  useEffect(() => { void load(days) }, [days])

  return (
    <SettingsGroup icon={Gauge} title="成本调优建议（待确认）" desc="按这台电脑最近的真实调用逐条回放，算出在多大上下文压缩、提示词缓存留多久最省加权 Token；只给建议，不改配置">
      <div className="flex items-center gap-2 border-t border-jn-line py-3">
        <SelectButton label="回放范围" value={days} options={DAYS} onChange={setDays} disabled={!isTauri || loading} />
        <ActionButton busy={loading} disabled={!isTauri || loading} onClick={() => void load(days)}>重新计算</ActionButton>
      </div>
      {error && <div className="pb-3"><Notice tone="danger">{error}</Notice></div>}
      {advice && advice.length === 0 && <span className="pb-3 text-xs text-jn-text-3">这段时间没有 Claude Code / Codex 的调用记录</span>}
      {advice?.map((agent) => (
        <div key={agent.platform} className="flex flex-col gap-2 border-t border-jn-line py-3">
          <span className="text-[13px] font-medium text-jn-text">{agentName(agent.platform)} · {agent.calls} 次调用</span>
          {agent.compactions.length === 0 && <span className="text-xs text-jn-text-3">压缩窗口：调用太少（每个模型至少 50 次）或从没接近窗口，暂不建议</span>}
          {[...agent.compactions.map((c) => compactionLine(c, agent.compact_setting)), ...(agent.cache ? [ttlLine(agent.cache)] : [])].map((line) => (
            <div key={line.title} className="flex flex-col gap-0.5">
              <span className={cn("text-xs", line.change ? "text-jn-text" : "text-jn-text-3")}>{line.title}</span>
              <span className="text-[11px] text-jn-text-3">{line.detail}</span>
            </div>
          ))}
        </div>
      ))}
      {advice && advice.length > 0 && <span className="pb-3 text-[11px] text-jn-text-3">结论只对你这台电脑这段时间的用法成立；改设置请在工具自己的配置里改（Claude Code 的 CLAUDE_CODE_AUTO_COMPACT_WINDOW、Codex 的 model_auto_compact_token_limit）</span>}
    </SettingsGroup>
  )
}
