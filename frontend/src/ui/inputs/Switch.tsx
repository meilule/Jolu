/**
 * Switch —— 设计稿 R1-C「分区 · 3 · 选择与输入 / 开关 Switch」。
 *
 * 规格取自画板：46×28、圆角 14（= 高度一半）、滑块 22×22、内边距 3。
 * 滑块走 `@/motion` 的 `<SpringThumb>`（`SPRING.snappy` 弹簧），轨道颜色过渡
 * 走 `--motion-feedback`。
 */
import type { InputHTMLAttributes } from "react"
import { cn } from "@/lib/utils"
import { SpringThumb } from "@/motion"

export interface SwitchProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "type" | "size" | "onChange"> {
  checked: boolean
  onChange: (checked: boolean) => void
  /** 无障碍名。没有可见标签时必填（如表格行里的开关） */
  label?: string
}

export function Switch({ checked, onChange, label, className, disabled, ...rest }: SwitchProps) {
  return (
    <label
      className={cn(
        "relative inline-flex h-7 w-[46px] shrink-0 cursor-pointer items-center justify-start rounded-full border p-[2px] transition-colors duration-[var(--motion-feedback)] ease-[var(--motion-ease-standard)]",
        checked ? "border-transparent bg-jn-primary" : "border-jn-line bg-jn-track",
        disabled && "pointer-events-none opacity-45",
        className,
      )}
    >
      <input
        type="checkbox"
        role="switch"
        className="peer sr-only"
        checked={checked}
        disabled={disabled}
        aria-label={label}
        onChange={(event) => onChange(event.target.checked)}
        {...rest}
      />
      <SpringThumb
        checked={checked}
        className={cn(
          "pointer-events-none block size-[22px] rounded-full shadow-[0_1px_3px_rgba(15,42,28,0.22)]",
          checked ? "bg-white" : "bg-jn-surface",
        )}
      />
      {/* 键盘焦点圈画在轨道外沿。 */}
      <span className="pointer-events-none absolute inset-0 rounded-full peer-focus-visible:outline-[3px] peer-focus-visible:outline-offset-1 peer-focus-visible:outline-jn-focus" />
    </label>
  )
}
