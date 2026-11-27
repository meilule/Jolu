/**
 * Button —— 设计稿 R1-C「分区 · 2 · 按钮与操作 / 按钮 Button」。
 *
 * ## 规格来源（逐项取自画板）
 *
 * | 变体 | 高度 | 圆角 | 间距 | 左右内边距 | 字号 |
 * |---|---|---|---|---|---|
 * | md | 40 | 20（等于高度一半 → 胶囊） | 7 | 16 / 18 | 13 semibold |
 * | sm | 32 | 16（同上） | 6 | 12 / 14 | 12 semibold |
 *
 * 左右内边距是**不对称**的（右侧多 2px）：这是设计稿给的数值，不是笔误——
 * 右侧多出的 2px 抵消字形右侧的视觉留白，让按钮看起来居中。照抄，不「修正」。
 *
 * ## 四种 kind
 *
 * primary 用实心主色，其余三种都是浅底 + 同色文字：视觉重量依次递减，
 * 一个区域里最多一个 primary。
 *
 * ## 状态
 *
 * 悬停加深与轻胀 / 按下弹簧回弹 / 聚焦 3px 绿圈 / 禁用 45% / 加载中换转圈且不可点。
 * 颜色叠层在 `theme/states.css`，弹簧触感统一由 `@/motion` 的 `TactileButton` 接管。
 */
import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react"
import { cn } from "@/lib/utils"
import { Icon, type IconName } from "@/icons"
import { TactileButton } from "@/motion"

/** 四种视觉重量。变体表与设计稿的 kind 一一对应。 */
const KIND = {
  /** 实心主色。一个区域最多一个 */
  primary: "bg-jn-primary text-jn-on-primary",
  /** 浅绿底 + 深绿字。次要动作 */
  soft: "bg-jn-primary-soft text-jn-primary-ink",
  /** well 底 + 正文色。中性动作（取消、更多） */
  ghost: "bg-jn-well text-jn-text",
  /** 浅红底 + 红字。删除、断开这类不可逆动作 */
  danger: "bg-jn-danger-soft text-jn-danger",
} as const

/** 两档尺寸。数字来自画板，不提供第三档——多一档就多一份不一致的机会。 */
const SIZE = {
  md: "h-10 gap-[7px] pl-4 pr-[18px] text-[13px] font-semibold",
  sm: "h-8 gap-[6px] pl-3 pr-[14px] text-xs font-semibold",
} as const

/** 加载中的转圈图标尺寸随按钮高度走，不跟随文字大小。 */
const SPINNER = { md: "md", sm: "sm" } as const

export type ButtonKind = keyof typeof KIND
export type ButtonSize = keyof typeof SIZE

export interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  /** 视觉重量，默认 primary */
  kind?: ButtonKind
  /** 尺寸，默认 md */
  size?: ButtonSize
  /** 前置图标。纯文字按钮不传 */
  icon?: IconName
  /** 加载中：图标位置换成转圈，按钮不可点。文案保持不变——不要改成「加载中…」，
   *  那会让按钮宽度跳一下。 */
  loading?: boolean
  children?: ReactNode
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button({
  kind = "primary",
  size = "md",
  icon,
  loading = false,
  disabled,
  className,
  children,
  type = "button",
  ...rest
}, ref) {
  return (
    <TactileButton
      ref={ref}
      variant="button"
      type={type}
      disabled={disabled || loading}
      className={cn(
        "jn-interactive jn-hover-tint inline-flex shrink-0 items-center justify-center rounded-full whitespace-nowrap",
        KIND[kind],
        SIZE[size],
        className,
      )}
      {...rest}
    >
      {loading ? (
        <Icon name="loading" size={SPINNER[size]} className="animate-spin" />
      ) : (
        icon && <Icon name={icon} size={size === "md" ? "md" : "sm"} />
      )}
      {children}
    </TactileButton>
  )
})
