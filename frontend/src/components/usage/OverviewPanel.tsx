/**
 * 用量统计 · 概览 —— 设计稿 `docs/ui/jolu-new.pen` R1-04（浅色 / 深色）。
 *
 * 四张指标卡全部经 `MetricCard` + `DataValue` + `SourceNote` 表达，
 * 区分确定读数、零值、估算（≈）、未知；趋势图为四色堆叠柱图，模型分布用 `BarRow`。
 */
import { useState } from "react"
import { cn } from "@/lib/utils"
import { bucketLabel, changeText, formatMoney, formatPercent, formatTokens, modelDisplayName, providerTokenCell, share, unpricedModels, unpricedNote } from "@/lib/usageStats"
import { convertFromUsd, type FxState } from "@/lib/costCurrency"
import { projectName } from "@/lib/sessionThreads"
import type { StatsModelRowDto, StatsOverviewDto, StatsTrendBucketDto } from "@/lib/api"
import { providerName } from "@/lib/providerCatalog"
import { Card, CardTitle } from "./usageControls"
import { TrendChart, TrendLegend, type TrendMode } from "./TrendChart"
import { MetricGrid } from "../ui/MetricGrid"
import { Icon } from "@/icons"
import { BarRow, Button, DataValue, MetricCard, SourceNote, Tabs, vizSeriesAt } from "@/ui"

const MODES: { value: TrendMode; label: string }[] = [
  { value: "tokens", label: "Token" },
  { value: "requests", label: "请求" },
  { value: "cost", label: "费用" },
]

function costNoteTitle(cost: StatsOverviewDto["cost"]): string | undefined {
  const missing = unpricedModels(cost)
  if (missing.length === 0) return undefined
  return `未收录价目的模型（${missing.length}）：\n${missing.join("\n")}`
}

