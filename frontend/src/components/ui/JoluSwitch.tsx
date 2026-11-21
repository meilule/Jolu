import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react"
import { animate, motion, useMotionValue } from "framer-motion"

/**
 * JoluSwitch —— 项目唯一的开关控件。
 *
 * 两种实现，由 `liquid` 决定，props / DOM 语义 / aria 完全一致：
 *
 *   · 默认（`liquid` 未传）—— 纯 CSS：track 变色 + thumb `translateX` 180ms。
 *     全项目现有调用点一行不用改。
 *   · `liquid` —— 液态动效（Liquid toggle，Bencho，MIT，bencho.dev/licence）。
 *     先在「灵动岛」设置页试用。
 *
 * ── 液态版取的是它哪个做法 ─────────────────────────────────
 * 「一个本体，靠形变表达弹性」：滑块沿移动方向**拉长**、垂直方向**收窄**，
 * 形状由它当前的位置推出来 —— 越接近轨道中点形变越大，两端回到正圆。
 *
 * 原版把早期的「拖尾 + goo 滤镜」整个删掉了，理由写在源码里：两个重叠圆靠
 * blur + alpha 阈值融合出的颈缩，在单圆上只会把边缘糊掉（"为什么这个点看起来
 * 不够清晰"）。所以这里也没有 goo filter —— 只有一个 CSS 圆，靠 transform 形变，
 * 任何缩放下都由浏览器按设备分辨率绘制，边缘永远是清晰的。
 *
 * ── 为什么位置交给 Framer ──────────────────────────────────
 * 落位要有弹簧（带一点回弹，没有硬停），CSS `transition` 只能给缓动曲线。
 * 位置写进 motion value：拖动时直接写它，松手后弹簧接管，全程不触发重渲染。
 *
 * ── 与原版的一处刻意偏离：位置驱动而非速度驱动 ─────────────
 * 原版用「滑块速度 → 伸长量」，因为它的轨道 92px、滑块 36px，快速拖动时速度
 * 信号很强。本项目滑块只有 16px、行程 16px，实测速度峰值太小，形变恒为 1
 * （调除数也救不回来）。改成位置驱动后，拖动、松手回弹、键盘切换三种路径
 * 都一定看得见形变。
 *
 * ── 手感参数 ──────────────────────────────────────────────
 * 落位弹簧 170 / 21.5（zeta≈0.87）：到位有一点点回弹，没有硬停。
 * 原版试过把 Bounce 做成可调旋钮，最后认定「开关不该有回弹刻度」，固定成常数。
 */

export interface JoluSwitchProps {
  on: boolean
  onChange?: (next: boolean) => void
  disabled?: boolean
  label: string
  compact?: boolean
  /** 液态动效（形变 + 拖动 + 弹簧）。默认关闭，逐页灰度。 */
  liquid?: boolean
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))

/* ── 几何 ──────────────────────────────────────────────────
   与 `index.css` 的 `--switch-*` 默认值一一对应（36×20 / thumb 16 / pad 2）。

   ⚠️ 滑块是**相对按钮左上角**定位的（`left: 0`），而轨道自己还有 `padding: 2px`。
   所以行程必须把这份 padding 算进来：
     · 静止位 = 轨道 padding + 轨道自身 margin（按钮比轨道宽/高时居中）
     · 行程   = trackW - thumb - 2×padding
   只写 `pad` 的话滑块会整体右移 2px，贴死在轨道右边缘 —— 这正是
   「开关看着歪」的原因。 */
const GEO = {
  /** outer: 按钮盒子；track: 轨道；thumb: 滑块；pad: 轨道内边距 */
  regular: { trackW: 36, trackH: 20, thumb: 16, pad: 2 },
  compact: { trackW: 30, trackH: 18, thumb: 14, pad: 2 },
} as const

/* ── 伸长量 ────────────────────────────────────────────────
   最宽 = 1.28 倍（16px → 20.5px，刚好填满 20px 轨道的圆角余量）。
   参考实现只拉 9%（1.09），因为它的轨道长 92px、滑块 36px，空间充裕；
   本项目的滑块只有 16px，拉太多会撞到轨道圆角。
   28% 是在「看得出液态」和「不撞边」之间取的中间值。 */
