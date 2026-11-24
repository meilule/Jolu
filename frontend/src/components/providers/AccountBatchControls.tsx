import { useRef, useState } from "react"
import type { AccountSelection } from "@/hooks/useAccountSelection"
import type { BatchAccountDeleteResult } from "@/lib/accountActions"
import { Banner, BatchAction, BatchBar, Button, Dialog, StatusPill } from "@/ui"

export interface BatchAccountTarget {
  id: string
  label: string
  provider: string
  current?: boolean
}

export function AccountBatchToolbar({ selection, total, disabled = false, onDelete }: {
  selection: AccountSelection
  total: number
  disabled?: boolean
  onDelete: () => void
}) {
  const count = selection.selectedIds.length
  if (!selection.active) {
    return (
      <Button kind="ghost" size="sm" icon="listChecks" disabled={disabled || total === 0} onClick={selection.enter}>
        批量管理
      </Button>
    )
  }
  return (
    <BatchBar
      summary={`已选 ${count} / ${total} 个账号`}
      disabled={disabled}
      onClose={selection.exit}
      actions={
        <>
          <BatchAction
            icon="check"
            disabled={disabled || total === 0 || count === total}
            onClick={selection.selectAll}
          >
            全选当前
          </BatchAction>
          <BatchAction
            icon="undo"
            disabled={disabled || count === 0}
            onClick={selection.clear}
          >
            清空已选
          </BatchAction>
          <BatchAction
            icon="delete"
            danger
            disabled={disabled || count === 0}
            onClick={onDelete}
          >
            删除所选（{count}）
          </BatchAction>
        </>
      }
    />
  )
}

/** R1-42 批量删除账号确认弹窗 */
export function BatchAccountDeleteDialog({ targets, hidden = false, onClose, onConfirm }: {
  targets: readonly BatchAccountTarget[]
  hidden?: boolean
  onClose: () => void
  onConfirm: (ids: string[]) => Promise<BatchAccountDeleteResult>
}) {
  // 确认期间固定目标快照；父级列表刷新或筛选变化不能更换本次删除对象。
  const [snapshot] = useState(() => targets.filter((target, index) => targets.findIndex(item => item.id === target.id) === index))
  const [busy, setBusy] = useState(false)
  const running = useRef(false)
  const [deletedIds, setDeletedIds] = useState<string[]>([])
  const [failed, setFailed] = useState<BatchAccountDeleteResult["failed"] | null>(null)
  const deleted = new Set(deletedIds)
  const failures = new Map(failed?.map(item => [item.id, item.message]))
  const pendingIds = failed ? failed.map(item => item.id) : snapshot.map(item => item.id)
  const close = () => { if (!running.current) onClose() }

  const confirm = async () => {
    if (running.current || pendingIds.length === 0) return
    running.current = true
    setBusy(true)
    try {
      const result = await onConfirm(pendingIds)
      const succeeded = new Set([...deletedIds, ...result.deletedIds])
      setDeletedIds([...succeeded])
      setFailed(result.failed)
      if (result.failed.length === 0) onClose()
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      setFailed(pendingIds.map(id => ({ id, message })))
    } finally {
      running.current = false
      setBusy(false)
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => { if (!open) close() }}
      preventClose={busy}
      width="md"
      title="删除所选账号？"
      description={`共 ${snapshot.length} 个账号连接${failed ? ` · 已删除 ${deletedIds.length} 个 · 失败 ${failed.length} 个` : ""}。仅从 Jolu 移除所选连接及保存的凭据，签到与用量历史保留；远端账号、API Key 和原工具登录不受影响。`}
      actions={
        <>
          <Button kind="ghost" size="md" disabled={busy} onClick={close}>{failed ? "关闭" : "取消"}</Button>
          <Button
            kind="danger"
            size="md"
            icon={failed ? "retry" : "delete"}
            loading={busy}
            disabled={busy || pendingIds.length === 0}
            onClick={() => void confirm()}
          >
            {failed ? `重试失败项（${pendingIds.length}）` : `删除 ${snapshot.length} 个账号`}
          </Button>
        </>
      }
    >
      {snapshot.some((target) => target.current) && (
        <Banner tone="warning">
          所选连接包含当前使用账号。删除后不会自动切换原工具或选择替代账号。
        </Banner>
      )}

      <ul className="flex flex-col gap-1.5" aria-label="待删除的账号连接">
        {snapshot.map((target, index) => (
          <li key={target.id} className="flex items-center gap-2.5 rounded-[14px] bg-jn-well px-3.5 py-2.5">
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <span className="break-all text-[13px] font-semibold text-jn-text">
                  {hidden ? `账号 ${index + 1}` : target.label}
                </span>
                {target.current && <StatusPill tone="success">当前使用</StatusPill>}
              </div>
              <p className="mt-0.5 text-[11.5px] text-jn-text-3">{target.provider}</p>
              {failures.has(target.id) && (
                <p role="alert" className="mt-1 break-words text-[11.5px] leading-relaxed text-jn-danger">
                  {hidden ? "删除失败，请显示账号信息后查看原因。" : failures.get(target.id)}
                </p>
              )}
            </div>
            {deleted.has(target.id) && <StatusPill tone="neutral">已删除</StatusPill>}
            {failures.has(target.id) && <StatusPill tone="danger">删除失败</StatusPill>}
          </li>
        ))}
      </ul>

      {failed && failed.length > 0 && (
        <p className="text-xs text-jn-danger" role="status">
          部分连接删除失败，已删除的连接不会再次提交。
        </p>
      )}
    </Dialog>
  )
}
