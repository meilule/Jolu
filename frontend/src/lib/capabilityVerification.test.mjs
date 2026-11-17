import { test } from "node:test"
import assert from "node:assert/strict"

import { CAPABILITIES, CELLS, PLATFORMS, STATE_LABEL, cellOf, cellSummary, stateCounts } from "./capabilityVerification.ts"

test("每个平台 × 能力都有且只有一格", () => {
  const keys = CELLS.map((cell) => `${cell.platform}:${cell.capability}`)
  assert.equal(new Set(keys).size, keys.length, "不能重复")
  assert.equal(CELLS.length, PLATFORMS.length * CAPABILITIES.length)
  for (const p of PLATFORMS) for (const c of CAPABILITIES) assert.ok(keys.includes(`${p.id}:${c.id}`), `${p.id}:${c.id}`)
})

test("已验证 / 部分验证必须有依据日期与记录文件；无记录的没有文件；日期格式统一", () => {
  for (const cell of CELLS) {
    if (cell.state === "verified" || cell.state === "partial") {
      assert.match(cell.date ?? "", /^\d{4}-\d{2}-\d{2}$/, `${cell.platform}:${cell.capability}`)
      assert.ok(cell.record?.startsWith("docs/"), `${cell.platform}:${cell.capability} 要指向验证记录`)
    }
    if (cell.state === "none") assert.equal(cell.record, null)
    if (cell.date) assert.match(cell.date, /^\d{4}-\d{2}-\d{2}$/)
    assert.ok(cell.evidence.length > 0)
  }
})

test("查不到的格当作无记录，不推断；计数与文字", () => {
  assert.equal(cellOf("unknown", "login").state, "none")
  assert.equal(cellOf("codex", "quota").state, "verified")
  const counts = stateCounts()
  assert.equal(Object.values(counts).reduce((a, b) => a + b, 0), CELLS.length)
  assert.equal(STATE_LABEL.partial, "部分验证")
  assert.match(cellSummary(cellOf("claude", "sessions")), /^已验证 · 2026-10-24 · 依据 docs\/verification\//)
})
