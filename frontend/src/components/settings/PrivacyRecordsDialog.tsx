import { EVENTS } from "@/lib/events"
import { useEffect, useState } from "react"
import { listenEvent } from "@/lib/api"
import { settingsApi } from "@/lib/settingsApi"
import { useToast } from "@/components/ui/Toast"
import { MASK_ACTIONS, hitSides, kindName, recordSummary, type MaskRecordDto } from "./privacyMasking"
import { cn } from "@/lib/utils"
import { Banner, Button, Dialog } from "@/ui"

/** 脱敏记录对照：每条请求遮了什么、发出前后的对照（只在内存，重启后清空）。 */
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
    <Dialog
      open
      onOpenChange={(next) => { if (!next) onClose() }}
      width="xl"
      title="脱敏记录"
      description="原值只在这台电脑的内存里，记录只留种类、长度和开头几个字；Jolu 重启后记录清空。弹窗打开时每 15 秒刷新一次。"
      footerStart={records ? `共 ${records.length} 条记录` : undefined}
      actions={
        <>
          <Button kind="ghost" size="md" disabled={!records?.length} onClick={() => void clear()}>清空记录</Button>
          <Button kind="primary" size="md" onClick={onClose}>关闭</Button>
        </>
      }
    >
      {error && <Banner tone="danger">读取失败：{error}</Banner>}
      {records && records.length === 0 && <p className="py-10 text-center text-sm text-jn-text-3">还没有脱敏记录</p>}
      {records && records.length > 0 && current && (
        <div className="grid max-h-[440px] min-h-0 flex-1 grid-cols-1 gap-4 overflow-hidden sm:grid-cols-[220px_1fr]">
          <ul className="flex min-h-0 flex-col gap-1 overflow-y-auto pr-1">
            {records.map((record) => (
              <li key={record.id}>
                <button
                  type="button"
                  onClick={() => setPicked(record.id)}
                  className={cn(
                    "jn-interactive flex w-full flex-col items-start gap-0.5 rounded-[12px] px-3 py-2 text-left outline-none",
                    record.id === current.id ? "bg-jn-primary-wash text-jn-primary-ink" : "hover:bg-jn-well",
                  )}
                >
                  <span className="tnum text-xs font-semibold text-jn-text">{new Date(record.at_ms).toLocaleTimeString()} · {record.entry}</span>
                  <span className={cn("text-[11.5px]", record.blocked ? "text-jn-danger" : "text-jn-text-3")}>{recordSummary(record)}</span>
                </button>
              </li>
            ))}
          </ul>
          <div className="flex min-h-0 flex-col gap-2 overflow-y-auto">
            <div className="grid grid-cols-2 gap-2 px-3 text-[11.5px] font-medium text-jn-text-3">
              <span>发出前</span>
              <span>发给厂商的</span>
            </div>
            {current.hits.map((hit, index) => {
              const sides = hitSides(hit)
              return (
                <div key={index} className="flex flex-col gap-2 rounded-[14px] border border-jn-line p-3">
                  <div className="flex items-center gap-2 text-xs">
                    <span className="font-semibold text-jn-text">{kindName(hit.kind)}</span>
                    <span className="rounded-full bg-jn-well px-2 py-0.5 text-[11px] text-jn-text-2">{MASK_ACTIONS.find((a) => a.value === hit.action)?.label}</span>
                  </div>
                  <div className="grid grid-cols-2 gap-2">
                    <code className="break-all rounded-[10px] bg-jn-well px-2.5 py-2 font-mono text-[12px] text-jn-text">{sides.before}</code>
                    <code className="break-all rounded-[10px] bg-jn-well px-2.5 py-2 font-mono text-[12px] text-jn-text">{sides.after}</code>
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      )}
    </Dialog>
  )
}

/** 第一次真的遮住值时，主窗口右上角提示一次（系统通知由后端同时发出）。 */
export function PrivacyFirstNotice() {
  const toast = useToast()
  useEffect(() => {
    let off: (() => void) | null = null
    let stopped = false
    void listenEvent(EVENTS.PRIVACY_FIRST_MASK, () => toast.success("隐私脱敏已生效", "请求离开本机前已把敏感值换成占位符，可在设置 › 隐私脱敏查看记录"))
      .then((unlisten) => { if (stopped) unlisten(); else off = unlisten })
      .catch(() => {})
    return () => { stopped = true; off?.() }
  }, [toast])
  return null
}
