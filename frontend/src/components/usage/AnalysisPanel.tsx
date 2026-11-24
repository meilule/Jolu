/**
 * 用量统计 · 用量分析 —— 设计稿 `docs/ui/jolu-new.pen` R1-23。
 *
 * Token 四分项（`MetricCard` + `DataValue` + `SourceNote`）、全年活跃日历、
 * 模型消耗与估算费用表（斑马纹圆角行）、供应商费用（`BarRow` + 四色系列色）。
 */
import { cn } from "@/lib/utils"
import { formatMoney, formatPercent, formatSeconds, formatTokens, modelDisplayName, modelPriceTip, share } from "@/lib/usageStats"
import { convertFromUsd, type FxState } from "@/lib/costCurrency"
import type { StatsActivityDto, StatsModelRowDto, StatsOverviewDto } from "@/lib/api"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import type { PlatformId } from "@/types"
import { ActivityCalendar } from "./ActivityCalendar"
import { providerName } from "@/lib/providerCatalog"
import { Card, CardTitle } from "./usageControls"
import { MetricGrid } from "../ui/MetricGrid"
import { Icon, type IconName } from "@/icons"
import { BarRow, DataValue, MetricCard, SourceNote, vizSeriesAt } from "@/ui"

export function AnalysisPanel({ data, activity, activityError, fx, onDrillModel, onDrillProvider }: {
  data: StatsOverviewDto
  activityError?: string | null
  activity: StatsActivityDto | null
  fx: FxState
  onDrillModel: (model: StatsModelRowDto) => void
  onDrillProvider: (platform: string) => void
}) {
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

  const models = [...data.models].sort((a, b) => (b.cost ?? -1) - (a.cost ?? -1) || (b.tokens ?? 0) - (a.tokens ?? 0)).slice(0, 6)
  const priced = data.providers.filter((p) => p.cost.priced_requests > 0).sort((a, b) => b.cost.amount - a.cost.amount)
  const total = data.cost.amount
  const totalDisplay = data.cost.priced_requests ? convertFromUsd(total, fx) : null

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

  return (
    <>
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

      <ActivityCalendar activity={activity} error={activityError} />

      <div className="grid min-h-[344px] grid-cols-1 gap-3.5 lg:grid-cols-[minmax(0,644fr)_minmax(0,348fr)]">
        <Card className="flex flex-col gap-2 p-5">
          <CardTitle title="模型消耗与估算费用" note="按费用降序 ↓" />
          <div className="flex h-8 items-center gap-2.5 px-2.5 text-[11.5px] font-medium text-jn-text-3">
            <span className="min-w-0 flex-1">模型</span>
            <span className="w-16 text-right">调用次数</span>
            <span className="w-16 text-right">总 Token</span>
            <span className="w-16 text-right">缓存命中</span>
            <span className="w-16 text-right">平均响应</span>
            <span className="w-20 text-right">估算费用</span>
          </div>
          <div className="flex flex-col gap-1">
            {models.map((m, index) => (
              <button
                key={`${m.platform}-${m.model}`}
                type="button"
                title={modelPriceTip(m)}
                onClick={() => onDrillModel(m)}
                className={cn(
                  "jn-interactive tnum flex h-[42px] items-center gap-2.5 rounded-[12px] px-2.5 text-left text-xs text-jn-text outline-none hover:bg-jn-primary-wash/60",
                  index % 2 === 0 && "bg-jn-well",
                )}
              >
                <span className="flex min-w-0 flex-1 items-center gap-2.5">
                  <span className="grid size-7 shrink-0 place-items-center rounded-[8px] bg-jn-surface">
                    <PlatformLogo platform={m.platform as PlatformId} size={18} />
                  </span>
                  <span className="truncate font-mono text-xs font-medium">{modelDisplayName(m.model)}</span>
                </span>
                <span className="w-16 text-right text-jn-text-2">{m.requests}</span>
                <span className="w-16 text-right text-jn-text-2">{formatTokens(m.tokens)}</span>
                <span className="w-16 text-right text-jn-text-2">{formatPercent(m.cache_hit_rate)}</span>
                <span className="w-16 text-right text-jn-text-2">{formatSeconds(m.avg_duration_ms, true)}</span>
                <span className={cn("w-20 text-right font-semibold", m.cost === null ? "text-jn-text-3" : "text-jn-text")}>
                  {formatMoney(convertFromUsd(m.cost, fx), fx.currency)}
                </span>
              </button>
            ))}
          </div>
          {!models.length && <span className="py-4 text-xs text-jn-text-3">范围内没有模型记录</span>}
          <span className="flex-1" />
          <span className="pt-1 text-[11.5px] text-jn-text-3">未计价显示 — · 平均响应只统计经本地代理的请求</span>
        </Card>

        <Card className="flex flex-col gap-3 p-5">
          <CardTitle title="供应商费用" note={`估算 · ${fx.currency}`} />
          <div className="flex flex-col gap-0.5 rounded-[16px] bg-jn-primary-wash px-4 py-3">
            <span className="text-[11.5px] font-medium text-jn-primary-ink">当前范围合计</span>
            {totalDisplay === null ? (
              <DataValue state="unknown" note="无已计价记录" />
            ) : (
              <DataValue
                state="estimated"
                value={totalDisplay}
                format={(v) => formatMoney(v, fx.currency)}
                note=""
                className="[&>span:nth-child(2)]:text-[26px] [&>span:nth-child(2)]:font-bold [&>span:nth-child(2)]:text-jn-primary-ink"
              />
            )}
          </div>
          <div className="flex flex-col gap-2">
            {priced.map((p, index) => {
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
            {!priced.length && <span className="py-4 text-xs text-jn-text-3">范围内没有可计价的记录</span>}
          </div>
          <span className="flex-1" />
          <SourceNote source="native" coverage="仅汇总已计价记录 · 不含订阅月费" />
        </Card>
      </div>
    </>
  )
}
