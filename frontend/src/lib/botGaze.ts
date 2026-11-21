import type { BotExpr } from "./botExpression.ts"

/**
 * bot 眼睛的视线（设计说明 §6.2）。偏移单位是脸的直径比例。
 */
export interface Vec { x: number; y: number }

export const GAZE_MAX_X = 0.13
export const GAZE_MAX_Y = 0.1
/** 每帧向目标逼近的比例；减少动效时直接对准 */
export const GAZE_SMOOTH = 0.22

/**
 * 小脸视线补偿的上限。1.35 的来历：1.9 会让眼睛中心跑到脸外（18px 脸实测
 * 半径比 1.12，观感是「眼睛脱离身体」）。`sphereEyeTransform` 里另有一道硬边界
 * 夹紧兜底，但这里先收敛，避免夹紧频繁触发导致跟随「贴住边缘不动」。
 *
 * 只此一份：`gazeTarget` 也引用它，不要再写第二个更大的数——那会是永不生效的死代码。
 */
export const GAZE_SIZE_GAIN_CAP = 1.35

/**
 * 小脸上的视线幅度补偿。
 * GAZE_MAX 是「脸直径的比例」，同样的比例在小脸上只剩几像素 ——
 * 收缩胶囊 F≈18 时最大位移只有 3.2px / 2.3px，等于看不出在跟随；
 * 展开大脸 F≈84 有 14.9px / 10.9px，观感正常。
 * 这里按脸的像素尺寸给一个温和的上浮：小脸放大到 `GAZE_SIZE_GAIN_CAP`，大脸保持 1×。
 * 用 sqrt 而非线性，避免迷你态眼睛「抽搐」。
 */
export const gazeSizeGain = (faceSize: number): number => {
  if (!(faceSize > 0)) return 1
  const scaled = Math.sqrt(84 / faceSize)
  return Math.max(1, Math.min(GAZE_SIZE_GAIN_CAP, scaled))
}

/**
 * 睡着只轻微转动；贴边偷看与被拖动减弱。
 * peek 从 0.4 提到 0.75：收缩/迷你态本来绝对位移就小（3px 上下），
 * 再压到 0.4× 几乎看不出眼睛在跟随鼠标。
 */
const AMP: Partial<Record<BotExpr, number>> = { sleep: 0.3, peek: 0.75, drag: 0.6 }
export const gazeAmp = (expr: BotExpr): number => AMP[expr] ?? 1

/**
 * 表情自带的视线偏移。
 *
 * peek（贴边偷看）曾经返回「眼睛挤向屏幕内侧」的固定偏移：顶部停靠 +0.15 向下。
 * 这在展开大脸（F≈84）上是 6px 的方向提示，但在收缩胶囊的小脸（F≈18）上，
 * 0.15 的比例 = 1.35px 的恒定下压 —— 鼠标移到最上方最多只能抵消约 1.3px，
 * 眼睛永远朝下，跟随感被彻底压死。这就是「停靠待机时眼睛不跟鼠标」的根因。
 *
 * 现在 peek 只保留眼型差异（EYES.peek 的 h: 0.9，比 idle 略矮），不再压视线：
 * 偷看的语义由「贴着屏幕边 + 略矮的眼型」表达，跟随完全交给指针。
 * work 的 base 一直是 {0,0}，所以它在同一套管线里表现得正常。
 */
export function gazeBase(_expr: BotExpr, _edge: "top" | "bottom" | "left" | "right" | null): Vec {
  return { x: 0, y: 0 }
}

/**
 * 视线目标 = 表情偏移 + 指向指针的偏移。
 * 指针离脸越远越接近最大偏移，达到最大值的距离为 max(80px, 脸宽 × 2.5)。
 * pointer 为 null（指针不在或位置未知）时只剩表情偏移。
 *
 * 指针分量按脸尺寸补偿（gazeSizeGain）：收缩胶囊的小脸按比例算出的绝对位移
 * 只有 3px 上下，肉眼看不出跟随；大脸不做额外放大。
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
      // 尺寸补偿：小脸放大比例，大脸保持 1×。上限直接取 gazeSizeGain 的封顶值——
      // 那里已经限到 GAZE_SIZE_GAIN_CAP，这里再写一个更大的常数就是永不生效的死代码，
      // 且两个数不一致时读代码的人会以为还有第二层放大。
      const gain = Math.min(GAZE_SIZE_GAIN_CAP, gazeSizeGain(face.width) * options.amp)
      x += (dx / distance) * GAZE_MAX_X * reach * gain
      y += (dy / distance) * GAZE_MAX_Y * reach * gain
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
 *
 * ── 小脸为什么不能只靠球面投影 ────────────────────────────────
 * 球面投影的横向位移来自「眼睛绕 y 轴转」，静息位 x0=±0.27 的眼睛转到
 * 侧向时主要沿 z 移动，x 的净变化在中段（lookX≈0.5）几乎为零 ——
 * 18px 小脸上横向只剩 0.2px，肉眼完全看不出跟随。
 * 所以小脸混入一部分整组平移（贴纸式）：EyeTracker 反对它是因为大脸上
 * 会「像贴纸贴在圆盘上」，但在 3px 量级的小脸上，平移恰恰是最有效的信号。
 *
 * ── 边界约束（关键）────────────────────────────────────────────
 * 补偿叠加后眼睛中心会跑到脸外：18px 脸实测半径比 1.12（出界），
 * 观感就是「眼睛脱离身体飘到脸边上」。这里在最后一步做圆形夹紧：
 * 眼睛中心距脸心不超过 EYE_MAX_RADIUS_RATIO × 半径，超出的部分按方向缩回。
 * 这是硬约束 —— 以后调 GAZE_MAX / gazeSizeGain / slide 都不会再越界。
 */
export const EYE_MAX_RADIUS_RATIO = 0.72

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
  // 小脸混合整组平移：脸越小权重越高（18px → 0.4，84px → 0）
  const slide = Math.max(0, Math.min(0.4, (84 - face.sizePx) / 140))
  // 先算相对脸心的偏移，再做圆形边界夹紧，最后转成元素坐标
  let dx = x1 * R + cur.x * face.sizePx * slide
  let dy = y2 * R + cur.y * face.sizePx * slide
  const maxR = EYE_MAX_RADIUS_RATIO * R
  const dist = Math.hypot(dx, dy)
  if (dist > maxR) {
    const scale = maxR / dist
    dx *= scale
    dy *= scale
  }
  const px = R + dx, py = R + dy
  const restX = R + (index === 0 ? -face.gapPx : face.gapPx)
  const f = (v: number) => Math.round(v * 100) / 100
  return `translate(${f(px)} ${f(py)}) rotate(${f(tilt)}) scale(${f(sw)}, ${f(k)}) translate(${f(-restX)} ${f(-face.cyPx)})`
}
