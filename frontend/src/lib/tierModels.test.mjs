import test from "node:test"
import assert from "node:assert/strict"
import { composeTierValue, emptyTierDraft, parseTierValue, tierDraftFrom, tierEfforts, tierModelsFrom, tierSummary } from "./tierModels.ts"

test("档位值拆分：只认 effort 词，模型名里的冒号保留", () => {
  assert.deepEqual(parseTierValue("zcode/glm-5.3-flash:low"), { model: "zcode/glm-5.3-flash", effort: "low" })
  assert.deepEqual(parseTierValue("local/qwen3:32b"), { model: "local/qwen3:32b", effort: null })
  assert.deepEqual(parseTierValue("claude/claude-opus-5-5[1m]:max"), { model: "claude/claude-opus-5-5[1m]", effort: "max" })
  assert.equal(composeTierValue("a/b", null), "a/b")
})

test("effort 范围与后端一致", () => {
  assert.deepEqual(tierEfforts("claude/claude-opus-4-5"), ["low", "medium", "high"])
  assert.deepEqual(tierEfforts("claude-sonnet-4.6"), ["low", "medium", "high", "max"])
  assert.deepEqual(tierEfforts("claude-haiku-4-5"), [])
  assert.equal(tierEfforts("claude-opus-5-5[1m]").length, 5)
  assert.equal(tierEfforts("zcode/glm-5.3-flash").length, 5)
})

test("草稿转后端值：跟随子代理由前端填值，模型不收的 effort 丢掉，全跟随为 null", () => {
  const draft = emptyTierDraft()
  assert.equal(tierModelsFrom(draft, "a/main", ""), null)
  draft.models.opus = "claude/claude-opus-4-5"
  draft.efforts.opus = "max"
  draft.models.haiku = "zcode/glm-5.3-flash"
  draft.efforts.haiku = "low"
  draft.auto = { kind: "subagent" }
  assert.deepEqual(tierModelsFrom(draft, "a/main", ""), { opus: "claude/claude-opus-4-5", haiku: "zcode/glm-5.3-flash:low", autoMode: "a/main" })
  assert.equal(tierModelsFrom(draft, "a/main", "b/sub").autoMode, "b/sub")
})

test("后端值回填草稿：Auto 等于子代理值时视为跟随子代理", () => {
  const draft = tierDraftFrom({ sonnet: "x/y:high", autoMode: "b/sub" }, "b/sub")
  assert.equal(draft.models.sonnet, "x/y")
  assert.equal(draft.efforts.sonnet, "high")
  assert.deepEqual(draft.auto, { kind: "subagent" })
  assert.deepEqual(tierDraftFrom({ autoMode: "c/other" }, "b/sub").auto, { kind: "model", model: "c/other" })
  assert.match(tierSummary(draft, "opus", "a/main"), /跟随主模型 · a\/main/)
  assert.equal(tierSummary(draft, "sonnet", "a/main"), "自选 · x/y:high")
})
