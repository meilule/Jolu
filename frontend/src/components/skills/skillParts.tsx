import * as Menu from "@radix-ui/react-dropdown-menu"
import { useState, type ReactNode } from "react"
import { cn } from "@/lib/utils"
import { PENDING_SKILL_PROVIDERS, SKILL_PLATFORMS, inlineParts, markdownBlocks, splitFrontmatter } from "@/lib/skillsManager"
import type { SkillItemDto, SkillPlatformDto } from "@/lib/skillsApi"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { providerName } from "@/lib/providerCatalog"
import type { PlatformId } from "@/types"
import { Icon, type IconName } from "@/icons"
import { Dialog as JnDialog, Switch as JnSwitch, type DialogWidth } from "@/ui"

/* ───────────── 小部件 ───────────── */

/** 标签：作用范围 / 状态 */
export function Tag({ tone = "muted", children, title }: { tone?: "muted" | "on" | "danger"; children: ReactNode; title?: string }) {
  return (
    <span
      title={title}
      className={cn(
        "inline-flex h-6 max-w-full shrink-0 items-center truncate rounded-full px-2.5 text-[11px] font-medium",
        tone === "on" && "bg-jn-primary-soft text-jn-primary-ink",
        tone === "muted" && "bg-jn-well text-jn-text-2",
        tone === "danger" && "bg-jn-danger-soft text-jn-danger",
      )}
    >
      <span className="truncate">{children}</span>
    </span>
  )
}

/** Skills 列表开关：统一用 R1-C 的 46×28 开关。 */
export function SkillSwitch({ on, onChange, disabled, label }: { on: boolean; onChange?: (on: boolean) => void; disabled?: boolean; label: string }) {
  return <JnSwitch checked={on} onChange={(next) => onChange?.(next)} disabled={disabled} label={label} />
}

const ICONS: readonly IconName[] = ["palette", "scan", "folder", "spark", "gitBranch", "edit", "fileText", "blocks"]

/** 列表图标：异常项用固定图标；其余按名称稳定取一个，只作区分 */
export function SkillIcon({ item, size = 30 }: { item: Pick<SkillItemDto, "name" | "status">; size?: 30 | 48 }) {
  let name: IconName = ICONS[[...item.name].reduce((sum, ch) => sum + ch.charCodeAt(0), 0) % ICONS.length]
  if (item.status === "invalid") name = "danger"
  if (item.status === "conflict") name = "layers"
  return (
    <span className={cn("grid shrink-0 place-items-center", size === 30 ? "size-[30px] rounded-[9px] bg-jn-well" : "size-12 rounded-[14px] bg-jn-primary-wash")}>
      <Icon name={name} size={size === 30 ? "md" : "lg"} className={item.status === "invalid" ? "text-jn-danger" : "text-jn-primary-ink"} />
    </span>
  )
}

export function ProviderMark({ platform, size = 18 }: { platform: string; size?: number }) {
  if (platform === "all") return <Icon name="blocks" size="sm" className="text-jn-text-3" />
  return <PlatformLogo platform={platform as PlatformId} size={size} />
}

/* ───────────── 供应商下拉 ───────────── */

