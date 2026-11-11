import assert from "node:assert/strict"
import test from "node:test"
import { MASKING_PLACEHOLDER_SAMPLE, PRIVACY_MASKING_RULES, maskingDisplay } from "./privacyMasking.ts"

test("脱敏范围与后端固定规则一一对应，共 7 条", () => {
  assert.deepEqual(PRIVACY_MASKING_RULES.map((rule) => rule.name), [
    "API Key",
    "私钥 PEM 段",
    "Bearer 令牌",
    "token= 凭证",
    "邮箱",
    "银行卡",
    "身份证号",
  ])
})

test("开关显示值：未保存前显示目标态，失败回滚到后端保存值", () => {
  assert.equal(maskingDisplay(false, null), false)
  assert.equal(maskingDisplay(false, true), true)
  assert.equal(maskingDisplay(true, false), false)
  // 保存失败把 applied 清回 null，显示退回后端真值
  assert.equal(maskingDisplay(false, null), false)
})

test("占位符示例与后端格式一致：{{JOLU_<类型>_<8位hash>}}", () => {
  assert.match(MASKING_PLACEHOLDER_SAMPLE, /^\{\{JOLU_[A-Z]+_[0-9a-f]{8}\}\}$/)
})

import { PRIVACY_MASKING_KINDS, actionOf, hitSides, parseAllow, recordSummary, withAction } from "./privacyMasking.ts"

test("可设置处理方式的种类与后端 TYPES 一致", () => {
  assert.deepEqual(PRIVACY_MASKING_KINDS.map((k) => k.kind), ["APIKEY", "PRIVATEKEY", "TOKEN", "EMAIL", "CARD", "IDCARD"])
})

test("处理方式只保存非默认项，未知值按遮住", () => {
  assert.equal(actionOf(undefined, "EMAIL"), "mask")
  assert.equal(actionOf({ EMAIL: "bogus" }, "EMAIL"), "mask")
  const next = withAction({ EMAIL: "warn", CARD: "block" }, "EMAIL", "mask")
  assert.deepEqual(next, { CARD: "block" })
  assert.deepEqual(withAction(next, "TOKEN", "warn"), { CARD: "block", TOKEN: "warn" })
})

test("豁免清单按行或逗号切开，不分大小写去重", () => {
  assert.deepEqual(parseAllow(" team@corp.io\nTEAM@corp.io , test-key，\n\n"), ["team@corp.io", "test-key"])
})

test("记录摘要与对照两侧", () => {
  const hit = { kind: "EMAIL", action: "mask", length: 13, preview: "a@…", placeholder: "{{JOLU_EMAIL_1a2b3c4d}}", left: "联系 ", right: " 即可" }
  assert.deepEqual(hitSides(hit), { before: "联系 【a@…（13 字）】 即可", after: "联系 {{JOLU_EMAIL_1a2b3c4d}} 即可" })
  const record = { id: 1, at_ms: 0, entry: "chat", blocked: false, masked: 1, hits: [hit, { ...hit, action: "warn", placeholder: null }] }
  assert.equal(recordSummary(record), "遮住 1 处 · 只记录 1 处")
  assert.equal(recordSummary({ ...record, blocked: true }), "已拦下，没有发出")
})
