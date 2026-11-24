import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react"
import { createPortal } from "react-dom"
import { cn } from "@/lib/utils"
import { Icon, type IconName } from "@/icons"
import { formatNoticeAmount } from "@/lib/appNotices"
import { PLATFORMS } from "@/lib/platforms"
import {
  TOAST_EXIT_MS,
  markToastExiting,
  pushToast,
  removeToast,
  toastRemaining,
  pauseToast,
  visibleToasts,
  type PauseReason,
  type ToastInput,
  type ToastItem,
  type ToastTone,
} from "@/lib/toastQueue"

/**
 * 主面板的轻量 Toast。
 *
 * 队列规则全在 lib/toastQueue.ts 里，这里只负责计时与渲染。
 * 刻意不引第三方库：配色与动效直接用项目既有的 token 和 .motion-* 过渡，
 * 深浅色、reduced-motion 都不需要额外桥接。
 */

export type ToastApi = {
  show: (input: ToastInput) => void
  success: (title: string, detail?: string) => void
  info: (title: string, detail?: string) => void
  warn: (title: string, detail?: string) => void
  danger: (title: string, detail?: string) => void
  dismiss: (id: number, ifCount?: number) => void
}

type ToastState = {
  items: ToastItem[]
  pause: (id: number, reason: PauseReason, paused: boolean) => void
}

/** 拆两个 context：调用方只订阅稳定的 api，不被队列变化牵连重渲染 */
const ToastApiContext = createContext<ToastApi | null>(null)
const ToastStateContext = createContext<ToastState | null>(null)

/** 没有 Provider 时的空操作。浏览器预览的 Showcase 也会渲染设置页，不能因此崩 */
const NOOP: ToastApi = {
  show: () => {},
  success: () => {},
  info: () => {},
  warn: () => {},
  danger: () => {},
  dismiss: () => {},
}

export function useToast(): ToastApi {
  return useContext(ToastApiContext) ?? NOOP
}

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([])
  const nextId = useRef(1)
  /**
   * 退场卸载的定时器，按 id 记账。
   *
   * 刻意**不**放进 effect 的 cleanup 里清：cleanup 会在 items 每次变化时跑，
   * 一条新提示入队就会取消上一条已排期的卸载，那条便永远留在 DOM 里——
   * 透明但仍占位，把还看得见的提示一路往下顶。只在组件卸载时统一清。
   */
  const exitTimers = useRef(new Map<number, number>())

  const dismiss = useCallback((id: number, ifCount?: number) => {
    setItems((list) => ifCount !== undefined && list.find(item => item.id === id)?.count !== ifCount
      ? list : markToastExiting(list, id))
  }, [])
  const pause = useCallback((id: number, reason: PauseReason, paused: boolean) => {
    const now = Date.now()
    setItems((list) => pauseToast(list, id, reason, paused, now))
  }, [])

  const api = useMemo<ToastApi>(() => {
    const show = (input: ToastInput) => {
      // id 与时间戳在 updater 之外取：updater 必须是纯函数，
      // StrictMode 下它会被调用两次，副作用留在里面会让号跳、时间错
      const id = nextId.current++
      const now = Date.now()
      setItems((list) => pushToast(list, input, id, now))
    }
    const shorthand = (tone: ToastTone) => (title: string, detail?: string) =>
      show({ tone, title, detail })
    return {
      show,
      success: shorthand("success"),
      info: shorthand("info"),
      warn: shorthand("warn"),
      danger: shorthand("danger"),
      dismiss,
    }
  }, [dismiss])

  // 只给已展示且未暂停的提示计时；排队和持续错误提示都不安排到期回调。
  useEffect(() => {
    const now = Date.now()
    const timers = visibleToasts(items)
      .filter((item) => !item.exiting && item.startedAt !== null && Number.isFinite(toastRemaining(item, now)))
      .map((item) =>
        window.setTimeout(
          () => {
            const at = Date.now()
            setItems((list) => {
              const current = list.find(value => value.id === item.id)
              // 同一节点可能刚刚续时或暂停，迟到的旧回调不能将它关闭。
              return current && current.startedAt !== null && toastRemaining(current, at) === 0
                ? markToastExiting(list, item.id) : list
            })
          },
          toastRemaining(item, now),
        ),
      )
    return () => timers.forEach(window.clearTimeout)
  }, [items])

  // 退场动画跑完再从列表里摘掉；每条只排期一次，且排了就不撤
  useEffect(() => {
    for (const item of items) {
      if (!item.exiting || exitTimers.current.has(item.id)) continue
      exitTimers.current.set(item.id, window.setTimeout(() => {
        exitTimers.current.delete(item.id)
        const now = Date.now()
        setItems((list) => removeToast(list, item.id, now))
      }, TOAST_EXIT_MS))
    }
  }, [items])

  useEffect(() => {
    const timers = exitTimers.current
    return () => {
      timers.forEach(window.clearTimeout)
      timers.clear()
    }
  }, [])

  const state = useMemo<ToastState>(() => ({ items, pause }), [items, pause])

  return (
    <ToastApiContext.Provider value={api}>
      <ToastStateContext.Provider value={state}>{children}</ToastStateContext.Provider>
    </ToastApiContext.Provider>
  )
}

