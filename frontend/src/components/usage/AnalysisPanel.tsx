/**
 * 用量统计 · 用量分析 —— 设计稿 `docs/ui/jolu-new.pen` R1-23 / R1-48。
 *
 * 顶部条件显隐的预算水位带、Token 四分项、支持费用着色的全年活跃日历、
 * 全量模型消耗表（含倍率微标与一键补价）、以及支持「按连接 / 按平台」切换的费用归属与官方实账/积分对照卡。
 */
import { useState } from "react"
import { cn } from "@/lib/utils"
import { formatMoney, formatPercent, formatSeconds, formatTokens, modelDisplayName, modelPriceTip, share } from "@/lib/usageStats"
import { convertFromUsd, type FxState } from "@/lib/costCurrency"
import type { BudgetStatusDto, StatsActivityDto, StatsModelRowDto, StatsOverviewDto } from "@/lib/api"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import type { PlatformId } from "@/types"
import { ActivityCalendar } from "./ActivityCalendar"
import { providerName } from "@/lib/providerCatalog"
import { Card } from "./usageControls"
import { MetricGrid } from "../ui/MetricGrid"
import { Icon, type IconName } from "@/icons"
import { BarRow, Button, DataValue, MetricCard, SourceNote, StatusPill, Tabs, VIZ_BG, vizSeriesAt } from "@/ui"

type CostGroupBy = "connection" | "platform"

function relativeSync(ms?: number | null): string {
  if (!ms) return ""
  const sec = Math.max(0, Math.round((Date.now() - ms) / 1000))
  if (sec < 60) return "刚刚"
  if (sec < 3600) return `${Math.floor(sec / 60)} 分钟前`
  if (sec < 86400) return `${Math.floor(sec / 3600)} 小时前`
  return `${Math.floor(sec / 86400)} 天前`
}

function formatCredits(v: number): string {
  return Number.isInteger(v) ? String(v) : v.toFixed(1).replace(/\.0$/, "")
}

