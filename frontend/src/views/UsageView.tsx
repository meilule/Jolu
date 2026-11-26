import { ContentTransition } from "@/components/ui/LoadingSurface"
import { useEffect, useState } from "react"
import { CalendarDays } from "lucide-react"
import { api, isTauri, type StatsCallPageDto, type StatsFilterDto, type StatsModelRowDto, type StatsTrendBucketDto } from "@/lib/api"
import { collectionWarning, dateKey, localZone, rangeLabel, rangeToMillis, todayRange, utcOffsetLabel, zoneName, type DateRange } from "@/lib/usageStats"
import { useCostFx } from "@/lib/costCurrency"
import { UNATTRIBUTED_ACCOUNT, accountsFor, keepApplicable } from "@/lib/usageFilters"
import { useConnections } from "@/hooks/useConnections"
import { useSettings } from "@/hooks/useSettings"
import { useStatsQuery } from "@/hooks/useStatsQuery"
import { useCollectionStatus } from "@/hooks/useCollectionStatus"
import { useMainContextReport } from "@/hooks/useMainContextReport"
import { demoActivity, demoBudgetStatus, demoCalls, demoOverview } from "@/mock/usageStats"
import { useToast } from "@/components/ui/Toast"
import { AnalysisPanel } from "@/components/usage/AnalysisPanel"
import { CallsPanel } from "@/components/usage/CallsPanel"
import { DateRangeDialog } from "@/components/usage/DateRangeDialog"
import { OverviewPanel } from "@/components/usage/OverviewPanel"
import { PricingDialog, type PricingDialogTab } from "@/components/usage/PricingDialog"
import {
  ALL, AccountFilter, EmptyCard, ErrorNotice, FilterTrigger, LoadingCard, MenuFilter, RefreshButton, ResetButton, providerOptions,
} from "@/components/usage/usageControls"
import { Banner, Button, Input, PageActions, PageTabs, Tabs } from "@/ui"
import type { IconName } from "@/icons"
import type { PlatformId } from "@/types"

export type UsageTab = "overview" | "analysis" | "calls"

const TABS: { value: UsageTab; label: string; icon: IconName }[] = [
  { value: "overview", label: "概览", icon: "activity" },
  { value: "analysis", label: "用量分析", icon: "navUsage" },
  { value: "calls", label: "调用明细", icon: "list" },
]

/** 调用明细的状态筛选：成功、失败（上游错误）、已取消；三者互斥 */
type CallStatus = "success" | "error" | "cancelled" | "unknown" | null

/**
 * 用量统计（Jolu.pen 模块 C，画板 30–39）：概览、用量分析、调用明细三个页签。
 * 日期范围与供应商在三个页签间共用；账号筛选只作用于概览与分析（调用明细画板没有账号筛选）。
 */
/** 外部要求打开调用明细：会话页「查看用量明细」带入会话 ID 与日期范围 */
export interface UsageRequest {
  search: string
  range: DateRange
  nonce: number
  /** 打开哪个分页；缺省为调用明细（会话 → 调用明细的原有跳转） */
  tab?: UsageTab
  /** 预筛的平台与账号（灵动岛「查看用量」） */
  platform?: PlatformId | null
  accounts?: string[]
}

