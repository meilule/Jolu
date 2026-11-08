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

test("策略文案：五个策略，smart / pace 标注待额度快照，未知值原样返回", () => {
  assert.equal(ROUTING_STRATEGIES.length, 5)
  assert.equal(strategyText("order"), "顺序（失败换下一个）")
  assert.equal(strategyText("rotate"), "轮转（逐请求换）")
  assert.equal(strategyText("usage"), "用量最少优先")
  assert.equal(strategyText("smart"), "智能（待额度快照，暂按顺序）")
  assert.equal(strategyText("pace"), "配速（待额度快照，暂按顺序）")
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
