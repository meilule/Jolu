import type { ButtonHTMLAttributes, ReactNode } from "react"
import type { LucideIcon } from "lucide-react"
import { cn } from "@/lib/utils"
import { Icon } from "@/icons"
import { Banner, Button, Select, Switch as JnSwitch, Tag, Tabs } from "@/ui"

/**
 * 设置页的公共壳 —— 设计稿 R1-12～17 的「设置组 / 设置行」。
 *
 * 所有分区都从这五个导出组合，不再各写一套类：
 * - `SettingsGroup`：圆角 24 的白卡，组标题 50 高（图标底 30 + 14/700）；
 * - `SettingsRow`：54/62 高，标题 13.5/600 + 说明 12，控件靠右，行间 1px 分隔；
 * - `Switch`：46×28 新开关（保留旧的 `on` 参数名）；
 * - `SelectButton`：R1-C 的下拉（旧调用传 `description`，这里映射成 `hint`）；
 * - `ActionButton` / `Segmented` / `PendingBadge` / `Notice`：按钮、分段、待接入、提示。
 */

/** 设置分组卡片：左右 20 内边距、圆角 24、组标题 50 高。 */
export function SettingsGroup({ icon: Glyph, title, desc, children, className }: {
  icon?: LucideIcon
  title: string
  desc?: string
  children?: ReactNode
  className?: string
}) {
  return (
    <section className={cn("flex w-full flex-col rounded-jn-card border border-jn-line bg-jn-surface px-5 shadow-jn-surface", className)}>
      <header className="flex h-[50px] items-center gap-2.5">
        {Glyph && (
          <span className="flex size-[30px] shrink-0 items-center justify-center rounded-[10px] bg-jn-primary-wash">
            <Glyph aria-hidden className="size-[15px] text-jn-primary" />
          </span>
        )}
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <h3 className="truncate text-[14px] font-bold text-jn-text">{title}</h3>
          {desc && <span className="truncate text-[12px] text-jn-text-3">{desc}</span>}
        </span>
      </header>
      {children}
    </section>
  )
}

/** 设置行：标题 13.5/600 + 说明 12；行之间是上边框，第一行不画。 */
export function SettingsRow({ title, desc, control, className, descClassName }: {
  title: ReactNode
  desc?: ReactNode
  control?: ReactNode
  className?: string
  descClassName?: string
}) {
  return (
    <div className={cn("flex min-h-[62px] items-center gap-3.5 border-t border-jn-line py-3 first:border-t-0", className)}>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-[13.5px] font-semibold text-jn-text">{title}</span>
        {desc && <span className={cn("text-[12px] leading-relaxed text-jn-text-3", descClassName)}>{desc}</span>}
      </div>
      {control && <div className="flex shrink-0 items-center gap-2">{control}</div>}
    </div>
  )
}

/** 开关（保留旧的 `on` / `liquid` 参数名，内部走 R1-C 的 46×28 开关）。 */
export function Switch({ on, onChange, label, disabled, liquid }: {
  on: boolean
  onChange: (on: boolean) => void
  label: string
  disabled?: boolean
  liquid?: boolean
}) {
  void liquid
  return <JnSwitch checked={on} onChange={onChange} label={label} disabled={disabled} />
}

/** 170 宽的下拉；旧调用传的 `description` 作为菜单行副标题。 */
export function SelectButton({ value, options, onChange, label, disabled, className }: {
  value: string
  options: { value: string; label: ReactNode; description?: string; disabled?: boolean }[]
  onChange: (value: string) => void
  label: string
  disabled?: boolean
  className?: string
  /** 旧参数：旧 SelectField 的自定义触发器内容，新组件不需要 */
  triggerLabel?: ReactNode
}) {
  return (
    <Select
      width="full"
      label={label}
      value={value}
      disabled={disabled}
      onChange={onChange}
      className={cn("w-[170px]", className)}
      options={options.map((option) => ({ value: option.value, label: option.label, hint: option.description, disabled: option.disabled }))}
    />
  )
}

type Tone = "secondary" | "soft" | "accent" | "danger"

const KIND = { secondary: "ghost", soft: "soft", accent: "primary", danger: "danger" } as const

/** 操作按钮；`busy` 时转圈并禁用。旧 34 高统一到新 Button 的 md（40）。 */
export function ActionButton({ tone = "secondary", icon, busy, children, className, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & {
  tone?: Tone
  icon?: ReactNode
  busy?: boolean
}) {
  return (
    <Button
      kind={KIND[tone]}
      size="md"
      loading={busy}
      className={className}
      disabled={props.disabled}
      onClick={props.onClick as (() => void) | undefined}
      title={props.title}
      aria-label={props["aria-label"]}
      aria-busy={busy || undefined}
      type={props.type ?? "button"}
    >
      {!busy && icon}
      {children}
    </Button>
  )
}

/** 分段选择（联网方式、列表密度、缩放）：R1-C 的白滑块分段。 */
export function Segmented<T extends string>({ items, value, onChange, label, disabled }: {
  items: { value: T; label: string }[]
  value: T
  onChange?: (value: T) => void
  label: string
  disabled?: boolean
}) {
  return (
    <div className={cn(disabled && "pointer-events-none opacity-60")} title={label}>
      <Tabs style="segment" value={value} onChange={(next) => onChange?.(next)}
        items={items.map((item) => ({ key: item.value, label: item.label }))} />
    </div>
  )
}

/** 画了但暂未接入的能力：入口旁标注「待接入」。 */
export function PendingBadge({ children = "待接入" }: { children?: ReactNode }) {
  return <Tag>{children}</Tag>
}

/** 说明提示。 */
export function Notice({ children, tone = "info", className }: { children: ReactNode; tone?: "info" | "danger"; className?: string }) {
  return <Banner tone={tone === "danger" ? "danger" : "info"} className={className}>{children}</Banner>
}

export { Icon }