const TONE: Record<ToastTone, { icon: IconName; className: string }> = {
  success: { icon: "check", className: "bg-jn-primary-soft text-jn-primary-ink" },
  info: { icon: "info", className: "bg-jn-info-soft text-jn-info" },
  warn: { icon: "warning", className: "bg-jn-warning-soft text-jn-warning" },
  danger: { icon: "close", className: "bg-jn-danger-soft text-jn-danger" },
}

/** 右下角公共通知区（R1-C）；卡片能接收指针，其余区域继续透传给页面。 */
export function ToastViewport({ className }: { className?: string }) {
  const toast = useToast()
  const state = useContext(ToastStateContext)
  const [dialog, setDialog] = useState<HTMLElement | null>(null)
  const [scrollable, setScrollable] = useState(false)
  const anchor = useRef<HTMLSpanElement>(null)
  const viewport = useRef<HTMLDivElement>(null)
  const shown = Boolean(state?.items.length)
  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || !(event.target instanceof Element) || !viewport.current?.contains(event.target)) return
      const card = event.target.closest<HTMLElement>("[data-toast-id]")
      if (!card) return
      // window 捕获阶段先于 Radix 的 document 监听，只关闭当前通知，不误关表单。
      event.preventDefault()
      event.stopImmediatePropagation()
      toast.dismiss(Number(card.dataset.toastId))
    }
    window.addEventListener("keydown", escape, true)
    return () => window.removeEventListener("keydown", escape, true)
  }, [toast.dismiss])
  useEffect(() => {
    // 通知的 DOM 跟随最上层表单，确保读屏和焦点约束都能访问关闭、重试按钮。
    const sync = () => setDialog([...document.querySelectorAll<HTMLElement>('[role="dialog"], [role="alertdialog"]')]
      .filter(element => element.dataset.state !== "closed" && !element.closest('[aria-hidden="true"]')).at(-1) ?? null)
    sync()
    const observer = new MutationObserver(sync)
    observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["data-state", "aria-hidden"] })
    return () => observer.disconnect()
  }, [])
  useEffect(() => {
    // 保存表单可能同时关闭弹窗并创建通知；MutationObserver 同步前宿主已被移除。
    const element = viewport.current
    if (dialog?.isConnected && shown && element?.isConnected) element.showPopover?.()
  }, [dialog, shown])
  useEffect(() => {
    const element = viewport.current
    if (!element) return
    // 普通通知不裁切阴影，否则透明容器也会形成一整块灰色矩形。
    const measure = () => setScrollable(element.scrollHeight > element.clientHeight + 1)
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    for (const child of element.children) observer.observe(child)
    measure()
    return () => observer.disconnect()
  }, [dialog, shown, state?.items])
  useEffect(() => {
    // Portal 换宿主时原按钮被卸载，不一定有 blur/leave；解除旧节点的暂停状态。
    for (const item of state?.items ?? []) {
      state?.pause(item.id, "pointer", false)
      state?.pause(item.id, "focus", false)
    }
  }, [dialog])
  const bounds = anchor.current?.parentElement?.getBoundingClientRect()
  const cards = state && shown ? (
    <div
      ref={viewport}
      popover={dialog ? "manual" : undefined}
      style={dialog ? { position: "fixed", bottom: Math.max(20, window.innerHeight - (bounds?.bottom ?? window.innerHeight) + 20), right: Math.max(20, window.innerWidth - (bounds?.right ?? window.innerWidth) + 20), left: "auto", top: "auto", margin: 0, border: 0, background: "transparent" } : undefined}
      role="region"
      aria-label="操作通知"
      aria-live="polite"
      aria-relevant="additions text"
      className={cn(
        "pointer-events-none absolute right-5 bottom-5 z-[60] flex max-h-[calc(100dvh-110px)] w-[360px] max-w-[calc(100%-32px)] flex-col-reverse gap-2.5 bg-transparent p-1 shadow-none",
        scrollable ? "overflow-y-auto overscroll-contain" : "overflow-visible",
        className,
      )}
    >
      {visibleToasts(state.items).map(item => <ToastCard key={item.id} item={item} pause={state.pause} />)}
      {state.items.length > 3 && <p className="px-2 text-right text-[12px] text-jn-text-3">另有 {state.items.length - 3} 条提示等待显示</p>}
    </div>
  ) : null
  // 手动 Popover 只提升绘制层级，不抢焦点；避免表单 transform 裁切或挪动通知。
  return <><span ref={anchor} hidden />{dialog && cards ? createPortal(cards, dialog) : cards}</>
}

