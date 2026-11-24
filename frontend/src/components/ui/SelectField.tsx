import { useState, type ReactNode } from "react"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { Check, ChevronDown, Search } from "lucide-react"
import { cn } from "@/lib/utils"

export interface SelectOption {
  value: string
  label: ReactNode
  description?: string
  icon?: ReactNode
  disabled?: boolean
}

/**
 * 全项目单选下拉的兼容层 —— 外观统一到 R1-C 的 `Select`。
 *
 * 旧调用还在用 `SelectField` 的这套参数（14 处，主要在灵动岛页与几个对话框），
 * 这里保留全部参数名，内部换成 jn 令牌与 Radix 下拉：
 * - `triggerLabel` / `readOnlySingle` / `menuWidth` / `searchable` 的行为保留；
 * - `glide`（共享滑块）不再实现——新菜单用行高亮，少一个绝对定位滑块；
 * - 旧 `.jolu-select-*` / `.jolu-menu` 类不再使用，旧样式可以删除。
 *
 * 新代码请直接用 `@/ui` 的 `Select`。
 */
export type SelectVariant = "default" | "filter" | "form" | "toolbar" | "picker"
export type SelectWidth = "xs" | "sm" | "md" | "lg" | "full"

const WIDTH: Record<SelectWidth, string> = {
  xs: "w-[120px]",
  sm: "w-[142px]",
  md: "w-[170px]",
  lg: "w-[280px]",
  full: "w-full",
}

const VARIANT: Record<SelectVariant, string> = {
  default: "h-9 rounded-[14px] bg-jn-well px-3 text-[13px]",
  filter: "h-9 rounded-full bg-transparent px-0 text-[13px]",
  form: "h-[38px] w-full rounded-[14px] bg-jn-well px-3.5 text-[13px]",
  toolbar: "h-[34px] rounded-full bg-jn-well px-3.5 text-[13px]",
  picker: "h-[38px] w-[176px] rounded-[14px] bg-jn-well px-3.5 text-[13px] font-medium",
}

export function SelectField({ value, options, onValueChange, label, disabled,
  className, triggerLabel, onOpenChange, readOnlySingle = false, placeholder = "请选择", glide = true,
  width = "sm", menuWidth, variant = "default", triggerIcon, searchable = false,
}: {
  value: string
  options: SelectOption[]
  onValueChange: (value: string) => void
  label: string
  disabled?: boolean
  className?: string
  triggerLabel?: ReactNode
  onOpenChange?: (open: boolean) => void
  readOnlySingle?: boolean
  placeholder?: string
  /** 旧参数：共享滑块动效，新菜单不再实现 */
  glide?: boolean
  width?: SelectWidth
  menuWidth?: number
  variant?: SelectVariant
  triggerIcon?: ReactNode
  searchable?: boolean
}) {
  void glide
  const [query, setQuery] = useState("")
  const selected = options.find((option) => option.value === value)
  if (readOnlySingle && value && selected && options.filter((option) => option.value && !option.disabled).length === 1) {
    return (
      <span aria-label={label} title={typeof selected.label === "string" ? selected.label : undefined}
        className={cn("inline-flex h-8 min-w-0 items-center text-[13px] text-jn-text", WIDTH[width], className)}>
        <span className="truncate">{selected.label}</span>
      </span>
    )
  }
  const shown = query.trim()
    ? options.filter((option) => `${typeof option.label === "string" ? option.label : ""} ${option.description ?? ""}`.toLowerCase().includes(query.trim().toLowerCase()))
    : options
  return (
    <Menu.Root modal={false} onOpenChange={(open) => { if (!open) setQuery(""); onOpenChange?.(open) }}>
      <Menu.Trigger disabled={disabled} type="button" aria-label={label}
        className={cn(
          "jn-interactive inline-flex min-w-0 shrink-0 items-center justify-between gap-2 text-left text-jn-text outline-none disabled:cursor-not-allowed disabled:opacity-45",
          VARIANT[variant],
          variant !== "filter" && variant !== "form" && variant !== "picker" && WIDTH[width],
          className,
        )}>
        {triggerIcon ? <span aria-hidden className="flex shrink-0 items-center">{triggerIcon}</span> : null}
        <span className="min-w-0 flex-1 truncate text-left">{triggerLabel ?? selected?.label ?? placeholder}</span>
        <ChevronDown aria-hidden className="jn-chevron size-3.5 shrink-0 text-jn-text-3" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content
          sideOffset={6} align="start" collisionPadding={8}
          style={{ minWidth: menuWidth ?? undefined }}
          className="jn-popover z-[140] max-h-[300px] w-[var(--radix-dropdown-menu-trigger-width)] overflow-y-auto overscroll-contain rounded-[14px] border border-jn-line bg-jn-surface p-1.5 text-jn-text shadow-jn-surface"
        >
          {searchable && (
            <div className="mb-1 flex items-center gap-1.5 rounded-[10px] bg-jn-well px-2.5 py-1.5">
              <Search aria-hidden className="size-3.5 shrink-0 text-jn-text-3" />
              <input autoFocus value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索" aria-label={`搜索${label}`}
                className="min-w-0 flex-1 bg-transparent text-[12px] text-jn-text outline-none placeholder:text-jn-text-3" />
            </div>
          )}
          {shown.length === 0 && <p className="px-2.5 py-2 text-[12px] text-jn-text-3">没有匹配项</p>}
          {shown.map((option) => (
            <Menu.Item key={option.value} disabled={option.disabled} onSelect={() => onValueChange(option.value)}
              className="flex cursor-pointer items-center justify-between gap-2 rounded-[10px] px-2.5 py-2 text-[13px] outline-none data-[disabled]:pointer-events-none data-[disabled]:opacity-45 data-[highlighted]:bg-jn-well">
              <span className="flex min-w-0 flex-col">
                <span className="truncate">{option.label}</span>
                {option.description && <span className="truncate text-[11px] text-jn-text-3">{option.description}</span>}
              </span>
              {option.value === value && <Check aria-hidden className="size-3.5 shrink-0 text-jn-primary-ink" />}
            </Menu.Item>
          ))}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}
