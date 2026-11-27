import { useEffect, useId, useLayoutEffect, useRef, type ReactNode } from "react"
import { SPRING, useMotionValue, useSpring } from "@/motion"
import { edgeCornerRadii, squirclePath, type CornerRadii, type EdgeKind } from "@/lib/squircle"
import type { IslandHitRectDto } from "@/lib/api"
import type { IslandAmbientGlow } from "@/lib/botExpression"
import { cn } from "@/lib/utils"

const BUBBLE_GAP = 10
const BUBBLE_H = 40

export interface IslandBubbleSpec {
  /** 气泡可见（从主体边缘液体分离） */
  visible: boolean
  /** 气泡目标宽度（逻辑 px） */
  width: number
  /** 气泡停在主体哪一侧 */
  side: "left" | "right"
  /** 气泡内的前景内容 */
  children: ReactNode
}

export interface IslandContourProps {
  width: number
  height: number
  radii: CornerRadii
  dockEdge: EdgeKind
  open: boolean
  edgeTab: boolean
  dragging: boolean
  scroll: boolean
  glow: IslandAmbientGlow
  reduced: boolean
  ariaLabel: string
  title?: string
  expr: string
  bubble?: IslandBubbleSpec | null
  onDoubleClick?: (event: React.MouseEvent<HTMLDivElement>) => void
  onPointerEnter?: () => void
  onPointerLeave?: () => void
  /** 每当弹簧轮廓尺寸/圆角或气泡位置变化时回报窗口内命中区 */
  onHitRects?: (rects: IslandHitRectDto[], immediate?: boolean) => void
  children: ReactNode
}

/**
 * B · 连续超椭圆轮廓 + Motion 弹簧 + SVG Goo 液体分离气泡（island-rendering-tech.md §4）。
 *
 * 宽、高、四角圆角、气泡伸出进度全部由 `SPRING.island` 驱动并直接写 SVG / DOM，
 * 帧期间零 React 重渲染、静止后自动停掉动画循环；被连续状态打断时保留速度顺滑转向。
 */
