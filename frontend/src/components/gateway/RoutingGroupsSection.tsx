import { useEffect, useState } from "react"
import { ChevronDown, ChevronRight, Plus, Trash2 } from "lucide-react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { gatewayApi, type CatalogModelDto } from "@/lib/gatewayApi"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { Button } from "./GatewayDialogs"
import {
  PAUSE_CHOICES, ROUTING_STRATEGIES, hasRoutingMember, isMemberPaused, offCatalogMembers, pauseMember, pausedText,
  routingMemberId, routingSlugError, selectableCatalog, strategyOrderNote, strategyText, toggleRoutingMember,
  MEMBER_EFFORTS, memberParts, setMemberEffort, setMemberWeight,
  INTENT_TIMEOUT_RANGE, intentConfig, intentError, isLightMember, setMemberTier,
  type RoutingGroupDto, type RoutingMember,
} from "./routingGroups"

/**
 * 路由分组（待确认的最小可用版）：把多个「来源/模型」成员组成一个模型 id `group/<分组id>`，
 * 网关按策略在成员间分摊与失败转移。Jolu.pen 暂无画板，按画板确认后移入主界面；
 * 折叠放在「管理模型来源」弹窗底部。
 *
 * 分组列表每行显示 名称（id）· 策略 · 成员数 与删除按钮；点开一行可增删成员、改策略（groupUpdate）；
 * 新建表单勾选模型目录条目即成员（成员模型 id = `来源/模型`），分组的目录条目（slug === "group"）不嵌套。
 */

const ICON_BUTTON = "grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-35"
const INPUT = "jolu-control h-8 rounded-lg border border-jolu-border bg-jolu-side px-2.5 text-[12px] text-jolu-text outline-none focus:border-jolu-accent"
const SELECT = "h-8 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[11.5px] text-jolu-text outline-none focus:border-jolu-accent"

