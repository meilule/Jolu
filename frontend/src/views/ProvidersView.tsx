/**
 * 供应商页 —— 设计稿 `docs/ui/jolu-new.pen` R1-03（供应商连接）与 R1-39（账号管理）。
 *
 * 页签送进主窗口页头（`PageTabs`）：
 * - 供应商连接：顶部概况横幅 → 已连接供应商卡 → 分组可连接目录（编码工具账号 / 模型服务 API / 本机与自定义）→ 连接安全说明；
 * - 账号管理：复用 `AccountsPanel` 管理各账号的额度、余额、切换与批量操作。
 */
import { useEffect, useMemo, useState, type ReactNode } from "react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import {
  PROVIDERS,
  canConnect,
  connectionBadge,
  connectionProviderId,
  type ProviderEntry,
  type ProviderId,
} from "@/lib/providerCatalog"
import { useConnections } from "@/hooks/useConnections"
import { useMainContextReport } from "@/hooks/useMainContextReport"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { ContentTransition, LoadingSurface } from "@/components/ui/LoadingSurface"
import { AccountConnectDialog } from "@/components/providers/AccountConnectDialog"
import { ImportDialog } from "@/components/providers/ImportDialog"
import { AccountsPanel } from "@/components/providers/AccountsPanel"
import { Icon, type IconName } from "@/icons"
import {
  Banner,
  Button,
  Card,
  Count,
  IconButton,
  Input,
  PageTabs,
  StatusPill,
  Tabs,
  Tag,
  Tooltip,
  type StatusTone,
} from "@/ui"
import type { Connection } from "@/types"

export type ProvidersTab = "connect" | "accounts"
type ConnectedFilter = "all" | "auth" | "api"
type CatalogFilter = "all" | "subscription" | "apiKey" | "local"

const LOCAL_IDS = new Set<ProviderId>(["ollama", "lmstudio", "custom"])

function isLocalOrCustom(entry: ProviderEntry): boolean {
  return LOCAL_IDS.has(entry.id)
}

function isToolAccount(entry: ProviderEntry): boolean {
  return !isLocalOrCustom(entry) && entry.subscription !== "none"
}

export function ProvidersView({
  initialTab = "connect",
  accountTarget,
  onAccountTargetHandled,
}: {
  initialTab?: ProvidersTab
  accountTarget?: { connectionId: string | null; nonce: number } | null
  onAccountTargetHandled?: () => void
}) {
  const [tab, setTab] = useState<ProvidersTab>(accountTarget ? "accounts" : initialTab)
  const conn = useConnections()
  const [connecting, setConnecting] = useState<ProviderId | null>(null)
  const [importOpen, setImportOpen] = useState(false)

  useEffect(() => {
    if (!accountTarget) return
    setTab("accounts")
    if (!accountTarget.connectionId) onAccountTargetHandled?.()
  }, [accountTarget, onAccountTargetHandled])

  // 4.2b：在账号管理里定位到某个账号时，告诉本体岛「主窗口在看这个连接」
  useMainContextReport("providers", tab === "accounts" ? accountTarget?.connectionId ?? null : null)

  return (
    <div className="flex h-full min-h-0 w-full flex-1 flex-col gap-3.5 overflow-hidden px-0.5 pb-2 pt-0.5">
      <PageTabs>
        <Tabs
          value={tab}
          onChange={setTab}
          items={[
            { key: "connect", label: "供应商连接", icon: "blocks" },
            { key: "accounts", label: "账号管理", icon: "users" },
          ]}
        />
      </PageTabs>

      <ContentTransition
        identity={`${tab}-${conn.loading && !conn.connections.length}`}
        className={cn("flex min-h-0 flex-1 flex-col gap-3.5", tab === "connect" && "overflow-y-auto pr-0.5 [&>*]:shrink-0")}
      >
        {conn.loading && !conn.connections.length && !conn.error ? (
          <LoadingSurface
            className={tab === "accounts" ? "min-h-0 flex-1" : undefined}
            layout={tab === "connect" ? "providers" : "accounts"}
            label="正在读取供应商连接"
          />
        ) : tab === "connect" ? (
          <ConnectDirectory
            connections={conn.connections}
            loading={conn.loading}
            error={conn.error}
            onConnect={setConnecting}
            onImport={() => setImportOpen(true)}
            onManageAccounts={() => setTab("accounts")}
            onRetry={() => void conn.reload()}
          />
        ) : (
          <AccountsPanel
            conn={conn}
            onConnect={setConnecting}
            target={accountTarget?.connectionId ? { connectionId: accountTarget.connectionId, nonce: accountTarget.nonce } : null}
            onTargetHandled={onAccountTargetHandled}
          />
        )}
      </ContentTransition>

      {connecting && (
        <AccountConnectDialog
          platform={connecting}
          onClose={() => setConnecting(null)}
          onSaved={() => void conn.reload()}
        />
      )}

      {importOpen && <ImportDialog onClose={() => setImportOpen(false)} />}
    </div>
  )
}

