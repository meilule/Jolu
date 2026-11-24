import { useContentMotion } from "@/lib/motion"
import { useCollectionStatus } from "@/hooks/useCollectionStatus"
import { collectionWarning } from "@/lib/usageStats"
import { LoadingSurface } from "@/components/ui/LoadingSurface"
import { CompressDialog } from '@/components/sessions/CompressDialog'
import { HandoffPreview } from '@/components/sessions/HandoffPreview'
import { handoffApi } from '@/lib/handoffApi'
import type { HandoffResult, HandoffState } from '@/lib/sessionHandoff'
import { useEffect, useMemo, useState } from "react"
import { cn } from "@/lib/utils"
import { api, isTauri, type SessionItemDto } from "@/lib/api"
import {
  TIME_PRESETS, UNKNOWN_PROJECT, groupByProject, projectKey, projectName, sessionKey, sessionTotals, sinceFor,
  type SessionSort, type TimePreset,
} from "@/lib/sessionThreads"
import { formatPercent, formatTokens } from "@/lib/usageStats"
import { UNATTRIBUTED_ACCOUNT, accountLabel, accountsFor } from "@/lib/usageFilters"
import { useConnections } from "@/hooks/useConnections"
import { useStatsQuery } from "@/hooks/useStatsQuery"
import { demoSessions } from "@/mock/sessionThreads"
import { useToast } from "@/components/ui/Toast"
import { SessionDetail } from "@/components/sessions/SessionDetail"
import { SessionTree } from "@/components/sessions/SessionTree"
import { AutomationConfirmDialog, ContinueDialog } from "@/components/sessions/sessionDialogs"
import { providerName } from "@/lib/providerCatalog"
import { ALL, ErrorNotice, MenuFilter, providerOptions } from "@/components/usage/usageControls"
import { Icon, type IconName } from "@/icons"
import { Banner, Button, EmptyState, IconButton, Input, PageActions } from "@/ui"
import type { PlatformId } from "@/types"

export interface SessionTarget {
  platform: string
  sessionId: string
  nonce: number
}

/**
 * 会话线程（Jolu.pen 模块 D，画板 40–57）：顶部统计、搜索与筛选、项目 → 会话树与只读详情。
 * 压缩会话通过独立 CLI 调用生成交接草稿，不修改原会话。
 */
