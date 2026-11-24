/**
 * Toast —— 设计稿 R1-C「分区 · 5 · 容器与反馈 / 通知 Toast」。
 *
 * 规格取自画板：宽 340、`jn-surface`、圆角 18、1px `jn-line`、内边距 14、间距 12。
 * 右下角堆叠，**4 秒自动消失，错误需手动关**。
 *
 * ## 为什么错误不自动消失
 *
 * 成功可以自动消失——用户已经看到结果了。错误不行：它往往带一句要读的
 * 原因（「连接返回 401，请重新授权」），4 秒读完还没反应过来就没了，
 * 只能重新触发一次同样的错误。所以 danger 一律手动关闭。
 *
 * ## 为什么用 Provider + hook 而不是受控列表
 *
 * 通知是「在任何地方都能触发」的横切行为。让每个页面自己维护一个
 * `useState<Toast[]>` 并把列表传给视口，等于把这份状态复制到所有页面。
 * Provider 挂在根部一次，页面调 `useToast().push(...)` 即可。
 *
 * ## 定位
 *
 * 视口用 `fixed bottom-4 right-4`，与灵动岛窗口同一角落——两者不会同时
 * 长时间出现（灵动岛是常驻，Toast 是瞬时），视觉上不冲突。
 */
import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode } from "react"
import { cn } from "@/lib/utils"
import { Icon, type IconName } from "@/icons"
import { IconButton } from "@/ui/actions/IconButton"

const TONE = {
  success: { icon: "success", accent: "text-jn-primary-ink", label: "成功" },
  danger: { icon: "danger", accent: "text-jn-danger", label: "错误" },
  info: { icon: "info", accent: "text-jn-info", label: "提示" },
} as const satisfies Record<string, { icon: IconName; accent: string; label: string }>

export type ToastTone = keyof typeof TONE

/** 自动消失的时长。错误不走这条（见模块注释）。 */
const AUTO_DISMISS_MS = 4000

interface ToastItem {
  id: number
  tone: ToastTone
  title: string
  description?: string
}

interface ToastApi {
  /** 推一条通知。重复调用会各占一行，不做去重——同一句话在两条不同的
   *  流程里各出现一次时，去重会让用户以为只发生了一次。 */
  push: (toast: { tone?: ToastTone; title: string; description?: string }) => void
}

const ToastContext = createContext<ToastApi | null>(null)

/** 推通知的入口。没有 Provider 时返回空实现并告警，不抛错——
 *  组件被单独渲染（测试、预览）时不该整个炸掉。 */
export function useToast(): ToastApi {
  const api = useContext(ToastContext)
  if (api) return api
  return {
    push: (toast) => {
      if (import.meta.env.DEV) {
        console.warn("[Toast] 缺少 ToastProvider，通知未显示：", toast.title)
      }
    },
  }
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([])
  const nextId = useRef(1)
  // 定时器挂在 ref 上：组件卸载时统一清掉，避免更新已卸载组件的状态
  const timers = useRef(new Map<number, ReturnType<typeof setTimeout>>())

  const dismiss = useCallback((id: number) => {
    const timer = timers.current.get(id)
    if (timer) {
      clearTimeout(timer)
      timers.current.delete(id)
    }
    setItems((list) => list.filter((item) => item.id !== id))
  }, [])

  const push = useCallback<ToastApi["push"]>(
    ({ tone = "info", title, description }) => {
      const id = nextId.current++
      setItems((list) => [...list, { id, tone, title, description }])
      if (tone !== "danger") {
        timers.current.set(
          id,
          setTimeout(() => dismiss(id), AUTO_DISMISS_MS),
        )
      }
    },
    [dismiss],
  )

  const api = useMemo(() => ({ push }), [push])

  return (
    <ToastContext.Provider value={api}>
      {children}
      <div className="pointer-events-none fixed right-4 bottom-4 z-[200] flex w-[340px] flex-col gap-2">
        {items.map((item) => (
          <ToastCard
            key={item.id}
            item={item}
            // 错误必须手动关：只有 danger 渲染关闭按钮，且不挂自动消失定时器
            onDismiss={() => dismiss(item.id)}
            dismissible={item.tone === "danger"}
          />
        ))}
      </div>
    </ToastContext.Provider>
  )
}

function ToastCard({
  item,
  onDismiss,
  dismissible,
}: {
  item: ToastItem
  onDismiss: () => void
  dismissible: boolean
}) {
  const tone = TONE[item.tone]
  return (
    <div
      role="status"
      className="pointer-events-auto flex w-full items-start gap-3 rounded-[18px] border border-jn-line bg-jn-surface p-3.5 shadow-jn-surface"
    >
      <Icon name={tone.icon} size="md" className={cn("mt-0.5 shrink-0", tone.accent)} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-[13px] font-semibold text-jn-text">{item.title}</span>
        {item.description && (
          <span className="text-[11px] leading-relaxed text-jn-text-3">{item.description}</span>
        )}
      </div>
      {/* 只有错误给关闭按钮：成功的 4 秒后自己走，多一个叉会让人以为必须点 */}
      {dismissible && <IconButton icon="close" label="关闭通知" size="sm" onClick={onDismiss} />}
    </div>
  )
}
