import { useMemo, useState } from "react"
import { activityLevel, calendarWeeks, formatMinutes, formatMoney, formatTokens, monthLabels } from "@/lib/usageStats"
import { convertFromUsd, type FxState } from "@/lib/costCurrency"
import type { StatsActivityDto } from "@/lib/api"
import { Tip } from "@/components/ui/Tip"
import { ContentTransition, LoadingSurface } from "@/components/ui/LoadingSurface"
import { Card } from "./usageControls"
import { Tabs } from "@/ui"

const LEVEL_COLORS = ["var(--jn-track)", "var(--jn-mint-2)", "var(--jn-mint)", "var(--jn-primary)", "var(--jn-primary-ink)"] as const

type ActivityMetric = "tokens" | "requests" | "cost"

/** 本机活跃趋势 · 全年日历（R1-23 / R1-48）：支持 Token / 请求 / 费用三档着色。 */
export function ActivityCalendar({ activity, error, fx }: {
  error?: string | null
  activity: StatsActivityDto | null
  fx: FxState
}) {
  const [metric, setMetric] = useState<ActivityMetric>("tokens")

  const view = useMemo(() => {
    if (!activity) return null
    const weeks = calendarWeeks(activity.first_date, activity.last_date)
    const byDate = new Map(activity.days.map((d) => [d.date, d]))
    const pick = (d: StatsActivityDto["days"][number]) =>
      metric === "tokens" ? d.tokens : metric === "requests" ? d.requests : (d.cost ?? 0)
    const sorted = activity.days
      .map(pick)
      .filter((v) => v > 0)
      .sort((a, b) => a - b)
    return { weeks, labels: monthLabels(weeks), byDate, sorted }
  }, [activity, metric])

  return (
    <Card className="flex flex-col gap-3 p-5">
      <div className="flex flex-wrap items-center gap-2.5">
        <h3 className="text-[15px] font-semibold text-jn-text">全年活跃</h3>
        <span className="text-xs text-jn-text-3">近一年 · 本机记录</span>
        <span className="min-w-0 flex-1" />
        <Tabs
          style="segment"
          value={metric}
          onChange={setMetric}
          items={[
            { key: "tokens", label: "Token" },
            { key: "requests", label: "请求" },
            { key: "cost", label: "费用" },
          ]}
        />
      </div>
      <ContentTransition identity={view ? "ready" : error ? "error" : "loading"}>
        {view ? (
          <div className="flex flex-col gap-2">
            <div className="relative h-4">
              {view.labels.map((l) => (
                <span key={`${l.column}-${l.label}`} className="absolute text-xs text-jn-text-3" style={{ left: `${(l.column / view.weeks.length) * 100}%` }}>{l.label}</span>
              ))}
            </div>
            <div className="grid gap-x-[1.4px]" style={{ gridTemplateColumns: `repeat(${view.weeks.length}, minmax(0, 1fr))` }}>
              {view.weeks.map((week, column) => (
                <div key={column} className="flex flex-col gap-[3px]">
                  {week.map((key, row) => {
                    if (!key) return <span key={row} className="h-[19px]" />
                    const day = view.byDate.get(key)
                    const val = day
                      ? (metric === "tokens" ? day.tokens : metric === "requests" ? day.requests : (day.cost ?? 0))
                      : 0
                    const level = activityLevel(val, view.sorted)
                    const dayCost = day && day.cost !== undefined ? formatMoney(convertFromUsd(day.cost, fx), fx.currency) : "—"
                    return (
                      <Tip key={key} side="top" className="min-w-[204px] rounded-xl px-3.5 py-3" label={
                        <span className="flex flex-col gap-2">
                          <span className="tnum border-b border-jn-line pb-2 text-xs font-semibold">{key}</span>
                          <span className="flex items-center justify-between gap-6"><span className="text-jn-text-3">Token 用量</span><span className="tnum font-medium">{day ? formatTokens(day.tokens) : "无记录"}</span></span>
                          <span className="flex items-center justify-between gap-6"><span className="text-jn-text-3">调用次数</span><span className="tnum font-medium">{day ? `${day.requests} 次` : "无记录"}</span></span>
                          <span className="flex items-center justify-between gap-6">
                            <span className="text-jn-text-3">估算费用</span>
                            <span className="tnum font-medium">
                              {day ? `≈ ${dayCost}` : "无记录"}
                              {day && (day.unpriced_requests ?? 0) > 0 ? ` (+${day.unpriced_requests} 未计价)` : ""}
                            </span>
                          </span>
                        </span>
                      }>
                        <span
                          tabIndex={0}
                          aria-label={day ? `${key}，${formatTokens(day.tokens)} Token，${day.requests} 次调用，估算费用 ${dayCost}` : `${key}，无记录`}
                          className="h-[19px] rounded-[3px] outline-none transition-[background-color,box-shadow] duration-200 hover:ring-2 hover:ring-jn-primary focus-visible:ring-2 focus-visible:ring-jn-primary"
                          style={{ background: LEVEL_COLORS[level] }}
                        />
                      </Tip>
                    )
                  })}
                </div>
              ))}
            </div>
          </div>
        ) : (
          error ? <span className="text-xs text-jn-text-3">活跃趋势暂不可用，请重试。</span> : <LoadingSurface layout="calendar" label="正在读取活跃趋势" />
        )}
      </ContentTransition>
      <div className="flex h-[18px] items-center justify-end gap-[5px] text-[11px] text-jn-text-3">
        少
        {LEVEL_COLORS.map((color) => <span key={color} aria-hidden className="size-2.5 rounded-sm" style={{ background: color }} />)}
        多
      </div>
      <span className="h-px bg-jn-line" />
      <div className="flex flex-wrap items-center gap-6">
        <Summary width={192} label="今日活跃时长（估算）" value={activity ? formatMinutes(activity.today_active_minutes) : "—"} />
        <span className="hidden h-9 w-px bg-jn-line sm:block" />
        <Summary width={148} label="本周活跃天数" value={activity ? `${activity.week_active_days} 天` : "—"} />
        <span className="hidden h-9 w-px bg-jn-line sm:block" />
        <Summary width={148} label="本月活跃天数" value={activity ? `${activity.month_active_days} 天` : "—"} />
      </div>
    </Card>
  )
}

function Summary({ width, label, value }: { width: number; label: string; value: string }) {
  return (
    <div className="flex flex-col gap-1" style={{ width }}>
      <span className="text-xs text-jn-text-3">{label}</span>
      <span className="tnum font-mono text-[17px] font-bold text-jn-text">{value}</span>
    </div>
  )
}
