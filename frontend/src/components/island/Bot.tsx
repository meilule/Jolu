import { useCallback, useEffect, useRef, useState, type ReactNode } from "react"
import { cn } from "@/lib/utils"
import { compactTokens } from "@/lib/api"
import { useLiveCount } from "@/lib/liveCounts"
import { useGaze } from "@/hooks/useGaze"
import { gazeAmp, gazeBase } from "@/lib/botGaze"
import { tickRingMarker, tickRingSegments } from "@/lib/quotaRings"
import { formatShort, tickerNext, tickerStart, type TickerFace } from "@/lib/tokenTicker"
import { isAlertExpr, type BotExpr, type BotProp, type BotShot } from "@/lib/botExpression"
import type { RingGeometry } from "@/lib/providers/types"
import { BotPropsOverlay } from "./BotPropsOverlay"
import { RiveBotFace } from "./RiveBotFace"

/**
 * 胶囊灵动岛的 bot：双圈 SVG 额度环 + Rive 脸部（未交付 `.riv` 或加载失败时自动回退 SVG 眼睛）。
 * 支持 R2-I-06 / §6.4 戳一戳：单击眨眼、连戳 3 次挤眼 `> <`（2 秒恢复），告警表情期间自动忽略。
 */
const W = "#FFFFFF"
interface EyeStyle { k: string; c: string; g: string; R?: string; h?: number; s?: number; dy?: number; cls?: string }
const EYES: Record<BotExpr, EyeStyle> = {
  idle: { k: "pill", c: W, g: "#FFFFFF66" },
  look: { k: "pill", c: W, g: "#FFFFFF66" },
  work: { k: "pill", h: 0.85, c: "#3DD68C", g: "color-mix(in srgb, var(--pn-capsule-green) 55%, transparent)", cls: "cbot-scan" },
  refresh: { k: "spin", c: "#7DD8FF", g: "#5AC8FA99", cls: "cbot-spin" },
  happy: { k: "happy", c: W, g: "#FFFFFF66" },
  surprise: { k: "round", s: 1.15, c: W, g: "#FFFFFF66", cls: "cbot-hop" },
  greet: { k: "pill", R: "happy", c: W, g: "#FFFFFF66" },
  drag: { k: "gt", c: W, g: "#FFFFFF66" },
  peek: { k: "pill", h: 0.9, c: W, g: "#FFFFFF66" },
  worry: { k: "lid", c: "#F5B83D", g: "#F5B83D99", cls: "cbot-breathe" },
  sad: { k: "sad", dy: 0.05, c: "#FF6B6F", g: "#FF6B6F99" },
  error: { k: "x", c: "#FF6B6F", g: "#FF6B6F99", cls: "cbot-shake" },
  sleep: { k: "line", dy: 0.06, c: "#8E8E93", g: "transparent", cls: "cbot-sleep" },
  reset: { k: "star", c: "#3DD68C", g: "#3DD68C99" },
  wait: { k: "pill", h: 0.55, dy: 0.04, c: "#F5B83D", g: "#F5B83D99" },
  relay: { k: "pill", h: 0.85, c: "#5AC8FA", g: "#5AC8FA99" },
}

const n = (value: number) => Math.round(value * 100) / 100

