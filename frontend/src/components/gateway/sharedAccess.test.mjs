import assert from "node:assert/strict"
import test from "node:test"

const {
  limitText,
  usageText,
  isExhausted,
  lastUsedText,
  sharingWithoutKeys,
  KEY_ONCE_NOTICE,
  TOKENS_LIMIT_NOTICE,
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

test("Key 原文只显示一次与 Token 上限的预扣口径都要如实标注", () => {
  assert.match(KEY_ONCE_NOTICE, /只显示这一次/)
  assert.match(KEY_ONCE_NOTICE, /无法再查看/)
  assert.match(TOKENS_LIMIT_NOTICE, /预扣/)
  assert.match(TOKENS_LIMIT_NOTICE, /回冲/)
})

const { EXPIRY_CHOICES, expiresAtFrom, parseModelList, isExpired, constraintTags } = await import("./sharedAccess.ts")

const DAY = 24 * 60 * 60 * 1000

test("有效期选项：永不过期给 null，其余按天数折算到期时间", () => {
  assert.equal(EXPIRY_CHOICES[0].days, null)
  assert.equal(expiresAtFrom(null, 1_000), null)
  assert.equal(expiresAtFrom(7, 1_000), 1_000 + 7 * DAY)
  assert.ok(EXPIRY_CHOICES.every((choice) => choice.days === null || choice.days > 0))
})

test("模型白名单：逗号、空白或换行分隔，去空去重；留空表示不限", () => {
  assert.deepEqual(parseModelList("zcode/GLM-5.2, claude/opus\nzcode/GLM-5.2  "), ["zcode/GLM-5.2", "claude/opus"])
  assert.equal(parseModelList(""), null)
  assert.equal(parseModelList(" , \n "), null)
})

test("过期判定：null 永不过期，到点即过期", () => {
  assert.equal(isExpired({ expires_ms: null }, 5_000), false)
  assert.equal(isExpired({ expires_ms: 5_001 }, 5_000), false)
  assert.equal(isExpired({ expires_ms: 5_000 }, 5_000), true)
})

test("约束标签：过期、到期剩余、模型白名单、只读分别如实列出；无约束为空", () => {
  const now = 10 * DAY
  const base = { expires_ms: null, models: null, read_only: false }
  assert.deepEqual(constraintTags(base, now), [])
  assert.deepEqual(constraintTags({ ...base, expires_ms: now - 1 }, now), [{ text: "已过期", danger: true }])
  assert.deepEqual(constraintTags({ ...base, expires_ms: now + 3 * DAY }, now), [{ text: "3 天后过期", danger: false }])
  assert.deepEqual(constraintTags({ ...base, expires_ms: now + 60_000 }, now), [{ text: "今天过期", danger: false }])
  assert.deepEqual(
    constraintTags({ ...base, models: ["a", "b"], read_only: true }, now),
    [{ text: "限 2 个模型", danger: false }, { text: "只读", danger: false }],
  )
})

test("同时请求数下拉：常用值加上当前的非常用值，0 表示不限", async () => {
  const { concurrencyOptions } = await import("./sharedAccess.ts")
  assert.deepEqual(concurrencyOptions(null).map((o) => o.label), ["不限", "1 个", "2 个", "3 个", "5 个", "10 个"])
  assert.deepEqual(concurrencyOptions(7).map((o) => o.value), [0, 1, 2, 3, 5, 7, 10], "经 CLI 设的 7 也能显示")
})

test("按 Key 出账与上限表单：空串不限、非法值拒绝、标签只列设了的", async () => {
  const { spendLines, budgetForm, budgetFromForm, budgetTags } = await import("./sharedAccess.ts")
  assert.deepEqual(spendLines({ spend: [[1500, 0.004], [2_300_000, 1.2], [0, 0]] }).map((l) => `${l.label} ${l.tokens} ${l.cost}`),
    ["今日 1.5K $0.0040", "本周 2.3M $1.20", "本月 0 $0"])
  assert.deepEqual(spendLines({}), [], "旧版后端没有数据")
  const form = budgetForm({ tokens_per_week: 50000, cost_per_month: 12.5 })
  assert.equal(form.tokens_per_week, "50000")
  assert.equal(form.cost_per_day, "")
  assert.deepEqual(budgetFromForm({ ...form, cost_per_day: " 0.5 " }), { budget: { tokens_per_day: null, tokens_per_week: 50000, tokens_per_month: null, cost_per_day: 0.5, cost_per_week: null, cost_per_month: 12.5 } })
  assert.match(budgetFromForm({ ...form, tokens_per_day: "1.5" }).error, /整数/)
  assert.match(budgetFromForm({ ...form, cost_per_week: "-1" }).error, /正数/)
  assert.deepEqual(budgetTags({ budget: { tokens_per_week: 50000, cost_per_month: 12.5 }, accounts: ["c1"] }), ["每周 Token 50K", "每月费用 $12.5", "限 1 个账号"])
})

