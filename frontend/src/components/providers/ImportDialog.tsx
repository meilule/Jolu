import { useCallback, useEffect, useRef, useState } from "react"
import { api } from "@/lib/api"
import { cn } from "@/lib/utils"
import { useToast } from "@/components/ui/Toast"
import { Icon } from "@/icons"
import { Banner, Button, Checkbox, Dialog, Tag } from "@/ui"
import {
  SOURCE_PATHS, groupImportDrafts, importToastDetail, importToastTitle, protocolLabel,
  selectableReferences, toggleReference, type ImportGroup,
} from "./importDrafts"

/**
 * 从本机导入（R1-41）：把 CC Switch / Claude Code / Codex 里已有的上游读成草稿，
 * 勾选后由后端重读原文创建自定义连接；键不出后端，草稿里只显示掩码，
 * 成功后后端广播 connections-changed，连接列表由既有监听自动刷新。
 */
export function ImportDialog({ onClose }: { onClose: () => void }) {
  const toast = useToast()
  const [phase, setPhase] = useState<"scanning" | "ready" | "error">("scanning")
  const [scanError, setScanError] = useState<string | null>(null)
  const [groups, setGroups] = useState<readonly ImportGroup[]>([])
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set())
  const [applying, setApplying] = useState(false)
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

  const allRefs = selectableReferences(groups)
  const total = allRefs.length
  const allChecked = total > 0 && selected.size === total
  const indeterminate = selected.size > 0 && selected.size < total

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
    <Dialog
      open
      onOpenChange={(next) => { if (!next && !applying) onClose() }}
      preventClose={applying}
      width="lg"
      title="从本机导入"
      description="把 CC Switch、Claude Code、Codex 里已有的上游读成草稿，勾选后导入为自定义连接；密钥只显示掩码，导入由后端重读原文，不改写来源文件。"
      footerStart={selected.size > 0 ? `已选 ${selected.size} / ${total} 个` : "勾选要导入的条目"}
      actions={
        <>
          <Button kind="ghost" size="md" disabled={applying} onClick={onClose}>取消</Button>
          <Button kind="primary" size="md" icon="download" loading={applying} disabled={applying || selected.size === 0} onClick={apply}>
            {selected.size > 0 ? `导入所选 ${selected.size} 条` : "导入所选"}
          </Button>
        </>
      }
    >
      {/* 读取范围 + 重新扫描 */}
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-xs font-medium text-jn-text-3">读取：</span>
        {SOURCE_PATHS.map((src) => <Tag key={src.label}>{src.label}</Tag>)}
        <span className="min-w-0 flex-1" />
        <Button kind="ghost" size="sm" icon="refresh" disabled={applying || phase === "scanning"} onClick={load}>
          重新扫描
        </Button>
      </div>

      {phase === "scanning" && (
        <div role="status" className="flex items-center justify-center gap-2 py-12 text-xs text-jn-text-3">
          <Icon name="loading" size="md" className="animate-spin" />
          正在扫描本机配置…
        </div>
      )}

      {phase === "error" && (
        <Banner tone="danger" action={<Button kind="ghost" size="sm" onClick={load}>重新扫描</Button>}>
          扫描失败：{scanError}
        </Banner>
      )}

      {phase === "ready" && (
        <>
          {total > 0 && (
            <div className="flex items-center justify-between px-1">
              <Checkbox
                checked={allChecked}
                indeterminate={indeterminate}
                disabled={applying}
                label={allChecked ? `已全选 ${total} 个` : `已选 ${selected.size} 个`}
                onChange={(checked) => setSelected(checked ? new Set(allRefs) : new Set())}
              />
              <span className="text-[11.5px] text-jn-text-3">勾选要导入的条目</span>
            </div>
          )}

          {total === 0 && <EmptyGuidance />}

          <div className="flex flex-col gap-3">
            {groups.map((group) => (
              <SourceGroup
                key={group.source}
                group={group}
                selected={selected}
                busy={applying}
                onToggle={(reference) => setSelected((current) => toggleReference(current, reference))}
              />
            ))}
          </div>

          <p className="text-[11.5px] text-jn-text-3">
            Jolu 自己接线写入这些文件的网关地址会被自动跳过，不会重复导入。
          </p>
        </>
      )}
    </Dialog>
  )
}

function SourceGroup({ group, selected, busy, onToggle }: {
  group: ImportGroup
  selected: ReadonlySet<string>
  busy: boolean
  onToggle: (reference: string) => void
}) {
  if (group.drafts.length === 0 && group.errors.length === 0) return null
  return (
    <section className="flex flex-col gap-1.5">
      {group.drafts.map((draft) => {
        const checked = selected.has(draft.reference)
        return (
          <div
            key={draft.reference}
            onClick={() => { if (!busy) onToggle(draft.reference) }}
            className={cn(
              "jn-interactive flex cursor-pointer items-center gap-3 rounded-[14px] px-3 py-2.5",
              checked ? "bg-jn-primary-wash" : "bg-jn-well hover:bg-jn-track/60",
              busy && "pointer-events-none opacity-60",
            )}
          >
            <span onClick={(e) => e.stopPropagation()}>
              <Checkbox
                checked={checked}
                disabled={busy}
                aria-label={`导入 ${draft.name}`}
                onChange={() => onToggle(draft.reference)}
              />
            </span>
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="flex min-w-0 items-center gap-1.5">
                <span className="truncate text-[13px] font-semibold text-jn-text">{draft.name}</span>
                <Tag>{group.label}</Tag>
                <Tag>{protocolLabel(draft.protocol)}</Tag>
              </span>
              <span className="truncate font-mono text-[11.5px] text-jn-text-3">{draft.baseUrl}</span>
            </span>
            <span className="shrink-0 font-mono text-[11.5px] font-medium text-jn-text-2">{draft.keyMasked}</span>
          </div>
        )
      })}
      {group.errors.map((message, index) => (
        <div
          key={`${group.source}-err-${index}`}
          className="flex items-center gap-3 rounded-[14px] bg-jn-well px-3 py-2.5 opacity-75"
        >
          <Checkbox checked={false} disabled aria-label={`${group.label} 读取失败`} onChange={() => undefined} />
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="flex items-center gap-1.5">
              <span className="text-[13px] font-semibold text-jn-text">{group.label}</span>
              <Tag>不可导入</Tag>
            </span>
            <span className="text-[11.5px] text-jn-danger">读取失败：{message}</span>
          </span>
          <span className="shrink-0 font-mono text-xs text-jn-text-3">—</span>
        </div>
      ))}
    </section>
  )
}

function EmptyGuidance() {
  return (
    <div className="flex flex-col gap-2 rounded-[16px] bg-jn-well p-4 text-xs">
      <span className="font-semibold text-jn-text">没有找到可导入的上游</span>
      <span className="text-jn-text-3">会读取以下本机配置里的地址与密钥，只读原文：</span>
      <ul className="flex flex-col gap-1.5 pt-1">
        {SOURCE_PATHS.map((item) => (
          <li key={item.label} className="flex items-center gap-2 text-[11.5px]">
            <Tag>{item.label}</Tag>
            <code className="truncate font-mono text-jn-text-2">{item.path}</code>
          </li>
        ))}
      </ul>
    </div>
  )
}
