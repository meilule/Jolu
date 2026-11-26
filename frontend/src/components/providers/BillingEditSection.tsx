import { useCallback, useEffect, useState } from "react"
import { api, isTauri, type PricingOverrideDto } from "@/lib/api"
import { SelectField } from "@/components/ui/SelectField"
import { useToast } from "@/components/ui/Toast"
import { Button, Input } from "@/ui"
import type { Connection } from "@/types"

const BILLING_OPTIONS = [
  { value: "auto", label: "自动（API 按量 / 订阅按等价）" },
  { value: "api", label: "API 实付（计入实付支出）" },
  { value: "subscription", label: "订阅等价（参考市场 API 价）" },
  { value: "free", label: "免费 / 计 $0（明确计为免费）" },
]

const PRESET_MULTIPLIERS = [0.2, 0.5, 1, 1.5]

/** 连接编辑弹窗内的「计费口径与倍率」分区：改动独立保存，实时重算全部统计费用。 */
export function BillingEditSection({ account, onChanged }: {
  account: Connection
  onChanged: () => void
}) {
  const toast = useToast()
  const [mode, setMode] = useState<"auto" | "api" | "subscription" | "free">(account.billingMode ?? "auto")
  const [multDraft, setMultDraft] = useState(String(account.priceMultiplier ?? 1))
  const [saving, setSaving] = useState(false)

  const [overrides, setOverrides] = useState<PricingOverrideDto[]>([])
  const [adding, setAdding] = useState(false)
  const [modelId, setModelId] = useState("")
  const [inPrice, setInPrice] = useState("")
  const [outPrice, setOutPrice] = useState("")
  const [crPrice, setCrPrice] = useState("0")

  const loadOverrides = useCallback(async () => {
    if (!isTauri) return
    try {
      const cat = await api.pricingCatalog()
      setOverrides(cat.connection_overrides.filter((o) => o.scope === account.id))
    } catch {
      // 忽略静默错误
    }
  }, [account.id])

  useEffect(() => {
    setMode(account.billingMode ?? "auto")
    setMultDraft(String(account.priceMultiplier ?? 1))
    void loadOverrides()
  }, [account.id, account.billingMode, account.priceMultiplier, loadOverrides])

  const parsedMult = Number(multDraft.trim())
  const multValid = Number.isFinite(parsedMult) && parsedMult >= 0 && parsedMult <= 1000
  const dirty = mode !== (account.billingMode ?? "auto") || (multValid && parsedMult !== (account.priceMultiplier ?? 1))

  const saveBilling = async () => {
    if (!isTauri) return toast.info("浏览器预览仅演示界面")
    if (!multValid) return toast.warn("价格倍率须在 0 到 1000 之间")
    setSaving(true)
    try {
      await api.connectionUpdateBilling(account.id, mode, mode === "free" ? 1 : parsedMult)
      toast.success("连接计费规则已保存，费用统计已刷新")
      onChanged()
    } catch (err) {
      toast.danger("保存计费规则失败", String(err))
    } finally {
      setSaving(false)
    }
  }

  const saveOverride = async () => {
    if (!isTauri) return
    const mid = modelId.trim()
    const i = Number(inPrice.trim())
    const o = Number(outPrice.trim())
    const cr = crPrice.trim() ? Number(crPrice.trim()) : 0
    if (!mid || !Number.isFinite(i) || i < 0 || !Number.isFinite(o) || o < 0 || !Number.isFinite(cr) || cr < 0) {
      return toast.warn("请填写模型 ID（或 *）和有效的非负美元单价")
    }
    try {
      await api.pricingUpsertOverride({
        scope: account.id,
        model_id: mid,
        display_name: mid,
        input: i,
        output: o,
        cache_read: cr,
        cache_write: null,
        cache_write_1h: null,
        long_ctx_threshold: null,
        long_ctx_input: null,
        long_ctx_output: null,
        long_ctx_cache_read: null,
        long_ctx_cache_write: null,
      })
      toast.success(`已为本连接设置「${mid}」专属到手价`)
      setModelId("")
      setInPrice("")
      setOutPrice("")
      setCrPrice("0")
      setAdding(false)
      await loadOverrides()
    } catch (err) {
      toast.danger("保存专属价目失败", String(err))
    }
  }

  const deleteOverride = async (mid: string) => {
    if (!isTauri) return
    try {
      await api.pricingDeleteOverride(account.id, mid)
      toast.success(`已移除本连接的「${mid}」专属价目`)
      await loadOverrides()
    } catch (err) {
      toast.danger("移除失败", String(err))
    }
  }

  return (
    <div className="flex flex-col gap-2.5 rounded-xl border border-jn-line bg-jn-well p-3.5">
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-semibold text-jn-text">费用计费与倍率</span>
        <Button kind="primary" size="sm" disabled={!dirty || !multValid || saving} loading={saving} onClick={() => void saveBilling()}>
          保存计费
        </Button>
      </div>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-[1fr_140px]">
        <div className="flex flex-col gap-1">
          <span className="text-note text-jn-text-3">计费口径</span>
          <SelectField
            label="连接计费口径"
            value={mode}
            onValueChange={(v) => setMode(v as "auto" | "api" | "subscription" | "free")}
            options={BILLING_OPTIONS}
          />
        </div>
        <div className="flex flex-col gap-1">
          <span className="text-note text-jn-text-3">价格倍率</span>
          <Input
            size="md"
            value={multDraft}
            disabled={mode === "free"}
            onChange={(e) => setMultDraft(e.target.value)}
            placeholder="1.0"
            className="border border-jn-line/80 bg-jn-surface"
          />
        </div>
      </div>
      {mode !== "free" && (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-note text-jn-text-3">常用倍率：</span>
          {PRESET_MULTIPLIERS.map((preset) => (
            <button
              key={preset}
              type="button"
              onClick={() => setMultDraft(String(preset))}
              className="jn-interactive rounded-full bg-jn-surface px-2 py-0.5 text-note text-jn-text-2 hover:bg-jn-primary-wash hover:text-jn-primary-ink"
            >
              {preset}×
            </button>
          ))}
          <span className="text-note text-jn-text-3">仅缩放目录与全局基准价</span>
        </div>
      )}

      <div className="flex flex-col gap-1.5 border-t border-jn-line pt-2">
        <div className="flex items-center justify-between">
          <span className="text-note text-jn-text-3">连接专属到手价（不乘倍率，可用 * 覆盖全模型）</span>
          <button
            type="button"
            onClick={() => setAdding((v) => !v)}
            className="text-note font-medium text-jn-primary-ink hover:underline"
          >
            {adding ? "收起" : "+ 添加专属价"}
          </button>
        </div>
        {adding && (
          <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-[1.4fr_1fr_1fr_1fr_auto]">
            <Input size="md" value={modelId} onChange={(e) => setModelId(e.target.value)} placeholder="模型 ID 或 *" className="border border-jn-line/80 bg-jn-surface" />
            <Input size="md" value={inPrice} onChange={(e) => setInPrice(e.target.value)} placeholder="输入 $/M" className="border border-jn-line/80 bg-jn-surface" />
            <Input size="md" value={outPrice} onChange={(e) => setOutPrice(e.target.value)} placeholder="输出 $/M" className="border border-jn-line/80 bg-jn-surface" />
            <Input size="md" value={crPrice} onChange={(e) => setCrPrice(e.target.value)} placeholder="缓存读 $/M" className="border border-jn-line/80 bg-jn-surface" />
            <Button kind="primary" size="sm" onClick={() => void saveOverride()}>添加</Button>
          </div>
        )}
        {overrides.map((ov) => (
          <div key={ov.model_id} className="tnum flex items-center justify-between gap-2 rounded-lg bg-jn-surface px-2.5 py-1 text-note">
            <span className="truncate font-mono text-jn-text">{ov.model_id}</span>
            <span className="text-jn-text-2">入 ${ov.input} · 出 ${ov.output} · 读 ${ov.cache_read}</span>
            <button type="button" onClick={() => void deleteOverride(ov.model_id)} className="text-jn-danger hover:underline">移除</button>
          </div>
        ))}
      </div>
    </div>
  )
}
