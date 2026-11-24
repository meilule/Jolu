/**
 * 会话压缩 / 修订 / 交接预览共用弹窗壳（R1-43 / R1-44 / R1-45）。
 * 统一走 `@/ui` `Dialog`（圆角 28、Motion 进出场、顶底固定内容滚动）。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"
import { Banner, Dialog } from "@/ui"

export const handoffButton =
  "jn-interactive inline-flex h-9 shrink-0 items-center justify-center gap-1.5 rounded-full bg-jn-well px-3.5 text-xs font-semibold whitespace-nowrap text-jn-text hover:bg-jn-track disabled:pointer-events-none disabled:opacity-45"

export const handoffPrimary = cn(
  handoffButton,
  "bg-jn-primary text-jn-on-primary hover:bg-jn-primary-hover",
)

export const handoffInput =
  "w-full rounded-[14px] border-[1.5px] border-transparent bg-jn-well px-3.5 py-2 text-xs text-jn-text outline-none focus:border-jn-primary focus:bg-jn-surface disabled:opacity-45"

export function HandoffShell({
  title,
  children,
  onClose,
  chat = false,
}: {
  title: string
  children: ReactNode
  onClose: () => void
  chat?: boolean
}) {
  return (
    <Dialog
      open
      onOpenChange={(open) => { if (!open) onClose() }}
      width={chat ? "xl" : "lg"}
      title={title}
      bodyClassName={chat ? "min-h-[540px]" : undefined}
    >
      {children}
    </Dialog>
  )
}

export function HandoffNotice({ children, danger = false }: { children: ReactNode; danger?: boolean }) {
  return <Banner tone={danger ? "danger" : "info"}>{children}</Banner>
}
