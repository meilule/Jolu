import { cn } from "@/lib/utils"
import { DASH, callStatus, effortLabel, formatMoney, formatSeconds, formatTokens, modelDisplayName, outputTps } from "@/lib/usageStats"
import { convertFromUsd, type FxState } from "@/lib/costCurrency"
import type { StatsCallPageDto, StatsCallRowDto } from "@/lib/api"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import type { PlatformId } from "@/types"
import { Card } from "./usageControls"
import { Button, Pagination, StatusPill } from "@/ui"

/** 画板 34 表头列宽 */
const COLUMNS = [
  ["时间 ↓", "w-[76px]"], ["模型", "w-[188px]"], ["输入", "w-[72px]"], ["缓存", "w-[72px]"], ["输出", "w-[72px]"],
  ["总 Token", "w-[80px]"], ["总成本", "w-[84px]"], ["延迟", "w-[132px]"], ["思考强度", "w-[72px]"], ["状态", "w-[74px]"],
] as const

const sumKnown = (a: number | null, b: number | null) => (a === null && b === null ? null : (a ?? 0) + (b ?? 0))

function time(ts: number): string {
  const d = new Date(ts)
  return [d.getHours(), d.getMinutes(), d.getSeconds()].map((n) => String(n).padStart(2, "0")).join(":")
}

function monthDay(ts: number): string {
  const d = new Date(ts)
  return `${String(d.getMonth() + 1).padStart(2, "0")}/${String(d.getDate()).padStart(2, "0")}`
}

/** 调用明细表格与选中调用的详情（画板 34）。「—」表示未取得或未计价，失败请求不默认免费 */
export function CallsPanel({ data, selectedId, fx, onSelect, onPage, onOpenSession }: {
  data: StatsCallPageDto
  selectedId: number | null
  fx: FxState
  onSelect: (id: number) => void
  onPage: (page: number) => void
  onOpenSession: (target: { platform: string; sessionId: string }) => void
}) {
  const selected = data.rows.find((r) => r.id === selectedId) ?? null
  return (
    <>
      <Card className="flex flex-col gap-1 p-3">
        <div className="flex items-center justify-between px-2 py-1">
          <span className="text-[13px] font-semibold text-jn-text">{data.total_count} 条调用记录</span>
          <span className="text-xs text-jn-text-3">总成本为估算 · {fx.currency} · {data.unpriced_count} 条未计价</span>
        </div>
        {/* 列宽固定且总宽超过容器时横向滚动：内容不再被右侧裁掉 */}
        <div className="overflow-x-auto">
          <div className="flex min-w-[1000px] flex-col gap-0.5">
            <div className="flex h-8 items-center gap-1.5 px-3">
              {COLUMNS.map(([label, width]) => <span key={label} className={cn("shrink-0 text-[11.5px] font-medium text-jn-text-3", width)}>{label}</span>)}
            </div>
            {data.rows.map((row, index) => <CallRow key={row.id} row={row} striped={index % 2 === 0} active={row.id === selectedId} fx={fx} onSelect={() => onSelect(row.id)} />)}
            {!data.rows.length && <div className="px-4 py-8 text-center text-xs text-jn-text-3">没有符合筛选条件的调用</div>}
          </div>
        </div>
        {data.page_count > 1 && (
          <div className="pt-2">
            <Pagination page={data.page} pageCount={data.page_count} onChange={onPage} summary={`共 ${data.total_count} 条 · 每页 ${data.page_size} 条`} />
          </div>
        )}
      </Card>
      {selected && <CallDetail row={selected} fx={fx} onOpenSession={onOpenSession} />}
      <p className="text-[11px] text-jn-text-3" title="导出遵循当前筛选，不包含 API Key 或对话正文。">— 表示未取得或未计价 · 失败请求不默认免费</p>
    </>
  )
}

function Num({ value, className }: { value: number | null; className?: string }) {
  return <span className={cn("tnum text-[12px]", value === null ? "text-jn-text-3" : "text-jn-text", className)}>{formatTokens(value)}</span>
}

