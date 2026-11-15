import { useEffect, useState } from "react"
import { Copy, GitFork, Pencil, Plus } from "lucide-react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { gatewayApi, type CatalogModelDto } from "@/lib/gatewayApi"
import { tierEfforts } from "@/lib/tierModels"
import { useToast } from "@/components/ui/Toast"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { Switch } from "@/components/agents/AgentList"
import {
  PAUSE_CHOICES, ROUTING_STRATEGIES, hasRoutingMember, isMemberPaused, offCatalogMembers, pauseMember, pausedText,
  routingMemberId, routingSlugError, selectableCatalog, strategyOrderNote, strategyText, toggleRoutingMember,
  MEMBER_EFFORTS, memberParts, setMemberEffort, setMemberWeight,
  INTENT_TIMEOUT_RANGE, intentConfig, intentError, isLightMember, setMemberTier,
  type RoutingGroupDto, type RoutingMember,
} from "./routingGroups"

const SELECT = "h-10 min-w-0 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-meta text-jolu-text outline-none focus:border-jolu-accent disabled:opacity-50"
const SMALL_SELECT = "h-8 min-w-0 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-note text-jolu-text outline-none focus:border-jolu-accent disabled:opacity-50"
const INPUT = "h-9 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-meta text-jolu-text outline-none focus:border-jolu-accent"

const MEMBER_COLUMNS = "grid grid-cols-[56px_minmax(0,1fr)_120px_80px_120px_120px] gap-3"

/**
 * 模型路由 · 路由分组（Jolu.pen「统一网关 · R2 重设计」V6 画板 fgm9F）：
 * 分组下拉 + `group/<slug>` 复制、策略区、成员模型表格（顺序 / 来源·模型 / 强度 / 权重 /
 * 状态 / 操作）与「保存更改」。新建分组与成员添加的展开表单画板未给出，沿用旧交互，标「待确认」。
 */
