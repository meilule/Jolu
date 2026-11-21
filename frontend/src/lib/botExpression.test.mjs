import assert from "node:assert/strict"
import test from "node:test"
import { BOT_NAMES, BOT_TRIGGERS, botExpression, quotaJustReset, SHOT_MS, SLEEP_AFTER_MS } from "./botExpression.ts"

const base = { connected: true, queryFailed: false, quotaUsed: [12, 59], dragging: false, refreshing: false, working: false }
const expr = (patch) => botExpression({ ...base, ...patch }).expr

test("没有任何状态时空闲", () => assert.equal(expr({}), "idle"))

test("优先级逐级压过低级", () => {
  assert.equal(expr({ queryFailed: true, relay: "rejected", quotaUsed: [100], dragging: true, working: true }), "error")
  assert.equal(expr({ relay: "rejected", quotaUsed: [100], dragging: true }), "worry")
  assert.equal(expr({ quotaUsed: [100, 90], dragging: true, working: true }), "sad")
  assert.equal(expr({ dragging: true, quotaUsed: [85], working: true }), "drag")
  assert.equal(expr({ quotaUsed: [85], waiting: true, refreshing: true }), "worry")
  assert.equal(expr({ waiting: true, refreshing: true, working: true }), "wait")
  assert.equal(expr({ refreshing: true, relay: "in", working: true }), "refresh")
  assert.equal(expr({ relay: "in", working: true }), "relay")
  assert.equal(expr({ relay: "ok", working: true }), "happy")
  assert.equal(expr({ working: true, shot: "surprise" }), "work")
})

test("一次性表情压过睡着与打招呼，打招呼低于睡着", () => {
  assert.equal(expr({ shot: "surprise", inactiveMs: SLEEP_AFTER_MS }), "surprise")
  assert.equal(expr({ shot: "greet", inactiveMs: SLEEP_AFTER_MS }), "sleep")
  assert.equal(expr({ shot: "greet", docked: true, hover: true }), "greet")
})

test("未知额度不当作 0，也不触发告警", () => {
  assert.equal(expr({ quotaUsed: [null, null] }), "idle")
  assert.equal(expr({ quotaUsed: [null, 80] }), "worry")
})

test("未连接睡着，且不报查询失败", () => {
  assert.equal(expr({ connected: false, queryFailed: true }), "sleep")
  assert.equal(expr({ inactiveMs: SLEEP_AFTER_MS - 1 }), "idle")
  assert.equal(expr({ inactiveMs: SLEEP_AFTER_MS }), "sleep")
})

test("停靠偷看低于悬停之外的低优先级，高于看向指针", () => {
  assert.equal(expr({ docked: true, hover: true }), "peek")
  assert.equal(expr({ hover: true }), "look")
})

test("额度重置只在高位回到低位时判定", () => {
  assert.equal(quotaJustReset({ "5h": 97 }, { "5h": 3 }), true)
  assert.equal(quotaJustReset({ "5h": 60 }, { "5h": 55 }), false)
  assert.equal(quotaJustReset({ "5h": null }, { "5h": 2 }), false)
  assert.equal(quotaJustReset({}, { "5h": 2 }), false)
})

test("每个表情都有中文名与触发说明（新增表情必须同时补 BOT_NAMES / BOT_TRIGGERS）", () => {
  const names = Object.keys(BOT_NAMES).sort()
  assert.deepEqual(Object.keys(BOT_TRIGGERS).sort(), names, "两个表的表情集合必须一致")
  for (const expr of names) {
    assert.ok(BOT_NAMES[expr].length > 0, `${expr} 缺中文名`)
    assert.ok(BOT_TRIGGERS[expr].length > 0, `${expr} 缺触发说明`)
  }
})

test("一次性表情的停留时长都有定义且为正", () => {
  for (const key of ["happy", "surprise", "greet", "reset"]) {
    assert.ok(SHOT_MS[key] > 0, `${key} 的停留时长应为正数`)
  }
})

test("peek 回归：展开态（docked=false）不得再压着视线", () => {
  // 顶部停靠 + 展开时 CapsuleIsland 传 docked: Boolean(dockEdge) && !open = false。
  // 若这里退回 peek，gazeBase 的固定下压会让眼睛永远朝下（历史 bug）。
  assert.equal(expr({ docked: false, hover: true }), "look")
  assert.equal(expr({ docked: false }), "idle")
})

test("wait 由 automationWaiting 触发，优先级低于担心高于刷新", () => {
  assert.equal(expr({ waiting: true }), "wait")
  assert.equal(expr({ waiting: true, quotaUsed: [85] }), "worry")
  assert.equal(expr({ waiting: true, refreshing: true }), "wait")
})

test("告警分界：79 空闲 / 80 担心 / 100 沮丧", () => {
  assert.equal(expr({ quotaUsed: [79] }), "idle")
  assert.equal(expr({ quotaUsed: [80] }), "worry")
  assert.equal(expr({ quotaUsed: [100] }), "sad")
})