export function AnalysisPanel({
  data,
  activity,
  activityError,
  fx,
  budgetStatus,
  billingScope,
  onDrillModel,
  onDrillProvider,
  onDrillConnection,
  onOpenBudgets,
  onOpenPricingForModel,
}: {
  data: StatsOverviewDto
  activityError?: string | null
  activity: StatsActivityDto | null
  fx: FxState
  budgetStatus?: BudgetStatusDto | null
  billingScope: "all" | "api" | "subscription"
  onDrillModel: (model: StatsModelRowDto) => void
  onDrillProvider: (platform: string) => void
  onDrillConnection: (connectionId: string | null) => void
  onOpenBudgets: () => void
  onOpenPricingForModel: (modelName: string) => void
}) {
  const [groupBy, setGroupBy] = useState<CostGroupBy>("connection")
  const t = data.tokens
  const part = (value: number | null, note: string) => `${formatPercent(share(value, t.total))} · ${note}`
  const coverage = (
    value: number | null,
    known: number | null | undefined,
    unknown: number | undefined,
    note: string,
  ) => {
    if (value != null) return part(value, note)
    if (known == null || known === 0) return "该范围内没有可汇总的已知值"
    return `已知部分小计 · ${unknown ?? 0} 条来源未提供`
  }

  const allModels = [...data.models].sort(
    (a, b) => (b.cost ?? -1) - (a.cost ?? -1) || (b.known_tokens ?? b.tokens ?? 0) - (a.known_tokens ?? a.tokens ?? 0),
  )
  const pricedProviders = data.providers.filter((p) => p.cost.priced_requests > 0).sort((a, b) => b.cost.amount - a.cost.amount)
  const connectionRows = data.connections ?? []
  const pricedConnections = connectionRows.filter((c) => c.priced_requests > 0 || c.credits > 0)
  const total = data.cost.amount
  const totalDisplay = data.cost.priced_requests ? convertFromUsd(total, fx) : null
  const creditsTotal = data.cost.credits ?? 0
  const officialUsd = data.cost.official_cost_usd ?? null
  const officialDiffPct = officialUsd !== null && total > 0 ? ((officialUsd - total) / total) * 100 : null

  const subRefAmount = billingScope === "api"
    ? (data.cost.subscription_ref_amount ?? 0)
    : (data.cost.subscription_amount ?? 0)
  const subRefCredits = billingScope === "api"
    ? (data.cost.subscription_ref_credits ?? 0)
    : 0
  const showSubRef = billingScope !== "subscription" && (subRefAmount > 0 || subRefCredits > 0)

  const scopeSuffix = billingScope === "api" ? "仅实付 API" : billingScope === "subscription" ? "仅订阅等价" : "全部口径"

  const tokenCards: {
    icon: IconName
    label: string
    scope: string
    exact: number | null
    known: number | null | undefined
    unknown: number | undefined
    note: string
  }[] = [
    { icon: "download", label: "未缓存输入", scope: "普通输入", exact: t.fresh_input, known: t.fresh_input_known, unknown: t.fresh_input_unknown, note: "占总 Token" },
    { icon: "upload", label: "输出 Token", scope: "模型生成", exact: t.output, known: t.output_known, unknown: t.output_unknown, note: "占总 Token" },
    { icon: "database", label: "缓存读取", scope: "已复用输入", exact: t.cache_read, known: t.cache_read_known, unknown: t.cache_read_unknown, note: "已复用" },
    { icon: "layers", label: "缓存写入", scope: "新建缓存", exact: t.cache_write, known: t.cache_write_known, unknown: t.cache_write_unknown, note: "新建缓存" },
  ]

  const budgetByConn = new Map((budgetStatus?.connections ?? []).map((b) => [b.connection_id, b]))

  return (
    <>
      {budgetStatus?.has_any_budget && (
        <BudgetWatermarkBar status={budgetStatus} fx={fx} onManage={onOpenBudgets} />
      )}

      <MetricGrid>
        {tokenCards.map((item) => {
          const shown = item.exact ?? item.known ?? null
          const isPartial = item.exact === null && shown !== null
          return (
            <MetricCard
              key={item.label}
              title={
                <span className="inline-flex items-center gap-1.5">
                  <Icon name={item.icon} size="sm" className="text-jn-text-3" />
                  {item.label}
                </span>
              }
              scope={item.scope}
              value={
                shown === null ? (
                  <DataValue state="unknown" note="未提供" />
                ) : (
                  <DataValue
                    state={shown === 0 ? "zero" : "value"}
                    value={shown}
                    format={(v) => formatTokens(v)}
                    className="[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none"
                  />
                )
              }
              compare={
                <span className={cn(isPartial && "text-jn-warning")}>
                  {coverage(item.exact, item.known, item.unknown, item.note)}
                </span>
              }
              source={<SourceNote source="native" coverage={isPartial ? "已知部分小计" : undefined} />}
            />
          )
        })}
      </MetricGrid>

      {(t.total == null && data.requests > 0) && (
        <p className="text-xs text-jn-text-3">
          {`当前范围内 ${data.requests} 次调用中，部分来源未提供完整分项，卡片显示的是已知部分小计；模型消耗表按各来源已知值汇总。`}
        </p>
      )}

      <ActivityCalendar activity={activity} error={activityError} fx={fx} />

      <div className="grid min-h-[344px] grid-cols-1 gap-3.5 lg:grid-cols-[minmax(0,620fr)_minmax(0,396fr)]">
        <Card className="flex flex-col gap-2 p-5">
          <div className="flex items-center justify-between gap-2">
            <span className="inline-flex items-center gap-2 text-sm font-semibold text-jn-text">
              <Icon name="cpu" size="sm" className="text-jn-text-2" />
              模型消耗与估算费用
            </span>
            <span className="text-xs text-jn-text-3">按费用降序 ↓ · 共 {allModels.length} 个模型</span>
          </div>
          <div className="flex h-8 items-center gap-2.5 px-2.5 text-[11.5px] font-medium text-jn-text-3">
            <span className="min-w-0 flex-1">模型</span>
            <span className="w-16 text-right">调用次数</span>
            <span className="w-16 text-right">总 Token</span>
            <span className="w-16 text-right">缓存命中</span>
            <span className="w-16 text-right">平均响应</span>
            <span className="w-20 text-right">估算费用</span>
          </div>
          <div className="flex max-h-[360px] flex-col gap-1 overflow-y-auto pr-0.5">
            {allModels.map((m, index) => (
              <div
                key={`${m.platform}-${m.model}`}
                title={modelPriceTip(m)}
                className={cn(
                  "jn-interactive tnum flex min-h-[42px] shrink-0 items-center gap-2.5 rounded-[12px] px-2.5 text-xs text-jn-text hover:bg-jn-primary-wash/60",
                  index % 2 === 0 && "bg-jn-well",
                )}
              >
                <button
                  type="button"
                  onClick={() => onDrillModel(m)}
                  className="flex min-w-0 flex-1 cursor-pointer items-center gap-2.5 text-left outline-none"
                >
                  <span className="flex min-w-0 flex-1 items-center gap-2">
                    <span className="grid size-7 shrink-0 place-items-center rounded-[8px] bg-jn-surface">
                      <PlatformLogo platform={m.platform as PlatformId} size={18} />
                    </span>
                    <span className="truncate font-mono text-xs font-medium">{modelDisplayName(m.model)}</span>
                    {m.multiplier_hint !== undefined && m.multiplier_hint !== null && Math.abs(m.multiplier_hint - 1) > 1e-4 && (
                      <span className="shrink-0 rounded-full bg-jn-primary-wash px-1.5 py-0.5 font-mono text-[10px] font-semibold text-jn-primary-ink">
                        {m.multiplier_hint}×
                      </span>
                    )}
                  </span>
                  <span className="w-16 text-right text-jn-text-2">{m.requests}</span>
                  <span className="w-16 text-right text-jn-text-2">{formatTokens(m.tokens ?? (m.known_requests ? m.known_tokens : null))}</span>
                  <span className="w-16 text-right text-jn-text-2">{formatPercent(m.cache_hit_rate)}</span>
                  <span className="w-16 text-right text-jn-text-2">{formatSeconds(m.avg_duration_ms, true)}</span>
                </button>
                <span className={cn("flex w-20 flex-col items-end font-semibold", m.cost === null ? "text-jn-text-3" : "text-jn-text")}>
                  {m.cost === null && m.model ? (
                    <button
                      type="button"
                      onClick={() => onOpenPricingForModel(m.model!)}
                      className="jn-interactive cursor-pointer rounded-full bg-jn-primary-wash px-2.5 py-0.5 text-[11px] font-semibold text-jn-primary-ink hover:bg-jn-primary-soft"
                    >
                      去补价
                    </button>
                  ) : (
                    <button
                      type="button"
                      onClick={() => onDrillModel(m)}
                      className="flex cursor-pointer flex-col items-end outline-none"
                    >
                      <span>{formatMoney(convertFromUsd(m.cost, fx), fx.currency)}</span>
                      {m.cost !== null && m.priced_requests !== undefined && m.priced_requests < m.requests && (
                        <span className="text-[10px] font-normal text-jn-text-3">{m.priced_requests}/{m.requests}</span>
                      )}
                    </button>
                  )}
                </span>
              </div>
            ))}
          </div>
          {!allModels.length && <span className="py-4 text-xs text-jn-text-3">范围内没有模型记录</span>}
          <span className="flex-1" />
          <span className="pt-1 text-[11.5px] text-jn-text-3">未计价模型可一键补价或映射 · 悬停查看价目出处与覆盖条数</span>
        </Card>

        <Card className="flex flex-col gap-3 p-5">
          <div className="flex items-center justify-between gap-2">
            <span className="inline-flex items-center gap-2 text-sm font-semibold text-jn-text">
              <Icon name="wallet" size="sm" className="text-jn-text-2" />
              费用归属
            </span>
            <Tabs
              style="segment"
              value={groupBy}
              onChange={setGroupBy}
              items={[
                { key: "connection", label: "按连接" },
                { key: "platform", label: "按平台" },
              ]}
            />
          </div>

          <div className="flex flex-col gap-1 rounded-[16px] bg-jn-primary-wash px-4 py-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="text-[11.5px] font-medium text-jn-primary-ink">当前范围合计 · {scopeSuffix}</span>
              {officialUsd !== null && (
                <span
                  className="inline-flex items-center gap-1.5 rounded-full bg-jn-surface px-2.5 py-0.5 text-[10.5px] font-semibold text-jn-text"
                  title={data.cost.official_synced_ms ? `官方 Admin API 实账快照，同步于 ${relativeSync(data.cost.official_synced_ms)}` : "官方 Admin API 实账快照"}
                >
                  <span className="size-1.5 rounded-full bg-jn-info" />
                  官方实账 {formatMoney(convertFromUsd(officialUsd, fx), fx.currency)}
                  {officialDiffPct !== null && (
                    <span className="font-mono text-jn-text-3">({officialDiffPct >= 0 ? "+" : ""}{officialDiffPct.toFixed(1)}%)</span>
                  )}
                </span>
              )}
            </div>
            {totalDisplay === null ? (
              <DataValue state="unknown" note="无已计价记录" />
            ) : (
              <DataValue
                state="estimated"
                value={totalDisplay}
                format={(v) => formatMoney(v, fx.currency)}
                note={fx.currency}
                className="[&>span:nth-child(2)]:text-[26px] [&>span:nth-child(2)]:font-bold [&>span:nth-child(2)]:text-jn-primary-ink"
              />
            )}
            {creditsTotal > 0 && (
              <span className="text-[11px] text-jn-text-2">
                另含 {formatCredits(creditsTotal)} 积分 · 独立计量不折算美元
              </span>
            )}
          </div>

          {groupBy === "connection" ? (
            <div className="flex max-h-[260px] flex-col gap-2.5 overflow-y-auto pr-0.5">
              {pricedConnections.map((c, index) => {
                const ratio = share(c.cost, total)
                const bInfo = c.connection_id ? budgetByConn.get(c.connection_id) : undefined
                const modeTag = c.billing_mode === "free"
                  ? "免费"
                  : c.billing_mode === "subscription"
                    ? "订阅等价"
                    : Math.abs(c.price_multiplier - 1) > 1e-4
                      ? `API · ${c.price_multiplier}×`
                      : c.connection_id
                        ? "API"
                        : ""
                const subNote = c.official_cost_usd != null
                  ? `官方实账 ${formatMoney(convertFromUsd(c.official_cost_usd, fx), fx.currency)}${c.official_synced_ms ? ` · 同步于 ${relativeSync(c.official_synced_ms)}` : ""}`
                  : bInfo && (bInfo.daily_limit || bInfo.monthly_limit)
                    ? bInfo.daily_limit
                      ? `日预算 ${formatMoney(convertFromUsd(bInfo.today_spend, fx), fx.currency)} / ${formatMoney(convertFromUsd(bInfo.daily_limit, fx), fx.currency)}${bInfo.exceeded ? " · 已超限" : " · 正常"}`
                      : `月预算 ${formatMoney(convertFromUsd(bInfo.month_spend, fx), fx.currency)} / ${formatMoney(convertFromUsd(bInfo.monthly_limit!, fx), fx.currency)}${bInfo.exceeded ? " · 已超限" : " · 正常"}`
                    : c.credits > 0
                      ? `已消耗 ${formatCredits(c.credits)} 积分`
                      : !c.connection_id
                        ? "未绑定连接 · 按官方基准价估算"
                        : null
                const barClass = c.connection_id ? VIZ_BG[vizSeriesAt(index)] : "bg-jn-viz-other"
                return (
                  <button
                    key={c.connection_id ?? "__unattributed__"}
                    type="button"
                    onClick={() => onDrillConnection(c.connection_id)}
                    className="jn-interactive flex w-full flex-col gap-1 rounded-[12px] p-1.5 text-left outline-none hover:bg-jn-well"
                  >
                    <div className="flex items-center gap-1.5 text-xs">
                      <span className="truncate font-semibold text-jn-text">{c.name}</span>
                      {modeTag && (
                        <span className="shrink-0 rounded-full bg-jn-well px-1.5 py-0.5 font-mono text-[10px] font-semibold text-jn-text-2">
                          {modeTag}
                        </span>
                      )}
                      <span className="min-w-0 flex-1" />
                      <span className="tnum font-mono font-semibold text-jn-text">
                        {formatMoney(convertFromUsd(c.cost, fx), fx.currency)}
                      </span>
                      <span className="tnum w-10 text-right font-mono text-[11px] text-jn-text-3">
                        {formatPercent(ratio)}
                      </span>
                    </div>
                    <div className="h-1.5 w-full overflow-hidden rounded-full bg-jn-well">
                      <div
                        className={cn("h-full rounded-full transition-all", barClass)}
                        style={{ width: `${Math.max(ratio && ratio > 0 ? 2 : 0, Math.min(100, (ratio ?? 0) * 100))}%` }}
                      />
                    </div>
                    {subNote && <span className="text-[11px] text-jn-text-3">{subNote}</span>}
                  </button>
                )
              })}
              {!pricedConnections.length && <span className="py-4 text-xs text-jn-text-3">范围内没有可计价的连接记录</span>}
            </div>
          ) : (
            <div className="flex flex-col gap-2">
              {pricedProviders.map((p, index) => {
                const ratio = share(p.cost.amount, total)
                return (
                  <BarRow
                    key={p.platform}
                    label={
                      <span className="inline-flex items-center gap-2">
                        <span className="grid size-5 shrink-0 place-items-center rounded-[6px] bg-jn-well">
                          <PlatformLogo platform={p.platform as PlatformId} size={14} />
                        </span>
                        <span className="truncate">{providerName(p.platform)}</span>
                      </span>
                    }
                    value={formatMoney(convertFromUsd(p.cost.amount, fx), fx.currency)}
                    percentText={formatPercent(ratio)}
                    ratio={ratio}
                    series={vizSeriesAt(index)}
                    onClick={() => onDrillProvider(p.platform)}
                  />
                )
              })}
              {!pricedProviders.length && <span className="py-4 text-xs text-jn-text-3">范围内没有可计价的记录</span>}
            </div>
          )}

          {showSubRef && (
            <div className="flex items-center justify-between gap-2 border-t border-jn-line pt-2 text-[11.5px]">
              <span className="text-jn-text-3">订阅参考（不计入实付）</span>
              <span className="tnum font-mono font-semibold text-jn-text-2">
                {subRefAmount > 0 ? `等价 ≈ ${formatMoney(convertFromUsd(subRefAmount, fx), fx.currency)}` : ""}
                {subRefAmount > 0 && subRefCredits > 0 ? " · " : ""}
                {subRefCredits > 0 ? `${formatCredits(subRefCredits)} 积分` : ""}
              </span>
            </div>
          )}

          <span className="flex-1" />
          <SourceNote source="mixed" coverage="估算、官方实账与积分分列展示 · 不含订阅月费" />
        </Card>
      </div>
    </>
  )
}

