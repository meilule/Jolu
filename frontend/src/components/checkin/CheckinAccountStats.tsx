import { useEffect, type ReactNode } from "react"
import { EVENTS } from "@/lib/events"
import { useQuota } from "@/hooks/useQuota"
import { listenEvent } from "@/lib/api"
import type { CheckinStatistics, OfficialCheckinStatistics } from "@/lib/checkinApi"
import { checkinPeriodDisplay, checkinQuotaDisplay, checkinQuotaSummary } from "@/lib/checkinStats"
import { cn } from "@/lib/utils"
import { Button, Dialog } from "@/ui"

export interface CheckinAccountStatsProps {
  accountId: string
  statistics: CheckinStatistics | null
  active: boolean
  identityKey: string | null
  official?: OfficialCheckinStatistics | null
  showHistory?: boolean
  accountLabel: string
  providerLabel: string
  statusMessage: string
  detailsOpen: boolean
  onDetailsOpenChange: (open: boolean) => void
  action: ReactNode
  deleteAction: ReactNode
}

export function CheckinAccountStats({
  accountId,
  statistics,
  active,
  identityKey,
  official,
  showHistory = true,
  accountLabel,
  providerLabel,
  statusMessage,
  detailsOpen,
  onDetailsOpenChange,
  action,
  deleteAction,
}: CheckinAccountStatsProps) {
  const enabled = active && identityKey !== null
  const quota = useQuota(accountId, false, enabled)
  const { refresh } = quota

  useEffect(() => {
    if (!enabled) return
    let stopped = false
    let off: (() => void) | undefined
    void listenEvent(EVENTS.DAILY_CHECKIN_CHANGED, (id) => {
      if (!stopped && id === accountId) void refresh()
    }).then((unlisten) => {
      if (stopped) unlisten()
      else off = unlisten
    }).catch(() => {})
    return () => { stopped = true; off?.() }
  }, [accountId, enabled, refresh])

  const periods = ([["week", "本周签到"], ["month", "本月签到"]] as const)
    .map(([period, title]) => ({ title, source: official?.[period] ? "官方返回日期" : "Jolu 记录", ...checkinPeriodDisplay(statistics, period, official) }))
  const all = checkinPeriodDisplay(statistics, "all", official)
  const display = checkinQuotaDisplay(quota)
  const summary = checkinQuotaSummary(quota)

  return (
    <>
      <section aria-label="账号数据摘要" data-checkin-stats={accountId} className="flex min-w-0 flex-1 flex-col gap-2.5">
        <dl className="grid grid-cols-2 gap-x-4 gap-y-3.5 rounded-[14px] bg-jn-well p-3.5">
          {periods.map((period) => (
            <div key={period.title} className="min-w-0">
              <dt className="text-[11.5px] text-jn-text-3">{period.title}</dt>
              <dd className="tnum mt-1 font-mono text-[20px] font-bold leading-tight text-jn-text">{showHistory ? period.days : "不适用"}</dd>
            </div>
          ))}
          <div className="min-w-0">
            <dt className="text-[11.5px] text-jn-text-3">总额度</dt>
            <dd className="tnum mt-1 break-words font-mono text-[14px] font-semibold leading-snug text-jn-text">{summary.total}</dd>
          </div>
          <div className="min-w-0">
            <dt className="text-[11.5px] text-jn-text-3">{summary.remainingLabel}</dt>
            <dd className="tnum mt-1 break-words font-mono text-[14px] font-semibold leading-snug text-jn-text">{summary.remaining}</dd>
          </div>
        </dl>
        {(summary.status || summary.scope !== "账户额度") && (
          <div className="flex min-w-0 flex-wrap items-center justify-between gap-x-2 gap-y-1 text-[11px] text-jn-text-3">
            {summary.scope !== "账户额度" ? <span>额度 · {summary.scope}</span> : <span />}
            {summary.status && <span role="status" className={cn(quota.stale && "text-jn-danger")}>额度{summary.status}</span>}
          </div>
        )}
        <div data-checkin-actions={accountId} className="mt-auto flex items-center gap-1.5 pt-1.5">
          <Button size="sm" kind="ghost" aria-label={`查看账号详情 ${accountLabel}`} onClick={() => onDetailsOpenChange(true)}>
            详情
          </Button>
          <div className="min-w-0 flex-1">{action}</div>
          {deleteAction}
        </div>
      </section>

      <Dialog
        open={detailsOpen}
        onOpenChange={onDetailsOpenChange}
        width="md"
        title="账号详情"
        description={`${providerLabel} · ${accountLabel}`}
        actions={<Button kind="ghost" size="md" onClick={() => onDetailsOpenChange(false)}>关闭</Button>}
      >
        <section aria-label="签到统计与账户额度" data-checkin-details={accountId} className="flex min-w-0 flex-col gap-3.5">
          <div className="flex flex-col gap-1.5 rounded-[14px] bg-jn-well p-3.5">
            <h3 className="text-xs font-semibold text-jn-text">签到状态</h3>
            <p className="break-words text-xs leading-relaxed text-jn-text-2">{statusMessage}</p>
          </div>

          {showHistory && (
            <>
              <div className="grid grid-cols-2 gap-3 rounded-[14px] bg-jn-well p-3.5">
                {periods.map((period) => (
                  <div key={period.title} className="flex min-w-0 flex-col gap-1">
                    <p className="text-[11.5px] text-jn-text-3">{period.title}</p>
                    <p className="tnum font-mono text-[22px] font-bold leading-tight text-jn-text">{period.days}</p>
                    <p className={cn("tnum break-words text-xs font-medium", period.reward === "—" ? "text-jn-text-3" : "text-jn-primary-ink")}>获得 {period.reward}</p>
                    <p className="text-[11px] text-jn-text-3">{period.source}</p>
                  </div>
                ))}
              </div>
              <div className="flex min-w-0 items-baseline justify-between gap-2 px-1 text-xs">
                <span className="shrink-0 text-jn-text-3">{official?.all ? `${official.scope}签到获得` : "累计签到获得"}</span>
                <span className={cn("tnum min-w-0 break-words text-right font-semibold", all.reward === "—" ? "text-jn-text-3" : "text-jn-primary-ink")}>{all.reward}</span>
              </div>
              {statistics?.has_records && !statistics.unit.trim() && <p className="text-[11px] text-jn-text-3">奖励单位未获取或不一致</p>}
            </>
          )}

          <div aria-label="账户额度" aria-busy={quota.loading} data-checkin-quota={accountId} className="flex min-w-0 flex-col gap-2.5 rounded-[14px] border border-jn-line p-3.5">
            <h3 className="text-xs font-semibold text-jn-text">账户额度</h3>
            {display.rows.map((row) => (
              <div key={row.key} className="flex min-w-0 flex-col gap-1.5">
                {row.label !== "账户额度" && <p className="break-words text-xs font-medium text-jn-text-2">{row.label}</p>}
                <dl className="grid min-w-0 grid-cols-2 gap-3">
                  <div className="min-w-0">
                    <dt className="text-[11.5px] text-jn-text-3">总额度</dt>
                    <dd className="tnum mt-1 break-words font-mono text-[13px] font-semibold leading-snug text-jn-text">{row.total}</dd>
                  </div>
                  <div className="min-w-0">
                    <dt className="text-[11.5px] text-jn-text-3">{row.remainingLabel ?? "可用额度"}</dt>
                    <dd className="tnum mt-1 break-words font-mono text-[13px] font-semibold leading-snug text-jn-text">{row.remaining}</dd>
                  </div>
                </dl>
                {row.used && <p className="tnum text-[11px] text-jn-text-3">已用 {row.used}</p>}
                {row.reason && <p className="break-words text-[11px] leading-relaxed text-jn-text-3">{row.reason}</p>}
              </div>
            ))}
            {display.message && <p role="status" className={cn("break-words text-[11.5px] leading-relaxed", display.stale ? "font-medium text-jn-text" : "text-jn-text-3")}>{display.message}</p>}
            {display.stale && quota.fetchedAt && <p className="text-[11px] text-jn-text-3">上次更新 {quota.fetchedAt.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}</p>}
          </div>

          {showHistory && (
            <div className="flex flex-col gap-1 text-[11px] leading-relaxed text-jn-text-3">
              {official?.note && <p>{official.note}</p>}
              <p>{official?.all ? `${official.scope}累计来自官方；周/月来源见各项标注` : "签到统计仅含 Jolu 已记录数据"}</p>
            </div>
          )}
        </section>
      </Dialog>
    </>
  )
}
