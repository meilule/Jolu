import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react"
import { cn } from "@/lib/utils"
import "./rubber-segment.css"

/**
 * RubberSegment —— 带「橡皮滑块」的分段控件（React Bits RubberSegment 的零依赖移植）。
 *
 * 与普通 tab 高亮的区别：滑块是**一个连续运动的实体**。它比单纯位移多了三件事：
 *
 *   1. 拉伸（stretch）：点新槽位时，滑块先向两侧膨胀到覆盖「旧槽 + 新槽」，
 *      再收缩到新槽。视觉上是橡皮被拽过去，而不是两个色块交换。
 *   2. 压缩（squash）：落位时后缘多冲出去几像素再弹回，给落点一点重量。
 *   3. 拖拽甩动（draggable + glide）：按住滑块横向拖，超出两端时按橡胶阻尼
 *      回弹；快速甩动会带着惯性落到更远的槽位。
 *
 * 上游用 `motion` 的 spring + motionValue 实现。这里用 rAF + 临界阻尼弹簧复刻，
 * 避免为一个控件引入 ~40KB 依赖（项目此前已把 hugeicons 换成已有的 lucide）。
 * 物理参数按上游同名常量对齐：DILATE / HANDOFF / FLICK / DEADZONE / SLOP / RUBBER。
 */

const EASE_OUT = "cubic-bezier(0.23, 1, 0.32, 1)"
/** 拉伸阶段时长（占整个切换的比例） */
const DILATE = 0.19
/** 拉伸完成到开始收缩的交接延迟（占整个切换的比例） */
const HANDOFF = 0.15
/** 判定为「甩动」的最小速度（px/s） */
const FLICK = 110
const MAX_VELOCITY = 2000
/** 按下后横向移动超过这个距离才算拖拽，否则视为点击 */
const DEADZONE = 4
/** 抬起时位移不超过这个值才算点击 */
const SLOP = 10
/** 橡胶阻尼系数：超出边界时实际位移 = over * dim * RUBBER / (dim + RUBBER * |over|) */
const RUBBER = 0.55

const SIZES: Record<string, { height: number; font: number; pad: number; min: number }> = {
  sm: { height: 28, font: 12, pad: 10, min: 36 },
  md: { height: 36, font: 13, pad: 14, min: 44 },
  lg: { height: 44, font: 14, pad: 18, min: 48 },
}

type Slot = { l: number; r: number }

type DragState = {
  id: number
  x0: number
  slot: number
  onThumb: boolean
  live: boolean
  offset: number
  w: number
  hist: [number, number][]
}

const clamp = (v: number, min: number, max: number) => Math.min(max, Math.max(min, v))
/** 橡胶阻尼：越界距离 over，容器尺寸 dim → 实际呈现的位移 */
const rubber = (over: number, dim: number) => (over * dim * RUBBER) / (dim + RUBBER * Math.abs(over))
/** 把速度投影成「还会滑多远」（指数衰减的落点估计） */
const project = (v: number, glide: number) => {
  const d = 1 - 0.1 * Math.pow(0.05, glide / 100)
  return (v / 1000) * d / (1 - d)
}
/** 用最近 100ms 的采样估速度（px/s） */
const velocityOf = (hist: [number, number][], now: number) => {
  const recent = hist.filter(([t]) => now - t <= 100)
  if (recent.length < 2) return 0
  const [t0, x0] = recent[0]
  const [t1, x1] = recent[recent.length - 1]
  return t1 - t0 >= 8 ? ((x1 - x0) / (t1 - t0)) * 1000 : 0
}
const nearestSlot = (slots: Slot[], x: number) => {
  let best = 0
  for (let i = 1; i < slots.length; i++) {
    if (Math.abs((slots[i].l + slots[i].r) / 2 - x) < Math.abs((slots[best].l + slots[best].r) / 2 - x)) best = i
  }
  return best
}

export type RubberSegmentItem = string | { value: string; label: ReactNode; icon?: ReactNode }