export function RoutingGroupsSection({ catalog, busy, run, embedded = false, focus, createNonce, onChanged }: {
  catalog: CatalogModelDto[]
  /** 嵌进模型路由页的检查器（R2 / V5）：默认展开、不显示折叠头 */
  embedded?: boolean
  /** 左栏点了某个分组：展开它的编辑 */
  focus?: string
  /** 左栏点了「＋」：打开新建表单（每次点击换一个值） */
  createNonce?: number
  /** 分组增删改后通知页面刷新左栏 */
  onChanged?: () => void
  busy: boolean
  /** 弹窗的统一操作入口：加锁、成功提示、刷新页面数据；失败时提示并返回 false */
  run: (work: () => Promise<unknown>, ok?: string, throwOnFailure?: boolean) => Promise<boolean>
}) {
  const [open, setOpen] = useState(embedded)
  const [groups, setGroups] = useState<RoutingGroupDto[] | null>(null)
  const [loadFailed, setLoadFailed] = useState(false)
  const [creating, setCreating] = useState(false)
  /** 正在编辑成员 / 策略的分组 id；同一时间只展开一行 */
  const [expanded, setExpanded] = useState<string | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<RoutingGroupDto | null>(null)

  /* ── 新建表单 ── */
  const [newName, setNewName] = useState("")
  const [newSlug, setNewSlug] = useState("")
  const [newStrategy, setNewStrategy] = useState("order")
  const [newMembers, setNewMembers] = useState<RoutingMember[]>([])
  const [newClassifier, setNewClassifier] = useState("")
  const [newTimeout, setNewTimeout] = useState("")

  /* ── 编辑表单（点开一行后从该分组带入） ── */
  const [editStrategy, setEditStrategy] = useState("order")
  const [editMembers, setEditMembers] = useState<RoutingMember[]>([])
  const [editClassifier, setEditClassifier] = useState("")
  const [editTimeout, setEditTimeout] = useState("")

  const reload = () => {
    onChanged?.()
    if (!isTauri) return
    gatewayApi.groupList()
      .then((rows) => { setGroups(rows); setLoadFailed(false) })
      .catch(() => setLoadFailed(true))
  }

  // 首次展开时读取；读取失败后收起再展开会重试
  useEffect(() => {
    if (!open || groups !== null || !isTauri) return
    let active = true
    gatewayApi.groupList()
      .then((rows) => { if (active) { setGroups(rows); setLoadFailed(false) } })
      .catch(() => { if (active) setLoadFailed(true) })
    return () => { active = false }
  }, [open, groups])

  const editing = expanded ? (groups?.find((item) => item.slug === expanded) ?? null) : null
  // 分组带回了未知策略时补一个原样显示的选项，避免一保存就被替换成默认策略
  const editStrategyOptions = editing && !ROUTING_STRATEGIES.some((item) => item.value === editStrategy)
    ? [...ROUTING_STRATEGIES, { value: editStrategy, text: strategyText(editStrategy) }]
    : ROUTING_STRATEGIES

  useEffect(() => {
    if (createNonce) setCreating(true)
  }, [createNonce])

  const toggleRow = (slug: string) => {
    if (expanded === slug) { setExpanded(null); return }
    const group = groups?.find((item) => item.slug === slug)
    if (!group) return
    setExpanded(slug)
    setEditStrategy(group.strategy)
    setEditMembers(group.members)
    setEditClassifier(group.intent?.classifier ?? "")
    setEditTimeout(group.intent?.timeout_ms ? String(group.intent.timeout_ms) : "")
  }

  // 左栏选中的分组：读到列表后展开它
  useEffect(() => {
    if (focus && groups && expanded !== focus && groups.some((item) => item.slug === focus)) toggleRow(focus)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus, groups])

  const resetCreating = () => {
    setCreating(false); setNewName(""); setNewSlug(""); setNewStrategy("order"); setNewMembers([]); setNewClassifier(""); setNewTimeout("")
  }

  const slugTrimmed = newSlug.trim()
  const slugError = routingSlugError(slugTrimmed)
  const slugTaken = !!groups?.some((item) => item.slug === slugTrimmed)
  const newIntentError = newStrategy === "intent" ? intentError(newMembers, newClassifier, newTimeout) : null
  const canCreate = !!newName.trim() && !!slugTrimmed && !slugError && !slugTaken && newMembers.length > 0 && !newIntentError
  const editIntentError = editStrategy === "intent" ? intentError(editMembers, editClassifier, editTimeout) : null

  const create = async () => {
    const name = newName.trim()
    const intent = newStrategy === "intent" ? intentConfig(newClassifier, newTimeout) : null
    const ok = await run(() => gatewayApi.groupCreate(slugTrimmed, name, newStrategy, newMembers, intent), `已新建分组「${name}」`)
    if (ok) { resetCreating(); reload() }
  }

  const saveEdit = async (group: RoutingGroupDto) => {
    const patch = editStrategy === "intent"
      ? { strategy: editStrategy, members: editMembers, intent: intentConfig(editClassifier, editTimeout) }
      : { strategy: editStrategy, members: editMembers }
    const ok = await run(() => gatewayApi.groupUpdate(group.slug, patch), "已保存分组")
    if (ok) { setExpanded(null); reload() }
  }

  return (
    <section className={embedded ? "" : "-mx-6 px-6 pb-1"}>
      {!embedded && <button type="button" aria-expanded={open} onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-1.5 rounded-lg px-1 py-1.5 text-left outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
        {open ? <ChevronDown aria-hidden className="size-3.5 shrink-0 text-jolu-muted" /> : <ChevronRight aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />}
        <span className="shrink-0 text-[12px] font-semibold text-jolu-text">路由分组（待确认）</span>
        {groups !== null && <span className="shrink-0 text-[11px] tabular-nums text-jolu-muted">{groups.length} 个</span>}
        <span className="flex-1" />
      </button>}
      {open && (
        <div className="flex flex-col gap-2.5 px-1 pb-2 pt-0.5">
          <p className="text-[9.5px] leading-relaxed text-jolu-muted">
            把多个「来源/模型」组成一个模型 id <code className="font-mono">group/分组id</code>，工具按它调用，网关按策略在成员间分摊与失败转移；按 Jolu.pen 画板确认后移入主界面。
          </p>
          {!isTauri ? (
            <p className="text-[11.5px] text-jolu-muted">桌面端可用。</p>
          ) : groups === null ? (
            <p className="text-[11.5px] text-jolu-muted">{loadFailed ? "分组读取失败，收起后再展开可重试。" : "读取分组…"}</p>
          ) : (
            <>
              <div className="flex flex-col">
                {groups.map((group) => {
                  const isOpen = expanded === group.slug
                  return (
                    <div key={group.slug} className="border-b border-jolu-table-divider last:border-b-0">
                      <div className="flex min-h-9 items-center gap-1.5">
                        <button type="button" aria-expanded={isOpen} onClick={() => toggleRow(group.slug)}
                          className="flex min-w-0 flex-1 items-center gap-1.5 rounded-md py-1.5 text-left outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
                          {isOpen ? <ChevronDown aria-hidden className="size-3.5 shrink-0 text-jolu-muted" /> : <ChevronRight aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />}
                          <span className="truncate text-[12px] font-semibold text-jolu-text">{group.name}</span>
                          <span className="shrink-0 font-mono text-[11px] text-jolu-muted">（{group.slug}）</span>
                          <span className="shrink-0 text-[11px] text-jolu-muted">· {strategyText(group.strategy)}</span>
                          <span className="shrink-0 text-[11px] text-jolu-muted">· {group.members.length} 个成员</span>
                          {group.members.some((member) => isMemberPaused(member, Date.now())) && (
                            <span className="shrink-0 text-[11px] text-jolu-warn">· {group.members.filter((member) => isMemberPaused(member, Date.now())).length} 个暂停中</span>
                          )}
                        </button>
                        <button type="button" className={cn(ICON_BUTTON, "hover:text-jolu-danger")} title="删除路由分组" aria-label={`删除路由分组 ${group.name}`}
                          disabled={busy} onClick={() => setConfirmDelete(group)}>
                          <Trash2 aria-hidden className="size-3.5" />
                        </button>
                      </div>
                      {isOpen && (
                        <div className="mb-2.5 flex flex-col gap-2 rounded-lg border border-jolu-border bg-jolu-side p-2.5">
                          <div className="flex items-center gap-2">
                            <span className="shrink-0 text-[9.5px] text-jolu-muted">策略</span>
                            <select value={editStrategy} disabled={busy} aria-label={`「${group.name}」的策略`} className={SELECT}
                              onChange={(event) => setEditStrategy(event.target.value)}>
                              {editStrategyOptions.map((item) => <option key={item.value} value={item.value}>{item.text}</option>)}
                            </select>
                          </div>
                          {strategyOrderNote(editStrategy) && <span className="text-[9.5px] text-jolu-muted">{strategyOrderNote(editStrategy)}</span>}
                          {editStrategy === "intent" && (
                            <IntentFields catalog={catalog} classifier={editClassifier} timeout={editTimeout} disabled={busy}
                              onClassifier={setEditClassifier} onTimeout={setEditTimeout} label={group.name} />
                          )}
                          <MemberPicker catalog={catalog} members={editMembers} disabled={busy} onChange={setEditMembers} showTier={editStrategy === "intent"} />
                          <div className="flex items-center gap-2">
                            <Button label="保存" primary disabled={busy || editMembers.length === 0 || !!editIntentError} onClick={() => void saveEdit(group)} />
                            <Button label="取消" disabled={busy} onClick={() => setExpanded(null)} />
                            {editMembers.length === 0 && <span className="text-[9.5px] text-jolu-warn">至少保留一个成员。</span>}
                            {editMembers.length > 0 && editIntentError && <span className="text-[9.5px] text-jolu-warn">{editIntentError}</span>}
                          </div>
                        </div>
                      )}
                    </div>
                  )
                })}
                {groups.length === 0 && (
                  <p className="py-3 text-center text-[11.5px] text-jolu-muted">还没有路由分组。在下面新建一个，把多个「来源/模型」组成一个可调用的 id。</p>
                )}
              </div>

              {creating ? (
                <div className="flex flex-col gap-2 rounded-lg border border-jolu-border bg-jolu-side p-2.5">
                  <span className="text-[11.5px] font-semibold text-jolu-text">新建路由分组</span>
                  <div className="flex flex-wrap items-end gap-2">
                    <label className="flex min-w-[140px] flex-1 flex-col gap-1">
                      <span className="text-[9.5px] text-jolu-muted">名称</span>
                      <input autoFocus value={newName} onChange={(event) => setNewName(event.target.value)} placeholder="如 主力轮换" aria-label="分组名称" className={INPUT} />
                    </label>
                    <label className="flex min-w-[140px] flex-1 flex-col gap-1">
                      <span className="text-[9.5px] text-jolu-muted">id</span>
                      <input value={newSlug} onChange={(event) => setNewSlug(event.target.value)} placeholder="如 main-team" aria-label="分组 id"
                        className={cn(INPUT, (slugError || slugTaken) && "border-jolu-danger focus:border-jolu-danger")} />
                    </label>
                    <label className="flex flex-col gap-1">
                      <span className="text-[9.5px] text-jolu-muted">策略</span>
                      <select value={newStrategy} disabled={busy} aria-label="新建分组的策略" className={SELECT}
                        onChange={(event) => setNewStrategy(event.target.value)}>
                        {ROUTING_STRATEGIES.map((item) => <option key={item.value} value={item.value}>{item.text}</option>)}
                      </select>
                    </label>
                  </div>
                  {strategyOrderNote(newStrategy) && <span className="text-[9.5px] text-jolu-muted">{strategyOrderNote(newStrategy)}</span>}
                  {slugError && <span className="text-[9.5px] text-jolu-danger">{slugError}</span>}
                  {slugTaken && <span className="text-[9.5px] text-jolu-danger">「{slugTrimmed}」已经有同 id 的分组。</span>}
                  {newStrategy === "intent" && (
                    <IntentFields catalog={catalog} classifier={newClassifier} timeout={newTimeout} disabled={busy}
                      onClassifier={setNewClassifier} onTimeout={setNewTimeout} label="新建分组" />
                  )}
                  <div className="flex flex-col gap-1">
                    <span className="text-[9.5px] text-jolu-muted">成员 · 勾选即加入，模型 id = 来源/模型</span>
                    <MemberPicker catalog={catalog} members={newMembers} disabled={busy} onChange={setNewMembers} showTier={newStrategy === "intent"} />
                  </div>
                  <div className="flex items-center gap-2">
                    <Button label="创建" primary disabled={busy || !canCreate} onClick={() => void create()} />
                    <Button label="取消" disabled={busy} onClick={resetCreating} />
                    {newMembers.length === 0 && <span className="text-[9.5px] text-jolu-muted">至少勾选一个成员。</span>}
                    {newMembers.length > 0 && newIntentError && <span className="text-[9.5px] text-jolu-warn">{newIntentError}</span>}
                  </div>
                </div>
              ) : (
                <button type="button" disabled={busy} onClick={() => setCreating(true)}
                  className="flex h-8 w-full items-center gap-1.5 rounded-lg border border-dashed border-jolu-border px-2.5 text-[12px] text-jolu-muted outline-none hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
                  <Plus aria-hidden className="size-3.5" />新建路由分组
                </button>
              )}
            </>
          )}
        </div>
      )}

      {confirmDelete && (
        <ConfirmDialog title={`删除路由分组「${confirmDelete.name}」？`}
          description={`删除后模型 id group/${confirmDelete.slug} 不再可用；成员本身（来源与模型）和历史记录不受影响。`}
          confirmLabel="删除分组" danger onClose={() => setConfirmDelete(null)}
          onConfirm={async () => {
            // throwOnFailure：失败把错误抛回确认框展示，确认框保持打开
            const ok = await run(() => gatewayApi.groupDelete(confirmDelete.slug), "已删除路由分组", true)
            if (ok) {
              if (expanded === confirmDelete.slug) setExpanded(null)
              reload()
            }
          }} />
      )}
    </section>
  )
}