function Eye({ kind, cx, cy, F, e, flip }: { kind: string; cx: number; cy: number; F: number; e: EyeStyle; flip: boolean }) {
  const sw = n(Math.max(1.5, F * 0.055))
  const stroke = { fill: "none", stroke: e.c, strokeWidth: sw, strokeLinecap: "round" as const, strokeLinejoin: "round" as const, vectorEffect: "non-scaling-stroke" as const }
  switch (kind) {
    case "pill": {
      const w = F * 0.14, h = F * 0.3 * (e.h ?? 1)
      return <rect className="cbot-eye-shape" x={n(cx - w / 2)} y={n(cy - h / 2)} width={n(w)} height={n(h)} rx={n(w / 2)} fill={e.c} />
    }
    case "line": {
      const w = F * 0.224
      return <rect className="cbot-eye-shape" x={n(cx - w / 2)} y={n(cy - sw / 2)} width={n(w)} height={sw} rx={n(sw / 2)} fill={e.c} />
    }
    case "round":
      return <circle className="cbot-eye-shape" cx={n(cx)} cy={n(cy)} r={n(F * 0.085 * (e.s ?? 1))} fill={e.c} />
    case "happy":
      return <path className="cbot-eye-shape" d={`M${n(cx - F * 0.1)} ${n(cy + F * 0.045)} Q${n(cx)} ${n(cy - F * 0.1)} ${n(cx + F * 0.1)} ${n(cy + F * 0.045)}`} {...stroke} />
    case "sad":
      return <path className="cbot-eye-shape" d={`M${n(cx - F * 0.1)} ${n(cy - F * 0.045)} Q${n(cx)} ${n(cy + F * 0.1)} ${n(cx + F * 0.1)} ${n(cy - F * 0.045)}`} {...stroke} />
    case "x": {
      const a = F * 0.07
      return <path className="cbot-eye-shape" d={`M${n(cx - a)} ${n(cy - a)} L${n(cx + a)} ${n(cy + a)} M${n(cx + a)} ${n(cy - a)} L${n(cx - a)} ${n(cy + a)}`} {...stroke} />
    }
    case "gt": {
      const a = F * 0.06, b = F * 0.075, s = flip ? -1 : 1
      return <path className="cbot-eye-shape" d={`M${n(cx - a * s)} ${n(cy - b)} L${n(cx + a * s)} ${n(cy)} L${n(cx - a * s)} ${n(cy + b)}`} {...stroke} />
    }
    case "spin": {
      const r = F * 0.08, c = 2 * Math.PI * r
      return (
        <g transform={`rotate(${flip ? 180 : 0} ${n(cx)} ${n(cy)})`}>
          <circle className="cbot-spin-eye cbot-eye-shape" style={{ ["--o" as string]: `${n(cx)}px ${n(cy)}px` }} cx={n(cx)} cy={n(cy)} r={n(r)}
            fill="none" stroke={e.c} strokeWidth={sw} strokeLinecap="round" strokeDasharray={`${n(c * 0.68)} ${n(c)}`} vectorEffect="non-scaling-stroke" />
        </g>
      )
    }
    case "lid": {
      const w = F * 0.14, h = F * 0.234, x0 = cx - w / 2, y0 = cy - h * 0.46, u = w / 10, v = h / 18
      const p = (x: number, y: number) => `${n(x0 + x * u)} ${n(y0 + y * v)}`
      const top = flip ? `M${p(0, 0)} L${p(10, 3)}` : `M${p(0, 3)} L${p(10, 0)}`
      return <path className="cbot-eye-shape" d={`${top} L${p(10, 14)} Q${p(10, 18)} ${p(5, 18)} Q${p(0, 18)} ${p(0, 14)} Z`} fill={e.c} />
    }
    case "star": {
      const r = F * 0.11, k = r * 0.12
      return <path className="cbot-eye-shape" d={`M${n(cx)} ${n(cy - r)} Q${n(cx + k)} ${n(cy - k)} ${n(cx + r)} ${n(cy)} Q${n(cx + k)} ${n(cy + k)} ${n(cx)} ${n(cy + r)} Q${n(cx - k)} ${n(cy + k)} ${n(cx - r)} ${n(cy)} Q${n(cx - k)} ${n(cy - k)} ${n(cx)} ${n(cy - r)} Z`} fill={e.c} />
    }
  }
  return null
}

