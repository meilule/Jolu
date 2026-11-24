import * as Menu from "@radix-ui/react-dropdown-menu"
import * as Tooltip from "@radix-ui/react-tooltip"
import type { ReactNode } from "react"
import { Ellipsis, Info, ListOrdered, Pause, Pencil, Play, RotateCcw, Square, Trash2, X, type LucideIcon } from "lucide-react"
import { cn } from "@/lib/utils"
import { canDeleteTask, DELETE_BLOCKED_TIP, taskStatus, type Tone } from "@/lib/automation"
import type { TaskAction, TaskItemDto } from "@/lib/automationApi"

export const TONE: Record<Tone, string> = {
  on: "text-jn-primary-ink",
  text: "text-jn-text",
  danger: "text-jn-danger",
  muted: "text-jn-text-3",
}

/** 卡片标题行：14px 标题 + 右侧 10px 口径（画板 62 / 66） */
export function SectionTitle({ title, note, children }: { title: string; note?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex min-h-5 items-center justify-between gap-3">
      <h3 className="truncate text-sm font-semibold text-jn-text">{title}</h3>
      {children ?? (note && <span className="shrink-0 text-[11px] text-jn-text-3">{note}</span>)}
    </div>
  )
}

/** 键值行：标签在左，值在右 */
export function KeyValue({ label, value, title }: { label: string; value: ReactNode; title?: string }) {
  return (
    <div className="flex items-center gap-2.5 text-[12px]">
      <span className="shrink-0 text-jn-text-3">{label}</span>
      <span className="flex-1" />
      <span className="min-w-0 truncate text-right text-jn-text" title={title}>{value}</span>
    </div>
  )
}

/** 提示框：选中绿底（等待额度）/ 浅底说明 / 危险（画板 60–68） */
export function Notice({ icon: Icon = Info, title, children, tone = "side", className }: {
  icon?: LucideIcon
  title: ReactNode
  children?: ReactNode
  tone?: "selected" | "side" | "danger"
  className?: string
}) {
  return (
    <div
      className={cn(
        "flex flex-col gap-[7px] rounded-xl border p-3.5",
        tone === "selected" && "border-jn-mint-2 bg-jn-primary-wash",
        tone === "side" && "border-jn-line bg-jn-surface",
        tone === "danger" && "border-jn-danger-soft bg-jn-danger-soft",
        className,
      )}
    >
      <div className="flex items-center gap-2.5">
        <Icon aria-hidden className={cn("size-4 shrink-0", tone === "danger" ? "text-jn-danger" : tone === "selected" ? "text-jn-primary-ink" : "text-jn-text-3")} />
        <span className={cn("text-xs font-semibold", tone === "danger" ? "text-jn-danger" : tone === "selected" ? "text-jn-primary-ink" : "text-jn-text")}>{title}</span>
      </div>
      {children && <div className={cn("whitespace-pre-line text-[12px] leading-[1.6]", tone === "selected" ? "text-jn-primary-ink" : tone === "danger" ? "text-jn-danger" : "text-jn-text-3")}>{children}</div>}
    </div>
  )
}

/** 页签：任务 / 运行记录（画板 58 / 64） */
export function AutomationTabs({ value, onChange }: { value: "tasks" | "runs"; onChange: (v: "tasks" | "runs") => void }) {
  return (
    <div role="tablist" aria-label="自动化页签" className="inline-flex h-[38px] items-center gap-0.5 rounded-full bg-jn-well p-[3px]">
      {([["tasks", "任务"], ["runs", "运行记录"]] as const).map(([key, label]) => (
        <button
          key={key}
          type="button"
          role="tab"
          aria-selected={value === key}
          onClick={() => onChange(key)}
          className={cn(
            "jn-interactive inline-flex h-[30px] items-center justify-center rounded-full px-3.5 text-[13px] font-semibold",
            // 分段栏里的选中项是白底滑块（画板 segment 样式）
            value === key ? "bg-jn-surface text-jn-text" : "text-jn-text-2 hover:text-jn-text",
          )}
        >
          {label}
        </button>
      ))}
    </div>
  )
}

/** 额度进度条（画板 62 右栏） */
export function QuotaBar({ label, value, note, percent, blocking }: { label: string; value: string; note?: string; percent: number | null; blocking: boolean }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex justify-between gap-2 text-[12px]">
        <span className="text-jn-text">{label}</span>
        <span className="tnum text-jn-text-3">{value}</span>
      </div>
      <span className="h-1.5 rounded-[3px] bg-jn-track">
        <span className={cn("jn-meter-fill block h-full rounded-[3px]", blocking ? "bg-jn-warning" : "bg-jn-primary")} style={{ width: `${Math.min(100, Math.max(percent === null ? 0 : 2, percent ?? 0))}%` }} />
      </span>
      {note && <span className="text-[11px] text-jn-text-3">{note}</span>}
    </div>
  )
}

