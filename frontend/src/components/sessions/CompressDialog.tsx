import { SelectField } from "@/components/ui/SelectField"
import { useEffect, useRef, useState } from "react"
import { listen } from "@tauri-apps/api/event"
import { isTauri, type SessionItemDto } from "@/lib/api"
import { automationApi, type AutomationAccountDto } from "@/lib/automationApi"
import { handoffApi } from "@/lib/handoffApi"
import { dialogMode, handoffFileName, fileNameError, formatTokens, HANDOFF_TARGETS, reduceChat, type ChatEventDto, type ChatItem, type Draft, type HandoffResult, type HandoffState } from "@/lib/sessionHandoff"
import { projectName, sessionTitle } from "@/lib/sessionThreads"
import { HandoffShell, HandoffNotice, handoffInput } from "./handoffParts"
import { CompressChat } from "./CompressChat"
import { EVENTS, type HandoffChatEvent } from "@/lib/events"
import { Button, OptionTile } from "@/ui"

type Envelope = HandoffChatEvent

/** 压缩会话弹窗（R1-43 / R1-44）：配置压缩参数、选择生成方式（OptionTile）并在同壳内进入修订对话。 */
export function CompressDialog({ session, onClose, onResult }: {
  session: SessionItemDto
  onClose: () => void
  onResult: (result: HandoffResult, name: string, target: string) => void
}) {
  const [state, setState] = useState<HandoffState | null>(null)
  const [accounts, setAccounts] = useState<(AutomationAccountDto & { platform: string })[]>([])
  const [connection, setConnection] = useState("")
  const [model, setModel] = useState("")
  const [models, setModels] = useState<string[]>([])
  const [target, setTarget] = useState("Claude Code")
  const [focus, setFocus] = useState("")
  const [channel, setChannel] = useState<"isolated" | "fork">("isolated")
  const canFork = session.platform === "claude" || session.platform === "codex"
  const project = projectName(session.cwd)
  const [name, setName] = useState(() => handoffFileName(project, "", "Claude", new Date()))
  const manualName = useRef(false)
  const [error, setError] = useState("")
  const [chat, setChat] = useState(false)
  const [busy, setBusy] = useState(false)
  const [confirm, setConfirm] = useState(false)
  const [items, setItems] = useState<ChatItem[]>([])
  const [drafts, setDrafts] = useState<Draft[]>([])
  const [round, setRound] = useState(0)
  const [estimate, setEstimate] = useState<number | null>(null)
  const job = useRef<string | null>(null)
  const buffer = useRef<Envelope[]>([])
  const starting = useRef(false)
  const alive = useRef(true)
  const ready = useRef<Promise<() => void> | null>(null)
  const estimateRevision = useRef(0)
  const instruction = useRef("")

  const accept = (event: ChatEventDto) => {
    setItems((value) => reduceChat(value, event))
    if (event.type === "draft") {
      setDrafts((value) => [...value, event])
      setRound(event.round)
      setBusy(false)
      setEstimate(null)
    }
    if (event.type === "error") setBusy(false)
  }

  const reload = async () => {
    setError("")
    try {
      setState(await handoffApi.state(session.platform, session.session_id))
    } catch (e) {
      setError(String(e))
    }
  }

  useEffect(() => {
    alive.current = true
    if (isTauri) {
      void reload()
      void Promise.all((["claude", "codex"] as const).map(async (platform) => (await automationApi.accounts(platform)).map((a) => ({ ...a, platform }))))
        .then((rows) => {
          if (!alive.current) return
          const all = rows.flat()
          setAccounts(all)
          setConnection(all.find((a) => a.current && a.status === "connected")?.id ?? "")
        })
        .catch((e) => { if (alive.current) setError(String(e)) })
      ready.current = listen<Envelope>(EVENTS.HANDOFF_CHAT, ({ payload }) => {
        if (!alive.current) return
        if (payload.job_id === job.current) accept(payload.event)
        else if (starting.current) buffer.current.push(payload)
      })
    } else {
      setState({ readable: session.has_source, total: 24, latest: null, digest_covered_to: 0, digest_range: null, new_range: [1, 24], reusable: false, estimate_tokens: 12400 })
    }
    return () => {
      alive.current = false
      void ready.current?.then((stop) => stop()).catch(() => {})
      if (job.current) void handoffApi.cancel(job.current).catch(() => {})
    }
  }, [session.platform, session.session_id])

  useEffect(() => {
    if (!manualName.current) setName(handoffFileName(project, focus, target === "Claude Code" ? "Claude" : target, new Date()))
  }, [project, focus, target])

  const selected = accounts.find((a) => a.id === connection)
  useEffect(() => {
    let active = true
    setModel("")
    setModels([])
    if (selected && isTauri) {
      void automationApi.models(selected.platform as "claude" | "codex", selected.id)
        .then((values) => { if (active) setModels(values) })
        .catch((e) => { if (active) setError(String(e)) })
    }
    return () => { active = false }
  }, [connection])

  const mode = state ? dialogMode(state) : null
  const forkBlocked = channel === "fork" && !!selected && selected.platform !== session.platform
  const close = () => (chat ? setConfirm(true) : onClose())
  const cancel = async () => {
    try {
      if (job.current) await handoffApi.cancel(job.current)
      job.current = null
      onClose()
    } catch (e) {
      setError(String(e))
    }
  }

  const start = async () => {
    if (!state || !isTauri) return
    setBusy(true)
    setError("")
    try {
      if (mode === "reuse") {
        const markdown = await handoffApi.render(session.platform, session.session_id, project, focus, target)
        onResult({ version: state.latest!.version, markdown, file_name: name, path_warnings: state.latest!.path_warnings, covered_range: state.latest!.covered_range }, name, target)
        return
      }
      await ready.current
      if (job.current) await handoffApi.cancel(job.current)
      job.current = null
      buffer.current = []
      starting.current = true
      setItems([])
      setDrafts([])
      setRound(0)
      setChat(true)
      instruction.current = ""
      const id = await handoffApi.start(session.platform, session.session_id, connection, model || null, focus, target, channel)
      if (!alive.current) {
        await handoffApi.cancel(id)
        return
      }
      job.current = id
      starting.current = false
      for (const event of buffer.current) if (event.job_id === id) accept(event.event)
      buffer.current = []
    } catch (e) {
      starting.current = false
      setError(String(e))
      setBusy(false)
      setItems((value) => [...value, { kind: "error", text: String(e), retryable: true }])
    }
  }

  const localExport = async () => {
    if (!state || !isTauri) return
    setBusy(true)
    setError("")
    try {
      const markdown = await handoffApi.localExport(session.platform, session.session_id, project, focus, target)
      onResult({ version: 0, markdown, file_name: name, path_warnings: [], covered_range: state.total ? [1, state.total] : null }, name, target)
    } catch (e) {
      setError(String(e))
      setBusy(false)
    }
  }

  const refine = async (text: string) => {
    if (!job.current) return
    instruction.current = text
    setBusy(true)
    setItems((v) => [...v, { kind: "user", text }])
    try {
      await handoffApi.refine(job.current, text)
    } catch (e) {
      setBusy(false)
      setItems((v) => [...v, { kind: "error", text: String(e), retryable: true }])
    }
  }

  const finish = async () => {
    if (!job.current) return
    setBusy(true)
    try {
      const result = await handoffApi.finish(job.current, round, project)
      job.current = null
      onResult(result, name, target)
    } catch (e) {
      setError(String(e))
      setBusy(false)
    }
  }

  const estimateInput = (text: string) => {
    const revision = ++estimateRevision.current
    setEstimate(null)
    if (job.current && text.trim()) {
      void handoffApi.estimate(job.current, text).then((n) => {
        if (alive.current && revision === estimateRevision.current) setEstimate(n)
      }).catch(() => {})
    }
  }

  const platformToolName = session.platform === "claude" ? "Claude Code" : "Codex"

  return (
    <HandoffShell
      title={chat ? `压缩会话 · ${sessionTitle(session)}` : mode === "reuse" ? "会话已压缩，直接生成 MD" : `压缩会话 · ${sessionTitle(session)}`}
      onClose={close}
      chat={chat}
    >
      {confirm ? (
        <>
          <HandoffNotice>关闭压缩会话？关闭将丢弃未生成的草稿，已整理的纪要会保留。</HandoffNotice>
          <div className="flex justify-end gap-2">
            <Button kind="ghost" size="sm" onClick={() => setConfirm(false)}>继续压缩</Button>
            <Button kind="danger" size="sm" onClick={() => void cancel()}>丢弃并关闭</Button>
          </div>
        </>
      ) : chat ? (
        <>
          <p className="text-xs text-jn-text-3">{project} · 共 {state?.total ?? "—"} 条 · 执行：{selected?.name ?? "—"} · {model || "默认模型"}</p>
          <CompressChat
            items={items}
            busy={busy}
            drafts={drafts}
            round={round}
            onRound={setRound}
            onSend={(text) => void refine(text)}
            onFinish={() => void finish()}
            onCancel={close}
            onRetry={() => void (instruction.current && job.current ? refine(instruction.current) : start())}
            onChangeConnection={() => { setChat(false); setError("") }}
            estimate={estimate}
            onInstruction={estimateInput}
          />
        </>
      ) : (
        <>
          <div className="rounded-[14px] bg-jn-well p-3.5 text-xs text-jn-text">
            <p className="font-semibold">{sessionTitle(session)}</p>
            <p className="mt-1 font-mono text-[11.5px] text-jn-text-3">{project} · 共 {state?.total ?? "—"} 条消息 · {session.session_id}</p>
          </div>

          {!state ? (
            <p className="text-xs text-jn-text-3">正在读取…</p>
          ) : (
            <HandoffNotice>
              <span className="font-semibold">
                {mode === "reuse" ? "没有新增消息 · 无需再次调用 AI" : mode === "unreadable" ? "仅用量记录 / 源文件不可读" : mode === "first" ? "首次整理" : "本次增量压缩"}
              </span>
              <p className="mt-0.5">
                {mode === "unreadable"
                  ? "无法仅凭 Token 数与会话标题生成可靠的开发交接。"
                  : mode === "reuse"
                    ? "本次只调整文档包装，主体事实和任务进度沿用有效摘要。"
                    : `本次整理第 ${state.new_range?.join("–") ?? `1–${state.total}`} 条消息，已有纪要覆盖 ${state.digest_covered_to} 条。`}
              </p>
            </HandoffNotice>
          )}

          {mode !== "reuse" && mode !== "unreadable" && (
            <>
              <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2">
                <label className="flex flex-col gap-1.5 text-xs font-medium text-jn-text-2">
                  执行压缩的 AI
                  <SelectField
                    width="full"
                    label="执行压缩的 AI"
                    className="h-10"
                    value={connection}
                    onValueChange={setConnection}
                    options={[
                      { value: "", label: "选择可用连接" },
                      ...accounts.map((a) => ({
                        value: a.id,
                        label: a.name,
                        disabled: !a.current || a.status !== "connected",
                        description: !a.current ? "需为 CLI 当前登录账号" : a.status !== "connected" ? "连接不可用" : undefined,
                      })),
                    ]}
                  />
                </label>
                <label className="flex flex-col gap-1.5 text-xs font-medium text-jn-text-2">
                  压缩模型
                  <SelectField
                    width="full"
                    label="压缩模型"
                    className="h-10"
                    value={model}
                    onValueChange={setModel}
                    options={[{ value: "", label: "默认模型" }, ...models.map((id) => ({ value: id, label: id }))]}
                  />
                </label>
              </div>
              <p className="text-xs leading-5 text-jn-text-3">
                预计发送约 {formatTokens(state?.estimate_tokens ?? 0)} token · 将会话内容发送给所选 AI。没有可用连接时暂不能压缩新增消息，保留上次摘要。
              </p>
            </>
          )}

          {canFork && (
            <div className="flex flex-col gap-1.5">
              <span className="text-xs font-medium text-jn-text-2">生成方式</span>
              <div role="radiogroup" aria-label="生成方式" className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                <OptionTile
                  active={channel === "isolated"}
                  icon="layers"
                  label="Jolu 整理"
                  description="所选 AI 读整理后的会话"
                  onClick={() => setChannel("isolated")}
                />
                <OptionTile
                  active={channel === "fork"}
                  icon="gitBranch"
                  label="在原工具里分叉"
                  description={`${platformToolName} 读完整原会话`}
                  onClick={() => setChannel("fork")}
                />
              </div>
              {channel === "fork" && (
                <p className={forkBlocked ? "text-xs text-jn-danger" : "text-xs text-jn-text-3"}>
                  {forkBlocked
                    ? `在原工具里分叉只能用 ${platformToolName} 的账号，请在上面换一个账号。`
                    : "续接原会话的分叉副本，不改原会话、也不留下新会话；会读完整原会话，消耗原工具的额度。"}
                </p>
              )}
            </div>
          )}

          <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2">
            <label className="flex flex-col gap-1.5 text-xs font-medium text-jn-text-2">
              交接给
              <SelectField width="full" label="交接给" className="h-10" value={target} onValueChange={setTarget} options={HANDOFF_TARGETS.map((t) => ({ value: t, label: t }))} />
            </label>
            <label className="flex flex-col gap-1.5 text-xs font-medium text-jn-text-2">
              交接主题
              <input className={handoffInput} value={focus} onChange={(e) => setFocus(e.target.value)} placeholder="接手后要做什么（可留空）" />
            </label>
          </div>

          <label className="flex flex-col gap-1.5 text-xs font-medium text-jn-text-2">
            文件名（可修改）
            <input className={handoffInput} value={name} onChange={(e) => { manualName.current = true; setName(e.target.value) }} />
          </label>
          {fileNameError(name) && <p className="text-xs text-jn-danger">{fileNameError(name)}</p>}

          <div className="rounded-[14px] border border-jn-line bg-jn-well/50 p-3.5 text-xs leading-6 text-jn-text-3">
            <p className="font-semibold text-jn-text">这份 MD 会包含</p>
            项目目标与约束 · 当前开发进度 · 已完成改动<br />
            关键文件与决策 · 验证结果 · 待办与下一步执行建议
          </div>
          <p className="text-xs text-jn-text-3">原会话保持完整 · 过时结论以新消息为准</p>
          {!isTauri && <HandoffNotice>浏览器预览不调用模型；请在桌面应用中执行压缩。</HandoffNotice>}

          <div className="flex flex-wrap items-center justify-end gap-2 pt-1">
            {(mode === "unreadable" || !!error) && <Button kind="ghost" size="sm" icon="retry" onClick={() => void reload()}>重新读取会话</Button>}
            <Button kind="ghost" size="md" onClick={onClose}>取消</Button>
            {mode !== "unreadable" && mode !== "reuse" && (
              <Button
                kind="soft"
                size="md"
                disabled={busy || !isTauri || !state || !!fileNameError(name)}
                title="不调用任何模型，直接摘出原文与结构化事实；不是模型压缩"
                onClick={() => void localExport()}
              >
                本地交接导出（不调用模型）
              </Button>
            )}
            <Button
              kind="primary"
              size="md"
              icon="compress"
              loading={busy}
              disabled={busy || !isTauri || !state || mode === "unreadable" || !!fileNameError(name) || (mode !== "reuse" && (!selected?.current || selected.status !== "connected" || forkBlocked))}
              onClick={() => void start()}
            >
              {mode === "reuse" ? "直接生成 MD" : "压缩并生成 MD"}
            </Button>
          </div>
        </>
      )}
      {error && <HandoffNotice danger>{error}</HandoffNotice>}
    </HandoffShell>
  )
}