function ToastCard({ item, pause }: { item: ToastItem; pause: ToastState["pause"] }) {
  const toast = useToast()
  const [expanded, setExpanded] = useState(false)
  const [overflow, setOverflow] = useState(false)
  const [acting, setActing] = useState(false)
  const detail = useRef<HTMLParagraphElement>(null)
  const tone = TONE[item.tone]
  const iconName: IconName = item.balance ? "wallet" : tone.icon
  useEffect(() => {
    const element = detail.current
    if (!element || expanded) return
    const measure = () => setOverflow(element.scrollHeight > element.clientHeight + 1)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [item.detail, expanded])
  const act = async () => {
    if (acting || !item.action) return
    setActing(true)
    try {
      const completed = await item.action.onClick()
      if (completed === false) return
      // 重试期间若同一错误再次出现并被合并，保留已更新的错误与重试入口。
      toast.dismiss(item.id, item.count)
    } catch {
      toast.danger("操作未完成", "请稍后重试")
    } finally { setActing(false) }
  }
  return (
    <article
      data-toast-id={item.id}
      data-toast-tone={item.tone}
      data-balance-notice={item.balance ? "true" : undefined}
      data-state={item.exiting ? "closed" : "open"}
      role={item.tone === "danger" ? "alert" : undefined}
      className={cn("motion-toast pointer-events-auto relative flex shrink-0 items-start gap-3 rounded-xl border border-jn-line bg-jn-surface p-3.5 text-jn-text shadow-jn-surface", item.balance && "flex-wrap p-5")}
      onPointerEnter={() => pause(item.id, "pointer", true)}
      onPointerLeave={() => pause(item.id, "pointer", false)}
      onFocusCapture={() => pause(item.id, "focus", true)}
      onBlurCapture={event => { if (!event.currentTarget.contains(event.relatedTarget)) pause(item.id, "focus", false) }}
    >
      <span className={cn("mt-0.5 grid size-7 shrink-0 place-items-center rounded-full", tone.className, item.balance && "size-[34px]")}>
        <Icon name={iconName} size="md" />
      </span>
      <div className="min-w-0 flex-1 py-0.5">
        <div className="flex min-w-0 items-center gap-2">
          <p className="min-w-0 truncate text-[13px] font-semibold leading-5" title={item.title}>{item.title}</p>
          {item.count > 1 && <span className="shrink-0 rounded-md bg-jn-stage px-1.5 text-[11px] leading-[18px] text-jn-text-3" aria-label={`重复 ${item.count} 次`}>×{item.count}</span>}
        </div>
        {item.balance && <p className="mt-1 truncate text-xs text-jn-text-3" title={item.balance.connectionName}>{item.balance.connectionName} · {PLATFORMS.find(platform => platform.id === item.balance?.platform)?.name ?? "API Key"}</p>}
        {item.detail && <p ref={detail} id={`toast-detail-${item.id}`} className={cn("mt-1 whitespace-pre-wrap text-xs leading-[1.6] text-jn-text-3 [overflow-wrap:anywhere]", !expanded && "line-clamp-2")}>{item.detail}</p>}
        {!item.balance && (overflow || item.action) && <div className="mt-2 flex flex-wrap items-center gap-3">
          {item.action && <button type="button" disabled={acting} onClick={() => void act()} className="flex min-h-7 items-center gap-1 rounded-md text-xs font-medium text-jn-primary outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jn-primary disabled:opacity-50">
            {acting && <Icon name="loading" size="xs" className="animate-spin" />}{item.action.label}
          </button>}
          {overflow && <button type="button" aria-expanded={expanded} aria-controls={`toast-detail-${item.id}`} onClick={() => setExpanded(value => !value)} className="min-h-7 rounded-md text-xs text-jn-text-3 outline-none hover:text-jn-text focus-visible:outline-2 focus-visible:outline-jn-primary">{expanded ? "收起原因" : "展开原因"}</button>}
        </div>}
      </div>
      <button type="button" aria-label={`关闭提示：${item.title}`} onClick={() => toast.dismiss(item.id)} className="-mr-1 -mt-1 grid size-7 shrink-0 place-items-center rounded-lg text-jn-text-3 outline-none transition-colors hover:bg-jn-stage hover:text-jn-text focus-visible:outline-2 focus-visible:outline-jn-primary">
        <Icon name="close" size="sm" />
      </button>
      {item.balance && <div className="mt-2 min-w-0 basis-full">
        <p className="text-xs text-jn-text-3">剩余余额</p>
        <p className="mt-1 flex flex-wrap items-baseline gap-2 font-mono text-[30px] font-semibold leading-10 tracking-tight" data-balance-amount>
          {formatNoticeAmount(item.balance.remaining, item.balance.currency)}<span className="font-sans text-xs font-normal tracking-normal text-jn-text-3">{item.balance.currency}</span>
        </p>
        <p className="mt-1 text-xs leading-5 text-jn-text-3">已低于设置的 <span className="text-jn-warning">{formatNoticeAmount(item.balance.threshold, item.balance.currency)} {item.balance.currency}</span> 提醒阈值</p>
        {item.action && <div className="mt-4 flex justify-end border-t border-jn-line pt-3">
          <button type="button" disabled={acting} onClick={() => void act()} className="flex min-h-8 items-center gap-1.5 rounded-lg bg-jn-primary-soft px-3 text-xs font-medium text-jn-primary-ink outline-none hover:bg-jn-primary-wash focus-visible:outline-2 focus-visible:outline-jn-primary disabled:opacity-50">
            {acting ? <Icon name="loading" size="sm" className="animate-spin" /> : null}{item.action.label}<Icon name="external" size="sm" />
          </button>
        </div>}
      </div>}
    </article>
  )
}
