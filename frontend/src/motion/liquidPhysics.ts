/**
 * Bencho LiquidToggle 物理与几何计算（MIT，bencho.dev/licence）的纯数学层。
 *
 * 保留所有数值与物理取舍的说明：这里的每一项常数都对应真实像素与动画帧率下的测量结果，
 * 抽成纯函数后方便 `node --test` 直接校验，不必挂浏览器就能兜住边界情况。
 */

export const clamp = (v: number, lo: number, hi: number): number => Math.min(hi, Math.max(lo, v))

/**
 * ── 开关几何（与 R1 设计稿 `docs/ui/jolu-new.pen` 一致）──────────────────
 * `md`（默认）：总宽 46 × 高 28（1px border + 2px padding = 3px 四周留白），
 * 滑块 22×22，行程 = 46 - 22 - 3 * 2 = 18px。
 * `sm`（超紧凑槽位，如路由卡片行内开关）：28 × 16，2px 内边距，滑块 12×12，行程 = 12px。
 *
 * 行程与滑块尺寸由 JS 传给组件样式（只保留单一来源），否则滑块与轨道总有一边算错 1-2px，
 * 看着像是「滑块顶歪了」。
 */
export const LIQUID_SWITCH_GEOMETRY = {
  md: {
    trackWidth: 46,
    trackHeight: 28,
    thumbSize: 22,
    inset: 3,
    travel: 18,
  },
  sm: {
    trackWidth: 28,
    trackHeight: 16,
    thumbSize: 12,
    inset: 2,
    travel: 12,
  },
} as const

export type LiquidSwitchSize = keyof typeof LIQUID_SWITCH_GEOMETRY

/**
 * Bencho 原版 stretch 旋钮默认值（0..100）。
 * 36 是原版调定的一档平衡：点按切换有肉眼可见的液体弹性，慢速拖动又能保持接近正圆。
 */
export const LIQUID_DEFAULT_STRETCH = 36

/**
 * ── 悬停微胀放进同一个 transform，不用 CSS `scale` ───────────────────────
 * 为什么不能用独立 CSS `scale` 属性：独立 transform 属性（`scale`）会先于 `transform` 生效，
 * `scale: 1.035` 会顺带把后面的 `translateX(openX)` 一并放大 3.5%，导致滑块在 ON 态往右漂移、
 * 右边距被吞掉。合进 `scaleX / scaleY` 后，微胀严格绕滑块自身圆心发生，坐标零偏移。
 */
export const LIQUID_HOVER_SWELL = 1.035

/**
 * ── 速度归一化基准：按行程缩放，而不写死 600 px/s ─────────────────────────
 * Bencho 原版轨道 92px、行程 46px；在 170 / 21.5 / 0.9 落位弹簧下，一次点按经过
 * 跟随弹簧滤波后的峰值速度约为 150px/s，除以 600 得到 0.25，再乘 0.36 ≈ 1.09（9% 伸长，
 * 视觉上约 +3.2px）。
 *
 * Jolu 的 R1 开关是 46×28，行程只有 18px（sm 只有 12px）。同一颗弹簧的峰值速度与行程
 * 成正比：18px 行程下，经跟随弹簧后的峰值约为 85px/s——若照搬固定 `/ 600`，伸长量只有
 * 4%（不到 0.9px，几乎看不出来）。因此把除数按 `travel * 8.5` 缩放（18px → 153px/s）：
 * · 18px 行程点按峰值 85/153 = 0.556，乘 0.36 → ~1.20（22px 球拉长约 +4.4px）；
 * · 慢速拖拽（~25px/s）只微幅给弹性（~1.06），快甩或点按才明显拉长，忠于原版手感；
 * · 上限卡在 0.6（默认 stretch=36 时最大 1.216），即使高速甩动也不会在紧凑轨道里压成细条。
 */
export const LIQUID_VELOCITY_TRAVEL_RATIO = 8.5
export const LIQUID_STRETCH_CAP = 0.6

/**
 * 认定为「拖动」而非「点按」的最小位移（本地 px）。
 * 防止鼠标轻抖 0.1–0.3px 被当成拖拽而吞掉本来想完成的点击切换。
 */
export const LIQUID_DRAG_THRESHOLD_PX = 0.4

/**
 * 根据水平瞬时速度（px/s）计算沿移动方向的拉伸倍率（>= 1）。
 * 与面积守恒搭配使用：`scaleX = k * swell`，`scaleY = (1 / k) * swell`。
 */
export function computeLiquidStretch(
  velocityPxPerSec: number,
  travelPx: number,
  stretch: number = LIQUID_DEFAULT_STRETCH,
): number {
  const safeVelocity = Number.isFinite(velocityPxPerSec) ? Math.abs(velocityPxPerSec) : 0
  const refVelocity = Math.max(1, travelPx * LIQUID_VELOCITY_TRAVEL_RATIO)
  const intensity = clamp(stretch, 0, 100) / 100
  return 1 + Math.min(LIQUID_STRETCH_CAP, safeVelocity / refVelocity) * intensity
}

/**
 * 组合「面积守恒拉伸」与「绕圆心悬停微胀」。
 * 静止时（v = 0, swell = 1）严格返回 `{ wide: 1, tall: 1 }`，保证滑块是正圆而不是歪椭圆。
 */
export function computeLiquidScale(
  velocityPxPerSec: number,
  swell = 1,
  travelPx: number = LIQUID_SWITCH_GEOMETRY.md.travel,
  stretch: number = LIQUID_DEFAULT_STRETCH,
): { wide: number; tall: number } {
  const k = computeLiquidStretch(velocityPxPerSec, travelPx, stretch)
  const s = Number.isFinite(swell) ? swell : 1
  return {
    wide: k * s,
    tall: (1 / k) * s,
  }
}

/**
 * 视口 clientX → 轨道局部 x（px）。
 * 设置页允许缩放（CSS `zoom`），此时 `getBoundingClientRect().width` 会含 zoom 倍率而 `offsetWidth` 不含，
 * 必须除以缩放比才能让手指/鼠标与滑块严格 1:1 跟手。
 */
export function computeLocalX(
  clientX: number,
  rectLeft: number,
  rectWidth: number,
  offsetWidth: number,
): number {
  const layoutWidth = offsetWidth || rectWidth
  const zoom = layoutWidth > 0 ? rectWidth / layoutWidth || 1 : 1
  return (clientX - rectLeft) / zoom
}

/** 过半即翻：拖拽超过轨道中点即同步切换状态，不必等松手。 */
export function shouldFlipOnDrag(nextX: number, shutX: number, openX: number): boolean {
  const midX = (shutX + openX) / 2
  return nextX > midX
}
