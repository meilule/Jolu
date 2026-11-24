/**
 * NavItem —— 设计稿 R1-C「分区 · 1 · 外壳 / 导航项 NavItem」。
 *
 * 规格取自画板：
 * | 项 | 值 |
 * |---|---|
 * | 高度 | 42 |
 * | 圆角 | 14 |
 * | 图标与文字间距 | 12 |
 * | 左右内边距 | 展开 14 / 收起 12 |
 * | 文字 | 14 / 500（选中 600） |
 *
 * ## 三种状态
 *
 * | 状态 | 底色 | 文字 |
 * |---|---|---|
 * | default | 无 | `jn-text-2` |
 * | hover | `jn-surface` 半透明 | `jn-text` |
 * | active | `jn-surface` 实心 | `jn-text` + 600 + 右侧主色圆点 |
 *
 * 选中态用「白底 + 圆点」而不是实心主色：侧栏底色是 `jn-stage`（灰绿），
 * 白底自然浮起来；主色留给真正要点的按钮，导航项不抢。
 *
 * ## 收起态
 *
 * 只留图标（文字宽度收为 0 + 透明），悬停用 `Tooltip` 出名称。
 * 不用 `display:none` 藏文字：那会让宽度过渡时文字瞬间消失，
 * 而透明 + 溢出隐藏可以让它随侧栏一起被裁掉，过渡更顺。
 */
import { cn } from "@/lib/utils"
import { Icon, type IconName } from "@/icons"
import { SlideIndicator } from "@/motion"
import { Tooltip } from "@/ui/containers/Tooltip"

export interface NavItemProps {
  label: string
  icon: IconName
  active: boolean
  collapsed?: boolean
  onClick: () => void
}

export function NavItem({ label, icon, active, collapsed = false, onClick }: NavItemProps) {
  const button = (
    <button
      type="button"
      aria-current={active ? "page" : undefined}
      aria-label={collapsed ? label : undefined}
      onClick={onClick}
      className={cn(
        "jn-interactive relative isolate flex h-[42px] w-full shrink-0 items-center gap-3 overflow-hidden rounded-[14px] text-left whitespace-nowrap",
        collapsed ? "justify-center px-3" : "px-3.5",
        active ? "font-semibold text-jn-text" : "text-jn-text-2 hover:bg-jn-surface/60 hover:text-jn-text",
      )}
    >
      {active && <SlideIndicator layoutId="jn-sidebar-nav-active" className="-z-10 bg-jn-surface shadow-jn-surface" />}
      <Icon name={icon} size="lg" className={cn("shrink-0 transition-colors", active ? "text-jn-primary" : "text-jn-text-3")} />
      {/* 收起时宽度收为 0：文字随侧栏一起被裁掉，不做 display 切换 */}
      <span className={cn("min-w-0 flex-1 truncate text-sm transition-opacity", collapsed && "w-0 flex-none opacity-0")}>{label}</span>
      {/* 选中圆点：收起时挂在右下角，展开时在最右 */}
      {active && !collapsed && <span aria-hidden className="size-1.5 shrink-0 rounded-full bg-jn-primary" />}
      {active && collapsed && <span aria-hidden className="absolute right-2 bottom-2 size-1.5 rounded-full bg-jn-primary" />}
    </button>
  )

  // 收起态才需要提示：展开时文字就在旁边
  return collapsed ? (
    <Tooltip content={label} side="right">
      {button}
    </Tooltip>
  ) : (
    button
  )
}
