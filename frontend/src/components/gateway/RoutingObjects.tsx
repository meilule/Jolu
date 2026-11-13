import { useState } from "react"
import { Activity, ArrowUpRight, Bot, Boxes, ChevronDown, ChevronRight, GitFork, LayoutDashboard, Layers, LifeBuoy, Network, Plus } from "lucide-react"
import { cn } from "@/lib/utils"

/** 模型路由页左栏可选的对象（画板 V5-3 / V5-4 的「对象清单」） */
export type RoutingObject =
  | { kind: "overview" }
  | { kind: "log" }
  | { kind: "groups"; slug?: string }
  | { kind: "pool" }
  | { kind: "access" }
  | { kind: "middleware" }

export function sameObject(a: RoutingObject, b: RoutingObject): boolean {
  return a.kind === b.kind && (a.kind !== "groups" || (b.kind === "groups" && (a.slug ?? "") === (b.slug ?? "")))
}

/**
 * 模型路由页左栏（宽 248）：网关卡、概览、路由流水、Agent 入口、路由分组、模型与访问、紧急切回直连。
 */
export function RoutingObjects({ selected, onSelect, running, address, shared, keyCount, busy, onToggleRunning, todayRequests, agentsInUse, onOpenAgents,
  groups, sources, models, middlewares, onAddGroup, canEscape, onEscape }: {
  selected: RoutingObject
  onSelect: (object: RoutingObject) => void
  running: boolean
  address: string
  shared: boolean
  keyCount: number | null
  busy: boolean
  onToggleRunning: (on: boolean) => void
  todayRequests: number | null
  agentsInUse: number
  onOpenAgents: () => void
  groups: { slug: string; problem: boolean }[]
  sources: number
  models: number
  middlewares: number | null
  onAddGroup: () => void
  canEscape: boolean
  onEscape: () => void
}) {
  const [folded, setFolded] = useState<Record<string, boolean>>({})
  const item = (object: RoutingObject, icon: React.ElementType, label: string, count?: string | null, dot?: boolean) => {
    const active = sameObject(selected, object)
    const Icon = icon
    return (
      <button key={`${object.kind}:${"slug" in object ? object.slug ?? "" : ""}`} type="button" onClick={() => onSelect(object)} aria-current={active ? "true" : undefined}
        className={cn("flex h-[34px] shrink-0 items-center gap-[9px] rounded-lg px-2 text-left outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent", active ? "bg-jolu-selected" : "hover:bg-jolu-stage")}>
        <Icon aria-hidden className={cn("size-[15px] shrink-0", active ? "text-jolu-on" : "text-jolu-muted")} />
        <span className={cn("min-w-0 flex-1 truncate text-meta", active ? "font-semibold text-jolu-on" : "text-jolu-text")}>{label}</span>
        {count && <span className="shrink-0 text-note text-jolu-muted">{count}</span>}
        {dot && <span aria-label="有问题" className="size-1.5 shrink-0 rounded-full bg-jolu-danger" />}
      </button>
    )
  }
  const head = (key: string, label: string, count?: number, action?: React.ReactNode) => (
    <div className="flex h-[30px] shrink-0 items-center gap-1.5 px-2 pt-2.5">
      <button type="button" aria-expanded={!folded[key]} onClick={() => setFolded((current) => ({ ...current, [key]: !current[key] }))}
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
        {folded[key] ? <ChevronRight aria-hidden className="size-3 text-jolu-muted" /> : <ChevronDown aria-hidden className="size-3 text-jolu-muted" />}
        <span className="text-note font-semibold text-jolu-muted">{label}</span>
        {count !== undefined && <span className="text-note text-jolu-muted">{count}</span>}
      </button>
      {action}
    </div>
  )
  return (
    <aside className="flex w-[248px] shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-jolu-border px-2.5 py-3.5" aria-label="模型路由对象">
      <div className="flex shrink-0 items-center gap-2 rounded-[10px] border border-jolu-border p-2.5">
        <span className={cn("size-[7px] shrink-0 rounded-full", running ? "bg-jolu-accent" : "bg-jolu-border")} />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-meta font-semibold text-jolu-text">{running ? "模型路由运行中" : "模型路由已停止"}</span>
          <span className="truncate text-note text-jolu-muted">{shared ? `局域网共享中${keyCount === null ? "" : ` · ${keyCount} 个 Key`}` : `${address} · 仅本机`}</span>
        </span>
        <button type="button" role="switch" aria-checked={running} aria-label={running ? "停止模型路由" : "开启模型路由"} disabled={busy} onClick={() => onToggleRunning(!running)}
          className={cn("flex h-[18px] w-[30px] shrink-0 items-center rounded-full p-0.5 outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:opacity-45", running ? "justify-end bg-jolu-accent" : "justify-start bg-jolu-track")}>
          <span className="size-3.5 rounded-full bg-white shadow-sm" />
        </button>
      </div>
      <span className="h-1.5 shrink-0" />
      {item({ kind: "overview" }, LayoutDashboard, "概览")}
      {item({ kind: "log" }, Activity, "路由流水", todayRequests === null ? null : todayRequests.toLocaleString("zh-CN"))}
      <button type="button" onClick={onOpenAgents}
        className="my-1 flex shrink-0 items-center gap-[9px] rounded-[9px] bg-jolu-side px-2.5 py-2 text-left outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
        <Bot aria-hidden className="size-[15px] shrink-0 text-jolu-muted" />
        <span className="flex min-w-0 flex-1 flex-col gap-px">
          <span className="text-meta font-medium text-jolu-text">{agentsInUse} 个 Agent 在用</span>
          <span className="text-note text-jolu-muted">在 Agent 页接入与设置</span>
        </span>
        <ArrowUpRight aria-hidden className="size-[13px] shrink-0 text-jolu-muted" />
      </button>
      {head("groups", "路由分组", groups.length, (
        <button type="button" aria-label="新建路由分组" onClick={onAddGroup} className="flex size-5 items-center justify-center rounded outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <Plus aria-hidden className="size-[13px] text-jolu-muted" />
        </button>
      ))}
      {!folded.groups && groups.map((group) => item({ kind: "groups", slug: group.slug }, GitFork, `group/${group.slug}`, null, group.problem))}
      {head("access", "模型与访问")}
      {!folded.access && (
        <>
          {item({ kind: "pool" }, Boxes, "模型池", `${sources} 来源 · ${models}`)}
          {item({ kind: "access" }, Network, "对外访问", shared ? "局域网" : "仅本机")}
          {item({ kind: "middleware" }, Layers, "中间件", middlewares === null ? null : `${middlewares} 启用`)}
        </>
      )}
      <div className="flex-1" />
      <button type="button" disabled={busy || !canEscape} onClick={onEscape} title="还原全部接入的工具配置并停止模型路由"
        className="flex h-[34px] shrink-0 items-center gap-2 rounded-lg px-2 text-meta text-jolu-danger outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40">
        <LifeBuoy aria-hidden className="size-[15px]" />
        紧急切回直连
      </button>
    </aside>
  )
}
