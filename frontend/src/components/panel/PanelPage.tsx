import { cn } from "@/lib/utils"

/**
 * 自带滚动的整页布局壳（Agent / 模型路由 / 供应商）。
 *
 * 为什么需要它：主面板的内容区是 `overflow-hidden`（或 `overflow-auto`）的 flex 项，
 * 页面里写 `h-full` 时会按**内容高**解析——详情抽屉那类 `absolute inset-0` 浮层
 * 随之缩成内容高，表现为「页面中部截断、下方一大片空白」。
 *
 * 这个壳用 `absolute inset-0` 钉住内容区可视区（父级即滚动容器，需要 `relative`），
 * 内部页面永远拿到真实高度，不再依赖百分比高度。新增同类页面一律用它，别再写 `h-full`。
 */
export function PanelPage({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={cn("absolute inset-0 flex min-h-0 overflow-hidden bg-jolu-bg", className)}>{children}</div>
}

/** 把页面挂到平时可滚动的容器里时，用这个包一层：容器不再滚动，交给 PanelPage 内部。 */
export const PANEL_PAGE_PAGES = ["agents", "gateway", "providers"] as const

export function usesOwnScroll(page: string): boolean {
  return (PANEL_PAGE_PAGES as readonly string[]).includes(page)
}
