import { useEffect, useRef, useState } from "react"
import * as RadixDialog from "@radix-ui/react-dialog"
import { TriangleAlert, X } from "lucide-react"
import { cn } from "@/lib/utils"
import type { CatalogModelDto, GatewayGroupDto, GroupKind } from "@/lib/gatewayApi"
import { gatewayApi } from "@/lib/gatewayApi"
import { isTauri } from "@/lib/api"
import { useToast } from "@/components/ui/Toast"
import { JoluButton } from "@/components/ui/JoluButton"
import type { Connection } from "@/types"

/**
 * 统一网关的弹窗与管理行（沿用模块 J 旧画板的弹窗与 E1–E10 状态）。
 * 来源进出模型池、改名与菜单 id、分组管理在「管理来源」弹窗（SourceManageDialog.tsx）。
 */

export function Modal({ title, icon, tone, children, footer, onClose, busy = false, width = 540, inline = false }: { title: string; icon?: React.ReactNode; tone?: "danger"; children: React.ReactNode; footer: React.ReactNode; onClose: () => void; busy?: boolean; width?: number; inline?: boolean }) {
  const trigger = useRef(document.activeElement)
  // 内嵌：模型路由页的右侧检查器直接放弹窗内容（R2 / V5-4），标题由检查器头部给出，不要底部按钮
  if (inline) return <section aria-label={title} aria-busy={busy} className="flex flex-col gap-4">{children}</section>
  return <RadixDialog.Root open onOpenChange={value => { if (!value && !busy) onClose() }}>
    <RadixDialog.Portal>
      <RadixDialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
      <RadixDialog.Content aria-describedby={undefined} aria-busy={busy} style={{ width }}
        onPointerDownOutside={event => event.preventDefault()}
        onEscapeKeyDown={event => { if (busy) event.preventDefault() }}
        onCloseAutoFocus={event => { event.preventDefault(); if (trigger.current instanceof HTMLElement && trigger.current.isConnected) trigger.current.focus() }}
        className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 overflow-y-auto rounded-2xl border border-jolu-border bg-jolu-cc-surface p-6 shadow-dialog outline-none">
        <div className="flex items-center gap-2">
          {icon}
          <RadixDialog.Title className={cn("min-w-0 flex-1 text-base font-semibold", tone === "danger" ? "text-jolu-danger" : "text-jolu-text")}>{title}</RadixDialog.Title>
          <RadixDialog.Close aria-label="关闭" disabled={busy} className="grid size-8 shrink-0 place-items-center rounded-lg text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40"><X aria-hidden className="size-4" /></RadixDialog.Close>
        </div>
        {children}
        <div className="flex flex-wrap justify-end gap-2">{footer}</div>
      </RadixDialog.Content>
    </RadixDialog.Portal>
  </RadixDialog.Root>
}

export function Button({ label, primary, danger, disabled, onClick }: { label: string; primary?: boolean; danger?: boolean; disabled?: boolean; onClick: () => void }) {
  return <JoluButton variant={primary ? (danger ? "danger" : "primary") : "outline"} disabled={disabled} onClick={onClick}>{label}</JoluButton>
}

export function Warning({ children, tone = "danger" }: { children: React.ReactNode; tone?: "danger" | "muted" }) {
  return <div className={cn("flex items-start gap-2 rounded-lg px-3 py-2.5 text-[11px]", tone === "danger" ? "bg-jolu-danger-soft text-jolu-danger" : "bg-jolu-side text-jolu-muted")}><TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" /><span>{children}</span></div>
}

export function MessageDialog({ title, tone, message, action, onClose }: { title: string; tone?: "danger"; message: string; action?: { label: string; onClick: () => void }; onClose: () => void }) {
  return (
    <Modal title={title} tone={tone} icon={<TriangleAlert className={cn("h-4 w-4", tone === "danger" ? "text-jolu-danger" : "text-jolu-muted")} />} onClose={onClose} footer={<><Button label="关闭" onClick={onClose} />{action && <Button label={action.label} primary onClick={action.onClick} />}</>}>
      <Warning>{message}</Warning>
    </Modal>
  )
}

export function RenameDialog({ connection, busy, onClose, onSave }: { connection: Connection; busy: boolean; onClose: () => void; onSave: (name: string, slug: string) => void }) {
  const [name, setName] = useState(connection.baseName)
  const [slug, setSlug] = useState(connection.slug ?? "")
  const reserved = ["route", "auto", "jolu", "group", "default"].includes(slug.trim())
  const invalid = !!slug && (!/^[a-z0-9-]+$/.test(slug) || slug.startsWith("-") || reserved)
  return (
    <Modal busy={busy} title="重命名来源" onClose={onClose} footer={<><Button label="取消" disabled={busy} onClick={onClose} /><Button label="保存" primary disabled={busy || invalid || !name.trim()} onClick={() => onSave(name.trim(), slug.trim())} /></>}>
      <div className="flex flex-col gap-1">
        <span className="text-[9.5px] text-jolu-muted">显示名称</span>
        <input value={name} onChange={(event) => setName(event.target.value)} className="jolu-control h-9 rounded-lg border border-jolu-border bg-jolu-side px-3 text-[12px] text-jolu-text outline-none focus:border-jolu-accent" />
        <span className="text-[8.5px] text-jolu-muted">只改显示名；已接入的工具不受影响。</span>
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-[9.5px] text-jolu-muted">菜单 id</span>
        <input value={slug} onChange={(event) => setSlug(event.target.value)} className={cn("h-9 rounded-lg border bg-jolu-side px-3 text-[12px] text-jolu-text outline-none", invalid ? "border-jolu-danger" : "border-jolu-border focus:border-jolu-accent")} />
        {invalid ? <span className="text-[8.5px] text-jolu-danger">{reserved ? `「${slug}」是保留字，建议改用 ${slug}-src` : "只能使用小写字母、数字和连字符，不能以连字符开头。"}</span> : <span className="text-[8.5px] text-jolu-muted">Agent 配置里存的是这个 id；改名不会改它，改 id 后旧 id 会返回 404 并说明新名字。</span>}
      </div>
    </Modal>
  )
}

