/**
 * Table / TableRow —— 设计稿 R1-C「分区 · 5 · 容器与反馈 / 表格 Table」。
 *
 * 规格取自画板：表头高 30（文字 12 / 500 / 次级色、左右内边距 12）；
 * 数据行高 52、圆角 14、左右内边距 12；斑马纹用 `jn-well`。
 *
 * ## 为什么行是圆角而不是用分隔线
 *
 * 表格放在卡片里，卡片自己已经有 24 圆角与外框。行再用横线分隔会变成
 * 「框里一层层横线」；圆角斑马纹更轻，且奇数行有底色时圆角让行看起来是
 * 独立的块，扫读时不会串行。
 *
 * ## 单元格内容的限制
 *
 * 设计稿写明「单元格内容只能是文字、数据值、进度条、状态胶囊、平台标识」。
 * 不允许放按钮——行内操作统一放到右侧的图标按钮区或点行进详情。
 * 这条靠约定而非类型约束：`children` 是 `ReactNode`，写之前要想一下。
 */
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

export interface TableColumn {
  key: string
  label: ReactNode
  /** 列宽。不传则各列均分。数字按像素，字符串按 CSS 宽度（如 "30%"） */
  width?: number | string
  /** 内容对齐。金额、Token 这类数字列用 right */
  align?: "left" | "right" | "center"
}

const ALIGN = { left: "text-left", right: "text-right", center: "text-center" } as const

/** 列宽的两种写法统一成 style，避免拼进 className 后无法被 Tailwind 识别。 */
function widthStyle(width?: number | string): { width?: string } {
  if (width == null) return {}
  return { width: typeof width === "number" ? `${width}px` : width }
}

export function Table({
  columns,
  children,
  className,
}: {
  columns: TableColumn[]
  /** 若干 `TableRow` */
  children: ReactNode
  className?: string
}) {
  return (
    <div className={cn("flex w-full flex-col", className)}>
      <div className="flex h-[30px] w-full items-center px-3">
        {columns.map((column) => (
          <span
            key={column.key}
            style={widthStyle(column.width)}
            className={cn(
              "shrink-0 text-xs font-medium text-jn-text-3",
              ALIGN[column.align ?? "left"],
              column.width == null && "min-w-0 flex-1",
            )}
          >
            {column.label}
          </span>
        ))}
      </div>
      <div className="flex w-full flex-col">{children}</div>
    </div>
  )
}

/**
 * 一行。
 *
 * `stripe` 由调用方按下标给（通常 `index % 2 === 1`），组件不自己数——
 * 分页后第一行的斑马纹应该跟着页内位置走，而不是全局序号，
 * 这件事只有调用方知道。
 */
export function TableRow({
  columns,
  stripe = false,
  onClick,
  className,
  children,
}: {
  columns: TableColumn[]
  stripe?: boolean
  onClick?: () => void
  className?: string
  children: ReactNode
}) {
  const cells = Array.isArray(children) ? children : [children]
  return (
    <div
      role={onClick ? "button" : "row"}
      tabIndex={onClick ? 0 : undefined}
      onClick={onClick}
      onKeyDown={
        onClick
          ? (event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault()
                onClick()
              }
            }
          : undefined
      }
      className={cn(
        "flex h-[52px] w-full items-center rounded-[14px] px-3",
        stripe && "bg-jn-well",
        onClick && "jn-interactive cursor-pointer hover:bg-jn-well",
        className,
      )}
    >
      {cells.map((cell, index) => {
        const column = columns[index]
        return (
          <span
            key={column?.key ?? index}
            style={widthStyle(column?.width)}
            className={cn(
              "flex min-w-0 shrink-0 items-center text-[13px] text-jn-text",
              ALIGN[column?.align ?? "left"],
              column?.width == null && "min-w-0 flex-1",
            )}
          >
            {cell}
          </span>
        )
      })}
    </div>
  )
}
