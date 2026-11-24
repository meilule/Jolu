/**
 * 选择日期范围弹窗 —— 设计稿 `docs/ui/jolu-new.pen` R1-28。
 *
 * 双月日历点选开始与结束，确认后才生效；取消保持原筛选。
 * 允许同一天；缺日期或开始晚于结束时确认不可用；今天之后的日期不可选。
 */
import { useState } from "react"
import { cn } from "@/lib/utils"
import { RANGE_PRESETS, canConfirmRange, dateKey, monthGrid, parseDateKey, pickDate, rangeDays, rangeLabel, zoneName, localZone, type DateRange } from "@/lib/usageStats"
import { Icon } from "@/icons"
import { Button, Dialog, IconButton } from "@/ui"

const WEEKDAYS = ["一", "二", "三", "四", "五", "六", "日"]

export function DateRangeDialog({ value, currentLabel, onConfirm, onClose }: {
  value: DateRange
  currentLabel?: string
  onConfirm: (range: DateRange) => void
  onClose: () => void
}) {
  const today = dateKey(new Date())
  const [draft, setDraft] = useState<{ start: string | null; end: string | null }>({ start: value.start, end: value.end })
  const [anchor, setAnchor] = useState(() => {
    const end = parseDateKey(value.end)
    return { year: end.getFullYear(), month: end.getMonth() }
  })
  const left = anchor.month === 0 ? { year: anchor.year - 1, month: 11 } : { year: anchor.year, month: anchor.month - 1 }
  const shift = (delta: number) => setAnchor(({ year, month }) => {
    const next = new Date(year, month + delta, 1)
    return { year: next.getFullYear(), month: next.getMonth() }
  })
  const nowMonth = new Date()
  const atLatest = anchor.year === nowMonth.getFullYear() && anchor.month === nowMonth.getMonth()
  const valid = canConfirmRange(draft.start, draft.end)
  const editing = draft.start === null ? "start" : "end"

  return (
    <Dialog
      open
      onOpenChange={(open) => { if (!open) onClose() }}
      width="lg"
      title="选择日期范围"
      description={`当前筛选：${currentLabel ?? rangeLabel(value)}。以下日期为待确认范围，确认后才生效。`}
      actions={
        <>
          <Button kind="ghost" size="md" onClick={onClose}>取消</Button>
          <Button
            kind="primary"
            size="md"
            icon="check"
            disabled={!valid}
            onClick={() => valid && onConfirm({ start: draft.start!, end: draft.end! })}
          >
            确认
          </Button>
        </>
      }
    >
      {/* 快捷预设胶囊 */}
      <div className="flex flex-wrap gap-1.5">
        {RANGE_PRESETS.map((preset) => {
          const range = preset.range(new Date())
          const active = draft.start === range.start && draft.end === range.end
          return (
            <button
              key={preset.key}
              type="button"
              onClick={() => setDraft({ start: range.start, end: range.end })}
              className={cn(
                "jn-interactive h-7 rounded-full px-3 text-xs font-medium",
                active
                  ? "bg-jn-primary text-jn-on-primary"
                  : "bg-jn-well text-jn-text-2 hover:bg-jn-track hover:text-jn-text",
              )}
            >
              {preset.label}
            </button>
          )
        })}
      </div>

      {/* 起止日期框 */}
      <div className="flex items-end gap-3">
        <DateBox label="开始日期" value={draft.start} active={editing === "start"} />
        <span aria-hidden className="pb-2.5 text-sm text-jn-text-3">→</span>
        <DateBox label="结束日期" value={draft.end} active={editing === "end"} />
      </div>

      {/* 双月日历 */}
      <div className="grid grid-cols-1 gap-5 sm:grid-cols-2">
        <Month year={left.year} month={left.month} draft={draft} today={today} onPrev={() => shift(-1)} onPick={(key) => setDraft((d) => pickDate(d, key))} />
        <Month year={anchor.year} month={anchor.month} draft={draft} today={today} onNext={atLatest ? undefined : () => shift(1)} onPick={(key) => setDraft((d) => pickDate(d, key))} />
      </div>

      {/* 底部汇总条（R1-28） */}
      <div className="flex flex-wrap items-center gap-2 rounded-[14px] bg-jn-well px-3.5 py-2.5 text-xs">
        <Icon name="calendar" size="sm" className="shrink-0 text-jn-primary" />
        <span className="font-semibold text-jn-text">
          {valid ? `已选择 ${rangeDays({ start: draft.start!, end: draft.end! })} 天` : "请选择结束日期"}
        </span>
        <span className="min-w-0 flex-1" />
        <span className="text-[11.5px] text-jn-text-3">
          按{zoneName(localZone())}时区统计，包含开始与结束当天；今天统计至当前时刻。
        </span>
      </div>
    </Dialog>
  )
}

function DateBox({ label, value, active }: { label: string; value: string | null; active: boolean }) {
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-1.5">
      <span className="text-xs font-semibold text-jn-text">{label}</span>
      <div className={cn("flex h-10 items-center gap-2 rounded-[14px] border-[1.5px] px-3.5", active ? "border-jn-primary bg-jn-surface" : "border-transparent bg-jn-well")}>
        <Icon name="calendar" size="sm" className="text-jn-text-3" />
        <span className={cn("tnum font-mono text-[13px]", value ? "text-jn-text" : "text-jn-text-3")}>
          {value ?? "请选择"}
        </span>
      </div>
    </div>
  )
}

function Month({ year, month, draft, today, onPrev, onNext, onPick }: {
  year: number
  month: number
  draft: { start: string | null; end: string | null }
  today: string
  onPrev?: () => void
  onNext?: () => void
  onPick: (key: string) => void
}) {
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-1.5">
      <div className="flex h-8 items-center justify-between">
        {onPrev ? <IconButton icon="prev" label="上一月" size="sm" kind="ghost" onClick={onPrev} /> : <span className="size-8" />}
        <span className="text-[13px] font-semibold text-jn-text">{year} 年 {month + 1} 月</span>
        {onNext ? <IconButton icon="next" label="下一月" size="sm" kind="ghost" onClick={onNext} /> : <span className="size-8" />}
      </div>
      <div className="grid grid-cols-7 gap-y-0.5">
        {WEEKDAYS.map((d) => <span key={d} className="grid h-6 place-items-center text-[11px] font-medium text-jn-text-3">{d}</span>)}
        {monthGrid(year, month).flat().map((key, index) => {
          if (!key) return <span key={`empty-${index}`} className="h-8" />
          const edge = key === draft.start || key === draft.end
          const inside = !!draft.start && !!draft.end && key > draft.start && key < draft.end
          const isToday = key === today
          const future = key > today
          return (
            <div key={key} className={cn("flex h-8 items-center justify-center", inside && "bg-jn-primary-wash")}>
              <button
                type="button"
                disabled={future}
                aria-pressed={edge || inside}
                aria-label={key}
                onClick={() => onPick(key)}
                className={cn(
                  "jn-interactive tnum grid size-[30px] place-items-center rounded-full text-xs font-medium disabled:pointer-events-none disabled:opacity-40",
                  edge
                    ? "bg-jn-primary font-bold text-jn-on-primary"
                    : isToday
                      ? "border border-jn-primary text-jn-primary-ink"
                      : "text-jn-text hover:bg-jn-well",
                )}
              >
                {Number(key.slice(8))}
              </button>
            </div>
          )
        })}
      </div>
    </div>
  )
}
