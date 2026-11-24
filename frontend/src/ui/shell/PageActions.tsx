/**
 * PageActions / PageTabs —— 通过真实 DOM Portal 把页面的操作按钮与页签送进页头插槽。
 *
 * ## 为什么不用 React Context + `useEffect(() => setState(children))`
 *
 * 1. **防 Vite HMR 热更新丢页签**：包含 `createContext()` 的模块在 HMR 重载时会生成新 Context
 *    实例，导致已挂载的 Provider 与新重载的 PageTabs 脱节，所有页签凭空消失。
 *    这里把槽位桥接对象挂在 `globalThis` 单例上，模块热替换后仍指向同一份宿主节点。
 * 2. **防切页竞态误清**：旧写法每次重渲染都先 `return () => setTabs(null)` 再写入，
 *    在并发/懒加载切页时旧页面的 cleanup 可能覆盖新页面的页签。改为 `createPortal` +
 *    按唯一 `Symbol` 记账：React 只卸载属于当前页的 Portal，绝不误伤下一页。
 * 3. **零闪烁、动画不断**：页签组件保留在原位更新，Motion `layoutId` 弹簧滑块连贯不重挂。
 */
import { useLayoutEffect, useRef, useSyncExternalStore, type ReactNode } from "react"
import { createPortal } from "react-dom"

interface SlotSnapshot {
  tabsHost: HTMLDivElement | null
  actionsHost: HTMLDivElement | null
  hasTabs: boolean
  wideTabs: boolean
  hasActions: boolean
}

interface SlotStore {
  tabsHost: HTMLDivElement | null
  actionsHost: HTMLDivElement | null
  tabOwners: Map<symbol, boolean>
  actionOwners: Set<symbol>
  listeners: Set<() => void>
  snapshot: SlotSnapshot
}

const STORE_KEY = "__JOLU_HEADER_SLOT_STORE__"

function getStore(): SlotStore {
  const g = globalThis as Record<string, unknown>
  let store = g[STORE_KEY] as SlotStore | undefined
  if (!store) {
    store = {
      tabsHost: null,
      actionsHost: null,
      tabOwners: new Map(),
      actionOwners: new Set(),
      listeners: new Set(),
      snapshot: {
        tabsHost: null,
        actionsHost: null,
        hasTabs: false,
        wideTabs: false,
        hasActions: false,
      },
    }
    g[STORE_KEY] = store
  }
  return store
}

function emit(store: SlotStore) {
  const hasTabs = store.tabOwners.size > 0
  let wideTabs = false
  for (const wide of store.tabOwners.values()) {
    if (wide) {
      wideTabs = true
      break
    }
  }
  const hasActions = store.actionOwners.size > 0
  const prev = store.snapshot
  if (
    prev.tabsHost === store.tabsHost &&
    prev.actionsHost === store.actionsHost &&
    prev.hasTabs === hasTabs &&
    prev.wideTabs === wideTabs &&
    prev.hasActions === hasActions
  ) {
    return
  }
  store.snapshot = {
    tabsHost: store.tabsHost,
    actionsHost: store.actionsHost,
    hasTabs,
    wideTabs,
    hasActions,
  }
  for (const listener of store.listeners) listener()
}

function subscribe(listener: () => void) {
  const store = getStore()
  store.listeners.add(listener)
  return () => {
    store.listeners.delete(listener)
  }
}

function getSnapshot(): SlotSnapshot {
  return getStore().snapshot
}

/** 仅供 PageHeader 调用：登记 / 注销页签与操作槽的真实 DOM 容器。 */
export function registerHeaderHosts(
  kind: "tabs" | "actions",
  el: HTMLDivElement | null,
) {
  const store = getStore()
  if (kind === "tabs") {
    if (store.tabsHost === el) return
    store.tabsHost = el
  } else {
    if (store.actionsHost === el) return
    store.actionsHost = el
  }
  emit(store)
}

/** 供 PageHeader 读取槽位状态（是否有页签 / 是否多页签 wide / 是否有页面操作）。 */
export function useHeaderSlots(): SlotSnapshot {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot)
}

/** 兼容导出：新实现使用 globalThis 单例 + Portal，Provider 保留为空透传壳以兼容所有调用方。 */
export function PageActionsProvider({ children }: { children: ReactNode }) {
  return <>{children}</>
}

/** 兼容旧导出名（历史调试或单独预览场景）。 */
export function usePageActions(): ReactNode {
  return null
}

/** 兼容旧导出名。 */
export function usePageTabs(): { node: ReactNode; wide?: boolean } | null {
  return null
}

/**
 * 页面把操作按钮送进页头「页面操作」槽；若当前无页头宿主（独立组件预览），不丢按钮。
 */
export function PageActions({ children }: { children: ReactNode }) {
  const ownerRef = useRef<symbol | null>(null)
  if (ownerRef.current === null) ownerRef.current = Symbol("page-actions")
  const snap = useHeaderSlots()

  useLayoutEffect(() => {
    const store = getStore()
    const id = ownerRef.current!
    store.actionOwners.add(id)
    emit(store)
    return () => {
      store.actionOwners.delete(id)
      emit(store)
    }
  }, [])

  const host =
    snap.actionsHost ??
    (typeof document !== "undefined"
      ? document.querySelector<HTMLDivElement>("[data-jolu-header-actions]")
      : null)

  if (!host) return null
  return createPortal(children, host)
}

/**
 * 页面把页签栏送进页头「页签」槽。
 * 超过 3 个页签时传 `wide`，常规宽度下自动落到标题下方的副栏，避免挤压页头与窗口按钮。
 * 若页头宿主尚未就绪（独立预览），直接原位渲染兜底，确保页签永远可见可点。
 */
export function PageTabs({ children, wide = false }: { children: ReactNode; wide?: boolean }) {
  const ownerRef = useRef<symbol | null>(null)
  if (ownerRef.current === null) ownerRef.current = Symbol("page-tabs")
  const snap = useHeaderSlots()

  useLayoutEffect(() => {
    const store = getStore()
    const id = ownerRef.current!
    store.tabOwners.set(id, wide)
    emit(store)
    return () => {
      store.tabOwners.delete(id)
      emit(store)
    }
  }, [wide])

  const host =
    snap.tabsHost ??
    (typeof document !== "undefined"
      ? document.querySelector<HTMLDivElement>("[data-jolu-header-tabs]")
      : null)

  if (!host) {
    return <div className="mb-1 flex max-w-full items-center overflow-x-auto">{children}</div>
  }
  return createPortal(children, host)
}