/**
 * 「按意图」的判定设置（待确认：Jolu.pen 无画板）：判意图用的模型从目录里选（不含分组），
 * 超时可空（默认 4000 毫秒）。判定模型宜选便宜、快的小模型——每个新提问都会调一次。
 */
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
  // 已保存的判定模型不在目录里（来源下架等）时保留显示，不悄悄换成别的
  const options = classifier && !ids.includes(classifier) ? [classifier, ...ids] : ids
  return (
    <div className="flex flex-wrap items-end gap-2">
      <label className="flex min-w-[180px] flex-1 flex-col gap-1">
        <span className="text-[9.5px] text-jolu-muted">判意图用的模型（选便宜、快的）</span>
        <select value={classifier} disabled={disabled} aria-label={`「${label}」判意图用的模型`} className={SELECT}
          onChange={(event) => onClassifier(event.target.value)}>
          <option value="">选择模型…</option>
          {options.map((id) => <option key={id} value={id}>{id}{ids.includes(id) ? "" : "（不在目录）"}</option>)}
        </select>
      </label>
      <label className="flex flex-col gap-1">
        <span className="text-[9.5px] text-jolu-muted">判定超时（毫秒，可空）</span>
        <input value={timeout} disabled={disabled} inputMode="numeric" aria-label={`「${label}」的判定超时`}
          placeholder={`默认 ${INTENT_TIMEOUT_RANGE.fallback}`} onChange={(event) => onTimeout(event.target.value)} className={cn(INPUT, "w-[120px]")} />
      </label>
    </div>
  )
}