export function ProviderMenu({ value, platforms, onChange }: { value: string; platforms: SkillPlatformDto[]; onChange: (platform: string) => void }) {
  const [query, setQuery] = useState("")
  const total = platforms.reduce((sum, p) => sum + p.count, 0)
  const q = query.trim().toLowerCase()
  const show = (name: string) => !q || name.toLowerCase().includes(q)
  const row = (id: string, name: string, note: string, enabled: boolean, icon: ReactNode) => (
    <Menu.Item
      key={id}
      disabled={!enabled}
      onSelect={() => onChange(id)}
      className={cn(
        "flex h-[34px] cursor-pointer select-none items-center gap-2.5 rounded-[10px] px-2.5 text-xs outline-none data-[highlighted]:bg-jn-well data-[disabled]:opacity-40",
        value === id ? "bg-jn-primary-wash text-jn-primary-ink data-[highlighted]:bg-jn-primary-wash" : "text-jn-text",
      )}
    >
      <span aria-hidden className="grid size-[18px] shrink-0 place-items-center">{icon}</span>
      <span className="min-w-0 flex-1 truncate">{name}</span>
      <span className="shrink-0 text-[11px] text-jn-text-3">{note}</span>
      {value === id && <Icon name="check" size="xs" className="shrink-0 text-jn-primary-ink" />}
    </Menu.Item>
  )
  return (
    <Menu.Root modal={false} onOpenChange={(open) => !open && setQuery("")}>
      <Menu.Trigger
        aria-label="选择供应商"
        className="jn-interactive inline-flex h-9 w-[176px] shrink-0 items-center gap-2 rounded-full border border-jn-line bg-jn-surface px-3.5 text-xs font-medium text-jn-text shadow-jn-surface outline-none hover:bg-jn-well data-[state=open]:bg-jn-well"
      >
        <ProviderMark platform={value} />
        <span className="min-w-0 flex-1 truncate text-left">{value === "all" ? "全部供应商" : providerName(value)}</span>
        <Icon name="expand" size="xs" className="jn-chevron text-jn-text-3" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="start" sideOffset={6} collisionPadding={12} className="jn-popover z-[140] flex w-[336px] flex-col gap-2 rounded-[18px] border border-jn-line bg-jn-surface p-3 shadow-jn-surface" style={{ maxHeight: "min(520px, var(--radix-dropdown-menu-content-available-height))" }}>
          <label className="flex h-9 shrink-0 items-center gap-2 rounded-full bg-jn-well px-3">
            <Icon name="search" size="sm" className="text-jn-text-3" />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => event.stopPropagation()}
              placeholder="搜索供应商"
              aria-label="搜索供应商"
              className="min-w-0 flex-1 bg-transparent text-xs text-jn-text outline-none placeholder:text-jn-text-3"
            />
          </label>
          <div className="flex min-h-0 flex-col gap-0.5 overflow-y-auto">
            {show("全部供应商") && row("all", "全部供应商", `${total} 个安装项`, true, <Icon name="blocks" size="sm" className="text-jn-text-3" />)}
            {SKILL_PLATFORMS.filter((id) => show(providerName(id))).map((id) => {
              const p = platforms.find((x) => x.id === id)
              return row(id, providerName(id), p?.detected ? `${p.count} 个安装项` : "未检测到目录", true, <PlatformLogo platform={id} size={18} />)
            })}
            {PENDING_SKILL_PROVIDERS.filter((p) => show(p.name)).map((p) => row(p.id, p.name, "待接入", false, <PlatformLogo platform={p.id as PlatformId} size={18} />))}
            {show("自定义") && row("custom", "自定义", "只读扫描", true, (
              <span className="grid size-5 place-items-center rounded-md bg-jn-primary-wash text-jn-primary-ink">
                <Icon name="file" size="xs" />
              </span>
            ))}
          </div>
          <span className="text-[11px] text-jn-text-3">安装项按供应商与实际目录区分；未核对目录规则的工具标「待接入」。</span>
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

/* ───────────── 表单下拉 ───────────── */

export interface FieldOption {
  value: string
  label: string
  disabled?: boolean
  note?: string
}

/** 表单下拉（统一 R1-C 浅槽胶囊外观） */
export function FieldSelect({ value, options, onChange, label, disabled, placeholder }: {
  value: string
  options: FieldOption[]
  onChange: (value: string) => void
  label: string
  disabled?: boolean
  placeholder?: string
}) {
  const selected = options.find((o) => o.value === value)
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger
        aria-label={label}
        disabled={disabled}
        className="jn-interactive flex h-10 w-full min-w-0 items-center gap-2 rounded-[14px] border-[1.5px] border-transparent bg-jn-well px-3.5 text-left text-xs text-jn-text outline-none hover:bg-jn-track/60 focus-visible:border-jn-primary data-[state=open]:border-jn-primary data-[state=open]:bg-jn-surface disabled:cursor-not-allowed disabled:opacity-45"
      >
        <span className={cn("min-w-0 flex-1 truncate", !selected && "text-jn-text-3")}>{selected?.label ?? placeholder ?? "请选择"}</span>
        <Icon name="expand" size="xs" className="jn-chevron shrink-0 text-jn-text-3" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="start" sideOffset={6} collisionPadding={12} className="jn-popover z-[160] min-w-[var(--radix-dropdown-menu-trigger-width)] overflow-y-auto rounded-[16px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface" style={{ maxHeight: "min(300px, var(--radix-dropdown-menu-content-available-height))" }}>
          <Menu.RadioGroup value={value} onValueChange={onChange}>
            {options.map((o) => (
              <Menu.RadioItem key={o.value} value={o.value} disabled={o.disabled} className="relative flex h-8 cursor-pointer select-none items-center gap-2 rounded-[10px] pl-2.5 pr-7 text-xs text-jn-text outline-none data-[highlighted]:bg-jn-well data-[state=checked]:text-jn-primary-ink data-[disabled]:opacity-40">
                <span className="min-w-0 flex-1 truncate">{o.label}</span>
                {o.note && <span className="shrink-0 text-[11px] text-jn-text-3">{o.note}</span>}
                <Menu.ItemIndicator className="absolute right-2"><Icon name="check" size="xs" /></Menu.ItemIndicator>
              </Menu.RadioItem>
            ))}
          </Menu.RadioGroup>
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

export function Field({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn("flex min-w-0 flex-1 flex-col gap-1.5", className)}>
      <span className="text-xs font-medium text-jn-text-2">{label}</span>
      {children}
    </div>
  )
}

export const INPUT_CLASS = "jn-interactive h-10 w-full min-w-0 rounded-[14px] border-[1.5px] border-transparent bg-jn-well px-3.5 text-xs text-jn-text outline-none placeholder:text-jn-text-3 focus:border-jn-primary focus:bg-jn-surface disabled:opacity-55"

/* ───────────── 弹窗外壳（统一走 @/ui Dialog） ───────────── */

function mapDialogWidth(width: number): DialogWidth {
  if (width <= 440) return "sm"
  if (width <= 580) return "md"
  if (width <= 720) return "lg"
  return "xl"
}

export function DialogShell({ open = true, width, title, description, actions, onClose, busy, children }: {
  open?: boolean
  width: number
  title: ReactNode
  description?: ReactNode
  actions?: ReactNode
  onClose: () => void
  busy?: boolean
  children: ReactNode
  height?: number
}) {
  return (
    <JnDialog
      open={open}
      onOpenChange={(next) => { if (!next && !busy) onClose() }}
      preventClose={busy}
      width={mapDialogWidth(width)}
      title={title}
      description={description}
      actions={actions}
    >
      {children}
    </JnDialog>
  )
}

/* ───────────── Markdown 预览 ───────────── */

function Inline({ text }: { text: string }) {
  return (
    <>
      {inlineParts(text).map((part, i) =>
        part.kind === "code" ? <code key={i} className="rounded bg-jn-well px-1.5 py-0.5 font-mono text-[12px]">{part.text}</code>
          : part.kind === "strong" ? <strong key={i} className="font-semibold">{part.text}</strong>
            : <span key={i}>{part.text}</span>,
      )}
    </>
  )
}

/** SKILL.md 预览：忽略 frontmatter，覆盖标题、段落、列表、代码块 */
export function MarkdownPreview({ source, stripFront = true }: { source: string; stripFront?: boolean }) {
  const body = stripFront ? splitFrontmatter(source).body : source
  const blocks = markdownBlocks(body)
  if (!blocks.length) return <p className="text-xs text-jn-text-3">没有正文内容</p>
  let first = true
  return (
    <div className="flex flex-col gap-[18px] break-words">
      {blocks.map((block, i) => {
        if (block.kind === "heading") {
          const top = first && block.level === 1
          first = false
          return (
            <h3 key={i} className={cn("font-semibold text-jn-text", top ? "text-2xl" : block.level <= 2 ? "text-sm" : "text-[13px]")}>
              <Inline text={block.text} />
            </h3>
          )
        }
        first = false
        if (block.kind === "paragraph") return <p key={i} className="whitespace-pre-wrap text-xs leading-5 text-jn-text"><Inline text={block.text} /></p>
        if (block.kind === "list") {
          const List = block.ordered ? "ol" : "ul"
          return (
            <List key={i} className={cn("flex flex-col gap-1 pl-5 text-xs leading-5 text-jn-text", block.ordered ? "list-decimal" : "list-disc")}>
              {block.items.map((text, j) => <li key={j}><Inline text={text} /></li>)}
            </List>
          )
        }
        if (block.kind === "code") return <pre key={i} className="overflow-x-auto rounded-[12px] bg-jn-well p-3 font-mono text-[12px] leading-5 text-jn-text">{block.text}</pre>
        return <hr key={i} className="border-jn-line" />
      })}
    </div>
  )
}
