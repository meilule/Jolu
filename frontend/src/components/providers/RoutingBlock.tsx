import { useCallback, useEffect, useState } from "react"
import { Activity, RefreshCw, Route } from "lucide-react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { gatewayApi, type CatalogModelDto, type GroupMemberDto, type SourceUsageDto } from "@/lib/gatewayApi"
import { useToast } from "@/components/ui/Toast"
import type { Connection } from "@/types"

/**
 * 供应商里的「模型路由」区块（Jolu.pen「统一网关 · R2 重设计」V5-5）：
 * 每个连接自己决定是否给模型路由用、菜单前缀是什么。
 * ① 账号卡多一行（开关 + 前缀 + 模型数 · 分组数）；② 编辑连接里的模型路由分区。
 * 待确认：旧前缀保留多久按现有 slug_history 规则；账号卡高度增加约 50px，四列卡片需整体复核。
 */

export interface RoutingIndex {
  modelsBySlug: Map<string, string[]>
  /** 连接 slug → 用到它的分组（含在分组里的位次，1 起） */
  groupsBySlug: Map<string, { slug: string; position: number }[]>
  defaultConnection: string | null
  usage: SourceUsageDto[]
  reload: () => void
}

/** 模型池目录与路由分组的索引；账号卡与编辑弹窗共用一份。 */
export function useRoutingIndex(): RoutingIndex {
  const [catalog, setCatalog] = useState<CatalogModelDto[]>([])
  const [groups, setGroups] = useState<{ slug: string; members: GroupMemberDto[] }[]>([])
  const [defaultConnection, setDefault] = useState<string | null>(null)
  const [usage, setUsage] = useState<SourceUsageDto[]>([])
  const reload = useCallback(() => {
    if (!isTauri) return
    void gatewayApi.catalog().then(setCatalog).catch(() => setCatalog([]))
    void gatewayApi.groupList().then(setGroups).catch(() => setGroups([]))
    void gatewayApi.status().then((status) => setDefault(status.default_connection)).catch(() => undefined)
    void gatewayApi.sourceUsage().then(setUsage).catch(() => setUsage([]))
  }, [])
  useEffect(() => { reload() }, [reload])
  const modelsBySlug = new Map<string, string[]>()
  for (const model of catalog) {
    if (!model.slug) continue
    modelsBySlug.set(model.slug, [...(modelsBySlug.get(model.slug) ?? []), model.model])
  }
  const groupsBySlug = new Map<string, { slug: string; position: number }[]>()
  for (const group of groups) {
    group.members.forEach((member, index) => {
      const list = groupsBySlug.get(member.slug) ?? []
      if (!list.some((item) => item.slug === group.slug)) list.push({ slug: group.slug, position: index + 1 })
      groupsBySlug.set(member.slug, list)
    })
  }
  return { modelsBySlug, groupsBySlug, defaultConnection, usage, reload }
}

function useServeToggle(account: Connection, onChanged: () => void) {
  const toast = useToast()
  const [busy, setBusy] = useState(false)
  const toggle = async () => {
    if (!isTauri || busy) return
    setBusy(true)
    try {
      await gatewayApi.setServe(account.id, !account.serve_enabled)
      toast.success(account.serve_enabled ? `已把 ${account.baseName} 移出模型池` : `已把 ${account.baseName} 加入模型池`)
      onChanged()
    } catch (error) {
      toast.danger("操作失败", String(error))
    } finally {
      setBusy(false)
    }
  }
  return { busy, toggle }
}

function RouteSwitch({ on, busy, label, onClick }: { on: boolean; busy: boolean; label: string; onClick: () => void }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} disabled={busy || !isTauri} onClick={onClick}
      className={cn("flex h-4 w-7 shrink-0 items-center rounded-full p-0.5 outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:opacity-45", on ? "justify-end bg-jolu-accent" : "justify-start bg-jolu-track")}>
      <span className="size-3 rounded-full bg-white shadow-sm" />
    </button>
  )
}

/** ① 账号卡的模型路由行（画板 V5-5 ①） */
export function RoutingCardRow({ account, index, readOnly, onChanged }: { account: Connection; index: RoutingIndex; readOnly?: boolean; onChanged: () => void }) {
  const { busy, toggle } = useServeToggle(account, () => { index.reload(); onChanged() })
  const slug = account.slug ?? ""
  const models = slug ? index.modelsBySlug.get(slug)?.length ?? 0 : 0
  const groups = slug ? index.groupsBySlug.get(slug)?.length ?? 0 : 0
  return (
    <div className="flex shrink-0 flex-col gap-1 rounded-lg bg-jolu-side px-2.5 py-2" data-routing-row>
      <div className="flex items-center gap-1.5">
        <Route aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />
        <span className="flex-1 text-[12px] text-jolu-text">模型路由</span>
        <RouteSwitch on={account.serve_enabled} busy={busy || !!readOnly} label={account.serve_enabled ? "移出模型池" : "加入模型池"} onClick={() => void toggle()} />
      </div>
      {account.serve_enabled && (
        <div className="flex min-w-0 items-center gap-1.5">
          <code className="shrink-0 rounded-[5px] bg-jolu-track px-1.5 py-px font-mono text-[11px] text-jolu-text">{slug || "—"}/</code>
          <span className="truncate text-[11px] text-jolu-muted">{models} 个模型{groups ? ` · ${groups} 个分组在用` : ""}</span>
        </div>
      )}
    </div>
  )
}

