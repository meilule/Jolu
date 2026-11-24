/**
 * ListRow —— 设计稿 R1-C「分区 · 5 · 容器与反馈 / 列表行 ListRow」。
 *
 * 卡片里的条目、动态、分组、导航都用它。规格取自画板：
 * 圆角 16、间距 12、内边距 上10 右12 下10 左10、图标底 34×34（圆角 12）。
 *
 * ## 三种状态
 *
 * | 状态 | 底色 | 描边 |
 * |---|---|---|
 * | default | 无 | 无 |
 * | hover | `jn-well` | 无 |
 * | selected | `jn-primary-wash` | 1px `jn-mint-2` |
 *
 * 选中态不用主色实心：列表行经常一次选中一行、旁边还有数据要读，
 * 实心绿会把整块区域压暗。淡底 + 细边既表明选中，又不抢内容。
 *
 * ## 为什么图标底要单独一块
 *
 * 34×34 的 well 方块给图标一个视觉锚点，让多行文字在左侧对齐。
 * 没有它时，图标尺寸不同会让每行的主文字起点都不一样。
 */
import type { HTMLAttributes, ReactNode } from "react"
import { cn } from "@/lib/utils"

const STATE = {
  default: "",
  hover: "bg-jn-well",
  selected: "border-jn-mint-2 bg-jn-primary-wash",
} as const

export type ListRowState = keyof typeof STATE

export interface ListRowProps extends Omit<HTMLAttributes<HTMLDivElement>, "title"> {
  /** 主文字。13px 600 */
  title: ReactNode
  /** 副文字。11px 次级色。不传则只显示主文字 */
  subtitle?: ReactNode
  /** 左侧图标区内容（平台标识、图标、序号） */
  icon?: ReactNode
  /** 右侧插槽：计数、状态胶囊、图标按钮 */
  trailing?: ReactNode
  /** 视觉状态。`hover` 只用于演示；真实悬停由 CSS 处理，见下方说明 */
  state?: ListRowState
  /** 可点行：加交互态与手型。不传则渲染成静态行 */
  onClick?: () => void
}

export function ListRow({
  title,
  subtitle,
  icon,
  trailing,
  state = "default",
  onClick,
  className,
  ...rest
}: ListRowProps) {
  const interactive = Boolean(onClick)
  return (
    <div
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? 0 : undefined}
      onClick={onClick}
      onKeyDown={
        interactive
          ? (event) => {
              // 键盘可达：Enter / Space 等价于点按。没有这个，纯 div 按钮对键盘用户不可用。
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault()
                onClick?.()
              }
            }
          : undefined
      }
      className={cn(
        "flex w-full items-center gap-3 rounded-jn-block border border-transparent py-[10px] pr-3 pl-[10px] text-left",
        STATE[state],
        // 真实悬停只对可点行生效：静态行悬停变色会让人以为能点。
        interactive && "jn-interactive cursor-pointer hover:bg-jn-well focus-visible:outline-none",
        className,
      )}
      {...rest}
    >
      {icon && (
        <span
          className={cn(
            "flex size-[34px] shrink-0 items-center justify-center rounded-[12px]",
            state === "selected" ? "bg-jn-primary-soft" : "bg-jn-well",
          )}
        >
          {icon}
        </span>
      )}
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-[13px] font-semibold text-jn-text">{title}</span>
        {subtitle && <span className="truncate text-[11px] text-jn-text-3">{subtitle}</span>}
      </span>
      {trailing && <span className="flex shrink-0 items-center gap-1.5">{trailing}</span>}
    </div>
  )
}
