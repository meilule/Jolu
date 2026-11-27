/**
 * Pagination —— 设计稿 R1-C「分区 · 5 · 容器与反馈 / 分页 Pagination」。
 *
 * 规格取自画板：页码按钮 30×30 圆形；当前页主色实心，其余 `jn-well`；
 * 左侧一行汇总（12 / 次级色）；超过 7 页用省略号折叠。
 *
 * ## 省略号的折叠规则
 *
 * 始终保留**首页与末页**，当前页两侧各留一个邻居，其余折成 `…`：
 *
 * ```
 * 1 … 4 5 6 … 42      （当前 5）
 * 1 2 3 4 5 6 … 42    （当前 3，左侧不折）
 * 1 … 37 38 39 40 41 42（当前 41，右侧不折）
 * ```
 *
 * 规则写死在这里而不是让调用方传「要显示哪些页」——那种 API 每个页面都会
 * 各算一遍，且很容易算出「只有 3 个页码没有省略号」这种不一致的结果。
 */
import { cn } from "@/lib/utils"
import { Icon } from "@/icons"
import { TactileButton } from "@/motion"
import { pageItems } from "./pageItems"

// 纯逻辑单独放 pageItems.ts：node --test 只能直接跑 .ts，跑不了含 JSX 的 .tsx。
export { pageItems }

const CELL = "flex size-[30px] items-center justify-center rounded-full text-xs font-semibold"

export interface PaginationProps {
  page: number
  pageCount: number
  onChange: (page: number) => void
  /** 左侧汇总，如「每页 10 条 · 共 42 页」 */
  summary?: string
  className?: string
}

export function Pagination({ page, pageCount, onChange, summary, className }: PaginationProps) {
  if (pageCount <= 1) return null
  const go = (next: number) => {
    if (next < 1 || next > pageCount || next === page) return
    onChange(next)
  }
  return (
    <div className={cn("flex w-full items-center gap-1.5", className)}>
      {summary && <span className="text-xs text-jn-text-3">{summary}</span>}
      <span className="flex-1" />
      <TactileButton
        variant="icon"
        type="button"
        aria-label="上一页"
        disabled={page <= 1}
        onClick={() => go(page - 1)}
        className={cn(CELL, "jn-interactive bg-jn-well text-jn-text-2 disabled:pointer-events-none disabled:opacity-45")}
      >
        <Icon name="prev" size="sm" />
      </TactileButton>
      {pageItems(page, pageCount).map((item, index) =>
        item == null ? (
          <span key={`gap-${index}`} aria-hidden className={cn(CELL, "text-jn-text-3")}>
            …
          </span>
        ) : item === page ? (
          <button
            key={item}
            type="button"
            aria-label={`第 ${item} 页`}
            aria-current="page"
            onClick={() => go(item)}
            className={cn(CELL, "jn-interactive jn-no-press bg-jn-primary text-jn-on-primary")}
          >
            {item}
          </button>
        ) : (
          <TactileButton
            key={item}
            variant="icon"
            type="button"
            aria-label={`第 ${item} 页`}
            onClick={() => go(item)}
            className={cn(CELL, "jn-interactive bg-jn-well text-jn-text-2")}
          >
            {item}
          </TactileButton>
        ),
      )}
      <TactileButton
        variant="icon"
        type="button"
        aria-label="下一页"
        disabled={page >= pageCount}
        onClick={() => go(page + 1)}
        className={cn(CELL, "jn-interactive bg-jn-well text-jn-text-2 disabled:pointer-events-none disabled:opacity-45")}
      >
        <Icon name="next" size="sm" />
      </TactileButton>
    </div>
  )
}
