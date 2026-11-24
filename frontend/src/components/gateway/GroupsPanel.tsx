import { useEffect, useState } from "react"
import { isTauri } from "@/lib/api"
import { gatewayApi, type CatalogModelDto } from "@/lib/gatewayApi"
import { accountQueries, formatMoney } from "@/lib/accountDisplay"
import { remainingPercent } from "@/lib/quota"
import { useBalance, useQuota } from "@/hooks/useQuota"
import { tierEfforts } from "@/lib/tierModels"
import { useToast } from "@/components/ui/Toast"
import { Icon } from "@/icons"
import { cn } from "@/lib/utils"
import { Button, Checkbox, ConfirmDialog, Dialog, IconButton, Input, OptionTile, Select, Switch, Tag } from "@/ui"
import type { Connection } from "@/types"
import {
  PAUSE_CHOICES, ROUTING_STRATEGIES, hasRoutingMember, isMemberPaused, offCatalogMembers, pauseMember, pausedText,
  routingMemberId, routingSlugError, selectableCatalog, strategyOrderNote, strategyText, toggleRoutingMember,
  MEMBER_EFFORTS, memberParts, setMemberEffort, setMemberWeight,
  INTENT_TIMEOUT_RANGE, intentConfig, intentError, isLightMember, setMemberTier,
  type RoutingGroupDto, type RoutingMember,
} from "./routingGroups"

const MEMBER_COLUMNS = "grid grid-cols-[44px_minmax(0,1fr)_88px_98px_62px_92px_68px] gap-2.5"

/**
 * 模型路由 · 路由分组 —— 设计稿 R1-19（浅色）。
 *
 * 主从布局：左栏 260 = 分组列表（名称、策略与成员数），右栏 = 分组详情
 * （名称与 `group/<slug>`、四档策略、成员表含剩余额度、保存）。新建分组没有 R1-19 详情画板，
 * 沿用旧表单并保留 `data-create-form` 钩子（标「待确认」）。
 */