/** 成员多选：模型目录条目（勾选即成员）+ 目录外的现存成员（保留显示，可取消勾选移除）。 */
function MemberPicker({ catalog, members, disabled, onChange, showTier }: {
  catalog: CatalogModelDto[]
  members: RoutingMember[]
  disabled?: boolean
  onChange: (next: RoutingMember[]) => void
  /** 「按意图」时每个已选成员多一个强 / 轻档切换 */
  showTier?: boolean
}) {
  const rows: { key: string; member: RoutingMember; off: boolean }[] = [
    ...selectableCatalog(catalog).map((item) => ({ key: routingMemberId({ slug: item.slug, model: item.model }), member: { slug: item.slug, model: item.model }, off: false })),
    ...offCatalogMembers(members, catalog).map((member) => ({ key: routingMemberId(member), member, off: true })),
  ]
  return (
    <div className="max-h-[168px] overflow-y-auto rounded-lg border border-jolu-border bg-jolu-bg">
      {rows.map((row) => (
        <label key={row.key} className="flex h-7 cursor-pointer items-center gap-2 border-b border-jolu-table-divider px-2.5 text-[11.5px] last:border-b-0 hover:bg-jolu-side">
          <input type="checkbox" checked={hasRoutingMember(members, row.member)} disabled={disabled}
            onChange={() => onChange(toggleRoutingMember(members, row.member))} className="size-3.5 accent-[var(--jolu-accent)]" />
          <code className="min-w-0 flex-1 truncate font-mono text-jolu-text">{routingMemberId(row.member)}</code>
          {row.off && <span className="shrink-0 text-[9px] text-jolu-warn">不在目录</span>}
          {hasRoutingMember(members, row.member) && (() => {
            const current = members.find((item) => item.slug === row.member.slug && memberParts(item.model).base === memberParts(row.member.model).base) ?? row.member
            return (
              <>
                {showTier && (
                  /* 待确认：意图档位在 Jolu.pen 没有画板 */
                  <select value={isLightMember(current) ? "light" : "strong"} disabled={disabled} aria-label={`${routingMemberId(row.member)} 的意图档位`}
                    title="强档答需要动脑的提问，轻档答快问题"
                    onClick={(event) => event.stopPropagation()}
                    onChange={(event) => onChange(setMemberTier(members, row.member, event.target.value === "light"))}
                    className="h-5 rounded border border-jolu-border bg-jolu-bg px-1 text-[9px] text-jolu-muted outline-none focus:border-jolu-accent">
                    <option value="strong">强档</option>
                    <option value="light">轻档</option>
                  </select>
                )}
                {/* 待确认：固定档位与轮转权重在 Jolu.pen 没有画板 */}
                <select value={memberParts(current.model).effort ?? ""} disabled={disabled} aria-label={`${routingMemberId(row.member)} 的固定档位`}
                  title="固定推理档位：转发给这个成员时按此档位问上游（请求本来没要推理时不加）"
                  onClick={(event) => event.stopPropagation()}
                  onChange={(event) => onChange(setMemberEffort(members, row.member, event.target.value || null))}
                  className="h-5 rounded border border-jolu-border bg-jolu-bg px-1 text-[9px] text-jolu-muted outline-none focus:border-jolu-accent">
                  <option value="">跟随请求</option>
                  {MEMBER_EFFORTS.map((effort) => <option key={effort} value={effort}>{effort}</option>)}
                </select>
                <input type="number" min={1} max={100} value={current.weight ?? 1} disabled={disabled} aria-label={`${routingMemberId(row.member)} 的轮转权重`}
                  title="「轮转」策略下每轮出现几次"
                  onClick={(event) => event.stopPropagation()}
                  onChange={(event) => onChange(setMemberWeight(members, row.member, Number(event.target.value) || null))}
                  className="h-5 w-10 rounded border border-jolu-border bg-jolu-bg px-1 text-[9px] text-jolu-muted outline-none focus:border-jolu-accent" />
                <PauseControl member={current} disabled={disabled} onChange={(until) => onChange(pauseMember(members, row.member, until))} />
              </>
            )
          })()}
        </label>
      ))}
      {rows.length === 0 && <p className="px-2.5 py-3 text-[11.5px] text-jolu-muted">模型目录为空：先在来源行「拉取」模型清单，再来勾选成员。</p>}
    </div>
  )
}