interface ProviderGroupSummary {
  entry: ProviderEntry
  connections: Connection[]
  hasAuth: boolean
  hasApi: boolean
  attentionCount: number
  pausedCount: number
}

function ConnectDirectory({
  connections,
  loading,
  error,
  onConnect,
  onImport,
  onManageAccounts,
  onRetry,
}: {
  connections: readonly Connection[]
  loading: boolean
  error: string | null
  onConnect: (platform: ProviderId) => void
  onImport: () => void
  onManageAccounts: () => void
  onRetry: () => void
}) {
  const [connFilter, setConnFilter] = useState<ConnectedFilter>("all")
  const [catalogFilter, setCatalogFilter] = useState<CatalogFilter>("all")
  const [query, setQuery] = useState("")

  const byProvider = useMemo(() => {
    const map = new Map<ProviderId, Connection[]>()
    for (const c of connections) {
      const id = connectionProviderId(c)
      if (!id) continue
      const list = map.get(id) ?? []
      list.push(c)
      map.set(id, list)
    }
    return map
  }, [connections])

  const connectedSummaries = useMemo<ProviderGroupSummary[]>(() => {
    const list: ProviderGroupSummary[] = []
    for (const entry of PROVIDERS) {
      const items = byProvider.get(entry.id)
      if (!items?.length) continue
      list.push({
        entry,
        connections: items,
        hasAuth: items.some((c) => c.kind === "auth"),
        hasApi: items.some((c) => c.kind === "api"),
        attentionCount: items.filter((c) => c.status === "expired" || c.status === "invalid" || c.status === "offline").length,
        pausedCount: items.filter((c) => c.status === "paused").length,
      })
    }
    return list
  }, [byProvider])

  const filteredConnected = useMemo(() => {
    if (connFilter === "all") return connectedSummaries
    return connectedSummaries.filter((s) => (connFilter === "auth" ? s.hasAuth : s.hasApi))
  }, [connectedSummaries, connFilter])

  const attentionTotal = useMemo(
    () => connections.filter((c) => c.status === "expired" || c.status === "invalid" || c.status === "offline").length,
    [connections],
  )

  const trimmed = query.trim().toLowerCase()
  const catalogMatches = useMemo(() => {
    return PROVIDERS.filter((entry) => {
      if (catalogFilter === "subscription" && entry.subscription === "none") return false
      if (catalogFilter === "apiKey" && entry.apiKey === "none") return false
      if (catalogFilter === "local" && !isLocalOrCustom(entry)) return false
      if (!trimmed) return true
      return (
        entry.name.toLowerCase().includes(trimmed) ||
        entry.id.toLowerCase().includes(trimmed) ||
        entry.access.toLowerCase().includes(trimmed) ||
        (entry.baseUrl?.value.toLowerCase().includes(trimmed) ?? false)
      )
    })
  }, [catalogFilter, trimmed])

  const toolGroup = catalogMatches.filter(isToolAccount)
  const apiGroup = catalogMatches.filter((e) => !isToolAccount(e) && !isLocalOrCustom(e))
  const localGroup = catalogMatches.filter(isLocalOrCustom)

  const importButton = isTauri ? (
    <Button kind="primary" size="md" icon="import" onClick={onImport}>
      从本机导入
    </Button>
  ) : (
    <Tooltip content="桌面运行时才能读取本机配置" side="left">
      <span className="inline-flex">
        <Button kind="primary" size="md" icon="import" disabled>
          从本机导入
        </Button>
      </span>
    </Tooltip>
  )

  return (
    <div className="@container flex min-w-0 flex-col gap-5 pb-2">
      {/* 顶部概况横幅（R1-03） */}
      <section className="relative flex flex-wrap items-center gap-5 overflow-hidden rounded-jn-card border border-jn-line bg-jn-surface px-6 py-5 shadow-jn-surface">
        <span aria-hidden className="pointer-events-none absolute -top-24 -left-16 size-[220px] rounded-full bg-jn-primary-wash/80" />
        <span aria-hidden className="pointer-events-none absolute -top-16 right-8 size-[180px] rounded-full border-[18px] border-jn-primary-soft/60" />

        <div className="relative flex min-w-[240px] flex-1 flex-col gap-1">
          <h2 className="text-[20px] font-bold tracking-tight text-jn-text">连接你的 AI 工具</h2>
          <p className="text-xs leading-relaxed text-jn-text-2">
            订阅账号、API Key 与本机已登录的工具都在这里统一接入；详细额度与余额在「账号管理」查看。
          </p>
        </div>

        <div className="relative flex flex-wrap items-center gap-5">
          <HeroStat value={loading && !connections.length ? "—" : String(connections.length)} label="已连接账号" onClick={onManageAccounts} />
          <HeroStat value={loading && !connections.length ? "—" : String(connectedSummaries.length)} label="家供应商" />
          {attentionTotal > 0 && (
            <HeroStat value={String(attentionTotal)} label="需要处理" tone="warning" onClick={onManageAccounts} />
          )}
          {importButton}
        </div>
      </section>

      {error && (
        <Banner
          tone="danger"
          action={
            <Button kind="danger" size="sm" onClick={onRetry}>
              重新读取
            </Button>
          }
        >
          连接列表读取失败：{error}；账号数量暂不可用。
        </Banner>
      )}

      {/* 已连接区块（R1-03） */}
      {connectedSummaries.length > 0 && (
        <section className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-2.5">
            <h3 className="text-[15px] font-bold text-jn-text">已连接</h3>
            <Count tone="primary">{connectedSummaries.length}</Count>
            <span className="flex-1" />
            <Tabs
              style="segment"
              value={connFilter}
              onChange={setConnFilter}
              items={[
                { key: "all", label: "全部" },
                { key: "auth", label: "订阅" },
                { key: "api", label: "API Key" },
              ]}
            />
          </div>

          {filteredConnected.length > 0 ? (
            <div className="grid grid-cols-1 gap-3.5 @[620px]:grid-cols-2 @[940px]:grid-cols-3 @[1180px]:grid-cols-4">
              {filteredConnected.map((summary) => (
                <ConnectedProviderCard
                  key={summary.entry.id}
                  summary={summary}
                  onAdd={() => onConnect(summary.entry.id)}
                  onManage={onManageAccounts}
                />
              ))}
            </div>
          ) : (
            <p className="rounded-jn-card border border-dashed border-jn-line px-4 py-6 text-center text-xs text-jn-text-3">
              当前筛选下没有已连接的供应商
            </p>
          )}
        </section>
      )}

      {/* 可连接分组目录（R1-03 扩展版） */}
      <section className="flex flex-col gap-3.5">
        <div className="flex flex-wrap items-center gap-2.5">
          <h3 className="text-[15px] font-bold text-jn-text">可连接</h3>
          <Count>{catalogMatches.length}</Count>
          <span className="hidden text-xs text-jn-text-3 @[760px]:inline">
            按可用接入方式连接，不支持的能力不会显示
          </span>
          <span className="flex-1" />
          <Tabs
            style="segment"
            value={catalogFilter}
            onChange={setCatalogFilter}
            items={[
              { key: "all", label: "全部" },
              { key: "subscription", label: "订阅登录" },
              { key: "apiKey", label: "API Key" },
              { key: "local", label: "本机" },
            ]}
          />
          <Input
            size="md"
            icon="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索供应商"
            aria-label="搜索供应商"
            className="w-[200px] shrink-0"
          />
        </div>

        {catalogMatches.length === 0 ? (
          <p className="rounded-jn-card border border-dashed border-jn-line px-4 py-10 text-center text-xs text-jn-text-3">
            没有匹配的供应商
          </p>
        ) : (
          <div className="flex flex-col gap-5">
            {toolGroup.length > 0 && (
              <CatalogGroup
                icon="terminal"
                title="编码工具账号"
                hint="用工具自己的账号登录或接入"
                entries={toolGroup}
                byProvider={byProvider}
                loading={loading || Boolean(error)}
                onConnect={onConnect}
              />
            )}
            {apiGroup.length > 0 && (
              <CatalogGroup
                icon="key"
                title="模型服务 API"
                hint="填写 Base URL 与 API Key，获取模型后保存"
                entries={apiGroup}
                byProvider={byProvider}
                loading={loading || Boolean(error)}
                onConnect={onConnect}
              />
            )}
            {localGroup.length > 0 && (
              <CatalogGroup
                icon="cpu"
                title="本机与自定义"
                hint="本机模型服务或任意 OpenAI / Anthropic 兼容接口"
                entries={localGroup}
                byProvider={byProvider}
                loading={loading || Boolean(error)}
                onConnect={onConnect}
              />
            )}
          </div>
        )}
      </section>

      {/* 底部两栏：最近动态 + 连接安全（R1-03） */}
      <div className="grid grid-cols-1 gap-3.5 @[820px]:grid-cols-[minmax(0,1fr)_320px]">
        <RecentActivityCard connections={connections} onManageAccounts={onManageAccounts} />

        <Card tone="wash" className="flex-col gap-2.5 p-5">
          <span className="grid size-9 shrink-0 place-items-center rounded-xl bg-jn-surface text-jn-primary shadow-jn-surface">
            <Icon name="shield" size="md" />
          </span>
          <span className="text-[14px] font-bold text-jn-text">凭据只在本机后端处理</span>
          <span className="text-xs leading-relaxed text-jn-text-2">
            API Key 与登录凭据加密保存在本机，前端列表只显示掩码，不写入日志、统计导出和诊断报告。
          </span>
          <span className="flex-1" />
          <button
            type="button"
            onClick={onManageAccounts}
            className="jn-interactive inline-flex items-center gap-1 self-start rounded-full px-1 py-0.5 text-xs font-semibold text-jn-primary-ink hover:underline"
          >
            前往账号管理核对
            <Icon name="next" size="xs" />
          </button>
        </Card>
      </div>
    </div>
  )
}

