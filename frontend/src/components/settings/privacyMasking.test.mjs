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
