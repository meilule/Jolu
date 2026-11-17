import { test } from "node:test"
import assert from "node:assert/strict"

import { SIGNAL_MERGE_MS, SIGNAL_SHOW_MS, emptySignals, signalVisible, stepSignals } from "./islandSignals.ts"

const input = (over = {}) => ({ usedPercents: [10], sessionIds: ["a"], rejection: null, ...over })

test("第一次只记基线，不报信号", () => {
  const { signal, state } = stepSignals(emptySignals(), input({ usedPercents: [100], sessionIds: ["a", "b"] }), 1_000)
  assert.equal(signal, null)
  assert.deepEqual(state.last.usedPercents, [100])
})

test("跨过 80%、用尽、网关拒绝、新会话各报一次；同时发生只报最严重的", () => {
  let s = stepSignals(emptySignals(), input(), 0).state
  let r = stepSignals(s, input({ usedPercents: [85] }), 1_000)
  assert.equal(r.signal.kind, "quota-80")
  r = stepSignals(r.state, input({ usedPercents: [90] }), 2_000)
  assert.equal(r.signal, null, "已经在 80% 以上，不重复")
  r = stepSignals(r.state, input({ usedPercents: [100], sessionIds: ["a", "b"] }), 3_000)
  assert.equal(r.signal.kind, "quota-exhausted", "用尽比新会话严重")
  r = stepSignals(r.state, input({ usedPercents: [100], sessionIds: ["a", "b"], rejection: "429@4000" }), 4_000)
  assert.equal(r.signal.kind, "rejected")
  r = stepSignals(r.state, input({ usedPercents: [100], sessionIds: ["a", "b", "c"], rejection: "429@4000" }), 10_000)
  assert.equal(r.signal.kind, "new-session")
})

test("同一种信号 5 秒内合并；显示 1.5 秒", () => {
  let s = stepSignals(emptySignals(), input({ sessionIds: [] }), 0).state
  let r = stepSignals(s, input({ sessionIds: ["x"] }), 1_000)
  assert.equal(r.signal.kind, "new-session")
  r = stepSignals(r.state, input({ sessionIds: ["x", "y"] }), 1_000 + SIGNAL_MERGE_MS - 1)
  assert.equal(r.signal, null)
  r = stepSignals(r.state, input({ sessionIds: ["x", "y", "z"] }), 1_000 + SIGNAL_MERGE_MS + 1)
  assert.equal(r.signal.kind, "new-session")
  assert.equal(signalVisible({ kind: "quota-80", at: 0 }, SIGNAL_SHOW_MS - 1), true)
  assert.equal(signalVisible({ kind: "quota-80", at: 0 }, SIGNAL_SHOW_MS), false)
  assert.equal(signalVisible(null, 0), false)
})

test("额度回落后再次跨过 80% 会再报", () => {
  let s = stepSignals(emptySignals(), input(), 0).state
  let r = stepSignals(s, input({ usedPercents: [85] }), 1_000)
  r = stepSignals(r.state, input({ usedPercents: [5] }), 100_000)
  r = stepSignals(r.state, input({ usedPercents: [81] }), 200_000)
  assert.equal(r.signal.kind, "quota-80")
})