export function GroupsPanel({ catalog, connections = [], busy, run, onChanged }: {
  catalog: CatalogModelDto[]
  connections?: Connection[]
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
    setCreating(false)
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
    <div className="flex min-w-0 flex-col gap-3.5">
      {!isTauri ? (
        <p className="rounded-jn-card border border-jn-line bg-jn-surface px-5 py-6 text-center text-[12px] text-jn-text-3 shadow-jn-surface">桌面端可用。</p>
      ) : groups === null ? (
        <p className="rounded-jn-card border border-jn-line bg-jn-surface px-5 py-6 text-center text-[12px] text-jn-text-3 shadow-jn-surface">{loadFailed ? "分组读取失败，请刷新重试。" : "读取分组…"}</p>
      ) : groups.length === 0 && !creating ? (
        <div className="flex flex-col items-center gap-3 rounded-jn-card border border-dashed border-jn-line px-5 py-10 text-center">
          <span className="text-[13px] text-jn-text">还没有路由分组</span>
          <span className="max-w-[480px] text-[12px] text-jn-text-3">把多个「来源/模型」组成 <code className="font-mono">group/分组</code>，工具选它就能按策略分摊与失败转移。</span>
          <Button kind="primary" size="sm" icon="add" disabled={busy} onClick={() => { setCreating(true); setSlug(null) }}>新建第一个分组</Button>
        </div>
      ) : creating ? (
        <CreateForm catalog={catalog} busy={busy} groups={groups} onCancel={() => setCreating(false)} onCreated={() => { setCreating(false); reload() }} run={run} />
      ) : (
        <div className="@container">
        <div className="grid grid-cols-1 gap-3.5 @[820px]:grid-cols-[260px_minmax(0,1fr)]">
          {/* 分组列表（R1-19 左栏） */}
          <aside className="flex min-w-0 flex-col gap-1.5 rounded-jn-card border border-jn-line bg-jn-surface p-3 shadow-jn-surface">
            <div className="flex h-10 items-center gap-2 px-1">
              <Icon name="blocks" size="sm" className="text-jn-text-2" />
              <span className="text-[14px] font-semibold text-jn-text">路由分组</span>
              <span className="min-w-0 flex-1" />
              <IconButton icon="add" label="新建分组" size="sm" disabled={busy} onClick={() => { setCreating(true); setSlug(null) }} />
            </div>
            {groups.map((group) => {
              const active = group.slug === current?.slug
              return (
                <button key={group.slug} type="button" onClick={() => select(group)}
                  className={cn("jn-interactive flex items-center gap-3 rounded-[16px] px-3 py-2.5 text-left", active ? "bg-jn-primary-wash" : "hover:bg-jn-well")}>
                  <span className={cn("flex size-[34px] shrink-0 items-center justify-center rounded-[12px] text-[14px] font-bold", active ? "bg-jn-primary-soft text-jn-primary-ink" : "bg-jn-well text-jn-text-2")}>
                    {(group.name.trim()[0] ?? "?").toUpperCase()}
                  </span>
                  <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className={cn("truncate font-mono text-[13px] font-semibold", active ? "text-jn-primary-ink" : "text-jn-text")}>group/{group.slug}</span>
                    <span className="truncate text-[11px] text-jn-text-3">{strategyText(group.strategy)} · {group.members.length} 个成员</span>
                  </span>
                  <Icon name="next" size="sm" className="shrink-0 text-jn-text-3" />
                </button>
              )
            })}
          </aside>

          {/* 分组详情（R1-19 右栏） */}
          {current && (
            <section className="flex min-w-0 flex-col gap-4 rounded-jn-card border border-jn-line bg-jn-surface px-[22px] pt-5 pb-4 shadow-jn-surface">
              <div className="flex items-center gap-3">
                <span className="flex min-w-0 flex-col gap-[3px]">
                  <span className="truncate text-[18px] font-bold text-jn-text">{current.name}</span>
                  <span className="flex items-center gap-1.5">
                    <code className="font-mono text-[12.5px] font-semibold text-jn-primary-ink">group/{current.slug}</code>
                    <CopyButton text={`group/${current.slug}`} label="复制分组 id" />
                  </span>
                </span>
                <span className="min-w-0 flex-1" />
                <IconButton icon="delete" label={`删除分组 ${current.name}`} size="sm" disabled={busy} onClick={() => setConfirmDelete(current)} />
              </div>

              <div className="flex flex-col gap-2.5">
                <span className="text-[13.5px] font-semibold text-jn-text">选择策略</span>
                <div role="radiogroup" aria-label="选择策略" className="grid grid-cols-2 gap-2 @[700px]:grid-cols-4">
                  {strategyOptions.map((item) => (
                    <OptionTile
                      key={item.value}
                      active={strategy === item.value}
                      disabled={busy}
                      icon={strategy === item.value ? "check" : undefined}
                      label={item.text}
                      onClick={() => setStrategy(item.value)}
                    />
                  ))}
                </div>
                <p className="text-[12px] text-jn-text-3">{strategyOrderNote(strategy) ?? "按策略决定这次请求用哪个成员。"}</p>
              </div>
              {strategy === "intent" && <IntentFields catalog={catalog} classifier={classifier} timeout={timeout} disabled={busy} onClassifier={setClassifier} onTimeout={setTimeoutMs} label={current.name} />}

              <div className="flex items-center gap-2">
                <span className="text-[13.5px] font-semibold text-jn-text">成员模型</span>
                <span className="flex h-5 items-center rounded-full bg-jn-well px-2 text-[11px] font-bold text-jn-text-2">{members.length}</span>
                <span className="min-w-0 flex-1" />
                <Button kind="ghost" size="sm" icon="add" disabled={busy} onClick={() => setAdding(true)}>添加成员</Button>
              </div>
              <Dialog
                open={adding}
                onOpenChange={(open) => { if (!open) setAdding(false) }}
                width="md"
                title={`添加成员 · group/${current.slug}`}
                description="勾选即加入，模型 id = 来源/模型；额度用满的成员会降低优先级，不会从分组移除。"
                footerStart={`当前已选 ${members.length} 个成员`}
                actions={<Button kind="primary" size="md" icon="check" onClick={() => setAdding(false)}>完成</Button>}
              >
                <MemberPicker catalog={catalog} members={members} disabled={busy} onChange={setMembers} showTier={strategy === "intent"} />
              </Dialog>

              <div className="overflow-x-auto">
              <div className="flex min-w-[620px] flex-col gap-0.5">
                <div className={cn(MEMBER_COLUMNS, "px-2.5 pb-1 text-[11.5px] font-medium text-jn-text-3")}>
                  <span>顺序</span><span>来源 / 模型</span><span>剩余额度</span><span>强度</span><span>权重</span><span>启用</span><span className="justify-self-end">操作</span>
                </div>
                {memberRows.length === 0 && (
                  <p className="px-2.5 py-6 text-center text-[12px] text-jn-text-3">还没有成员；点「添加成员」从模型目录勾选。</p>
                )}
                {memberRows.map((row, index) => {
                  const id = routingMemberId(row.member)
                  const editing = editingMember === id
                  const paused = isMemberPaused(row.member, Date.now())
                  const efforts = tierEfforts(row.member.model)
                  const weight = row.member.weight ?? 1
                  const conn = connections.find((c) => c.slug === row.member.slug) ?? null
                  return (
                    <div key={`${id}-${index}`} className={cn(MEMBER_COLUMNS, "h-[52px] items-center rounded-[14px] px-2.5", index % 2 === 0 && "bg-jn-well")}>
                      <span className="text-[12px] font-semibold tabular-nums text-jn-text-3">{String(index + 1).padStart(2, "0")}</span>
                      <code className={cn("min-w-0 truncate font-mono text-[12px]", row.off ? "text-jn-warning" : "text-jn-text")} title={row.off ? "不在模型目录（来源下架或清单已刷新）" : id}>{id}</code>
                      <MemberQuotaCell connection={conn} />
                      {editing ? (
                        <Select width="full" label={`${id} 的固定强度`} value={memberParts(row.member.model).effort ?? ""} disabled={busy}
                          onChange={(value) => setMembers(setMemberEffort(members, row.member, value || null))}
                          options={[{ value: "", label: "跟随请求" }, ...MEMBER_EFFORTS.map((effort) => ({ value: effort, label: effort }))]} />
                      ) : (
                        <span className="truncate text-[12px] text-jn-text-3" title="转发给这个成员时固定请求的思考强度">{efforts.length === 0 ? "不支持" : memberParts(row.member.model).effort ?? "跟随请求"}</span>
                      )}
                      {editing ? (
                        <Input size="md" type="number" min={1} max={100} value={weight} disabled={busy} aria-label={`${id} 的轮转权重`}
                          onChange={(event) => setMembers(setMemberWeight(members, row.member, Number(event.target.value) || null))} className="h-8" />
                      ) : (
                        <span className="text-[12px] tabular-nums text-jn-text-3" title="「轮转」策略下每轮出现几次">{weight}</span>
                      )}
                      <span className="flex items-center gap-1.5">
                        <Switch checked={!paused} disabled={busy} label={`${id} ${paused ? "恢复" : "暂停"}`}
                          onChange={() => setMembers(pauseMember(members, row.member, paused ? null : Date.now() + 30 * 60_000))} />
                        <span className="text-[11px] text-jn-text-3">{paused ? pausedText(row.member, Date.now()) : "启用"}</span>
                      </span>
                      <span className="flex items-center justify-self-end gap-0.5">
                        <IconButton icon="edit" label={`编辑 ${id}`} size="sm" aria-pressed={editing} disabled={busy} onClick={() => setEditingMember(editing ? null : id)} />
                        <MemberMenu member={row.member} busy={busy} index={index} total={memberRows.length}
                          onMove={move} onRemove={() => setMembers(members.filter((item) => routingMemberId(item) !== id))}
                          onPause={(until) => setMembers(pauseMember(members, row.member, until))} paused={paused} />
                      </span>
                    </div>
                  )
                })}
              </div>
              </div>
              {members.length === 0 && <p className="text-[11px] text-jn-warning">至少保留一个成员。</p>}
              {intentErr && <p className="text-[11px] text-jn-warning">{intentErr}</p>}

              <div className="flex items-center gap-3">
                <span className="text-[11px] text-jn-text-3">额度用满的成员会降低优先级，不会从分组移除。</span>
                <span className="min-w-0 flex-1" />
                <Button kind="primary" size="md" icon="success" disabled={busy || !dirty || members.length === 0 || !!intentErr} onClick={() => void save()}>保存更改</Button>
              </div>
            </section>
          )}
        </div>
        </div>
      )}

      {confirmDelete && (
        <ConfirmDialog
          open
          onOpenChange={(open) => { if (!open) setConfirmDelete(null) }}
          title={`删除路由分组「${confirmDelete.name}」？`}
          description={`删除后模型 id group/${confirmDelete.slug} 不再可用；成员本身（来源与模型）和历史记录不受影响。`}
          confirmText="删除分组"
          loading={busy}
          onConfirm={() => {
            const group = confirmDelete
            setConfirmDelete(null)
            void (async () => {
              const ok = await run(() => gatewayApi.groupDelete(group.slug), "已删除路由分组", true)
              if (ok) { setSlug(null); reload() }
            })()
          }}
        />
      )}
    </div>
  )
}

