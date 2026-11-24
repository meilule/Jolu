import { useCallback, useEffect, useRef, useState } from "react"
import { EVENTS } from "@/lib/events"
import { api, invoke, isTauri, listenEvent } from "@/lib/api"
import { checkinApi, type CheckinRecordDto, type CheckinUpdate } from "@/lib/checkinApi"
import { demoCheckinApi } from "@/mock/checkin"
import { CHECKIN_EDITIONS, STATE_LABELS, applyCheckinUpdate, checkinAccountLabel, checkinSummary, checkinTargets, currentOutcome, initialCheckinAccount, rewardLabel, uniqueCheckinAccounts, type CheckinAccount, type CheckinEdition } from "@/lib/dailyCheckin"
import { localZone, zoneName } from "@/lib/usageStats"
import { cn } from "@/lib/utils"
import { Banner, Button, Card, CardTitle, Checkbox, EmptyState, IconButton, QuotaRing, SkeletonLines, Tabs, Tag } from "@/ui"
import { useToast } from "@/components/ui/Toast"
import { AccountConnectDialog } from "@/components/providers/AccountConnectDialog"
import { connectionProviderId, type ProviderId } from "@/lib/providerCatalog"
import { CheckinProviderDialog, CheckinRemoveAccountDialog } from "@/components/checkin/CheckinDialogs"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { CheckinAccountStats } from "@/components/checkin/CheckinAccountStats"
import { useAccountSelection, type AccountSelection } from "@/hooks/useAccountSelection"
import { AccountBatchToolbar, BatchAccountDeleteDialog, type BatchAccountTarget } from "@/components/providers/AccountBatchControls"
import { removeAccounts, type BatchAccountDeleteResult } from "@/lib/accountActions"

type Activity = { kind: "refresh" | "claim"; done: number; total: number; name: string } | null
const PRIVACY_KEY = "jolu:checkin:hideAccounts"
// 浏览器预览使用示例数据；桌面端读取失败时不回落到示例。
const checkinSource: typeof checkinApi = isTauri ? checkinApi : demoCheckinApi

