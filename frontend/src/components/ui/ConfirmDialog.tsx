import { useEffect, useRef, useState, type ReactNode } from "react"
import { Banner, Button, Dialog } from "@/ui"

/** 共用确认弹窗（R1-38）：统一走 `@/ui` `Dialog`，确认前不执行，成功才关闭；失败与执行中保留上下文。 */
export function ConfirmDialog({
  title,
  description,
  confirmLabel,
  cancelLabel = "取消",
  busyLabel = "正在处理…",
  width = 480,
  returnFocusTo,
  danger = false,
  children,
  onConfirm,
  onClose,
}: {
  title: string
  description: ReactNode
  confirmLabel: string
  cancelLabel?: string
  busyLabel?: string
  width?: number
  compact?: boolean
  returnFocusTo?: HTMLElement | null
  danger?: boolean
  children?: ReactNode
  onConfirm: () => void | boolean | Promise<void | boolean>
  onClose: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const running = useRef(false)
  const trigger = useRef(returnFocusTo ?? (typeof document !== "undefined" ? document.activeElement : null))

  useEffect(() => () => {
    if (trigger.current instanceof HTMLElement && trigger.current.isConnected) {
      trigger.current.focus()
    }
  }, [])

  const dismiss = () => {
    if (!running.current) onClose()
  }

  const confirm = async () => {
    if (running.current) return
    running.current = true
    setBusy(true)
    setError(null)
    try {
      const result = await onConfirm()
      if (result === false) throw new Error("操作未完成，请检查原因后重试。")
      onClose()
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
      running.current = false
      setBusy(false)
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(next) => { if (!next) dismiss() }}
      preventClose={busy}
      width={width <= 440 ? "sm" : "md"}
      title={title}
      actions={
        <>
          <Button kind="ghost" size="md" disabled={busy} onClick={dismiss}>
            {cancelLabel}
          </Button>
          <Button kind={danger ? "danger" : "primary"} size="md" loading={busy} disabled={busy} onClick={() => void confirm()}>
            {busy ? busyLabel : confirmLabel}
          </Button>
        </>
      }
    >
      <Banner tone={danger ? "warning" : "info"}>
        <span className="whitespace-pre-line">{description}</span>
      </Banner>
      {children}
      {error && (
        <Banner tone="danger">
          <span role="alert">{error}</span>
        </Banner>
      )}
    </Dialog>
  )
}
