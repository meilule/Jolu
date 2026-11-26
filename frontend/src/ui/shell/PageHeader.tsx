/**
 * PageHeader —— 设计稿 R1-C「分区 · 1 · 外壳 / 页头 PageHeader」。
 *
 * ## 单实例 CSS Grid 响应结构 + 真实宽度测量 + DOM Portal 插槽
 *
 * - 第一行始终锁定为 `标题`（左上）与 `全局操作 + 窗口按钮`（右上），绝不因页签或
 *   页面操作变宽（例如新版本徽章 `v0.1.11`、调用明细多出「导出 CSV」）而被挤换行。
 * - 空间不足或页面传入 `<PageTabs wide>` 时，页签通过 CSS Grid 行列定位稳定落到
 *   标题正下方第二行左对齐；极窄窗口下页面操作同样自动下沉，DOM 节点原地不动，
 *   Portal 与 Motion `layoutId` 弹簧滑块零重挂、零闪烁。
 * - 第一行固定 42px 轨道 + 13px 上边距（与单行 68px 居中对齐一致），在不同页面间切换时
 *   标题与右上角窗口按钮纵坐标像素级稳定。
 */
import { useCallback, useLayoutEffect, useRef, useState, type MouseEvent, type ReactNode } from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { cn } from "@/lib/utils"
import { isTauri, api } from "@/lib/api"
import { WindowControls } from "./WindowControls"
import { registerHeaderHosts, useHeaderSlots } from "./PageActions"
import { resolveHeaderLayout, type HeaderLayoutResult } from "./pageHeaderLayout"

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
  const hasAnyActions = Boolean(actions) || hasPortalActions

  const headerRef = useRef<HTMLElement | null>(null)
  const titleRef = useRef<HTMLHeadingElement | null>(null)
  const tabsRef = useRef<HTMLDivElement | null>(null)
  const actionsWrapRef = useRef<HTMLDivElement | null>(null)
  const chromeRef = useRef<HTMLDivElement | null>(null)

  const [layout, setLayout] = useState<HeaderLayoutResult>(() =>
    resolveHeaderLayout({
      containerWidth: 0,
      titleWidth: 0,
      tabsWidth: 0,
      actionsWidth: 0,
      chromeWidth: 0,
      hasTabs,
      wideTabs,
      hasActions: hasAnyActions,
    }),
  )

  const setTabsHost = useCallback((el: HTMLDivElement | null) => {
    tabsRef.current = el
    registerHeaderHosts("tabs", el)
  }, [])
  const setActionsHost = useCallback((el: HTMLDivElement | null) => {
    registerHeaderHosts("actions", el)
  }, [])

  useLayoutEffect(() => {
    const measure = () => {
      const containerWidth = headerRef.current?.clientWidth ?? 0
      const titleWidth = titleRef.current?.scrollWidth ?? 0
      const tabsWidth = hasTabs ? (tabsRef.current?.scrollWidth ?? 0) : 0
      const actionsWidth = hasAnyActions ? (actionsWrapRef.current?.scrollWidth ?? 0) : 0
      const chromeWidth = chromeRef.current?.scrollWidth ?? 0

      const next = resolveHeaderLayout({
        containerWidth,
        titleWidth,
        tabsWidth,
        actionsWidth,
        chromeWidth,
        hasTabs,
        wideTabs,
        hasActions: hasAnyActions,
      })

      setLayout((prev) =>
        prev.tabsRow === next.tabsRow && prev.actionsRow === next.actionsRow ? prev : next,
      )
    }

    measure()

    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(measure) : null
    if (ro) {
      for (const node of [headerRef.current, titleRef.current, tabsRef.current, actionsWrapRef.current, chromeRef.current]) {
        if (node) ro.observe(node)
      }
    }

    // 监听页签/操作/全局按钮内部子节点增删（例如切到调用明细多出「导出 CSV」或出现版本更新按钮）
    const mo = typeof MutationObserver !== "undefined" ? new MutationObserver(measure) : null
    if (mo) {
      for (const node of [tabsRef.current, actionsWrapRef.current, chromeRef.current]) {
        if (node) mo.observe(node, { childList: true, subtree: true, characterData: true })
      }
    }

    return () => {
      ro?.disconnect()
      mo?.disconnect()
    }
  }, [title, hasTabs, wideTabs, hasAnyActions])

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

  const tabsBelow = hasTabs && layout.tabsRow === "below"
  const actionsBelow = hasAnyActions && layout.actionsRow !== "inline"
  const actionsOnRow3 = hasAnyActions && layout.actionsRow === "row3"
  const multiRow = tabsBelow || actionsBelow

  const gridClass = !multiRow
    ? "grid-cols-[auto_auto_minmax(12px,1fr)_auto_auto] grid-rows-[42px] py-[13px]"
    : !actionsBelow
      ? "grid-cols-[auto_minmax(12px,1fr)_auto_auto] grid-rows-[42px_auto] gap-y-2 pt-[13px] pb-1.5"
      : actionsOnRow3
        ? "grid-cols-[auto_minmax(12px,1fr)_auto] grid-rows-[42px_auto_auto] gap-y-2 pt-[13px] pb-1.5"
        : "grid-cols-[auto_minmax(12px,1fr)_auto] grid-rows-[42px_auto] gap-y-2 pt-[13px] pb-1.5"

  const chromeColClass = !multiRow
    ? "col-start-5 row-start-1"
    : !actionsBelow
      ? "col-start-4 row-start-1"
      : "col-start-3 row-start-1"

  const actionsPlacementClass = !hasAnyActions
    ? "hidden"
    : !actionsBelow
      ? !multiRow
        ? "col-start-4 row-start-1 mr-2"
        : "col-start-3 row-start-1 mr-2"
      : actionsOnRow3
        ? "col-span-full row-start-3 justify-self-start max-w-full overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
        : tabsBelow
          ? "col-span-full row-start-2 justify-self-end"
          : "col-span-full row-start-2 justify-self-start max-w-full overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"

  const tabsPlacementClass = !hasTabs
    ? "hidden"
    : tabsBelow
      ? "col-span-full row-start-2 justify-self-start max-w-full overflow-x-auto py-0.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      : "col-start-2 row-start-1 ml-4"

  return (
    <header
      ref={headerRef}
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
      className={cn("@container w-full shrink-0 select-none", className)}
    >
      <div className={cn("grid w-full items-center", gridClass)}>
        <h1
          ref={titleRef}
          className="col-start-1 row-start-1 shrink-0 whitespace-nowrap text-[26px] leading-none font-bold text-jn-text"
        >
          {title}
        </h1>

        {/* 页签插槽容器始终常驻 DOM（无页签时隐藏占位），让 <PageTabs> 的 createPortal 首帧立即可用 */}
        <div
          ref={setTabsHost}
          data-jolu-header-tabs
          data-tabs-row={hasTabs ? layout.tabsRow : undefined}
          className={cn("flex w-fit items-center", tabsPlacementClass)}
        >
          {tabs}
        </div>

        {/* 页面操作插槽：宽屏在全局操作左侧；极窄窗口下沉到副行，确保窗口按钮永不换行 */}
        <div
          ref={actionsWrapRef}
          data-jolu-header-page-actions
          data-actions-row={hasAnyActions ? layout.actionsRow : undefined}
          className={cn("flex min-h-9 w-fit shrink-0 items-center gap-2", actionsPlacementClass)}
        >
          <div
            ref={setActionsHost}
            data-jolu-header-actions
            className={cn("flex h-9 shrink-0 items-center gap-2", !hasPortalActions && "hidden")}
          />
          {actions && <div className="flex h-9 shrink-0 items-center gap-2">{actions}</div>}
        </div>

        {/* 全局操作 + 窗口按钮：始终锁定在第一行最右端 */}
        <div
          ref={chromeRef}
          data-jolu-header-chrome
          className={cn("flex shrink-0 items-center justify-self-end gap-2", chromeColClass)}
        >
          {globalActions && <div className="flex shrink-0 items-center gap-2">{globalActions}</div>}
          <WindowControls />
        </div>
      </div>
    </header>
  )
}
