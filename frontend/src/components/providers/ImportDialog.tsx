import { useCallback, useEffect, useRef, useState } from "react"
import * as Dialog from "@radix-ui/react-dialog"
import { AlertTriangle, LoaderCircle, X } from "lucide-react"
import { api } from "@/lib/api"
import { cn } from "@/lib/utils"
import { useDialogClose } from "@/hooks/useDialogClose"
import { JoluButton } from "@/components/ui/JoluButton"
import { useToast } from "@/components/ui/Toast"
import {
  SOURCE_PATHS, groupImportDrafts, importToastDetail, importToastTitle, protocolLabel,
  selectableReferences, toggleReference, type ImportGroup,
} from "./importDrafts"

/**
 * 从本机导入（P4.3）：把 CC Switch / Claude Code / Codex 里已有的上游读成草稿，
 * 勾选后由后端重读原文创建自定义连接；键不出后端，草稿里只显示掩码，
 * 成功后后端广播 connections-changed，连接列表由既有监听自动刷新。
 * 待确认：Jolu.pen 模块 B 未画此弹窗，外壳与行布局按既有连接弹窗的常规做法实现。
 */
export function ImportDialog({ onClose: onDismiss }: { onClose: () => void }) {
  const { open, close: onClose } = useDialogClose(onDismiss)
  const toast = useToast()
  const [phase, setPhase] = useState<"scanning" | "ready" | "error">("scanning")
  const [scanError, setScanError] = useState<string | null>(null)
  const [groups, setGroups] = useState<readonly ImportGroup[]>([])
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set())
  const [applying, setApplying] = useState(false)
  // 每次扫描一个序号；重新扫描后旧结果到达直接丢弃
  const scan = useRef(0)

  const load = useCallback(() => {
    const id = ++scan.current
    setPhase("scanning")
    setScanError(null)
    api.providersImportScan()
      .then((result) => {
        if (scan.current !== id) return
        const next = groupImportDrafts(result.drafts)
        setGroups(next)
        // 默认全选草稿；单来源失败的错误条目不可勾
        setSelected(new Set(selectableReferences(next)))
        setPhase("ready")
      })
      .catch((reason) => {
        if (scan.current !== id) return
        setScanError(String(reason))
        setPhase("error")
      })
  }, [])
  useEffect(() => { load() }, [load])

  const total = selectableReferences(groups).length
  const apply = () => {
    if (applying || selected.size === 0) return
    setApplying(true)
    api.providersImportApply([...selected])
      .then((created) => {
        onClose()
        toast.success(importToastTitle(created), importToastDetail(created))
      })
      .catch((reason) => toast.danger("导入未完成", String(reason)))
      .finally(() => setApplying(false))
  }

  return (
    <Dialog.Root open={open} onOpenChange={(next) => { if (next || applying) return; onClose() }}>
      <Dialog.Portal>
        <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
        <Dialog.Content
          aria-describedby={undefined}
          className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] w-[640px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-2xl border border-jolu-border bg-jolu-cc-surface p-7 shadow-dialog outline-none"
        >
          <div className="flex items-center justify-between">
            <Dialog.Title className="text-xl font-semibold text-jolu-text">从本机导入</Dialog.Title>
            <Dialog.Close asChild>
              <button
                type="button"
                aria-label="关闭"
                disabled={applying}
                className="grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent"
              >
                <X aria-hidden className="size-5" />
              </button>
            </Dialog.Close>
          </div>
          <p className="-mt-1 text-xs leading-[1.6] text-jolu-muted">
            把 CC Switch、Claude Code、Codex 里已有的上游读成草稿，勾选后导入为自定义连接；
            密钥只显示掩码，导入由后端重读原文，不改写来源文件。
          </p>

          <div className="-mx-1 flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto border-0 px-1 py-0">
            {phase === "scanning" && (
              <div role="status" className="flex flex-1 items-center justify-center gap-2 py-12 text-xs text-jolu-muted">
                <LoaderCircle aria-hidden className="size-4 animate-spin" />
                正在扫描本机配置…
              </div>
            )}
            {phase === "error" && (
              <div role="alert" className="flex items-start gap-2.5 rounded-[9px] bg-jolu-danger-soft p-3.5 text-[11px] text-jolu-danger">
                <AlertTriangle aria-hidden className="mt-px size-4 shrink-0" />
                <span className="min-w-0 flex-1 leading-[1.6]">扫描失败：{scanError}</span>
                <button
                  type="button"
                  onClick={load}
                  className="shrink-0 rounded-md px-2 py-1 font-medium outline-none hover:bg-jolu-danger-pressed focus-visible:outline-2 focus-visible:outline-jolu-accent"
                >
                  重新扫描
                </button>
              </div>
            )}
            {phase === "ready" && (
              <>
                {total === 0 && <EmptyGuidance />}
                <div className="flex flex-col gap-5">
                  {groups.map((group) => <SourceGroup key={group.source} group={group} selected={selected} busy={applying} onToggle={(reference) => setSelected((current) => toggleReference(current, reference))} />)}
                </div>
                {total === 0 && (
                  <p className="text-[10.5px] text-jolu-muted">Jolu 自己接线写入这些文件的网关地址会被自动跳过，不会重复导入。</p>
                )}
              </>
            )}
          </div>

          <div className="flex items-center gap-3 border-t border-jolu-border pt-4">
            <span className="text-[11px] text-jolu-muted">{selected.size > 0 ? `已选 ${selected.size} / ${total} 个` : "勾选要导入的条目"}</span>
            <span className="flex-1" />
            <JoluButton variant="outline" disabled={applying} onClick={onClose}>取消</JoluButton>
            <JoluButton variant="primary" disabled={applying || selected.size === 0} onClick={apply}>
              {applying && <LoaderCircle aria-hidden className="animate-spin" />}
              导入所选
            </JoluButton>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

/** 单个来源：草稿行可勾选，读取失败的错误条目只展示原因 */
function SourceGroup({ group, selected, busy, onToggle }: {
  group: ImportGroup
  selected: ReadonlySet<string>
  busy: boolean
  onToggle: (reference: string) => void
}) {
  if (group.drafts.length === 0 && group.errors.length === 0) return null
  return (
    <section className="flex flex-col">
      <header className="flex items-center gap-2 pb-1.5">
        <span className="text-[13px] font-semibold text-jolu-text">{group.label}</span>
        {group.drafts.length > 0 && (
          <span className="rounded-[5px] bg-jolu-side px-1.5 text-[10px] leading-[18px] text-jolu-muted">{group.drafts.length} 条</span>
        )}
      </header>
      {group.drafts.length > 0 && (
        <ul className="flex flex-col rounded-lg border border-jolu-border">
          {group.drafts.map((draft) => (
            <li key={draft.reference} className="border-t border-jolu-border first:border-t-0">
              <label className={cn("flex cursor-pointer items-start gap-2.5 px-3 py-2.5", !busy && "hover:bg-jolu-stage")}>
                <input
                  type="checkbox"
                  checked={selected.has(draft.reference)}
                  disabled={busy}
                  aria-label={`导入 ${draft.name}`}
                  onChange={() => onToggle(draft.reference)}
                  className="mt-[3px] size-3.5 accent-[var(--jolu-accent)]"
                />
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="flex min-w-0 items-center gap-2">
                    <span className="min-w-0 truncate text-xs font-medium text-jolu-text">{draft.name}</span>
                    <span className="shrink-0 rounded-[5px] bg-jolu-side px-1.5 py-px text-[10px] leading-[16px] text-jolu-muted">{protocolLabel(draft.protocol)}</span>
                  </span>
                  <span className="truncate text-[10.5px] text-jolu-muted">{draft.from} · {draft.baseUrl}</span>
                </span>
                <span className="shrink-0 pt-0.5 font-mono text-[10.5px] tabular-nums text-jolu-muted">{draft.keyMasked}</span>
              </label>
            </li>
          ))}
        </ul>
      )}
      {group.errors.map((error, index) => (
        <p key={index} className="flex items-start gap-2 py-2 text-[10.5px] leading-[1.6] text-jolu-danger">
          <AlertTriangle aria-hidden className="mt-px size-3.5 shrink-0" />
          <span className="min-w-0 flex-1">读取失败：{error}</span>
        </p>
      ))}
    </section>
  )
}

/** 空状态（画板待确认）：给出来源路径，提示去哪里准备要导入的上游 */
function EmptyGuidance() {
  return (
    <div className="flex flex-col gap-2.5 rounded-xl bg-jolu-side p-4">
      <span className="text-xs font-medium text-jolu-text">没有找到可导入的上游</span>
      <p className="text-[11px] leading-[1.6] text-jolu-muted">Jolu 会读取以下本机配置里的地址与密钥，只读原文：</p>
      <ul className="flex flex-col gap-1.5">
        {SOURCE_PATHS.map((entry) => (
          <li key={entry.label} className="flex items-baseline gap-2 text-[11px]">
            <span className="w-[84px] shrink-0 text-jolu-text">{entry.label}</span>
            <code className="min-w-0 truncate font-mono text-[10.5px] text-jolu-muted">{entry.path}</code>
          </li>
        ))}
      </ul>
    </div>
  )
}
