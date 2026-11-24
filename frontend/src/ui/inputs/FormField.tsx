/**
 * FormField —— 设计稿 R1-C「分区 · 3 · 选择与输入 / 表单字段 FormField」。
 *
 * 「标签 + 控件 + 说明或错误」这三行的固定排列。弹窗与设置页共用，
 * 不再各写一套 INPUT 常量（现状有 5 个 INPUT 常量，字号与内边距各不相同）。
 *
 * ## 规格（取自画板）
 *
 * | 元素 | 字号 / 字重 | 颜色 |
 * |---|---|---|
 * | 标签 | 13 / 600 | `jn-text` |
 * | 必填星号 | 13 / 600 | `jn-danger` |
 * | 说明 | 12 / normal | `jn-text-3` |
 *
 * 标签行与控件之间 7px，标签与星号之间 4px。
 *
 * ## 说明与错误是同一个位置
 *
 * 校验失败时**替换**说明文字，不是再加一行——两行同时出现会把弹窗撑高，
 * 而且在「说明」和「错误」并列时用户不知道该信哪一句。错误用 danger 色，
 * 说明用次级色，位置不变，视线不会跳。
 *
 * ## 为什么控件是插槽而不是 `type` 参数
 *
 * `type: "text" | "select" | "switch"` 这种写法每加一种控件就要改组件。
 * 插槽把「长什么样」交给调用方选 `Input` / `SelectField` / `Switch`，
 * FormField 只管这三行的排版与间距。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

export interface FormFieldProps {
  label: ReactNode
  /** 必填：在标签后显示红色星号。只影响观感，校验仍由调用方做 */
  required?: boolean
  /** 常态说明。与 `error` 互斥——有 error 时显示 error */
  hint?: ReactNode
  /** 校验错误。给了它就用 danger 色显示，并替掉 `hint` */
  error?: ReactNode
  /** 控件。`Input` / `SelectField` / `Switch` 等 */
  children: ReactNode
  /**
   * 控件自身的 id。**给了它才会渲染成 `<label htmlFor>`**，点击标签即聚焦控件。
   *
   * 不自动生成 id：生成出来但没接到控件上的 `htmlFor` 是一个指向空元素的引用，
   * 点了没反应，而且读屏也会读到一个不存在的标注对象。宁可退化成普通文字，
   * 也不留一个看起来对、实际无效的关联。
   */
  htmlFor?: string
  className?: string
}

export function FormField({
  label,
  required = false,
  hint,
  error,
  children,
  htmlFor,
  className,
}: FormFieldProps) {
  const message = error ?? hint
  return (
    <div className={cn("flex flex-col gap-[7px]", className)}>
      <div className="flex items-center gap-1">
        {/* 有 htmlFor 才是真 label（可点击聚焦控件）；没有就退化成文字，
            行为与观感一致，只是不支持点标签聚焦。 */}
        {htmlFor ? (
          <label htmlFor={htmlFor} className="text-[13px] font-semibold text-jn-text">
            {label}
          </label>
        ) : (
          <span className="text-[13px] font-semibold text-jn-text">{label}</span>
        )}
        {required && (
          <span aria-hidden className="text-[13px] font-semibold text-jn-danger">
            *
          </span>
        )}
      </div>
      {children}
      {message && (
        <p className={cn("text-xs leading-relaxed", error ? "text-jn-danger" : "text-jn-text-3")}>
          {message}
        </p>
      )}
    </div>
  )
}