const MAX_STRETCH = 0.28
/* 高度收缩比：**故意小于宽度增幅**。
   参考实现严格面积守恒（scaleY = 1/scaleX），但它只拉 9%，高度也只收 9%，
   所以看着是"微微拉长"。若在 28% 上照搬面积守恒，16px 的球会压到 12.5px ——
   在 20px 轨道里就是一根扁条，这正是「不好看」的来源。
   收 12% 换来"被拉长的胶体"，而不是"被压扁的椭圆"。 */
const HEIGHT_SHRINK = 0.12
/** 静止时**不形变**：圆必须是正圆，否则永远是椭的，看着"歪" */
const BASE_STRETCH = 0
/** 悬停微胀：乘进 scaleX/scaleY，不是 CSS `scale` 属性（见下方注释） */
const HOVER_SWELL = 1.05

/** 默认实现：纯 CSS，与改造前逐字一致 */
function PlainSwitch({ on, onChange, disabled, label, compact = false }: JoluSwitchProps) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label}
      disabled={disabled || !onChange} onClick={() => onChange?.(!on)}
      data-size={compact ? "compact" : "regular"} className="jolu-switch">
      <span aria-hidden className="jolu-switch-track">
        <span className="jolu-switch-thumb" />
      </span>
    </button>
  )
}