/** R1-19 成员表「剩余额度」单元格：读取缓存的真实额度 / 余额，未知显示「—」。 */
function MemberQuotaCell({ connection }: { connection: Connection | null }) {
  const queries = connection ? accountQueries(connection) : { quota: null, balance: null }
  const quota = useQuota(queries.quota, false, Boolean(isTauri && queries.quota))
  const balance = useBalance(isTauri ? queries.balance : null, Boolean(isTauri && queries.balance))

  if (!connection) return <span className="text-xs text-jn-text-3">—</span>
  if (connection.kind === "api") {
    if (balance.state?.state === "ok" && typeof balance.state.balance === "number") {
      return <span className="tnum truncate font-mono text-xs font-semibold text-jn-text">{formatMoney(balance.state.balance, balance.state.currency)}</span>
    }
    return <span className="truncate text-xs text-jn-text-3">按量</span>
  }
  const first = quota.state?.state === "ok"
    ? quota.state.windows.find((w) => typeof w.used_percent === "number") ?? null
    : null
  const rem = first && typeof first.used_percent === "number" ? remainingPercent(first.used_percent) : null
  if (rem === null) return <span className="text-xs text-jn-text-3">—</span>
  return (
    <span className={cn("tnum font-mono text-xs font-semibold", rem <= 15 ? "text-jn-warning" : "text-jn-primary-ink")}>
      {Math.round(rem)}%
    </span>
  )
}

