/**
 * 连续超椭圆轮廓（Squircle）与贴边直角计算（docs/specs/2026-11-24-island-rendering-tech.md §4 / §5）。
 *
 * 与 CSS `border-radius` 的圆弧切线突变不同，这里用带平滑延展系数（smoothing）的
 * 三次贝塞尔曲线把直线段平滑过渡进圆角，观感接近硬件倒角；任意角半径 ≤ 0.5px 时
 * 退化为精确直角顶点，供「从屏幕边缘长出来」的贴边形态无缝贴合屏幕边缘。
 */

export type CornerRadii = [number, number, number, number] // [tl, tr, br, bl]
export type EdgeKind = "top" | "bottom" | "left" | "right" | null

/** 默认平滑延展系数（Apple squircle 约 0.6）；0 = 纯圆弧近似，越大过渡越柔和 */
const DEFAULT_SMOOTHING = 0.6

const f = (n: number) => (Number.isFinite(n) ? Math.round(n * 100) / 100 : 0)

/**
 * 规范化四角半径：防止相邻两角半径之和超过所在边长，避免弹簧过冲时路径自交。
 */
export function clampRadii(width: number, height: number, radii: CornerRadii): CornerRadii {
  const w = Math.max(0, Number.isFinite(width) ? width : 0)
  const h = Math.max(0, Number.isFinite(height) ? height : 0)
  const raw = radii.map((r) => (Number.isFinite(r) ? Math.max(0, r) : 0)) as CornerRadii
  if (w === 0 || h === 0) return [0, 0, 0, 0]

  const scales = [
    raw[0] + raw[1] > 0 ? w / (raw[0] + raw[1]) : 1,
    raw[1] + raw[2] > 0 ? h / (raw[1] + raw[2]) : 1,
    raw[2] + raw[3] > 0 ? w / (raw[2] + raw[3]) : 1,
    raw[3] + raw[0] > 0 ? h / (raw[3] + raw[0]) : 1,
  ]
  const k = Math.min(1, ...scales)
  return [raw[0] * k, raw[1] * k, raw[2] * k, raw[3] * k]
}

/**
 * 生成以 `(x, y)` 为左上角、宽 `width`、高 `height`、四角独立半径 `[tl, tr, br, bl]`
 * 的单条封闭超椭圆 SVG 路径。
 */
export function squirclePath(
  width: number,
  height: number,
  radii: CornerRadii,
  x = 0,
  y = 0,
  smoothing = DEFAULT_SMOOTHING,
): string {
  const w = Math.max(0, Number.isFinite(width) ? width : 0)
  const h = Math.max(0, Number.isFinite(height) ? height : 0)
  if (w < 0.5 || h < 0.5) return ""

  const [tl, tr, br, bl] = clampRadii(w, h, radii)
  const sm = Math.max(0, Math.min(1, Number.isFinite(smoothing) ? smoothing : DEFAULT_SMOOTHING))
  // 圆角沿直边延展距离不超过半边长（保证两条直边段不重叠）
  const ext = (r: number, maxSpan: number) => (r <= 0.5 ? 0 : Math.min(maxSpan, r * (1 + sm * 0.28)))
  // 控制点拉近角点的比例：0.5523 为圆弧，乘 (1 - sm * 0.14) 形成平缓曲率渐进
  const handle = (r: number, span: number) => Math.max(0, span - r * (0.5523 - sm * 0.06))

  const tlH = ext(tl, w / 2)
  const trH = ext(tr, w / 2)
  const trV = ext(tr, h / 2)
  const brV = ext(br, h / 2)
  const brH = ext(br, w / 2)
  const blH = ext(bl, w / 2)
  const blV = ext(bl, h / 2)
  const tlV = ext(tl, h / 2)

  const x0 = x
  const y0 = y
  const x1 = x + w
  const y1 = y + h

  const seg: string[] = []
  seg.push(`M ${f(x0 + tlH)} ${f(y0)}`)

  // 顶边 → 右上角
  seg.push(`L ${f(x1 - trH)} ${f(y0)}`)
  if (tr > 0.5) {
    const cx1 = x1 - handle(tr, trH)
    const cy2 = y0 + handle(tr, trV)
    seg.push(`C ${f(cx1)} ${f(y0)} ${f(x1)} ${f(cy2)} ${f(x1)} ${f(y0 + trV)}`)
  }

  // 右边 → 右下角
  seg.push(`L ${f(x1)} ${f(y1 - brV)}`)
  if (br > 0.5) {
    const cy1 = y1 - handle(br, brV)
    const cx2 = x1 - handle(br, brH)
    seg.push(`C ${f(x1)} ${f(cy1)} ${f(cx2)} ${f(y1)} ${f(x1 - brH)} ${f(y1)}`)
  }

  // 底边 → 左下角
  seg.push(`L ${f(x0 + blH)} ${f(y1)}`)
  if (bl > 0.5) {
    const cx1 = x0 + handle(bl, blH)
    const cy2 = y1 - handle(bl, blV)
    seg.push(`C ${f(cx1)} ${f(y1)} ${f(x0)} ${f(cy2)} ${f(x0)} ${f(y1 - blV)}`)
  }

  // 左边 → 左上角
  seg.push(`L ${f(x0)} ${f(y0 + tlV)}`)
  if (tl > 0.5) {
    const cy1 = y0 + handle(tl, tlV)
    const cx2 = x0 + handle(tl, tlH)
    seg.push(`C ${f(x0)} ${f(cy1)} ${f(cx2)} ${f(y0)} ${f(x0 + tlH)} ${f(y0)}`)
  }

  seg.push("Z")
  return seg.join(" ")
}

/**
 * 按停靠边决定四角半径（§5 贴边长出来）：
 * 贴哪条边，靠那条边的两个角平滑变为 0px 直角；拖离边缘后四脚恢复 `base`。
 */
export function edgeCornerRadii(base: number, dockEdge: EdgeKind): CornerRadii {
  const r = Math.max(0, base)
  switch (dockEdge) {
    case "top":
      return [0, 0, r, r]
    case "bottom":
      return [r, r, 0, 0]
    case "left":
      return [0, r, r, 0]
    case "right":
      return [r, 0, 0, r]
    default:
      return [r, r, r, r]
  }
}
