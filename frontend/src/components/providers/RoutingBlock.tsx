import { useCallback, useEffect, useState } from "react"
import { Activity, ChevronDown, LoaderCircle, RefreshCw, Route } from "lucide-react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { gatewayApi, type CatalogModelDto, type GroupMemberDto, type SourceUsageDto } from "@/lib/gatewayApi"
import { useToast } from "@/components/ui/Toast"
import { JoluButton } from "@/components/ui/JoluButton"
import type { Connection } from "@/types"

/**
 * 供应商里的「模型路由」区块（Jolu.pen「统一网关 · R2 重设计」V5-5）：
 * 每个连接自己决定是否给模型路由用、菜单前缀是什么。
 * ① 账号卡多一行（开关 + 前缀 + 模型数 · 分组数）；② 编辑连接里的模型路由分区。
 * 待确认：旧前缀保留多久按现有 slug_history 规则；账号卡高度增加约 50px，四列卡片需整体复核。
 */

export interface RoutingIndex {
  modelsBySlug: Map<string, string[]>
  /** 连接 slug → (key → 官方 display_name)；Qoder 等来源的 key 是内部 id，名字在这 */
  labelsBySlug: Map<string, Map<string, string>>
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
  const labelsBySlug = new Map<string, Map<string, string>>()
  for (const model of catalog) {
    if (!model.slug) continue
    modelsBySlug.set(model.slug, [...(modelsBySlug.get(model.slug) ?? []), model.model])
    if (model.label) {
      const labels = labelsBySlug.get(model.slug) ?? new Map<string, string>()
      labels.set(model.model, model.label)
      labelsBySlug.set(model.slug, labels)
    }
  }
  const groupsBySlug = new Map<string, { slug: string; position: number }[]>()
  for (const group of groups) {
    group.members.forEach((member, index) => {
      const list = groupsBySlug.get(member.slug) ?? []
      if (!list.some((item) => item.slug === group.slug)) list.push({ slug: group.slug, position: index + 1 })
      groupsBySlug.set(member.slug, list)
    })
  }
  return { modelsBySlug, labelsBySlug, groupsBySlug, defaultConnection, usage, reload }
}

/**
 * 开 / 关模型路由。
 *
 * 打开时顺手把模型清单拉一次（只在还没有缓存时）：开关本身就是「这个来源要进池了」，
 * 池里没有模型清单的话它等于没进去——从本机导入的连接就只有 Key 和地址，没有清单。
 * 清单已经在的不重拉，免得每次开开关都等一次上游。
 *
 * 拉取失败**不回滚开关**：连接本身是好的，只是上游这次没答上来；失败原因弹提示，
 * 用户可在模型路由那一行点「拉取」重试。
 */
function useServeToggle(account: Connection, onChanged: () => void, hasModels: () => boolean) {
  const toast = useToast()
  const [busy, setBusy] = useState(false)
  const toggle = async () => {
    if (!isTauri || busy) return
    setBusy(true)
    const turningOn = !account.serve_enabled
    try {
      await gatewayApi.setServe(account.id, turningOn)
      toast.success(turningOn ? `已把 ${account.baseName} 加入模型池` : `已把 ${account.baseName} 移出模型池`)
      onChanged()
      if (turningOn && !hasModels()) {
        try {
          await gatewayApi.refreshModels(account.id)
          onChanged()
        } catch (error) {
          toast.danger("模型清单还没拉到", `${String(error)}。可以在模型路由里点「拉取」重试`)
        }
      }
    } catch (error) {
      toast.danger("操作失败", String(error))
    } finally {
      setBusy(false)
    }
  }
  return { busy, toggle }
}

/**
 * 模型路由行的模型数 + 拉取入口。
 *
 * 三种状态都在这一个位置表达，不再往账号卡塞第 6 个按钮：
 * - 还没拉到（0 个）→ 琥珀色「尚未拉取 · 点此拉取」，是这里最该做的事；
 * - 拉取中 → 转圈并禁用；
 * - 有清单 → 「N 个模型」，hover 才露出刷新图标（平时保持干净）。
 */