export function IslandContour({
  width,
  height,
  radii,
  dockEdge,
  open,
  edgeTab,
  dragging,
  scroll,
  glow,
  reduced,
  ariaLabel,
  title,
  expr,
  bubble,
  onDoubleClick,
  onPointerEnter,
  onPointerLeave,
  onHitRects,
  children,
}: IslandContourProps) {
  const uid = useId().replace(/[^a-zA-Z0-9_-]/g, "")
  const gooId = `cap-goo-${uid}`
  const hlId = `cap-hl-${uid}`

  const rootRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const mainPathRef = useRef<SVGPathElement>(null)
  const bubblePathRef = useRef<SVGPathElement>(null)
  const strokePathRef = useRef<SVGPathElement>(null)
  const hlPathRef = useRef<SVGPathElement>(null)
  const bubbleHlRef = useRef<SVGPathElement>(null)
  const bubbleStrokeRef = useRef<SVGPathElement>(null)
  const gooGroupRef = useRef<SVGGElement>(null)
  const bubbleBoxRef = useRef<HTMLDivElement>(null)

  const bubbleVisible = Boolean(bubble?.visible && (bubble?.width ?? 0) > 0)
  const bubbleTargetW = bubble?.width ?? 40
  const bubbleSide = bubble?.side ?? "right"

  const springCfg = SPRING.island
  const mvW = useMotionValue(width)
  const mvH = useMotionValue(height)
  const mvTL = useMotionValue(radii[0])
  const mvTR = useMotionValue(radii[1])
  const mvBR = useMotionValue(radii[2])
  const mvBL = useMotionValue(radii[3])
  const mvBubble = useMotionValue(bubbleVisible ? 1 : 0)

  const sW = useSpring(mvW, springCfg)
  const sH = useSpring(mvH, springCfg)
  const sTL = useSpring(mvTL, springCfg)
  const sTR = useSpring(mvTR, springCfg)
  const sBR = useSpring(mvBR, springCfg)
  const sBL = useSpring(mvBL, springCfg)
  const sBubble = useSpring(mvBubble, springCfg)

  const latestRef = useRef({
    bubbleTargetW,
    bubbleSide,
    bubbleVisible,
    dockEdge,
    reduced,
    onHitRects,
  })
  latestRef.current = { bubbleTargetW, bubbleSide, bubbleVisible, dockEdge, reduced, onHitRects }

  const paint = (immediate = false) => {
    const w = Math.max(24, sW.get())
    const h = Math.max(24, sH.get())
    const snapCorner = (v: number) => (v < 0.1 ? 0 : v)
    const curRadii: CornerRadii = [
      snapCorner(sTL.get()),
      snapCorner(sTR.get()),
      snapCorner(sBR.get()),
      snapCorner(sBL.get()),
    ]
    const bt = Math.max(0, Math.min(1.15, sBubble.get()))
    const {
      bubbleTargetW: bwTarget,
      bubbleSide: bSide,
      dockEdge: curEdge,
      reduced: isReduced,
      onHitRects: report,
    } = latestRef.current

    const mainD = squirclePath(w, h, curRadii, 0, 0)
    const innerD = squirclePath(Math.max(2, w - 1), Math.max(2, h - 1), curRadii.map((r) => Math.max(0, r - 0.5)) as CornerRadii, 0.5, 0.5)

    mainPathRef.current?.setAttribute("d", mainD)
    strokePathRef.current?.setAttribute("d", innerD)
    hlPathRef.current?.setAttribute("d", innerD)

    const content = contentRef.current
    if (content) {
      content.style.width = `${w}px`
      content.style.height = `${h}px`
      content.style.borderRadius = `${curRadii[0]}px ${curRadii[1]}px ${curRadii[2]}px ${curRadii[3]}px`
    }

    // 液体分离气泡：从主体左右边缘鼓出、拉丝并分离到目标间距
    const showB = bt > 0.02
    const bw = Math.max(26, bwTarget * Math.min(1, bt * 1.15))
    const bh = BUBBLE_H
    const by = (h - bh) / 2
    // bt=0 时气泡藏在主体边缘内侧（重叠），bt=1 时分离到 BUBBLE_GAP 之外
    const bx = bSide === "right"
      ? w - bw * 0.55 + bt * (bw * 0.55 + BUBBLE_GAP)
      : -bw * 0.45 - bt * (bw * 0.55 + BUBBLE_GAP)
    const br = bh / 2
    // 正圆卫星气泡（如多会话并发旋转圆圈 40×40）保持完整圆形；横向长条气泡在顶/底贴边时同步贴齐屏幕边直角
    const bRadii: CornerRadii = bwTarget > bh + 4 && (curEdge === "top" || curEdge === "bottom")
      ? edgeCornerRadii(br, curEdge)
      : [br, br, br, br]

    if (showB) {
      const bInnerRadii = bRadii.map((r) => Math.max(0, r - 0.5)) as CornerRadii
      const bD = squirclePath(bw, bh, bRadii, bx, by)
      const bInnerD = squirclePath(Math.max(2, bw - 1), Math.max(2, bh - 1), bInnerRadii, bx + 0.5, by + 0.5)
      const rimAlpha = String(Math.max(0, (bt - 0.45) / 0.55))
      bubblePathRef.current?.setAttribute("d", bD)
      bubbleHlRef.current?.setAttribute("d", bInnerD)
      bubbleStrokeRef.current?.setAttribute("d", bInnerD)
      if (bubbleHlRef.current) bubbleHlRef.current.style.opacity = rimAlpha
      if (bubbleStrokeRef.current) bubbleStrokeRef.current.style.opacity = rimAlpha
    } else {
      bubblePathRef.current?.setAttribute("d", "")
      bubbleHlRef.current?.setAttribute("d", "")
      bubbleStrokeRef.current?.setAttribute("d", "")
      if (bubbleHlRef.current) bubbleHlRef.current.style.opacity = "0"
      if (bubbleStrokeRef.current) bubbleStrokeRef.current.style.opacity = "0"
    }

    // 仅在气泡拉丝分离/收回过程中启用 goo SVG 滤镜，静止或减少动效时零滤镜开销
    const gooActive = !isReduced && bt > 0.02 && bt < 0.94
    if (gooGroupRef.current) {
      if (gooActive) gooGroupRef.current.setAttribute("filter", `url(#${gooId})`)
      else gooGroupRef.current.removeAttribute("filter")
    }

    const bubbleBox = bubbleBoxRef.current
    if (bubbleBox) {
      if (showB) {
        bubbleBox.style.display = "flex"
        bubbleBox.style.width = `${bw}px`
        bubbleBox.style.left = `${bx}px`
        bubbleBox.style.top = `${by}px`
        bubbleBox.style.opacity = String(Math.max(0, Math.min(1, (bt - 0.35) / 0.65)))
      } else {
        bubbleBox.style.display = "none"
        bubbleBox.style.opacity = "0"
      }
    }

    if (report && content) {
      const box = content.getBoundingClientRect()
      if (box.width > 1 && box.height > 1) {
        const scaleX = w > 0 ? box.width / w : 1
        const scaleY = h > 0 ? box.height / h : 1
        const rects: IslandHitRectDto[] = [{
          x: box.left,
          y: box.top,
          width: box.width,
          height: box.height,
          radii: [
            curRadii[0] * scaleX,
            curRadii[1] * scaleX,
            curRadii[2] * scaleX,
            curRadii[3] * scaleX,
          ],
        }]
        if (showB && bt > 0.35) {
          rects.push({
            x: box.left + bx * scaleX,
            y: box.top + by * scaleY,
            width: bw * scaleX,
            height: bh * scaleY,
            radii: [
              bRadii[0] * scaleX,
              bRadii[1] * scaleX,
              bRadii[2] * scaleX,
              bRadii[3] * scaleX,
            ],
          })
        }
        report(rects, immediate)
      }
    }
  }

  // 订阅全部弹簧通道：直接写 SVG 与 DOM，零 React 重渲染
  useEffect(() => {
    const offs = [
      sW.on("change", () => paint(false)),
      sH.on("change", () => paint(false)),
      sTL.on("change", () => paint(false)),
      sTR.on("change", () => paint(false)),
      sBR.on("change", () => paint(false)),
      sBL.on("change", () => paint(false)),
      sBubble.on("change", () => paint(false)),
      sW.on("animationComplete", () => paint(true)),
      sH.on("animationComplete", () => paint(true)),
      sBubble.on("animationComplete", () => paint(true)),
    ]
    return () => offs.forEach((off) => off())
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sW, sH, sTL, sTR, sBR, sBL, sBubble])

  // 驱动目标值变化：减少动效直接跳终态；否则交由 SPRING.island 保留速度连续插值
  const firstMount = useRef(true)
  useLayoutEffect(() => {
    const jump = firstMount.current || reduced
    firstMount.current = false
    const setTarget = (mv: typeof mvW, sp: typeof sW, target: number) => {
      mv.set(target)
      if (jump) sp.jump(target)
    }
    setTarget(mvW, sW, width)
    setTarget(mvH, sH, height)
    setTarget(mvTL, sTL, radii[0])
    setTarget(mvTR, sTR, radii[1])
    setTarget(mvBR, sBR, radii[2])
    setTarget(mvBL, sBL, radii[3])
    setTarget(mvBubble, sBubble, bubbleVisible ? 1 : 0)
    paint(true)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [width, height, radii[0], radii[1], radii[2], radii[3], bubbleVisible, bubbleTargetW, bubbleSide, reduced, dockEdge])

  return (
    <div
      ref={rootRef}
      role="group"
      aria-label={ariaLabel}
      title={title}
      data-capsule-island
      data-cap-expr={expr}
      data-open={open}
      data-edge-tab={edgeTab}
      data-dragging={dragging || undefined}
      data-docked={Boolean(dockEdge)}
      data-dock-edge={dockEdge ?? undefined}
      data-glow={glow}
      className="cap-stage-root"
      onDoubleClick={onDoubleClick}
      onPointerEnter={onPointerEnter}
      onPointerLeave={onPointerLeave}
    >
      {/* 连续轮廓层（SVG 硬件实体面 + Goo 液体分离气泡 + 1px 内描边 + 顶部内高光） */}
      <svg className="cap-contour-svg" aria-hidden>
        <defs>
          <filter id={gooId} x="-40%" y="-40%" width="180%" height="180%" colorInterpolationFilters="sRGB">
            <feGaussianBlur in="SourceGraphic" stdDeviation="5" result="blur" />
            <feColorMatrix in="blur" mode="matrix" values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 20 -8" result="goo" />
            <feComposite in="SourceGraphic" in2="goo" operator="atop" />
          </filter>
          <linearGradient id={hlId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="rgba(255,255,255,0.14)" />
            <stop offset="28%" stopColor="rgba(255,255,255,0.03)" />
            <stop offset="100%" stopColor="rgba(255,255,255,0)" />
          </linearGradient>
        </defs>
        <g ref={gooGroupRef}>
          <path ref={mainPathRef} fill="var(--pn-capsule-bg)" />
          <path ref={bubblePathRef} fill="var(--pn-capsule-bg)" />
        </g>
        <path ref={hlPathRef} fill={`url(#${hlId})`} />
        <path ref={bubbleHlRef} fill={`url(#${hlId})`} style={{ opacity: 0 }} />
        <path ref={strokePathRef} fill="none" stroke="var(--pn-capsule-stroke)" strokeWidth={1} />
        <path ref={bubbleStrokeRef} fill="none" stroke="var(--pn-capsule-stroke)" strokeWidth={1} style={{ opacity: 0 }} />
      </svg>

      {/* 前景内容裁切盒：宽高与圆角每帧紧贴轮廓路径 */}
      <div
        ref={contentRef}
        className={cn("cap-content-box", scroll && "cap-scroll")}
        style={{ width, height, borderRadius: `${radii[0]}px ${radii[1]}px ${radii[2]}px ${radii[3]}px` }}
      >
        <span aria-hidden className="cap-ambient-glow" />
        {children}
      </div>

      {/* 液体分离气泡的前景内容（仅在存在气泡时挂载） */}
      <div
        ref={bubbleBoxRef}
        className="cap-bubble-content"
        style={{ display: "none", opacity: 0 }}
      >
        {bubble?.children}
      </div>
    </div>
  )
}