function CallRow({ row, striped, active, fx, onSelect }: { row: StatsCallRowDto; striped: boolean; active: boolean; fx: FxState; onSelect: () => void }) {
  const status = callStatus(row)
  const tps = outputTps(row)
  const metric = (label: string, value: string, known: boolean) => (
    <span className="flex h-3.5 items-center gap-1.5">
      <span className="text-[11px] text-jn-text-3">{label}</span>
      <span className={cn("tnum text-[11.5px] font-medium", known ? "text-jn-primary-ink" : "text-jn-text-3")}>{value}</span>
    </span>
  )
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onSelect}
      className={cn(
        "jn-interactive flex h-[54px] w-full items-center gap-1.5 rounded-[12px] border px-3 text-left outline-none [.density-compact_&]:h-[44px]",
        active
          ? "border-jn-primary-soft bg-jn-primary-wash"
          : striped
            ? "border-transparent bg-jn-well hover:bg-jn-primary-wash/50"
            : "border-transparent bg-transparent hover:bg-jn-well",
      )}
    >
      <span className="tnum w-[76px] shrink-0 font-mono text-[11.5px] text-jn-text-2">{time(row.ts)}</span>
      <span className="flex w-[188px] shrink-0 items-center gap-2" title={row.model ?? undefined}>
        <span className="grid size-5 place-items-center"><PlatformLogo platform={row.platform as PlatformId} size={16} /></span>
        <span className="truncate font-mono text-xs font-medium text-jn-text">{modelDisplayName(row.model)}</span>
      </span>
      <Num value={row.fresh_input} className="w-[72px] shrink-0" />
      <Num value={sumKnown(row.cache_read, row.cache_write)} className="w-[72px] shrink-0" />
      <Num value={row.output} className="w-[72px] shrink-0" />
      <Num value={row.total} className="w-[80px] shrink-0 font-semibold" />
      <span className={cn("tnum w-[84px] shrink-0 text-xs", row.cost === null ? "text-jn-text-3" : "font-semibold text-jn-primary-ink")}>{formatMoney(convertFromUsd(row.cost, fx), fx.currency)}</span>
      <span className="flex w-[132px] shrink-0 items-center gap-[7px]">
        <span className="h-10 w-[3px] rounded-full bg-jn-viz-2" />
        <span className="flex flex-col gap-px">
          {metric("首字", formatSeconds(row.first_token_ms), row.first_token_ms !== null)}
          {metric("总耗时", formatSeconds(row.duration_ms), row.duration_ms !== null)}
          {metric("输出 TPS", tps === null ? DASH : `${tps.toFixed(1)} tok/s`, false)}
        </span>
      </span>
      <span className={cn("w-[72px] shrink-0 text-[12px]", row.effort ? "text-jn-text" : "text-jn-text-3")}>{effortLabel(row.effort)}</span>
      <span className="w-[74px] shrink-0">
        <StatusPill tone={status.tone === "ok" ? "success" : status.tone === "error" ? "danger" : "neutral"}>
          {status.label}
        </StatusPill>
      </span>
    </button>
  )
}

function CallDetail({ row, fx, onOpenSession }: { row: StatsCallRowDto; fx: FxState; onOpenSession: (target: { platform: string; sessionId: string }) => void }) {
  const tps = outputTps(row)
  const latencyItems: [string, string, boolean][] = [
    ["首字", formatSeconds(row.first_token_ms), row.first_token_ms !== null],
    ["总耗时", formatSeconds(row.duration_ms), row.duration_ms !== null],
    ["输出 TPS", tps === null ? DASH : `${tps.toFixed(1)} tok/s`, tps !== null],
  ]
  const items: [string, number | null][] = [
    ["普通输入", row.fresh_input], ["输出", row.output], ["缓存读取", row.cache_read], ["缓存写入", row.cache_write], ["合计", row.total],
  ]
  return (
    <Card className="flex flex-col gap-3 p-5">
      <div className="flex items-center gap-3">
        <span className="grid size-9 shrink-0 place-items-center rounded-[12px] bg-jn-well">
          <PlatformLogo platform={row.platform as PlatformId} size={20} />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <h3 className="text-sm font-semibold text-jn-text">调用详情 · {time(row.ts)}</h3>
          <span className="truncate text-xs text-jn-text-3" title={row.call_id}>
            来源：本机日志 · {monthDay(row.ts)} · {row.call_id} · {row.cost === null ? "未计价" : `估算 ${formatMoney(convertFromUsd(row.cost, fx), fx.currency)}`}
          </span>
        </div>
        <Button
          kind="soft"
          size="sm"
          icon="external"
          disabled={!row.session_id}
          onClick={() => row.session_id && onOpenSession({ platform: row.platform, sessionId: row.session_id })}
        >
          打开会话
        </Button>
      </div>
      <div className="grid grid-cols-3 gap-2.5">
        {latencyItems.map(([label, value, known]) => (
          <div key={label} className="flex min-w-0 flex-col gap-1 rounded-[14px] bg-jn-well px-3.5 py-2.5">
            <span className="text-[11px] text-jn-text-3">{label}</span>
            <span className={cn("tnum font-mono text-[15px] font-bold", known ? "text-jn-primary-ink" : "text-jn-text-3")}>{value}</span>
          </div>
        ))}
      </div>
      <div className="grid grid-cols-5 gap-3 rounded-[16px] bg-jn-well p-3.5">
        {items.map(([label, value]) => (
          <div key={label} className="flex min-w-0 flex-col gap-1">
            <span className="text-[11px] text-jn-text-3">{label}</span>
            <span className={cn("tnum font-mono text-base font-bold", value === null ? "text-jn-text-3" : "text-jn-text")}>{formatTokens(value)}</span>
          </div>
        ))}
      </div>
      <CostBreakdown row={row} fx={fx} />
    </Card>
  )
}

