import assert from "node:assert/strict"
import test from "node:test"
import { gazeAmp, gazeBase, gazeSizeGain, gazeSmooth, gazeStep, gazeTarget, GAZE_MAX_X, GAZE_MAX_Y, GAZE_SIZE_GAIN_CAP } from "./botGaze.ts"

const face = { cx: 100, cy: 100, width: 28 }
const opt = { base: { x: 0, y: 0 }, amp: 1 }

test("方向与指针一致", () => {
  const t = gazeTarget(face, { x: 200, y: 140 }, opt)
  const cos = (t.x * 100 + t.y * 40) / (Math.hypot(t.x, t.y) * Math.hypot(100, 40))
  assert.ok(cos > 0.9)
})

test("偏移不超过 13% × 尺寸补偿上限（水平）与 10% × 上限（垂直）", () => {
  const t = gazeTarget(face, { x: 5000, y: 5000 }, opt)
  assert.ok(Math.abs(t.x) <= GAZE_MAX_X * GAZE_SIZE_GAIN_CAP + 1e-9, `横向 ${t.x} 超出补偿上限`)
  assert.ok(Math.abs(t.y) <= GAZE_MAX_Y * GAZE_SIZE_GAIN_CAP + 1e-9, `纵向 ${t.y} 超出补偿上限`)
  // 水平方向指针：横向取满，纵向只有指针的小角度分量
  const far = gazeTarget(face, { x: -5000, y: 100 }, opt)
  assert.ok(far.x < -GAZE_MAX_X, "28px 的小脸要做尺寸补偿，超过名义 13%")
  assert.ok(far.x >= -GAZE_MAX_X * GAZE_SIZE_GAIN_CAP - 1e-9)
})

test("尺寸补偿：小脸放大、大脸保持 1×、未知尺寸不补偿", () => {
  assert.equal(gazeSizeGain(84), 1, "展开的大脸不做额外放大")
  assert.equal(gazeSizeGain(200), 1, "更大的脸也不缩小")
  // 28px 的原始倍率是 sqrt(84/28)=1.73，超过上限，所以取封顶值
  assert.ok(Math.sqrt(84 / 28) > GAZE_SIZE_GAIN_CAP, "28px 的原始倍率应高于上限（这条锁住上限确实是生效的）")
  assert.equal(gazeSizeGain(28), GAZE_SIZE_GAIN_CAP, "28px 截在上限")
  assert.equal(gazeSizeGain(1), GAZE_SIZE_GAIN_CAP, "极小脸也截在上限内（球面投影会饱和）")
  assert.equal(gazeSizeGain(0), 1, "脸尺寸未知按 1×")
  assert.equal(gazeSizeGain(Number.NaN), 1, "非法尺寸按 1×")
  // 同一指针位置，小脸的绝对位移更大，否则在 18px 胶囊上看不出跟随
  const small = gazeTarget({ cx: 100, cy: 100, width: 18 }, { x: 5000, y: 100 }, opt)
  const big = gazeTarget({ cx: 100, cy: 100, width: 84 }, { x: 5000, y: 100 }, opt)
  assert.ok(small.x > big.x * 1.3, `小脸 ${small.x} 应明显大于大脸 ${big.x}`)
})

test("距离越近偏移越小，指针在脸上不偏移", () => {
  const near = gazeTarget(face, { x: 110, y: 100 }, opt)
  const far = gazeTarget(face, { x: 180, y: 100 }, opt)
  assert.ok(near.x < far.x)
  assert.deepEqual(gazeTarget(face, { x: 100, y: 100 }, opt), { x: 0, y: 0 })
})

test("没有指针时只剩表情偏移，幅度为 0 时不跟随", () => {
  assert.deepEqual(gazeTarget(face, null, { base: { x: 0.2, y: 0 }, amp: 1 }), { x: 0.2, y: 0 })
  assert.deepEqual(gazeTarget(face, { x: 500, y: 500 }, { base: { x: 0, y: 0 }, amp: 0 }), { x: 0, y: 0 })
})

test("睡着、偷看、被拖动按幅度减弱", () => {
  assert.equal(gazeAmp("sleep"), 0.3)
  // peek 从 0.4 提到 0.75：小脸绝对位移本来就小，压太狠就看不见跟随
  assert.equal(gazeAmp("peek"), 0.75)
  assert.equal(gazeAmp("drag"), 0.6)
  assert.equal(gazeAmp("idle"), 1)
  const full = gazeTarget(face, { x: 5000, y: 100 }, opt)
  const sleepy = gazeTarget(face, { x: 5000, y: 100 }, { base: { x: 0, y: 0 }, amp: gazeAmp("sleep") })
  assert.ok(Math.abs(sleepy.x - full.x * 0.3) < 1e-9)
})

test("表情不再压固定视线偏移：跟随完全交给指针", () => {
  // peek 曾经给「朝屏幕内侧」的固定偏移，在小脸上把跟随压死；
  // 现在偷看只靠眼型（EYES.peek）表达，base 恒为 0，任何一边都不偏。
  for (const edge of ["top", "bottom", "left", "right", null]) {
    assert.deepEqual(gazeBase("peek", edge), { x: 0, y: 0 }, edge ?? "null")
  }
  assert.deepEqual(gazeBase("idle", "left"), { x: 0, y: 0 })
  assert.deepEqual(gazeBase("work", "top"), { x: 0, y: 0 })
  // 传进来的 base 仍原样进入目标（上层要临时加偏移时还有这条路）
  assert.deepEqual(gazeTarget(face, null, { base: { x: -0.1, y: 0.05 }, amp: 0 }), { x: -0.1, y: 0.05 })
})

test("插值逼近目标并最终落稳；减少动效一步到位", () => {
  let cur = { x: 0, y: 0 }
  const target = { x: 0.1, y: 0.05 }
  let settled = false
  for (let i = 0; i < 60 && !settled; i++) ({ next: cur, settled } = gazeStep(cur, target, gazeSmooth(false)))
  assert.equal(settled, true)
  assert.deepEqual(cur, target)
  assert.equal(gazeStep({ x: 0, y: 0 }, target, gazeSmooth(true)).settled, true)
})
