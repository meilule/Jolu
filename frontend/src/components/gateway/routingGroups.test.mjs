import { test } from "node:test"
import assert from "node:assert/strict"

import {
  ROUTING_STRATEGIES, hasRoutingMember, offCatalogMembers, routingMemberId, routingSlugError,
  selectableCatalog, strategyText, toggleRoutingMember,
} from "./routingGroups.ts"

const catalog = [
  { id: "openai/gpt-5", source: "OpenAI", slug: "openai", model: "gpt-5", kind: "api", status: "connected" },
  { id: "glm/glm-4", source: "智谱", slug: "glm", model: "glm-4", kind: "auth", status: "connected" },
  { id: "group/team", source: "路由分组", slug: "group", model: "team", kind: "auth", status: "connected" },
  { id: "broken/x", source: "坏数据", slug: "", model: "x", kind: "api", status: "connected" },
  { id: "empty/y", source: "坏数据", slug: "empty", model: "", kind: "api", status: "connected" },
]

test("策略文案：五个策略，smart / pace 说明按额度重置时间排序，未知值原样返回", () => {
  assert.equal(ROUTING_STRATEGIES.length, 5)
  assert.equal(strategyText("order"), "顺序（失败换下一个）")
  assert.equal(strategyText("rotate"), "轮转（逐请求换）")
  assert.equal(strategyText("usage"), "用量最少优先")
  assert.equal(strategyText("smart"), "智能（重置最近的先用）")
  assert.equal(strategyText("pace"), "配速（余量最足的先用）")
  assert.equal(strategyText("mystery"), "mystery")
})

test("成员 id：`来源slug/模型`", () => {
  assert.equal(routingMemberId({ slug: "openai", model: "gpt-5" }), "openai/gpt-5")
  assert.equal(routingMemberId({ slug: "glm", model: "glm-4" }), "glm/glm-4")
})

test("可选成员：跳过分组条目（slug === \"group\"）与缺 slug / model 的条目", () => {
  assert.deepEqual(selectableCatalog(catalog).map((item) => item.id), ["openai/gpt-5", "glm/glm-4"])
  assert.deepEqual(selectableCatalog([]), [])
})

test("成员勾选：勾选即成员，重复勾选取消，不重复追加", () => {
  const a = { slug: "openai", model: "gpt-5" }
  const b = { slug: "glm", model: "glm-4" }
  assert.equal(hasRoutingMember([a], b), false)
  assert.deepEqual(toggleRoutingMember([], a), [a])
  const both = toggleRoutingMember([a], b)
  assert.deepEqual(both, [a, b])
  assert.equal(hasRoutingMember(both, b), true)
  assert.deepEqual(toggleRoutingMember(both, a), [b])
  // 同一模型再勾一次：不会出现两个相同成员
  assert.deepEqual(toggleRoutingMember([a, b], { slug: "openai", model: "gpt-5" }), [b])
})

test("目录外成员：保留显示，可被取消勾选移除", () => {
  const members = [
    { slug: "openai", model: "gpt-5" },
    { slug: "retired", model: "old-model" },
  ]
  assert.deepEqual(offCatalogMembers(members, catalog), [{ slug: "retired", model: "old-model" }])
  assert.deepEqual(offCatalogMembers([], catalog), [])
  // 目录外的成员照常参与勾选切换
  assert.deepEqual(toggleRoutingMember(members, { slug: "retired", model: "old-model" }), [{ slug: "openai", model: "gpt-5" }])
})

test("分组 id 校验：小写字母数字连字符，不以连字符开头", () => {
  assert.equal(routingSlugError(""), null) // 未填由按钮禁用负责
  assert.equal(routingSlugError("main-team"), null)
  assert.equal(routingSlugError("a1"), null)
  assert.equal(routingSlugError("Main"), "只能使用小写字母、数字和连字符。")
  assert.equal(routingSlugError("a b"), "只能使用小写字母、数字和连字符。")
  assert.equal(routingSlugError("a_b"), "只能使用小写字母、数字和连字符。")
  assert.equal(routingSlugError("-abc"), "不能以连字符开头。")
})

const { PAUSE_CHOICES, pauseMember, pausedText, isMemberPaused } = await import("./routingGroups.ts")

test("暂停成员：设到期时间、恢复清除；其他成员与已有字段不受影响", () => {
  const members = [{ slug: "a", model: "m1" }, { slug: "b", model: "m2" }]
  const paused = pauseMember(members, { slug: "a", model: "m1" }, 5_000)
  assert.deepEqual(paused, [{ slug: "a", model: "m1", paused_until_ms: 5_000 }, { slug: "b", model: "m2" }])
  assert.deepEqual(pauseMember(paused, { slug: "a", model: "m1" }, null), [{ slug: "a", model: "m1", paused_until_ms: null }, { slug: "b", model: "m2" }])
})

test("勾选切换保留其他成员的暂停字段", () => {
  const members = [{ slug: "a", model: "m1", paused_until_ms: 9_000 }]
  assert.deepEqual(toggleRoutingMember(members, { slug: "b", model: "m2" }), [{ slug: "a", model: "m1", paused_until_ms: 9_000 }, { slug: "b", model: "m2" }])
})

test("暂停判定与文案：到期视为恢复", () => {
  assert.equal(isMemberPaused({ slug: "a", model: "m", paused_until_ms: 2_000 }, 1_000), true)
  assert.equal(isMemberPaused({ slug: "a", model: "m", paused_until_ms: 1_000 }, 1_000), false)
  assert.equal(isMemberPaused({ slug: "a", model: "m" }, 1_000), false)
  assert.equal(pausedText({ slug: "a", model: "m" }, 1_000), "")
  assert.match(pausedText({ slug: "a", model: "m", paused_until_ms: 1_000 + 3_600_000 }, 1_000), /^暂停至 /)
  assert.ok(PAUSE_CHOICES.length >= 3 && PAUSE_CHOICES.every((choice) => choice.minutes > 0))
})
