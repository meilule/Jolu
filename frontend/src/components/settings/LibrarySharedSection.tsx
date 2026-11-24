import { useEffect, useState } from "react"
import { Bot, ScrollText } from "lucide-react"
import { isTauri, listenEvent } from "@/lib/api"
import { EVENTS } from "@/lib/events"
import { settingsApi } from "@/lib/settingsApi"
import { cn } from "@/lib/utils"
import { useToast } from "@/components/ui/Toast"
import {
  outcomeSummary,
  sharedAgentLabel,
  subagentNameError,
  targetText,
  type FileOutcomeDto,
  type LibrarySharedStateDto,
  type SubagentDto,
  type TargetStatusDto,
} from "@/lib/librarySharedView"
import { ActionButton, Notice, SettingsGroup } from "./settingsUi"

/**
 * Library 5.1 第二批：共享指令与子代理（后端 `library/shared.rs`）。
 *
 * 待确认：`Jolu.pen` 没有这块的画板，按既有 `SettingsGroup` 布局挂在「数据与存储」页尾
 * （与运维项同处：都是 Jolu 写进本机各工具目录的东西），画板补齐后再定稿。
 */
const FIELD = "min-w-0 rounded-lg border border-jn-line bg-transparent px-2.5 text-xs text-jn-text outline-none placeholder:text-jn-text-3 focus:border-jn-primary disabled:opacity-45"
const TONE = { ok: "text-jn-primary", warn: "text-jn-warning", muted: "text-jn-text-3" } as const

function AgentTicks({ agents, picked, targets, disabled, onChange }: {
  agents: string[]
  picked: string[]
  targets?: TargetStatusDto[]
  disabled: boolean
  onChange: (next: string[]) => void
}) {
  return (
    <div className="flex flex-col gap-1.5">
      {agents.map((agent) => {
        const on = picked.includes(agent)
        const target = targets?.find((t) => t.agent === agent)
        const status = target ? targetText(target) : null
        return (
          <label key={agent} className="flex items-center gap-2 text-xs text-jn-text">
            <input type="checkbox" checked={on} disabled={disabled} className="size-3.5 accent-[var(--color-jn-primary)]"
              onChange={() => onChange(on ? picked.filter((a) => a !== agent) : [...picked, agent])} />
            <span className="w-[92px] shrink-0">{sharedAgentLabel(agent)}</span>
            {status && <span className={cn("shrink-0", TONE[status.tone])}>{status.text}</span>}
            {target?.path && <span className="min-w-0 truncate font-mono text-[11px] text-jn-text-3" title={target.path}>{target.path}</span>}
          </label>
        )
      })}
    </div>
  )
}

const EMPTY_SUBAGENT: SubagentDto = { name: "", description: "", prompt: "", model: null, agents: ["claude-code"] }

