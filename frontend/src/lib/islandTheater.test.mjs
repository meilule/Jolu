import assert from "node:assert/strict"
import test from "node:test"
import {
  CACHE_SAVINGS_MIN_USD,
  estimateCacheSavingsUsd,
  isLateNightHour,
  makeCoffeeCue,
  makeFlagCue,
  makeGiftCue,
  makeUtensilsCue,
  nextTokenMilestone,
} from "./islandTheater.ts"

test("里程碑彩带按 1M / 10M 逐档触发且每日每档最多一次", () => {
  const seen = new Set()
  assert.equal(nextTokenMilestone(999_999, seen), null)
  assert.equal(nextTokenMilestone(1_000_000, seen)?.key, "1m")
  seen.add("1m")
  assert.equal(nextTokenMilestone(4_500_000, seen), null)
  assert.equal(nextTokenMilestone(10_200_000, seen)?.key, "10m")
  seen.add("10m")
  assert.equal(nextTokenMilestone(25_000_000, seen), null)
})

test("缓存省钱只按真实单价差估算，缺单价时返回 null 不伪造", () => {
  // 200K cache read × ($3.00 - $0.30)/M = $0.54 >= $0.50
  const saved = estimateCacheSavingsUsd(200_000, 3.0, 0.3)
  assert.ok(saved !== null && Math.abs(saved - 0.54) < 0.001)
  assert.ok(saved >= CACHE_SAVINGS_MIN_USD)
  assert.equal(estimateCacheSavingsUsd(200_000, null, null), null)
  assert.equal(estimateCacheSavingsUsd(0, 3.0, 0.3), null)
})

test("深夜咖啡仅在 00:00–05:00 生效且为静默陪伴不打断", () => {
  assert.equal(isLateNightHour(0), true)
  assert.equal(isLateNightHour(2), true)
  assert.equal(isLateNightHour(4), true)
  assert.equal(isLateNightHour(5), false)
  assert.equal(isLateNightHour(23), false)

  const coffee = makeCoffeeCue("Claude Code", new Date(2026, 10, 25, 2, 14))
  assert.equal(coffee.prop, "coffee")
  assert.equal(coffee.quiet, true)
  assert.equal(coffee.durationMs, null)
  assert.ok(coffee.title.includes("02:14"))
})

test("道具剧场工厂正确携带跳转动作与表情映射", () => {
  const gift = makeGiftCue(3)
  assert.equal(gift.prop, "gift")
  assert.equal(gift.action.kind, "checkin")

  const ctx = makeUtensilsCue(0.84, "claude", "sess-1")
  assert.equal(ctx.prop, "utensils")
  assert.equal(ctx.expr, "surprise")
  assert.deepEqual(ctx.action, { kind: "session", platform: "claude", sessionId: "sess-1" })

  const flag = makeFlagCue("Codex", 12 * 60_000, "codex", "sess-2")
  assert.equal(flag.prop, "flag")
  assert.equal(flag.expr, "greet")
  assert.ok(flag.title.includes("12 分钟"))
})