function TickRing({ size, radius, ringWidth, percent, color, warn, dimmed }: {
  size: number
  radius: number
  ringWidth: number
  percent: number | null
  color: string
  warn: boolean
  dimmed: boolean
}) {
  const segments = tickRingSegments({ size, radius, ringWidth, percent })
  const marker = tickRingMarker({ size, radius, ringWidth })
  const ticks = segments.map((segment, index) => ({ segment, index }))
  return (
    <>
      {ticks.filter(({ segment }) => !segment.on).map(({ segment, index }) => (
        <rect key={`off-${index}`} x={n(segment.x)} y={n(segment.y)} width={n(segment.width)} height={n(segment.height)}
          rx={n(segment.width / 2)} fill="var(--pn-capsule-track)" style={{ opacity: dimmed ? 0.5 : 1 }}
          transform={`rotate(${n(segment.angle)} ${n(segment.cx)} ${n(segment.cy)})`} />
      ))}
      <g className={cn(warn && "cbot-warn-ring")} style={{ opacity: dimmed ? 0.3 : 1 }}>
        {ticks.filter(({ segment }) => segment.on).map(({ segment, index }) => (
          <rect key={`on-${index}`} x={n(segment.x)} y={n(segment.y)} width={n(segment.width)} height={n(segment.height)}
            rx={n(segment.width / 2)} fill={color}
            transform={`rotate(${n(segment.angle)} ${n(segment.cx)} ${n(segment.cy)})`} />
        ))}
      </g>
      <circle cx={n(marker.cx)} cy={n(marker.cy)} r={n(marker.r)} fill="var(--pn-capsule-text-3)" style={{ opacity: dimmed ? 0.5 : 1 }} />
    </>
  )
}

function LiveText({ countKey, short, ...rest }: { countKey: string; short: boolean } & React.SVGProps<SVGTextElement>) {
  const value = useLiveCount(countKey) ?? 0
  return <text {...rest}>{short ? formatShort(value) : `+${compactTokens(value)}`}</text>
}

export interface BotRing {
  p: number | null
  geometry: RingGeometry
  color: string
  warn?: boolean
  aria?: string
}

export interface BotProps {
  size: number
  certainty?: "confirmed" | "inferred" | "unknown" | "idle"
  ringWidth?: number
  ringGap?: number
  inset?: number
  rings?: BotRing[]
  expr: BotExpr
  prop?: BotProp
  shot?: BotShot | null
  reduced?: boolean
  /** 启用 R2-I-06 戳一戳（单击眨眼、连戳 3 次挤眼，告警期自动屏蔽） */
  pokeable?: boolean
  /** R2-I-06 看向主窗口：主窗口中心的桌面物理坐标（开启跟随主窗口且不在本岛悬停时眼睛转向它） */
  lookAtPhysical?: { x: number; y: number } | null
  /** R2-I-06 长按 bot 呼出快捷动作轮盘 */
  onFacePointerDown?: (event: React.PointerEvent) => void
  /** 本轮点击是否刚触发过长按（为 true 时抑制随后的戳一戳 click） */
  suppressPokeClickRef?: { current: boolean }
  dockEdge?: "top" | "bottom" | "left" | "right" | null
  ticker?: { countKey: string; label: string; sub?: string } | null
  focus?: number | null
  display?: [string, string, string?] | null
  interactive?: boolean
  onRingHover?: (index: number | null) => void
  onRingClick?: (index: number) => void
  pinnedRing?: number | null
  className?: string
}

const SMALL_FACE = 40
const MIN_TICK_FACE = 16

