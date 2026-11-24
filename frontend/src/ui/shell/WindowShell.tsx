/**
 * WindowShell —— 设计稿 R1-C「分区 · 1 · 外壳 / 窗口外壳 AppShell」。
 *
 * 规格取自画板：1280×900、`jn-stage` 底、圆角 14；
 * 侧栏在左，右侧内容区分两行——页头（高 72）+ 页面区。
 * 内容区自身内边距 **上0 右24 下20 左6**，元素间距 14。
 *
 * ## 左侧 6 / 右侧 24 不是笔误
 *
 * 侧栏有 20 圆角，内容页有 24 圆角。左边只留 6，是因为侧栏的圆角与内容页的
 * 圆角之间只需要一条窄缝；右侧 24 是窗口边缘的呼吸位。等边距会让左侧看起来
 * 空了一大块——画板就是这样，照抄。
 *
 * ## 页面区本身无描边
 *
 * 画板组件库母版 `WfVEz` 里的 `1.5px jn-mint-2` 框是插槽占位提示；所有页面实稿
 * （R1-01/03…）替换插槽后都是透明容器，层级由各张 `jn-surface` 卡片自己承载。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

export interface WindowShellProps {
  /** 左侧栏 */
  sidebar: ReactNode
  /** 页头 */
  header: ReactNode
  /** 页面内容 */
  children: ReactNode
  className?: string
}

/** 画板尺寸。主窗口固定这个大小（可被窗口最大化覆盖）。 */
export const SHELL_WIDTH = 1280
export const SHELL_HEIGHT = 900

export function WindowShell({ sidebar, header, children, className }: WindowShellProps) {
  return (
    <div className={cn("flex h-full min-h-0 w-full overflow-hidden rounded-[14px] bg-jn-stage", className)}>
      {sidebar}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3.5 pt-0 pr-6 pb-5 pl-1.5">
        {header}
        <main className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          {children}
        </main>
      </div>
    </div>
  )
}
