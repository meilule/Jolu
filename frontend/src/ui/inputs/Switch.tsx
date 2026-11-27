/**
 * Switch —— 设计稿 R1-C「分区 · 3 · 选择与输入 / 开关 Switch」。
 *
 * 外观按设计稿 46×28、圆角 14、滑块 22×22、四周 3px 留白（1px 描边 + 2px 内边距），
 * 另有 `size="sm"`（28×16 / 12×12）供行内紧凑位（如模型路由账号卡）。
 *
 * 交互与物理动效移植自 Bencho Liquid toggle（MIT，bencho.dev/licence）：
 * - **NO GOO**：单圆不再套 SVG `feGaussianBlur + alpha threshold` 滤镜。原版作者在源码中
 *   拆掉拖尾小圆并指明：单圆经过 9px 模糊再锐化只会在每一帧把边缘磨糊（"why does the
 *   dot look low definition"）。纯 DOM 圆在任意缩放比下都由浏览器按设备分辨率锐利绘制，
 *   弹性改由 CSS transform 承担。
 * - **单一位置 motion value**：指尖拖拽直接写 `x`，松手后落位弹簧 `170 / 21.5 / 0.9`
 *   （`zeta ≈ 0.87`，带轻微柔和回弹，不硬停）接管，全程零 React 重渲染。
 * - **第一次 move 才算抓点偏移（`grab = null`）**：若在 `pointerdown` 就硬算手指与滑块中心
 *   差值，点轨道远端时第一次 move 会让滑块瞬间跳到指尖下；延迟到首次 move 记录初始偏移，
 *   之后按差值跟手，点在轨道任何位置拖都不会跳。
 * - **悬停微胀放进同一个 transform**：`swell`（1 → 1.035）乘进 `scaleX / scaleY`，
 *   不用 CSS 独立 `scale` 属性——独立 `scale` 先于 `transform` 生效，会把 `translateX(openX)`
 *   一并放大导致 ON 态往右漂移。放进同构矩阵后，微胀严格绕滑块圆心发生，且拉伸保持面积守恒
 *   （`scaleY = 1 / scaleX`）。
 */
import {
  forwardRef,
  useEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type KeyboardEvent,
  type PointerEvent,
} from "react"
import { cn } from "@/lib/utils"
import {
  animate,
  clamp,
  computeLiquidScale,
  computeLocalX,
  LIQUID_DEFAULT_STRETCH,
  LIQUID_DRAG_THRESHOLD_PX,
  LIQUID_HOVER_SWELL,
  LIQUID_SWITCH_GEOMETRY,
  m,
  shouldFlipOnDrag,
  SPRING,
  useEffectiveReducedMotion,
  useMotionValue,
  useSpring,
  useTransform,
  useVelocity,
  type LiquidSwitchSize,
} from "@/motion"

export interface SwitchProps
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "type" | "onChange"> {
  checked: boolean
  onChange: (checked: boolean) => void
  /** 无障碍名。没有可见标签时必填（如表格行里的开关） */
  label?: string
  /** 尺寸：`md`（46×28，默认 R1 规格）/ `sm`（28×16，紧凑行内） */
  size?: LiquidSwitchSize
  /** 液态拉伸强度（0..100，默认 36 与 Bencho 一致） */
  stretch?: number
}

