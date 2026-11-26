/**
 * TrendChart / TrendLegend —— 设计稿 `docs/ui/jolu-new.pen` R1-04（用量统计 · 概览）。
 *
 * 按画板做**堆叠柱图**：
 * - Token 模式下按 `jn-viz-1..4` 堆叠（输入、输出、缓存读取、缓存写入）；
 * - 请求 / 费用模式下为单柱（`jn-viz-1`）；
 * - 悬停某柱时，其余柱压暗到 32% 不透明，并弹出 `ChartTooltip`；
 * - 点击柱子下钻到对应时间段的调用明细。
 */
import { useState } from "react"
import { cn } from "@/lib/utils"
import { axisTicks, labelEvery } from "@/lib/chartGeometry"
import { convertFromUsd, type FxState } from "@/lib/costCurrency"
import { formatMoney, formatPercent, formatTokens } from "@/lib/usageStats"
import type { StatsTrendBucketDto } from "@/lib/api"
import { ChartTooltip, Legend, VIZ_BG, type ChartTooltipRow, type VizSeries } from "@/ui"

export type TrendMode = "tokens" | "requests" | "cost" | "cache_hit_rate"

const DEFAULT_FX: FxState = { currency: "USD", rate: 1, asOf: "", fallback: false, fixed: false, loading: false }

const TOKEN_SERIES: readonly { key: "fresh_input" | "output" | "cache_read" | "cache_write"; label: string; series: VizSeries }[] = [
  { key: "fresh_input", label: "输入", series: 1 },
  { key: "output", label: "输出", series: 2 },
  { key: "cache_read", label: "缓存读取", series: 3 },
  { key: "cache_write", label: "缓存写入", series: 4 },
]

/** 轴刻度去掉多余小数：1 M、500 K、2.5 M、100% */
function axisLabel(value: number, mode: TrendMode, fx: FxState): string {
  if (mode === "cache_hit_rate") return `${Math.round(value)}%`
  if (mode === "cost") {
    if (value === 0) return "0"
    const symbol = fx.currency === "CNY" ? "¥" : "$"
    return `${symbol}${Number(value.toFixed(2))}`
  }
  if (mode === "requests") return String(Math.round(value))
  if (value >= 1_000_000) return `${Number((value / 1_000_000).toFixed(1))} M`
  if (value >= 1_000) return `${Number((value / 1_000).toFixed(1))} K`
  return String(Math.round(value))
}

function tooltipTitle(point: StatsTrendBucketDto, bucket: "1h" | "2h" | "day" | "week"): string {
  const d = new Date(point.start)
  const day = `${d.getMonth() + 1} 月 ${d.getDate()} 日`
  if (bucket === "1h" || bucket === "2h") return `${day} · ${point.label}`
  return bucket === "week" ? `${day}起一周` : day
}

function pointTotal(point: StatsTrendBucketDto, mode: TrendMode, fx: FxState): number {
  if (mode === "requests") return point.requests
  if (mode === "cost") return convertFromUsd(point.cost ?? 0, fx) ?? 0
  if (mode === "cache_hit_rate") {
    return point.cache_hit_rate !== null && point.cache_hit_rate !== undefined ? point.cache_hit_rate * 100 : 0
  }
  return (point.fresh_input ?? 0) + (point.output ?? 0) + (point.cache_read ?? 0) + (point.cache_write ?? 0)
}

export function TrendLegend({ mode }: { mode: TrendMode }) {
  const items =
    mode === "tokens"
      ? TOKEN_SERIES.map((s) => ({ key: s.key, label: s.label, series: s.series }))
      : [
          {
            key: mode,
            label: mode === "requests" ? "请求次数" : mode === "cost" ? "估算费用" : "输入缓存命中率",
            series: 1 as VizSeries,
          },
        ]
  return <Legend items={items} />
}