function CopyButton({ text, label }: { text: string; label: string }) {
  const toast = useToast()
  return (
    <IconButton icon="copy" label={label} size="sm"
      onClick={() => { void navigator.clipboard.writeText(text).then(() => toast.success("已复制")).catch(() => toast.danger("复制失败", "请手动选中复制")) }} />
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
      <IconButton icon="more" label={`更多操作 ${routingMemberId(member)}`} size="sm" disabled={busy} aria-expanded={open} onClick={() => setOpen((value) => !value)} />
      {open && (
        <div role="menu" className="absolute top-8 right-0 z-10 w-44 rounded-[14px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface" onMouseLeave={() => setOpen(false)}>
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
      className={cn("w-full rounded-[10px] px-2.5 py-1.5 text-left text-[12px] hover:bg-jn-well disabled:opacity-45", danger ? "text-jn-danger" : "text-jn-text")}>
      {label}
    </button>
  )
}

/** 「按意图」的判定设置：判定模型从目录里选（不含分组），超时可空（默认 4000 毫秒）。 */
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
    <div className="flex flex-wrap items-end gap-3">
      <label className="flex min-w-[200px] flex-1 flex-col gap-1.5">
        <span className="text-[11px] text-jn-text-3">判意图用的模型（选便宜、快的）</span>
        <Select width="full" label={`「${label}」判意图用的模型`} value={classifier || null} disabled={disabled}
          onChange={onClassifier} placeholder="选择模型…"
          options={options.map((id) => ({ value: id, label: id + (ids.includes(id) ? "" : "（不在目录）") }))} />
      </label>
      <label className="flex flex-col gap-1.5">
        <span className="text-[11px] text-jn-text-3">判定超时（毫秒，可空）</span>
        <Input size="md" value={timeout} disabled={disabled} inputMode="numeric" aria-label={`「${label}」的判定超时`}
          placeholder={`默认 ${INTENT_TIMEOUT_RANGE.fallback}`} onChange={(event) => onTimeout(event.target.value)} className="w-[140px]" />
      </label>
    </div>
  )
}

