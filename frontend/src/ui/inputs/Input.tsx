/**
 * Input —— 设计稿 R1-C「分区 · 3 · 选择与输入 / 输入框 Input」。
 *
 * ## 两档尺寸，圆角不同
 *
 * | size | 高度 | 圆角 | 用在 |
 * |---|---|---|---|
 * | md | 36 | 18（胶囊） | 工具条、搜索框 |
 * | lg | 40 | 14（圆角矩形） | 表单 |
 *
 * 两档圆角不一样是设计稿定的，不是笔误：胶囊看起来轻、适合工具条里的
 * 一次性输入；表单里的输入需要跟下拉、字段标签对齐，方一点更稳。
 *
 * ## 四种状态的底色与描边（取自画板）
 *
 * | 状态 | 底色 | 描边 |
 * |---|---|---|
 * | default | `jn-well` | 无 |
 * | focus | `jn-surface` | 1.5px `jn-primary` |
 * | error | `jn-well` | 1.5px `jn-danger` |
 * | disabled | `jn-well` | 无 + 45% 不透明 |
 *
 * 焦点时底色从 well 变成 surface：输入区「浮起来」变成白底，与卡片同色，
 * 视觉上强调「正在这里输入」。
 *
 * ## 为什么不用 outline 表示焦点
 *
 * 设计稿的焦点态是**描边 + 换底色**，不是外圈。外圈会与相邻元素重叠，
 * 在紧凑的工具条里尤其明显。
 */
import type { InputHTMLAttributes } from "react"
import { cn } from "@/lib/utils"
import { Icon, type IconName } from "@/icons"

const SIZE = {
  /** 工具条 / 搜索 */
  md: "h-9 rounded-full pl-[14px] pr-[14px] text-[13px]",
  /** 表单 */
  lg: "h-10 rounded-[14px] pl-[14px] pr-[14px] text-[13px]",
} as const

export type InputSize = keyof typeof SIZE

export interface InputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size"> {
  size?: InputSize
  /** 前置图标。搜索框用 `search`，其余多数不传 */
  icon?: IconName
  /** 校验失败。画红描边；说明文字由 FormField 负责 */
  invalid?: boolean
}

export function Input({ size = "lg", icon, invalid = false, className, disabled, ...rest }: InputProps) {
  return (
    <span
      className={cn(
        "flex w-full items-center gap-2 border-[1.5px] bg-jn-well",
        SIZE[size],
        // 描边色分两种情况，不能靠类名顺序决定胜负（focus-within 的优先级更高，
        // 简单叠加会让「错误 + 聚焦」变成绿色，等于把错误提示盖掉）：
        //   正常：未聚焦透明，聚焦变主色
        //   错误：未聚焦与聚焦都保持红色
        invalid
          ? "border-jn-danger focus-within:border-jn-danger"
          : "border-transparent focus-within:border-jn-primary",
        "focus-within:bg-jn-surface",
        disabled && "pointer-events-none opacity-45",
        className,
      )}
    >
      {icon && <Icon name={icon} size="sm" className="shrink-0 text-jn-text-3" />}
      <input
        disabled={disabled}
        aria-invalid={invalid || undefined}
        className="w-full bg-transparent text-jn-text outline-none placeholder:text-jn-text-3 disabled:cursor-not-allowed"
        {...rest}
      />
    </span>
  )
}