export function Bot({
  size, ringWidth = 0, ringGap = 0, inset = 0, rings = [], expr, prop = "none", shot = null, reduced = false,
  pokeable = false, lookAtPhysical = null, onFacePointerDown, suppressPokeClickRef, dockEdge = null, ticker = null, certainty,
  focus = null, display = null, interactive = false, onRingHover, onRingClick, pinnedRing = null, className,
}: BotProps) {
  const count = rings.length
  const offset = count ? count * (ringWidth + ringGap) : inset
  const F = size - 2 * offset
  const small = F < SMALL_FACE
  const c = size / 2

  const svgRef = useRef<SVGSVGElement>(null)
  const faceRef = useRef<SVGCircleElement>(null)
  const eyesRef = useRef<SVGGElement>(null)
  const [riveReady, setRiveReady] = useState(false)

  // R2-I-06 戳一戳状态：单击眨眼，1.2s 内连戳 3 次挤眼（> <）停留 2 秒；告警期静默不响应
  const [pokeSeq, setPokeSeq] = useState(0)
  const [squeezed, setSqueezed] = useState(false)
  const pokeTimes = useRef<number[]>([])
  const squeezeTimer = useRef(0)
  const blinkTimer = useRef(0)

  useEffect(() => {
    if (isAlertExpr(expr) && squeezed) {
      window.clearTimeout(squeezeTimer.current)
      setSqueezed(false)
    }
  }, [expr, squeezed])

  useEffect(() => () => {
    window.clearTimeout(squeezeTimer.current)
    window.clearTimeout(blinkTimer.current)
  }, [])

  const handleFaceClick = useCallback((event: React.MouseEvent) => {
    if (suppressPokeClickRef?.current) {
      suppressPokeClickRef.current = false
      return
    }
    if (!pokeable || isAlertExpr(expr) || event.detail > 1) return
    const now = Date.now()
    pokeTimes.current = [...pokeTimes.current.filter((t) => now - t <= 1200), now]
    setPokeSeq((seq) => seq + 1)

    if (pokeTimes.current.length >= 3) {
      pokeTimes.current = []
      setSqueezed(true)
      window.clearTimeout(squeezeTimer.current)
      squeezeTimer.current = window.setTimeout(() => setSqueezed(false), 2000)
      return
    }
    const svg = svgRef.current
    if (svg && !reduced) {
      svg.classList.remove("cbot-blink")
      void svg.getBoundingClientRect()
      svg.classList.add("cbot-blink")
      window.clearTimeout(blinkTimer.current)
      blinkTimer.current = window.setTimeout(() => svg.classList.remove("cbot-blink"), 160)
    }
  }, [pokeable, expr, reduced])

  const shownExpr: BotExpr = squeezed && !isAlertExpr(expr) ? "drag" : expr
  const e = EYES[shownExpr] ?? EYES.idle

  const gapPx = F * 0.135, eyeCy = F / 2 + (e.dy ?? 0) * F
  useGaze(faceRef, eyesRef, { base: gazeBase(shownExpr, dockEdge), amp: gazeAmp(shownExpr), gapPx, cyPx: eyeCy, lookAtPhysical }, F)

  const tickerOn = Boolean(ticker) && F >= MIN_TICK_FACE && !display
  const [face, setFace] = useState<TickerFace>("eyes")
  useEffect(() => {
    if (!tickerOn) { setFace("eyes"); return }
    let timer = 0
    let step = tickerStart()
    setFace(step.face)
    const advance = () => {
      step = tickerNext(step.face)
      setFace(step.face)
      timer = window.setTimeout(advance, step.delay)
    }
    timer = window.setTimeout(advance, step.delay)
    return () => window.clearTimeout(timer)
  }, [tickerOn])
  const ticking = tickerOn && face === "tick"

  useEffect(() => {
    if (reduced || !["idle", "look", "peek"].includes(shownExpr)) return
    let timer = 0, reset = 0
    const schedule = () => {
      timer = window.setTimeout(() => {
        const svg = svgRef.current
        if (svg) { svg.classList.add("cbot-blink"); reset = window.setTimeout(() => svg.classList.remove("cbot-blink"), 160) }
        schedule()
      }, 4000 + Math.random() * 2000)
    }
    schedule()
    return () => { window.clearTimeout(timer); window.clearTimeout(reset) }
  }, [shownExpr, reduced])

  const previous = useRef<BotExpr>(shownExpr)
  useEffect(() => {
    const svg = svgRef.current
    if (!reduced && shownExpr === "happy" && previous.current !== "happy" && svg && count) {
      svg.classList.remove("cbot-sweeping"); void svg.getBoundingClientRect(); svg.classList.add("cbot-sweeping")
      const timer = window.setTimeout(() => svg.classList.remove("cbot-sweeping"), 1150)
      previous.current = shownExpr
      return () => window.clearTimeout(timer)
    }
    previous.current = shownExpr
  }, [shownExpr, count, reduced])

  const g = gapPx, cy = eyeCy
  const ringEls: ReactNode[] = rings.map((ring, i) => {
    const r = size / 2 - i * (ringWidth + ringGap) - ringWidth / 2
    const C = 2 * Math.PI * r
    const p = ring.p === null ? 0 : Math.max(0, Math.min(100, ring.p)) / 100
    const dimmed = focus != null && focus !== i
    const w = focus === i ? ringWidth * 1.35 : ringWidth
    const ticks = ring.geometry === "ticks"
    return (
      <g
        key={i}
        className="cbot-ring"
        {...(interactive ? {
          tabIndex: 0, role: "button", "aria-label": ring.aria, "aria-pressed": pinnedRing === i,
          onPointerEnter: () => onRingHover?.(i), onPointerLeave: () => onRingHover?.(null),
          onFocus: () => onRingHover?.(i), onBlur: () => onRingHover?.(null),
          onClick: () => onRingClick?.(i),
          onDoubleClick: (event: React.MouseEvent) => event.stopPropagation(),
          onKeyDown: (event: React.KeyboardEvent) => {
            if (event.key === "Enter" || event.key === " ") { event.preventDefault(); event.stopPropagation(); onRingClick?.(i) }
          },
        } : {})}
      >
        {ticks ? (
          <TickRing size={size} radius={r} ringWidth={w} percent={ring.p} color={ring.color}
            warn={Boolean(ring.warn)} dimmed={dimmed} />
        ) : (
          <>
            <circle className="cbot-ring-track" cx={c} cy={c} r={n(r)} fill="none" stroke="var(--pn-capsule-track)" strokeWidth={w} style={{ opacity: dimmed ? 0.5 : 1 }} />
            <circle className={cn("cbot-ring-arc", ring.warn && "cbot-warn-ring")} cx={c} cy={c} r={n(r)} fill="none" stroke={ring.color} strokeWidth={w}
              strokeLinecap="round" strokeDasharray={n(C)} strokeDashoffset={n(C * (1 - p))} transform={`rotate(-90 ${c} ${c})`}
              style={{ opacity: p <= 0 ? 0 : dimmed ? 0.3 : 1 }} />
          </>
        )}
        {i === 0 && !ticks && (
          <circle className="cbot-sweep" cx={c} cy={c} r={n(r)} fill="none" stroke="#FFFFFF" strokeWidth={ringWidth} strokeLinecap="round"
            strokeDasharray={`${n(C * 0.1)} ${n(C)}`} transform={`rotate(-90 ${c} ${c})`} style={{ ["--c" as string]: n(C) }} />
        )}
        {interactive && <circle cx={c} cy={c} r={n(r)} fill="none" stroke="transparent" strokeWidth={ringWidth + ringGap + 2} pointerEvents="stroke" />}
      </g>
    )
  })

  return (
    <span className={cn("cbot-wrap", className)} style={{ width: size, height: size }}>
      <svg
        ref={svgRef}
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        aria-hidden={interactive ? undefined : true}
        className={cn("cbot", e.cls, ticking && "cbot-ticking", display && "cbot-showing")}
        data-expr={shownExpr}
        data-certainty={certainty}
        data-rive={riveReady ? "on" : "fallback"}
        data-small={small && count ? 1 : 0}
        style={{ ["--F" as string]: n(F), ["--z" as string]: n(size / F), ["--glow" as string]: e.g }}
      >
        {ringEls}
        <g transform={`translate(${n(offset)} ${n(offset)})`}>
          <g className="cbot-zoom">
            <circle
              ref={faceRef}
              cx={n(F / 2)}
              cy={n(F / 2)}
              r={n(F / 2 - 0.5)}
              fill="var(--pn-capsule-bot)"
              stroke="var(--pn-capsule-bot-stroke)"
              strokeWidth={1}
              className={pokeable && !isAlertExpr(expr) ? "cbot-face-hit" : undefined}
              onPointerDown={onFacePointerDown}
              onClick={handleFaceClick}
            />
            {!riveReady && (
              <g ref={eyesRef} className="cbot-eyes" style={{ filter: `drop-shadow(0 0 ${n(Math.max(1.5, F * 0.1))}px var(--glow))`, pointerEvents: "none" }}>
                <g className="cbot-eyes-slide">
                  <g className="cbot-eyes-inner" key={shownExpr}>
                    <g className="cbot-eye-p"><Eye kind={e.k} cx={F / 2 - g} cy={cy} F={F} e={e} flip={false} /></g>
                    <g className="cbot-eye-p"><Eye kind={e.R ?? e.k} cx={F / 2 + g} cy={cy} F={F} e={e} flip /></g>
                  </g>
                </g>
              </g>
            )}
            {ticker && (
              <g className="cbot-tick" textAnchor="middle" fontFamily="var(--font-mono, ui-monospace)" style={{ filter: `drop-shadow(0 0 ${n(Math.max(1.5, F * 0.06))}px #3DD68C80)`, pointerEvents: "none" }}>
                {small ? (
                  <LiveText countKey={ticker.countKey} short x={n(F / 2)} y={n(F / 2 + F * 0.13)} fontSize={n(F * 0.36)} fontWeight={700} fill="#3DD68C" letterSpacing={-0.3} />
                ) : (
                  <>
                    <LiveText countKey={ticker.countKey} short={false} x={n(F / 2)} y={n(F * 0.53)} fontSize={n(F * 0.2)} fontWeight={700} fill="#3DD68C" letterSpacing={-0.6} />
                    <text x={n(F / 2)} y={n(F * 0.67)} fontSize={n(F * 0.095)} fontWeight={600} fill="#FFFFFFB3">{ticker.label}</text>
                    {ticker.sub && <text x={n(F / 2)} y={n(F * 0.79)} fontSize={n(F * 0.08)} fontWeight={500} fill="#FFFFFF80">{ticker.sub}</text>}
                  </>
                )}
              </g>
            )}
            {display && (
              <g className="cbot-disp" textAnchor="middle" style={{ pointerEvents: "none" }}>
                <text x={n(F / 2)} y={n(F * 0.52)} fontSize={n(F * 0.24)} fontWeight={700} fill="#FFFFFF" letterSpacing={-0.5}>{display[0]}</text>
                <text x={n(F / 2)} y={n(F * 0.66)} fontSize={n(F * 0.095)} fontWeight={600} fill="#FFFFFFB3">{display[1]}</text>
                {display[2] && <text x={n(F / 2)} y={n(F * 0.78)} fontSize={n(F * 0.08)} fontWeight={500} fill="#FFFFFF80">{display[2]}</text>}
              </g>
            )}
            {!riveReady && (
              <g className="cbot-zz" fontWeight={700} style={{ pointerEvents: "none" }}>
                <text x={n(F * 0.68)} y={n(F * 0.34)} fontSize={n(F * 0.17)} fill="#FFFFFF80">z</text>
                <text x={n(F * 0.8)} y={n(F * 0.22)} fontSize={n(F * 0.12)} fill="#FFFFFF55">z</text>
              </g>
            )}
          </g>
        </g>
      </svg>

      {/* Rive 脸部叠加在额度环内部：未交付 bot.riv 时不渲染、透明回退 SVG 眼睛 */}
      <span
        style={{
          position: "absolute",
          left: offset,
          top: offset,
          width: F,
          height: F,
          borderRadius: "50%",
          overflow: "hidden",
          pointerEvents: "none",
          display: ticking || display ? "none" : "block",
        }}
      >
        <RiveBotFace
          size={F}
          expr={shownExpr}
          certainty={certainty}
          prop={prop}
          shot={shot}
          pokeSeq={pokeSeq}
          small={small}
          reduced={reduced}
          onReadyChange={setRiveReady}
        />
      </span>
      {/* SVG 兜底模式下的 R2-I-05 道具覆盖层（Rive 就绪后由 Rive 状态机接管） */}
      {!riveReady && !ticking && !display && prop !== "none" && (
        <BotPropsOverlay prop={prop} size={size} reduced={reduced} />
      )}
    </span>
  )
}
