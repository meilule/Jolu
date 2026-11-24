import { forwardRef, type ButtonHTMLAttributes } from "react"
import { cn } from "@/lib/utils"
import { Button } from "@/ui"

/**
 * 旧版按钮兼容层 —— 统一渲染成 R1-C 的 `Button`。
 *
 * 调用点还保留 `variant="primary|soft|neutral|outline|danger-soft|danger"` 这套名字
 * （134 处），这里只做映射，不再维护第二套按钮外观。新代码请直接用 `@/ui` 的
 * `Button`（`kind` / `size` / `icon`）。
 *
 * 图标尺寸：旧调用常直接塞 lucide 图标（默认 24px），这里保留 `[&_svg]:size-4`
 * 的约束，避免换壳后按钮里的图标突然变大。
 */
export type JoluButtonVariant = "primary" | "soft" | "neutral" | "outline" | "danger-soft" | "danger"

const KIND = {
  primary: "primary",
  soft: "soft",
  neutral: "ghost",
  outline: "ghost",
  "danger-soft": "danger",
  danger: "danger",
} as const

export const JoluButton = forwardRef<HTMLButtonElement, ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: JoluButtonVariant
  size?: "md" | "sm"
}>(function JoluButton({ variant = "neutral", size = "md", className, type = "button", ...props }, ref) {
  return (
    <Button
      ref={ref}
      type={type}
      kind={KIND[variant]}
      size={size}
      className={cn("[&_svg]:size-4 [&_svg]:shrink-0", className)}
      {...props}
    />
  )
})
