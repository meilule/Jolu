import { useEffect, useState, type ReactNode } from "react"
import type { CatalogModelDto, GatewayGroupDto, GroupKind } from "@/lib/gatewayApi"
import { gatewayApi } from "@/lib/gatewayApi"
import { isTauri } from "@/lib/api"
import { useToast } from "@/components/ui/Toast"
import { cn } from "@/lib/utils"
import { Icon } from "@/icons"
import { Banner, Button as JnButton, Dialog, FormField, Input, type DialogWidth } from "@/ui"
import type { Connection } from "@/types"

/**
 * 模型路由的通用弹窗（设计稿 R1-34/35/36 共用 R1-C 的 `Dialog`）。
 *
 * 旧的 `Modal` API 保留（title / children / footer / onClose / busy / width / inline），
 * 内部换成新 `Dialog`：圆角 28、上 24 下 20、按钮右对齐。宽度按数字映射到
 * sm 400 / md 520 / lg 720 —— 原调用点传 540 / 560 / 640，四舍五入到最近一档。
 */

function widthOf(width: number): DialogWidth {
  return width > 600 ? "lg" : width > 450 ? "md" : "sm"
}

export function Modal({ title, icon, tone, children, footer, onClose, busy = false, width = 540, inline = false }: {
  title: string
  icon?: ReactNode
  tone?: "danger"
  children: ReactNode
  footer: ReactNode
  onClose: () => void
  busy?: boolean
  width?: number
  inline?: boolean
}) {
  // 内嵌：模型路由页的右侧检查器直接放弹窗内容，标题由检查器头部给出，不要底部按钮
  if (inline) return <section aria-label={title} aria-busy={busy} className="flex flex-col gap-4">{children}</section>
  return (
    <Dialog
      open
      onOpenChange={(open) => { if (!open && !busy) onClose() }}
      width={widthOf(width)}
      title={<span className={cn("flex items-center gap-2", tone === "danger" && "text-jn-danger")}>{icon}{title}</span>}
      actions={footer}
    >
      {children}
    </Dialog>
  )
}

/** 旧调用的 `Button`（label / primary / danger）→ 新 Button。保留导出名，调用点不用改。 */
export function Button({ label, primary, danger, disabled, onClick }: { label: string; primary?: boolean; danger?: boolean; disabled?: boolean; onClick: () => void }) {
  return <JnButton kind={primary ? (danger ? "danger" : "primary") : "ghost"} disabled={disabled} onClick={onClick}>{label}</JnButton>
}

export function Warning({ children, tone = "danger" }: { children: ReactNode; tone?: "danger" | "muted" }) {
  return <Banner tone={tone === "danger" ? "danger" : "info"}>{children}</Banner>
}

export function MessageDialog({ title, tone, message, action, onClose }: { title: string; tone?: "danger"; message: string; action?: { label: string; onClick: () => void }; onClose: () => void }) {
  return (
    <Modal title={title} tone={tone} icon={<Icon name="warning" size="sm" className={tone === "danger" ? "text-jn-danger" : "text-jn-text-3"} />} onClose={onClose}
      footer={<><JnButton kind="ghost" onClick={onClose}>关闭</JnButton>{action && <JnButton kind="primary" onClick={action.onClick}>{action.label}</JnButton>}</>}>
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
    <Modal busy={busy} title="重命名来源" onClose={onClose}
      footer={<><JnButton kind="ghost" disabled={busy} onClick={onClose}>取消</JnButton><JnButton kind="primary" disabled={busy || invalid || !name.trim()} onClick={() => onSave(name.trim(), slug.trim())}>保存</JnButton></>}>
      <FormField label="显示名称" hint="只改显示名；已接入的工具不受影响。">
        <Input autoFocus value={name} onChange={(event) => setName(event.target.value)} />
      </FormField>
      <FormField label="菜单 id" htmlFor="gateway-rename-slug"
        error={invalid ? (reserved ? `「${slug}」是保留字，建议改用 ${slug}-src` : "只能使用小写字母、数字和连字符，不能以连字符开头。") : undefined}
        hint="Agent 配置里存的是这个 id；改 id 后旧 id 会返回 404">
        <Input id="gateway-rename-slug" value={slug} onChange={(event) => setSlug(event.target.value)} invalid={invalid} />
      </FormField>
    </Modal>
  )
}

/**
 * 模型元数据（models.dev：上下文窗口、显示名、推理档位）的现状与手动刷新。
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
    <span className="flex items-center gap-1.5 text-[11px] text-jn-text-3">
      <span className="min-w-0 truncate">模型元数据：{error ?? text}</span>
      <button type="button" disabled={busy} className="shrink-0 font-semibold text-jn-primary-ink disabled:opacity-50"
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
    <Modal title="模型池里的模型" onClose={onClose} footer={<JnButton kind="ghost" onClick={onClose}>关闭</JnButton>}>
      <p className="text-[12px] text-jn-text-3">复制「来源/模型」填入调用工具</p>
      <div className="flex max-h-[50vh] flex-col gap-0.5 overflow-y-auto">
        {models.map((model) => (
          <div key={model.id} className="flex items-center gap-2 rounded-[12px] bg-jn-well px-3 py-2">
            <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-jn-text">{model.id}</span>
            <button type="button" className="shrink-0 text-[11px] font-semibold text-jn-primary-ink"
              onClick={() => { void navigator.clipboard.writeText(model.id).then(() => toast.success("模型名已复制"), () => toast.danger("复制失败，请重试")) }}>
              复制模型名
            </button>
            <span className={cn("shrink-0 text-[11px]", model.status === "connected" ? "text-jn-text-3" : "text-jn-danger")}>{model.status === "connected" ? model.source : "未连接"}</span>
          </div>
        ))}
      </div>
    </Modal>
  )
}

export function Line({ label, value, danger }: { label: string; value: string; danger?: boolean }) {
  return (
    <div className="flex h-9 items-center gap-2 border-b border-jn-line text-[12px] last:border-0">
      <span className="text-jn-text-3">{label}</span>
      <span className="min-w-0 flex-1" />
      <span className={cn("font-semibold", danger ? "text-jn-danger" : "text-jn-text")}>{value}</span>
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
