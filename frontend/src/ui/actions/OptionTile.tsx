/**
 * OptionTile —— 设计稿 R1-C「分区 · 2 · 按钮与操作 / 选项块 OptionTile」。
 *
 * 规格取自画板：高 46–52、圆角 18、图标 16 + 标题 13–14 semibold。
 * 选中态用主色实心 + 柔和品牌投影（`jn-glow`），未选态用 `jn-well`。
 * 用于新建任务「启动方式」、路由分组「选择策略」、压缩会话「生成方式」等横向单选块。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"
import { Icon, type IconName } from "@/icons"

export interface OptionTileProps {
  active: boolean
  label: ReactNode
  icon?: IconName
  description?: ReactNode
  disabled?: boolean
  onClick: () => void
  className?: string
}

export function OptionTile({
  active,
  label,
  icon,
  description,
  disabled = false,
  onClick,
  className,
}: OptionTileProps) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "jn-interactive flex min-h-[46px] min-w-0 flex-1 items-center justify-center gap-2 rounded-[18px] px-3.5 py-2 text-[13px] font-semibold",
        active
          ? "bg-jn-primary text-jn-on-primary shadow-[0_4px_10px_var(--jn-glow)]"
          : "bg-jn-well text-jn-text-2 hover:bg-jn-track/70 hover:text-jn-text",
        description && "flex-col items-start justify-center gap-0.5 py-2.5 text-left",
        className,
      )}
    >
      <span className="flex items-center gap-1.5">
        {icon && <Icon name={icon} size="md" className="shrink-0" />}
        <span className="truncate">{label}</span>
      </span>
      {description && (
        <span
          className={cn(
            "text-[11px] font-normal",
            active ? "text-jn-on-primary/85" : "text-jn-text-3",
          )}
        >
          {description}
        </span>
      )}
    </button>
  )
}
