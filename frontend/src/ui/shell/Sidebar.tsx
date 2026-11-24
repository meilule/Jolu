/**
 * Sidebar —— 设计稿 R1-C「分区 · 1 · 外壳 / 侧栏 Sidebar」。
 *
 * 规格取自画板：展开 216 / 收起 72、`jn-stage` 底、圆角 20、
 * 内边距 上18 右12 下20 左14、元素间距 4；**宽度过渡 180ms**。
 *
 * 结构是三个槽：
 * 1. **Logo 行**（高 44）：头像 30×30 + 艺术字标；收起时只留头像
 * 2. **导航插槽**：`NavItem` 列表
 * 3. **底部插槽**：灵动岛迷你状态、设置入口
 *
 * ## 为什么侧栏自己不做滚动
 *
 * 导航项是固定清单（9 项 × 42px + 间距），最矮的窗口也放得下。
 * 给它 `overflow-auto` 会多一条无用的滚动条，而且宽度过渡时滚动条会闪。
 *
 * ## 为什么收起用宽度过渡而不是替换两套 JSX
 *
 * 换整棵树会让 NavItem 重新挂载、Tooltip 状态丢失，且看不到过渡。
 * 这里只改外层宽度与内边距，NavItem 内部靠 `collapsed` 收文字——
 * 同一条 DOM 在 180ms 里连续变窄。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

export interface SidebarProps {
  collapsed: boolean
  /** Logo 行内容。收起时调用方应传只含头像的版本 */
  logo: ReactNode
  /** 侧栏收起 / 展开按钮：按设计稿 R1-C 放在 Logo 行最右侧（收起态在头像下方居中） */
  toggle?: ReactNode
  /** 导航区（`NavItem` 列表） */
  children: ReactNode
  /** 底部区：灵动岛迷你状态、设置入口 */
  footer?: ReactNode
  className?: string
}

export function Sidebar({ collapsed, logo, toggle, children, footer, className }: SidebarProps) {
  return (
    <aside
      aria-label="主导航"
      data-collapsed={collapsed}
      className={cn(
        "flex h-full shrink-0 flex-col gap-1 rounded-[20px] bg-jn-stage pb-5 pt-[18px] transition-[width,padding] duration-[180ms] ease-[var(--motion-ease-standard)]",
        collapsed ? "w-[72px] px-3" : "w-[216px] pr-3 pl-3.5",
        className,
      )}
    >
      <div className={cn("flex shrink-0 items-center", collapsed ? "flex-col justify-center gap-2" : "h-11 gap-2.5 pr-1 pl-1.5")}>
        {logo}
        {!collapsed && <span className="min-w-0 flex-1" />}
        {toggle}
      </div>
      {/* 导航区上边距 14：与 Logo 行拉开一档层级 */}
      <nav className="flex min-h-0 flex-1 flex-col gap-1 overflow-hidden pt-3.5">{children}</nav>
      {footer && <div className="flex shrink-0 flex-col gap-2">{footer}</div>}
    </aside>
  )
}