/**
 * 成员暂停（待确认：Jolu.pen 无画板）。暂停中的成员不参与选路，到期自动恢复；
 * 全员暂停时网关如实报「所有成员均暂停」。改动随「保存」一起写入。
 */
function PauseControl({ member, disabled, onChange }: { member: RoutingMember; disabled?: boolean; onChange: (untilMs: number | null) => void }) {
  const now = Date.now()
  const paused = isMemberPaused(member, now)
  return (
    <span className="flex shrink-0 items-center gap-1">
      {paused && <span className="text-[9px] text-jolu-warn">{pausedText(member, now)}</span>}
      <select value="" disabled={disabled} aria-label={`暂停 ${routingMemberId(member)}`}
        onChange={(event) => {
          const value = event.target.value
          if (value === "resume") onChange(null)
          else if (value) onChange(Date.now() + Number(value) * 60_000)
        }}
        className="h-5 rounded border border-jolu-border bg-jolu-bg px-1 text-[9px] text-jolu-muted outline-none focus:border-jolu-accent">
        <option value="">{paused ? "调整" : "暂停"}</option>
        {PAUSE_CHOICES.map((choice) => <option key={choice.minutes} value={choice.minutes}>暂停 {choice.text}</option>)}
        {paused && <option value="resume">立即恢复</option>}
      </select>
    </span>
  )
}
