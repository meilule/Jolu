/**
 * Tooltip —— 设计稿 R1-C「分区 · 5 · 容器与反馈 / 提示 Tooltip」。
 *
 * 规格取自画板：高 28、圆角 9、左右内边距 10、深色小气泡。
 *
 * 深色用 `jn-inverse` / `jn-on-inverse` 这一对令牌，而不是写死深色：
 * 深色主题下 `jn-inverse` 会变成浅色，气泡在深底上仍然可读。
 *
 * ## 为什么基于 Radix
 *
 * 延迟显示、跟随指针、贴边翻转、`aria-describedby` 关联都由 Radix 处理。
 * 自己用 `title` 属性做不到样式；自己写浮层会漏掉翻转与无障碍。
 *
 * ## 什么时候不该用它
 *
 * 提示只用于**补充说明**（图标按钮的含义、被截断文字的全文）。
 * 关键信息不能只放在 tooltip 里——触摸设备没有悬停，键盘用户也不一定发现得了。
 */
import type { ReactNode } from "react"
import * as TooltipPrimitive from "@radix-ui/react-tooltip"
import { cn } from "@/lib/utils"

/** 全局 Provider。放在主窗口根部一次即可（`app/providers.tsx` 会挂）。 */
export const TooltipProvider = TooltipPrimitive.Provider

export function Tooltip({
  content,
  children,
  side = "top",
  className,
}: {
  content: ReactNode
  children: ReactNode
  side?: "top" | "right" | "bottom" | "left"
  className?: string
}) {
  return (
    <TooltipPrimitive.Root>
      <TooltipPrimitive.Trigger asChild>{children}</TooltipPrimitive.Trigger>
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Content
          side={side}
          sideOffset={6}
          className={cn(
            "jn-popover z-[130] inline-flex h-7 max-w-[280px] items-center rounded-[9px] bg-jn-inverse px-2.5 text-[11px] leading-tight text-jn-on-inverse shadow-jn-surface",
            className,
          )}
        >
          {content}
        </TooltipPrimitive.Content>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  )
}