export function GroupsPanel({ catalog, busy, run, onChanged }: {
  catalog: CatalogModelDto[]
  busy: boolean
  run: (work: () => Promise<unknown>, ok?: string, throwOnFailure?: boolean) => Promise<boolean>
  onChanged?: () => void
}) {
  const [groups, setGroups] = useState<RoutingGroupDto[] | null>(null)
  const [loadFailed, setLoadFailed] = useState(false)
  const [slug, setSlug] = useState<string | null>(null)
  const [strategy, setStrategy] = useState("order")
  const [members, setMembers] = useState<RoutingMember[]>([])
  const [classifier, setClassifier] = useState("")
  const [timeout, setTimeoutMs] = useState("")
  const [editingMember, setEditingMember] = useState<string | null>(null)
  const [adding, setAdding] = useState(false)
  const [creating, setCreating] = useState(false)
  const [confirmDelete, setConfirmDelete] = useState<RoutingGroupDto | null>(null)
  const [menu, setMenu] = useState(false)

  useEffect(() => {
    if (!isTauri) return
    let alive = true
    gatewayApi.groupList()
      .then((rows) => { if (alive) { setGroups(rows); setLoadFailed(false) } })
      .catch(() => { if (alive) setLoadFailed(true) })
    return () => { alive = false }
  }, [])

  /** 载入后选中第一个（或聚焦新建时的空表单） */
  useEffect(() => {
    if (!groups || slug !== null) return
    if (groups.length > 0) select(groups[0])
  }, [groups, slug])

  const select = (group: RoutingGroupDto) => {
    setSlug(group.slug)
    setStrategy(group.strategy)
    setMembers(group.members)
    setClassifier(group.intent?.classifier ?? "")
    setTimeoutMs(group.intent?.timeout_ms ? String(group.intent.timeout_ms) : "")
    setEditingMember(null)
    setAdding(false)
    setMenu(false)
  }

  const current = groups?.find((group) => group.slug === slug) ?? null
  const dirty = !!current && (current.strategy !== strategy || JSON.stringify(current.members) !== JSON.stringify(members))
  const intentErr = strategy === "intent" ? intentError(members, classifier, timeout) : null
  const strategyOptions = current && !ROUTING_STRATEGIES.some((item) => item.value === strategy)
    ? [...ROUTING_STRATEGIES, { value: strategy, text: strategyText(strategy) }]
    : ROUTING_STRATEGIES

  const save = async () => {
    if (!current) return
    const patch = strategy === "intent"
      ? { strategy, members, intent: intentConfig(classifier, timeout) }
      : { strategy, members }
    const ok = await run(() => gatewayApi.groupUpdate(current.slug, patch), "已保存分组")
    if (ok) onChanged?.()
  }

  const move = (member: RoutingMember, delta: -1 | 1) => {
    const id = routingMemberId(member)
    const at = members.findIndex((item) => routingMemberId(item) === id)
    const next = [...members]
    const [row] = next.splice(at, 1)
    next.splice(Math.max(0, Math.min(members.length - 1, at + delta)), 0, row)
    setMembers(next)
  }

  const reload = () => {
    onChanged?.()
    if (!isTauri) return
    gatewayApi.groupList().then((rows) => setGroups(rows)).catch(() => setLoadFailed(true))
  }

  const memberRows: { key: string; member: RoutingMember; off: boolean }[] = current
    ? members.map((member) => ({ key: routingMemberId(member), member, off: offCatalogMembers(members, catalog).some((item) => routingMemberId(item) === routingMemberId(member)) }))
    : []

  return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 px-8 pb-8 pt-2">
      <div className="flex items-start justify-between gap-4">
        <div className="flex flex-col gap-1.5">
          <h2 className="text-[20px] font-semibold text-jolu-text">把模型组成可用的一组</h2>
          <p className="text-meta text-jolu-muted">工具只选一个分组名，Jolu 按策略为每次请求选择成员。</p>
        </div>
        <button type="button" disabled={busy} onClick={() => { setCreating(true); setSlug(null) }}
          className="flex h-10 shrink-0 items-center gap-1.5 rounded-lg bg-jolu-text px-4 text-meta font-semibold text-jolu-bg outline-none hover:opacity-90 focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
          <Plus aria-hidden className="size-4" />新建分组
        </button>
      </div>

      {!isTauri ? (
        <p className="rounded-xl border border-jolu-border px-5 py-6 text-center text-meta text-jolu-muted">桌面端可用。</p>
      ) : groups === null ? (
        <p className="rounded-xl border border-jolu-border px-5 py-6 text-center text-meta text-jolu-muted">{loadFailed ? "分组读取失败，请刷新重试。" : "读取分组…"}</p>
      ) : creating ? (
        <CreateForm catalog={catalog} busy={busy} groups={groups} onCancel={() => setCreating(false)} onCreated={() => { setCreating(false); reload() }} run={run} />
      ) : groups.length === 0 ? (
        <div className="flex flex-col items-center gap-3 rounded-xl border border-jolu-border px-5 py-10 text-center">
          <span className="text-meta text-jolu-text">还没有路由分组</span>
          <span className="max-w-[480px] text-note text-jolu-muted">把多个「来源/模型」组成一个可调用的模型 id <code className="font-mono">group/分组</code>，工具选它就能按策略分摊与失败转移。</span>
          <button type="button" disabled={busy} onClick={() => setCreating(true)} className="text-meta font-medium text-jolu-on outline-none hover:underline">＋ 新建第一个分组</button>
        </div>
      ) : current ? (
        <div className="flex flex-col gap-4">
          <section className="flex flex-col gap-4 rounded-xl border border-jolu-border px-5 py-4">
            <div className="flex items-center gap-3">
              <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-jolu-selected"><GitFork aria-hidden className="size-4 text-jolu-on" /></span>
              <select aria-label="选择分组" value={current.slug} disabled={busy} onChange={(event) => select(groups!.find((group) => group.slug === event.target.value)!)} className={cn(SELECT, "w-[240px]")}>
                {groups!.map((group) => <option key={group.slug} value={group.slug}>{group.name}</option>)}
              </select>
              <code className="shrink-0 font-mono text-meta font-medium text-jolu-on">{`group/${current.slug}`}</code>
              <CopyButton text={`group/${current.slug}`} label="复制分组 id" />
              <span className="flex-1" />
              <div className="relative">
                <button type="button" aria-label="更多操作" aria-expanded={menu} disabled={busy} onClick={() => setMenu((value) => !value)}
                  className="flex h-9 w-10 items-center justify-center rounded-lg border border-jolu-border outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
                  <EllipsisDot />
                </button>
                {menu && (
                  <div role="menu" className="absolute right-0 top-10 z-10 w-40 rounded-lg border border-jolu-border bg-jolu-bg p-1 shadow-lg">
                    <button type="button" role="menuitem" disabled={busy} onClick={() => { setMenu(false); setConfirmDelete(current) }}
                      className="w-full rounded-md px-2.5 py-1.5 text-left text-meta text-jolu-danger hover:bg-jolu-side disabled:opacity-45">
                      删除分组
                    </button>
                  </div>
                )}
              </div>
            </div>

            <div className="flex items-start justify-between gap-6 border-t border-jolu-table-divider pt-4">
              <div className="flex flex-col gap-1.5 pt-0.5">
                <h3 className="text-[15px] font-semibold text-jolu-text">选择策略</h3>
                <p className="text-meta text-jolu-muted">{strategyOrderNote(strategy) ?? "按策略决定这次请求用哪个成员。"}</p>
              </div>
              <select aria-label="选择策略" value={strategy} disabled={busy} onChange={(event) => setStrategy(event.target.value)} className={cn(SELECT, "w-[280px] shrink-0")}>
                {strategyOptions.map((item) => <option key={item.value} value={item.value}>{item.text}</option>)}
              </select>
            </div>
            {strategy === "intent" && <IntentFields catalog={catalog} classifier={classifier} timeout={timeout} disabled={busy} onClassifier={setClassifier} onTimeout={setTimeoutMs} label={current.name} />}

            <div className="flex items-center justify-between border-t border-jolu-table-divider pt-4">
              <h3 className="text-[15px] font-semibold text-jolu-text">成员模型</h3>
              <button type="button" disabled={busy} onClick={() => setAdding((value) => !value)}
                className="flex h-9 shrink-0 items-center gap-1.5 rounded-lg border border-jolu-border px-3.5 text-meta font-medium text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
                <Plus aria-hidden className="size-3.5" />添加成员
              </button>
            </div>
            {adding && <MemberPicker catalog={catalog} members={members} disabled={busy} onChange={setMembers} showTier={strategy === "intent"} />}

            <div className={cn("flex flex-col overflow-hidden rounded-xl border border-jolu-border", adding && "opacity-90")}>
              <div className={cn(MEMBER_COLUMNS, "items-center bg-jolu-side px-4 py-2 text-note text-jolu-muted")}>
                <span>顺序</span><span>来源 / 模型</span><span>强度</span><span>权重</span><span>状态</span><span className="justify-self-end">操作</span>
              </div>
              {memberRows.length === 0 && (
                <p className="border-t border-jolu-table-divider px-4 py-6 text-center text-meta text-jolu-muted">还没有成员；点「添加成员」从模型目录勾选。</p>
              )}
              {memberRows.map((row, index) => {
                const id = routingMemberId(row.member)
                const editing = editingMember === id
                const paused = isMemberPaused(row.member, Date.now())
                const efforts = tierEffortsOf(row.member.model)
                const weight = row.member.weight ?? 1
                return (
                  <div key={`${id}-${index}`} className={cn(MEMBER_COLUMNS, "items-center border-t border-jolu-table-divider px-4 py-2.5", paused && "opacity-75")}>
                    <span className="text-note tabular-nums text-jolu-muted">{String(index + 1).padStart(2, "0")}</span>
                    <code className={cn("min-w-0 truncate font-mono text-meta", row.off ? "text-jolu-warn" : "text-jolu-text")} title={row.off ? "不在模型目录（来源下架或清单已刷新）" : id}>{id}</code>
                    {editing ? (
                      <select aria-label={`${id} 的固定强度`} value={memberParts(row.member.model).effort ?? ""} disabled={busy}
                        onClick={(event) => event.stopPropagation()}
                        onChange={(event) => setMembers(setMemberEffort(members, row.member, event.target.value || null))}
                        className={cn(SMALL_SELECT, "justify-self-start")}>
                        <option value="">跟随请求</option>
                        {MEMBER_EFFORTS.map((effort) => <option key={effort} value={effort}>{effort}</option>)}
                      </select>
                    ) : (
                      <span className="text-note text-jolu-muted" title="转发给这个成员时固定请求的思考强度">{efforts.length === 0 ? "不支持" : memberParts(row.member.model).effort ?? "跟随请求"}</span>
                    )}
                    {editing ? (
                      <input type="number" min={1} max={100} value={weight} disabled={busy} aria-label={`${id} 的轮转权重`}
                        onClick={(event) => event.stopPropagation()}
                        onChange={(event) => setMembers(setMemberWeight(members, row.member, Number(event.target.value) || null))}
                        className={cn(SMALL_SELECT, "w-16")} />
                    ) : (
                      <span className="text-note tabular-nums text-jolu-muted" title="「轮转」策略下每轮出现几次">{weight}</span>
                    )}
                    <span className="flex items-center gap-2">
                      <Switch on={!paused} small disabled={busy} label={`${id} ${paused ? "恢复" : "暂停"}`} onClick={() => setMembers(pauseMember(members, row.member, paused ? null : Date.now() + 30 * 60_000))} />
                      <span className="text-note text-jolu-muted">{paused ? pausedText(row.member, Date.now()) : "启用"}</span>
                    </span>
                    <span className="flex items-center gap-1 justify-self-end">
                      <button type="button" aria-label={`编辑 ${id}`} aria-pressed={editing} disabled={busy} onClick={() => setEditingMember(editing ? null : id)}
                        className="grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40">
                        <Pencil aria-hidden className="size-3.5" />
                      </button>
                      <MemberMenu member={row.member} busy={busy} index={index} total={memberRows.length}
                        onMove={move} onRemove={() => setMembers(members.filter((item) => routingMemberId(item) !== id))}
                        onPause={(until) => setMembers(pauseMember(members, row.member, until))} paused={paused} />
                    </span>
                  </div>
                )
              })}
            </div>
            {members.length === 0 && <p className="text-note text-jolu-warn">至少保留一个成员。</p>}
            {intentErr && <p className="text-note text-jolu-warn">{intentErr}</p>}

            <div className="flex items-center gap-3">
              <span className="text-note text-jolu-muted">额度用满的成员会降低优先级，不会从分组移除。</span>
              <span className="flex-1" />
              <button type="button" disabled={busy || !dirty || members.length === 0 || !!intentErr} onClick={() => void save()}
                className="flex h-10 items-center gap-1.5 rounded-lg bg-jolu-text px-4 text-meta font-semibold text-jolu-bg outline-none hover:opacity-90 focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
                <CheckIcon />保存更改
              </button>
            </div>
          </section>
        </div>
      ) : (
        <p className="rounded-xl border border-jolu-border px-5 py-6 text-center text-meta text-jolu-muted">读取分组…</p>
      )}

      {confirmDelete && (
        <ConfirmDialog title={`删除路由分组「${confirmDelete.name}」？`}
          description={`删除后模型 id group/${confirmDelete.slug} 不再可用；成员本身（来源与模型）和历史记录不受影响。`}
          confirmLabel="删除分组" danger onClose={() => setConfirmDelete(null)}
          onConfirm={async () => {
            const ok = await run(() => gatewayApi.groupDelete(confirmDelete.slug), "已删除路由分组", true)
            if (ok) {
              setSlug(null)
              setConfirmDelete(null)
              reload()
            }
          }} />
      )}
    </div>
  )
}