/** R1-03 底部左卡「最近动态」：按真实连接状态与同步记录派生，绝不编造假事件。 */
function RecentActivityCard({
  connections,
  onManageAccounts,
}: {
  connections: readonly Connection[]
  onManageAccounts: () => void
}) {
  const items = useMemo(() => {
    const rank = (status: Connection["status"]) =>
      status === "invalid" || status === "offline" ? 0 : status === "paused" ? 1 : 2
    return [...connections]
      .sort((a, b) => rank(a.status) - rank(b.status))
      .slice(0, 4)
      .map((c) => {
        const prov = PROVIDERS.find((p) => p.id === connectionProviderId(c))?.name ?? c.platformId
        if (c.status === "invalid" || c.status === "offline") {
          return {
            id: c.id,
            tone: "warning" as const,
            icon: "warning" as IconName,
            title: `${prov} · ${c.baseName} 需要处理`,
            detail: c.status === "invalid" ? "认证已失效，请重新登录或更换密钥" : "连接离线，请检查网络或配置",
            time: c.lastSyncText || "—",
          }
        }
        if (c.status === "paused") {
          return {
            id: c.id,
            tone: "neutral" as const,
            icon: "pause" as IconName,
            title: `${prov} · ${c.baseName} 已暂停查询`,
            detail: "历史统计与配置保留，随时可在账号管理恢复",
            time: c.lastSyncText || "已暂停",
          }
        }
        return {
          id: c.id,
          tone: "success" as const,
          icon: "success" as IconName,
          title: `${prov} · ${c.baseName} 连接正常`,
          detail: `${c.kind === "auth" ? "订阅连接" : "API Key"}${c.serve_enabled ? " · 已加入模型路由" : ""}`,
          time: c.lastSyncText || "已连接",
        }
      })
  }, [connections])

  return (
    <Card
      className="gap-2 p-5"
      header={
        <>
          <Icon name="clock" size="md" className="shrink-0 text-jn-text-2" />
          <span className="text-sm font-semibold text-jn-text">最近动态</span>
          <span className="flex-1" />
          {connections.length > 0 && (
            <button
              type="button"
              onClick={onManageAccounts}
              className="jn-interactive inline-flex items-center gap-0.5 rounded-full px-2 py-0.5 text-xs font-semibold text-jn-primary-ink hover:bg-jn-primary-wash"
            >
              全部账号
              <Icon name="next" size="xs" />
            </button>
          )}
        </>
      }
    >
      {items.length === 0 ? (
        <p className="py-4 text-xs text-jn-text-3">还没有已连接的账号。接入一个供应商后，状态与同步记录会显示在这里。</p>
      ) : (
        <div className="flex flex-col gap-1">
          {items.map((item) => (
            <button
              key={item.id}
              type="button"
              onClick={onManageAccounts}
              className="jn-interactive flex items-center gap-3 rounded-[14px] px-2.5 py-2 text-left hover:bg-jn-well"
            >
              <span
                className={cn(
                  "grid size-8 shrink-0 place-items-center rounded-[10px]",
                  item.tone === "warning"
                    ? "bg-jn-warning-soft text-jn-warning"
                    : item.tone === "success"
                      ? "bg-jn-primary-wash text-jn-primary"
                      : "bg-jn-well text-jn-text-3",
                )}
              >
                <Icon name={item.icon} size="sm" />
              </span>
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="truncate text-[13px] font-semibold text-jn-text">{item.title}</span>
                <span className="truncate text-[11px] text-jn-text-3">{item.detail}</span>
              </span>
              <span className="shrink-0 text-[11px] text-jn-text-3">{item.time}</span>
            </button>
          ))}
        </div>
      )}
    </Card>
  )
}

