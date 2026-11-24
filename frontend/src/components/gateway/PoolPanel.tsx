import { useEffect, useState } from "react"
import type { CatalogModelDto } from "@/lib/gatewayApi"
import { gatewayApi } from "@/lib/gatewayApi"
import { isTauri } from "@/lib/api"
import { accountQueries, formatMoney } from "@/lib/accountDisplay"
import { remainingPercent } from "@/lib/quota"
import { useBalance, useQuota } from "@/hooks/useQuota"
import type { Connection } from "@/types"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { useToast } from "@/components/ui/Toast"
import { Icon } from "@/icons"
import { Button, IconButton, Input, StatusPill, Tag } from "@/ui"
import { cn } from "@/lib/utils"
import { ZcodeChannelOption } from "./ZcodeChannelOption"

/**
 * 模型路由 · 模型池 —— 设计稿 R1-20（浅色）：只读。
 *
 * 顶部：标题 + 来源 / 模型计数 + 搜索 +「刷新模型信息 / 管理来源」；
 * 来源卡：可展开的卡片（标识、名称、默认标签、`slug /`、模型数、剩余额度、管理入口），
 * 展开后是模型行（全名、上下文、复制）。添加 / 停用来源或改前缀统一在「供应商 → 账号管理」。
 */
export function PoolPanel({ connections, catalog, defaultConnection, busy = false, run, onOpenConnection, onManage }: {
  connections: Connection[]
  catalog: CatalogModelDto[]
  defaultConnection: string | null
  busy?: boolean
  run?: (work: () => Promise<unknown>, ok?: string) => Promise<boolean>
  onOpenConnection: (connectionId: string) => void
  onManage: () => void
}) {
  const serving = connections.filter((connection) => connection.serve_enabled)
  const [expanded, setExpanded] = useState<string | null>(serving[0]?.id ?? null)
  const [showAll, setShowAll] = useState<Record<string, boolean>>({})
  const [search, setSearch] = useState("")
  const query = search.trim().toLowerCase()
  const total = catalog.filter((model) => model.slug && serving.some((connection) => connection.slug === model.slug)).length
  const hasZcode = connections.some((connection) => connection.platformId === "zcode")
  return (
    <div className="flex min-w-0 flex-col gap-3.5">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[14px] font-semibold text-jn-text">模型池</span>
        <span className="text-[12px] text-jn-text-3">{serving.length} 个来源 · {total} 个模型</span>
        <span className="min-w-0 flex-1" />
        <Input size="md" icon="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索模型" aria-label="搜索模型" className="w-[200px] shrink-0" />
        <MetaRefreshButton />
        <Button kind="primary" size="sm" icon="external" onClick={onManage}>管理来源</Button>
      </div>

      {serving.length === 0 && (
        <p className="rounded-jn-card border border-dashed border-jn-line px-5 py-8 text-center text-[12px] text-jn-text-3">
          还没有启用中的来源。先在供应商里添加账号或 API Key，并打开「进入模型池」。
        </p>
      )}

      <div className="flex flex-col gap-3">
        {serving.map((connection) => {
          const models = catalog.filter((model) => model.slug && model.slug === connection.slug && (!query || `${model.slug}/${model.model}`.toLowerCase().includes(query)))
          const open = expanded === connection.id || Boolean(query)
          const shown = showAll[connection.id] ? models : models.slice(0, 6)
          return (
            <section key={connection.id} className="flex flex-col overflow-hidden rounded-jn-card border border-jn-line bg-jn-surface shadow-jn-surface">
              <div className="flex min-h-[68px] items-center gap-3 py-2 pr-4 pl-[18px]">
                <button type="button" aria-expanded={open} onClick={() => setExpanded(open && !query ? null : connection.id)}
                  className="flex min-w-0 flex-1 items-center gap-3 text-left">
                  <Icon name={open ? "expand" : "next"} size="sm" className="shrink-0 text-jn-text-3" />
                  <span aria-hidden className="flex size-10 shrink-0 items-center justify-center rounded-[13px] bg-jn-well">
                    <PlatformLogo platform={connection.platformId} size={30} />
                  </span>
                  <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className="flex items-center gap-1.5">
                      <span className="truncate text-[14px] font-bold text-jn-text">{connection.name}</span>
                      {defaultConnection === connection.id && <Tag>默认</Tag>}
                    </span>
                    <span className="truncate text-[11.5px] text-jn-text-3">{platformLabel(connection)} · {kindLabel(connection)}</span>
                  </span>
                  <code className="max-w-[140px] shrink-0 truncate font-mono text-[12.5px] font-medium text-jn-text-2">{connection.slug ?? "—"}/</code>
                  <span className="flex h-5 shrink-0 items-center rounded-full bg-jn-well px-2 text-[11px] font-bold text-jn-text-2">{models.length} 个模型</span>
                  <SourceQuotaBadge connection={connection} />
                </button>
                <Button kind="ghost" size="sm" icon="external" onClick={() => onOpenConnection(connection.id)}>管理来源</Button>
              </div>
              {open && (
                <div className="flex flex-col gap-0.5 px-3 pb-3">
                  {models.length === 0 && (
                    <p className="px-3 py-4 text-[12px] text-jn-text-3">{query ? "没有匹配的模型。" : "这个来源还没有模型清单：先在供应商连接里「拉取模型」。"}</p>
                  )}
                  {shown.map((model, index) => <ModelRow key={model.id} model={model} striped={index % 2 === 0} />)}
                  {models.length > shown.length && (
                    <button type="button" onClick={() => setShowAll((current) => ({ ...current, [connection.id]: true }))}
                      className="self-start px-3 pt-2 text-[12px] font-semibold text-jn-primary-ink">
                      查看另外 {models.length - shown.length} 个模型
                    </button>
                  )}
                </div>
              )}
            </section>
          )
        })}
      </div>

      {hasZcode && run && <ZcodeChannelOption busy={busy} run={run} />}
    </div>
  )
}

