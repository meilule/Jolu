/**
 * LiquidToggle 物理与几何单元测试，以及 Tactile 预设基本约束校验。
 *
 * 直接跑纯函数（`motion/liquidPhysics.ts` 与 `motion/tokens.ts` 均无外部运行时依赖），
 * 兜住面积守恒、比例缩放基准、CSS zoom 坐标换算、过半阈值与弹簧预设完整性。
 */
import test from "node:test"
import assert from "node:assert/strict"

import {
  clamp,
  computeLiquidScale,
  computeLiquidStretch,
  computeLocalX,
  LIQUID_DEFAULT_STRETCH,
  LIQUID_HOVER_SWELL,
  LIQUID_STRETCH_CAP,
  LIQUID_SWITCH_GEOMETRY,
  shouldFlipOnDrag,
} from "./liquidPhysics.ts"
import { SPRING, TACTILE } from "./tokens.ts"

test("R1 开关几何留白对称：行程 = trackWidth - thumbSize - inset * 2", () => {
  for (const [name, g] of Object.entries(LIQUID_SWITCH_GEOMETRY)) {
    assert.equal(
      g.travel,
      g.trackWidth - g.thumbSize - g.inset * 2,
      `${name} 规格的 travel 与 inset/thumbSize 不一致`,
    )
    assert.equal(g.trackHeight - g.thumbSize, g.inset * 2, `${name} 规格上下留白不对称`)
  }
})

test("computeLiquidStretch：静止正圆、夹紧范围、按行程同比例缩放、高速封顶", () => {
  const mdTravel = LIQUID_SWITCH_GEOMETRY.md.travel
  const smTravel = LIQUID_SWITCH_GEOMETRY.sm.travel

  assert.equal(computeLiquidStretch(0, mdTravel), 1)
  assert.equal(computeLiquidStretch(Number.NaN, mdTravel), 1)
  assert.equal(computeLiquidStretch(Number.POSITIVE_INFINITY, mdTravel), 1)
  assert.equal(computeLiquidStretch(200, mdTravel, 0), 1)
  assert.equal(computeLiquidStretch(200, mdTravel, -20), 1)

  // 正负速度对称
  assert.equal(
    computeLiquidStretch(90, mdTravel, 36),
    computeLiquidStretch(-90, mdTravel, 36),
  )

  // sm (12px) 与 md (18px) 在相同归一化速度下给出同一拉伸倍数
  const mdK = computeLiquidStretch(85, mdTravel, LIQUID_DEFAULT_STRETCH)
  const smK = computeLiquidStretch((85 * smTravel) / mdTravel, smTravel, LIQUID_DEFAULT_STRETCH)
  assert.ok(Math.abs(mdK - smK) < 1e-12, `md=${mdK} sm=${smK} 未同比例对齐`)
  assert.ok(mdK > 1.15 && mdK < 1.3, `默认点击峰值拉伸应明显可见 (${mdK})`)

  // 极限速度严格封顶
  const maxAtDefault = 1 + LIQUID_STRETCH_CAP * (LIQUID_DEFAULT_STRETCH / 100)
  assert.ok(Math.abs(computeLiquidStretch(10_000, mdTravel) - maxAtDefault) < 1e-12)
  assert.ok(Math.abs(computeLiquidStretch(10_000, mdTravel, 200) - (1 + LIQUID_STRETCH_CAP)) < 1e-12)
})

test("computeLiquidScale：保持面积守恒并将悬停微胀合入同一缩放", () => {
  const rest = computeLiquidScale(0, 1)
  assert.deepEqual(rest, { wide: 1, tall: 1 })

  const hovered = computeLiquidScale(0, LIQUID_HOVER_SWELL)
  assert.deepEqual(hovered, { wide: LIQUID_HOVER_SWELL, tall: LIQUID_HOVER_SWELL })

  for (const v of [25, 85, -160, 500]) {
    const moving = computeLiquidScale(v, LIQUID_HOVER_SWELL, LIQUID_SWITCH_GEOMETRY.md.travel, 48)
    assert.ok(moving.wide > moving.tall, `移动时应水平拉长 (${v})`)
    assert.ok(
      Math.abs(moving.wide * moving.tall - LIQUID_HOVER_SWELL * LIQUID_HOVER_SWELL) < 1e-12,
      `面积未守恒: wide*tall=${moving.wide * moving.tall}`,
    )
  }
})

test("computeLocalX：1x 与 CSS zoom 下坐标严格跟手，zero offsetWidth 不 NaN", () => {
  assert.equal(computeLocalX(123, 100, 46, 46), 23)
  // zoom = 1.25 时屏幕上的 12.5px 位移等价于组件内部 10px
  assert.ok(Math.abs(computeLocalX(112.5, 100, 57.5, 46) - 10) < 1e-12)
  assert.equal(computeLocalX(110, 100, 46, 0), 10)
  assert.equal(clamp(25, 0, 18), 18)
})

test("shouldFlipOnDrag：过半即翻边界准确", () => {
  assert.equal(shouldFlipOnDrag(8.9, 0, 18), false)
  assert.equal(shouldFlipOnDrag(9, 0, 18), false)
  assert.equal(shouldFlipOnDrag(9.01, 0, 18), true)
  assert.equal(shouldFlipOnDrag(6, 0, 12), false)
  assert.equal(shouldFlipOnDrag(6.05, 0, 12), true)
})

test("SPRING 与 TACTILE 预设完整且全部使用弹簧语义", () => {
  assert.equal(SPRING.liquidSettle.type, "spring")
  assert.ok(SPRING.liquidSettle.stiffness === 170 && SPRING.liquidSettle.damping === 21.5)
  assert.equal(TACTILE.row.hover, undefined, "列表行悬停不放大以免溢出裁切")
  for (const [name, spec] of Object.entries(TACTILE)) {
    assert.equal(spec.transition.type, "spring", `${name} 应使用弹簧过渡`)
    assert.ok(spec.tap.scale < 1 && spec.tap.scale > 0.85, `${name} tap 缩放比例异常`)
  }
})
