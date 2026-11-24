/**
 * PageHeader —— 设计稿 R1-C「分区 · 1 · 外壳 / 页头 PageHeader」。
 *
 * ## 单实例响应两层结构（@container）+ DOM Portal 插槽
 *
 * - 顶行固定为 `标题` +（空间充足时放 `页签`）+ `弹性间隔` + `页面操作` + `全局操作` + `窗口按钮`。
 * - 常规/窄窗口下（2–3 页签在 `<800px`、多页签 `wide` 在 `<1180px`），页签自动通过 CSS `order`
 *   排到第二行左对齐，第一行右侧的操作与窗口控制按钮始终稳固在右上角、绝不被挤压出屏。
 * - 页签与页面操作槽使用持久 DOM 容器（`data-jolu-header-tabs` / `data-jolu-header-actions`）
 *   接收 `<PageTabs>` / `<PageActions>` 的 `createPortal`，免疫 Vite HMR Context 重置与切页竞态。
 */
import { useCallback, type MouseEvent, type ReactNode } from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { cn } from "@/lib/utils"
import { isTauri, api } from "@/lib/api"
import { WindowControls } from "./WindowControls"
import { registerHeaderHosts, useHeaderSlots } from "./PageActions"

export interface PageHeaderProps {
  title: string
  /** 页签插槽。多数页面没有，直接传 prop 时优先于 Portal */
  tabs?: ReactNode
  /** 页面级操作（主按钮、刷新…）。放在全局操作左边 */
  actions?: ReactNode
  /** 全局操作：GitHub、灵动岛显隐、主题、版本号 */
  globalActions?: ReactNode
  className?: string
}

export function PageHeader({ title, tabs, actions, globalActions, className }: PageHeaderProps) {
  const slots = useHeaderSlots()
  const hasTabs = Boolean(tabs) || slots.hasTabs
  const wideTabs = !tabs && slots.wideTabs
  const hasPortalActions = slots.hasActions

  const setTabsHost = useCallback((el: HTMLDivElement | null) => {
    registerHeaderHosts("tabs", el)
  }, [])
  const setActionsHost = useCallback((el: HTMLDivElement | null) => {
    registerHeaderHosts("actions", el)
  }, [])

  /** 拖动与双击最大化。点在按钮/输入/页签上时不抢事件。 */
  const onPointerDown = (event: MouseEvent<HTMLElement>) => {
    if (!isTauri || event.button !== 0) return
    if ((event.target as HTMLElement).closest("button, a, input, [role='tab']")) return
    void api.mainDrag().catch(() => {})
  }
  const onDoubleClick = (event: MouseEvent<HTMLElement>) => {
    if (!isTauri) return
    if ((event.target as HTMLElement).closest("button, a, input, [role='tab']")) return
    void getCurrentWindow().toggleMaximize().catch(() => {})
  }

  return (
    <header
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
      className={cn("@container w-full shrink-0 select-none", className)}
    >
      <div className="flex min-h-[68px] w-full flex-wrap items-center gap-x-4 gap-y-2 py-2.5">
        <h1 className="order-1 shrink-0 truncate text-[26px] leading-none font-bold text-jn-text">
          {title}
        </h1>

        {/* 页签插槽容器始终常驻 DOM（无页签时隐藏占位），让 <PageTabs> 的 createPortal 首帧立即可用 */}
        <div
          ref={setTabsHost}
          data-jolu-header-tabs
          className={cn(
            "order-4 flex max-w-full basis-full items-center overflow-x-auto pb-0.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
            wideTabs
              ? "@[1180px]:order-2 @[1180px]:basis-auto @[1180px]:overflow-visible @[1180px]:pb-0"
              : "@[800px]:order-2 @[800px]:basis-auto @[800px]:overflow-visible @[800px]:pb-0",
            !hasTabs && "hidden",
          )}
        >
          {tabs}
        </div>

        {/* 弹性间隔：窄屏时在标题与右侧操作之间；宽屏页签上浮后移到页签与右侧操作之间 */}
        <span
          className={cn(
            "order-2 min-w-4 flex-1",
            hasTabs && (wideTabs ? "@[1180px]:order-3" : "@[800px]:order-3"),
          )}
        />

        {/* 右侧整组锁定在第一行最右，绝不因页签过长被挤换行 */}
        <div
          className={cn(
            "order-3 flex shrink-0 items-center gap-2",
            hasTabs && (wideTabs ? "@[1180px]:order-4" : "@[800px]:order-4"),
          )}
        >
          <div
            ref={setActionsHost}
            data-jolu-header-actions
            className={cn("flex h-9 shrink-0 items-center gap-2", !hasPortalActions && "hidden")}
          />
          {actions && <div className="flex h-9 shrink-0 items-center gap-2">{actions}</div>}
          {globalActions && <div className="flex shrink-0 items-center gap-2">{globalActions}</div>}
          <WindowControls />
        </div>
      </div>
    </header>
  )
}
