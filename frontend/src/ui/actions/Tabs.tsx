/**
 * Tabs / TabBar / SegmentBar —— 设计稿 R1-C
 * 「分区 · 2 · 按钮与操作 / 页签项 TabItem」与「页签栏 TabBar / 分段栏 SegmentBar」。
 *
 * ## 三种 style
 *
 * | style | 高度 | 选中外观 | 容器 | 用途 |
 * |---|---|---|---|---|
 * | tab | 34 | 主色实心胶囊 | `jn-surface`、圆角 999、间距 2、内边距 4 | 切换页面内容（页头里） |
 * | segment | 28 | 白底胶囊 | `jn-well`、圆角 999、间距 2、内边距 3 | 卡内筛选 |
 * | segment-lg | 34 | 主色实心胶囊 | `jn-surface`、圆角 999、间距 2、内边距 3 | 工具条里的大分段 |
 *
 * 选中滑块统一走 `@/motion` 的 `<SlideIndicator layoutId>`（`SPRING.snappy` 弹簧），
 * 项宽不等时也能无缝跟随，减少动效由根部 `MotionProvider` 一处接管。
 */
import { useId, type ReactNode } from "react"
import { cn } from "@/lib/utils"
import { Icon, type IconName } from "@/icons"
import { SlideIndicator } from "@/motion"

const STYLE = {
  tab: {
    item: "h-[34px] px-3.5 text-xs",
    box: "bg-jn-surface p-1 shadow-jn-surface",
    indicator: "bg-jn-primary",
    activeText: "text-jn-on-primary",
    role: "tab",
  },
  segment: {
    item: "h-7 px-3 text-[11.5px]",
    box: "bg-jn-well p-[3px]",
    indicator: "bg-jn-surface shadow-jn-surface",
    activeText: "text-jn-text",
    role: "group",
  },
  "segment-lg": {
    item: "h-[34px] px-3.5 text-[13px]",
    box: "bg-jn-surface p-[3px] shadow-jn-surface",
    indicator: "bg-jn-primary",
    activeText: "text-jn-on-primary",
    role: "group",
  },
} as const

export type TabStyle = keyof typeof STYLE

/** 单个页签项。传入 `indicatorId` 时启用弹簧滑动指示器。 */
export function TabItem({
  label,
  icon,
  active,
  style = "tab",
  indicatorId,
  onClick,
  className,
}: {
  label: ReactNode
  icon?: IconName
  active: boolean
  style?: TabStyle
  indicatorId?: string
  onClick?: () => void
  className?: string
}) {
  const spec = STYLE[style]
  return (
    <button
      type="button"
      role={spec.role === "tab" ? "tab" : undefined}
      aria-selected={spec.role === "tab" ? active : undefined}
      aria-pressed={spec.role === "tab" ? undefined : active}
      onClick={onClick}
      className={cn(
        "jn-interactive relative isolate inline-flex shrink-0 items-center justify-center gap-1.5 rounded-full font-semibold whitespace-nowrap",
        spec.item,
        active ? spec.activeText : "text-jn-text-2 hover:text-jn-text",
        active && !indicatorId && spec.indicator,
        className,
      )}
    >
      {active && indicatorId && <SlideIndicator layoutId={indicatorId} className={cn("-z-10", spec.indicator)} />}
      {icon && <Icon name={icon} size="sm" />}
      {label}
    </button>
  )
}

/**
 * 页签 / 分段栏容器。每个实例自动分配唯一的 `layoutId`，切项自带弹簧滑块。
 */
export function Tabs<T extends string>({
  items,
  value,
  onChange,
  style = "tab",
  iconMode = "all",
  className,
}: {
  items: { key: T; label: ReactNode; icon?: IconName }[]
  value: T
  onChange: (key: T) => void
  style?: TabStyle
  /** all = 每项都显示图标；active = 仅当前选中项显示图标（对齐 R1-01/18–22 多页签画板，避免 6 页签拥挤） */
  iconMode?: "all" | "active"
  className?: string
}) {
  const spec = STYLE[style]
  const groupId = useId()
  return (
    <div
      role={spec.role === "tab" ? "tablist" : "group"}
      className={cn("inline-flex shrink-0 items-center gap-0.5 rounded-full", spec.box, className)}
    >
      {items.map((item) => {
        const active = item.key === value
        return (
          <TabItem
            key={item.key}
            label={item.label}
            icon={iconMode === "active" ? (active ? item.icon : undefined) : item.icon}
            active={active}
            style={style}
            indicatorId={`jn-tabs-${groupId}`}
            onClick={() => onChange(item.key)}
          />
        )
      })}
    </div>
  )
}
