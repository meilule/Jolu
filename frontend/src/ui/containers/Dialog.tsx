/**
 * Dialog —— 设计稿 R1-C「分区 · 5 · 容器与反馈 / 弹窗 Dialog」。
 *
 * 规格取自画板：`jn-surface`、圆角 28、内边距 上24 右24 下20 左24、间距 18；
 * 宽度三档 sm 400 / md 520 / lg 720。底部按钮右对齐、主按钮在最右。
 *
 * ## 为什么基于 Radix
 *
 * 焦点陷阱、Esc 关闭、遮罩点击、`aria-modal`、关闭后焦点归还——这五件事
 * 自己写一定会漏一件。项目里已有 `@radix-ui/react-dialog`，只替换外观。
 *
 * ## 下内边距比上小 4px
 *
 * 上 24 下 20：底部是按钮行，按钮自己带高度与留白，再加 24 会显得空。
 *
 * ## 危险操作没有单独的组件
 *
 * `ConfirmDialog` 只是「主按钮换成 danger」的 Dialog。单独一个组件会让两处
 * 布局代码分叉，所以用一个 `danger` 标记，样式复用。
 */
import type { ReactNode, Ref } from "react"
import * as DialogPrimitive from "@radix-ui/react-dialog"
import { cn } from "@/lib/utils"
import { DialogBackdrop, DialogPanel, Presence } from "@/motion"
import { IconButton } from "@/ui/actions/IconButton"
import { Button, type ButtonKind } from "@/ui/actions/Button"

const WIDTH = {
  sm: "max-w-[420px]",
  md: "max-w-[560px]",
  lg: "max-w-[680px]",
  xl: "max-w-[900px]",
} as const

export type DialogWidth = keyof typeof WIDTH

export interface DialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: ReactNode
  /** 一句话说明。放在标题下方 */
  description?: ReactNode
  width?: DialogWidth
  /** 内容区 */
  children?: ReactNode
  /** 内容区额外类名（如内部滚动） */
  bodyClassName?: string
  /** 弹窗本体根节点回调 ref（用于内部自定义浮层锚点） */
  panelRef?: Ref<HTMLDivElement>
  /** 滚动内容容器 ref */
  bodyRef?: Ref<HTMLDivElement>
  /** 底部按钮。主按钮放最后（最右） */
  actions?: ReactNode
  /** 底部左侧辅助信息（如计数、后台运行说明） */
  footerStart?: ReactNode
  /** 右上角关闭按钮。默认显示 */
  closable?: boolean
  /** 阻止遮罩点击与 Esc 关闭（如保存 / 删除进行中） */
  preventClose?: boolean
}

export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  width = "md",
  children,
  bodyClassName,
  panelRef,
  bodyRef,
  actions,
  footerStart,
  closable = true,
  preventClose = false,
}: DialogProps) {
  return (
    <DialogPrimitive.Root open={open} onOpenChange={(next) => { if (!next && preventClose) return; onOpenChange(next) }}>
      <Presence>
        {open && (
          <DialogPrimitive.Portal forceMount>
            <DialogPrimitive.Overlay forceMount asChild>
              <DialogBackdrop className="fixed inset-0 z-[150] bg-black/40 backdrop-blur-[2px]" />
            </DialogPrimitive.Overlay>
            <div className="pointer-events-none fixed inset-0 z-[150] grid place-items-center p-3 sm:p-6">
              <DialogPrimitive.Content
                forceMount
                asChild
                aria-describedby={description ? undefined : undefined}
                onEscapeKeyDown={(event) => { if (preventClose) event.preventDefault() }}
                onPointerDownOutside={(event) => { if (preventClose) event.preventDefault() }}
              >
                <DialogPanel
                  ref={panelRef}
                  className={cn(
                    "pointer-events-auto relative flex max-h-[calc(100dvh-48px)] w-full flex-col gap-[18px] overflow-hidden",
                    "rounded-[28px] border border-jn-line bg-jn-surface px-6 pt-6 pb-5 shadow-jn-surface focus:outline-none",
                    WIDTH[width],
                  )}
                >
                  <div className="flex w-full shrink-0 items-start gap-3">
                    <div className="flex min-w-0 flex-1 flex-col gap-1">
                      <DialogPrimitive.Title className="text-[16px] font-bold text-jn-text">
                        {title}
                      </DialogPrimitive.Title>
                      {description && (
                        <DialogPrimitive.Description className="text-xs leading-relaxed text-jn-text-3">
                          {description}
                        </DialogPrimitive.Description>
                      )}
                    </div>
                    {closable && (
                      <DialogPrimitive.Close asChild>
                        <IconButton icon="close" label="关闭" size="sm" disabled={preventClose} />
                      </DialogPrimitive.Close>
                    )}
                  </div>
                  {children && (
                    <div ref={bodyRef} className={cn("flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-0.5", bodyClassName)}>
                      {children}
                    </div>
                  )}
                  {(actions || footerStart) && (
                    <div className="flex w-full shrink-0 flex-wrap items-center gap-2.5 pt-0.5">
                      {footerStart && <div className="min-w-0 flex-1 text-xs text-jn-text-3">{footerStart}</div>}
                      {!footerStart && <span className="flex-1" />}
                      <div className="flex items-center gap-2.5">{actions}</div>
                    </div>
                  )}
                </DialogPanel>
              </DialogPrimitive.Content>
            </div>
          </DialogPrimitive.Portal>
        )}
      </Presence>
    </DialogPrimitive.Root>
  )
}

/**
 * 危险确认。与 Dialog 同一个外壳，只把主按钮换成 danger。
 *
 * 文案固定为「取消 / {confirmText}」：危险操作的两个按钮顺序不能变
 * （主操作在右、可取消在左），每个调用方都自己拼按钮就会有人反过来。
 */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmText = "确认",
  cancelText = "取消",
  onConfirm,
  /** 主按钮语气。删除类用 danger，普通确认用 primary */
  kind = "danger",
  loading = false,
  children,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: ReactNode
  description?: ReactNode
  confirmText?: string
  cancelText?: string
  onConfirm: () => void
  kind?: ButtonKind
  loading?: boolean
  children?: ReactNode
}) {
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      preventClose={loading}
      title={title}
      description={description}
      width="sm"
      actions={
        <>
          <Button kind="ghost" disabled={loading} onClick={() => onOpenChange(false)}>
            {cancelText}
          </Button>
          <Button kind={kind} onClick={onConfirm} loading={loading} disabled={loading}>
            {confirmText}
          </Button>
        </>
      }
    >
      {children}
    </Dialog>
  )
}
