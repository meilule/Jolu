import { useEffect, useState } from "react"
import { ChevronRight, Copy, FolderOpen, Package } from "lucide-react"
import { cn } from "@/lib/utils"
import { gatewayApi, type CatalogModelDto } from "@/lib/gatewayApi"
import { isTauri } from "@/lib/api"
import type { Connection } from "@/types"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { useToast } from "@/components/ui/Toast"

/**
 * 模型路由 · 模型池（Jolu.pen「统一网关 · R2 重设计」V6 画板 gaGlT）：只读。
 * 来源手风琴（logo、默认 badge、`slug /`、模型数、管理来源）+ 展开的模型行
 * （全名、上下文、复制）；添加 / 停用来源或改前缀在供应商连接里，点「管理来源」过去。
 * 「来源分组」按钮沿用「管理来源」弹窗（V6 没有单独画板，标「待确认」）。
 */
export function PoolPanel({ connections, catalog, defaultConnection, onOpenConnection, onManage }: {
  connections: Connection[]
  catalog: CatalogModelDto[]
  defaultConnection: string | null
  onOpenConnection: (connectionId: string) => void
  /** 来源分组 / 排序 / 批量加入（沿用「管理来源」弹窗） */
  onManage: () => void
}) {
  const serving = connections.filter((connection) => connection.serve_enabled)
  const modelTotal = catalog.filter((model) => model.slug && serving.some((connection) => connection.slug === model.slug)).length
  const [expanded, setExpanded] = useState<string | null>(serving[0]?.id ?? null)
  const [showAll, setShowAll] = useState<Record<string, boolean>>({})
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 px-8 pb-8 pt-2">
      <div className="flex items-center justify-between gap-4">
        <div className="flex items-baseline gap-2.5">
          <h2 className="text-[20px] font-semibold text-jolu-text">模型池</h2>
          <span className="text-note text-jolu-muted">{serving.length} 个来源 · {modelTotal} 个模型</span>
        </div>
        <button type="button" onClick={onManage}
          className="flex h-10 shrink-0 items-center gap-1.5 rounded-lg bg-jolu-text px-4 text-meta font-semibold text-jolu-bg outline-none hover:opacity-90 focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <ArrowUpRightIcon />管理来源
        </button>
      </div>

      <div className="flex items-center justify-end gap-2.5">
        <button type="button" onClick={onManage} title="来源分组与排序沿用「管理来源」弹窗（待确认：V6 无单独画板）"
          className="flex h-9 items-center gap-2 rounded-lg border border-jolu-border px-3.5 text-meta text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <FolderOpen aria-hidden className="size-3.5 text-jolu-muted" />来源分组
        </button>
        <MetaRefreshButton />
      </div>

      <div className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
        {serving.length === 0 && (
          <p className="px-5 py-8 text-center text-meta text-jolu-muted">还没有启用中的来源。先在供应商里添加账号或 API Key，并打开「进入模型池」。</p>
        )}
        {serving.map((connection) => {
          const models = catalog.filter((model) => model.slug && model.slug === connection.slug)
          const open = expanded === connection.id
          const shown = showAll[connection.id] ? models : models.slice(0, 5)
          return (
            <div key={connection.id} className={cn(open && "bg-jolu-side/50")}>
              <div className="flex items-center gap-3 px-4 py-3.5">
                <button type="button" aria-expanded={open} onClick={() => setExpanded(open ? null : connection.id)}
                  className="flex min-w-0 flex-1 items-center gap-3 text-left outline-none focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-jolu-accent">
                  <ChevronRight aria-hidden className={cn("size-4 shrink-0 text-jolu-muted transition-transform", open && "rotate-90")} />
                  <span aria-hidden className="flex size-10 shrink-0 items-center justify-center rounded-[10px] border border-jolu-border bg-jolu-bg">
                    <PlatformLogo platform={connection.platformId} size={30} />
                  </span>
                  <span className="flex w-[200px] shrink-0 flex-col gap-0.5">
                    <span className="flex items-center gap-2">
                      <span className="truncate text-[15px] font-semibold text-jolu-text">{connection.name}</span>
                      {defaultConnection === connection.id && (
                        <span className="shrink-0 rounded bg-jolu-track px-1.5 py-0.5 text-note text-jolu-muted">默认</span>
                      )}
                    </span>
                    <span className="truncate text-note text-jolu-muted">{platformLabel(connection)} · {kindLabel(connection)}</span>
                  </span>
                  <code className="w-[120px] shrink-0 font-mono text-meta text-jolu-muted">{connection.slug ?? "—"}/</code>
                  <span className="shrink-0 text-meta tabular-nums text-jolu-muted">{models.length} 个模型</span>
                  <span className="flex-1" />
                </button>
                <button type="button" onClick={() => onOpenConnection(connection.id)}
                  className="shrink-0 text-meta font-medium text-jolu-on outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">
                  管理来源 ↗
                </button>
              </div>
              {open && (
                <div className="flex flex-col border-t border-jolu-table-divider bg-jolu-bg pb-3">
                  {models.length === 0 && (
                    <p className="px-5 py-4 text-meta text-jolu-muted">这个来源还没有模型清单：先在供应商连接里「拉取模型」。</p>
                  )}
                  {shown.map((model) => <ModelRow key={model.id} model={model} />)}
                  {models.length > shown.length && (
                    <button type="button" onClick={() => setShowAll((current) => ({ ...current, [connection.id]: true }))}
                      className="self-start px-5 pt-2 text-meta font-medium text-jolu-on outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">
                      查看另外 {models.length - shown.length} 个模型
                    </button>
                  )}
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}

function ModelRow({ model }: { model: CatalogModelDto }) {
  const toast = useToast()
  const id = `${model.slug}/${model.model}`
  const copy = () => {
    void navigator.clipboard.writeText(id).then(() => toast.success("已复制")).catch(() => toast.danger("复制失败", "请手动选中复制"))
  }
  return (
    <div className="flex items-center gap-3 border-t border-jolu-table-divider px-5 py-2 first:border-t-0">
      <Package aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />
      <code className="min-w-0 flex-1 truncate font-mono text-meta text-jolu-text" title={id}>{id}</code>
      <span className="shrink-0 text-note tabular-nums text-jolu-muted">{model.context_window ? `上下文 ${Math.round(model.context_window / 1000)}K` : "上下文未知"}</span>
      <button type="button" aria-label={`复制 ${id}`} onClick={copy}
        className="grid size-7 shrink-0 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent">
        <Copy aria-hidden className="size-3.5" />
      </button>
    </div>
  )
}

/** 模型元数据刷新（画板 gaGlT「刷新模型信息」按钮；models.dev 快照状态在按钮 title 里）。 */
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
    <button type="button" title={title} disabled={busy}
      onClick={async () => {
        setBusy(true)
        try {
          const [total, at] = await gatewayApi.modelsDevRefresh()
          setStatus([at, total])
        } catch { /* 状态仅作提示，失败保持原值 */ }
        setBusy(false)
      }}
      className="flex h-9 items-center gap-2 rounded-lg border border-jolu-border px-3.5 text-meta text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
      <RefreshIcon busy={busy} />刷新模型信息
    </button>
  )
}

function RefreshIcon({ busy }: { busy: boolean }) {
  return (
    <svg aria-hidden viewBox="0 0 14 14" className={cn("size-3.5 text-jolu-muted", busy && "animate-spin")} fill="none" stroke="currentColor" strokeWidth="1.5">
      <path d="M11.5 7a4.5 4.5 0 1 1-1.32-3.18M11.5 2.5v2.6h-2.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
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

function ArrowUpRightIcon() {
  return (
    <svg aria-hidden viewBox="0 0 14 14" className="size-3.5" fill="none" stroke="currentColor" strokeWidth="1.5">
      <path d="M4 10L10 4M10 4H5.5M10 4V8.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}