/** ② 编辑连接里的模型路由分区（画板 V5-5 ②）：改动立即生效，不跟随弹窗的「保存」。 */
export function RoutingEditSection({ account, onChanged }: { account: Connection; onChanged: () => void }) {
  const toast = useToast()
  const index = useRoutingIndex()
  const { busy: toggling, toggle } = useServeToggle(account, () => { index.reload(); onChanged() })
  const [prefix, setPrefix] = useState(account.slug ?? "")
  const [working, setWorking] = useState(false)
  useEffect(() => setPrefix(account.slug ?? ""), [account.slug])
  const slug = account.slug ?? ""
  const models = slug ? index.modelsBySlug.get(slug) ?? [] : []
  const used = slug ? index.groupsBySlug.get(slug) ?? [] : []
  const usage = index.usage.find((row) => row.connection_id === account.id)
  const isDefault = index.defaultConnection === account.id
  const act = async (work: () => Promise<unknown>, ok: string) => {
    setWorking(true)
    try {
      await work()
      toast.success(ok)
      index.reload()
      onChanged()
    } catch (error) {
      toast.danger("操作失败", String(error))
    } finally {
      setWorking(false)
    }
  }
  const busy = toggling || working || !isTauri
  const firstModel = models[0] ?? "模型"
  return (
    <section className="flex flex-col gap-3 rounded-xl border border-jolu-border p-3.5" aria-label="模型路由">
      <div className="flex items-center gap-2.5">
        <Route aria-hidden className="size-4 shrink-0 text-jolu-muted" />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-[13px] font-semibold text-jolu-text">模型路由</span>
          <span className="text-[12px] text-jolu-muted">加入模型池后，接入的 Agent 可以用这个账号的模型</span>
        </span>
        <RouteSwitch on={account.serve_enabled} busy={busy} label={account.serve_enabled ? "移出模型池" : "加入模型池"} onClick={() => void toggle()} />
      </div>
      {account.serve_enabled && (
        <>
          <Field label="菜单前缀">
            <form className="flex items-center gap-2" onSubmit={(event) => { event.preventDefault(); const next = prefix.trim(); if (next && next !== slug) void act(() => gatewayApi.setSlug(account.id, next), "已更新菜单前缀") }}>
              <input value={prefix} onChange={(event) => setPrefix(event.target.value)} aria-label="菜单前缀" spellCheck={false}
                className="h-8 w-40 rounded-lg border border-jolu-border bg-jolu-bg px-2.5 font-mono text-[12px] text-jolu-text outline-none focus:border-jolu-accent" />
              <button type="submit" disabled={busy || !prefix.trim() || prefix.trim() === slug} className="text-[12px] font-medium text-jolu-on disabled:opacity-40">保存前缀</button>
            </form>
            <span className="text-[11px] text-jolu-muted">Agent 里显示为 {prefix.trim() || slug}/{firstModel}；改前缀后旧名称继续可用（沿用现有改名记录）</span>
          </Field>
          <Field label="提供的模型">
            <div className="flex flex-wrap gap-1.5">
              {models.slice(0, 12).map((model) => <code key={model} className="rounded-md bg-jolu-side px-1.5 py-0.5 font-mono text-[11px] text-jolu-text">{model}</code>)}
              {models.length > 12 && <span className="text-[11px] text-jolu-muted">等 {models.length} 个</span>}
              {models.length === 0 && <span className="text-[12px] text-jolu-muted">还没有拉到模型清单</span>}
            </div>
            <button type="button" disabled={busy} onClick={() => void act(() => gatewayApi.refreshModels(account.id), "模型清单已更新")}
              className="flex items-center gap-1 self-start text-[11px] text-jolu-muted hover:text-jolu-text disabled:opacity-40">
              <RefreshCw aria-hidden className="size-3" />按账号权限自动获取 · 刷新
            </button>
          </Field>
          <Field label="用在哪里">
            {used.length === 0
              ? <span className="text-[12px] text-jolu-muted">还没有路由分组用到它；也可以按「{slug}/模型」直接调用</span>
              : <div className="flex flex-wrap items-center gap-2">{used.map((item) => (
                <span key={item.slug} className="flex items-center gap-1 text-[11px] text-jolu-muted">
                  <code className="rounded-md bg-jolu-side px-1.5 py-0.5 font-mono text-jolu-text">group/{item.slug}</code>第 {item.position} 位
                </span>
              ))}</div>}
          </Field>
          <Field label="默认来源">
            <label className="flex items-center gap-2 text-[12px] text-jolu-text">
              <input type="radio" checked={isDefault} disabled={busy} onChange={() => void act(() => gatewayApi.setDefaultSource(account.id), "已设为默认来源")} className="accent-jolu-accent" />
              不带前缀的模型名落到这个账号
            </label>
            {isDefault && (
              <button type="button" disabled={busy} onClick={() => void act(() => gatewayApi.setDefaultSource(null), "已清除默认来源")} className="self-start text-[11px] text-jolu-muted hover:text-jolu-text">清除默认来源</button>
            )}
          </Field>
          <div className="flex items-center gap-2 rounded-lg bg-jolu-side px-3 py-2 text-[12px] text-jolu-muted">
            <Activity aria-hidden className="size-3.5" />
            <span className="flex-1">{usage ? `流水保留范围内经模型路由 ${usage.requests} 次${usage.failed ? ` · 失败 ${usage.failed} 次` : ""}` : "还没有经模型路由的请求"}</span>
          </div>
        </>
      )}
    </section>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-3">
      <span className="w-16 shrink-0 pt-1.5 text-[12px] text-jolu-muted">{label}</span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">{children}</div>
    </div>
  )
}
