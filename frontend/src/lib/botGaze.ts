import type { BotExpr } from "./botExpression.ts"

/**
 * bot 眼睛的视线（设计说明 §6.2）。偏移单位是脸的直径比例。
 */
export interface Vec { x: number; y: number }

export const GAZE_MAX_X = 0.13
export const GAZE_MAX_Y = 0.1
/** 每帧向目标逼近的比例；减少动效时直接对准 */
export const GAZE_SMOOTH = 0.22

/** 睡着只轻微转动，贴边偷看与被拖动减弱 */
const AMP: Partial<Record<BotExpr, number>> = { sleep: 0.3, peek: 0.4, drag: 0.6 }
export const gazeAmp = (expr: BotExpr): number => AMP[expr] ?? 1

/** 表情自带的视线偏移：贴边偷看时眼睛挤向朝屏幕内的一侧 */
export function gazeBase(expr: BotExpr, edge: "top" | "bottom" | "left" | "right" | null): Vec {
  if (expr !== "peek" || !edge) return { x: 0, y: 0 }
  return edge === "left" ? { x: 0.2, y: 0 } : edge === "right" ? { x: -0.2, y: 0 }
    : edge === "top" ? { x: 0, y: 0.15 } : { x: 0, y: -0.15 }
}

/**
 * 视线目标 = 表情偏移 + 指向指针的偏移。
 * 指针离脸越远越接近最大偏移，达到最大值的距离为 max(80px, 脸宽 × 2.5)。
 * pointer 为 null（指针不在或位置未知）时只剩表情偏移。
 */
export function gazeTarget(
  face: { cx: number; cy: number; width: number },
  pointer: Vec | null,
  options: { base: Vec; amp: number },
): Vec {
  let { x, y } = options.base
  if (pointer && options.amp > 0) {
    const dx = pointer.x - face.cx, dy = pointer.y - face.cy
    const distance = Math.hypot(dx, dy)
    if (distance > 0.5) {
      const reach = Math.min(1, distance / Math.max(80, face.width * 2.5))
      x += (dx / distance) * GAZE_MAX_X * reach * options.amp
      y += (dy / distance) * GAZE_MAX_Y * reach * options.amp
    }
  }
  return { x, y }
}

/** 插值一步；与目标足够接近时直接落在目标上并标记已稳定 */
export function gazeStep(current: Vec, target: Vec, smooth: number): { next: Vec; settled: boolean } {
  const next = { x: current.x + (target.x - current.x) * smooth, y: current.y + (target.y - current.y) * smooth }
  const settled = Math.abs(target.x - next.x) < 0.0008 && Math.abs(target.y - next.y) < 0.0008
  return { next: settled ? { ...target } : next, settled }
}

export const gazeSmooth = (reduced: boolean): number => (reduced ? 1 : GAZE_SMOOTH)

/* ── 球面眼睛（Bencho EyeTracker 移植，bencho.dev，MIT 协议） ──────────
   视线跟随的透视变形：不再把两只眼当贴纸整组平移，而是把每只眼看作
   球面上的一个点——偏航/俯仰随视线向量转动，靠近轮廓的那只整体变小、
   宽度收窄更多，双眼随斜视方向倾斜。平面圆因此读出立体感。
   数学与 output/island-prototype/bot.js 的移植版一致，参数按 bot 收敛。 */

/** cur（脸直径分数）→ 球面视线分数的放大倍率：bot 原本克制的最大偏移 0.26
    半径分数不足以读出变形，放大后视线最远到轮廓的 ~57% */
export const GAZE_GAIN = 1.35
/** 斜视时双眼整体倾斜的上限（deg） */
export const EYE_TILT_MAX = 30

/** EyeTracker 弹簧：速度朝目标积累、按阻尼衰减；dt 以 1/60 帧计，
    丢一帧衰减的能量与它替代的两帧相同（乘阻尼才会忙页面上手感变化）。 */
export function gazeSpringStep(
  current: Vec, vel: Vec, target: Vec, dt: number,
  spring: { k: number; d: number } = { k: 0.06, d: 0.3 },
): { next: Vec; vel: Vec } {
  const vx = vel.x + ((target.x - current.x) * spring.k - vel.x * spring.d) * dt
  const vy = vel.y + ((target.y - current.y) * spring.k - vel.y * spring.d) * dt
  return { next: { x: current.x + vx * dt, y: current.y + vy * dt }, vel: { x: vx, y: vy } }
}

/**
 * 单只眼的球面变换链（SVG transform 属性，绕静息中心旋转/缩放）。
 * index 0 = 左眼、1 = 右眼；gapPx/cyPx 是静息眼距与垂直偏移，sizePx 是脸的直径。
 * 返回值直接喂给 <g className="cbot-eye-p" transform=…>。
 */
export function sphereEyeTransform(
  index: 0 | 1,
  cur: Vec,
  face: { gapPx: number; cyPx: number; sizePx: number },
): string {
  const R = face.sizePx / 2
  const clamp = (v: number) => Math.max(-0.85, Math.min(0.85, v))
  const as = (v: number) => Math.asin(v)
  const lookX = clamp(cur.x * 2 * GAZE_GAIN), lookY = clamp(cur.y * 2 * GAZE_GAIN)
  const yaw = as(lookX), pitch = -as(lookY)
  const cyw = Math.cos(yaw), syw = Math.sin(yaw), cp = Math.cos(pitch), sp = Math.sin(pitch)
  const tilt = EYE_TILT_MAX * lookX * lookY
  const x0 = (index === 0 ? -1 : 1) * (face.gapPx / R)
  const z0 = Math.sqrt(1 - x0 * x0)
  const x1 = x0 * cyw + z0 * syw
  const z1 = -x0 * syw + z0 * cyw
  const y2 = -z1 * sp
  const z2 = z1 * cp
  // 透视：靠轮廓的那只整体变小（k），宽度收窄更多（sw）
  const near = Math.max(0, Math.min(1, z2))
  const k = 0.45 + 0.55 * near
  const sw = k * (0.7 + 0.3 * near)
  const px = R + x1 * R, py = R + y2 * R
  const restX = R + (index === 0 ? -face.gapPx : face.gapPx)
  const f = (v: number) => Math.round(v * 100) / 100
  return `translate(${f(px)} ${f(py)}) rotate(${f(tilt)}) scale(${f(sw)}, ${f(k)}) translate(${f(-restX)} ${f(-face.cyPx)})`
}
