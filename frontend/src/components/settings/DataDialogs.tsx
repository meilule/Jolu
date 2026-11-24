import { useEffect, useState } from "react"
import { api, isTauri, type CleanupPreviewDto } from "@/lib/api"
import { PLATFORMS } from "@/lib/platforms"
import { RETENTION_OPTIONS, cleanupCutoffDate, formatCount } from "@/lib/settingsPage"
import { rangeLabel, rangeToMillis, todayRange, type DateRange } from "@/lib/usageStats"
import { useToast } from "@/components/ui/Toast"
import { DateRangeDialog } from "@/components/usage/DateRangeDialog"
import { SelectButton } from "./settingsUi"
import { Banner, Button, Checkbox, Dialog } from "@/ui"

/** 导出本地数据：按供应商与日期范围导出 JSON；不选日期时导出全部 */
export function ExportDialog({ onClose }: { onClose: () => void }) {
  const toast = useToast()
  const [platform, setPlatform] = useState("all")
  const [range, setRange] = useState<DateRange | null>(null)
  const [picking, setPicking] = useState(false)
  const [busy, setBusy] = useState(false)

  const run = () => {
    setBusy(true)
    const millis = range ? rangeToMillis(range) : null
    api.exportDataFile(platform === "all" ? null : platform, millis?.start ?? null, millis?.end ?? null)
      .then((path) => {
        if (!path) return
        toast.success("导出完成", path)
        onClose()
      })
      .catch((reason) => toast.danger("导出失败", String(reason)))
      .finally(() => setBusy(false))
  }

  return (
    <>
      <Dialog
        open
        onOpenChange={(next) => { if (!next && !busy) onClose() }}
        preventClose={busy}
        width="md"
        title="导出本地数据"
        description="导出统计记录与会话标题为 JSON，可在另一台设备导入。"
        actions={
          <>
            <Button kind="ghost" size="md" disabled={busy} onClick={onClose}>取消</Button>
            <Button kind="primary" size="md" icon="download" loading={busy} disabled={!isTauri || busy} onClick={run}>导出数据</Button>
          </>
        }
      >
        <div className="flex flex-col gap-3 rounded-[16px] bg-jn-well p-4">
          <div className="flex items-center gap-4">
            <span className="flex-1 text-[13px] font-medium text-jn-text">供应商</span>
            <SelectButton
              label="导出范围"
              value={platform}
              disabled={busy}
              onChange={setPlatform}
              options={[{ value: "all", label: "全部供应商" }, ...PLATFORMS.map((p) => ({ value: p.id, label: p.name }))]}
            />
          </div>
          <div className="flex items-center gap-3 border-t border-jn-line pt-3">
            <span className="flex-1 text-[13px] font-medium text-jn-text">日期范围</span>
            {range && (
              <button
                type="button"
                disabled={busy}
                onClick={() => setRange(null)}
                className="text-xs text-jn-text-3 outline-none hover:text-jn-primary-ink disabled:opacity-45"
              >
                全部日期
              </button>
            )}
            <Button kind="ghost" size="sm" icon="calendar" disabled={busy} onClick={() => setPicking(true)}>
              {range ? rangeLabel(range) : "全部日期"}
            </Button>
          </div>
        </div>
        <Banner tone="info">导出文件不包含 API Key、登录凭证和原始会话正文。</Banner>
      </Dialog>

      {picking && (
        <DateRangeDialog
          value={range ?? todayRange()}
          currentLabel={range ? undefined : "全部日期"}
          onConfirm={(next) => { setRange(next); setPicking(false) }}
          onClose={() => setPicking(false)}
        />
      )}
    </>
  )
}

