import * as Menu from "@radix-ui/react-dropdown-menu"
import { Check, ChevronDown } from "lucide-react"
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

export interface SelectOption {
  value: string
  label: string
  description?: string
  icon?: ReactNode
  disabled?: boolean
}

/** 原型统一选择器：菜单、选中、禁用与焦点均由应用主题绘制。 */
export function SelectField({ value, options, onValueChange, label, disabled,
  className, triggerLabel, onOpenChange, readOnlySingle = false, placeholder = "请选择",
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
  /** value 不落在任何选项上时触发器显示的文案 */
  placeholder?: string
}) {
  const selected = options.find((option) => option.value === value)
  if (readOnlySingle && value && selected && options.filter((option) => option.value && !option.disabled).length === 1) {
    return <span aria-label={label} title={selected.label} className={cn("inline-flex h-8 min-w-0 items-center text-xs text-text-primary", className)}><span className="truncate">{selected.label}</span></span>
  }
  return (
    <Menu.Root modal={false} onOpenChange={onOpenChange}>
      <Menu.Trigger disabled={disabled}
        aria-label={label}
        type="button"
        className={cn("jolu-control jolu-select-trigger inline-flex h-8 min-w-0 shrink-0 items-center justify-between gap-2 rounded-[8px] border bg-surface px-2.5 text-left text-xs text-text-primary outline-none transition-colors hover:bg-hover focus-visible:border-accent-blue focus-visible:ring-2 focus-visible:ring-accent-blue-soft data-[state=open]:border-border-strong disabled:cursor-not-allowed disabled:opacity-45", className)}
      >
        <span className="min-w-0 truncate">{triggerLabel ?? selected?.label ?? placeholder}</span>
        <ChevronDown aria-hidden className="motion-select-chevron size-3 shrink-0 text-text-secondary" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content loop align="start" sideOffset={6} collisionPadding={12}
          className="jolu-select-menu motion-popover z-[140] min-w-[160px] w-[var(--radix-dropdown-menu-trigger-width)] max-w-[min(360px,calc(100vw-24px))] overflow-y-auto overscroll-contain rounded-[8px] border bg-surface text-text-primary shadow-popover"
          style={{ maxHeight: "min(248px, var(--radix-dropdown-menu-content-available-height))" }}
        >
          <Menu.RadioGroup value={value} onValueChange={onValueChange} className="p-1">
            {options.map((option) => (
              <Menu.RadioItem key={option.value} value={option.value} disabled={option.disabled}
                textValue={option.label}
                className="relative flex min-h-7 cursor-default select-none items-center gap-2 rounded-[6px] py-1 pl-3 pr-7 text-xs outline-none data-[highlighted]:bg-surface-3 data-[state=checked]:bg-surface-3 data-[disabled]:opacity-40"
              >
                {option.icon && <span aria-hidden className="grid size-4 shrink-0 place-items-center">{option.icon}</span>}
                <span className="min-w-0">
                  <span className="block break-words">{option.label}</span>
                  {option.description && <span className="mt-px block max-w-[290px] whitespace-normal text-[12px] text-text-secondary" title={option.description}>{option.description}</span>}
                </span>
                <Menu.ItemIndicator className="absolute right-2"><Check aria-hidden className="size-3" /></Menu.ItemIndicator>
              </Menu.RadioItem>
            ))}
          </Menu.RadioGroup>
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}