const MENU_CLASS = "jn-popover z-[140] min-w-[168px] rounded-[14px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface"
const ITEM_CLASS = "flex h-8 cursor-pointer select-none items-center gap-2 rounded-[10px] px-2.5 text-xs text-jn-text outline-none data-[highlighted]:bg-jn-well data-[disabled]:opacity-40 [&_svg]:size-3.5 [&_svg]:text-jn-text-3"
/** 危险项（删除任务）：文字与图标都用危险色 */
const DANGER_ITEM_CLASS = ITEM_CLASS.replace("text-jn-text", "text-jn-danger").replace("[&_svg]:text-jn-text-3", "[&_svg]:text-jn-danger")

/**
 * 任务更多操作（画板 68 第 08 格）：编辑任务 / 暂停等待 / 查看账号队列 / 取消任务，分隔线后为危险色的「删除任务」；
 * 已暂停时为「恢复任务」，运行中为「停止运行」。没有进行中的运行时可立即运行（已错过为「补跑」、失败为「重试」）。
 * 运行中「删除任务」禁用，悬浮或聚焦时显示原因（第 12 格）；为了能悬浮、聚焦，用 aria-disabled 而不是 disabled。
 */
export function TaskMenu({ item, now, onAction, onEdit, onQueue, onCancel, onDelete, trigger }: {
  item: TaskItemDto
  now: number
  onAction: (action: TaskAction) => void
  onEdit: () => void
  onQueue: () => void
  onCancel: () => void
  onDelete: () => void
  trigger?: ReactNode
}) {
  const running = item.active?.status === "running"
  const cancelled = item.state === "cancelled"
  const status = taskStatus(item, now)
  const canRun = !cancelled && item.state !== "paused" && (!item.active || item.active.status === "attention")
  const deletable = canDeleteTask(item.active?.status ?? item.state)
  const runLabel = item.active?.status === "attention" ? "重新排队" : item.missed_at ? "补跑" : status.label === "失败" ? "重试" : "立即运行"
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger asChild>
        {trigger ?? (
          <button type="button" aria-label={`${item.name} 的更多操作`} className="grid size-8 shrink-0 place-items-center rounded-lg bg-jn-surface text-jn-text-3 outline-none hover:bg-jn-stage focus-visible:outline-2 focus-visible:outline-jn-focus data-[state=open]:bg-jn-well">
            <Ellipsis aria-hidden className="size-4" />
          </button>
        )}
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="end" sideOffset={6} collisionPadding={12} className={MENU_CLASS}>
          <Menu.Item className={ITEM_CLASS} disabled={cancelled} onSelect={onEdit}><Pencil aria-hidden />编辑任务</Menu.Item>
          {running ? (
            <Menu.Item className={ITEM_CLASS} onSelect={() => onAction("stop")}><Square aria-hidden />停止运行</Menu.Item>
          ) : item.state === "paused" ? (
            <Menu.Item className={ITEM_CLASS} onSelect={() => onAction("resume")}><Play aria-hidden />恢复任务</Menu.Item>
          ) : (
            <Menu.Item className={ITEM_CLASS} disabled={item.state !== "active"} onSelect={() => onAction("pause")}><Pause aria-hidden />{item.active?.status === "waiting" ? "暂停等待" : "暂停任务"}</Menu.Item>
          )}
          {canRun && <Menu.Item className={ITEM_CLASS} onSelect={() => onAction(item.active ? "retry" : "run")}><RotateCcw aria-hidden />{runLabel}</Menu.Item>}
          <Menu.Item className={ITEM_CLASS} onSelect={onQueue}><ListOrdered aria-hidden />查看账号队列</Menu.Item>
          <Menu.Item className={ITEM_CLASS} disabled={cancelled} onSelect={onCancel}><X aria-hidden />取消任务</Menu.Item>
          <Menu.Separator className="mx-2 my-1 h-px bg-jn-line" />
          {deletable ? (
            <Menu.Item className={DANGER_ITEM_CLASS} onSelect={onDelete}><Trash2 aria-hidden />删除任务</Menu.Item>
          ) : (
            <Tooltip.Root>
              <Tooltip.Trigger asChild>
                <Menu.Item className={cn(DANGER_ITEM_CLASS, "opacity-40 data-[highlighted]:bg-transparent")} aria-disabled onSelect={(e) => e.preventDefault()}><Trash2 aria-hidden />删除任务</Menu.Item>
              </Tooltip.Trigger>
              <Tooltip.Portal>
                <Tooltip.Content side="bottom" align="start" sideOffset={6} collisionPadding={8} className="motion-popover z-[120] rounded-md bg-jn-text px-2.5 py-[7px] text-[12px] text-jn-stage">
                  {DELETE_BLOCKED_TIP}
                </Tooltip.Content>
              </Tooltip.Portal>
            </Tooltip.Root>
          )}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}