export function TrendChart({ points, bucket, mode, fx = DEFAULT_FX, onDrill }: {
  points: StatsTrendBucketDto[]
  bucket: "1h" | "2h" | "day" | "week"
  mode: TrendMode
  fx?: FxState
  onDrill: (point: StatsTrendBucketDto) => void
}) {
  const [hover, setHover] = useState(-1)
  const totals = points.map((p) => pointTotal(p, mode, fx))
  const maxVal = Math.max(0, ...totals)
  const ticks = mode === "cache_hit_rate" ? [100, 50, 0] : axisTicks(maxVal, 3)
  const top = ticks[0] || 1
  const every = labelEvery(points.length, 7)
  const hovered = hover >= 0 ? points[hover] ?? null : null

  const tooltipRows: ChartTooltipRow[] = hovered
    ? mode === "tokens"
      ? [
          { label: "普通输入", value: formatTokens(hovered.fresh_input), series: 1 },
          { label: "输出", value: formatTokens(hovered.output), series: 2 },
          { label: "缓存读取", value: formatTokens(hovered.cache_read), series: 3 },
          { label: "缓存写入", value: formatTokens(hovered.cache_write), series: 4 },
        ]
      : mode === "cache_hit_rate"
        ? [
            { label: "缓存命中率", value: formatPercent(hovered.cache_hit_rate), series: 1 },
            { label: "缓存读取", value: formatTokens(hovered.cache_read), series: 3 },
            { label: "普通输入", value: formatTokens(hovered.fresh_input), series: 1 },
            { label: "调用次数", value: `${hovered.requests} 次`, series: 2 },
          ]
        : [
            {
              label: mode === "requests" ? "请求次数" : `估算费用 (${fx.currency})`,
              value: mode === "requests" ? `${hovered.requests} 次` : formatMoney(convertFromUsd(hovered.cost, fx), fx.currency),
              series: 1,
            },
          ]
    : []

  const unknownCount = hovered?.unknown_requests ?? 0
  const tooltipFooter = hovered
    ? mode === "tokens"
      ? unknownCount > 0
        ? `已知 Token · ${formatTokens(hovered.total)} · 含 ${unknownCount} 条未提供分项`
        : `总 Token · ${formatTokens(hovered.total)}`
      : unknownCount > 0
        ? `点击柱子查看明细 · 含 ${unknownCount} 条未提供分项`
        : "点击柱子查看此时段调用明细"
    : undefined

  return (
    <div
      className="relative flex h-[170px] w-full flex-col select-none"
      onMouseLeave={() => setHover(-1)}
    >
      {/* 主绘图区（左侧刻度 + 右侧柱区） */}
      <div className="relative flex min-h-0 flex-1 items-stretch">
        {/* 网格线与 Y 轴刻度 */}
        <div className="pointer-events-none absolute inset-0 flex flex-col justify-between">
          {ticks.map((tick) => (
            <div key={tick} className="flex items-center gap-2">
              <span className="tnum w-10 shrink-0 text-right font-mono text-[10px] text-jn-text-3">
                {axisLabel(tick, mode, fx)}
              </span>
              <span className="h-px flex-1 bg-jn-viz-grid" />
            </div>
          ))}
        </div>

        {/* 堆叠柱组 */}
        <div
          role="img"
          aria-label="用量趋势柱图"
          className="relative z-[1] ml-12 flex flex-1 items-end gap-1.5 pt-2 pb-1"
        >
          {points.map((point, index) => {
            const sum = totals[index]
            const heightPct = top > 0 && sum > 0 ? Math.max(3, (sum / top) * 100) : 0
            const dimmed = hover >= 0 && hover !== index
            const active = hover === index

            const segments =
              mode === "tokens"
                ? TOKEN_SERIES.map((s) => ({
                    key: s.key,
                    series: s.series,
                    value: point[s.key] ?? 0,
                  })).filter((seg) => seg.value > 0)
                : sum > 0
                  ? [{ key: mode, series: 1 as VizSeries, value: sum }]
                  : []

            return (
              <button
                key={point.start}
                type="button"
                aria-label={`${tooltipTitle(point, bucket)}：${
                  mode === "requests"
                    ? `${point.requests} 次`
                    : mode === "cost"
                      ? formatMoney(convertFromUsd(point.cost, fx), fx.currency)
                      : mode === "cache_hit_rate"
                        ? formatPercent(point.cache_hit_rate)
                        : formatTokens(point.total)
                }`}
                onMouseEnter={() => setHover(index)}
                onFocus={() => setHover(index)}
                onClick={() => onDrill(point)}
                className="group relative flex h-full min-w-0 flex-1 cursor-pointer flex-col justify-end outline-none"
              >
                <span
                  className={cn(
                    "mx-auto flex w-full max-w-[34px] flex-col-reverse overflow-hidden rounded-t-[6px] transition-[opacity,transform] duration-[var(--motion-feedback)]",
                    dimmed && "opacity-30",
                    active && "brightness-[1.04]",
                  )}
                  style={{ height: `${heightPct}%` }}
                >
                  {segments.map((seg) => (
                    <span
                      key={seg.key}
                      className={cn("w-full shrink-0", VIZ_BG[seg.series])}
                      style={{ height: `${sum > 0 ? (seg.value / sum) * 100 : 0}%` }}
                    />
                  ))}
                </span>
              </button>
            )
          })}
        </div>
      </div>

      {/* X 轴标签 */}
      <div className="ml-12 flex h-5 items-center gap-1.5 pt-1">
        {points.map((p, index) => (
          <span
            key={p.start}
            className="tnum min-w-0 flex-1 truncate text-center font-mono text-[10px] text-jn-text-3"
          >
            {index % every === 0 ? p.label : ""}
          </span>
        ))}
      </div>

      {/* 悬停气泡提示（R1-04 / R1-C ChartTooltip） */}
      {hovered && (
        <ChartTooltip
          title={tooltipTitle(hovered, bucket)}
          rows={tooltipRows}
          footer={tooltipFooter}
          className="absolute top-1 z-10"
          style={
            hover >= points.length / 2
              ? { left: 56 }
              : { right: 8 }
          }
        />
      )}
    </div>
  )
}