function ModelCountLine({ account, models, groups }: { account: Connection; models: number; groups: number }) {
  const toast = useToast()
  const [busy, setBusy] = useState(false)
  const refresh = async () => {
    if (!isTauri || busy) return
    setBusy(true)
    try {
      const list = await gatewayApi.refreshModels(account.id)
      toast.success(`已拉到 ${list.length} 个模型`)
    } catch (error) {
      toast.danger("拉取失败", String(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="flex min-w-0 items-center gap-1.5">
      <code className="shrink-0 rounded-[5px] bg-jolu-track px-1.5 py-px font-mono text-[11px] text-jolu-text">{account.slug || "—"}/</code>
      <button type="button" onClick={() => void refresh()} disabled={busy || !isTauri}
        aria-label={models ? `重新拉取模型清单，当前 ${models} 个` : "拉取模型清单"}
        className={cn(
          "group flex min-w-0 items-center gap-1 truncate rounded px-1 text-[11px] outline-none focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-jolu-accent disabled:opacity-50",
          models ? "text-jolu-muted hover:text-jolu-text" : "text-jolu-warn hover:brightness-95",
        )}>
        {busy
          ? <LoaderCircle aria-hidden className="size-3 shrink-0 animate-spin" />
          : !models && <RefreshCw aria-hidden className="size-3 shrink-0" />}
        <span className="truncate">
          {busy ? "正在拉取…" : models ? `${models} 个模型${groups ? ` · ${groups} 个分组在用` : ""}` : "尚未拉取 · 点此拉取"}
        </span>
        {models > 0 && !busy && <RefreshCw aria-hidden className="size-3 shrink-0 opacity-0 transition-opacity group-hover:opacity-100" />}
      </button>
    </div>
  )
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
  const slug = account.slug ?? ""
  const models = slug ? index.modelsBySlug.get(slug)?.length ?? 0 : 0
  const groups = slug ? index.groupsBySlug.get(slug)?.length ?? 0 : 0
  const { busy, toggle } = useServeToggle(account, () => { index.reload(); onChanged() }, () => models > 0)
  return (
    <div className="flex shrink-0 flex-col gap-1 rounded-lg bg-jolu-side px-2.5 py-2" data-routing-row>
      <div className="flex items-center gap-1.5">
        <Route aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />
        <span className="flex-1 text-[12px] text-jolu-text">模型路由</span>
        <RouteSwitch on={account.serve_enabled} busy={busy || !!readOnly} label={account.serve_enabled ? "移出模型池" : "加入模型池"} onClick={() => void toggle()} />
      </div>
      {account.serve_enabled && <ModelCountLine account={account} models={models} groups={groups} />}
    </div>
  )
}

/**
 * ② 编辑连接里的模型路由分区（画板 V5-5 ②）：改动立即生效，不跟随弹窗的「保存」。
 *
 * 做成可折叠分区，是为了不让模型数量决定弹窗高度：24 个模型全铺开会把下面的
 * 「用在哪里」「默认来源」挤到三屏开外，用户还没看到就以为弹窗卡住了。
 * 折叠时一行摘要（开关 + 前缀 + 模型数），展开后才出现细节；开／关本身不折叠，
 * 因为它是这个分区里最常点的东西。
 *
 * 字段骨架与弹窗上半部分一致（label 在上、控件在下）：原来这里是 label 在左的
 * 横向两列，和「连接名称」「API Key」的纵向表单混在一张表单里，读起来像两套界面。
 */
export function RoutingEditSection({ account, onChanged }: { account: Connection; onChanged: () => void }) {
  const toast = useToast()
  const index = useRoutingIndex()
  const { busy: toggling, toggle } = useServeToggle(account, () => { index.reload(); onChanged() }, () => models.length > 0)
  const [prefix, setPrefix] = useState(account.slug ?? "")
  const [working, setWorking] = useState(false)
  const [open, setOpen] = useState(false)
  useEffect(() => setPrefix(account.slug ?? ""), [account.slug])
  const slug = account.slug ?? ""
  const models = slug ? index.modelsBySlug.get(slug) ?? [] : []
  const labels = slug ? index.labelsBySlug.get(slug) ?? EMPTY_LABELS : EMPTY_LABELS
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
  // 折叠摘要：前缀与模型数，开关关着时说明它不在池里
  const summary = !account.serve_enabled ? "未加入模型池" : models.length ? `${slug || "—"} · ${models.length} 个模型` : "已加入 · 还没有模型清单"
  return (
    <section className="flex flex-col gap-3 rounded-xl border border-jolu-border p-3.5" aria-label="模型路由">
      <div className="flex items-center gap-2.5">
        <Route aria-hidden className="size-4 shrink-0 text-jolu-muted" />
        <button type="button" onClick={() => setOpen(!open)} aria-expanded={open}
          className="flex min-w-0 flex-1 items-center gap-1.5 rounded text-left outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent">
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="text-[13px] font-semibold text-jolu-text">模型路由</span>
            <span className="truncate text-[12px] text-jolu-muted">{open ? "加入模型池后，接入的 Agent 可以用这个账号的模型" : summary}</span>
          </span>
          <ChevronDown aria-hidden className={cn("size-3.5 shrink-0 text-jolu-muted transition-transform", open && "rotate-180")} />
        </button>
        <RouteSwitch on={account.serve_enabled} busy={busy} label={account.serve_enabled ? "移出模型池" : "加入模型池"} onClick={() => void toggle()} />
      </div>
      {account.serve_enabled && open && (
        <>
          <Field label="菜单前缀">
            <form className="flex items-center gap-2" onSubmit={(event) => { event.preventDefault(); const next = prefix.trim(); if (next && next !== slug) void act(() => gatewayApi.setSlug(account.id, next), "已更新菜单前缀") }}>
              <input value={prefix} onChange={(event) => setPrefix(event.target.value)} aria-label="菜单前缀" spellCheck={false}
                className="h-9 min-w-0 flex-1 rounded-lg border border-jolu-border bg-jolu-bg px-2.5 font-mono text-[12px] text-jolu-text outline-none focus:border-jolu-accent" />
              <JoluButton size="sm" variant="neutral" type="submit" disabled={busy || !prefix.trim() || prefix.trim() === slug} className="h-9 shrink-0">保存</JoluButton>
            </form>
            <span className="text-[11px] text-jolu-muted">Agent 里显示为 <code className="font-mono">{prefix.trim() || slug}/{firstModel}</code>；改前缀后旧名称继续可用</span>
          </Field>
          <Field label="提供的模型">
            {/* 单列清单框：模型名动辄 30 字符，散落的 chip 换行后间隙不齐、看着像一堆文字；
                行行对齐才扫得动。常驻滚动（max-h 封死、内部 overflow），滚轮上来就能滚，
                不需要先点展开——「还有 N 个」那层交互就此拿掉，也少占一行。
                按名字排序，同一模型的 -thinking 变体紧跟主模型。
                Qoder 这类来源的 key 是内部 id（gmodel…），官方 display_name 在主位、
                key 右侧小字保留——Agent 里调用的仍是 `前缀/key`。 */}
            <div className="max-h-[88px] overflow-y-auto rounded-lg border border-jolu-border bg-jolu-bg py-1" role="list" aria-label={`提供的模型，共 ${models.length} 个`}>
              {models.length === 0 && <span className="block px-2.5 py-1 text-[12px] text-jolu-muted">还没有拉到模型清单</span>}
              {[...models].sort((a, b) => (labels.get(a) ?? a).localeCompare(labels.get(b) ?? b)).map((model) => {
                const label = labels.get(model)
                return (
                  <div key={model} role="listitem" title={label ? `调用 id：${slug}/${model}` : model}
                    className="flex min-w-0 items-center gap-2 px-2.5 py-[2px] text-[11px] leading-4 odd:bg-jolu-side/40">
                    <span className="min-w-0 flex-1 truncate text-jolu-text">{label ?? model}</span>
                    {label && <code className="shrink-0 font-mono text-[10px] text-jolu-muted">{model}</code>}
                  </div>
                )
              })}
            </div>
            <div className="flex items-center gap-2">
              {models.length > 0 && <span className="text-[11px] tabular-nums text-jolu-muted">共 {models.length} 个</span>}
              <button type="button" disabled={busy} onClick={() => void act(() => gatewayApi.refreshModels(account.id), "模型清单已更新")}
                className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] text-jolu-muted outline-none hover:bg-jolu-side hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-jolu-accent disabled:opacity-40">
                <RefreshCw aria-hidden className="size-3" />按账号权限自动获取
              </button>
            </div>
          </Field>
          {used.length > 0 && (
            <Field label="用在哪里">
              <div className="flex flex-wrap items-center gap-2">{used.map((item) => (
                <span key={item.slug} className="flex items-center gap-1 text-[11px] text-jolu-muted">
                  <code className="rounded-md bg-jolu-side px-1.5 py-0.5 font-mono text-jolu-text">group/{item.slug}</code>第 {item.position} 位
                </span>
              ))}</div>
            </Field>
          )}
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

/** 连接 slug → (key → 官方名) 的空表：避免每个组件各建一份 */
const EMPTY_LABELS = new Map<string, string>()

/** 分区内的一个字段：label 在上、内容在下，与弹窗上半部分的表单同一套骨架 */
function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-[12px] font-medium text-jolu-muted">{label}</span>
      {children}
    </label>
  )
}
