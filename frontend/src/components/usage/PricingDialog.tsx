import { useCallback, useEffect, useMemo, useState } from "react"
import {
  api,
  isTauri,
  type BudgetStatusDto,
  type ModelsDevPriceDto,
  type PricingCatalogViewDto,
  type PricingEntryDto,
  type PricingOverrideDto,
} from "@/lib/api"
import { demoBudgetStatus, demoPricingCatalog } from "@/mock/usageStats"
import type { FxState } from "@/lib/costCurrency"
import { formatMoney } from "@/lib/usageStats"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { useToast } from "@/components/ui/Toast"
import { SelectField } from "@/components/ui/SelectField"
import { cn } from "@/lib/utils"
import { Icon } from "@/icons"
import { Button, Dialog, Input, StatusPill, Switch, Tabs } from "@/ui"
import type { Connection, PlatformId } from "@/types"

export type PricingDialogTab = "catalog" | "aliases" | "budgets" | "rules"
type SourceFilter = "all" | "unpriced" | "user_custom" | "models_dev" | "builtin"

interface PricingDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  connections: Connection[]
  fx: FxState
  initialTab?: PricingDialogTab
  /** 打开时预聚焦到某模型（如从调用明细「去补价」点进来） */
  initialModel?: string | null
  initialFilter?: SourceFilter
}

interface BudgetDraft {
  globalEnabled: boolean
  globalDaily: string
  globalMonthly: string
  warnPercent: string
  connections: Record<string, { daily: string; monthly: string; action: "alert" | "block" }>
}

interface EditDraft {
  modelId: string
  displayName: string
  isNew: boolean
  input: string
  output: string
  cacheRead: string
  cacheWrite: string
  cacheWrite1h: string
  showLongCtx: boolean
  longCtxThreshold: string
  longCtxInput: string
  longCtxOutput: string
  longCtxCacheRead: string
  longCtxCacheWrite: string
}

const EMPTY_DRAFT: EditDraft = {
  modelId: "",
  displayName: "",
  isNew: true,
  input: "",
  output: "",
  cacheRead: "0",
  cacheWrite: "",
  cacheWrite1h: "",
  showLongCtx: false,
  longCtxThreshold: "",
  longCtxInput: "",
  longCtxOutput: "",
  longCtxCacheRead: "",
  longCtxCacheWrite: "",
}

function fmtUsdUnit(v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—"
  const fixed = v.toFixed(4).replace(/0+$/, "")
  const [whole, frac = ""] = fixed.split(".")
  return `$${whole}.${frac.padEnd(2, "0")}`
}

function parseNonNeg(raw: string, required: boolean): number | null | "invalid" {
  const s = raw.trim()
  if (!s) return required ? "invalid" : null
  const n = Number(s)
  if (!Number.isFinite(n) || n < 0 || n > 1_000_000) return "invalid"
  return n
}

