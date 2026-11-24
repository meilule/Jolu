/**
 * 动效封装层 —— 所有新界面与 `ui/` 组件的唯一动效实现。
 *
 * 页面与基础组件不直接 import `framer-motion`；这里统一使用
 * `LazyMotion`（domAnimation）+ `m.*` + `tokens.ts` 的参数，
 * 并通过根部 `<MotionProvider>` 接管系统偏好与 Jolu 的「减少动效」设置。
 */
import { useEffect, useRef, useState, type ComponentPropsWithoutRef, type ReactNode, type Ref } from "react"
import { AnimatePresence, LazyMotion, MotionConfig, domAnimation, m, useMotionValue, useReducedMotion, useSpring } from "framer-motion"
import { cn } from "@/lib/utils"
import { DURATION, EASE, SPRING, STAGGER, STAGGER_LIMIT } from "./tokens"

export { useMotionValue, useReducedMotion, useSpring }

/** 全局动效容器：挂在主窗口根部一次，统一接管 reducedMotion 与默认缓动。 */
export function MotionProvider({
  reduced = false,
  children,
}: {
  reduced?: boolean
  children: ReactNode
}) {
  return (
    <LazyMotion features={domAnimation}>
      <MotionConfig
        reducedMotion={reduced ? "always" : "user"}
        transition={{ duration: DURATION.pop, ease: EASE.standard }}
      >
        {children}
      </MotionConfig>
    </LazyMotion>
  )
}

/**
 * 共享布局滑块：页签、分段栏、侧栏导航的选中底板共用。
 * 选中项切换时以 `SPRING.snappy` 弹簧滑到目标项位置与宽度。
 */
export function SlideIndicator({
  layoutId,
  className,
}: {
  layoutId: string
  className?: string
}) {
  return (
    <m.span
      layoutId={layoutId}
      aria-hidden
      transition={SPRING.snappy}
      className={cn("pointer-events-none absolute inset-0 rounded-[inherit]", className)}
    />
  )
}

/**
 * 开关弹簧滑块：使用 layout 过渡，保证打断重按时保留速度。
 */
export function SpringThumb({ className }: { className?: string }) {
  return <m.span layout aria-hidden transition={SPRING.snappy} className={className} />
}

/**
 * 区块入场：默认纯淡入（`y = 0` 不触发 transform，避免影响内部 fixed 弹窗参照）；
 * 卡片、列表项需要轻浮感时传 `y = 6`。
 */
export function Reveal({
  children,
  className,
  y = 0,
  delay = 0,
}: {
  children: ReactNode
  className?: string
  y?: number
  delay?: number
}) {
  return (
    <m.div
      initial={y ? { opacity: 0, y } : { opacity: 0 }}
      animate={y ? { opacity: 1, y: 0 } : { opacity: 1 }}
      transition={{ duration: DURATION.dialog, ease: EASE.standard, delay }}
      className={className}
    >
      {children}
    </m.div>
  )
}

/**
 * 列表错开入场容器。超过 STAGGER_LIMIT（30 项）时取消错开，整体淡入。
 */
export function Stagger({
  children,
  count,
  className,
}: {
  children: ReactNode
  count?: number
  className?: string
}) {
  const stagger = count !== undefined && count > STAGGER_LIMIT ? 0 : STAGGER
  return (
    <m.div
      initial="hidden"
      animate="visible"
      variants={{
        hidden: {},
        visible: { transition: { staggerChildren: stagger } },
      }}
      className={className}
    >
      {children}
    </m.div>
  )
}

export function StaggerItem({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <m.div
      variants={{
        hidden: { opacity: 0, y: 6 },
        visible: { opacity: 1, y: 0, transition: { duration: DURATION.dialog, ease: EASE.standard } },
      }}
      className={className}
    >
      {children}
    </m.div>
  )
}

/**
 * 平滑折叠展开容器（高级设置、手风琴、筛选展开等）。
 */
export function Collapse({
  open,
  children,
  className,
}: {
  open: boolean
  children: ReactNode
  className?: string
}) {
  return (
    <AnimatePresence initial={false}>
      {open && (
        <m.div
          key="collapse"
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={{ duration: DURATION.slide, ease: EASE.standard }}
          className={cn("overflow-hidden", className)}
        >
          {children}
        </m.div>
      )}
    </AnimatePresence>
  )
}

/** 弹窗遮罩淡入淡出（配合 Dialog + AnimatePresence）。 */
export function DialogBackdrop({
  className,
  ref,
  ...rest
}: {
  className?: string
  ref?: Ref<HTMLDivElement>
} & Omit<ComponentPropsWithoutRef<typeof m.div>, "className" | "ref">) {
  return (
    <m.div
      ref={ref}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: DURATION.pop, ease: EASE.standard }}
      className={className}
      {...rest}
    />
  )
}

/** 弹窗本体进出场：微缩放 + 轻量上浮，退场更快。 */
export function DialogPanel({
  children,
  className,
  ref,
  ...rest
}: {
  children?: ReactNode
  className?: string
  ref?: Ref<HTMLDivElement>
} & Omit<ComponentPropsWithoutRef<typeof m.div>, "className" | "children" | "ref">) {
  return (
    <m.div
      ref={ref}
      initial={{ opacity: 0, scale: 0.96, y: 8 }}
      animate={{ opacity: 1, scale: 1, y: 0 }}
      exit={{
        opacity: 0,
        scale: 0.97,
        y: 4,
        transition: { duration: DURATION.popOut, ease: EASE.exit },
      }}
      transition={{ duration: DURATION.dialog, ease: EASE.standard }}
      className={className}
      {...rest}
    >
      {children}
    </m.div>
  )
}

/** AnimatePresence 薄封装，给需要退场动画的容器（Dialog / Toast / 抽屉）使用。 */
export function Presence({
  children,
  mode = "sync",
}: {
  children: ReactNode
  mode?: "sync" | "wait" | "popLayout"
}) {
  return <AnimatePresence mode={mode}>{children}</AnimatePresence>
}

/**
 * 指标数字滚动（R1-00）：数值变化时平滑滚到新值，单位不动；
 * 首次挂载直接落位，减少动效开启时直接替换。
 */
export function AnimatedNumber({
  value,
  format,
  className,
}: {
  value: number
  format: (value: number) => string
  className?: string
}) {
  const reduced = useReducedMotion()
  const [shown, setShown] = useState(value)
  const currentRef = useRef(value)

  useEffect(() => {
    if (!Number.isFinite(value) || !Number.isFinite(currentRef.current)) {
      currentRef.current = value
      setShown(value)
      return
    }
    const htmlReduced = typeof document !== "undefined" && document.documentElement.classList.contains("reduce-motion")
    if (reduced || htmlReduced || currentRef.current === value) {
      currentRef.current = value
      setShown(value)
      return
    }
    const from = currentRef.current
    const delta = value - from
    const start = performance.now()
    const duration = 300
    let raf = 0
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / duration)
      const eased = 1 - Math.pow(1 - t, 3)
      const next = t >= 1 ? value : from + delta * eased
      currentRef.current = next
      setShown(next)
      if (t < 1) raf = window.requestAnimationFrame(step)
    }
    raf = window.requestAnimationFrame(step)
    return () => window.cancelAnimationFrame(raf)
  }, [value, reduced])

  return <span className={className}>{format(shown)}</span>
}