export function OverviewPanel({ data, isToday, fx, onDrillBucket, onDrillModel, onOpenSession }: {
  fx: FxState
  data: StatsOverviewDto
  isToday: boolean
  onDrillBucket: (point: StatsTrendBucketDto) => void
  onDrillModel: (model: StatsModelRowDto) => void
  onOpenSession: (target?: { platform: string; sessionId: string }) => void
}) {
  const [mode, setMode] = useState<TrendMode>("tokens")
  const total = data.tokens.total ?? data.token_coverage.known_total
  const partial = data.tokens.total === null && total !== null
  const cacheRate = data.cache_hit_rate ?? data.token_coverage.cache_hit_rate
  const partialCache = cacheRate !== null && data.token_coverage.cache_requests < data.requests
  const change = partial ? null : changeText(data.tokens.total, data.previous_total)
  const cancelled = data.cancelled ?? 0
  const unknown = data.unknown ?? 0
  const success = data.requests ? (data.requests - data.failed - cancelled - unknown) / data.requests : null
  const topModels = data.models.filter((m) => (m.known_tokens ?? m.tokens) !== 0).slice(0, 5)
  const unattributed = data.requests - data.attributed
  const costAmount = data.cost.priced_requests ? convertFromUsd(data.cost.amount, fx) : null
  const scopeLabel = isToday ? "今天" : "所选范围"

  return (
    <>
      <MetricGrid>
        <MetricCard
          title={
            <span className="inline-flex items-center gap-1.5">
              <Icon name="layers" size="sm" className="text-jn-text-3" />
              {partial ? "已知 Token" : "总 Token"}
            </span>
          }
          scope={scopeLabel}
          value={
            total === null ? (
              <DataValue state="unknown" note="未提供" />
            ) : (
              <DataValue
                state={total === 0 ? "zero" : "value"}
                value={total}
                format={(v) => formatTokens(v)}
                className="[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none"
              />
            )
          }
          compare={
            partial
              ? `${data.token_coverage.known_requests} / ${data.requests} 次有 Token · ${data.requests - data.token_coverage.known_requests} 次未提供`
              : change
                ? `${isToday ? "较昨天同时段" : "较上一周期"} ${change}`
                : "本机汇总统计"
          }
          source={<SourceNote source="native" coverage={partial ? "已知部分小计" : undefined} />}
        />

        <MetricCard
          title={
            <span className="inline-flex items-center gap-1.5">
              <Icon name="activity" size="sm" className="text-jn-text-3" />
              调用次数
            </span>
          }
          scope={scopeLabel}
          value={
            <DataValue
              state={data.requests === 0 ? "zero" : "value"}
              value={data.requests}
              unit="次"
              className="[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none"
            />
          }
          compare={`成功率 ${formatPercent(success)} · ${data.failed} 次失败${cancelled ? ` · ${cancelled} 次已取消` : ""}${unknown ? ` · ${unknown} 次状态未知` : ""}`}
          source={<SourceNote source="mixed" coverage="本机记录与网关流水" />}
        />

        <MetricCard
          title={
            <span className="inline-flex items-center gap-1.5" title={costNoteTitle(data.cost)}>
              <Icon name="coins" size="sm" className="text-jn-text-3" />
              估算费用 · {fx.currency}
            </span>
          }
          scope={scopeLabel}
          value={
            costAmount === null ? (
              <DataValue state="unknown" note="未计价" />
            ) : (
              <DataValue
                state="estimated"
                value={costAmount}
                format={(v) => formatMoney(v, fx.currency)}
                note="估算"
                className="[&>span:nth-child(2)]:text-[28px] [&>span:nth-child(2)]:font-bold [&>span:nth-child(2)]:leading-none"
              />
            )
          }
          compare={<span title={costNoteTitle(data.cost)}>{unpricedNote(data.cost)}</span>}
          source={<SourceNote source="native" coverage="按公开价目推算 · 不含月费" />}
        />

        <MetricCard
          title={
            <span className="inline-flex items-center gap-1.5">
              <Icon name="spark" size="sm" className="text-jn-text-3" />
              输入缓存命中率
            </span>
          }
          scope={scopeLabel}
          value={
            cacheRate === null ? (
              <DataValue state="unknown" note="无输入数据" />
            ) : (
              <DataValue
                state={cacheRate === 0 ? "zero" : "value"}
                value={cacheRate}
                format={(v) => formatPercent(v)}
                className="[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none"
              />
            )
          }
          compare={
            partialCache
              ? `${data.token_coverage.cache_requests} / ${data.requests} 次可计算 · 基于已知输入`
              : "缓存读取 / 全部输入 Token"
          }
          source={<SourceNote source="native" />}
        />
      </MetricGrid>

      <div className="grid min-h-[282px] grid-cols-1 gap-3.5 lg:grid-cols-[minmax(0,644fr)_minmax(0,348fr)]">
        <Card className="flex flex-col gap-3 p-5">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-baseline gap-2">
              <h3 className="text-sm font-semibold text-jn-text">{isToday ? "今日用量" : "用量趋势"}</h3>
              <span className="text-[11px] text-jn-text-3">{bucketLabel(data.trend.bucket)}</span>
            </div>
            <Tabs
              style="segment"
              value={mode}
              onChange={setMode}
              items={MODES.map((m) => ({ key: m.value, label: m.label }))}
            />
          </div>
          <TrendLegend mode={mode} />
          <TrendChart points={data.trend.points} bucket={data.trend.bucket} mode={mode} onDrill={onDrillBucket} />
        </Card>

        <Card className="flex flex-col gap-3 p-5">
          <CardTitle title="模型分布" note={partial ? "按已知 Token" : "按 Token"} />
          <div className="flex flex-col gap-2">
            {topModels.length ? topModels.map((m, index) => {
              const known = m.known_tokens ?? m.tokens
              const ratio = share(known, total)
              const cell = providerTokenCell({
                requests: m.requests,
                tokens: m.tokens,
                known_tokens: known ?? 0,
                known_requests: m.known_requests ?? m.requests,
              })
              return (
                <BarRow
                  key={`${m.platform}-${m.model}`}
                  label={modelDisplayName(m.model)}
                  value={cell.value}
                  percentText={formatPercent(ratio)}
                  ratio={ratio}
                  series={vizSeriesAt(index)}
                  title={m.model ?? undefined}
                  note={cell.note}
                  onClick={() => onDrillModel(m)}
                />
              )
            }) : <span className="py-4 text-xs text-jn-text-3">范围内没有带模型名的记录</span>}
          </div>
        </Card>
      </div>

      <div className="grid grid-cols-1 gap-3.5 lg:grid-cols-2">
        <Card className="flex min-h-[268px] flex-col gap-2 p-5">
          <CardTitle title="供应商用量" note="Token · 调用次数" />
          <div className="flex flex-col gap-1">
            {data.providers.map((p) => {
              const cell = providerTokenCell(p)
              return (
                <div key={p.platform} className="flex min-h-9 items-center gap-2.5 rounded-[12px] px-2 text-xs hover:bg-jn-well">
                  <span className="min-w-0 flex-1 truncate font-medium text-jn-text">{providerName(p.platform)}</span>
                  {cell.note ? (
                    <span className="flex shrink-0 flex-col items-end">
                      <span className="tnum font-semibold leading-3 text-jn-text">{cell.value}</span>
                      <span className="tnum text-note leading-3 text-jn-text-3">{cell.note}</span>
                    </span>
                  ) : (
                    <span className="tnum font-semibold text-jn-text">{cell.value}</span>
                  )}
                  <span className="tnum w-16 text-right text-xs text-jn-text-3">{p.requests} 次</span>
                </div>
              )
            })}
          </div>
          <span className="flex-1" />
          <SourceNote
            source="mixed"
            coverage={`${data.attributed} / ${data.requests} 次可归属账号 · ${unattributed} 次未归属`}
          />
        </Card>

        <Card className="flex min-h-[268px] flex-col gap-2 p-5">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-jn-text">高消耗会话</h3>
            <Button kind="soft" size="sm" icon="arrowRight" onClick={() => onOpenSession()}>
              查看会话
            </Button>
          </div>
          <div className="flex flex-col gap-1">
            {data.sessions.length ? data.sessions.map((s, index) => (
              <button
                key={`${s.platform}-${s.session_id}`}
                type="button"
                onClick={() => onOpenSession({ platform: s.platform, sessionId: s.session_id })}
                className="jn-interactive flex min-h-11 items-center gap-2.5 rounded-[14px] px-2 py-1.5 text-left outline-none hover:bg-jn-well"
              >
                <span className={cn("tnum grid size-7 shrink-0 place-items-center rounded-[10px] bg-jn-well font-mono text-[11px] font-bold", index === 0 ? "text-jn-primary-ink" : "text-jn-text-3")}>
                  {String(index + 1).padStart(2, "0")}
                </span>
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="truncate text-xs font-semibold text-jn-text">{s.title ?? `会话 ${s.session_id.slice(0, 8)}`}</span>
                  <span className="truncate text-[11px] text-jn-text-3">{s.cwd ? `${projectName(s.cwd)} · ${providerName(s.platform)}` : providerName(s.platform)}</span>
                </span>
                <span className="tnum text-xs font-semibold text-jn-text" title={s.unknown_requests ? `${s.unknown_requests} 次未提供 Token，只计已知部分` : undefined}>
                  {s.unknown_requests ? `已知 ${formatTokens(s.tokens)}` : formatTokens(s.tokens)}
                </span>
                <Icon name="next" size="sm" className="text-jn-text-3" />
              </button>
            )) : <span className="py-2 text-xs text-jn-text-3">范围内没有可归入会话的记录</span>}
          </div>
        </Card>
      </div>
      <p className="text-xs text-jn-text-3" title="缓存四项按互斥口径统计；费用仅汇总已计价记录。">本机统计，不等于服务方账单</p>
    </>
  )
}
