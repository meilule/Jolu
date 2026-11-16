import test from "node:test"
import assert from "node:assert/strict"
import { createLatestGate } from "./latestGate.ts"

test("先发的请求后到时被丢弃，只认最新一次", async () => {
  const gate = createLatestGate()
  let shown = null
  const load = async (value, delay) => {
    const isLatest = gate.begin()
    await new Promise((resolve) => setTimeout(resolve, delay))
    if (isLatest()) shown = value
  }
  // 第一次慢、第二次快：慢的那次后到，不能盖掉
  await Promise.all([load("旧", 30), load("新", 5)])
  assert.equal(shown, "新")
})

test("依次完成的请求都照常写回", async () => {
  const gate = createLatestGate()
  const seen = []
  for (const value of [1, 2, 3]) {
    const isLatest = gate.begin()
    await Promise.resolve()
    if (isLatest()) seen.push(value)
  }
  assert.deepEqual(seen, [1, 2, 3])
})

test("invalidate 之后，进行中的请求全部作废", () => {
  const gate = createLatestGate()
  const first = gate.begin()
  const second = gate.begin()
  gate.invalidate()
  assert.equal(first(), false)
  assert.equal(second(), false)
  assert.equal(gate.begin()(), true, "作废之后新开的请求照常有效")
})

test("每个闸门各自计数，互不影响", () => {
  const a = createLatestGate()
  const b = createLatestGate()
  const fromA = a.begin()
  b.begin()
  assert.equal(fromA(), true)
})