function tierEffortsOf(model: string): readonly string[] {
  // 与 Agent 档位同一份 effort 表（tierModels.tierEfforts）
  return tierEfforts(model)
}

function CopyButton({ text, label }: { text: string; label: string }) {
  const toast = useToast()
  return (
    <button type="button" aria-label={label} title={label} onClick={() => {
      void navigator.clipboard.writeText(text).then(() => toast.success("已复制")).catch(() => toast.danger("复制失败", "请手动选中复制"))
    }} className="grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent">
      <Copy aria-hidden className="size-3.5" />
    </button>
  )
}

function EllipsisDot() {
  return (
    <svg aria-hidden viewBox="0 0 16 16" className="size-4 text-jolu-muted" fill="currentColor">
      <circle cx="3.2" cy="8" r="1.4" /><circle cx="8" cy="8" r="1.4" /><circle cx="12.8" cy="8" r="1.4" />
    </svg>
  )
}

function CheckIcon() {
  return (
    <svg aria-hidden viewBox="0 0 14 14" className="size-3.5" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M2.5 7.5L5.5 10.5L11.5 3.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}

function MemberMenu({ member, busy, index, total, paused, onMove, onRemove, onPause }: {
  member: RoutingMember
  busy: boolean
  index: number
  total: number
  paused: boolean
  onMove: (member: RoutingMember, delta: -1 | 1) => void
  onRemove: () => void
  onPause: (until: number | null) => void
}) {
  const [open, setOpen] = useState(false)
  return (
    <span className="relative">
      <button type="button" aria-label={`更多操作 ${routingMemberId(member)}`} aria-expanded={open} disabled={busy} onClick={() => setOpen((value) => !value)}
        className="grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40">
        <EllipsisDot />
      </button>
      {open && (
        <div role="menu" className="absolute right-0 top-8 z-10 w-44 rounded-lg border border-jolu-border bg-jolu-bg p-1 shadow-lg" onMouseLeave={() => setOpen(false)}>
          <MenuButton label="上移一位" disabled={busy || index === 0} onClick={() => { setOpen(false); onMove(member, -1) }} />
          <MenuButton label="下移一位" disabled={busy || index === total - 1} onClick={() => { setOpen(false); onMove(member, 1) }} />
          {PAUSE_CHOICES.map((choice) => (
            <MenuButton key={choice.minutes} label={`暂停 ${choice.text}`} disabled={busy} onClick={() => { setOpen(false); onPause(Date.now() + choice.minutes * 60_000) }} />
          ))}
          {paused && <MenuButton label="立即恢复" disabled={busy} onClick={() => { setOpen(false); onPause(null) }} />}
          <MenuButton label="移出分组" danger disabled={busy} onClick={() => { setOpen(false); onRemove() }} />
        </div>
      )}
    </span>
  )
}

function MenuButton({ label, disabled, danger, onClick }: { label: string; disabled?: boolean; danger?: boolean; onClick: () => void }) {
  return (
    <button type="button" role="menuitem" disabled={disabled} onClick={onClick}
      className={cn("w-full rounded-md px-2.5 py-1.5 text-left text-meta hover:bg-jolu-side disabled:opacity-45", danger ? "text-jolu-danger" : "text-jolu-text")}>
      {label}
    </button>
  )
}

/** 「按意图」的判定设置（待确认：V6 无画板）：判定模型从目录里选（不含分组），超时可空（默认 4000 毫秒）。 */
function IntentFields({ catalog, classifier, timeout, disabled, onClassifier, onTimeout, label }: {
  catalog: CatalogModelDto[]
  classifier: string
  timeout: string
  disabled?: boolean
  onClassifier: (value: string) => void
  onTimeout: (value: string) => void
  label: string
}) {
  const ids = selectableCatalog(catalog).map((item) => routingMemberId({ slug: item.slug, model: item.model }))
  const options = classifier && !ids.includes(classifier) ? [classifier, ...ids] : ids
  return (
    <div className="flex flex-wrap items-end gap-3 border-t border-jolu-table-divider pt-4">
      <label className="flex min-w-[200px] flex-1 flex-col gap-1">
        <span className="text-note text-jolu-muted">判意图用的模型（选便宜、快的）</span>
        <select value={classifier} disabled={disabled} aria-label={`「${label}」判意图用的模型`} className={SELECT} onChange={(event) => onClassifier(event.target.value)}>
          <option value="">选择模型…</option>
          {options.map((id) => <option key={id} value={id}>{id}{ids.includes(id) ? "" : "（不在目录）"}</option>)}
        </select>
      </label>
      <label className="flex flex-col gap-1">
        <span className="text-note text-jolu-muted">判定超时（毫秒，可空）</span>
        <input value={timeout} disabled={disabled} inputMode="numeric" aria-label={`「${label}」的判定超时`}
          placeholder={`默认 ${INTENT_TIMEOUT_RANGE.fallback}`} onChange={(event) => onTimeout(event.target.value)} className={cn(INPUT, "w-[140px]")} />
      </label>
    </div>
  )
}

/** 成员多选（待确认：V6 只画了「添加成员」按钮，勾选表单沿用旧交互）：目录条目 + 目录外的现存成员。 */
function MemberPicker({ catalog, members, disabled, onChange, showTier }: {
  catalog: CatalogModelDto[]
  members: RoutingMember[]
  disabled?: boolean
  onChange: (next: RoutingMember[]) => void
  showTier?: boolean
}) {
  const rows: { key: string; member: RoutingMember; off: boolean }[] = [
    ...selectableCatalog(catalog).map((item) => ({ key: routingMemberId({ slug: item.slug, model: item.model }), member: { slug: item.slug, model: item.model }, off: false })),
    ...offCatalogMembers(members, catalog).map((member) => ({ key: routingMemberId(member), member, off: true })),
  ]
  return (
    <div className="max-h-[220px] overflow-y-auto rounded-xl border border-jolu-border">
      {rows.map((row) => (
        <label key={row.key} className="flex h-9 cursor-pointer items-center gap-2.5 border-b border-jolu-table-divider px-4 text-meta last:border-b-0 hover:bg-jolu-side">
          <input type="checkbox" checked={hasRoutingMember(members, row.member)} disabled={disabled}
            onChange={() => onChange(toggleRoutingMember(members, row.member))} className="size-3.5 accent-[var(--jolu-accent)]" />
          <code className="min-w-0 flex-1 truncate font-mono text-jolu-text">{routingMemberId(row.member)}</code>
          {row.off && <span className="shrink-0 text-note text-jolu-warn">不在目录</span>}
          {hasRoutingMember(members, row.member) && (() => {
            const current = members.find((item) => item.slug === row.member.slug && memberParts(item.model).base === memberParts(row.member.model).base) ?? row.member
            return (
              <>
                {showTier && (
                  <select value={isLightMember(current) ? "light" : "strong"} disabled={disabled} aria-label={`${routingMemberId(row.member)} 的意图档位`}
                    title="强档答需要动脑的提问，轻档答快问题"
                    onClick={(event) => event.stopPropagation()}
                    onChange={(event) => onChange(setMemberTier(members, row.member, event.target.value === "light"))}
                    className="h-7 rounded-md border border-jolu-border bg-jolu-bg px-1.5 text-note text-jolu-muted outline-none focus:border-jolu-accent">
                    <option value="strong">强档</option>
                    <option value="light">轻档</option>
                  </select>
                )}
                <select value={memberParts(current.model).effort ?? ""} disabled={disabled} aria-label={`${routingMemberId(row.member)} 的固定档位`}
                  title="固定推理档位：转发给这个成员时按此档位问上游（请求本来没要推理时不加）"
                  onClick={(event) => event.stopPropagation()}
                  onChange={(event) => onChange(setMemberEffort(members, row.member, event.target.value || null))}
                  className="h-7 rounded-md border border-jolu-border bg-jolu-bg px-1.5 text-note text-jolu-muted outline-none focus:border-jolu-accent">
                  <option value="">跟随请求</option>
                  {MEMBER_EFFORTS.map((effort) => <option key={effort} value={effort}>{effort}</option>)}
                </select>
                <input type="number" min={1} max={100} value={current.weight ?? 1} disabled={disabled} aria-label={`${routingMemberId(row.member)} 的轮转权重`}
                  title="「轮转」策略下每轮出现几次"
                  onClick={(event) => event.stopPropagation()}
                  onChange={(event) => onChange(setMemberWeight(members, row.member, Number(event.target.value) || null))}
                  className="h-7 w-14 rounded-md border border-jolu-border bg-jolu-bg px-1.5 text-note tabular-nums text-jolu-muted outline-none focus:border-jolu-accent" />
              </>
            )
          })()}
        </label>
      ))}
      {rows.length === 0 && <p className="px-4 py-4 text-meta text-jolu-muted">模型目录为空：先在来源行「拉取」模型清单，再来勾选成员。</p>}
    </div>
  )
}

/** 新建分组（待确认：V6 无画板，沿用旧表单）。 */
function CreateForm({ catalog, busy, groups, run, onCancel, onCreated }: {
  catalog: CatalogModelDto[]
  busy: boolean
  groups: RoutingGroupDto[]
  run: (work: () => Promise<unknown>, ok?: string, throwOnFailure?: boolean) => Promise<boolean>
  onCancel: () => void
  onCreated: () => void
}) {
  const [name, setName] = useState("")
  const [slug, setSlug] = useState("")
  const [strategy, setStrategy] = useState("order")
  const [members, setMembers] = useState<RoutingMember[]>([])
  const [classifier, setClassifier] = useState("")
  const [timeout, setTimeoutMs] = useState("")
  const slugTrimmed = slug.trim()
  const slugError = routingSlugError(slugTrimmed)
  const slugTaken = !!groups.some((item) => item.slug === slugTrimmed)
  const intentErr = strategy === "intent" ? intentError(members, classifier, timeout) : null
  const canCreate = !!name.trim() && !!slugTrimmed && !slugError && !slugTaken && members.length > 0 && !intentErr
  return (
    <section className="flex flex-col gap-3.5 rounded-xl border border-jolu-border px-5 py-4" data-create-form>
      <div className="flex items-center justify-between">
        <h3 className="text-[15px] font-semibold text-jolu-text">新建分组（待确认：V6 无画板，沿用旧表单）</h3>
      </div>
      <div className="flex flex-wrap items-end gap-3">
        <label className="flex min-w-[160px] flex-1 flex-col gap-1">
          <span className="text-note text-jolu-muted">名称</span>
          <input autoFocus value={name} onChange={(event) => setName(event.target.value)} placeholder="如 编码主力" aria-label="分组名称" className={INPUT} />
        </label>
        <label className="flex min-w-[160px] flex-1 flex-col gap-1">
          <span className="text-note text-jolu-muted">id</span>
          <input value={slug} onChange={(event) => setSlug(event.target.value)} placeholder="如 coding" aria-label="分组 id" className={cn(INPUT, (slugError || slugTaken) && "border-jolu-danger")} />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-note text-jolu-muted">策略</span>
          <select value={strategy} disabled={busy} aria-label="新建分组的策略" className={SELECT} onChange={(event) => setStrategy(event.target.value)}>
            {ROUTING_STRATEGIES.map((item) => <option key={item.value} value={item.value}>{item.text}</option>)}
          </select>
        </label>
      </div>
      {slugError && <span className="text-note text-jolu-danger">{slugError}</span>}
      {slugTaken && <span className="text-note text-jolu-danger">「{slugTrimmed}」已经有同 id 的分组。</span>}
      {strategy === "intent" && <IntentFields catalog={catalog} classifier={classifier} timeout={timeout} disabled={busy} onClassifier={setClassifier} onTimeout={setTimeoutMs} label="新建分组" />}
      <span className="text-note text-jolu-muted">成员 · 勾选即加入，模型 id = 来源/模型</span>
      <MemberPicker catalog={catalog} members={members} disabled={busy} onChange={setMembers} showTier={strategy === "intent"} />
      <div className="flex items-center gap-2">
        <button type="button" disabled={busy || !canCreate} onClick={async () => {
          const intent = strategy === "intent" ? intentConfig(classifier, timeout) : null
          const ok = await run(() => gatewayApi.groupCreate(slugTrimmed, name.trim(), strategy, members, intent), `已新建分组「${name.trim()}」`)
          if (ok) onCreated()
        }}
          className="flex h-9 items-center rounded-lg bg-jolu-accent px-4 text-meta font-semibold text-[#071B10] outline-none disabled:opacity-45">创建</button>
        <button type="button" disabled={busy} onClick={onCancel} className="text-meta text-jolu-muted outline-none hover:text-jolu-text">取消</button>
        {members.length === 0 && <span className="text-note text-jolu-muted">至少勾选一个成员。</span>}
      </div>
    </section>
  )
}