function HeroStat({
  value,
  label,
  tone = "default",
  onClick,
}: {
  value: string
  label: string
  tone?: "default" | "warning"
  onClick?: () => void
}) {
  const inner = (
    <>
      <span className={cn("tnum font-mono text-[22px] leading-none font-bold", tone === "warning" ? "text-jn-warning" : "text-jn-text")}>
        {value}
      </span>
      <span className="text-[11px] text-jn-text-3">{label}</span>
    </>
  )
  if (!onClick) {
    return <div className="flex flex-col items-start gap-1 px-1">{inner}</div>
  }
  return (
    <button
      type="button"
      onClick={onClick}
      className="jn-interactive flex flex-col items-start gap-1 rounded-xl px-2 py-1 text-left hover:bg-jn-well"
    >
      {inner}
    </button>
  )
}

function ConnectedProviderCard({
  summary,
  onAdd,
  onManage,
}: {
  summary: ProviderGroupSummary
  onAdd: () => void
  onManage: () => void
}) {
  const { entry, connections, attentionCount, pausedCount } = summary
  const primary = connections.find((c) => c.status === "connected") ?? connections[0]
  const statusTone: StatusTone = attentionCount > 0 ? "warning" : pausedCount === connections.length ? "neutral" : "success"
  const statusLabel = attentionCount > 0 ? `${attentionCount} 个需处理` : pausedCount === connections.length ? "已暂停查询" : "可用"

  return (
    <div className="flex min-w-0 flex-col gap-3 rounded-jn-card border border-jn-line bg-jn-surface p-4 shadow-jn-surface">
      <div className="flex items-center gap-2.5">
        <span className="grid size-9 shrink-0 place-items-center rounded-xl bg-jn-well">
          {entry.id === "custom" ? (
            <Icon name="connect" size="md" className="text-jn-primary" />
          ) : (
            <PlatformLogo platform={entry.id} size={20} />
          )}
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-[14px] font-bold text-jn-text">{entry.name}</span>
          <span className="truncate text-[11px] text-jn-text-3">
            {connections.length} 个连接 · {[summary.hasAuth && "订阅", summary.hasApi && "API Key"].filter(Boolean).join(" / ")}
          </span>
        </div>
        <IconButton icon="add" label={`添加 ${entry.name} 账号`} size="sm" kind="ghost" onClick={onAdd} />
      </div>

      {/* 当前主账号槽（R1-03 浅灰块） */}
      <div className="flex items-center gap-2.5 rounded-[14px] bg-jn-well px-3 py-2.5">
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-[12.5px] font-semibold text-jn-text">{primary.name}</span>
          <span className="truncate text-[11px] text-jn-text-3">
            {primary.kind === "auth" ? "订阅连接" : "API Key"}
            {connections.length > 1 ? ` · 等 ${connections.length} 个账号` : ""}
          </span>
        </span>
        <Tag>{primary.kind === "auth" ? "订阅" : "API"}</Tag>
      </div>

      <div className="flex items-center justify-between gap-2 pt-0.5">
        <StatusPill tone={statusTone}>{statusLabel}</StatusPill>
        <button
          type="button"
          onClick={onManage}
          className="jn-interactive inline-flex items-center gap-0.5 rounded-full px-2 py-1 text-xs font-semibold text-jn-primary-ink hover:bg-jn-primary-wash"
        >
          管理
          <Icon name="next" size="xs" />
        </button>
      </div>
    </div>
  )
}