/** 成员多选（R1-36）：带搜索 + 目录条目 + 目录外的现存成员。 */
function MemberPicker({ catalog, members, disabled, onChange, showTier }: {
  catalog: CatalogModelDto[]
  members: RoutingMember[]
  disabled?: boolean
  onChange: (next: RoutingMember[]) => void
  showTier?: boolean
}) {
  const [query, setQuery] = useState("")
  const q = query.trim().toLowerCase()
  const rows: { key: string; member: RoutingMember; off: boolean }[] = [
    ...selectableCatalog(catalog).map((item) => ({ key: routingMemberId({ slug: item.slug, model: item.model }), member: { slug: item.slug, model: item.model }, off: false })),
    ...offCatalogMembers(members, catalog).map((member) => ({ key: routingMemberId(member), member, off: true })),
  ].filter((row) => !q || row.key.toLowerCase().includes(q))
  return (
    <div className="flex flex-col gap-2.5">
      <Input
        size="md"
        icon="search"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="搜索来源或模型"
        aria-label="搜索来源或模型"
      />
      <div className="max-h-[300px] overflow-y-auto rounded-[16px] border border-jn-line">
        {rows.map((row) => {
          const selected = hasRoutingMember(members, row.member)
          const currentMember = selected
            ? members.find((item) => item.slug === row.member.slug && memberParts(item.model).base === memberParts(row.member.model).base) ?? row.member
            : row.member
          return (
            <div key={row.key} className={cn("flex min-h-10 items-center gap-2.5 border-b border-jn-line px-3.5 py-1.5 last:border-b-0", selected ? "bg-jn-primary-wash/70" : "hover:bg-jn-well")}>
              <Checkbox checked={selected} disabled={disabled} label={routingMemberId(row.member)} onChange={() => onChange(toggleRoutingMember(members, row.member))} />
              <code className="min-w-0 flex-1 truncate font-mono text-[12px] text-jn-text">{routingMemberId(row.member)}</code>
              {row.off && <Tag>不在目录</Tag>}
              {selected && (
                <>
                  {showTier && (
                    <div className="w-[90px]">
                      <Select variant="filter" width="full" label={`${routingMemberId(row.member)} 的意图档位`} value={isLightMember(currentMember) ? "light" : "strong"} disabled={disabled}
                        onChange={(value) => onChange(setMemberTier(members, row.member, value === "light"))}
                        options={[{ value: "strong", label: "强档" }, { value: "light", label: "轻档" }]} />
                    </div>
                  )}
                  <div className="w-[110px]">
                    <Select variant="filter" width="full" label={`${routingMemberId(row.member)} 的固定档位`} value={memberParts(currentMember.model).effort ?? ""} disabled={disabled}
                      onChange={(value) => onChange(setMemberEffort(members, row.member, value || null))}
                      options={[{ value: "", label: "跟随请求" }, ...MEMBER_EFFORTS.map((effort) => ({ value: effort, label: effort }))]} />
                  </div>
                  <Input size="md" type="number" min={1} max={100} value={currentMember.weight ?? 1} disabled={disabled} aria-label={`${routingMemberId(row.member)} 的轮转权重`}
                    onChange={(event) => onChange(setMemberWeight(members, row.member, Number(event.target.value) || null))} className="h-8 w-14" />
                </>
              )}
            </div>
          )
        })}
        {rows.length === 0 && (
          <p className="px-4 py-6 text-center text-[12px] text-jn-text-3">
            {q ? "没有匹配的模型" : "模型目录为空：先在来源行「拉取」模型清单，再来勾选成员。"}
          </p>
        )}
      </div>
    </div>
  )
}