/**
 * 单次调用费用拆解（价目与用量对齐展示）。
 * 金额以 `row.cost` 为准；这里只解释"怎么算的"，前端不重算总额，
 * 避免浮点/单位差异造成"明细加起来不等于总额"。
 */
function CostBreakdown({ row, fx }: { row: StatsCallRowDto; fx: FxState }) {
  const p = row.price
  if (!p || row.cost === null) {
    return (
      <div className="rounded-[14px] bg-jn-well px-3.5 py-2.5 text-meta text-jn-text-3">
        {row.model ? `「${row.model}」未收录价目，无法估算费用` : "该记录没有模型信息，无法估算费用"}
      </div>
    )
  }
  // 四项用量 × 生效单价；单价为每百万 token
  const lines: { label: string; tokens: number | null; unit: number }[] = [
    { label: "输入", tokens: row.fresh_input, unit: p.input },
    { label: "输出", tokens: row.output, unit: p.output },
    { label: "缓存读", tokens: row.cache_read, unit: p.cached_input },
    { label: "缓存写", tokens: row.cache_write, unit: p.cache_write ?? 0 },
  ].filter((line) => (line.tokens ?? 0) > 0 && line.unit > 0)

  return (
    <div className="flex flex-col gap-1.5 rounded-[14px] bg-jn-well px-3.5 py-2.5">
      <div className="flex items-center gap-2">
        <span className="text-meta font-medium text-jn-text">费用拆解</span>
        {p.long_context && <Tag tone="warn">长上下文加价</Tag>}
        {p.off_peak && <Tag tone="on">谷时半价</Tag>}
        <span className="flex-1" />
        <span className="text-note text-jn-text-3">{shortSource(p.source)}</span>
      </div>
      {lines.map((line) => (
        <div key={line.label} className="flex items-baseline gap-2 text-note">
          <span className="w-12 shrink-0 text-jn-text-3">{line.label}</span>
          <span className="tnum w-16 shrink-0 text-right text-jn-text">{formatTokens(line.tokens)}</span>
          <span className="text-jn-text-3">×</span>
          <span className="tnum w-20 shrink-0 text-right text-jn-text">${line.unit.toFixed(3)}/M</span>
          <span className="text-jn-text-3">=</span>
          <span className="tnum text-jn-text">{formatMoney(convertFromUsd(((line.tokens ?? 0) * line.unit) / 1_000_000, fx), fx.currency)}</span>
        </div>
      ))}
      <div className="flex items-baseline gap-2 border-t border-jn-line pt-1.5 text-meta">
        <span className="w-12 shrink-0 text-jn-text-3">合计</span>
        <span className="flex-1" />
        <span className="tnum font-semibold text-jn-primary-ink">{formatMoney(convertFromUsd(row.cost, fx), fx.currency)}</span>
      </div>
    </div>
  )
}

function Tag({ tone, children }: { tone: "warn" | "on"; children: React.ReactNode }) {
  return (
    <span className={cn(
      "rounded px-1.5 py-0.5 text-note",
      tone === "warn" ? "bg-jn-danger-soft text-jn-danger" : "bg-jn-primary-wash text-jn-primary-ink",
    )}>
      {children}
    </span>
  )
}

/** 价目来源只显示域名，完整 URL 放 title */
function shortSource(url: string): string {
  const match = url.match(/^https?:\/\/([^/]+)/)
  return match ? match[1] : url
}
