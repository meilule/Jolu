import { useEffect, useRef, useState } from "react"
import { type ChatItem, type Draft, formatTokens } from "@/lib/sessionHandoff"
import { MarkdownPreview } from "@/components/skills/skillParts"
import { HandoffNotice, handoffInput } from "./handoffParts"
import { cn } from "@/lib/utils"
import { Collapse } from "@/motion"
import { Icon } from "@/icons"
import { Button, StatusPill } from "@/ui"

/** 压缩修订对话（R1-44）：草稿版本卡片、逐轮修订输入与选定版本生成 MD。 */
export function CompressChat({
  items,
  busy,
  drafts,
  round,
  onRound,
  onSend,
  onFinish,
  onCancel,
  onRetry,
  onChangeConnection,
  estimate,
  onInstruction,
}: {
  items: ChatItem[]
  busy: boolean
  drafts: Draft[]
  round: number
  onRound: (round: number) => void
  onSend: (text: string) => void
  onFinish: () => void
  onCancel: () => void
  onRetry: () => void
  onChangeConnection: () => void
  estimate: number | null
  onInstruction: (text: string) => void
}) {
  const [text, setText] = useState("")
  const [expandedRounds, setExpandedRounds] = useState<Set<number>>(new Set())
  const scroll = useRef<HTMLDivElement>(null)
  const pinned = useRef(true)

  useEffect(() => {
    if (pinned.current && scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight
  }, [items])

  const last = drafts.at(-1)
  const limited = (last?.round ?? 0) >= 11

  const toggleExpand = (r: number) => {
    setExpandedRounds((prev) => {
      const next = new Set(prev)
      if (next.has(r)) next.delete(r)
      else next.add(r)
      return next
    })
  }

  return (
    <>
      <div
        ref={scroll}
        onScroll={() => {
          const el = scroll.current
          if (el) pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60
        }}
        className="min-h-0 flex-1 space-y-3 overflow-y-auto border-y border-jn-line py-4 pr-1"
        aria-live="polite"
      >
        {items.map((item, i) => {
          if (item.kind === "draft") {
            const r = item.round ?? i + 1
            const selected = round === r
            const isLatest = r === last?.round
            const expanded = expandedRounds.has(r)
            return (
              <div key={i} className={cn("flex flex-col gap-2.5 rounded-[16px] border p-3.5", selected ? "border-jn-primary bg-jn-primary-wash/40" : "border-jn-line bg-jn-well")}>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-[13px] font-semibold text-jn-text">交接草稿 v{r}</span>
                  {isLatest && <StatusPill tone="success">最新</StatusPill>}
                  <span className="text-xs text-jn-text-3">约 {item.text.length.toLocaleString("zh-CN")} 字</span>
                  <span className="flex-1" />
                  <Button kind="ghost" size="sm" icon={expanded ? "collapse" : "expand"} onClick={() => toggleExpand(r)}>
                    {expanded ? "收起全文" : "展开全文"}
                  </Button>
                  <Button
                    kind={selected ? "primary" : "soft"}
                    size="sm"
                    icon={selected ? "check" : undefined}
                    disabled={busy}
                    onClick={() => onRound(r)}
                  >
                    {selected ? `已选 v${r}` : `使用 v${r}`}
                  </Button>
                </div>
                {!expanded && (
                  <p className="line-clamp-3 whitespace-pre-wrap text-xs leading-relaxed text-jn-text-2">
                    {item.text.split(/(?=^## )/m).slice(0, 3).join("").slice(0, 320)}
                  </p>
                )}
                <Collapse open={expanded}>
                  <div className="rounded-[12px] bg-jn-surface p-3.5">
                    <MarkdownPreview source={item.text} stripFront={false} />
                  </div>
                </Collapse>
                {!!item.warnings?.length && (
                  <HandoffNotice danger>以下路径未在会话中出现：{item.warnings.join("、")}</HandoffNotice>
                )}
              </div>
            )
          }

          return (
            <div key={i} className={cn("flex gap-3 text-xs", item.kind === "user" && "rounded-[14px] bg-jn-well p-3")}>
              <span className="w-8 shrink-0 font-semibold text-jn-text-3">{item.kind === "user" ? "你" : "Jolu"}</span>
              <div className="min-w-0 flex-1">
                <p className={item.kind === "error" ? "text-jn-danger" : "whitespace-pre-wrap leading-6 text-jn-text"}>
                  {item.text}{item.total ? `（${item.done}/${item.total}）` : ""}
                </p>
                {item.kind === "error" && !busy && (
                  <div className="mt-2 flex gap-2">
                    {item.retryable && <Button kind="soft" size="sm" icon="retry" onClick={onRetry}>重试</Button>}
                    <Button kind="ghost" size="sm" onClick={onChangeConnection}>更换连接</Button>
                  </div>
                )}
              </div>
            </div>
          )
        })}
        {busy && (
          <div className="flex items-center gap-2 text-xs text-jn-text-3">
            <Icon name="loading" size="sm" className="animate-spin text-jn-primary" />
            正在整理交接草稿…
          </div>
        )}
      </div>

      <form
        className="flex items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          if (text.trim() && !busy && !limited) {
            onSend(text.trim())
            setText("")
          }
        }}
      >
        <textarea
          aria-label="修改意见"
          className={cn(handoffInput, "min-h-[56px] resize-none")}
          rows={2}
          disabled={busy || limited || !last}
          placeholder={busy ? "生成中，完成后可继续提意见" : limited ? "已达到修改上限" : "对草稿提修改意见…"}
          value={text}
          onChange={(e) => { setText(e.target.value); onInstruction(e.target.value) }}
        />
        <Button kind="primary" size="md" icon="arrowRight" disabled={busy || limited || !last || !text.trim()} type="submit">
          发送
        </Button>
      </form>

      <p className="tnum text-xs text-jn-text-3">
        本轮预计发送约 {estimate === null ? "—" : formatTokens(estimate)} token · 已用 输入 {last?.input_tokens == null ? "—" : formatTokens(last.input_tokens)} / 输出 {last?.output_tokens == null ? "—" : formatTokens(last.output_tokens)} token
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <p className="min-w-0 flex-1 text-[11px] text-jn-text-3">取消保留旧摘要与原会话；已发出的模型请求可能产生用量。</p>
        <Button kind="ghost" size="md" onClick={onCancel}>取消压缩</Button>
        <Button kind="primary" size="md" icon="fileText" loading={busy} disabled={busy || !round} onClick={onFinish}>
          用 v{round || 1} 生成 MD
        </Button>
      </div>
    </>
  )
}
