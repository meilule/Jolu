/**
 * SettingRow / SettingGroup —— 设计稿 R1-C
 * 「分区 · 3 · 选择与输入 / 设置行 SettingRow」与「设置分区 SettingsNav / 设置组 SettingGroup」。
 *
 * ## SettingRow 规格（取自画板）
 *
 * 高 64、圆角 18、底色 `jn-surface`、描边 1px `jn-line`、
 * 左右内边距 18、文字与控件间距 14。标题 14/600，说明 12/次级色（间距 2）。
 *
 * ## 两种形态
 *
 * | 形态 | 外观 | 用在 |
 * |---|---|---|
 * | 独立（默认） | 自带描边与圆角的行卡片 | 设置页里单独一项 |
 * | 组内（`bare`） | 无描边无圆角，与相邻行之间用 1px 细线分隔 | 放进同一张 Card |
 *
 * 设计稿的原话是「放进卡片后自动分隔」：卡片已经是一个 24 圆角的白框，
 * 里面每一行再各带一圈描边会变成「框中框」。所以 `SettingGroup` 负责画分隔线，
 * `SettingRow` 只负责内容行。
 *
 * ## 为什么整行不做成可点
 *
 * 设置行右侧是可交互控件（开关 / 下拉）。如果整行也响应点击，用户点开关时
 * 会同时触发两次状态变更（行点击 + 控件点击），而两者含义可能不同
 * （行点击常表示「进入详情」）。行本身保持静态，交互只发生在控件上。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

export interface SettingRowProps {
  title: ReactNode
  /** 说明。不传则只显示标题，行高随之变矮 */
  description?: ReactNode
  /** 右侧控件：Switch / 按钮 / 下拉 */
  control?: ReactNode
  /** 组内形态：去掉描边与圆角，由 SettingGroup 负责分隔 */
  bare?: boolean
  className?: string
}

export function SettingRow({ title, description, control, bare = false, className }: SettingRowProps) {
  return (
    <div
      className={cn(
        "flex w-full items-center gap-3.5 px-[18px]",
        // 高度固定 64 只在有说明时成立；没有说明时用上下内边距撑开，
        // 免得单行文字被塞进一个空荡的 64px 盒子里。
        description ? "min-h-16 py-3" : "min-h-14 py-2.5",
        bare
          ? "bg-transparent"
          : "rounded-[18px] border border-jn-line bg-jn-surface shadow-jn-surface",
        className,
      )}
    >
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-sm font-semibold text-jn-text">{title}</span>
        {description && (
          <span className="truncate text-xs leading-relaxed text-jn-text-3">{description}</span>
        )}
      </div>
      {control && <div className="flex shrink-0 items-center gap-2">{control}</div>}
    </div>
  )
}

/**
 * 一组设置行。行之间画 1px 分隔线，首尾不加线。
 *
 * 直接用 `divide-y` 会连第一行的上边也画，所以用 `[&>*+*]` 只给「后面跟着兄弟的行」
 * 加上边框——这样无论组里放几行，视觉上都是「n 行 + n-1 条线」。
 */
export function SettingGroup({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn("flex w-full flex-col [&>*+*]:border-t [&>*+*]:border-jn-line", className)}>
      {children}
    </div>
  )
}
