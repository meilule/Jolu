/**
 * PanelPage —— 自带滚动的整页壳（Agent / 模型路由 / 供应商）。
 *
 * 与旧 `components/panel/PanelPage.tsx` 契约一致，只是底色换成 `jn-stage`。
 * 保留它解决的问题：主面板内容区是带 `overflow` 的 flex 项，页面里写 `h-full`
 * 会按**内容高**解析，`absolute inset-0` 的详情抽屉随之缩成内容高，表现为
 * 「页面中部截断、下方大片空白」。这里用 `absolute inset-0` 钉住可视区。
 *
 * 新增同类页面必须在 `PANEL_PAGES` 登记，主窗口靠 `usesOwnScroll` 统一处理，
 * 不要再在各处写 `page === "xx"` 特例。
 */

/** 需要自带滚动的页面。替换期间新外壳沿用同一份清单。 */
export const PANEL_PAGES = ["agents", "gateway", "providers"] as const

export function usesOwnScroll(page: string): boolean {
  return (PANEL_PAGES as readonly string[]).includes(page)
}