function CatalogGroup({
  icon,
  title,
  hint,
  entries,
  byProvider,
  loading,
  onConnect,
}: {
  icon: IconName
  title: string
  hint: string
  entries: readonly ProviderEntry[]
  byProvider: Map<ProviderId, Connection[]>
  loading: boolean
  onConnect: (id: ProviderId) => void
}) {
  return (
    <div className="flex flex-col gap-2.5">
      <div className="flex items-center gap-2">
        <Icon name={icon} size="sm" className="text-jn-text-2" />
        <span className="text-[13px] font-semibold text-jn-text">{title}</span>
        <span className="text-xs text-jn-text-3">{entries.length} 个</span>
        <span className="hidden text-[11.5px] text-jn-text-3 @[680px]:inline">· {hint}</span>
      </div>
      <div className="grid grid-cols-1 gap-3 @[620px]:grid-cols-2 @[940px]:grid-cols-3 @[1180px]:grid-cols-4">
        {entries.map((entry) => {
          const count = loading ? null : (byProvider.get(entry.id)?.length ?? 0)
          return <CatalogCard key={entry.id} entry={entry} count={count} onConnect={() => onConnect(entry.id)} />
        })}
      </div>
    </div>
  )
}

function CatalogCard({
  entry,
  count,
  onConnect,
}: {
  entry: ProviderEntry
  count: number | null
  onConnect: () => void
}) {
  const ready = canConnect(entry)
  const isCustom = entry.id === "custom"
  const subLabel = entry.baseUrl?.value && !isCustom
    ? entry.baseUrl.value.replace(/^https?:\/\//, "")
    : entry.access

  const modeTags: ReactNode[] = []
  if (entry.subscription === "ready") modeTags.push(<Tag key="sub">订阅登录</Tag>)
  if (entry.apiKey === "ready") modeTags.push(<Tag key="api">API Key</Tag>)
  if (isCustom) {
    modeTags.length = 0
    modeTags.push(<Tag key="oai">OpenAI 兼容</Tag>, <Tag key="ant">Anthropic 兼容</Tag>)
  }

  return (
    <div className="flex min-w-0 flex-col gap-2.5 rounded-[20px] border border-jn-line bg-jn-surface p-3.5 shadow-jn-surface">
      <div className="flex items-center gap-2.5">
        <span className={cn("grid size-9 shrink-0 place-items-center rounded-xl", isCustom ? "bg-jn-primary-wash text-jn-primary" : "bg-jn-well")}>
          {isCustom ? <Icon name="connect" size="md" /> : <PlatformLogo platform={entry.id} size={20} />}
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-[13.5px] font-bold text-jn-text">{entry.name}</span>
          <span className="truncate text-[11px] text-jn-text-3" title={subLabel}>{subLabel}</span>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-1.5">{modeTags}</div>

      <div className="mt-auto flex items-center justify-between gap-2 border-t border-jn-line pt-2.5">
        <span className={cn("truncate text-[11.5px]", count ? "font-medium text-jn-primary-ink" : "text-jn-text-3")}>
          {!ready ? "待接入" : count === null ? "读取中" : count > 0 ? connectionBadge(count) : "未连接"}
        </span>
        <Button kind={count ? "ghost" : "soft"} size="sm" icon="add" disabled={!ready} onClick={onConnect}>
          {count ? "添加" : "连接"}
        </Button>
      </div>
    </div>
  )
}
