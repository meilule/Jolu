import assert from "node:assert/strict"
import test from "node:test"
import { clampRadii, edgeCornerRadii, squirclePath } from "./squircle.ts"

test("clampRadii 按比例收紧超出边长的相邻圆角，不出现自交或负数", () => {
  assert.deepEqual(clampRadii(40, 40, [20, 20, 20, 20]), [20, 20, 20, 20])
  assert.deepEqual(clampRadii(40, 20, [20, 20, 20, 20]), [10, 10, 10, 10])
  assert.deepEqual(clampRadii(0, 40, [20, 20, 20, 20]), [0, 0, 0, 0])
  assert.deepEqual(clampRadii(100, 40, [-5, Number.NaN, 12, 8]), [0, 0, 12, 8])
})

test("edgeCornerRadii 让靠屏幕边两角归零，其余两角保持圆角", () => {
  assert.deepEqual(edgeCornerRadii(20, null), [20, 20, 20, 20])
  assert.deepEqual(edgeCornerRadii(20, "top"), [0, 0, 20, 20])
  assert.deepEqual(edgeCornerRadii(20, "bottom"), [20, 20, 0, 0])
  assert.deepEqual(edgeCornerRadii(20, "left"), [0, 20, 20, 0])
  assert.deepEqual(edgeCornerRadii(20, "right"), [20, 0, 0, 20])
})

test("squirclePath 生成合法封闭路径且不产生 NaN，直角不发贝塞尔弯段", () => {
  const pill = squirclePath(200, 40, [20, 20, 20, 20])
  assert.ok(pill.startsWith("M "))
  assert.ok(pill.endsWith(" Z"))
  assert.ok(!pill.includes("NaN"))
  assert.equal((pill.match(/\bC\b/g) ?? []).length, 4)

  // 顶部贴边：上两角 0px，只有下两角画贝塞尔弯段
  const topDocked = squirclePath(200, 40, edgeCornerRadii(20, "top"))
  assert.equal((topDocked.match(/\bC\b/g) ?? []).length, 2)
  assert.ok(topDocked.startsWith("M 0 0 L 200 0"))

  // 零尺寸安全返回空串
  assert.equal(squirclePath(0, 40, [20, 20, 20, 20]), "")
})