export function LibrarySharedSection() {
  const toast = useToast()
  const [state, setState] = useState<LibrarySharedStateDto | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [text, setText] = useState("")
  const [agents, setAgents] = useState<string[]>([])
  const [draft, setDraft] = useState<SubagentDto | null>(null)
  const [last, setLast] = useState<{ text: string; warn: boolean } | null>(null)
  const disabled = !isTauri || busy !== null

  const load = async () => {
    try {
      const next = await settingsApi.librarySharedState()
      setState(next)
      setText(next.instructions.text)
      setAgents(next.instructions.agents)
      setError(null)
    } catch (e) {
      setError(String(e))
    }
  }
  useEffect(() => {
    if (!isTauri) return
    void load()
    const off = listenEvent(EVENTS.LIBRARY_CHANGED, () => { void settingsApi.librarySharedState().then(setState).catch(() => {}) })
    return () => { void off.then((un) => un()).catch(() => {}) }
  }, [])

  const run = async (key: string, work: () => Promise<FileOutcomeDto[]>) => {
    setBusy(key)
    try {
      const summary = outcomeSummary(await work())
      setLast(summary)
      toast.show({ tone: summary.warn ? "warn" : "success", title: summary.text })
      await load()
    } catch (e) {
      toast.danger(String(e))
    } finally {
      setBusy(null)
    }
  }

  const draftError = draft ? (subagentNameError(draft.name) ?? (!draft.description.trim() ? "要写描述" : !draft.prompt.trim() ? "要写提示词" : null)) : null
  const editingExisting = draft != null && state?.subagents.some((s) => s.name === draft.name)

  return (
    <>
      <SettingsGroup icon={ScrollText} title="共享指令（待确认）" desc="一段指令写进勾选工具的用户级指令文件（CLAUDE.md / AGENTS.md / GEMINI.md / QWEN.md），放在 Jolu 标记的区块里；区块外是你自己的内容，不会改动">
        <div className="flex flex-col gap-3 border-t border-jn-line py-3">
          {error && <Notice tone="danger">{error}</Notice>}
          <textarea aria-label="共享指令" rows={6} value={text} disabled={disabled} spellCheck={false}
            placeholder="例如：回答用中文；改代码后列出改动总结"
            onChange={(event) => setText(event.target.value)} className={cn(FIELD, "w-full resize-y py-2 font-mono")} />
          {state && <AgentTicks agents={state.instruction_targets.map((t) => t.agent)} picked={agents} targets={state.instruction_targets} disabled={disabled} onChange={setAgents} />}
          <div className="flex items-center gap-2">
            <ActionButton tone="accent" busy={busy === "instructions"} disabled={disabled || !state}
              onClick={() => void run("instructions", () => settingsApi.libraryInstructionsSave(text, agents))}>保存并写入</ActionButton>
            <span className="text-[11px] text-jn-text-3">没勾选但写过的工具会移除区块；区块里被你改过的不会覆盖</span>
          </div>
        </div>
      </SettingsGroup>

      <SettingsGroup icon={Bot} title="子代理（待确认）" desc="一份定义按各工具自己的格式写成文件（Claude Code / Gemini / Qwen / Droid / OpenCode 的 md，Codex 的 role toml）；只有 Codex 认模型">
        <div className="flex flex-col gap-2 border-t border-jn-line py-3">
          {state?.subagents.length === 0 && !draft && <span className="text-xs text-jn-text-3">还没有子代理</span>}
          {state?.subagents.map((sub) => (
            <div key={sub.name} className="flex items-center gap-3 rounded-lg border border-jn-line px-3 py-2">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="font-mono text-xs text-jn-text">{sub.name}</span>
                <span className="truncate text-[11px] text-jn-text-3">{sub.description}</span>
                <span className="text-[11px] text-jn-text-3">
                  {sub.targets.filter((t) => t.written).map((t) => `${sharedAgentLabel(t.agent)}${t.changed ? "（被改动）" : ""}`).join("、") || "未写入任何工具"}
                </span>
              </div>
              <ActionButton disabled={disabled} onClick={() => setDraft({ name: sub.name, description: sub.description, prompt: sub.prompt, model: sub.model, agents: sub.agents })}>编辑</ActionButton>
              <ActionButton tone="danger" busy={busy === `remove:${sub.name}`} disabled={disabled}
                onClick={() => void run(`remove:${sub.name}`, () => settingsApi.librarySubagentRemove(sub.name))}>删除</ActionButton>
            </div>
          ))}
          {draft && state ? (
            <div className="flex flex-col gap-2 rounded-lg border border-jn-line p-3">
              <input aria-label="子代理名称" value={draft.name} disabled={disabled || editingExisting} placeholder="名称，如 code-reviewer" spellCheck={false}
                onChange={(event) => setDraft({ ...draft, name: event.target.value.trim() })} className={cn(FIELD, "h-9 font-mono")} />
              <input aria-label="子代理描述" value={draft.description} disabled={disabled} placeholder="描述：主代理靠它判断什么时候调用"
                onChange={(event) => setDraft({ ...draft, description: event.target.value })} className={cn(FIELD, "h-9")} />
              <textarea aria-label="子代理提示词" rows={5} value={draft.prompt} disabled={disabled} placeholder="提示词" spellCheck={false}
                onChange={(event) => setDraft({ ...draft, prompt: event.target.value })} className={cn(FIELD, "w-full resize-y py-2 font-mono")} />
              <input aria-label="子代理模型" value={draft.model ?? ""} disabled={disabled} placeholder="模型（可选，只写进 Codex 的 role 文件）" spellCheck={false}
                onChange={(event) => setDraft({ ...draft, model: event.target.value.trim() || null })} className={cn(FIELD, "h-9 font-mono")} />
              <AgentTicks agents={state.subagent_agents} picked={draft.agents} targets={state.subagents.find((s) => s.name === draft.name)?.targets} disabled={disabled}
                onChange={(next) => setDraft({ ...draft, agents: next })} />
              {draftError && <span className="text-[11px] text-jn-warning">{draftError}</span>}
              <div className="flex items-center gap-2">
                <ActionButton tone="accent" busy={busy === "subagent"} disabled={disabled || draftError != null}
                  onClick={() => void run("subagent", async () => { const out = await settingsApi.librarySubagentSave(draft); setDraft(null); return out })}>保存并写入</ActionButton>
                <ActionButton disabled={disabled} onClick={() => setDraft(null)}>取消</ActionButton>
              </div>
            </div>
          ) : (
            <div><ActionButton disabled={disabled || !state} onClick={() => setDraft({ ...EMPTY_SUBAGENT })}>新建子代理</ActionButton></div>
          )}
          {last && <span className={cn("text-[11px]", last.warn ? "text-jn-warning" : "text-jn-text-3")}>上次保存：{last.text}</span>}
        </div>
      </SettingsGroup>
    </>
  )
}
