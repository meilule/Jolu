import { ArrowUpRight, Boxes } from "lucide-react"
import { cn } from "@/lib/utils"
import type { CatalogModelDto } from "@/lib/gatewayApi"
import type { Connection } from "@/types"
import { ModelsMetaRefresh } from "./GatewayDialogs"

/**
 * 模型路由 · 模型池（只读，画板 V5-5 ③）：不在这里改，点一行跳到供应商对应连接；
 * 加入或移出、改前缀在供应商的账号卡与「编辑连接」里管理。
 */
export function ModelPoolPanel({ connections, catalog, defaultConnection, onOpenConnection, onManage }: {
  connections: Connection[]
  catalog: CatalogModelDto[]
  defaultConnection: string | null
  onOpenConnection: (connectionId: string) => void
  /** 过渡入口：来源分组、排序与批量加入（V5-5 尚未覆盖的旧功能） */
  onManage: () => void
}) {
  const rows = [...connections].sort((a, b) => Number(b.serve_enabled) - Number(a.serve_enabled))
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto px-7 py-5">
      <div className="flex items-center gap-3">
        <span className="flex size-10 shrink-0 items-center justify-center rounded-[10px] bg-jolu-side"><Boxes aria-hidden className="size-4 text-jolu-muted" /></span>
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="text-[17px] font-semibold text-jolu-text">模型池</span>
          <span className="text-meta text-jolu-muted">不在这里改，点一行跳到供应商对应连接</span>
        </span>
      </div>
      <div className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
        {rows.length === 0 && <p className="px-4 py-3 text-meta text-jolu-muted">还没有连接。先在供应商里添加账号或 API Key。</p>}
        {rows.map((connection, index) => {
          const count = catalog.filter((model) => model.slug && model.slug === connection.slug).length
          return (
            <button key={connection.id} type="button" onClick={() => onOpenConnection(connection.id)}
              className={cn("flex items-center gap-3 px-4 py-2.5 text-left outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-jolu-accent", index > 0 && "border-t border-jolu-table-divider")}>
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="truncate text-meta font-medium text-jolu-text">
                  {connection.name}
                  {defaultConnection === connection.id && <span className="ml-2 text-note font-normal text-jolu-on">默认来源</span>}
                </span>
                <span className="truncate text-note text-jolu-muted">
                  {connection.serve_enabled ? `${connection.slug ?? "—"}/ · ${count} 个模型` : "未加入模型池"}
                </span>
              </span>
              <ArrowUpRight aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />
            </button>
          )
        })}
      </div>
      <div className="flex items-center gap-2">
        <ModelsMetaRefresh />
        <span className="flex-1" />
        <button type="button" onClick={onManage} className="text-meta text-jolu-muted outline-none hover:text-jolu-text">来源分组与排序</button>
      </div>
    </div>
  )
}
