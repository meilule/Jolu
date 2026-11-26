/**
 * Select —— 设计稿 R1-C「分区 · 3 · 选择与输入 / 下拉触发器 Select」与「菜单 Menu + 菜单行 MenuItem」。
 *
 * ## 两种 variant
 *
 * | variant | 底色 | 用在 |
 * |---|---|---|
 * | field | `jn-well` | 表单与设置（与 Input 的 lg 同款） |
 * | filter | 白底胶囊 | 工具条里的筛选 |
 *
 * 宽度只用四档：`sm 142 / md 170 / lg 280 / full`。不允许随手给像素值——
 * 工具条里三个下拉宽度不一致时，右侧的搜索框会参差不齐。
 *
 * ## 为什么不用原生 `<select>`
 *
 * 原生下拉的选项外观由系统绘制，无法做出设计稿的菜单行（副标题、选中勾、
 * 分组标题）。项目已有 `@radix-ui/react-dropdown-menu`，键盘与焦点交给它。
 *
 * ## 与 `components/ui/SelectField.tsx` 的关系
 *
 * 旧组件是给旧界面用的，宽度挡位与外观按 `Jolu.pen`。这个新组件按
 * `jolu-new.pen` 重写，两者在替换期间并存；旧页面替换完成后删掉旧的。
 */
import type { ReactNode } from "react"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { cn } from "@/lib/utils"
import { Icon } from "@/icons"
import { useGlidePill } from "@/motion"

const VARIANT = {
  /** 表单与设置：well 底、圆角 14（与 Input lg 对齐） */
  field: "h-10 rounded-[14px] bg-jn-well px-3.5",
  /** 工具条筛选：白底胶囊、带描边 */
  filter: "h-9 rounded-full border border-jn-line bg-jn-surface px-3.5",
} as const

const WIDTH = {
  sm: "w-[142px]",
  md: "w-[170px]",
  lg: "w-[280px]",
  full: "w-full",
} as const

export type SelectVariant = keyof typeof VARIANT
export type SelectWidth = keyof typeof WIDTH

export interface SelectOption {
  value: string
  label: ReactNode
  /** 菜单行副标题 */
  hint?: string
  disabled?: boolean
}

export interface SelectProps {
  value: string | null
  options: SelectOption[]
  onChange: (value: string) => void
  /** 无选中值时的占位文字 */
  placeholder?: string
  variant?: SelectVariant
  width?: SelectWidth
  disabled?: boolean
  /** 无障碍名。没有可见标签时必填 */
  label?: string
  className?: string
}

export function Select({
  value,
  options,
  onChange,
  placeholder = "请选择",
  variant = "field",
  width = "md",
  disabled = false,
  label,
  className,
}: SelectProps) {
  const selected = options.find((option) => option.value === value)
  const glide = useGlidePill()
  return (
    <Menu.Root>
      <Menu.Trigger
        disabled={disabled}
        aria-label={label}
        className={cn(
          "jn-interactive flex shrink-0 items-center justify-between gap-2 text-left",
          VARIANT[variant],
          WIDTH[width],
          disabled && "pointer-events-none opacity-45",
          className,
        )}
      >
        <span className={cn("min-w-0 truncate text-[13px]", selected ? "text-jn-text" : "text-jn-text-3")}>
          {selected?.label ?? placeholder}
        </span>
        <Icon name="expand" size="sm" className="jn-chevron shrink-0 text-jn-text-3" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content
          ref={glide}
          sideOffset={6}
          align="start"
          // 菜单面板与触发器同宽：宽度挡位已经约束好了，菜单跟着走才不会错位。
          className="jn-popover z-[140] max-h-[280px] w-[var(--radix-dropdown-menu-trigger-width)] overflow-y-auto overscroll-contain rounded-[14px] border border-jn-line bg-jn-surface p-1.5 text-jn-text shadow-jn-surface"
        >
          {options.map((option) => (
            <Menu.Item
              key={option.value}
              disabled={option.disabled}
              onSelect={() => onChange(option.value)}
              className="flex cursor-pointer items-center justify-between gap-2 rounded-[10px] px-2.5 py-2 text-[13px] outline-none data-[disabled]:pointer-events-none data-[disabled]:opacity-45 data-[highlighted]:bg-jn-well"
            >
              <span className="flex min-w-0 flex-col">
                <span className="truncate">{option.label}</span>
                {option.hint && <span className="truncate text-[11px] text-jn-text-3">{option.hint}</span>}
              </span>
              {option.value === value && <Icon name="check" size="sm" className="shrink-0 text-jn-primary-ink" />}
            </Menu.Item>
          ))}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}