export function UsageView({ active, request, onOpenSession }: {
  active: boolean
  request: UsageRequest | null
  /** 不带参数时只打开会话线程页 */
  onOpenSession: (target?: { platform: string; sessionId: string }) => void
}) {
  const toast = useToast()
  const conn = useConnections()
  const [tab, setTab] = useState<UsageTab>("overview")
  const [range, setRange] = useState<DateRange>(() => todayRange())
  const [platform, setPlatform] = useState<PlatformId | null>(null)
  const collection = useCollectionStatus()
  const collectionNote = collectionWarning(collection.status, platform)
  const [accounts, setAccounts] = useState<string[]>([])
  // 4.2b：只筛了一个真实账号时，告诉本体岛「主窗口在看这个连接」（岛开了「跟随主窗口」才会用）
  useMainContextReport("usage", accounts.length === 1 && accounts[0] !== UNATTRIBUTED_ACCOUNT ? accounts[0] : null)
  const [model, setModel] = useState<string | null>(null)
  const [status, setStatus] = useState<CallStatus>(null)
  const [billingScope, setBillingScope] = useState<"all" | "api" | "subscription">("all")
  const [pricingOpen, setPricingOpen] = useState(false)
  const [pricingTarget, setPricingTarget] = useState<{ tab?: PricingDialogTab; model?: string | null; filter?: "all" | "unpriced" }>({})
  const [searchDraft, setSearchDraft] = useState("")
  const [search, setSearch] = useState("")
  const [page, setPage] = useState(1)
  const [selected, setSelected] = useState<number | null>(null)
  const [picking, setPicking] = useState(false)
  const [exporting, setExporting] = useState(false)

  const openPricing = (
    modelName?: string | null,
    filter: "all" | "unpriced" = "all",
    initialTab: PricingDialogTab = "catalog",
  ) => {
    setPricingTarget({ tab: initialTab, model: modelName ?? null, filter })
    setPricingOpen(true)
  }

  useEffect(() => {
    if (!request) return
    setTab(request.tab ?? "calls")
    setRange(request.range)
    setPlatform(request.platform ?? null)
    setAccounts(request.accounts ?? [])
    setModel(null)
    setStatus(null)
    setSearch(request.search)
    setSearchDraft(request.search)
    setPage(1)
  }, [request])

  const statsFilter = (): StatsFilterDto => ({ ...rangeToMillis(range), platform, connections: accounts, billing_scope: billingScope })
  const callsFilter = (): StatsFilterDto => ({ ...rangeToMillis(range), platform, connections: accounts, model, status, search: search.trim() || null, billing_scope: billingScope })

  const cfg = useSettings()
  const costCurrency = (cfg.settings.cost_currency === "CNY" ? "CNY" : "USD") as "USD" | "CNY"
  const fx = useCostFx(costCurrency, `${cfg.settings.fx_mode ?? "auto"}:${cfg.settings.fx_fixed_usd_cny ?? ""}`)

  const overview = useStatsQuery(
    "stats-overview:" + JSON.stringify([range, platform, accounts, billingScope]),
    () => api.statsOverview(statsFilter()),
    () => demoOverview(rangeToMillis(range).start),
    platform,
    active && tab !== "calls",
  )
  const budgetStatus = useStatsQuery(
    "pricing-budget-status:" + JSON.stringify([cfg.settings.global_budget_enabled, cfg.settings.global_budget_daily_usd, cfg.settings.global_budget_monthly_usd, cfg.settings.global_budget_warn_percent]),
    () => api.pricingBudgetStatus(),
    () => demoBudgetStatus(),
    null,
    active && tab === "analysis",
  )
  const activity = useStatsQuery(
    "stats-activity:" + JSON.stringify([platform, accounts]),
    () => api.statsActivity(platform, accounts, Date.now()),
    () => demoActivity(new Date()),
    platform,
    active && tab === "analysis",
  )
  const calls = useStatsQuery<StatsCallPageDto>(
    "stats-calls:" + JSON.stringify([range, platform, accounts, model, status, search, billingScope, page]),
    () => api.statsCalls(callsFilter(), page),
    () => demoCalls(Date.now(), page),
    platform,
    active && tab === "calls",
  )

  const changePlatform = (next: PlatformId | null) => {
    setPlatform(next)
    setAccounts((value) => keepApplicable(value, conn.connections, next))
    setModel(null)
    setPage(1)
  }
  const changeRange = (next: DateRange) => {
    setRange(next)
    setPage(1)
  }
  const resetCalls = () => {
    setRange(todayRange())
    setPlatform(null)
    setAccounts([])
    setBillingScope("all")
    setModel(null)
    setStatus(null)
    setSearch("")
    setSearchDraft("")
    setPage(1)
  }
  const clearFilters = () => {
    setPlatform(null)
    setAccounts([])
    setBillingScope("all")
  }

  /** 下钻到调用明细：图表数据点按所在日期，模型 / 供应商按名称（画板 38「交互」） */
  const drillBucket = (point: StatsTrendBucketDto) => {
    changeRange({ start: dateKey(new Date(point.start)), end: dateKey(new Date(point.end - 1)) })
    setTab("calls")
  }
  const drillModel = (row: StatsModelRowDto) => {
    setPlatform(row.platform as PlatformId)
    setModel(row.model)
    setPage(1)
    setTab("calls")
  }
  const drillProvider = (id: string) => {
    changePlatform(id as PlatformId)
    setTab("calls")
  }

  const exportCsv = async () => {
    if (!isTauri) return toast.info("浏览器预览不支持导出", "请在桌面应用中导出 CSV")
    setExporting(true)
    try {
      const result = await api.exportCallsCsv(callsFilter())
      if (result) toast.success(`导出完成 · ${result.count} 条`, result.path)
    } catch (error) {
      toast.danger("导出失败", error instanceof Error ? error.message : String(error))
    } finally {
      setExporting(false)
    }
  }

  const isToday = range.start === range.end && range.start === dateKey(new Date())
  const zone = zoneName(localZone())
  const current = tab === "calls" ? calls : overview
  const callRows = calls.data?.rows ?? []
  const selectedId = callRows.some((row) => row.id === selected) ? selected : callRows[0]?.id ?? null

  // 方案 A 供应商下拉：保留同日期/账号口径下全平台计数，切到单平台后下拉仍可直切其余活跃平台
  const scopeKey = JSON.stringify([range, accounts, billingScope])
  const [scopeCounts, setScopeCounts] = useState<{ key: string; map: Map<string, number>; total: number } | null>(null)
  useEffect(() => {
    if (platform !== null || !overview.data) return
    setScopeCounts({
      key: scopeKey,
      map: new Map(overview.data.providers.map((p) => [p.platform, p.requests])),
      total: overview.data.requests,
    })
  }, [platform, overview.data, scopeKey])
  const supplierCounts =
    scopeCounts?.key === scopeKey
      ? scopeCounts.map
      : overview.data
        ? new Map(overview.data.providers.map((p) => [p.platform, p.requests]))
        : undefined
  const supplierTotal = scopeCounts?.key === scopeKey ? scopeCounts.total : undefined
  const supplierOptions = providerOptions(supplierCounts, supplierTotal)

  const dateButton = (calendar: boolean) => (
    <FilterTrigger
      aria-label="日期范围"
      onClick={() => setPicking(true)}
      leading={calendar ? <CalendarDays aria-hidden className="size-4 text-jn-text-3" /> : undefined}
      trailingChevron={calendar}
    >
      {rangeLabel(range)}
    </FilterTrigger>
  )

  return (
    <div className="flex w-full flex-col gap-3.5">
      <PageTabs wide>
        <Tabs
          value={tab}
          onChange={setTab}
          items={TABS.map((t) => ({ key: t.value, label: t.label, icon: t.icon }))}
        />
      </PageTabs>

      <PageActions>
        <Button kind="ghost" size="sm" icon="sliders" onClick={() => openPricing(null, "all", "catalog")}>
          预算与价目
        </Button>
        {tab === "calls" && (
          <Button
            kind="ghost"
            size="sm"
            icon="download"
            loading={exporting}
            disabled={exporting || !calls.data?.total_count}
            onClick={() => void exportCsv()}
          >
            {exporting ? `导出 ${calls.data?.total_count ?? 0} 条…` : "导出 CSV"}
          </Button>
        )}
      </PageActions>

      {tab !== "calls" ? (
        <div className="flex flex-wrap items-center gap-2">
          {dateButton(true)}
          <MenuFilter
            label="供应商筛选"
            trailingChevron
            active={platform !== null}
            menuClassName="min-w-[240px]"
            value={platform ?? ALL}
            options={supplierOptions}
            onChange={(v) => changePlatform(v === ALL ? null : (v as PlatformId))}
          />
          <AccountFilter accounts={accountsFor(conn.connections, platform)} value={accounts} onChange={setAccounts} />
          <MenuFilter
            label="计费口径筛选"
            trailingChevron
            value={billingScope}
            options={[
              { value: "all", label: "全部口径" },
              { value: "api", label: "仅 API 实付" },
              { value: "subscription", label: "仅订阅等价" },
            ]}
            onChange={(v) => setBillingScope(v as "all" | "api" | "subscription")}
          />
          <span className="min-w-0 flex-1" />
          <span className="text-xs text-jn-text-3">{range.end.slice(5).replace("-", "/")} · {zone}时区</span>
          <CurrencyPicker value={costCurrency} loading={fx.loading} onChange={(next) => void cfg.setCostCurrency(next)} hint={fx} />
          <RefreshButton loading={overview.refreshing} onClick={() => { overview.refresh(); if (tab === "analysis") { activity.refresh(); budgetStatus.refresh() } }} />
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          {dateButton(true)}
          <MenuFilter
            label="供应商筛选"
            trailingChevron
            active={platform !== null}
            menuClassName="min-w-[240px]"
            value={platform ?? ALL}
            options={supplierOptions}
            onChange={(v) => changePlatform(v === ALL ? null : (v as PlatformId))}
          />
          <AccountFilter accounts={accountsFor(conn.connections, platform)} value={accounts} onChange={(next) => { setAccounts(next); setPage(1) }} />
          <MenuFilter
            label="计费口径筛选"
            trailingChevron
            value={billingScope}
            options={[
              { value: "all", label: "全部口径" },
              { value: "api", label: "仅 API 实付" },
              { value: "subscription", label: "仅订阅等价" },
            ]}
            onChange={(v) => { setBillingScope(v as "all" | "api" | "subscription"); setPage(1) }}
          />
          <MenuFilter
            label="模型筛选"
            trailingChevron
            value={model ?? ALL}
            options={[{ value: ALL, label: "全部模型" }, ...(calls.data?.models ?? []).map((m) => ({ value: m, label: m }))]}
            onChange={(v) => { setModel(v === ALL ? null : v); setPage(1) }}
          />
          <MenuFilter
            label="状态筛选"
            trailingChevron
            value={status ?? ALL}
            options={[{ value: ALL, label: "全部状态" }, { value: "success", label: "成功" }, { value: "error", label: "失败" }, { value: "cancelled", label: "已取消" }, { value: "unknown", label: "状态未知" }]}
            onChange={(v) => { setStatus(v === ALL ? null : (v as CallStatus)); setPage(1) }}
          />
          <Input
            size="md"
            icon="search"
            value={searchDraft}
            onChange={(event) => setSearchDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter") return
              setSearch(searchDraft)
              setPage(1)
            }}
            placeholder="搜索调用 ID / 会话 ID，按 Enter 查询"
            aria-label="搜索调用 ID 或会话 ID"
            className="w-[240px] shrink-0 bg-jn-surface shadow-jn-surface"
          />
          <span className="min-w-0 flex-1" />
          <CurrencyPicker value={costCurrency} loading={fx.loading} onChange={(next) => void cfg.setCostCurrency(next)} hint={fx} />
          <span className="hidden text-xs text-jn-text-3 xl:inline">时区 · {zone} {utcOffsetLabel()}</span>
          <ResetButton onClick={resetCalls} />
          <RefreshButton loading={calls.refreshing} onClick={calls.refresh} />
        </div>
      )}

      {current.error && <ErrorNotice error={current.error} stale={current.data !== null} onRetry={current.refresh} />}
      {collectionNote && <Banner tone="warning">{collectionNote}</Banner>}

      {tab === "analysis" && activity.error && <ErrorNotice error={activity.error} stale={!!activity.data} onRetry={activity.refresh} />}
      <ContentTransition identity={`${tab}-${!!current.data}`} className="flex flex-col gap-3.5">
      {tab === "calls" ? (
        calls.data ? (
          <CallsPanel
            data={calls.data}
            selectedId={selectedId}
            fx={fx}
            onSelect={setSelected}
            onPage={setPage}
            onOpenSession={onOpenSession}
            onOpenPricing={(modelName) => openPricing(modelName, modelName ? "all" : "unpriced")}
          />
        ) : !calls.error && <LoadingCard />
      ) : overview.data ? (
        overview.data.requests === 0 ? (
          <EmptyCard onClear={platform || accounts.length || billingScope !== "all" ? clearFilters : undefined} />
        ) : tab === "overview" ? (
          <OverviewPanel
            data={overview.data}
            isToday={isToday}
            fx={fx}
            onDrillBucket={drillBucket}
            onDrillModel={drillModel}
            onOpenSession={onOpenSession}
            onOpenPricing={() => openPricing(null, "unpriced")}
          />
        ) : (
          <AnalysisPanel
            data={overview.data}
            activity={activity.data}
            fx={fx}
            budgetStatus={budgetStatus.data}
            billingScope={billingScope}
            activityError={activity.error}
            onDrillModel={drillModel}
            onDrillProvider={drillProvider}
            onDrillConnection={(connectionId) => {
              setAccounts([connectionId ?? UNATTRIBUTED_ACCOUNT])
              setPage(1)
              setTab("calls")
            }}
            onOpenBudgets={() => openPricing(null, "all", "budgets")}
            onOpenPricingForModel={(modelName) => openPricing(modelName, "unpriced", "catalog")}
          />
        )
      ) : !overview.error && <LoadingCard layout="stats" />}
      </ContentTransition>

      {picking && (
        <DateRangeDialog
          value={range}
          onClose={() => setPicking(false)}
          onConfirm={(next) => {
            changeRange(next)
            setPicking(false)
          }}
        />
      )}
      <PricingDialog
        open={pricingOpen}
        onOpenChange={setPricingOpen}
        connections={conn.connections}
        fx={fx}
        initialTab={pricingTarget.tab}
        initialModel={pricingTarget.model}
        initialFilter={pricingTarget.filter}
      />
    </div>
  )
}