function BudgetWatermarkBar({ status, fx, onManage }: {
  status: BudgetStatusDto
  fx: FxState
  onManage: () => void
}) {
  const useMonthly = status.global_enabled && status.global_monthly_limit !== null
  const useDaily = status.global_enabled && !useMonthly && status.global_daily_limit !== null
  const title = useMonthly
    ? "本月预算 · 全局实付 API"
    : useDaily
      ? "今日预算 · 全局实付 API"
      : "连接预算监控 · 仅实付 API"
  const spent = useDaily ? status.global_today_spend : status.global_month_spend
  const limit = useMonthly ? status.global_monthly_limit : useDaily ? status.global_daily_limit : null
  const pct = limit && limit > 0 ? (spent / limit) * 100 : null
  const pillTone = status.global_exceeded || status.exceeded_connections > 0
    ? "danger"
    : status.global_warned
      ? "warning"
      : "success"
  const pillLabel = status.global_exceeded
    ? "全局已超限"
    : status.exceeded_connections > 0
      ? `${status.exceeded_connections} 个连接超限`
      : status.global_warned
        ? `超过 ${status.warn_percent}% 预警`
        : `预警线 ${status.warn_percent}%`

  return (
    <Card className="flex flex-col gap-2.5 px-5 py-3.5">
      <div className="flex flex-wrap items-center gap-2">
        <Icon name="gauge" size="sm" className="text-jn-primary" />
        <span className="text-[13.5px] font-semibold text-jn-text">{title}</span>
        <StatusPill tone={pillTone}>{pillLabel}</StatusPill>
        <span className="min-w-0 flex-1" />
        {limit !== null && (
          <span className="tnum inline-flex items-baseline gap-1.5 font-mono">
            <span className="text-sm font-bold text-jn-text">{formatMoney(convertFromUsd(spent, fx), fx.currency)}</span>
            <span className="text-xs text-jn-text-3">/ {formatMoney(convertFromUsd(limit, fx), fx.currency)}</span>
            {pct !== null && (
              <span className={cn("text-xs font-semibold", status.global_exceeded ? "text-jn-danger" : status.global_warned ? "text-jn-warning" : "text-jn-primary-ink")}>
                {pct.toFixed(1)}%
              </span>
            )}
          </span>
        )}
        <Button kind="soft" size="sm" icon="sliders" onClick={onManage}>管理</Button>
      </div>
      <div className="flex flex-wrap items-center gap-3.5">
        <div className="relative h-2 min-w-[200px] flex-1 overflow-hidden rounded-full bg-jn-well">
          {pct !== null && (
            <div
              className={cn(
                "h-full rounded-full transition-all",
                status.global_exceeded ? "bg-jn-danger" : status.global_warned ? "bg-jn-warning" : "bg-jn-primary",
              )}
              style={{ width: `${Math.min(100, Math.max(pct > 0 ? 2 : 0, pct))}%` }}
            />
          )}
          {limit !== null && (
            <span
              aria-hidden
              title={`预警线 ${status.warn_percent}%`}
              className="absolute top-0 bottom-0 w-0.5 bg-jn-warning"
              style={{ left: `${status.warn_percent}%` }}
            />
          )}
        </div>
        <span className="text-[11.5px] text-jn-text-3">
          {status.projected_month_spend !== null
            ? `预计月底 ${formatMoney(convertFromUsd(status.projected_month_spend, fx), fx.currency)} · `
            : ""}
          连接超限 {status.exceeded_connections} 个 · 不含订阅等价
        </span>
      </div>
    </Card>
  )
}