export function SessionsView({ active, target, onDrillUsage, onCreateAutomation }: {
  active: boolean
  target: SessionTarget | null
  onDrillUsage: (session: SessionItemDto) => void
  onCreateAutomation: (session: SessionItemDto) => void
}) {
  const toast = useToast()
  const conn = useConnections()
  const [platform, setPlatform] = useState<PlatformId | null>(null)
  const collection = useCollectionStatus()
  const collectionNote = collectionWarning(collection.status, platform)
  const [account, setAccount] = useState<string | null>(null)
  const [project, setProject] = useState<string | null>(null)
  const [time, setTime] = useState<TimePreset>("all")
  const [sort, setSort] = useState<SessionSort>("recent")
  const [searchDraft, setSearchDraft] = useState("")
  const [search, setSearch] = useState("")
  const [selected, setSelected] = useState<string | null>(null)
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [compressing,setCompressing]=useState<SessionItemDto|null>(null)
  const [handoff,setHandoff]=useState<{result:HandoffResult;name:string;target:string;session:SessionItemDto}|null>(null)
  const [handoffState,setHandoffState]=useState<HandoffState|null>(null)
  const [continuing, setContinuing] = useState<SessionItemDto | null>(null)
  const [automating, setAutomating] = useState<SessionItemDto | null>(null)
  const [syncedAt, setSyncedAt] = useState(() => Date.now())
  const [scanning, setScanning] = useState(false)
  /** 外部跳转要定位的会话：列表刷新完成前不被「默认选中最近会话」覆盖 */
  const [pending, setPending] = useState<string | null>(null)

  const query = useStatsQuery(
    "sessions:" + JSON.stringify([platform, account, time, search]),
    () => api.sessionsList({ platform, connection: account, project: null, since: sinceFor(time), search: search.trim() || null }),
    () => demoSessions(Date.now()),
    platform,
    active,
  )
  const listMotion = useContentMotion(String(!!query.data))
  useEffect(() => { if (query.data) setSyncedAt(Date.now()) }, [query.data])

  const all = useMemo(() => query.data?.sessions ?? [], [query.data])
  const projects = useMemo(() => {
    const seen = new Map<string, string>()
    for (const s of all) if (!seen.has(projectKey(s.cwd))) seen.set(projectKey(s.cwd), projectName(s.cwd))
    return [...seen.entries()].sort((a, b) => (!a[0] !== !b[0] ? (a[0] ? -1 : 1) : a[1].localeCompare(b[1])))
  }, [all])
  const sessions = useMemo(() => (project === null ? all : all.filter((s) => projectKey(s.cwd) === project)), [all, project])
  const groups = useMemo(() => groupByProject(sessions, sort), [sessions, sort])
  const totals = sessionTotals(sessions)
  /** 列表尚未读到时统计未知，显示「—」而不是 0 */
  const known = query.data !== null
  const current = sessions.find((s) => sessionKey(s) === selected) ?? null
  const searching = !!search.trim()
  const filtered = !!(platform || account || project !== null || time !== "all" || searching)

  // 默认选中最近的会话并展开它所在的项目；搜索时展开全部匹配项目
  useEffect(() => {
    if (pending) {
      if (sessions.some((s) => sessionKey(s) === pending)) {
        setSelected(pending)
        setPending(null)
        return
      }
      if (query.loading) return
      setPending(null)
      toast.info("没有找到这条会话", "可能已被清理或不在本机记录中")
    }
    if (!groups.length) return
    if (searching) setExpanded(new Set(groups.map((g) => g.key)))
    if (!current) {
      const first = groups[0].sessions[0]
      setSelected(sessionKey(first))
      setExpanded((value) => new Set([...value, groups[0].key]))
    }
  }, [groups, searching, current, pending, query.loading])

  // 从用量统计跳转过来时定位到指定会话
  useEffect(() => {
    if (!target) return
    setPlatform(null)
    setAccount(null)
    setProject(null)
    setTime("all")
    setSearch("")
    setSearchDraft("")
    setPending(`${target.platform}:${target.sessionId}`)
  }, [target])
  useEffect(() => {
    const found = current ?? null
    if (found) setExpanded((value) => (value.has(projectKey(found.cwd)) ? value : new Set([...value, projectKey(found.cwd)])))
  }, [current])

  const toggle = (key: string, open?: boolean) => setExpanded((value) => {
    const next = new Set(value)
    if (open ?? !next.has(key)) next.add(key)
    else next.delete(key)
    return next
  })

  const clearAll = () => {
    setPlatform(null)
    setAccount(null)
    setProject(null)
    setTime("all")
    setSearch("")
    setSearchDraft("")
  }

  const rescan = async () => {
    if (!isTauri) return query.refresh()
    setScanning(true)
    try {
      await api.scanLocalSessions()
    } catch (reason) {
      toast.danger("重新扫描失败", reason instanceof Error ? reason.message : String(reason))
    } finally {
      setScanning(false)
      query.refresh()
    }
  }

  const accountOptions = [
    { value: ALL, label: "全部账号" },
    ...accountsFor(conn.connections, platform).map((c) => ({ value: c.id, label: accountLabel(c) })),
    { value: UNATTRIBUTED_ACCOUNT, label: "未归属账号" },
  ]
  const minutes = Math.floor((Date.now() - syncedAt) / 60_000)

  return (
    <div className="relative flex w-full flex-col gap-3.5">
      {/* 同步状态与刷新进页头操作槽（R1-05） */}
      <PageActions>
        <span className="whitespace-nowrap text-xs text-jn-text-3">{minutes < 1 ? "刚刚同步" : `${minutes} 分钟前同步`}</span>
        <IconButton
          icon={scanning || query.refreshing ? "loading" : "refresh"}
          label="刷新会话"
          kind="surface"
          size="md"
          disabled={scanning || query.refreshing}
          className={scanning || query.refreshing ? "[&_svg]:animate-spin" : undefined}
          onClick={() => void rescan()}
        />
      </PageActions>

      {/* 工具条：搜索 + 全部筛选 + 排序（R1-05） */}
      <div className="flex flex-wrap items-center gap-2">
        <Input
          size="md"
          icon="search"
          value={searchDraft}
          onChange={(event) => {
            setSearchDraft(event.target.value)
            if (!event.target.value) setSearch("")
          }}
          onKeyDown={(event) => event.key === "Enter" && setSearch(searchDraft)}
          placeholder="搜索标题、会话 ID 或可用正文"
          aria-label="搜索会话"
          className="w-[240px] shrink-0 bg-jn-surface shadow-jn-surface"
        />
        <MenuFilter trailingChevron label="供应商筛选" value={platform ?? ALL} options={providerOptions()} onChange={(v) => {
          const next = v === ALL ? null : (v as PlatformId)
          setPlatform(next)
          if (account && account !== UNATTRIBUTED_ACCOUNT && !accountsFor(conn.connections, next).some((c) => c.id === account)) setAccount(null)
        }} />
        <MenuFilter trailingChevron label="账号筛选" value={account ?? ALL} options={accountOptions} onChange={(v) => setAccount(v === ALL ? null : v)} />
        <MenuFilter
          trailingChevron
          label="项目筛选"
          value={project === null ? ALL : project || "__unknown__"}
          options={[{ value: ALL, label: "全部项目" }, ...projects.map(([key, name]) => ({ value: key || "__unknown__", label: key ? name : UNKNOWN_PROJECT }))]}
          onChange={(v) => setProject(v === ALL ? null : v === "__unknown__" ? "" : v)}
        />
        <MenuFilter trailingChevron label="时间筛选" value={time} options={TIME_PRESETS} onChange={(v) => setTime(v as TimePreset)} />
        <span className="min-w-0 flex-1" />
        <MenuFilter
          trailingChevron
          label="排序"
          value={sort}
          options={[{ value: "recent", label: "最近更新", icon: <Icon name="filter" size="xs" className="text-jn-text-3" /> }, { value: "tokens", label: "消耗最多", icon: <Icon name="filter" size="xs" className="text-jn-text-3" /> }]}
          onChange={(v) => setSort(v as SessionSort)}
        />
      </div>

      {/* 汇总条（R1-05：圆角 20 白卡一行汇总，窄屏可折行） */}
      <div className="flex flex-wrap items-center gap-2 rounded-[20px] border border-jn-line bg-jn-surface px-4 py-3 shadow-jn-surface">
        <Stat selected icon="navSessions" label="线程" value={known ? String(totals.threads) : "—"} note="去重会话" />
        <span className="h-9 w-px bg-jn-line" />
        <Stat icon="layers" label="消耗 Token" value={known ? formatTokens(totals.consumption) : "—"} note={totals.unknown ? `${totals.unknown} 条未知` : "输入 + 输出"} />
        <span className="h-9 w-px bg-jn-line" />
        <Stat icon="spark" label="缓存命中" value={known ? formatPercent(totals.hitRate) : "—"} note="缓存占输入" />
        <span className="h-9 w-px bg-jn-line" />
        <Stat icon="download" label="输入" value={known ? formatTokens(totals.input) : "—"} note="含缓存" />
        <span className="h-9 w-px bg-jn-line" />
        <Stat icon="database" label="缓存输入" value={known ? formatTokens(totals.cache) : "—"} note="已复用" />
        <span className="h-9 w-px bg-jn-line" />
        <Stat icon="upload" label="输出" value={known ? formatTokens(totals.output) : "—"} note="生成" />
        <span className="h-9 w-px bg-jn-line" />
        <Stat icon="fileText" label="来源文件" value={query.data ? String(query.data.files) : "—"} note="去重文件" />
      </div>

      {query.error && <ErrorNotice error={query.error} stale={query.data !== null} onRetry={query.refresh} />}
      {collectionNote && <Banner tone="warning">{collectionNote}</Banner>}

      {/* 详情面板最小宽 420：更窄的窗口整行横向滚动（列表 + 详情一起拖），内容不再被裁破版 */}
      <div ref={listMotion} className="flex h-[630px] gap-4 overflow-x-auto pb-1 [scrollbar-width:thin]">
        {!query.data ? (
          !query.error && <LoadingSurface layout="sessions" label="正在读取本机会话" />
        ) : !all.length && !filtered ? (
          <div className="flex flex-1 items-center justify-center rounded-jn-card border border-jn-line bg-jn-surface p-8 shadow-jn-surface">
            <EmptyState
              icon="navSessions"
              title="还没有本机会话"
              reason="先在支持的工具中开始会话，再回来重新扫描。"
              action={<Button kind="soft" size="sm" icon="refresh" loading={scanning} onClick={() => void rescan()}>重新扫描</Button>}
            />
          </div>
        ) : !sessions.length ? (
          <div className="flex flex-1 items-center justify-center rounded-jn-card border border-jn-line bg-jn-surface p-8 shadow-jn-surface">
            <EmptyState
              icon="search"
              title="没有找到匹配的会话"
              reason="尝试缩短关键词，或清除供应商、项目与日期筛选。"
              action={<Button kind="ghost" size="sm" icon="retry" onClick={clearAll}>清除筛选</Button>}
            />
          </div>
        ) : (
          <>
            <SessionTree
              groups={groups}
              expanded={expanded}
              selected={current ? sessionKey(current) : null}
              searching={searching}
              total={sessions.length}
              onToggle={toggle}
              onSelect={setSelected}
              onClear={clearAll}
            />
            {current && (
              <SessionDetail
                key={sessionKey(current)}
                session={current}
                connections={conn.connections}
                onDrillUsage={() => onDrillUsage(current)}
                onCompress={() => setCompressing(current)}
                onContinue={() => setContinuing(current)}
                onCreateAutomation={() => setAutomating(current)}
              />
            )}
          </>
        )}
      </div>
      {compressing && <CompressDialog session={compressing} onClose={()=>setCompressing(null)} onResult={(result,name,target)=>{setHandoff({result,name,target,session:compressing});setCompressing(null);setHandoffState(null);if(isTauri)void handoffApi.state(compressing.platform,compressing.session_id).then(setHandoffState).catch(()=>{})}}/>}
      {handoff && <HandoffPreview {...handoff} fileName={handoff.name} state={handoffState} onClose={()=>setHandoff(null)} onUpdate={()=>{setCompressing(handoff.session);setHandoff(null)}}/>}
      {continuing && (
        <ContinueDialog
          session={continuing}
          accountKnown={!!continuing.connection_id}
          onClose={() => setContinuing(null)}
          onDone={() => {
            toast.success(`已在 ${providerName(continuing.platform)} 中打开`, "后续消息由原工具处理")
            setContinuing(null)
          }}
        />
      )}
      {automating && (
        <AutomationConfirmDialog
          session={automating}
          onClose={() => setAutomating(null)}
          onConfirm={() => {
            onCreateAutomation(automating)
            setAutomating(null)
          }}
        />
      )}
    </div>
  )
}

function Stat({ icon, label, value, note, selected }: { icon: IconName; label: string; value: string; note: string; selected?: boolean }) {
  return (
    <div
      title={note}
      className={cn(
        "flex min-w-[92px] flex-1 flex-col justify-center gap-1 rounded-[14px] px-2.5 py-1",
        selected && "bg-jn-primary-wash",
      )}
    >
      <span className={cn("flex items-center gap-1.5 whitespace-nowrap text-[11px] font-medium", selected ? "text-jn-primary-ink" : "text-jn-text-3")}>
        <Icon name={icon} size="xs" />
        {label}
      </span>
      <span className={cn("tnum truncate font-mono text-[18px] font-bold leading-none", selected ? "text-jn-primary-ink" : "text-jn-text")}>{value}</span>
    </div>
  )
}
