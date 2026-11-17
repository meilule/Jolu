import { test } from "node:test"
import assert from "node:assert/strict"

import { outcomeSummary, sharedAgentLabel, subagentNameError, targetText } from "./librarySharedView.ts"

test("目标状态：未写入 / 已写入 / 被改过分开说", () => {
  assert.deepEqual(targetText({ agent: "codex", path: null, written: false, changed: false }), { text: "未写入", tone: "muted" })
  assert.equal(targetText({ agent: "codex", path: "x", written: true, changed: false }).tone, "ok")
  assert.equal(targetText({ agent: "codex", path: "x", written: true, changed: true }).tone, "warn")
})

test("保存结果汇总：跳过的写明原因", () => {
  assert.deepEqual(outcomeSummary([]), { text: "没有需要改动的工具", warn: false })
  const summary = outcomeSummary([
    { agent: "claude-code", path: "a", action: "written", reason: null },
    { agent: "codex", path: "b", action: "removed", reason: null },
    { agent: "gemini", path: "c", action: "skipped", reason: "同名文件不是 Jolu 写的，不覆盖" },
  ])
  assert.equal(summary.warn, true)
  assert.equal(summary.text, "写入 1 处，移除 1 处，跳过 1 处：Gemini CLI（同名文件不是 Jolu 写的，不覆盖）")
  assert.equal(sharedAgentLabel("unknown"), "unknown")
})

test("子代理名称规则与后端一致", () => {
  assert.equal(subagentNameError("code-reviewer"), null)
  for (const bad of ["A", "-ab", "ab-", "a b", "x", "main"]) assert.ok(subagentNameError(bad), bad)
})
