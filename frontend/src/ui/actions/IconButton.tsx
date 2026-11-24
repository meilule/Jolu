/**
 * IconButton —— 设计稿 R1-C「分区 · 2 · 按钮与操作 / 图标按钮 IconButton」。
 *
 * 只有图标的按钮。两档尺寸 **36 / 28**（比同档 Button 的 40 / 32 小一圈），
 * 因为纯图标没有文字撑开，用按钮的高度会显得笨重。
 *
 * ## 两种 kind
 *
 * - `surface`：页头与工具条用，well 底 —— 那里背景是白的，需要一点底才看得出可点。
 * - `ghost`：卡内用，无底色 —— 卡片本身已经是白底，再加一层 well 会显脏。
 *
 * ## 无障碍
 *
 * **必须传 `label`。** 没有可见文字时读屏只能靠它，漏了就是一个无法被辅助技术
 * 识别的按钮。这里把它做成必填参数，不是可选项。
 */
import type { ButtonHTMLAttributes } from "react"
import { cn } from "@/lib/utils"
import { Icon, type IconName } from "@/icons"

const KIND = {
  /** 页头 / 工具条：well 底 */
  surface: "bg-jn-well text-jn-text-2",
  /** 卡内：无底，悬停才浮现 */
  ghost: "text-jn-text-2",
} as const

const SIZE = {
  /** 36 见方 */
  md: "size-9",
  /** 28 见方 */
  sm: "size-7",
} as const

const ICON_SIZE = { md: "md", sm: "sm" } as const

export type IconButtonKind = keyof typeof KIND
export type IconButtonSize = keyof typeof SIZE

export interface IconButtonProps
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children" | "aria-label"> {
  icon: IconName
  /** 无障碍名，同时作为 title 提示。必填——纯图标按钮没有它读屏就认不出来。 */
  label: string
  kind?: IconButtonKind
  size?: IconButtonSize
}

export function IconButton({
  icon,
  label,
  kind = "ghost",
  size = "md",
  className,
  type = "button",
  ...rest
}: IconButtonProps) {
  return (
    <button
      type={type}
      title={label}
      aria-label={label}
      className={cn(
        "jn-interactive jn-hover-tint inline-flex shrink-0 items-center justify-center rounded-full",
        KIND[kind],
        SIZE[size],
        className,
      )}
      {...rest}
    >
      <Icon name={icon} size={ICON_SIZE[size]} />
    </button>
  )
}