export const Switch = forwardRef<HTMLButtonElement, SwitchProps>(function Switch(
  {
    checked,
    onChange,
    label,
    size = "md",
    stretch = LIQUID_DEFAULT_STRETCH,
    className,
    disabled,
    onPointerDown,
    onPointerMove,
    onPointerUp,
    onPointerCancel,
    onPointerEnter,
    onPointerLeave,
    onKeyDown,
    onKeyUp,
    onClick,
    ...rest
  },
  forwardedRef,
) {
  const geo = LIQUID_SWITCH_GEOMETRY[size]
  const shutX = 0
  const openX = geo.travel
  const targetX = checked ? openX : shutX

  const reduced = useEffectiveReducedMotion()
  const [held, setHeld] = useState(false)
  const [hot, setHot] = useState(false)

  const rail = useRef<HTMLButtonElement | null>(null)
  const setRefs = (node: HTMLButtonElement | null) => {
    rail.current = node
    if (typeof forwardedRef === "function") forwardedRef(node)
    else if (forwardedRef) forwardedRef.current = node
  }

  // `grab: null` 等第一次 move 再记偏移，避免点击滑块外侧时第一帧瞬移跳变；
  // `syncedChecked` 记录本轮拖拽最近一次上报给受控父组件的值，防止异步父组件未立即翻转时反复触发 onChange。
  const grip = useRef<{
    id: number
    grab: number | null
    moved: boolean
    syncedChecked: boolean
  } | null>(null)
  const ignoreNextClick = useRef(false)
  const ignoreResetRaf = useRef(0)

  const x = useMotionValue<number>(targetX)
  const vel = useVelocity(x)
  const eased = useSpring(vel, SPRING.liquidFollow)
  // 注：framer-motion@14 的 `useSpring(number)` 只在挂载时绑定初始值、依赖项不含 source，
  // 传 React state 数值不会触发重绑；直接用 `animate(swell, ...)` 驱动同一个 spring 参数才能响应 `hot` 切换。
  const swell = useMotionValue<number>(1)

  const wide = useTransform([eased, swell], ([v, s]: number[]) =>
    reduced ? 1 : computeLiquidScale(v, s, openX, stretch).wide,
  )
  const tall = useTransform([eased, swell], ([v, s]: number[]) =>
    reduced ? 1 : computeLiquidScale(v, s, openX, stretch).tall,
  )

  // `disabled` 若在拖拽半途变成 true（例如触发乐观锁），立刻交还控制权给落位弹簧，避免卡死在 `held`
  useEffect(() => {
    if (!disabled) return
    grip.current = null
    setHeld(false)
    setHot(false)
  }, [disabled])

  // 悬停微胀弹簧：减少动效开启时立刻归 1，普通态用 `SPRING.liquidFollow` 丝滑过渡，打断时保留速度
  useEffect(() => {
    const next = hot && !disabled && !reduced ? LIQUID_HOVER_SWELL : 1
    if (reduced) {
      swell.jump(1)
      return
    }
    if (Math.abs(swell.get() - next) < 0.0005 && Math.abs(swell.getVelocity()) < 0.0005) {
      swell.jump(next)
      return
    }
    const run = animate(swell, next, { type: "spring", ...SPRING.liquidFollow })
    return () => run.stop()
  }, [hot, disabled, reduced, swell])

  useEffect(() => {
    return () => cancelAnimationFrame(ignoreResetRaf.current)
  }, [])

  // `held` 期间弹簧让位给手指；`stop()` 保证半途抓住滑块能立即接管。
  useEffect(() => {
    if (held) return
    if (reduced) {
      x.jump(targetX)
      return
    }
    if (Math.abs(x.get() - targetX) < 0.01 && Math.abs(x.getVelocity()) < 0.01) {
      x.jump(targetX)
      return
    }
    const run = animate(x, targetX, SPRING.liquidSettle)
    return () => run.stop()
  }, [targetX, held, reduced, x])

  const handlePointerDown = (e: PointerEvent<HTMLButtonElement>) => {
    onPointerDown?.(e)
    if (e.defaultPrevented || disabled || e.button !== 0) return
    ignoreNextClick.current = false
    cancelAnimationFrame(ignoreResetRaf.current)
    grip.current = {
      id: e.pointerId,
      grab: null,
      moved: false,
      syncedChecked: checked,
    }
    setHeld(true)
    // 合成事件没有真实指针时会抛错；拖拽本身不强依赖指针捕获，必须兜住
    try {
      rail.current?.setPointerCapture(e.pointerId)
    } catch {
      /* not a live pointer */
    }
  }

  const handlePointerMove = (e: PointerEvent<HTMLButtonElement>) => {
    onPointerMove?.(e)
    const g = grip.current
    if (!g || g.id !== e.pointerId) return
    const el = rail.current
    if (!el) return
    const rect = el.getBoundingClientRect()
    const at = computeLocalX(e.clientX, rect.left, rect.width, el.offsetWidth)
    if (g.grab === null) g.grab = at - x.get()
    const next = clamp(at - g.grab, shutX, openX)
    if (Math.abs(next - x.get()) > LIQUID_DRAG_THRESHOLD_PX) g.moved = true
    x.set(next)
    const past = shouldFlipOnDrag(next, shutX, openX)
    if (past !== g.syncedChecked) {
      g.syncedChecked = past
      onChange(past)
    }
  }

  const finishPointer = (e: PointerEvent<HTMLButtonElement>, isCancel: boolean) => {
    const g = grip.current
    if (!g || g.id !== e.pointerId) return
    grip.current = null
    // `releasePointerCapture` 在未成功捕获或已释放时同样会抛；不兜住会卡死 `held = true`
    try {
      rail.current?.releasePointerCapture?.(e.pointerId)
    } catch {
      /* never captured */
    }
    if (!isCancel) {
      // pointerup 后浏览器同帧会派发 click：这里已完成点按/拖拽结算，抑制随后的 click 防二次翻转
      ignoreNextClick.current = true
      cancelAnimationFrame(ignoreResetRaf.current)
      ignoreResetRaf.current = requestAnimationFrame(() => {
        ignoreNextClick.current = false
      })
      if (!g.moved) onChange(!checked)
    }
    setHeld(false)
  }

  const handlePointerUp = (e: PointerEvent<HTMLButtonElement>) => {
    onPointerUp?.(e)
    finishPointer(e, false)
  }

  const handlePointerCancel = (e: PointerEvent<HTMLButtonElement>) => {
    onPointerCancel?.(e)
    finishPointer(e, true)
  }

  const handleClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    onClick?.(e)
    if (ignoreNextClick.current) {
      ignoreNextClick.current = false
      cancelAnimationFrame(ignoreResetRaf.current)
      return
    }
    if (e.defaultPrevented || disabled) return
    onChange(!checked)
  }

  const handleKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    onKeyDown?.(e)
    if (e.defaultPrevented || disabled || e.repeat) return
    if (e.key !== " " && e.key !== "Enter") return
    e.preventDefault()
    onChange(!checked)
  }

  const handleKeyUp = (e: KeyboardEvent<HTMLButtonElement>) => {
    onKeyUp?.(e)
    if (e.key === " ") e.preventDefault()
  }

  return (
    <button
      ref={setRefs}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerCancel={handlePointerCancel}
      onPointerEnter={(e) => {
        onPointerEnter?.(e)
        setHot(true)
      }}
      onPointerLeave={(e) => {
        onPointerLeave?.(e)
        setHot(false)
      }}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      onKeyUp={handleKeyUp}
      className={cn(
        "relative inline-flex shrink-0 cursor-pointer touch-none select-none items-center justify-start rounded-full border transition-colors duration-[var(--motion-feedback)] ease-[var(--motion-ease-standard)] focus-visible:outline-[3px] focus-visible:outline-offset-1 focus-visible:outline-jn-focus disabled:pointer-events-none disabled:opacity-45",
        size === "sm" ? "h-4 w-7 p-px" : "h-7 w-[46px] p-[2px]",
        checked ? "border-transparent bg-jn-primary" : "border-jn-line bg-jn-track",
        className,
      )}
      {...rest}
    >
      <m.span
        aria-hidden
        style={{
          x,
          scaleX: wide,
          scaleY: tall,
          transformOrigin: "center center",
          width: geo.thumbSize,
          height: geo.thumbSize,
          willChange: "transform",
        }}
        className={cn(
          "pointer-events-none block rounded-full shadow-jn-surface transition-colors duration-[var(--motion-feedback)] ease-[var(--motion-ease-standard)]",
          size === "sm" || checked ? "bg-white" : "bg-jn-surface",
        )}
      />
    </button>
  )
})
