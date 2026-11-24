import { useState } from "react"
import { HANDOFF_PROMPT, fileNameError, type HandoffResult, type HandoffState } from "@/lib/sessionHandoff"
import { handoffApi } from "@/lib/handoffApi"
import { isTauri, type SessionItemDto } from "@/lib/api"
import { MarkdownPreview } from "@/components/skills/skillParts"
import { useToast } from "@/components/ui/Toast"
import { HandoffNotice, handoffInput } from "./handoffParts"
import { Banner, Button, Checkbox, Dialog, Tabs } from "@/ui"

/** 交接文档预览（R1-45）：统一居中弹窗，左侧 Markdown 预览 / 源码，右侧本次压缩记录与交接指令。 */
export function HandoffPreview({
  result,
  fileName,
  target,
  session,
  state,
  onClose,
  onUpdate,
}: {
  result: HandoffResult
  fileName: string
  target: string
  session: SessionItemDto
  state: HandoffState | null
  onClose: () => void
  onUpdate: () => void
}) {
  const [markdown, setMarkdown] = useState(result.markdown)
  const [name, setName] = useState(fileName)
  const [tab, setTab] = useState<"preview" | "source">("preview")
  const [projectDir, setProjectDir] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const toast = useToast()

  const copy = (text: string) =>
    void navigator.clipboard.writeText(text).then(() => toast.success("已复制"), () => toast.danger("复制失败"))

  const save = async () => {
    setBusy(true)
    setError("")
    try {
      if (!isTauri) throw new Error("浏览器预览不保存文件")
      const path = await handoffApi.save(markdown, name, projectDir ? session.cwd : null)
      if (path) toast.success("已保存", path)
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  const extra = state ? Math.max(0, state.total - (result.covered_range?.[1] ?? 0)) : 0

  return (
    <Dialog
      open
      onOpenChange={(next) => { if (!next && !busy) onClose() }}
      preventClose={busy}
      width="xl"
      title="交接文档已生成"
      description={`${result.version === 0 ? "已导出本地交接文档（未调用模型）" : "已生成完整交接文档"} · 覆盖第 ${result.covered_range?.join("–") ?? "—"} 条。可交给新的 AI 接着开发。`}
      footerStart={
        <div className="flex items-center gap-2">
          <Checkbox checked={projectDir} disabled={!session.cwd || busy} label="保存到项目目录" onChange={setProjectDir} />
          <button
            type="button"
            disabled={!session.cwd || busy}
            onClick={() => setProjectDir((v) => !v)}
            className="text-xs text-jn-text-2 disabled:opacity-45"
          >
            保存到项目目录
          </button>
        </div>
      }
      actions={
        <>
          <Button kind="ghost" size="md" disabled={busy} onClick={onClose}>关闭</Button>
          <Button kind="soft" size="md" icon="copy" onClick={() => copy(markdown)}>复制 Markdown</Button>
          <Button kind="primary" size="md" icon="download" loading={busy} disabled={busy || !!fileNameError(name)} onClick={() => void save()}>
            {busy ? "正在保存…" : "保存 MD"}
          </Button>
        </>
      }
    >
      {extra > 0 && (
        <Banner
          tone="warning"
          action={<Button kind="soft" size="sm" onClick={onUpdate}>更新压缩</Button>}
        >
          生成期间又新增 {extra} 条消息，未包含在本次文档中。
        </Banner>
      )}
      {!!result.path_warnings.length && (
        <HandoffNotice danger>以下路径未在会话中出现：{result.path_warnings.join("、")}</HandoffNotice>
      )}

      <div className="flex flex-wrap items-end gap-3.5">
        <label className="flex min-w-[240px] flex-1 flex-col gap-1.5 text-xs font-medium text-jn-text-2">
          文件名
          <input className={handoffInput} value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <div className="flex flex-col gap-1.5 rounded-[14px] bg-jn-well px-3.5 py-2 text-xs">
          <span className="text-[11px] text-jn-text-3">交接对象</span>
          <span className="font-semibold text-jn-text">{target}</span>
        </div>
      </div>
      {fileNameError(name) && <p className="text-xs text-jn-danger">{fileNameError(name)}</p>}

      <div className="grid min-h-[340px] flex-1 grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_248px]">
        <div className="flex min-h-0 flex-col rounded-[18px] border border-jn-line bg-jn-surface">
          <div className="flex items-center justify-between border-b border-jn-line px-3.5 py-2.5">
            <Tabs
              style="segment"
              value={tab}
              onChange={setTab}
              items={[
                { key: "preview", label: "预览" },
                { key: "source", label: "Markdown 源码" },
              ]}
            />
          </div>
          <div className="min-h-0 max-h-[380px] flex-1 overflow-auto p-4">
            {tab === "source" ? (
              <textarea
                aria-label="Markdown 源码"
                value={markdown}
                onChange={(e) => setMarkdown(e.target.value)}
                className="h-full min-h-[280px] w-full resize-none rounded-[12px] bg-jn-well p-3 font-mono text-xs leading-5 text-jn-text outline-none focus:ring-2 focus:ring-jn-primary/30"
              />
            ) : (
              <MarkdownPreview source={markdown} stripFront={false} />
            )}
          </div>
        </div>

        <aside className="flex flex-col gap-3 text-xs">
          <div className="flex flex-col gap-1.5 rounded-[16px] bg-jn-well p-4 text-jn-text-2">
            {result.version === 0 ? (
              <>
                <h3 className="mb-1 text-[13px] font-semibold text-jn-text">本地交接导出</h3>
                <p>未调用模型：原文摘录 + 结构化事实，未经归纳</p>
                <p>当前覆盖：第 {result.covered_range?.join("–") ?? "—"} 条</p>
                <p className="text-[11px] text-jn-text-3">不写交接版本，不推进压缩进度</p>
              </>
            ) : (
              <>
                <h3 className="mb-1 text-[13px] font-semibold text-jn-text">本次压缩记录</h3>
                <MetaRow label="摘要版本" value={`第 ${result.version} 版`} />
                <MetaRow label="当前覆盖" value={`第 ${result.covered_range?.join("–") ?? "—"} 条`} />
                <MetaRow label="执行 AI" value={state?.latest?.executor_platform ?? "—"} />
                <MetaRow label="模型" value={state?.latest?.model ?? "默认模型"} />
                <MetaRow label="输入 / 输出" value={`${state?.latest?.input_tokens ?? "—"} / ${state?.latest?.output_tokens ?? "—"}`} />
                <MetaRow label="生成轮次" value={String(state?.latest?.draft_rounds ?? "—")} />
                <p className="pt-1 text-[11px] text-jn-text-3">模型用量不并入用量统计</p>
              </>
            )}
          </div>

          <div className="flex flex-col gap-2.5 rounded-[16px] bg-jn-well p-4 text-jn-text-2">
            <h3 className="text-[13px] font-semibold text-jn-text">交给另一个 AI</h3>
            <p className="leading-relaxed text-jn-text-3">{HANDOFF_PROMPT}</p>
            <Button kind="soft" size="sm" icon="copy" className="self-start" onClick={() => copy(HANDOFF_PROMPT)}>
              复制交接指令
            </Button>
          </div>
        </aside>
      </div>

      {error && <HandoffNotice danger>{error}</HandoffNotice>}
    </Dialog>
  )
}

function MetaRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <span className="shrink-0 text-jn-text-3">{label}</span>
      <span className="tnum truncate font-mono font-medium text-jn-text">{value}</span>
    </div>
  )
}
