import assert from "node:assert/strict"
import test from "node:test"
import {
  groupImportDrafts, importToastDetail, importToastTitle, isImportSourceError,
  protocolLabel, selectableReferences, sourceLabel, toggleReference,
} from "./importDrafts.ts"

/** @param {Partial<import("./importDrafts.ts").ImportDraft>} overrides */
const draft = (overrides = {}) => ({
  reference: "claude-code:env",
  source: "claude-code",
  from: "settings.json env",
  name: "我的中转",
  baseUrl: "https://relay.example.com",
  protocol: "anthropic",
  keyMasked: "••••••••abcd",
  ...overrides,
})

test("错误条目与草稿按 reference 区分", () => {
  assert.equal(isImportSourceError({ source: "codex", error: "config.toml 不存在" }), true)
  assert.equal(isImportSourceError(draft()), false)
})

test("按来源分组并保持扫描顺序，错误并入对应来源", () => {
  const groups = groupImportDrafts([
    draft({ reference: "claude-code:env" }),
    { source: "codex", error: "config.toml 解析失败" },
    draft({ reference: "cc-switch:a1", source: "cc-switch", from: "CC Switch·Claude", name: "CC 的 A" }),
    draft({ reference: "cc-switch:a2", source: "cc-switch", from: "CC Switch·Codex", name: "CC 的 B" }),
  ])
  assert.deepEqual(groups.map((g) => g.source), ["claude-code", "codex", "cc-switch"])
  assert.deepEqual(groups[0].drafts.map((d) => d.reference), ["claude-code:env"])
  assert.deepEqual(groups[0].errors, [])
  assert.deepEqual(groups[1].drafts, [])
  assert.deepEqual(groups[1].errors, ["config.toml 解析失败"])
  assert.equal(groups[2].drafts.length, 2)
})

test("未知来源的显示名退回原始 source", () => {
  assert.equal(sourceLabel("claude-code"), "Claude Code")
  assert.equal(sourceLabel("cc-switch"), "CC Switch")
  assert.equal(sourceLabel("future-app"), "future-app")
  assert.equal(protocolLabel("openai"), "OpenAI")
  assert.equal(protocolLabel("anthropic"), "Anthropic")
})

test("默认全选草稿；错误条目不参与；勾选只影响对应引用", () => {
  const groups = groupImportDrafts([
    draft({ reference: "a" }),
    draft({ reference: "b", source: "cc-switch", from: "x", name: "y" }),
    { source: "codex", error: "无法定位用户目录" },
  ])
  assert.deepEqual(selectableReferences(groups), ["a", "b"])
  const selected = new Set(selectableReferences(groups))
  assert.deepEqual(new Set(toggleReference(selected, "a")), new Set(["b"]))
  assert.deepEqual(new Set(toggleReference(toggleReference(selected, "a"), "a")), new Set(["a", "b"]))
})

test("导入完成文案：标题计数、详情只留名称去掉 id", () => {
  assert.equal(importToastTitle(["甲（id-1）", "乙（id-2）"]), "已导入 2 个连接")
  assert.equal(importToastDetail(["Claude Code · 中转（abc-123）", "CC Switch · 备用（def-456）"]), "Claude Code · 中转、CC Switch · 备用")
  assert.equal(importToastDetail([]), undefined)
})