/** Jolu.pen 模块 I（133–160）：四列摘要、右侧详情、统一操作区及批量删除/领取。 */
export function CheckinView({ active = true }: { active?: boolean }) {
  const [connecting, setConnecting] = useState<ProviderId | null>(null)
  const connectingRef = useRef<ProviderId | null>(null)
  const [choosingProvider, setChoosingProvider] = useState(false)
  const choosingProviderRef = useRef(false)
  const openConnection = (provider: ProviderId | null) => {
    if (!provider) return
    connectingRef.current = provider
    setConnecting(provider)
  }
  /**
   * ZCode 的礼包要在客户端里领（领取接口要求阿里云人机验证，Jolu 不代领）：
   * 卡片上「去 ZCode 领取」直接打开客户端。
   */
  const openZcodeClient = async () => {
    if (!isTauri) return
    try {
      await invoke<void>("open_zcode_client")
    } catch (error) {
      setError(String(error))
    }
  }
  const [accounts, setAccounts] = useState<CheckinAccount[]>([])
  const [records, setRecords] = useState<CheckinRecordDto[]>([])
  const [loaded, setLoaded] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [activity, setActivity] = useState<Activity>(null)
  const [removingAccount, setRemovingAccount] = useState<CheckinAccount | null>(null)
  const removingAccountRef = useRef<CheckinAccount | null>(null)
  const [removing, setRemoving] = useState(false)
  const [removeError, setRemoveError] = useState<string | null>(null)
  const selection = useAccountSelection(accounts)
  const [batchTargets, setBatchTargets] = useState<BatchAccountTarget[] | null>(null)
  const batchTargetsRef = useRef<BatchAccountTarget[] | null>(null)
  const [batchDeleting, setBatchDeleting] = useState(false)
  const [hidden, setHidden] = useState(() => { try { return localStorage.getItem(PRIVACY_KEY) === "1" } catch { return false } })
  const [statusFilter, setStatusFilter] = useState<"all" | "ready" | "claimed" | "auto">("all")
  const [, tick] = useState(0)
  const alive = useRef(true)
  const operating = useRef(false)
  const activeRef = useRef(active)
  activeRef.current = active
  const pendingRefresh = useRef(false)
  const refreshRef = useRef<() => Promise<void>>(async () => {})
  const toast = useToast()

  const refresh = useCallback(async (notify = false) => {
    if (connectingRef.current || choosingProviderRef.current || removingAccountRef.current || batchTargetsRef.current) { pendingRefresh.current = true; return }
    if (operating.current) return
    operating.current = true
    pendingRefresh.current = false
    if (alive.current) { setError(null); setActivity({ kind: "refresh", done: 0, total: 0, name: "" }) }
    try {
      const data = await checkinSource.overview()
      if (!alive.current) return
      let rows = data.accounts.map(initialCheckinAccount)
      setAccounts(rows)
      setRecords(data.records)
      setLoaded(true)
      const targets = rows.filter(a => a.connection_status !== "paused" && CHECKIN_EDITIONS.some(e => e.manual && e.platform === a.platform && e.edition === a.edition))
      let cursor = 0, done = 0
      // 只读状态至多三个并行；任何账号失败都保留独立结果。
      await Promise.all(Array.from({ length: Math.min(3, targets.length) }, async () => {
        while (alive.current && cursor < targets.length) {
          const account = targets[cursor++]
          let update: CheckinUpdate
          try { update = await checkinSource.status(account.id) }
          catch { update = failedUpdate(account, "签到状态读取失败，请重新查询") }
          // 先刷新缓存再挂载该身份的额度区，末个完成的账号也能拿到本次手动查询结果。
          if (notify && alive.current && activeRef.current && update.account_key !== null) {
            try { await api.connectionQuota(account.id, true) } catch { /* 额度区独立展示查询失败。 */ }
          }
          if (!alive.current) return
          rows = applyCheckinUpdate(rows, update)
          setAccounts(rows)
          setActivity({ kind: "refresh", done: ++done, total: targets.length, name: "" })
        }
      }))
      if (notify && alive.current) {
        const summary = checkinSummary(rows)
        const detail = `${summary.ready} 个可签到 · ${summary.claimed} 个已签到${summary.errors ? ` · ${summary.errors} 个需处理，请查看对应账号状态` : ""}`
        if (summary.errors) toast.warn("签到状态已刷新", detail)
        else toast.success("签到状态已刷新", detail)
      }
    } catch {
      if (alive.current) {
        setError("签到账号或记录读取失败，请重新读取。")
        if (notify) toast.danger("签到状态刷新失败", "签到账号或记录读取失败，请重新读取。")
      }
    } finally {
      operating.current = false
      if (alive.current) {
        setActivity(null)
        if (pendingRefresh.current) void refreshRef.current()
      }
    }
  }, [toast])
  refreshRef.current = refresh

  useEffect(() => {
    alive.current = true
    void refresh()
    let off: (() => void) | undefined
    void listenEvent(EVENTS.CONNECTIONS_CHANGED, () => {
      if (!alive.current) return
      if (operating.current) pendingRefresh.current = true
      else if (activeRef.current) void refresh()
    }).then(unlisten => { if (alive.current) off = unlisten; else unlisten() }).catch(() => {})
    return () => { alive.current = false; off?.() }
  }, [refresh])

  useEffect(() => {
    if (!active) return
    const onFocus = () => { if (document.visibilityState === "visible") void refresh() }
    const timer = window.setInterval(() => {
      tick(value => value + 1)
      if (document.visibilityState === "visible") void refresh()
    }, 60_000)
    window.addEventListener("focus", onFocus)
    document.addEventListener("visibilitychange", onFocus)
    if (loaded && !operating.current) void refresh()
    return () => { window.clearInterval(timer); window.removeEventListener("focus", onFocus); document.removeEventListener("visibilitychange", onFocus) }
    // 返回页面及每分钟只读刷新，避免跨日或原工具换账号后继续使用旧状态。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, refresh])

  const queryOne = async (account: CheckinAccount) => {
    if (operating.current) return
    operating.current = true
    setActivity({ kind: "refresh", done: 0, total: 1, name: account.name })
    try {
      const result = await checkinSource.status(account.id)
      if (alive.current) setAccounts(rows => applyCheckinUpdate(rows, result))
    } catch {
      if (alive.current) setAccounts(rows => applyCheckinUpdate(rows, failedUpdate(account, "签到状态读取失败，请重新查询")))
    } finally {
      operating.current = false
      if (alive.current) { setActivity(null); if (pendingRefresh.current) void refresh() }
    }
  }

  const run = async (selected?: CheckinAccount) => {
    if (operating.current) return
    const targets = checkinTargets(selected ? [selected] : accounts)
    if (!targets.length) return
    operating.current = true
    let rows = accounts, finished = 0, claimed = 0, failures = 0
    setActivity({ kind: "claim", done: 0, total: targets.length, name: "" })
    try {
      for (const account of targets) {
        if (!alive.current) break
        // 上一个结果已将重复连接同步为已领取时，不再提交相同身份。
        if (!checkinTargets(rows).some(a => a.id === account.id)) { finished++; continue }
        setActivity({ kind: "claim", done: finished, total: targets.length, name: hidden ? "账号" : account.name })
        let update: CheckinUpdate
        try { update = await checkinSource.claim(account.id, account.account_key!) }
        catch { update = failedUpdate(account, "领取结果尚未确认，请先刷新状态核实", "unknown") }
        rows = applyCheckinUpdate(rows, update)
        if (update.outcome.state === "claimed") claimed++
        else failures++
        finished++
        if (alive.current) { setAccounts(rows); setActivity({ kind: "claim", done: finished, total: targets.length, name: "" }) }
      }
      if (alive.current) {
        const title = failures ? `本次处理完成：${claimed} 个已签到，${failures} 个需处理` : `本次处理完成：${claimed} 个已签到`
        if (failures) toast.warn(title, "请查看对应账号状态；未自动重试领取。")
        else toast.success(title)
        try { setRecords((await checkinSource.overview()).records) }
        catch { setError("签到操作已结束，但最新记录读取失败，请刷新状态。") }
      }
    } finally {
      operating.current = false
      if (alive.current) { setActivity(null); if (pendingRefresh.current) void refresh() }
    }
  }

  const summary = checkinSummary(accounts)
  const unresolved = accounts.filter(a => !CHECKIN_EDITIONS.some(e => e.platform === a.platform && e.edition === a.edition))
  const operationBusy = activity !== null || removingAccount !== null || batchTargets !== null || batchDeleting
  const busy = operationBusy || selection.active
  const closeRemove = () => {
    if (operating.current) return
    removingAccountRef.current = null
    setRemovingAccount(null)
    setRemoveError(null)
    if (pendingRefresh.current) void refresh()
  }
  const removeAccount = async () => {
    const account = removingAccountRef.current
    if (!account || operating.current) return
    operating.current = true
    setRemoving(true)
    setRemoveError(null)
    try {
      await api.removeConnection(account.id)
      if (!alive.current) return
      setAccounts(rows => rows.filter(row => row.id !== account.id))
      removingAccountRef.current = null
      setRemovingAccount(null)
      toast.success("账号已删除", "已移除 Jolu 中的连接，签到记录保留。")
      pendingRefresh.current = true
    } catch (reason) {
      if (alive.current) setRemoveError(String(reason))
    } finally {
      operating.current = false
      if (alive.current) {
        setRemoving(false)
        if (!removingAccountRef.current && pendingRefresh.current) void refresh()
      }
    }
  }
  const closeBatch = () => {
    if (operating.current) return
    batchTargetsRef.current = null
    setBatchTargets(null)
    if (pendingRefresh.current) void refresh()
  }
  const removeSelected = async (ids: string[]): Promise<BatchAccountDeleteResult> => {
    if (operating.current) return { deletedIds: [], failed: ids.map(id => ({ id, message: "当前操作尚未结束，请稍后重试" })) }
    const confirmed = new Set(batchTargetsRef.current?.map(target => target.id))
    const accepted = ids.filter(id => confirmed.has(id))
    operating.current = true
    setBatchDeleting(true)
    try {
      const result = await removeAccounts(accepted, api.removeConnection)
      if (alive.current) {
        const deleted = new Set(result.deletedIds)
        setAccounts(rows => rows.filter(row => !deleted.has(row.id)))
        selection.discard(result.deletedIds)
        if (result.failed.length) toast.warn(`已删除 ${result.deletedIds.length} 个账号，${result.failed.length} 个未删除`, "失败账号已保留，可在确认窗口重试。")
        else toast.success(`已删除 ${result.deletedIds.length} 个账号`, "签到记录保留，原工具登录不受影响。")
        pendingRefresh.current = true
      }
      return result
    } finally {
      operating.current = false
      if (alive.current) setBatchDeleting(false)
    }
  }
  const privacy = () => setHidden(value => {
    try { localStorage.setItem(PRIVACY_KEY, value ? "0" : "1") } catch { /* 不影响本次隐私切换 */ }
    return !value
  })

  const editionMatches = (entry: CheckinEdition, filter: "all" | "ready" | "claimed" | "auto") => {
    if (filter === "all" || selection.active) return true
    if (filter === "auto") return !entry.manual
    const rows = uniqueCheckinAccounts(
      accounts.filter(a => a.platform === entry.platform && a.edition === entry.edition && a.connection_status !== "paused"),
    )
    if (filter === "ready") return rows.some(a => currentOutcome(a).state === "ready")
    if (filter === "claimed") return rows.some(a => currentOutcome(a).state === "claimed")
    return true
  }

  const actionableTotal = summary.ready + summary.claimed
  const progressPercent = actionableTotal > 0 ? Math.round((summary.claimed / actionableTotal) * 100) : null
  const autoCount = CHECKIN_EDITIONS.filter(e => !e.manual).length
  const unconnectedCount = CHECKIN_EDITIONS.filter(
    e => e.manual && !accounts.some(a => a.platform === e.platform && a.edition === e.edition && a.connection_status !== "paused"),
  ).length
  const visibleEditions = CHECKIN_EDITIONS.filter(entry => editionMatches(entry, statusFilter))
  const readyEditionsCount = CHECKIN_EDITIONS.filter(entry => editionMatches(entry, "ready")).length
  const claimedEditionsCount = CHECKIN_EDITIONS.filter(entry => editionMatches(entry, "claimed")).length

  // 页头（标题 + 窗口按钮）由外壳统一渲染，页面不再重复 <h1>。
  // `data-checkin-*` 是验收脚本的钩子，替换样式时必须保留。
  return <section aria-label="每日签到" className="@container flex min-w-0 flex-col gap-4">
    {/* R1-07 顶部大进度汇总卡 */}
    <Card className="flex-row flex-wrap items-center gap-4 px-5 py-4">
      <div data-checkin-toolbar className="flex min-w-0 flex-1 flex-wrap items-center gap-4">
        <div className="flex items-center gap-3.5">
          <QuotaRing size="md" usedPercent={progressPercent}>
            {loaded && actionableTotal > 0 ? `${summary.claimed}/${actionableTotal}` : "?"}
          </QuotaRing>
          <div className="flex min-w-0 flex-col gap-1">
            <span className="text-[16px] font-bold text-jn-text">
              {activity?.kind === "refresh"
                ? "正在查询账号签到状态…"
                : error
                  ? "账号状态可能已过期，请刷新核实"
                  : !loaded
                    ? "尚未读取账号状态"
                    : summary.ready > 0
                      ? `今天还有 ${summary.ready} 个账号可以签到`
                      : actionableTotal > 0
                        ? "今日可签到账号已全部完成"
                        : "尚未连接可签到账号"}
            </span>
            <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-jn-text-3">
              {loaded && !error ? (
                <>
                  <span>{summary.claimed} 个已签到</span>
                  {autoCount > 0 && <span>· {autoCount} 个无需签到（自动活跃奖励）</span>}
                  {unconnectedCount > 0 && <span>· {unconnectedCount} 个尚未连接</span>}
                  {summary.errors > 0 && <span className="font-medium text-jn-warning">· {summary.errors} 个需处理</span>}
                </>
              ) : (
                <span>刷新状态以查看各平台今日签到进度</span>
              )}
            </span>
          </div>
        </div>
        <span className="min-w-0 flex-1" />
        <div className="flex flex-wrap items-center gap-2">
          <IconButton
            icon={hidden ? "eyeOff" : "eye"}
            label={hidden ? "显示账号" : "隐藏账号"}
            kind="surface"
            onClick={privacy}
          />
          <Button kind="ghost" icon="refresh" disabled={busy} loading={activity?.kind === "refresh"} onClick={() => void refresh(true)}>
            刷新状态
          </Button>
          <Button kind="soft" icon="add" disabled={busy} onClick={() => { choosingProviderRef.current = true; setChoosingProvider(true) }}>
            添加账号
          </Button>
          <Button
            kind="primary"
            icon="check"
            disabled={busy || !summary.ready}
            loading={activity?.kind === "claim"}
            onClick={() => void run()}
          >
            {summary.ready > 0 ? `一键签到 ${summary.ready} 个` : "一键签到"}
          </Button>
        </div>
      </div>
    </Card>

    {/* R1-07 状态分段筛选条 + 重置时间 + 批量管理 */}
    <div className="flex flex-wrap items-center gap-2.5">
      <Tabs
        style="segment"
        value={selection.active ? "all" : statusFilter}
        onChange={(next) => { if (!selection.active) setStatusFilter(next) }}
        items={[
          { key: "all" as const, label: `全部 ${CHECKIN_EDITIONS.length}` },
          { key: "ready" as const, label: `可签到 ${readyEditionsCount}` },
          { key: "claimed" as const, label: `已签到 ${claimedEditionsCount}` },
          { key: "auto" as const, label: `无需签到 ${autoCount}` },
        ]}
      />
      <span className="text-xs text-jn-text-3">今天 00:00 重置 · {zoneName(localZone())}时区</span>
      <span className="min-w-0 flex-1" />
      <div data-checkin-batch-toolbar className="flex flex-wrap items-center justify-end gap-2">
        <AccountBatchToolbar selection={selection} total={accounts.length} disabled={operationBusy || !loaded} onDelete={() => {
          if (operating.current) return
          const targets = accounts.filter(account => selection.selected.has(account.id)).map(account => {
            const entry = CHECKIN_EDITIONS.find(item => item.platform === account.platform && item.edition === account.edition)
            return { id: account.id, label: checkinAccountLabel(account, hidden), provider: entry ? `${entry.name} ${entry.region}` : "版本未识别", current: account.id.startsWith("monitor:native:") }
          })
          if (!targets.length) return
          batchTargetsRef.current = targets
          setBatchTargets(targets)
        }} />
      </div>
    </div>

    {error && (
      <Banner tone="warning" action={<Button size="sm" kind="danger" disabled={busy} onClick={() => void refresh()}>重新读取</Button>}>
        {error}
      </Banner>
    )}

    {!loaded && busy ? (
      <Card><SkeletonLines lines={4} /></Card>
    ) : (
      <>
        {unresolved.length > 0 && (
          <Banner tone="warning" action={selection.active ? undefined : <Button size="sm" kind="ghost" disabled={busy} onClick={() => openConnection("claude")}>连接账号</Button>}>
            {unresolved.length} 个账号的版本无法核实，请重新授权对应版本。
          </Banner>
        )}

        {visibleEditions.length === 0 ? (
          <Card>
            <EmptyState
              icon="check"
              title="当前筛选下没有平台"
              reason="切换筛选分类，或点击「全部」查看全部签到平台。"
              action={<Button kind="soft" size="sm" onClick={() => setStatusFilter("all")}>查看全部</Button>}
            />
          </Card>
        ) : (
          <div className="grid grid-cols-1 gap-3.5 @[560px]:grid-cols-2 @[780px]:grid-cols-3 @[1040px]:grid-cols-4">
            {visibleEditions.map(entry => {
              const rows = accounts.filter(account => account.platform === entry.platform && account.edition === entry.edition)
              const displayed = selection.active ? rows : uniqueCheckinAccounts(rows.filter(account => account.connection_status !== "paused"))
              return <CheckinCard key={entry.id} entry={entry} accounts={displayed} paused={selection.active ? 0 : rows.filter(account => account.connection_status === "paused").length} busy={busy} selection={selection} selectionDisabled={operationBusy} hidden={hidden} active={active} onConnect={() => openConnection(connectionProviderId({ platformId: entry.platform, edition: entry.edition }))} onQuery={account => void queryOne(account)} onClaim={account => void run(account)} onRemove={account => { if (operating.current) return; removingAccountRef.current = account; setRemovingAccount(account); setRemoveError(null) }} onOpenZcodeClient={() => void openZcodeClient()} />
            })}
          </div>
        )}

        <Card header={<CardTitle hint="每次领取或查询成功后会追加一条">最近签到记录</CardTitle>}>
          {!records.length ? (
            <EmptyState
              icon="check"
              title="还没有签到记录"
              reason="在任一供应商卡片上点「签到」或「刷新状态」后，结果会记在这里。"
            />
          ) : (
            <ol className="flex max-h-64 flex-col gap-1 overflow-auto">
              {records.map(record => {
                const entry = CHECKIN_EDITIONS.find(e => e.platform === record.platform && e.edition === record.edition)
                const claimed = record.outcome.state === "claimed"
                return (
                  <li key={record.id} className="flex flex-wrap items-start justify-between gap-x-4 gap-y-1 rounded-jn-block px-2 py-2.5">
                    <div className="flex min-w-0 flex-1 flex-col gap-1">
                      <span className="break-words text-[13px] text-jn-text">
                        {entry ? `${entry.name} ${entry.region}` : record.platform} · {hidden ? "账号已隐藏" : record.name}
                      </span>
                      <span className={cn("break-words text-[11px] leading-relaxed", claimed ? "text-jn-primary-ink" : "text-jn-text-3")}>
                        {record.outcome.message}{record.outcome.reward !== null && ` · ${rewardLabel(record.outcome)}`}
                      </span>
                    </div>
                    <time dateTime={new Date(record.created_ms).toISOString()} className="shrink-0 font-mono text-[11px] text-jn-text-3">
                      {new Date(record.created_ms).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" })}
                    </time>
                  </li>
                )
              })}
            </ol>
          )}
        </Card>
      </>
    )}

    {connecting && <div data-checkin-connect-dialog><AccountConnectDialog platform={connecting} onClose={() => { connectingRef.current = null; setConnecting(null); if (pendingRefresh.current) void refresh() }} onSaved={() => { pendingRefresh.current = true }} /></div>}
    {choosingProvider && <CheckinProviderDialog onClose={() => { choosingProviderRef.current = false; setChoosingProvider(false); if (pendingRefresh.current) void refresh() }} onPick={provider => { choosingProviderRef.current = false; setChoosingProvider(false); openConnection(provider) }} />}
    {removingAccount && <CheckinRemoveAccountDialog account={checkinAccountLabel(removingAccount, hidden)} busy={removing} error={removeError} onClose={closeRemove} onConfirm={() => void removeAccount()} />}
    {batchTargets && <BatchAccountDeleteDialog targets={batchTargets} hidden={hidden} onClose={closeBatch} onConfirm={removeSelected} />}
  </section>
}


/**
 * ZCode 的「签到」实际是礼包领取：文案如实区分，不把领取说成每日签到（鼠鼠 2026-11-19）。
 *
 * 而且 **Jolu 不代领**：ZCode 领取接口要求阿里云人机验证（`code 3007`），验证 token
 * 只能在浏览器环境里生成。所以这里的动作是「去 ZCode 领取」——点开客户端让用户自己领，
 * 不假装能在这里完成（`unsupported` 状态如实表达「不在 Jolu 职责内」）。
 */
const claimVerb = (platform: string) => (platform === "zcode" ? "去 ZCode 领取" : "签到")
const claimedLabel = (platform: string) => (platform === "zcode" ? "本批已领取" : "本轮已签到")

function failedUpdate(account: CheckinAccount, message: string, state: "failed" | "unknown" = "failed"): CheckinUpdate {
  return { id: account.id, account_key: null, account: null, checked_at: Date.now(), statistics: null, outcome: { state, message, reward: null, unit: account.outcome.unit, period_key: null, reset_at: null } }
}

function CheckinCard({ entry, accounts, paused, busy, hidden, active, onConnect, onQuery, onClaim, onRemove, onOpenZcodeClient, selection, selectionDisabled }: {
  entry: CheckinEdition; accounts: CheckinAccount[]; paused: number; busy: boolean; hidden: boolean; active: boolean
  onConnect: () => void; onQuery: (account: CheckinAccount) => void; onClaim: (account: CheckinAccount) => void
  onRemove: (account: CheckinAccount) => void
  /** 「去 ZCode 领取」：打开客户端（领取需要人机验证，Jolu 不代领） */
  onOpenZcodeClient: () => void
  selection: AccountSelection; selectionDisabled: boolean
}) {
  const [detailsAccountId, setDetailsAccountId] = useState<string | null>(null)
  useEffect(() => {
    if (detailsAccountId && (!active || !accounts.some(account => account.id === detailsAccountId))) setDetailsAccountId(null)
  }, [active, accounts, detailsAccountId])
  // 卡片用统一的 Card 外壳（圆角 24、1px jn-line、软阴影），不再自己写 jolu-card。
  return <Card aria-label={`${entry.name} ${entry.region}`} data-checkin-edition={entry.id} className="min-h-[210px] gap-2.5">
    <header className="flex items-center justify-between gap-2">
      <h2 className="flex min-w-0 items-center gap-2.5 text-sm font-semibold text-jn-text">
        <PlatformLogo platform={entry.platform} size={22} />
        <span className="truncate">{entry.name}</span>
      </h2>
      <Tag>{entry.region}</Tag>
    </header>

    {!entry.manual && !accounts.length ? (
      <>
        <p className="text-[11px] text-jn-text-3">{entry.platform === "workbuddy" ? "自动活跃奖励" : "暂未开放"}</p>
        <p className="text-2xl font-bold text-jn-text">{entry.platform === "workbuddy" ? "无需签到" : "—"}</p>
        <p className="text-[11px] leading-relaxed text-jn-text-3">{entry.description}</p>
        <Button kind="ghost" size="sm" className="mt-auto w-full" disabled>
          {entry.platform === "workbuddy" ? "无需手动领取" : "暂未开放"}
        </Button>
      </>
    ) : !accounts.length ? (
      <>
        <p className="text-[11px] text-jn-text-3">{paused ? `${paused} 个账号已暂停连接` : "尚未连接账号"}</p>
        <p className="text-2xl font-bold text-jn-text">
          {entry.platform === "qoder" ? "每日 Credits" : entry.platform === "zcode" ? "礼包额度" : "每日积分"}
        </p>
        <p className="text-[11px] leading-relaxed text-jn-text-3">{entry.description}</p>
        <Button kind="soft" size="sm" className="mt-auto w-full" disabled={busy} onClick={() => onConnect()}>
          {paused ? "管理连接" : "去连接"}
        </Button>
      </>
    ) : (
      <div className="flex flex-1 flex-col gap-3.5">
        {accounts.map(account => {
          const outcome = currentOutcome(account)
          const ready = outcome.state === "ready", claimed = outcome.state === "claimed"
          const accountPaused = account.connection_status === "paused"
          const label = checkinAccountLabel(account, hidden)
          const verb = claimVerb(entry.platform)
          const actionLabel = accountPaused ? "已暂停" : !entry.manual ? entry.platform === "workbuddy" ? "无需手动领取" : "暂未开放" : ready ? verb : claimed ? claimedLabel(entry.platform) : outcome.state === "unauthorized" ? "更新登录" : outcome.state === "inactive" ? "活动未开放" : STATE_LABELS[outcome.state]
          // ZCode 的 unsupported 不是「不能点」而是「去客户端领」：可点，点了打开 ZCode
          const zcodeClient = entry.platform === "zcode" && outcome.state === "unsupported"
          const action = (
            <Button
              size="sm"
              kind={ready || claimed ? "soft" : "ghost"}
              className="min-w-0 flex-1"
              disabled={busy || accountPaused || !entry.manual || claimed || (outcome.state === "unsupported" && !zcodeClient) || outcome.state === "inactive"}
              aria-label={`${entry.name} ${entry.region} ${label} ${ready ? verb : claimed ? claimedLabel(entry.platform) : outcome.state === "unauthorized" ? "更新登录" : "查询状态"}`}
              onClick={() => zcodeClient ? onOpenZcodeClient() : ready ? onClaim(account) : outcome.state === "unauthorized" ? onConnect() : onQuery(account)}
            >
              {actionLabel}
            </Button>
          )
          const deleteAction = (
            <Button size="sm" kind="danger" aria-label={`删除账号 ${label}`} disabled={busy} onClick={() => onRemove(account)}>
              删除
            </Button>
          )
          return (
            <div
              key={account.id}
              data-checkin-account={account.id}
              className={cn(
                "flex flex-1 flex-col gap-2",
                // 批量选择：选中的行加一圈主色描边，未选中的不加——用边框而不是底色，
                // 避免与卡片自己的底色叠成第三种颜色
                selection.active && "rounded-jn-block border p-2",
                selection.active && (selection.selected.has(account.id) ? "border-jn-primary" : "border-transparent"),
              )}
            >
              <div className="flex items-start gap-2">
                {selection.active && (
                  <Checkbox
                    label={`选择账号 ${label}`}
                    checked={selection.selected.has(account.id)}
                    disabled={selectionDisabled}
                    onChange={() => selection.toggle(account.id)}
                    className="mt-0.5"
                  />
                )}
                <span className="min-w-0 flex-1 break-all text-[11px] text-jn-text-3">账号：{label}</span>
                {accountPaused && <Tag>已暂停</Tag>}
              </div>
              <div className="flex flex-wrap items-baseline justify-between gap-2">
                <p className={cn("font-mono text-2xl font-bold tabular-nums", ready || claimed ? "text-jn-primary-ink" : "text-jn-text")}>
                  {!entry.manual ? (entry.platform === "workbuddy" ? "自动活跃奖励" : "暂未开放") : outcome.reward !== null ? rewardLabel(outcome) : claimed ? "已领取" : "—"}
                </p>
                {entry.manual && (
                  <span className="text-[11px] text-jn-text-3">{entry.platform === "zcode" ? "本批领取额度" : "今日签到奖励"}</span>
                )}
              </div>
              <CheckinAccountStats
                key={`${account.id}:${account.account_key ?? account.account ?? "unknown"}`}
                accountId={account.id}
                statistics={account.statistics}
                official={outcome.official_statistics}
                identityKey={accountPaused ? null : entry.manual ? account.account_key : account.account ?? null}
                active={active}
                showHistory={entry.manual}
                accountLabel={label}
                providerLabel={`${entry.name} ${entry.region}`}
                statusMessage={accountPaused ? "已暂停查询，配置与历史保留。" : account.checked_at === null ? "还没有查询过签到状态" : outcome.message}
                detailsOpen={detailsAccountId === account.id}
                onDetailsOpenChange={open => setDetailsAccountId(open ? account.id : null)}
                action={action}
                deleteAction={deleteAction}
              />
            </div>
          )
        })}
        {paused > 0 && (
          <button
            type="button"
            disabled={busy}
            onClick={() => onConnect()}
            className="jn-interactive text-left text-[11px] text-jn-text-3 underline underline-offset-2 disabled:opacity-45"
          >
            另有 {paused} 个账号已暂停连接
          </button>
        )}
      </div>
    )}
  </Card>
}