/** 清理历史数据确认（R1-38）：先预览范围，勾选确认后才允许删除 */
export function CleanupDialog({ initialDays, onClose, onExport, onDone }: {
  initialDays: number
  onClose: () => void
  onExport: () => void
  onDone: () => void
}) {
  const toast = useToast()
  const [days, setDays] = useState(initialDays)
  const [preview, setPreview] = useState<CleanupPreviewDto | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [confirmed, setConfirmed] = useState(false)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    setPreview(null)
    setError(null)
    setConfirmed(false)
    if (!isTauri) return
    let stopped = false
    api.cleanupPreview(days)
      .then((next) => { if (!stopped) setPreview(next) })
      .catch((reason) => { if (!stopped) setError(String(reason)) })
    return () => { stopped = true }
  }, [days])

  const count = preview?.requests ?? null
  const run = () => {
    setBusy(true)
    api.cleanupHistory(days)
      .then((result) => {
        toast.success(`已清理 ${result.requests.toLocaleString("zh-CN")} 条请求记录`, `保留最近 ${days} 天；采集位置保留，旧日志不会重新导入`)
        onDone()
      })
      .catch((reason) => toast.danger("历史清理失败", String(reason)))
      .finally(() => setBusy(false))
  }

  return (
    <Dialog
      open
      onOpenChange={(next) => { if (!next && !busy) onClose() }}
      preventClose={busy}
      width="lg"
      title="清理历史数据"
      description="先确认范围，再删除本地统计记录。"
      footerStart={
        <Button kind="ghost" size="sm" icon="download" disabled={busy} onClick={onExport}>
          先导出数据
        </Button>
      }
      actions={
        <>
          <Button kind="ghost" size="md" disabled={busy} onClick={onClose}>取消</Button>
          <Button kind="danger" size="md" loading={busy} disabled={!confirmed || !count || busy} onClick={run}>
            {count === null ? "清理" : count > 0 ? `清理 ${formatCount(count)} 条` : "没有可清理的记录"}
          </Button>
        </>
      }
    >
      <section className="flex flex-col rounded-[16px] border border-jn-line px-4">
        <div className="flex min-h-[64px] items-center gap-4">
          <span className="flex flex-1 flex-col gap-0.5">
            <span className="text-[13px] font-medium text-jn-text">保留最近</span>
            <span className="text-xs text-jn-text-3">此日期之前的数据将被清理。</span>
          </span>
          <SelectButton
            label="保留最近"
            value={String(days)}
            disabled={busy}
            onChange={(v) => setDays(Number(v))}
            options={RETENTION_OPTIONS.map((n) => ({ value: String(n), label: `${n} 天` }))}
          />
        </div>
        <div className="flex min-h-[64px] items-center gap-4 border-t border-jn-line">
          <span className="flex flex-1 flex-col gap-0.5">
            <span className="text-[13px] font-medium text-jn-text">截止日期</span>
            <span className="text-xs text-jn-text-3">根据当前日期与时区计算。</span>
          </span>
          <span className="tnum font-mono text-xs text-jn-text-2">{cleanupCutoffDate(days, new Date(), preview?.cutoff_ms)}</span>
        </div>
      </section>

      <div className="grid grid-cols-2 gap-3 rounded-[16px] bg-jn-well p-4">
        <span className="flex flex-col gap-1">
          <span className="text-xs text-jn-text-3">请求记录</span>
          <span className="tnum font-mono text-[22px] font-bold text-jn-text">{count === null ? (error ? "—" : "读取中…") : `${formatCount(count)} 条`}</span>
        </span>
        <span className="flex flex-col gap-1">
          <span className="text-xs text-jn-text-3">预计释放</span>
          <span className="tnum font-mono text-[22px] font-bold text-jn-text">—</span>
          <span className="text-[11px] text-jn-text-3">数据库文件不会立即缩小，释放量待确认</span>
        </span>
      </div>

      {error && <Banner tone="danger"><span role="alert">清理预览失败：{error}</span></Banner>}
      {preview && <p className="text-xs text-jn-text-3">同时删除 {formatCount(preview.usage_events)} 条增量事件、{formatCount(preview.sessions)} 条已结束会话信息。</p>}

      <Banner tone="danger">
        删除后无法撤销；原始会话文件、账号与 Skills 均会保留。采集位置会保留，旧日志不会在下一轮重新导入。
      </Banner>

      <div className="flex items-center gap-2.5 self-start">
        <Checkbox checked={confirmed} disabled={busy || !preview} label="我已了解本次清理范围" onChange={setConfirmed} />
        <button type="button" disabled={busy || !preview} onClick={() => setConfirmed((v) => !v)} className="text-xs font-medium text-jn-text outline-none disabled:opacity-45">
          我已了解本次清理范围
        </button>
      </div>
    </Dialog>
  )
}
