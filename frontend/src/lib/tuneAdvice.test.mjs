import { test } from "node:test"
import assert from "node:assert/strict"

import { agentName, compactionLine, kTokens, savedShare, ttlLine } from "./tuneAdvice.ts"

const facts = { calls: 100, streams: 3, compacts: 4, quick: 0.9, pause: 0.08, away: 0.02 }

test("数字与名称", () => {
  assert.equal(kTokens(167_000), "167K")
  assert.equal(savedShare(100, 64), "36%")
  assert.equal(savedShare(0, 0), "0%")
  assert.equal(agentName("claude"), "Claude Code")
})

test("压缩窗口：保持 / 建议改 / 窗口未知要提醒", () => {
  const keep = compactionLine({ model: "m", current: 200_000, window: 200_000, best: 200_000, cost: 10, low: 10, curve: [], facts }, null)
  assert.equal(keep.change, false)
  assert.match(keep.title, /没设，按压缩点推断/)
  const change = compactionLine({ model: "m", current: 360_000, window: 0, best: 1_000_000, cost: 100, low: 57, curve: [], facts }, 360_000)
  assert.equal(change.title, "m：现在 360K → 建议 1000K，加权 Token 省 43%")
  assert.match(change.detail, /模型窗口未知/)
})

test("缓存时长：推断的当前值写明", () => {
  const line = ttlLine({ current: "5m", current_inferred: true, best: "1h", cost: 100, low: 80, facts })
  assert.equal(line.change, true)
  assert.match(line.title, /按 5 分钟推断/)
  assert.match(line.detail, /5 分钟内 90%/)
})