/** 新建分组（R1-19 未画详情，沿用旧表单，标「待确认」）。 */
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
    <section className="flex flex-col gap-3.5 rounded-jn-card border border-jn-line bg-jn-surface p-5 shadow-jn-surface" data-create-form>
      <h3 className="text-[14px] font-bold text-jn-text">新建分组</h3>
      <div className="flex flex-wrap items-end gap-3">
        <label className="flex min-w-[160px] flex-1 flex-col gap-1.5">
          <span className="text-[11px] text-jn-text-3">名称</span>
          <Input autoFocus size="lg" value={name} onChange={(event) => setName(event.target.value)} placeholder="如 编码主力" aria-label="分组名称" />
        </label>
        <label className="flex min-w-[160px] flex-1 flex-col gap-1.5">
          <span className="text-[11px] text-jn-text-3">id</span>
          <Input size="lg" value={slug} onChange={(event) => setSlug(event.target.value)} placeholder="如 coding" aria-label="分组 id" invalid={Boolean(slugError || slugTaken)} />
        </label>
        <div className="flex w-[170px] flex-col gap-1.5">
          <span className="text-[11px] text-jn-text-3">策略</span>
          <Select width="full" label="新建分组的策略" value={strategy} disabled={busy} onChange={setStrategy}
            options={ROUTING_STRATEGIES.map((item) => ({ value: item.value, label: item.text, hint: strategyOrderNote(item.value) || undefined }))} />
        </div>
      </div>
      {slugError && <span className="text-[11px] text-jn-danger">{slugError}</span>}
      {slugTaken && <span className="text-[11px] text-jn-danger">「{slugTrimmed}」已经有同 id 的分组。</span>}
      {strategy === "intent" && <IntentFields catalog={catalog} classifier={classifier} timeout={timeout} disabled={busy} onClassifier={setClassifier} onTimeout={setTimeoutMs} label="新建分组" />}
      <span className="text-[11px] text-jn-text-3">成员 · 勾选即加入，模型 id = 来源/模型</span>
      <MemberPicker catalog={catalog} members={members} disabled={busy} onChange={setMembers} showTier={strategy === "intent"} />
      <div className="flex items-center gap-2">
        <Button kind="primary" size="md" disabled={busy || !canCreate} onClick={async () => {
          const intent = strategy === "intent" ? intentConfig(classifier, timeout) : null
          const ok = await run(() => gatewayApi.groupCreate(slugTrimmed, name.trim(), strategy, members, intent), `已新建分组「${name.trim()}」`)
          if (ok) onCreated()
        }}>创建</Button>
        <Button kind="ghost" size="md" disabled={busy} onClick={onCancel}>取消</Button>
        {members.length === 0 && <span className="text-[11px] text-jn-text-3">至少勾选一个成员。</span>}
      </div>
    </section>
  )
}