/**
 * 费用币种切换（USD / CNY，R1-04 胶囊分段样式）。
 */
function CurrencyPicker({ value, loading, hint, onChange }: {
  value: "USD" | "CNY"
  loading: boolean
  hint: { rate: number; asOf: string; fallback: boolean; fixed?: boolean }
  onChange: (next: "USD" | "CNY") => void
}) {
  const title = value === "CNY"
    ? `1 USD = ${hint.rate.toFixed(3)} CNY · ${hint.asOf}${hint.fallback ? "（离线参考价）" : ""}`
    : "费用按美元显示（基准币种）"
  return (
    <span className="inline-flex items-center gap-1.5" title={title}>
      <Tabs
        style="segment"
        value={value}
        onChange={(next) => { if (next !== value) onChange(next) }}
        className="border border-jn-line/80"
        items={[
          { key: "USD", label: "$ USD" },
          { key: "CNY", label: "¥ CNY" },
        ]}
      />
      {value === "CNY" && (
        <span className={hint.fallback ? "text-[11px] text-jn-danger" : hint.fixed ? "text-[11px] font-medium text-jn-primary-ink" : "text-[11px] text-jn-text-3"}>
          {loading ? "汇率…" : hint.fallback ? "参考" : hint.fixed ? `固定 ${hint.rate.toFixed(2)}` : `≈${hint.rate.toFixed(2)}`}
        </span>
      )}
    </span>
  )
}