export interface RubberSegmentProps {
  items: RubberSegmentItem[]
  /** 受控值。外部改动直接跳过去，不播动画 */
  value?: string
  defaultValue?: string
  onChange?: (value: string, index: number) => void
  trackColor?: string
  thumbColor?: string
  textColor?: string
  activeTextColor?: string
  size?: "sm" | "md" | "lg"
  radius?: number
  inset?: number
  /** 每个槽等宽；关掉则按内容贴边，滑块随槽变宽 */
  equalSlots?: boolean
  /** 拉伸幅度：100 为标准，0 退化成普通滑动 */
  stretch?: number
  /** 落位时后缘冲出的像素数，0 去掉压缩 */
  squash?: number
  /** 全局速度倍率，0.25 是慢放 */
  speed?: number
  /** 甩动惯性：越大滑得越远，0 永远落到最近槽 */
  glide?: number
  draggable?: boolean
  disabled?: boolean
  className?: string
  "aria-label"?: string
}

export function RubberSegment({
  items,
  value,
  defaultValue,
  onChange,
  trackColor = "var(--jolu-stage)",
  thumbColor = "var(--jolu-bg)",
  textColor = "var(--jolu-muted)",
  activeTextColor = "var(--jolu-on)",
  size = "md",
  radius = 10,
  inset = 3,
  equalSlots = true,
  stretch = 100,
  squash = 3,
  speed = 1,
  glide = 75,
  draggable = true,
  disabled = false,
  className = "",
  "aria-label": ariaLabel = "Segmented control",
}: RubberSegmentProps) {
  const list = items.map((item) => (typeof item === "string" ? { value: item, label: item } : item))
  const [inner, setInner] = useState(defaultValue ?? list[0]?.value)
  const current = value !== undefined ? value : inner
  const index = Math.max(0, list.findIndex((item) => item.value === current))
  const reduce = usePrefersReducedMotion()

  const trackRef = useRef<HTMLDivElement>(null)
  const itemRefs = useRef<(HTMLButtonElement | null)[]>([])
  const slots = useRef<Slot[]>([])
  const box = useRef<DOMRect | null>(null)
  const committed = useRef(index)
  const handoff = useRef<number>(0)
  const drag = useRef<DragState | null>(null)
  const gen = useRef(0)
  const raf = useRef(0)

  const [edges, setEdges] = useState({ l: 0, r: 0 })
  /** 轨道内宽（给滑块内的文字副本铺满用） */
  const [trackW, setTrackW] = useState(0)
  /** 首次测量完成前不渲染滑块（否则会看到一个从 0 宽滑出来的过程） */
  const [ready, setReady] = useState(false)
  const innerW = useRef(0)
  const animating = useRef(false)

  const thumbRadius = Math.max(0, radius - inset)
  const preset = SIZES[size] ?? SIZES.md
  const t = (seconds: number) => seconds / speed

  /** 直接落位（不动画）。打开、受控值外部变化、减少动效时用 */
  const jumpTo = useCallback((i: number) => {
    const s = slots.current[i]
    if (!s) return
    window.clearTimeout(handoff.current)
    gen.current += 1
    animating.current = false
    cancelAnimationFrame(raf.current)
    // 首帧/受控值外部变化：直接落位，不要从 0 滑过来
    setEdges({ l: s.l, r: s.r })
  }, [])

  const measure = useCallback(() => {
    const track = trackRef.current
    if (!track) return
    const rect = track.getBoundingClientRect()
    box.current = rect
    // item 的 rect 相对 track 外框；滑块绝对定位在 padding box 内（top/left = inset），
    // 所以这里要减掉 inset 才是滑块坐标系里的位置。
    slots.current = list.map((_, i) => {
      const el = itemRefs.current[i]
      if (!el) return { l: 0, r: 0 }
      const r = el.getBoundingClientRect()
      return { l: r.left - rect.left - inset, r: r.right - rect.left - inset }
    })
    innerW.current = rect.width - inset * 2
    setTrackW(innerW.current)
    jumpTo(committed.current)
    setReady(true)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inset, jumpTo])

  const listKey = list.map((item) => item.value).join("|")
  useLayoutEffect(() => {
    measure()
    const observer = new ResizeObserver(measure)
    if (trackRef.current) observer.observe(trackRef.current)
    if (typeof document !== "undefined" && document.fonts) void document.fonts.ready.then(measure)
    return () => observer.disconnect()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [listKey, size, inset, equalSlots])

  /** 受控值从外部变了（且不是自己拖的）：直接跳过去 */
  useEffect(() => {
    if (!drag.current && committed.current !== index) {
      committed.current = index
      jumpTo(index)
    }
  }, [index, jumpTo])

  useEffect(() => () => {
    window.clearTimeout(handoff.current)
    cancelAnimationFrame(raf.current)
  }, [])

  const commit = (i: number) => {
    committed.current = i
    if (i === index) return
    if (value === undefined) setInner(list[i].value)
    onChange?.(list[i].value, i)
  }

  /**
   * 弹簧逼近。用临界阻尼的离散形式：
   *   v += (target - x) * k - v * d
   *   x += v
   * 比 CSS transition 好在可以带初速度（甩动）与中途改目标（拖拽交接）。
   */
  const springTo = (
    from: { l: number; r: number },
    to: { l: number; r: number },
    opts: { duration: number; velocity?: number; bounce?: number; onDone?: () => void },
  ) => {
    const g = ++gen.current
    const start = performance.now()
    const dur = t(opts.duration) * 1000
    const k = 1 - Math.exp(-6.9 / (dur / 16.67)) // 让 1/k 帧内收敛到 ~99.9%
    const d = 0.62 - (opts.bounce ?? 0) * 0.3
    const state = { l: from.l, r: from.r, vl: (opts.velocity ?? 0) * 0.016, vr: 0 }
    animating.current = true

    const step = () => {
      if (gen.current !== g) return
      const elapsed = performance.now() - start
      state.vl += (to.l - state.l) * k - state.vl * d
      state.vr += (to.r - state.r) * k - state.vr * d
      state.l += state.vl
      state.r += state.vr
      const settled = Math.abs(to.l - state.l) < 0.4 && Math.abs(to.r - state.r) < 0.4 &&
        Math.abs(state.vl) < 0.4 && Math.abs(state.vr) < 0.4
      if (settled || elapsed > dur * 2.2) {
        setEdges({ l: to.l, r: to.r })
        animating.current = false
        opts.onDone?.()
        return
      }
      setEdges({ l: state.l, r: state.r })
      raf.current = requestAnimationFrame(step)
    }
    raf.current = requestAnimationFrame(step)
  }

  /** 落位：前缘奔向目标，后缘按 squash 冲出再回弹 */
  const land = (to: number, velocity: number | null, flick: boolean, withSquash: boolean) => {
    const b = slots.current[to]
    if (!b) return
    const dir = Math.sign((b.l + b.r) / 2 - (edges.l + edges.r) / 2) || 1
    const leadingRight = dir > 0
    const from = { ...edges }
    const overshoot = withSquash && squash > 0 ? dir * squash : 0

    // 后缘先冲到目标 + overshoot，再单独回正
    const firstTarget = leadingRight
      ? { l: b.l, r: b.r + overshoot }
      : { l: b.l + overshoot, r: b.r }

    springTo(from, firstTarget, {
      duration: flick ? 0.4 : 0.3,
      bounce: flick ? 0.2 : 0,
      velocity: velocity === null ? undefined : clamp(velocity, -MAX_VELOCITY, MAX_VELOCITY),
      onDone: () => {
        if (!overshoot) return
        springTo({ l: b.l, r: b.r + overshoot }, { l: b.l, r: b.r }, { duration: 0.16, bounce: 0 })
      },
    })
  }

  /** 拉伸 → 交接 → 落位（点选时走这条） */
  const travel = (from: number, to: number) => {
    const a = slots.current[from]
    const b = slots.current[to]
    if (!a || !b) return
    window.clearTimeout(handoff.current)
    gen.current += 1
    if (reduce) {
      setEdges({ l: b.l, r: b.r })
      return
    }
    const u = stretch / 100
    // 第一段：膨胀到覆盖「旧槽 ∪ 新槽」
    const dilateTo = {
      l: b.l + (Math.min(a.l, b.l) - b.l) * u,
      r: b.r + (Math.max(a.r, b.r) - b.r) * u,
    }
    springTo({ ...edges }, dilateTo, {
      duration: DILATE,
      onDone: () => {
        handoff.current = window.setTimeout(() => {
          land(to, null, false, true)
        }, t(HANDOFF) * 1000)
      },
    })
  }

  const localX = (e: React.PointerEvent) => e.clientX - (box.current?.left ?? 0) - inset

  const handlePointerDown = (e: React.PointerEvent, i: number) => {
    if (disabled || drag.current || e.button !== 0) return
    box.current = trackRef.current!.getBoundingClientRect()
    try {
      e.currentTarget.setPointerCapture(e.pointerId)
    } catch { /* 合成事件没有活动指针 */ }
    const x = localX(e)
    const onThumb = draggable && x >= edges.l && x <= edges.r
    drag.current = { id: e.pointerId, x0: x, slot: i, onThumb, live: false, offset: 0, w: 0, hist: [[e.timeStamp, x]] }
    if (onThumb) {
      window.clearTimeout(handoff.current)
      gen.current += 1
      cancelAnimationFrame(raf.current)
      animating.current = false
    } else if (!reduce) {
      ;(e.currentTarget as HTMLElement).dataset.pressed = ""
    }
  }

  const handlePointerMove = (e: React.PointerEvent) => {
    const d = drag.current
    if (!d || e.pointerId !== d.id || !d.onThumb) return
    const x = localX(e)
    d.hist.push([e.timeStamp, x])
    if (d.hist.length > 8) d.hist.shift()
    if (!d.live) {
      if (Math.abs(x - d.x0) < DEADZONE) return
      d.live = true
      d.offset = x - edges.l
      d.w = edges.r - edges.l
      if (trackRef.current) trackRef.current.dataset.held = ""
    }
    const width = innerW.current
    const l = x - d.offset
    const maxL = width - d.w
    if (reduce) {
      const c = clamp(l, 0, maxL)
      setEdges({ l: c, r: c + d.w })
    } else if (l < 0) {
      // 左越界：橡胶阻尼
      setEdges({ l: 0, r: d.w - rubber(-l, d.w) })
    } else if (l > maxL) {
      setEdges({ l: maxL + rubber(l - maxL, d.w), r: width })
    } else {
      setEdges({ l, r: l + d.w })
    }
  }

  const release = () => {
    const d = drag.current
    drag.current = null
    if (trackRef.current) delete trackRef.current.dataset.held
    const el = itemRefs.current[d!.slot]
    if (el) delete el.dataset.pressed
    return d!
  }

  const handlePointerUp = (e: React.PointerEvent) => {
    const d = drag.current
    if (!d || e.pointerId !== d.id) return
    release()
    const x = localX(e)
    if (!d.live) {
      // 没拖动：当点击处理
      if (Math.abs(x - d.x0) <= SLOP && d.slot !== committed.current) {
        const from = committed.current
        commit(d.slot)
        travel(from, d.slot)
      }
      return
    }
    // 拖完：按速度投影落点
    const v = velocityOf(d.hist, e.timeStamp)
    const flick = Math.abs(v) > FLICK
    let to = nearestSlot(slots.current, (edges.l + edges.r) / 2 + project(v, glide))
    if (flick && to === committed.current) to = clamp(to + Math.sign(v), 0, list.length - 1)
    commit(to)
    if (reduce) jumpTo(to)
    else land(to, v, flick, flick)
  }

  const handlePointerCancel = (e: React.PointerEvent) => {
    const d = drag.current
    if (!d || e.pointerId !== d.id) return
    release()
    if (!d.live) return
    if (reduce) jumpTo(committed.current)
    else land(committed.current, null, false, false)
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (disabled) return
    const last = list.length - 1
    let next: number | null = null
    if (e.key === "ArrowRight" || e.key === "ArrowDown") next = Math.min(last, index + 1)
    else if (e.key === "ArrowLeft" || e.key === "ArrowUp") next = Math.max(0, index - 1)
    else if (e.key === "Home") next = 0
    else if (e.key === "End") next = last
    if (next === null) return
    e.preventDefault()
    if (next === index) return
    commit(next)
    jumpTo(next)
    itemRefs.current[next]?.focus()
  }

  const thumbW = Math.max(0, edges.r - edges.l)

  return (
    <div
      ref={trackRef}
      role="radiogroup"
      aria-label={ariaLabel}
      aria-disabled={disabled || undefined}
      data-equal={equalSlots ? "" : undefined}
      data-draggable={draggable && !disabled ? "" : undefined}
      data-animating={animating.current ? "" : undefined}
      className={cn("rubber-segment", className)}
      style={{
        ["--rs-track" as string]: trackColor,
        ["--rs-thumb" as string]: thumbColor,
        ["--rs-ink" as string]: textColor,
        ["--rs-ink-active" as string]: activeTextColor,
        ["--rs-radius" as string]: `${radius}px`,
        ["--rs-inset" as string]: `${inset}px`,
        ["--rs-thumb-radius" as string]: `${thumbRadius}px`,
        ["--rs-h" as string]: `${preset.height}px`,
        ["--rs-font" as string]: `${preset.font}px`,
        ["--rs-pad" as string]: `${preset.pad}px`,
        ["--rs-min" as string]: `${preset.min}px`,
        // 滑块位置由 rAF 逐帧写入这两个变量；不用内联 style 是因为
        // 滑块是 React 渲染的节点，重渲染会把内联样式清掉（同 SelectField 的教训）
        ["--rs-l" as string]: `${edges.l}px`,
        ["--rs-w" as string]: `${thumbW}px`,
        ["--rs-transition" as string]: animating.current ? "none" : `left 160ms ${EASE_OUT}, width 160ms ${EASE_OUT}`,
        ["--rs-track-w" as string]: `${trackW}px`,
      }}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerCancel={handlePointerCancel}
      onLostPointerCapture={handlePointerCancel}
    >
      {list.map((item, i) => (
        <button
          key={item.value}
          ref={(el) => { itemRefs.current[i] = el }}
          type="button"
          role="radio"
          aria-checked={i === index}
          tabIndex={i === index ? 0 : -1}
          disabled={disabled}
          className="rubber-segment__item"
          onPointerDown={(e) => handlePointerDown(e, i)}
          onKeyDown={handleKeyDown}
        >
          {"icon" in item ? item.icon : null}
          {item.label}
        </button>
      ))}
      {/* 滑块：内部复刻一份文字，用 clip-path 裁出「被滑块覆盖的那部分」，
          这样滑块移动时文字颜色是「从滑块里透出来」而不是整体变色 */}
      <div className="rubber-segment__thumb" aria-hidden="true" data-ready={ready ? "" : undefined}>
        <div className="rubber-segment__copy-row">
          {list.map((item) => (
            <span key={item.value} className="rubber-segment__item rubber-segment__copy">
              {"icon" in item ? item.icon : null}
              {item.label}
            </span>
          ))}
        </div>
      </div>
    </div>
  )
}

/** 跟随系统的「减少动效」偏好 */
function usePrefersReducedMotion() {
  const [reduce, setReduce] = useState(() =>
    typeof window !== "undefined" && !!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches)
  useEffect(() => {
    const mq = window.matchMedia?.("(prefers-reduced-motion: reduce)")
    if (!mq) return
    const onChange = () => setReduce(mq.matches)
    mq.addEventListener("change", onChange)
    return () => mq.removeEventListener("change", onChange)
  }, [])
  return reduce
}
