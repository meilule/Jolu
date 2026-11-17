import { useEffect, useState } from "react"
import { Server } from "lucide-react"
import { isTauri } from "@/lib/api"
import { settingsApi } from "@/lib/settingsApi"
import { cn } from "@/lib/utils"
import { useToast } from "@/components/ui/Toast"
import { WSL_AGENT_LABELS, netSummary, rowState, type WslWiringStateDto } from "@/lib/wslWiring"
import { ActionButton, Notice, SelectButton, SettingsGroup, SettingsRow } from "./settingsUi"

/**
 * 5.6 WSL 里的工具接线（后端 `gateway/wiring_wsl.rs`）。
 *
 * 待确认：`Jolu.pen` 没有这块的画板，按既有 `SettingsGroup` 布局挂在「数据与存储」页尾；
 * 只接 Claude Code / Codex / OpenCode。NAT 网络下只在用户自己开了局域网共享时接，Jolu 不替用户打开。
 */
const TONE = { ok: "text-jolu-green", warn: "text-jolu-amber", muted: "text-jolu-muted" } as const

export function WslWiringSection() {
  const toast = useToast()
  const [distros, setDistros] = useState<string[] | null>(null)
  const [probeError, setProbeError] = useState<string | null>(null)
  const [distro, setDistro] = useState("")
  const [state, setState] = useState<WslWiringStateDto | null>(null)
  const [loading, setLoading] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)

  useEffect(() => {
    if (!isTauri) return
    let alive = true
    void settingsApi.wslProbeAgents().then((probe) => {
      if (!alive) return
      if (!probe.available) { setProbeError(probe.reason ?? "WSL 不可用"); setDistros([]); return }
      // docker-desktop 一类不是给人用的发行版，仍列出来由用户选
      setDistros(probe.distros)
      setDistro(probe.default_distro ?? probe.distros[0] ?? "")
    }).catch((e) => { if (alive) { setProbeError(String(e)); setDistros([]) } })
    return () => { alive = false }
  }, [])

  const load = async (name: string) => {
    if (!name) return
    setLoading(true)
    try { setState(await settingsApi.wslWiringState(name)) } catch (e) { setState(null); setProbeError(String(e)) } finally { setLoading(false) }
  }
  useEffect(() => { void load(distro) }, [distro])

  const act = async (agent: string, wire: boolean) => {
    setBusy(agent)
    try {
      const message = wire ? await settingsApi.wslWire(distro, agent) : await settingsApi.wslUnwire(distro, agent)
      toast.success(message)
    } catch (e) {
      toast.danger(String(e))
    } finally {
      setBusy(null)
      await load(distro)
    }
  }

  const summary = state ? netSummary(state) : null
  return (
    <SettingsGroup icon={Server} title="WSL 里的工具（待确认）" desc="把 WSL 发行版里的 Claude Code、Codex、OpenCode 接到模型路由；写的是发行版里的配置文件，断开时按快照还原">
      {probeError && <div className="border-t border-jolu-border py-3"><Notice tone="danger">{probeError}</Notice></div>}
      {distros && distros.length > 0 && (
        <SettingsRow title="发行版" desc={summary ? <span className={TONE[summary.tone]}>{summary.text}</span> : loading ? "正在读取发行版的网络情况…" : undefined}
          control={<SelectButton label="发行版" value={distro} options={distros.map((d) => ({ value: d, label: d }))} onChange={setDistro} disabled={!isTauri || busy !== null} />} />
      )}
      {state && state.agents.map((agent) => {
        const row = state.rows.find((r) => r.agent === agent)
        const status = rowState(row)
        return (
          <SettingsRow key={agent} title={WSL_AGENT_LABELS[agent] ?? agent}
            desc={<span className={cn(TONE[status.tone])}>{status.text}{row ? ` · ${row.base}` : ""}</span>}
            control={status.wired
              ? <ActionButton busy={busy === agent} disabled={busy !== null} onClick={() => void act(agent, false)}>断开并还原</ActionButton>
              : <ActionButton tone="accent" busy={busy === agent} disabled={busy !== null || !summary?.canWire} onClick={() => void act(agent, true)}>接入</ActionButton>} />
        )
      })}
    </SettingsGroup>
  )
}
