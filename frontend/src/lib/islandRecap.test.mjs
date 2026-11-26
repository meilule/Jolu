import assert from "node:assert/strict"
import test from "node:test"
import {
  computeCheckinStreak,
  evaluateAchievements,
  formatRecapDate,
  RECAP_IDLE_MS,
  shouldShowDailyRecap,
} from "./islandRecap.ts"

test("今日回顾仅在 18:00 后、空闲 ≥ 30 分钟、非免打扰且今日未关闭时触发", () => {
  const base = { enabled: true, dnd: false, hour: 18, writing: false, inactiveMs: RECAP_IDLE_MS, dismissedToday: false }
  assert.equal(shouldShowDailyRecap(base), true)
  assert.equal(shouldShowDailyRecap({ ...base, hour: 17 }), false)
  assert.equal(shouldShowDailyRecap({ ...base, inactiveMs: RECAP_IDLE_MS - 1 }), false)
  assert.equal(shouldShowDailyRecap({ ...base, writing: true }), false)
  assert.equal(shouldShowDailyRecap({ ...base, dnd: true }), false)
  assert.equal(shouldShowDailyRecap({ ...base, dismissedToday: true }), false)
  assert.equal(shouldShowDailyRecap({ ...base, enabled: false }), false)
})

test("连续签到天数按本地日历天倒推，不因同一天多条记录虚增", () => {
  const now = new Date(2026, 10, 25, 19, 0)
  const ts = (dayOffset) => new Date(2026, 10, 25 - dayOffset, 9, 30).getTime()
  assert.equal(computeCheckinStreak([ts(0), ts(0), ts(1), ts(2), ts(3), ts(4), ts(5), ts(6)], now), 7)
  // 断签一天后只剩 2 天
  assert.equal(computeCheckinStreak([ts(0), ts(1), ts(3), ts(4)], now), 2)
  assert.equal(computeCheckinStreak([], now), 0)
})

test("6 枚成就按真实记录评估，保留历史解锁日期不重复覆盖", () => {
  const existing = { three_agents: { unlockedOn: "2026-11-20" } }
  const { nextStore, unlockedToday, changed } = evaluateAchievements(
    {
      checkinStreakDays: 7,
      cacheHitRate: 0.92,
      todayRequests: 15,
      wiredAgentCount: 3,
      todayRouteRequests: 40,
      todayRouteFailures: 0,
      wroteAfter2am: true,
      todayReroutes: 12,
    },
    existing,
    "2026-11-25",
  )
  assert.equal(changed, true)
  assert.equal(nextStore.three_agents?.unlockedOn, "2026-11-20")
  assert.equal(nextStore.checkin_streak_7?.unlockedOn, "2026-11-25")
  assert.equal(nextStore.cache_master?.unlockedOn, "2026-11-25")
  assert.equal(nextStore.zero_failure_day?.unlockedOn, "2026-11-25")
  assert.equal(nextStore.night_owl?.unlockedOn, "2026-11-25")
  assert.equal(nextStore.relay_king?.unlockedOn, "2026-11-25")
  assert.equal(unlockedToday.length, 5)
  assert.ok(formatRecapDate(new Date(2026, 10, 25)).includes("11 月 25 日"))
})