function formatSyncTime(ms: number): string {
  if (!ms) return "尚未联网同步（使用内置快照）"
  const d = new Date(ms)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`
}

function entryToDraft(entry: PricingEntryDto): EditDraft {
  const hasTier = entry.long_ctx_threshold !== null && entry.long_ctx_threshold > 0
  return {
    modelId: entry.model_id,
    displayName: entry.display_name,
    isNew: false,
    input: entry.input !== null ? String(entry.input) : "",
    output: entry.output !== null ? String(entry.output) : "",
    cacheRead: entry.cache_read !== null ? String(entry.cache_read) : "0",
    cacheWrite: entry.cache_write !== null ? String(entry.cache_write) : "",
    cacheWrite1h: entry.cache_write_1h !== null ? String(entry.cache_write_1h) : "",
    showLongCtx: hasTier,
    longCtxThreshold: hasTier ? String(entry.long_ctx_threshold) : "",
    longCtxInput: entry.long_ctx_input !== null ? String(entry.long_ctx_input) : "",
    longCtxOutput: entry.long_ctx_output !== null ? String(entry.long_ctx_output) : "",
    longCtxCacheRead: entry.long_ctx_cache_read !== null ? String(entry.long_ctx_cache_read) : "",
    longCtxCacheWrite: entry.long_ctx_cache_write !== null ? String(entry.long_ctx_cache_write) : "",
  }
}

export function PricingDialog({ open, onOpenChange, connections, fx, initialTab, initialModel, initialFilter }: PricingDialogProps) {
  const toast = useToast()
  const [tab, setTab] = useState<PricingDialogTab>("catalog")
  const [catalog, setCatalog] = useState<PricingCatalogViewDto | null>(null)
  const [budgetStatus, setBudgetStatus] = useState<BudgetStatusDto | null>(null)
  const [budgetDraft, setBudgetDraft] = useState<BudgetDraft | null>(null)
  const [savingBudgets, setSavingBudgets] = useState(false)
  const [fxMode, setFxMode] = useState<"auto" | "fixed">("auto")
  const [fxFixedInput, setFxFixedInput] = useState("7.25")
  const [savingFx, setSavingFx] = useState(false)
  const [loading, setLoading] = useState(false)
  const [syncing, setSyncing] = useState(false)
  const [search, setSearch] = useState("")
  const [sourceFilter, setSourceFilter] = useState<SourceFilter>("all")
  const [draft, setDraft] = useState<EditDraft | null>(null)
  const [saving, setSaving] = useState(false)

  // models.dev 预填选择器
  const [pickerOpen, setPickerOpen] = useState(false)
  const [mdList, setMdList] = useState<ModelsDevPriceDto[]>([])
  const [mdSearch, setMdSearch] = useState("")

  // 同价映射表单
  const [aliasScope, setAliasScope] = useState<string>("*")
  const [aliasModel, setAliasModel] = useState("")
  const [targetModel, setTargetModel] = useState("")
  const [savingAlias, setSavingAlias] = useState(false)

  const syncBudgetDraft = (status: BudgetStatusDto) => {
    const connMap: BudgetDraft["connections"] = {}
    for (const c of status.connections) {
      connMap[c.connection_id] = {
        daily: c.daily_limit !== null ? String(c.daily_limit) : "",
        monthly: c.monthly_limit !== null ? String(c.monthly_limit) : "",
        action: c.action === "block" && c.serve_enabled ? "block" : "alert",
      }
    }
    setBudgetDraft({
      globalEnabled: status.global_enabled,
      globalDaily: status.global_daily_limit !== null ? String(status.global_daily_limit) : "",
      globalMonthly: status.global_monthly_limit !== null ? String(status.global_monthly_limit) : "",
      warnPercent: String(status.warn_percent),
      connections: connMap,
    })
  }

  const reload = useCallback(async () => {
    setLoading(true)
    try {
      const [data, bStatus] = isTauri
        ? await Promise.all([api.pricingCatalog(), api.pricingBudgetStatus()])
        : [demoPricingCatalog(), demoBudgetStatus()]
      setCatalog(data)
      setBudgetStatus(bStatus)
      syncBudgetDraft(bStatus)
      setFxMode(data.fx_mode === "fixed" ? "fixed" : "auto")
      setFxFixedInput(data.fx_fixed_usd_cny ? String(data.fx_fixed_usd_cny) : String(Number(fx.rate > 1 ? fx.rate.toFixed(2) : 7.25)))
    } catch (err) {
      toast.danger("读取模型价目失败", String(err))
    } finally {
      setLoading(false)
    }
  }, [toast, fx.rate])

  useEffect(() => {
    if (!open) return
    setDraft(null)
    setPickerOpen(false)
    setTab(initialTab ?? "catalog")
    setSourceFilter(initialFilter ?? "all")
    setSearch(initialModel ?? "")
    void reload()
  }, [open, initialTab, initialModel, initialFilter, reload])

  useEffect(() => {
    if (!open || !initialModel || !catalog) return
    const lower = initialModel.trim().toLowerCase()
    const match = catalog.entries.find((e) => e.model_id.toLowerCase() === lower)
    if (match && match.source_kind === "unpriced") {
      setDraft(entryToDraft(match))
    } else if (!match && lower) {
      setDraft({ ...EMPTY_DRAFT, modelId: lower, displayName: initialModel.trim(), isNew: true })
    }
  }, [open, initialModel, catalog])

  const openPicker = async () => {
    setPickerOpen(true)
    setMdSearch("")
    if (!mdList.length && isTauri) {
      try {
        setMdList(await api.pricingModelsDevCatalog())
      } catch (err) {
        toast.danger("读取 models.dev 目录失败", String(err))
      }
    }
  }

  const syncModelsDev = async () => {
    if (!isTauri) return toast.info("浏览器预览不支持联网同步")
    setSyncing(true)
    try {
      const [count] = await api.pricingRefreshModelsDev()
      toast.success(`已同步 models.dev 目录（${count} 个模型）`)
      setMdList([])
      await reload()
    } catch (err) {
      toast.danger("同步 models.dev 失败", String(err))
    } finally {
      setSyncing(false)
    }
  }

  const saveDraft = async () => {
    if (!draft) return
    const modelId = draft.modelId.trim()
    if (!modelId) return toast.warn("请填写模型 ID")
    const input = parseNonNeg(draft.input, true)
    const output = parseNonNeg(draft.output, true)
    const cacheRead = parseNonNeg(draft.cacheRead, false)
    const cacheWrite = parseNonNeg(draft.cacheWrite, false)
    const cacheWrite1h = parseNonNeg(draft.cacheWrite1h, false)
    if (input === "invalid" || output === "invalid" || cacheRead === "invalid" || cacheWrite === "invalid" || cacheWrite1h === "invalid") {
      return toast.warn("请输入有效的非负单价（美元 / 百万 Token）")
    }
    let longThreshold: number | null = null
    let longIn: number | null = null
    let longOut: number | null = null
    let longCr: number | null = null
    let longCw: number | null = null
    if (draft.showLongCtx && draft.longCtxThreshold.trim()) {
      const th = Number(draft.longCtxThreshold.trim())
      if (!Number.isFinite(th) || th <= 0) return toast.warn("长上下文阈值须为正整数")
      const li = parseNonNeg(draft.longCtxInput, false)
      const lo = parseNonNeg(draft.longCtxOutput, false)
      const lcr = parseNonNeg(draft.longCtxCacheRead, false)
      const lcw = parseNonNeg(draft.longCtxCacheWrite, false)
      if (li === "invalid" || lo === "invalid" || lcr === "invalid" || lcw === "invalid") {
        return toast.warn("长上下文单价须为非负数字")
      }
      longThreshold = Math.round(th)
      longIn = li
      longOut = lo
      longCr = lcr
      longCw = lcw
    }
    if (!isTauri) {
      toast.info("浏览器预览仅演示界面")
      setDraft(null)
      return
    }
    setSaving(true)
    try {
      const payload: PricingOverrideDto = {
        scope: "*",
        model_id: modelId,
        display_name: draft.displayName.trim() || modelId,
        input: input as number,
        output: output as number,
        cache_read: cacheRead ?? 0,
        cache_write: cacheWrite,
        cache_write_1h: cacheWrite1h,
        long_ctx_threshold: longThreshold,
        long_ctx_input: longIn,
        long_ctx_output: longOut,
        long_ctx_cache_read: longCr,
        long_ctx_cache_write: longCw,
        disabled: false,
      }
      await api.pricingUpsertOverride(payload)
      toast.success(`已保存「${modelId}」价目，历史与当前费用已重算`)
      setDraft(null)
      await reload()
    } catch (err) {
      toast.danger("保存模型价目失败", String(err))
    } finally {
      setSaving(false)
    }
  }

  const restoreDefault = async (modelId: string) => {
    if (!isTauri) return
    try {
      await api.pricingDeleteOverride("*", modelId)
      toast.success(`已恢复「${modelId}」为目录默认价目`)
      await reload()
    } catch (err) {
      toast.danger("恢复默认失败", String(err))
    }
  }

  const disableEntry = async (modelId: string) => {
    if (!isTauri) return
    try {
      await api.pricingDisableOverride("*", modelId)
      toast.success(`已停用「${modelId}」计价`)
      await reload()
    } catch (err) {
      toast.danger("停用失败", String(err))
    }
  }

  const addAlias = async () => {
    const a = aliasModel.trim()
    const t = targetModel.trim()
    if (!a || !t) return toast.warn("请填写别名模型和目标基准模型")
    if (!isTauri) return toast.info("浏览器预览仅演示界面")
    setSavingAlias(true)
    try {
      await api.pricingUpsertAlias(aliasScope, a, t)
      toast.success(`已映射 ${a} → ${t}`)
      setAliasModel("")
      setTargetModel("")
      await reload()
    } catch (err) {
      toast.danger("保存映射失败", String(err))
    } finally {
      setSavingAlias(false)
    }
  }

  const removeAlias = async (scope: string, alias: string) => {
    if (!isTauri) return
    try {
      await api.pricingDeleteAlias(scope, alias)
      toast.success(`已删除映射 ${alias}`)
      await reload()
    } catch (err) {
      toast.danger("删除映射失败", String(err))
    }
  }

  const updateRules = async (
    source: "upstream" | "request" | null,
    autoSync: boolean | null,
    nextFxMode?: "auto" | "fixed" | null,
    nextFxFixed?: number | null,
  ) => {
    if (!isTauri) {
      toast.info("浏览器预览仅演示界面")
      return
    }
    try {
      await api.pricingSetRules(source, autoSync, nextFxMode, nextFxFixed)
      toast.success("计费规则已更新")
      await reload()
    } catch (err) {
      toast.danger("保存计费规则失败", String(err))
    }
  }

  const saveFxRules = async () => {
    if (fxMode === "fixed") {
      const parsed = Number(fxFixedInput.trim())
      if (!Number.isFinite(parsed) || parsed < 1 || parsed > 20) {
        return toast.warn("自定义固定汇率须在 1.0 到 20.0 之间")
      }
      if (!isTauri) return toast.info("浏览器预览仅演示界面")
      setSavingFx(true)
      try {
        await api.pricingSetRules(null, null, "fixed", parsed)
        toast.success(`已锁定固定汇率 1 USD = ${parsed.toFixed(2)} CNY`)
        await reload()
      } catch (err) {
        toast.danger("保存汇率设置失败", String(err))
      } finally {
        setSavingFx(false)
      }
    } else {
      if (!isTauri) return toast.info("浏览器预览仅演示界面")
      setSavingFx(true)
      try {
        await api.pricingSetRules(null, null, "auto", null)
        toast.success("已切换为自动在线汇率")
        await reload()
      } catch (err) {
        toast.danger("保存汇率设置失败", String(err))
      } finally {
        setSavingFx(false)
      }
    }
  }

  const parsePositiveOpt = (raw: string): number | null | "invalid" => {
    const s = raw.trim()
    if (!s) return null
    const n = Number(s)
    if (!Number.isFinite(n) || n <= 0 || n > 1_000_000) return "invalid"
    return n
  }

  const saveBudgets = async () => {
    if (!budgetDraft || !budgetStatus) return
    const gd = parsePositiveOpt(budgetDraft.globalDaily)
    const gm = parsePositiveOpt(budgetDraft.globalMonthly)
    const warn = Number(budgetDraft.warnPercent.trim())
    if (gd === "invalid" || gm === "invalid") return toast.warn("全局预算上限须为正数或留空")
    if (!Number.isFinite(warn) || warn < 50 || warn > 99) return toast.warn("预警阈值须在 50% 到 99% 之间")
    const connPayloads: { connection_id: string; daily_limit: number | null; monthly_limit: number | null; action: "alert" | "block" }[] = []
    for (const row of budgetStatus.connections) {
      const item = budgetDraft.connections[row.connection_id]
      if (!item) continue
      const dl = parsePositiveOpt(item.daily)
      const ml = parsePositiveOpt(item.monthly)
      if (dl === "invalid" || ml === "invalid") return toast.warn(`「${row.name}」的日/月上限须为正数或留空`)
      connPayloads.push({
        connection_id: row.connection_id,
        daily_limit: dl,
        monthly_limit: ml,
        action: row.serve_enabled ? item.action : "alert",
      })
    }
    if (!isTauri) {
      toast.info("浏览器预览仅演示界面")
      return
    }
    setSavingBudgets(true)
    try {
      const updated = await api.pricingSaveBudgets({
        global_enabled: budgetDraft.globalEnabled,
        global_daily_limit: gd,
        global_monthly_limit: gm,
        warn_percent: Math.round(warn),
        connections: connPayloads,
      })
      setBudgetStatus(updated)
      syncBudgetDraft(updated)
      toast.success("预算限额已保存并生效")
    } catch (err) {
      toast.danger("保存预算限额失败", String(err))
    } finally {
      setSavingBudgets(false)
    }
  }

  const unpricedCount = useMemo(
    () => (catalog?.entries ?? []).filter((e) => e.source_kind === "unpriced" && !e.disabled).length,
    [catalog],
  )

  const filteredEntries = useMemo(() => {
    const q = search.trim().toLowerCase()
    return (catalog?.entries ?? []).filter((entry) => {
      if (sourceFilter === "unpriced" && entry.source_kind !== "unpriced") return false
      if (sourceFilter === "user_custom" && entry.source_kind !== "user_custom") return false
      if (sourceFilter === "models_dev" && entry.source_kind !== "models_dev") return false
      if (sourceFilter === "builtin" && entry.source_kind !== "builtin") return false
      if (!q) return true
      return (
        entry.model_id.toLowerCase().includes(q)
        || entry.display_name.toLowerCase().includes(q)
        || entry.vendor.toLowerCase().includes(q)
      )
    })
  }, [catalog, search, sourceFilter])

  const filteredMd = useMemo(() => {
    const q = mdSearch.trim().toLowerCase()
    const list = mdList.filter(
      (m) => !q || m.model_id.toLowerCase().includes(q) || m.display_name.toLowerCase().includes(q) || m.vendor.toLowerCase().includes(q),
    )
    return list.slice(0, 80)
  }, [mdList, mdSearch])

  const connMap = useMemo(() => new Map(connections.map((c) => [c.id, c.baseName])), [connections])
  const scopeOptions = useMemo(
    () => [{ value: "*", label: "全局生效" }, ...connections.map((c) => ({ value: c.id, label: c.baseName }))],
    [connections],
  )
  const customConnections = useMemo(
    () => connections.filter((c) => (c.billingMode && c.billingMode !== "auto") || ((c.priceMultiplier ?? 1) !== 1)),
    [connections],
  )

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      width="xl"
      title="模型价目、预算与汇率"
      description="自定义单价、预算提醒与汇率来源实时生效；历史费用与统计口径立即同步重算。"
      bodyClassName="gap-3.5"
      footerStart={
        <span className="text-note text-jn-text-3">
          models.dev：{catalog ? `${catalog.models_dev_count} 个带价模型 · ${formatSyncTime(catalog.models_dev_fetched_ms)}` : "加载中…"}
        </span>
      }
      actions={
        <>
          <Button kind="ghost" size="md" onClick={() => onOpenChange(false)}>
            {tab === "budgets" || tab === "rules" ? "取消" : "关闭"}
          </Button>
          {tab === "budgets" && (
            <Button kind="primary" size="md" icon="check" loading={savingBudgets} onClick={() => void saveBudgets()}>
              保存预算
            </Button>
          )}
          {tab === "rules" && (
            <Button kind="primary" size="md" icon="check" loading={savingFx} onClick={() => void saveFxRules()}>
              保存汇率
            </Button>
          )}
        </>
      }
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Tabs
          style="segment"
          value={tab}
          onChange={setTab}
          items={[
            { key: "catalog", label: unpricedCount > 0 ? `模型价目（${unpricedCount} 待补）` : "模型价目" },
            { key: "aliases", label: `同价映射（${catalog?.aliases.length ?? 0}）` },
            { key: "budgets", label: "预算限额" },
            { key: "rules", label: "规则与汇率" },
          ]}
        />
        <div className="flex items-center gap-2">
          {tab === "budgets" ? (
            <span className="text-note text-jn-text-3">
              当前换算：1 USD = ¥{(fxMode === "fixed" && Number(fxFixedInput) > 0 ? Number(fxFixedInput) : (fx.rate > 1 ? fx.rate : 7.2)).toFixed(2)}
              {fxMode === "fixed" ? "（固定）" : ""}
            </span>
          ) : (
            <Button kind="soft" size="sm" icon="refresh" loading={syncing} onClick={() => void syncModelsDev()}>
              同步 models.dev
            </Button>
          )}
          {tab === "catalog" && (
            <Button kind="primary" size="sm" icon="add" onClick={() => setDraft({ ...EMPTY_DRAFT })}>
              新增价目
            </Button>
          )}
        </div>
      </div>

      {tab === "catalog" && (
        <>
          {draft && (
            <div className="flex flex-col gap-3 rounded-[18px] border border-jn-primary-soft bg-jn-well p-4">
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs font-semibold text-jn-text">
                  {draft.isNew ? "新增全局模型价目" : `编辑价目 · ${draft.modelId}`}
                </span>
                <div className="flex items-center gap-2">
                  <Button kind="soft" size="sm" icon="globe" onClick={() => void openPicker()}>
                    从 models.dev 预填
                  </Button>
                  <Button kind="ghost" size="sm" onClick={() => { setDraft(null); setPickerOpen(false) }}>取消</Button>
                  <Button kind="primary" size="sm" loading={saving} onClick={() => void saveDraft()}>保存</Button>
                </div>
              </div>

              {pickerOpen && (
                <div className="flex flex-col gap-2 rounded-[14px] border border-jn-line bg-jn-surface p-3">
                  <div className="flex items-center gap-2">
                    <Input
                      size="md"
                      icon="search"
                      value={mdSearch}
                      onChange={(e) => setMdSearch(e.target.value)}
                      placeholder="搜索 models.dev 模型 ID / 厂商…"
                      className="flex-1"
                    />
                    <Button kind="ghost" size="sm" onClick={() => setPickerOpen(false)}>收起</Button>
                  </div>
                  <div className="flex max-h-44 flex-col gap-1 overflow-y-auto">
                    {filteredMd.map((m) => (
                      <button
                        key={`${m.vendor}/${m.model_id}`}
                        type="button"
                        onClick={() => {
                          setDraft((prev) => prev ? {
                            ...prev,
                            modelId: prev.isNew ? m.model_id : prev.modelId,
                            displayName: m.display_name || m.model_id,
                            input: String(m.input),
                            output: String(m.output),
                            cacheRead: String(m.cache_read),
                            cacheWrite: m.cache_write !== null ? String(m.cache_write) : "",
                            cacheWrite1h: m.cache_write_1h !== null ? String(m.cache_write_1h) : "",
                            showLongCtx: m.long_ctx_threshold !== null,
                            longCtxThreshold: m.long_ctx_threshold !== null ? String(m.long_ctx_threshold) : "",
                            longCtxInput: m.long_ctx_input !== null ? String(m.long_ctx_input) : "",
                            longCtxOutput: m.long_ctx_output !== null ? String(m.long_ctx_output) : "",
                            longCtxCacheRead: m.long_ctx_cache_read !== null ? String(m.long_ctx_cache_read) : "",
                            longCtxCacheWrite: m.long_ctx_cache_write !== null ? String(m.long_ctx_cache_write) : "",
                          } : null)
                          setPickerOpen(false)
                        }}
                        className="jn-interactive flex items-center justify-between gap-2 rounded-[10px] px-2.5 py-1.5 text-left text-xs hover:bg-jn-primary-wash/60"
                      >
                        <span className="min-w-0 flex-1 truncate font-mono text-jn-text">
                          {m.model_id}
                          <span className="ml-2 font-sans text-note text-jn-text-3">{m.vendor}</span>
                        </span>
                        <span className="tnum shrink-0 text-note text-jn-text-2">
                          入 {fmtUsdUnit(m.input)} · 出 {fmtUsdUnit(m.output)} · 读 {fmtUsdUnit(m.cache_read)}
                        </span>
                      </button>
                    ))}
                    {!filteredMd.length && <span className="py-3 text-center text-note text-jn-text-3">无匹配的 models.dev 价目</span>}
                  </div>
                </div>
              )}

              <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
                <label className="flex flex-col gap-1">
                  <span className="text-note text-jn-text-3">模型 ID *</span>
                  <Input
                    size="md"
                    value={draft.modelId}
                    disabled={!draft.isNew}
                    onChange={(e) => setDraft({ ...draft, modelId: e.target.value })}
                    placeholder="如 claude-sonnet-4-5 或 glm-5.3"
                    className="border border-jn-line/80 bg-jn-surface"
                  />
                </label>
                <label className="flex flex-col gap-1">
                  <span className="text-note text-jn-text-3">显示名称</span>
                  <Input
                    size="md"
                    value={draft.displayName}
                    onChange={(e) => setDraft({ ...draft, displayName: e.target.value })}
                    placeholder="可选，留空同模型 ID"
                    className="border border-jn-line/80 bg-jn-surface"
                  />
                </label>
              </div>

              <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-5">
                <PriceInput label="输入 ($/M) *" value={draft.input} onChange={(v) => setDraft({ ...draft, input: v })} placeholder="3.00" />
                <PriceInput label="输出 ($/M) *" value={draft.output} onChange={(v) => setDraft({ ...draft, output: v })} placeholder="15.00" />
                <PriceInput label="缓存读 ($/M)" value={draft.cacheRead} onChange={(v) => setDraft({ ...draft, cacheRead: v })} placeholder="0.30" />
                <PriceInput label="缓存写 5m ($/M)" value={draft.cacheWrite} onChange={(v) => setDraft({ ...draft, cacheWrite: v })} placeholder="选填" />
                <PriceInput label="缓存写 1h ($/M)" value={draft.cacheWrite1h} onChange={(v) => setDraft({ ...draft, cacheWrite1h: v })} placeholder="Claude 默认 2×输入" />
              </div>

              <div className="flex items-center justify-between pt-1">
                <button
                  type="button"
                  onClick={() => setDraft({ ...draft, showLongCtx: !draft.showLongCtx })}
                  className="text-note font-medium text-jn-primary-ink hover:underline"
                >
                  {draft.showLongCtx ? "收起长上下文加价阶梯" : "+ 配置长上下文加价阶梯（如 >200K / >272K）"}
                </button>
                <span className="text-note text-jn-text-3">填入 0 表示免费模型（区别于未计价）</span>
              </div>

              {draft.showLongCtx && (
                <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-5">
                  <PriceInput label="超过 Token 阈值" value={draft.longCtxThreshold} onChange={(v) => setDraft({ ...draft, longCtxThreshold: v })} placeholder="200000" />
                  <PriceInput label="阶梯输入 ($/M)" value={draft.longCtxInput} onChange={(v) => setDraft({ ...draft, longCtxInput: v })} placeholder="留空同基础价" />
                  <PriceInput label="阶梯输出 ($/M)" value={draft.longCtxOutput} onChange={(v) => setDraft({ ...draft, longCtxOutput: v })} placeholder="留空同基础价" />
                  <PriceInput label="阶梯缓存读" value={draft.longCtxCacheRead} onChange={(v) => setDraft({ ...draft, longCtxCacheRead: v })} placeholder="留空同基础价" />
                  <PriceInput label="阶梯缓存写" value={draft.longCtxCacheWrite} onChange={(v) => setDraft({ ...draft, longCtxCacheWrite: v })} placeholder="留空同基础价" />
                </div>
              )}
            </div>
          )}

          <div className="flex flex-wrap items-center gap-2">
            <Input
              size="md"
              icon="search"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="搜索模型 ID、名称或厂商…"
              className="w-60 bg-jn-well"
            />
            <div className="flex flex-wrap items-center gap-1">
              {([
                ["all", "全部"],
                ["unpriced", `待补价 (${unpricedCount})`],
                ["user_custom", "自定义"],
                ["builtin", "内置规则"],
                ["models_dev", "models.dev"],
              ] as const).map(([key, label]) => (
                <button
                  key={key}
                  type="button"
                  onClick={() => setSourceFilter(key)}
                  className={cn(
                    "jn-interactive rounded-full px-2.5 py-1 text-note font-medium",
                    sourceFilter === key
                      ? "bg-jn-primary text-jn-on-primary"
                      : "bg-jn-well text-jn-text-2 hover:bg-jn-primary-wash",
                  )}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>

          <div className="flex min-h-[280px] flex-col gap-1 overflow-y-auto pr-1">
            <div className="flex h-7 shrink-0 items-center gap-2 px-2.5 text-[11px] font-medium text-jn-text-3">
              <span className="min-w-0 flex-1">模型</span>
              <span className="w-16 text-right">输入</span>
              <span className="w-16 text-right">输出</span>
              <span className="w-16 text-right">缓存读</span>
              <span className="w-16 text-right">缓存写</span>
              <span className="w-20 text-center">来源</span>
              <span className="w-32 text-right">操作</span>
            </div>
            {loading && !catalog ? (
              <div className="py-10 text-center text-xs text-jn-text-3">加载模型价目中…</div>
            ) : filteredEntries.slice(0, 160).map((entry, idx) => (
              <div
                key={entry.model_id}
                className={cn(
                  "tnum flex min-h-10 items-center gap-2 rounded-[12px] px-2.5 py-1.5 text-xs",
                  idx % 2 === 0 && "bg-jn-well",
                )}
              >
                <div className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate font-mono font-medium text-jn-text" title={entry.model_id}>
                    {entry.model_id}
                  </span>
                  <span className="truncate text-note text-jn-text-3">
                    {entry.vendor ? `${entry.vendor}` : entry.display_name}
                    {entry.recent_requests > 0 ? ` · 近 90 天 ${entry.recent_requests} 次` : ""}
                    {entry.long_ctx_threshold ? ` · >${Math.round(entry.long_ctx_threshold / 1000)}K 阶梯` : ""}
                  </span>
                </div>
                <span className="w-16 text-right text-jn-text">{fmtUsdUnit(entry.input)}</span>
                <span className="w-16 text-right text-jn-text">{fmtUsdUnit(entry.output)}</span>
                <span className="w-16 text-right text-jn-text-2">{fmtUsdUnit(entry.cache_read)}</span>
                <span className="w-16 text-right text-jn-text-2">{fmtUsdUnit(entry.cache_write)}</span>
                <span className="flex w-20 justify-center">
                  <SourceBadge kind={entry.source_kind} disabled={entry.disabled} />
                </span>
                <div className="flex w-32 items-center justify-end gap-1">
                  <Button kind="ghost" size="sm" onClick={() => setDraft(entryToDraft(entry))}>
                    {entry.source_kind === "unpriced" ? "补价" : "编辑"}
                  </Button>
                  {entry.source_kind === "unpriced" && !entry.disabled && (
                    <Button
                      kind="ghost"
                      size="sm"
                      onClick={() => {
                        setAliasModel(entry.model_id)
                        setTab("aliases")
                      }}
                    >
                      映射
                    </Button>
                  )}
                  {entry.has_override ? (
                    <Button kind="ghost" size="sm" onClick={() => void restoreDefault(entry.model_id)}>
                      还原
                    </Button>
                  ) : entry.source_kind !== "unpriced" ? (
                    <Button kind="ghost" size="sm" onClick={() => void disableEntry(entry.model_id)}>
                      停用
                    </Button>
                  ) : null}
                </div>
              </div>
            ))}
            {!loading && filteredEntries.length === 0 && (
              <div className="py-10 text-center text-xs text-jn-text-3">没有匹配的模型价目</div>
            )}
            {filteredEntries.length > 160 && (
              <div className="py-2 text-center text-note text-jn-text-3">
                已显示前 160 条（共 {filteredEntries.length} 条），请输入关键词缩小范围
              </div>
            )}
          </div>
        </>
      )}

      {tab === "aliases" && (
        <div className="flex flex-col gap-3">
          <p className="text-xs text-jn-text-3">
            当第三方中转或私服返回了自定义模型名（如 <span className="font-mono text-jn-text">my-sonnet-fast</span>）时，可映射到已知基准模型复用其价格与阶梯规则；不改写历史原始模型名。
          </p>
          <div className="grid grid-cols-1 gap-2 rounded-[16px] bg-jn-well p-3.5 sm:grid-cols-[160px_1fr_1fr_auto]">
            <SelectField
              label="映射作用域"
              value={aliasScope}
              onValueChange={setAliasScope}
              options={scopeOptions}
            />
            <Input
              size="md"
              value={aliasModel}
              onChange={(e) => setAliasModel(e.target.value)}
              placeholder="中转/自定义模型名"
              className="border border-jn-line/80 bg-jn-surface"
            />
            <Input
              size="md"
              value={targetModel}
              onChange={(e) => setTargetModel(e.target.value)}
              placeholder="目标基准模型（如 claude-sonnet-4-5）"
              className="border border-jn-line/80 bg-jn-surface"
            />
            <Button kind="primary" size="sm" loading={savingAlias} onClick={() => void addAlias()}>
              添加映射
            </Button>
          </div>
          <div className="flex max-h-72 flex-col gap-1 overflow-y-auto">
            {(catalog?.aliases ?? []).map((item) => (
              <div key={`${item.scope}:${item.alias_model}`} className="flex items-center gap-2 rounded-[12px] bg-jn-well px-3 py-2 text-xs">
                <StatusPill tone="neutral">{item.scope === "*" ? "全局" : (connMap.get(item.scope) ?? item.scope)}</StatusPill>
                <span className="font-mono font-medium text-jn-text">{item.alias_model}</span>
                <span className="text-jn-text-3">→</span>
                <span className="min-w-0 flex-1 truncate font-mono text-jn-primary-ink">{item.target_model}</span>
                <Button kind="ghost" size="sm" onClick={() => void removeAlias(item.scope, item.alias_model)}>删除</Button>
              </div>
            ))}
            {!(catalog?.aliases.length) && (
              <div className="py-8 text-center text-xs text-jn-text-3">暂无同价映射规则</div>
            )}
          </div>
        </div>
      )}

      {tab === "budgets" && budgetDraft && budgetStatus && (() => {
        const previewRate = fxMode === "fixed" && Number(fxFixedInput) >= 1 && Number(fxFixedInput) <= 20
          ? Number(fxFixedInput)
          : (fx.rate > 1 ? fx.rate : 7.2)
        const dailyNum = Number(budgetDraft.globalDaily)
        const monthlyNum = Number(budgetDraft.globalMonthly)
        return (
          <div className="flex flex-col gap-3">
            <div className="flex flex-col gap-3 rounded-[16px] bg-jn-well p-4">
              <div className="flex items-center justify-between gap-3">
                <div className="flex items-center gap-2">
                  <Icon name="gauge" size="sm" className="text-jn-primary" />
                  <div className="flex flex-col gap-0.5">
                    <span className="text-[13px] font-semibold text-jn-text">全局实付 API 预算</span>
                    <span className="text-note text-jn-text-3">仅汇总实付 API 花费，不将订阅等价消耗或积分混入告警。</span>
                  </div>
                </div>
                <Switch
                  checked={budgetDraft.globalEnabled}
                  onChange={(checked) => setBudgetDraft({ ...budgetDraft, globalEnabled: checked })}
                  label="启用全局实付预算"
                />
              </div>
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                <label className="flex flex-col gap-1">
                  <span className="text-xs font-medium text-jn-text-2">日上限 (USD)</span>
                  <Input
                    size="md"
                    value={budgetDraft.globalDaily}
                    disabled={!budgetDraft.globalEnabled}
                    onChange={(e) => setBudgetDraft({ ...budgetDraft, globalDaily: e.target.value })}
                    placeholder="留空不限，如 10.00"
                    className="border border-jn-line bg-jn-surface font-mono"
                  />
                  <span className="text-note text-jn-text-3">
                    {Number.isFinite(dailyNum) && dailyNum > 0 ? `≈ ¥${(dailyNum * previewRate).toFixed(2)} · ` : ""}今日已用 ${budgetStatus.global_today_spend.toFixed(2)}
                  </span>
                </label>
                <label className="flex flex-col gap-1">
                  <span className="text-xs font-medium text-jn-text-2">月上限 (USD)</span>
                  <Input
                    size="md"
                    value={budgetDraft.globalMonthly}
                    disabled={!budgetDraft.globalEnabled}
                    onChange={(e) => setBudgetDraft({ ...budgetDraft, globalMonthly: e.target.value })}
                    placeholder="留空不限，如 60.00"
                    className="border border-jn-line bg-jn-surface font-mono"
                  />
                  <span className="text-note text-jn-text-3">
                    {Number.isFinite(monthlyNum) && monthlyNum > 0 ? `≈ ¥${(monthlyNum * previewRate).toFixed(2)} · ` : ""}本月已用 ${budgetStatus.global_month_spend.toFixed(2)}
                  </span>
                </label>
                <label className="flex flex-col gap-1">
                  <span className="text-xs font-medium text-jn-text-2">预警阈值 (%)</span>
                  <Input
                    size="md"
                    value={budgetDraft.warnPercent}
                    disabled={!budgetDraft.globalEnabled}
                    onChange={(e) => setBudgetDraft({ ...budgetDraft, warnPercent: e.target.value })}
                    placeholder="80"
                    className="border border-jn-line bg-jn-surface font-mono"
                  />
                  <span className="text-note text-jn-text-3">达到比例时发送系统通知（50–99%）</span>
                </label>
              </div>
            </div>

            <div className="flex flex-col gap-1.5">
              <div className="flex items-center gap-2 px-3 text-[11.5px] font-medium text-jn-text-3">
                <span className="w-[176px] shrink-0">连接</span>
                <span className="w-[82px] shrink-0">口径</span>
                <span className="min-w-0 flex-1">本月进度</span>
                <span className="w-[152px] shrink-0">日 / 月上限 (USD)</span>
                <span className="w-[156px] shrink-0 text-right">超限动作</span>
              </div>
              <div className="flex max-h-60 flex-col gap-1.5 overflow-y-auto pr-0.5">
                {budgetStatus.connections.map((row, idx) => {
                  const rowDraft = budgetDraft.connections[row.connection_id] ?? { daily: "", monthly: "", action: "alert" as const }
                  const lim = row.monthly_limit ?? row.daily_limit
                  const spent = row.monthly_limit ? row.month_spend : row.today_spend
                  const pct = lim && lim > 0 ? Math.min(100, (spent / lim) * 100) : 0
                  const modeTag = row.billing_mode === "free"
                    ? "免费"
                    : row.billing_mode === "subscription"
                      ? "订阅等价"
                      : Math.abs(row.price_multiplier - 1) > 1e-4
                        ? `API · ${row.price_multiplier}×`
                        : "API"
                  const updateConn = (patch: Partial<{ daily: string; monthly: string; action: "alert" | "block" }>) => {
                    setBudgetDraft((prev) => prev ? {
                      ...prev,
                      connections: { ...prev.connections, [row.connection_id]: { ...rowDraft, ...patch } },
                    } : null)
                  }
                  return (
                    <div
                      key={row.connection_id}
                      className={cn(
                        "flex min-h-[52px] items-center gap-2 rounded-[14px] px-3 py-1.5 text-xs",
                        idx % 2 === 0 ? "bg-jn-well" : "border border-jn-line bg-jn-surface",
                      )}
                    >
                      <span className="flex w-[176px] shrink-0 items-center gap-2">
                        <PlatformLogo platform={row.platform as PlatformId} size={16} />
                        <span className="truncate font-semibold text-jn-text" title={row.name}>{row.name}</span>
                      </span>
                      <span className="w-[82px] shrink-0">
                        <span className="inline-block rounded-full border border-jn-line bg-jn-surface px-2 py-0.5 font-mono text-[10.5px] font-semibold text-jn-text-2">
                          {modeTag}
                        </span>
                      </span>
                      <div className="flex min-w-0 flex-1 flex-col gap-1 pr-2">
                        <span className={cn("tnum font-mono text-[11px]", row.billing_mode === "api" ? "text-jn-text-2" : "text-jn-text-3")}>
                          {row.billing_mode !== "api"
                            ? "不计入实付"
                            : lim
                              ? `$${spent.toFixed(2)} / $${lim.toFixed(2)}`
                              : `$${row.month_spend.toFixed(2)} · 未设限`}
                        </span>
                        {row.billing_mode === "api" && lim && (
                          <div className="h-1.5 w-full overflow-hidden rounded-full bg-jn-surface">
                            <div
                              className={cn("h-full rounded-full", row.exceeded ? "bg-jn-danger" : row.warned ? "bg-jn-warning" : "bg-jn-primary")}
                              style={{ width: `${Math.max(pct > 0 ? 3 : 0, pct)}%` }}
                            />
                          </div>
                        )}
                      </div>
                      <div className="flex w-[152px] shrink-0 items-center gap-1.5">
                        <Input
                          size="md"
                          value={rowDraft.daily}
                          onChange={(e) => updateConn({ daily: e.target.value })}
                          placeholder="日"
                          aria-label={`${row.name} 日上限`}
                          className="h-8 w-[72px] rounded-[10px] border border-jn-line bg-jn-surface px-2 font-mono text-xs"
                        />
                        <Input
                          size="md"
                          value={rowDraft.monthly}
                          onChange={(e) => updateConn({ monthly: e.target.value })}
                          placeholder="月"
                          aria-label={`${row.name} 月上限`}
                          className="h-8 w-[72px] rounded-[10px] border border-jn-line bg-jn-surface px-2 font-mono text-xs"
                        />
                      </div>
                      <div className="flex w-[156px] shrink-0 justify-end">
                        {row.serve_enabled ? (
                          <SelectField
                            label={`${row.name} 超限动作`}
                            value={rowDraft.action}
                            onValueChange={(v) => updateConn({ action: v === "block" ? "block" : "alert" })}
                            options={[
                              { value: "alert", label: "仅系统提醒" },
                              { value: "block", label: "网关硬限额 429" },
                            ]}
                            className="h-8 text-xs"
                          />
                        ) : (
                          <span className="text-[11px] text-jn-text-3">仅提醒 · 本机直连</span>
                        )}
                      </div>
                    </div>
                  )
                })}
                {budgetStatus.connections.length === 0 && (
                  <div className="py-6 text-center text-xs text-jn-text-3">暂无已保存连接</div>
                )}
              </div>
            </div>

            <div className="flex items-center gap-2 rounded-[12px] bg-jn-info-soft px-3.5 py-2.5 text-[11.5px] text-jn-text-2">
              <Icon name="info" size="sm" className="shrink-0 text-jn-info" />
              <span>只有开启「模型路由」的连接支持网关 429 硬限额；本机直连日志为事后扫描，仅触发超额提醒。</span>
            </div>
          </div>
        )
      })()}

      {tab === "rules" && (() => {
        const previewRate = fxMode === "fixed" && Number(fxFixedInput) >= 1 && Number(fxFixedInput) <= 20
          ? Number(fxFixedInput)
          : (fx.liveRate ?? (fx.rate > 1 ? fx.rate : 7.2))
        const sampleUsd = budgetStatus?.global_month_spend ?? 0
        return (
          <div className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-[16px] bg-jn-well p-4">
              <div className="flex flex-col gap-0.5">
                <span className="text-[13px] font-semibold text-jn-text">网关路由计价模型来源</span>
                <span className="text-note text-jn-text-3">
                  模型路由把客户端请求转发到其他替代模型时，决定按哪一方模型单价核算费用。
                </span>
              </div>
              <Tabs
                style="segment"
                value={catalog?.pricing_model_source === "request" ? "request" : "upstream"}
                onChange={(next) => void updateRules(next as "upstream" | "request", null)}
                className="bg-jn-track"
                items={[
                  { key: "upstream", label: "按实际路由/返回" },
                  { key: "request", label: "按客户端请求" },
                ]}
              />
            </div>

            <div className="flex items-center justify-between gap-3 rounded-[16px] bg-jn-well p-4">
              <div className="flex flex-col gap-0.5">
                <span className="text-[13px] font-semibold text-jn-text">models.dev 自动同步</span>
                <span className="text-note text-jn-text-3">
                  每 24 小时后台刷新，遇未收录模型自动补拉（6 小时冷却）；已同步 {catalog?.models_dev_count ?? 0} 个带价模型。
                </span>
              </div>
              <Switch
                checked={catalog?.models_dev_auto_sync ?? true}
                onChange={(checked) => void updateRules(null, checked)}
                label="自动同步 models.dev 价格"
              />
            </div>

            <div className="flex flex-col gap-3 rounded-[16px] bg-jn-well p-4">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex flex-col gap-0.5">
                  <span className="text-[13px] font-semibold text-jn-text">USD → CNY 汇率来源</span>
                  <span className="text-note text-jn-text-3">
                    支持每日在线汇率或手动锁定固定结算汇率；锁定后全站按固定汇率换算，离线不报参考价。
                  </span>
                </div>
                <Tabs
                  style="segment"
                  value={fxMode}
                  onChange={(next) => setFxMode(next as "auto" | "fixed")}
                  className="bg-jn-track"
                  items={[
                    { key: "auto", label: "自动汇率" },
                    { key: "fixed", label: "自定义固定" },
                  ]}
                />
              </div>

              <div className="flex flex-wrap items-center gap-3">
                {fxMode === "fixed" && (
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-xs font-semibold text-jn-text-2">1 USD =</span>
                    <Input
                      size="md"
                      value={fxFixedInput}
                      onChange={(e) => setFxFixedInput(e.target.value)}
                      placeholder="7.25"
                      aria-label="固定汇率值"
                      className="h-9 w-28 border border-jn-primary bg-jn-surface font-mono font-bold"
                    />
                    <span className="font-mono text-xs font-semibold text-jn-text-2">CNY</span>
                  </div>
                )}
                <div className="flex min-h-9 min-w-[240px] flex-1 items-center gap-2 rounded-[12px] bg-jn-primary-wash px-3 text-[11.5px] font-medium text-jn-primary-ink">
                  <Icon name="diff" size="sm" className="shrink-0 text-jn-primary-ink" />
                  <span className="tnum">
                    {sampleUsd > 0
                      ? `本月已用 ${formatMoney(sampleUsd, "USD")} ≈ ¥${(sampleUsd * previewRate).toFixed(2)}`
                      : `$10.00 ≈ ¥${(10 * previewRate).toFixed(2)}`}
                    {fx.liveRate ? ` · 实时参考汇率 ${fx.liveRate.toFixed(2)}` : ""}
                  </span>
                </div>
              </div>
            </div>

            <div className="flex flex-col gap-2 rounded-[16px] bg-jn-well p-4">
              <div className="flex items-center justify-between">
                <span className="text-xs font-semibold text-jn-text">已自定义倍率或计费口径的连接（{customConnections.length}）</span>
                <span className="text-note text-jn-text-3">在「供应商 → 编辑连接」中修改</span>
              </div>
              {customConnections.length ? (
                <div className="flex flex-wrap gap-2">
                  {customConnections.map((c) => (
                    <div key={c.id} className="flex items-center gap-2 rounded-[12px] border border-jn-line bg-jn-surface px-3 py-1.5 text-xs">
                      <span className="font-semibold text-jn-text">{c.baseName}</span>
                      <span className="tnum font-mono text-[11px] text-jn-text-2">
                        {c.billingMode === "free" ? "免费 / $0" : c.billingMode === "subscription" ? "订阅等价" : c.billingMode === "api" ? "API 实付" : "自动"}
                        {c.billingMode !== "free" && ` · ${c.priceMultiplier ?? 1}×`}
                      </span>
                    </div>
                  ))}
                </div>
              ) : (
                <span className="text-note text-jn-text-3">当前全部连接均使用默认计费口径与 1.0× 倍率。</span>
              )}
            </div>
          </div>
        )
      })()}
    </Dialog>
  )
}

function PriceInput({ label, value, onChange, placeholder }: {
  label: string
  value: string
  onChange: (value: string) => void
  placeholder: string
}) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-note text-jn-text-3">{label}</span>
      <Input
        size="md"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="border border-jn-line/80 bg-jn-surface"
      />
    </label>
  )
}

function SourceBadge({ kind, disabled }: { kind: string; disabled: boolean }) {
  if (disabled) return <StatusPill tone="neutral">已停用</StatusPill>
  if (kind === "user_custom") return <StatusPill tone="success">自定义</StatusPill>
  if (kind === "builtin") return <StatusPill tone="info">内置规则</StatusPill>
  if (kind === "models_dev") return <StatusPill tone="neutral">models.dev</StatusPill>
  return <StatusPill tone="warning">未收录</StatusPill>
}
