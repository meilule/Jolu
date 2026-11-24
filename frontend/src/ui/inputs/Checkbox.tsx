/**
 * Checkbox —— 设计稿 R1-C「分区 · 3 · 选择与输入 / 勾选 Checkbox」。
 *
 * 规格取自画板：20×20、圆角 7、描边 1.5。
 *
 * | 状态 | 外观 |
 * |---|---|
 * | on | `jn-primary` 实心 + 白色勾 |
 * | off | `jn-surface` 实心 + `jn-text-3` 描边 |
 * | mixed | `jn-primary` 实心 + 白色横线（批量选择的半选） |
 *
 * ## 为什么 mixed 不能省
 *
 * 批量选择列表有 10 项、勾了 3 项时，表头那个勾选框必须是「半选」而不是
 * 「未选」也不是「全选」——它表示「下面不是全都没选，也不是全都选了」。
 * 没有这一档，用户点一下表头会全选或全清，永远猜不到当前是什么状态。
 *
 * ## 为什么用 ref 手动设 indeterminate
 *
 * `indeterminate` 是 DOM 属性，不是 HTML 属性，React 没有对应的 prop
 * （`indeterminate` 写在 JSX 里会被忽略）。只能在 effect 里通过 ref 设。
 */
import { useEffect, useRef, type InputHTMLAttributes } from "react"
import { cn } from "@/lib/utils"
import { Icon } from "@/icons"

export interface CheckboxProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "type" | "size" | "onChange"> {
  checked: boolean
  /** 半选。为 true 时忽略 `checked` 的外观，显示横线 */
  indeterminate?: boolean
  onChange: (checked: boolean) => void
  /** 无障碍名。没有可见标签时必填 */
  label?: string
}

export function Checkbox({
  checked,
  indeterminate = false,
  onChange,
  label,
  className,
  disabled,
  ...rest
}: CheckboxProps) {
  const inputRef = useRef<HTMLInputElement>(null)

  // indeterminate 只能通过 DOM 属性设置，且每次渲染都要重新同步：
  // 用户点一下之后浏览器会把 indeterminate 清掉，React 不会替我们恢复。
  useEffect(() => {
    if (inputRef.current) inputRef.current.indeterminate = indeterminate
  }, [indeterminate])

  const active = checked || indeterminate

  return (
    <label
      className={cn(
        "relative inline-flex size-5 shrink-0 cursor-pointer items-center justify-center rounded-[7px] border-[1.5px] transition-colors",
        active ? "border-transparent bg-jn-primary" : "border-jn-text-3 bg-jn-surface",
        disabled && "pointer-events-none opacity-45",
        className,
      )}
    >
      <input
        ref={inputRef}
        type="checkbox"
        className="peer sr-only"
        checked={checked}
        disabled={disabled}
        aria-label={label}
        onChange={(event) => onChange(event.target.checked)}
        {...rest}
      />
      {indeterminate ? (
        // 半选：一条 8px 的横线，比缩小版的勾更不容易被误读成「已选」
        <span aria-hidden className="h-[2px] w-2 rounded-full bg-jn-on-primary" />
      ) : (
        checked && <Icon name="check" size="xs" className="text-jn-on-primary" />
      )}
      <span className="pointer-events-none absolute inset-0 rounded-[7px] peer-focus-visible:outline-[3px] peer-focus-visible:outline-offset-1 peer-focus-visible:outline-jn-focus" />
    </label>
  )
}
