import type { ReactNode } from "react"

/**
 * 统计卡行：按所在容器宽度排列，≥ 720px 四列，更窄两列（两行）。
 *
 * 为什么用容器宽度而不是视口：主窗口有侧栏、详情栏，同一视口下内容区宽窄不同。
 * 720px ≈ 四列每列 ≥ 170px（标签 + 数字 + 一行说明放得下）；固定四列在 800px
 * 容器里会把卡片压到读不下，`auto-fit + minmax` 又会在 800px 时仍排四列而裁切。
 * 骨架屏（`LoadingSurface` 的 tasks / stats）用同一断点，加载前后列数一致。
 */
export const METRIC_GRID = "grid grid-cols-2 gap-3 @[720px]:grid-cols-4"

export function MetricGrid({ children }: { children: ReactNode }) {
  return (
    <div className="@container min-w-0">
      <div className={METRIC_GRID}>{children}</div>
    </div>
  )
}
