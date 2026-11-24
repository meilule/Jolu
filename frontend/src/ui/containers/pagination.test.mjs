/**
 * `pageItems` 的折叠规则。这是唯一有真实分支的纯逻辑，其余组件都是渲染。
 *
 * 规则：始终保留首页与末页，当前页两侧各留一个邻居，其余折成 null（省略号）；
 * 靠近两端时把另一侧额度让出来，保持总宽度稳定。
 */
import test from "node:test"
import assert from "node:assert/strict"

import { pageItems } from "./pageItems.ts"

test("页数不超过 7 时全部列出，不折叠", () => {
  assert.deepEqual(pageItems(1, 1), [1])
  assert.deepEqual(pageItems(4, 7), [1, 2, 3, 4, 5, 6, 7])
})

test("当前页在中间时两侧都折", () => {
  assert.deepEqual(pageItems(5, 42), [1, null, 4, 5, 6, null, 42])
})

test("当前页靠左时左侧不折，右侧多显示几页补齐宽度", () => {
  // 当前 3：左侧 1、2 连续，不需要省略号；右侧补到 5 保持总项数 7
  assert.deepEqual(pageItems(3, 42), [1, 2, 3, 4, 5, null, 42])
})

test("当前页靠右时右侧不折，左侧多显示几页补齐宽度", () => {
  assert.deepEqual(pageItems(41, 42), [1, null, 38, 39, 40, 41, 42])
})

test("当前页是首页或末页时不出现重复页码", () => {
  assert.deepEqual(pageItems(1, 42), [1, 2, 3, 4, 5, null, 42])
  // 末页：1、省略号、38–42。不能出现两个 42
  assert.deepEqual(pageItems(42, 42), [1, null, 38, 39, 40, 41, 42])
})

test("边界：刚好 8 页时开始折叠", () => {
  const items = pageItems(4, 8)
  assert.equal(items[0], 1, "首页必在")
  assert.equal(items[items.length - 1], 8, "末页必在")
  assert.ok(items.includes(4), "当前页必在")
  // 不允许出现连续两个省略号
  for (let index = 1; index < items.length; index += 1) {
    assert.ok(!(items[index] === null && items[index - 1] === null), "不能有连续省略号")
  }
})