function LiquidSwitch({ on, onChange, disabled, label, compact = false }: JoluSwitchProps) {
  const geo = compact ? GEO.compact : GEO.regular
  const { trackW, trackH, thumb, pad } = geo

  /* ── 行程两端：translateX 的 0 点是滑块在轨道内的**流内位置** ──────────
     滑块是 `display:block`，坐在轨道的 `padding: 2px` 里 —— 也就是说
     translateX(0) 时它已经在轨道左边内缘了。所以：
       静止位 = 0（不用再加 pad）
       终点   = 轨道宽度 - 滑块 - 两侧 padding
     之前写成 `trackW - thumb - pad` 漏了一侧的 padding，滑块会顶到右边缘外 2px
     —— 这就是「开关看着歪、滑块贴死在右端」的来源。 */
  const shutX = 0
  const openX = trackW - thumb - pad * 2
  /** 过半即翻：拖动时轨道跟着手指走，不用等松手 */
  const midX = (shutX + openX) / 2

  /** 位置：一个 motion value。拖动直接写它，松手后弹簧接管，全程不触发 React 重渲染。 */
  const x = useMotionValue(on ? openX : shutX)
  const [held, setHeld] = useState(false)
  const [hot, setHot] = useState(false)

  const rail = useRef<HTMLButtonElement | null>(null)
  const grip = useRef<{ id: number; grab: number | null; moved: boolean } | null>(null)

  /* ── 形变：由位置直接推出，不用速度 ─────────────────────────
     原版用「速度 → 伸长」是因为轨道长（92px），快速拖动时速度信号很强。
     本项目的开关只有 36px、行程 16px，速度峰值太小 —— 实测用 useVelocity
     采出来的形变恒等于 1（合成事件与真人拖动都一样），调参也救不回来。

     改成**位置驱动**：滑块离「起点 / 终点」越远、越接近中点，形变越大。
     好处：
       · 拖动过程中一定看得见（跟手，不依赖速度采样）
       · 松手弹回时也一样（位置在动，形变跟着走）
       · 快速点击时经过中点，形变最大 —— 观感与速度版一致
     面积守恒仍然成立（宽 × k，高 ÷ k）。 */
  const [shape, setShape] = useState({ wide: 1, tall: 1 })

  /* 订阅位置，把「离两端的距离」换算成伸长倍率。
     用 rAF 合并写入，避免每帧 setState 抖动。 */
  useEffect(() => {
    let raf = 0
    const apply = () => {
      const at = x.get()
      // 归一化到 0..1：0 = 在某一端，1 = 正中间
      const span = openX - shutX || 1
      const t = clamp(1 - Math.abs((at - shutX) / span * 2 - 1), 0, 1)
      /* 中间最宽。宽度与高度分别按各自的系数收放 —— 见 HEIGHT_SHRINK 的注释：
         严格面积守恒会把小球压成扁条。 */
      const k = 1 + (BASE_STRETCH + MAX_STRETCH * t) * (hot ? 1.12 : 1)
      const h = 1 - (BASE_STRETCH + HEIGHT_SHRINK * t) * (hot ? 1.12 : 1)
      const s = hot ? HOVER_SWELL : 1
      setShape({ wide: k * s, tall: h * s })
    }
    const kick = () => { cancelAnimationFrame(raf); raf = requestAnimationFrame(apply) }
    const off = x.on("change", kick)
    apply()
    return () => { off(); cancelAnimationFrame(raf) }
  }, [x, shutX, openX, hot])

  const settle = useMemo(() => ({ type: "spring" as const, stiffness: 170, damping: 21.5, mass: 0.9 }), [])

  /* 外部 `on` 变化 → 弹到目标位置。`held` 期间不抢：手指在拖时弹簧让位，
     松手后接管。cleanup 里的 stop() 是「半路抓住滑块」能生效的关键。 */
  useEffect(() => {
    if (held) return
    const run = animate(x, on ? openX : shutX, settle)
    return () => run.stop()
  }, [on, held, x, openX, shutX, settle])

  /** 客户端坐标 → 轨道内部坐标。父级可能有 CSS zoom，要除掉缩放比。 */
  const local = (clientX: number) => {
    const el = rail.current
    if (!el) return 0
    const b = el.getBoundingClientRect()
    const k = b.width / (el.offsetWidth || b.width) || 1
    return (clientX - b.left) / k
  }

  const down = (e: React.PointerEvent) => {
    if (disabled || !onChange) return
    /* 偏移量留到**第一次 move** 才取（grab = null）。
       若在这里就按「按下点 vs 滑块中心」算好，`x.set()` 是硬写、背后没有弹簧，
       第一次 move 会让滑块一帧内跳到手指下（原版实测两帧之间跳 46px）。
       留到第一次 move，那次 move 的位移为 0，之后每一次都按 delta 跟随 ——
       任何位置的按下都不会让滑块跳。 */
    grip.current = { id: e.pointerId, grab: null, moved: false }
    setHeld(true)
    // 合成事件没有活动指针会抛；拖拽本身不依赖捕获，不能让它带崩整个手势
    try { rail.current?.setPointerCapture(e.pointerId) } catch { /* not live */ }
  }

  const move = (e: React.PointerEvent) => {
    const g = grip.current
    if (!g || g.id !== e.pointerId) return
    const at = local(e.clientX)
    if (g.grab === null) g.grab = at - x.get()
    const next = clamp(at - g.grab, shutX, openX)
    if (Math.abs(next - x.get()) > 0.4) g.moved = true
    x.set(next)
    const past = next > midX
    if (past !== on) onChange?.(past)
  }

  const up = (e: React.PointerEvent) => {
    const g = grip.current
    if (!g) return
    grip.current = null
    /* release 也会抛（捕获本身可能已经失败过）。不兜住的话 `setHeld(false)`
       执行不到，开关就卡在 held —— 落位弹簧被拆掉、滑块停在拖到的位置不动。 */
    try { rail.current?.releasePointerCapture?.(e.pointerId) } catch { /* never captured */ }
    // 没移动过的按下 = 点击，开关要能当开关用
    if (!g.moved) onChange?.(!on)
    setHeld(false)
  }

  return (
    <button
      ref={rail}
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled || !onChange}
      data-size={compact ? "compact" : "regular"}
      className="jolu-switch jolu-switch--liquid"
      /* 几何回写给样式表：轨道/滑块尺寸只有这一份来源 */
      style={{
        "--switch-width": `${trackW}px`,
        "--switch-height": `${trackH}px`,
        "--switch-thumb": `${thumb}px`,
      } as CSSProperties}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={up}
      onPointerEnter={() => setHot(true)}
      onPointerLeave={() => setHot(false)}
      /* 键盘仍是一个普通开关 */
      onKeyDown={(e) => {
        if (e.key !== " " && e.key !== "Enter") return
        e.preventDefault()
        onChange?.(!on)
      }}
    >
      <span aria-hidden className="jolu-switch-track">
        <motion.span className="jolu-switch-thumb" style={{ x, scaleX: shape.wide, scaleY: shape.tall }} />
      </span>
    </button>
  )
}

export function JoluSwitch({ liquid = false, ...props }: JoluSwitchProps) {
  return liquid ? <LiquidSwitch {...props} /> : <PlainSwitch {...props} />
}
