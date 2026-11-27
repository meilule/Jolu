/**
 * 动效封装层 —— 所有新界面与 `ui/` 组件的唯一动效实现。
 *
 * 页面与基础组件不直接 import `framer-motion`；这里统一使用
 * `LazyMotion`（domMax，含 layout/layoutId 投影）+ `m.*` + `tokens.ts` 的参数，
 * 并通过根部 `<MotionProvider>` 接管系统偏好与 Jolu 的「减少动效」设置。
 */
import { forwardRef, useEffect, useRef, useState, type ButtonHTMLAttributes, type ComponentPropsWithoutRef, type HTMLAttributes, type ReactNode, type Ref } from "react"
import { AnimatePresence, LazyMotion, MotionConfig, animate, domMax, m, useMotionValue, useReducedMotion, useSpring, useTransform, useVelocity, type HTMLMotionProps } from "framer-motion"
import { cn } from "@/lib/utils"
import { DURATION, EASE, SPRING, STAGGER, STAGGER_LIMIT, TACTILE, type TactileVariant } from "./tokens"

export { animate, m, useMotionValue, useReducedMotion, useSpring, useTransform, useVelocity }

/**
 * 同时尊重系统 `prefers-reduced-motion`、根部 `<MotionProvider>`，
 * 以及主窗口设置页挂在 `<html class="reduce-motion">` 的用户偏好。
 *
 * 为什么不能只靠 Framer 的 `<MotionConfig>`：
 * `animate(motionValue, ...)` 是命令式 API，不经过 React Context 读取 `<MotionConfig>`，
 * 且预览路由 / 独立节点不一定总是在 `<MotionProvider>` 内。这里双轨监听确保任何情况下都不会漏掉减少动效。
 */
export function useEffectiveReducedMotion(): boolean {
  const framerReduced = Boolean(useReducedMotion())
  const [classReduced, setClassReduced] = useState(() =>
    typeof document !== "undefined" && document.documentElement.classList.contains("reduce-motion"),
  )

  useEffect(() => {
    if (typeof document === "undefined") return
    const root = document.documentElement
    const sync = () => setClassReduced(root.classList.contains("reduce-motion"))
    sync()
    const observer = new MutationObserver(sync)
    observer.observe(root, { attributes: true, attributeFilter: ["class"] })
    return () => observer.disconnect()
  }, [])

  return framerReduced || classReduced
}

/** 全局动效容器：挂在主窗口根部一次，统一接管 reducedMotion 与默认缓动。 */
export function MotionProvider({
  reduced = false,
  children,
}: {
  reduced?: boolean
  children: ReactNode
}) {
  return (
    <LazyMotion features={domMax}>
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
 * 传入 `radius` 以内联写入 `borderRadius`，确保宽窄不等切换时 Framer Motion 反向校正圆角不拉伸。
 */
export function SlideIndicator({
  layoutId,
  radius,
  className,
}: {
  layoutId: string
  radius?: number
  className?: string
}) {
  return (
    <m.span
      layoutId={layoutId}
      aria-hidden
      transition={SPRING.snappy}
      style={radius !== undefined ? { borderRadius: radius } : undefined}
      className={cn("pointer-events-none absolute inset-0", radius === undefined && "rounded-[inherit]", className)}
    />
  )
}

/**
 * 开关弹簧滑块：纯水平位移弹簧（不依赖 layout 测量），避免页面滚动或邻近折叠重排引起竖向跳动。
 */
export function SpringThumb({ checked = false, travel = 18, className }: { checked?: boolean; travel?: number; className?: string }) {
  return (
    <m.span
      aria-hidden
      initial={false}
      animate={{ x: checked ? travel : 0 }}
      transition={SPRING.snappy}
      className={className}
    />
  )
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

/** AnimatePresence 薄封装，给需要退场动画的容器（Dialog / Toast / 抽屉 / 勾选态）使用。 */
export function Presence({
  children,
  mode = "sync",
  initial = true,
}: {
  children: ReactNode
  mode?: "sync" | "wait" | "popLayout"
  initial?: boolean
}) {
  return (
    <AnimatePresence mode={mode} initial={initial}>
      {children}
    </AnimatePresence>
  )
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

export interface TactileButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** 触感档位（按钮 / 图标 / 选项块 / 控件 / 行 / 花瓣主开关） */
  variant?: TactileVariant
}

/**
 * 统一触感弹簧按钮：用 Motion 弹簧驱动悬停与按压弹性，完全保留原生 `<button>` 属性、
 * `forwardRef`（支持 Radix `asChild`）与无障碍语义。
 *
 * 注意两点工程细节：
 * 1. Framer Motion 默认仍会在原生 `disabled` 按钮上触发手势，这里显式在禁用/旋转态屏蔽 `whileHover` / `whileTap`；
 * 2. 配合 `.jn-interactive.jn-spring` 屏蔽 CSS 的 `transform` transition 与 `:active scale(0.97)`，防止两套缩放互相重插值发糊。
 */
export const TactileButton = forwardRef<HTMLButtonElement, TactileButtonProps>(function TactileButton(
  { variant = "button", disabled, className, children, ...rest },
  ref,
) {
  const reduced = useEffectiveReducedMotion()
  const spec = TACTILE[variant]
  const inert =
    Boolean(disabled) ||
    rest["aria-disabled"] === true ||
    rest["aria-disabled"] === "true" ||
    /\banimate-spin\b/.test(className ?? "")

  return (
    <m.button
      ref={ref}
      disabled={disabled}
      whileHover={reduced || inert ? undefined : spec.hover}
      whileTap={reduced || inert ? undefined : spec.tap}
      transition={reduced ? { duration: 0 } : spec.transition}
      className={cn(className, "jn-spring")}
      {...(rest as Omit<HTMLMotionProps<"button">, "ref" | "disabled" | "className" | "whileHover" | "whileTap" | "transition">)}
    >
      {children}
    </m.button>
  )
})

export interface TactileBoxProps extends HTMLAttributes<HTMLDivElement> {
  /** 仅当容器处于可交互态时触发按压与悬停弹簧 */
  interactive?: boolean
  variant?: TactileVariant
}

/** 可点击容器（ListRow / MetricCard / TableRow）的弹簧外壳；非交互态退化为普通零开销 `<div>`。 */
export const TactileBox = forwardRef<HTMLDivElement, TactileBoxProps>(function TactileBox(
  { interactive = false, variant = "row", className, children, ...rest },
  ref,
) {
  const reduced = useEffectiveReducedMotion()
  if (!interactive) {
    return (
      <div ref={ref} className={className} {...rest}>
        {children}
      </div>
    )
  }

  const spec = TACTILE[variant]
  const inert = rest["aria-disabled"] === true || rest["aria-disabled"] === "true"

  return (
    <m.div
      ref={ref}
      whileHover={reduced || inert ? undefined : spec.hover}
      whileTap={reduced || inert ? undefined : spec.tap}
      transition={reduced ? { duration: 0 } : spec.transition}
      className={cn(className, "jn-spring")}
      {...(rest as Omit<HTMLMotionProps<"div">, "ref" | "className" | "whileHover" | "whileTap" | "transition">)}
    >
      {children}
    </m.div>
  )
})
