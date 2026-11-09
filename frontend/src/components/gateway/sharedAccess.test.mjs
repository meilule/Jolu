import assert from "node:assert/strict"
import test from "node:test"

const {
  limitText,
  usageText,
  isExhausted,
  lastUsedText,
  sharingWithoutKeys,
  KEY_ONCE_NOTICE,
  TOKENS_NOT_ENFORCED_NOTICE,
} = await import("./sharedAccess.ts")

test("每日上限未设置时显示「不限」而不是 0", () => {
  assert.equal(limitText(null), "不限")
  assert.equal(limitText(undefined), "不限")
  assert.equal(limitText(0), "不限")
  assert.equal(limitText(100), "100")
})

test("今日用量有限额时给进度感，无限额只给数字", () => {
  assert.equal(usageText({ today_requests: 12, requests_per_day: 100 }), "12 / 100")
  assert.equal(usageText({ today_requests: 12, requests_per_day: null }), "12")
  assert.equal(usageText({ today_requests: 0, requests_per_day: 0 }), "0")
})

test("用满当日上限要能判定出来（后续请求会被 429）", () => {
  assert.equal(isExhausted({ today_requests: 100, requests_per_day: 100 }), true)
  assert.equal(isExhausted({ today_requests: 101, requests_per_day: 100 }), true)
  assert.equal(isExhausted({ today_requests: 99, requests_per_day: 100 }), false)
  assert.equal(isExhausted({ today_requests: 999, requests_per_day: null }), false, "无限额不算用满")
})

test("最近使用：从未使用给明确文案，不显示 1970", () => {
  const now = 1_800_000_000_000
  assert.equal(lastUsedText(null, now), "从未使用")
  assert.equal(lastUsedText(0, now), "从未使用")
  assert.equal(lastUsedText(now - 30_000, now), "刚刚")
  assert.equal(lastUsedText(now - 5 * 60_000, now), "5 分钟前")
  assert.equal(lastUsedText(now - 3 * 3_600_000, now), "3 小时前")
  assert.equal(lastUsedText(now - 2 * 86_400_000, now), "2 天前")
  assert.doesNotMatch(lastUsedText(now - 400 * 86_400_000, now), /1970/)
})

test("开了共享但没有 Key 属于危险态", () => {
  assert.equal(sharingWithoutKeys(true, 0), true, "开了共享却没 Key：局域网设备进不来")
  assert.equal(sharingWithoutKeys(true, 1), false)
  assert.equal(sharingWithoutKeys(false, 0), false, "没开共享就不是危险态")
})

test("Key 原文只显示一次与 Token 上限不强制都要如实标注", () => {
  assert.match(KEY_ONCE_NOTICE, /只显示这一次/)
  assert.match(KEY_ONCE_NOTICE, /无法再查看/)
  assert.match(TOKENS_NOT_ENFORCED_NOTICE, /只记录/)
  assert.match(TOKENS_NOT_ENFORCED_NOTICE, /不做强制拦截/)
})