function ModelRow({ model, striped }: { model: CatalogModelDto; striped: boolean }) {
  const toast = useToast()
  const id = `${model.slug}/${model.model}`
  const copy = () => {
    void navigator.clipboard.writeText(id).then(() => toast.success("已复制")).catch(() => toast.danger("复制失败", "请手动选中复制"))
  }
  return (
    <div className={cn("flex h-[42px] items-center gap-2.5 rounded-[12px] pr-2 pl-3.5", striped && "bg-jn-well")}>
      <Icon name="file" size="sm" className="shrink-0 text-jn-text-3" />
      <code className="min-w-0 flex-1 truncate font-mono text-[12.5px] font-medium text-jn-text" title={id}>{id}</code>
      <span className="shrink-0 text-[11.5px] text-jn-text-3">{model.context_window ? `上下文 ${Math.round(model.context_window / 1000)}K` : "上下文未知"}</span>
      <IconButton icon="copy" label={`复制 ${id}`} size="sm" onClick={copy} />
    </div>
  )
}

/** 模型元数据刷新（R1-20「刷新模型信息」按钮；models.dev 快照状态在 title 里）。 */
function MetaRefreshButton() {
  const [status, setStatus] = useState<[number, number] | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (!isTauri) return
    gatewayApi.modelsDevStatus().then(setStatus).catch(() => setStatus(null))
  }, [])
  if (!isTauri) return null
  const [fetched, count] = status ?? [0, 0]
  const title = !status ? "读取元数据状态…" : fetched === 0 ? `当前是内置快照（${count} 个模型）` : `上次拉取：${new Date(fetched).toLocaleString("zh-CN", { hour12: false })}（${count} 个模型）`
  return (
    <Button kind="ghost" size="sm" icon="refresh" loading={busy} title={title}
      onClick={async () => {
        setBusy(true)
        try {
          const [total, at] = await gatewayApi.modelsDevRefresh()
          setStatus([at, total])
        } catch { /* 状态仅作提示，失败保持原值 */ }
        setBusy(false)
      }}>刷新模型信息</Button>
  )
}

/** R1-20 来源行右侧真实额度 / 余额读数（不主动触发强制刷新，未知时不伪造 0%） */
function SourceQuotaBadge({ connection }: { connection: Connection }) {
  const queries = accountQueries(connection)
  const quota = useQuota(queries.quota, false, Boolean(isTauri && queries.quota))
  const balance = useBalance(isTauri ? queries.balance : null, Boolean(isTauri && queries.balance))

  if (connection.kind === "api") {
    if (balance.state?.state === "ok" && typeof balance.state.balance === "number") {
      return (
        <span className="hidden shrink-0 items-center gap-1 rounded-full bg-jn-well px-2.5 py-0.5 text-[11.5px] sm:inline-flex">
          <span className="text-jn-text-3">余额</span>
          <span className="tnum font-mono font-semibold text-jn-text">{formatMoney(balance.state.balance, balance.state.currency)}</span>
        </span>
      )
    }
    return <Tag className="hidden sm:inline-flex">按量</Tag>
  }

  const first = quota.state?.state === "ok"
    ? quota.state.windows.find((w) => typeof w.used_percent === "number") ?? null
    : null
  const rem = first && typeof first.used_percent === "number" ? remainingPercent(first.used_percent) : null
  if (rem === null) return null
  return (
    <StatusPill tone={rem <= 15 ? "warning" : "success"} className="hidden sm:inline-flex">
      {`剩余 ${Math.round(rem)}%`}
    </StatusPill>
  )
}

function platformLabel(connection: Connection): string {
  const names: Record<string, string> = {
    claude: "Claude Code", codex: "Codex", gemini: "Gemini CLI", opencode: "OpenCode", zcode: "ZCode",
    grok: "Grok", workbuddy: "WorkBuddy", qoder: "Qoder", trae: "Trae",
  }
  return names[connection.platformId] ?? connection.platformId
}

function kindLabel(connection: Connection): string {
  if (connection.kind === "api") return "API Key"
  if (connection.platformId === "claude" || connection.platformId === "codex") return "订阅"
  return "订阅连接"
}
