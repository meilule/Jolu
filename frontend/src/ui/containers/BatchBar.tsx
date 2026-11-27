/**
 * BatchBar —— 设计稿 R1-C「分区 · 5 · 容器与反馈 / 批量条 BatchBar」。
 *
 * 规格取自画板：高 48、圆角 16、深色反差底（`jn-inverse` / `jn-on-inverse`），
 * 左「已选 N 项」计数、右动作胶囊与取消叉号。
 * Skills、供应商账号管理、一键签到共用这一个组件，不再各写一套。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"
import { Icon, type IconName } from "@/icons"
import { Reveal, TactileButton } from "@/motion"

export interface BatchBarProps {
  /** 已选数量或自定义计数文案 */
  summary: ReactNode
  /** 动作按钮（使用 `BatchAction`） */
  actions: ReactNode
  /** 附加提示（如批量执行结果摘要） */
  note?: ReactNode
  /** 取消 / 退出批量选择 */
  onClose: () => void
  disabled?: boolean
  className?: string
}

export function BatchBar({
  summary,
  actions,
  note,
  onClose,
  disabled = false,
  className,
}: BatchBarProps) {
  return (
    <Reveal
      y={4}
      className={cn(
        "flex min-h-12 w-full flex-wrap items-center gap-2.5 rounded-[16px] bg-jn-inverse py-2 pr-2.5 pl-[18px] text-jn-on-inverse shadow-jn-surface",
        className,
      )}
    >
      <span className="text-[13px] font-semibold whitespace-nowrap">{summary}</span>
      {note && <span className="text-xs opacity-80">{note}</span>}
      <span className="min-w-0 flex-1" />
      <div className="flex flex-wrap items-center gap-2">{actions}</div>
      <TactileButton
        variant="icon"
        type="button"
        disabled={disabled}
        onClick={onClose}
        aria-label="取消选择"
        className="jn-interactive grid size-8 shrink-0 place-items-center rounded-full bg-jn-inverse-chip text-jn-on-inverse hover:opacity-85 disabled:pointer-events-none disabled:opacity-45"
      >
        <Icon name="close" size="sm" />
      </TactileButton>
    </Reveal>
  )
}

/** 批量条内部的深色半透明胶囊动作按钮。 */
export function BatchAction({
  icon,
  children,
  danger = false,
  disabled = false,
  onClick,
}: {
  icon?: IconName
  children: ReactNode
  danger?: boolean
  disabled?: boolean
  onClick: () => void
}) {
  return (
    <TactileButton
      variant="button"
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "jn-interactive inline-flex h-8 shrink-0 items-center gap-1.5 rounded-full px-3 text-[12.5px] font-semibold whitespace-nowrap disabled:pointer-events-none disabled:opacity-45",
        danger
          ? "bg-jn-danger text-white hover:brightness-110"
          : "bg-jn-inverse-chip text-jn-on-inverse hover:opacity-85",
      )}
    >
      {icon && <Icon name={icon} size="sm" />}
      {children}
    </TactileButton>
  )
}
