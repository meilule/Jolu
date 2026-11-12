import * as Dialog from "@radix-ui/react-dialog"
import { useEffect, useState } from "react"
import { X } from "lucide-react"
import { listenEvent } from "@/lib/api"
import { settingsApi } from "@/lib/settingsApi"
import { useToast } from "@/components/ui/Toast"
import { ActionButton } from "./settingsUi"
import { MASK_ACTIONS, hitSides, kindName, recordSummary, type MaskRecordDto } from "./privacyMasking"

/**
 * 脱敏记录对照（3.7）：每条请求遮了什么、发出前后的对照。
 * 待确认：Jolu.pen 没有此弹窗，按设置页既有对话框样式做最小版。
 * 记录只在内存、不含原值（只有种类、长度、开头几个字与前后文），重启后清空。
 */
export function PrivacyRecordsDialog({ onClose }: { onClose: () => void }) {
  const toast = useToast()
  const [records, setRecords] = useState<MaskRecordDto[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [picked, setPicked] = useState<number | null>(null)

  useEffect(() => {
    let stopped = false
    const load = () => settingsApi.privacyRecords()
      .then((next) => { if (!stopped) { setRecords(next); setError(null) } })
      .catch((reason) => { if (!stopped) setError(String(reason)) })
    void load()
    const timer = window.setInterval(() => void load(), 15000)
    return () => { stopped = true; window.clearInterval(timer) }
  }, [])

  const clear = () => settingsApi.privacyRecordsClear()
    .then(() => { setRecords([]); setPicked(null); toast.success("脱敏记录已清空", "之后的请求照常记录") })
    .catch((reason) => toast.danger("清空失败", String(reason)))

  const current = records?.find((record) => record.id === picked) ?? records?.[0] ?? null

  return (
    <Dialog.Root open onOpenChange={(open) => !open && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/35" />
        <Dialog.Content
          aria-describedby={undefined}
          className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] w-[760px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-2xl border border-jolu-border bg-jolu-cc-surface p-6 shadow-dialog outline-none"
        >
          <div className="flex items-center gap-2.5">
            <Dialog.Title className="flex-1 text-xl font-semibold text-jolu-text">脱敏记录</Dialog.Title>
            <ActionButton tone="secondary" disabled={!records?.length} onClick={() => void clear()}>清空记录</ActionButton>
            <Dialog.Close aria-label="关闭" className="grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
              <X aria-hidden className="size-4" />
            </Dialog.Close>
          </div>
          <p className="text-xs leading-relaxed text-jolu-muted">
            原值只在这台电脑的内存里，记录只留种类、长度和开头几个字；Jolu 重启后记录清空。弹窗打开时每 15 秒刷新一次。
          </p>
          {error && <p className="text-xs text-jolu-danger">读取失败：{error}</p>}
          {records && records.length === 0 && <p className="py-8 text-center text-sm text-jolu-muted">还没有脱敏记录</p>}
          {records && records.length > 0 && current && (
            <div className="grid min-h-0 flex-1 grid-cols-[220px_1fr] gap-4 overflow-hidden">
              <ul className="flex min-h-0 flex-col gap-1 overflow-y-auto pr-1">
                {records.map((record) => (
                  <li key={record.id}>
                    <button
                      type="button"
                      onClick={() => setPicked(record.id)}
                      className={`flex w-full flex-col items-start gap-0.5 rounded-lg px-3 py-2 text-left outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent ${record.id === current.id ? "bg-jolu-stage" : "hover:bg-jolu-side"}`}
                    >
                      <span className="text-xs font-medium text-jolu-text">{new Date(record.at_ms).toLocaleTimeString()} · {record.entry}</span>
                      <span className={`text-[12px] ${record.blocked ? "text-jolu-danger" : "text-jolu-muted"}`}>{recordSummary(record)}</span>
                    </button>
                  </li>
                ))}
              </ul>
              <div className="flex min-h-0 flex-col gap-2 overflow-y-auto">
                <div className="grid grid-cols-2 gap-2 px-3 text-[12px] text-jolu-muted">
                  <span>发出前</span>
                  <span>发给厂商的</span>
                </div>
                {current.hits.map((hit, index) => {
                  const sides = hitSides(hit)
                  return (
                    <div key={index} className="flex flex-col gap-1.5 rounded-lg border border-jolu-border p-3">
                      <div className="flex items-center gap-2 text-xs">
                        <span className="font-medium text-jolu-text">{kindName(hit.kind)}</span>
                        <span className="text-jolu-muted">{MASK_ACTIONS.find((a) => a.value === hit.action)?.label}</span>
                      </div>
                      <div className="grid grid-cols-2 gap-2">
                        <code className="break-all rounded bg-jolu-side px-2 py-1.5 font-mono text-[12px] text-jolu-text">{sides.before}</code>
                        <code className="break-all rounded bg-jolu-side px-2 py-1.5 font-mono text-[12px] text-jolu-text">{sides.after}</code>
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

/** 第一次真的遮住值时，主窗口右上角提示一次（系统通知由后端同时发出）。 */
export function PrivacyFirstNotice() {
  const toast = useToast()
  useEffect(() => {
    let off: (() => void) | null = null
    let stopped = false
    void listenEvent("privacy-first-mask", () => toast.success("隐私脱敏已生效", "请求离开本机前已把敏感值换成占位符，可在设置 › 隐私脱敏查看记录"))
      .then((unlisten) => { if (stopped) unlisten(); else off = unlisten })
      .catch(() => {})
    return () => { stopped = true; off?.() }
  }, [toast])
  return null
}
