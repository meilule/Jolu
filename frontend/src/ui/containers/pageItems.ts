/**
 * 分页的页码折叠规则（纯逻辑，不含 JSX）。
 *
 * 单独成文件的原因：`node --test` 能直接跑 `.ts`，跑不了含 JSX 的 `.tsx`。
 * 把这段逻辑留在组件里，就得为它引入一层构建才能测——不值得。
 *
 * 规则：始终保留首页与末页；当前页两侧各留一个邻居，其余折成 `null`（省略号）。
 * **总项数固定 7**：靠近两端时不够折的时候，用更多连续页码补齐——
 * 宽度固定，翻页时按钮不会左右跳。
 */

/** 生成要显示的页码序列，`null` 表示省略号。 */
export function pageItems(current: number, total: number): (number | null)[] {
  if (total <= 7) return Array.from({ length: total }, (_, index) => index + 1)
  const items: (number | null)[] = [1]
  // 当前页左右各留一个邻居；靠近两端时把另一侧的额度让出来，保持总宽度稳定
  const start = Math.max(2, Math.min(current - 1, total - 4))
  const end = Math.min(total - 1, Math.max(current + 1, 5))
  if (start > 2) items.push(null)
  for (let page = start; page <= end; page += 1) items.push(page)
  if (end < total - 1) items.push(null)
  items.push(total)
  return items
}