/**
 * Agent 接线。
 *
 * Claude Code 的主模型（magpie writeTiers 语义）：接线写 settings.json 顶层 `model`、
 * 全部档位 env（opus/sonnet/haiku/fable/small-fast 默认跟随主模型），并移除
 * `ANTHROPIC_MODEL`——它压过 /model 的选择，写了切换只在本会话生效。
 * 断开时按快照还原用户原来的全部模型配置。不选模型也能接（只写地址和令牌）。
 */
/**
 * 模型元数据（models.dev：上下文窗口、显示名、推理档位）的现状与手动刷新（2.8，待确认：无画板）。
 * 平时 7 天自动刷新一次；从没拉到过时用内置快照。
 */
export function ModelsMetaRefresh() {
  const [status, setStatus] = useState<[number, number] | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!isTauri) return
    gatewayApi.modelsDevStatus().then(setStatus).catch(() => setStatus(null))
  }, [])
  if (!isTauri) return null
  const [fetched, count] = status ?? [0, 0]
  const text = !status ? "读取中…" : fetched === 0 ? `内置快照（${count} 个模型）` : `${new Date(fetched).toLocaleString("zh-CN", { hour12: false })} 拉取（${count} 个模型）`
  return (
    <span className="flex items-center gap-1.5 text-[8.5px] text-jolu-muted">
      <span className="min-w-0 truncate">模型元数据：{error ?? text}</span>
      <button type="button" disabled={busy} className="shrink-0 text-jolu-on outline-none hover:underline disabled:opacity-50"
        onClick={async () => {
          setBusy(true)
          setError(null)
          try {
            const [total, at] = await gatewayApi.modelsDevRefresh()
            setStatus([at, total])
          } catch (reason) {
            setError(`刷新失败：${String(reason)}`)
          } finally {
            setBusy(false)
          }
        }}>
        {busy ? "刷新中…" : "立即刷新"}
      </button>
    </span>
  )
}

export function CatalogDialog({ models, onClose }: { models: CatalogModelDto[]; onClose: () => void }) {
  const toast = useToast()
  return (
    <Modal title="模型池里的模型" onClose={onClose} footer={<Button label="关闭" onClick={onClose} />}>
      <p className="text-[9.5px] text-jolu-muted">复制「来源/模型」填入调用工具。订阅模型菜单不代表已完成真实调用验收。</p>
      <div className="flex max-h-[50vh] flex-col gap-1 overflow-y-auto">
        {models.map((model) => (
          <div key={model.id} className="flex items-center gap-2 rounded-lg bg-jolu-side px-2.5 py-1.5">
            <span className="truncate text-[11px] text-jolu-text">{model.id}</span>
            <button type="button" className="shrink-0 text-[9px] text-jolu-on" onClick={() => { void navigator.clipboard.writeText(model.id).then(() => toast.success("模型名已复制"), () => toast.danger("复制失败，请重试")) }}>复制模型名</button>
            <div className="flex-1" />
            <span className={cn("text-[8.5px]", model.status === "connected" ? "text-jolu-muted" : "text-jolu-danger")}>{model.status === "connected" ? model.source : "未连接"}</span>
          </div>
        ))}
      </div>
    </Modal>
  )
}

export function Line({ label, value, danger }: { label: string; value: string; danger?: boolean }) {
  return (
    <div className="flex h-8 items-center gap-2 border-b border-jolu-table-divider text-[9.5px] last:border-0">
      <span className="text-jolu-muted">{label}</span>
      <div className="flex-1" />
      <span className={cn("font-semibold", danger ? "text-jolu-danger" : "text-jolu-text")}>{value}</span>
    </div>
  )
}

/** 分组按置顶优先、再按保存的顺序排列（只影响排列，不改路由、接线和 Agent 菜单）。 */
export function orderGroups(groups: GatewayGroupDto[], pinned: string[], kind: GroupKind): GatewayGroupDto[] {
  return [...groups].sort((a, b) => {
    const aPin = pinned.includes(`${kind}:${a.id}`) ? 0 : 1
    const bPin = pinned.includes(`${kind}:${b.id}`) ? 0 : 1
    return aPin - bPin || a.sort - b.sort
  })
}
