import assert from "node:assert/strict"
import test from "node:test"

const { MIDDLEWARE_KINDS, emptyFields, toConfig, fromConfig, summary, reorder } = await import("./middlewareForm.ts")

test("六个内置类型都有中文名与说明", () => {
  assert.deepEqual(MIDDLEWARE_KINDS.map((kind) => kind.id), ["model-map", "param-override", "system-prompt", "word-guard", "think-tags", "js-hook"])
  assert.ok(MIDDLEWARE_KINDS.every((kind) => kind.label && kind.hint))
})

test("model-map：每行「请求名 = 来源/模型」，空行与缺等号的行报错", () => {
  assert.deepEqual(toConfig("model-map", { ...emptyFields(), mapping: "fast = zcode/GLM-5.2\n\n smart=claude/opus " }),
    { ok: true, config: { mapping: { fast: "zcode/GLM-5.2", smart: "claude/opus" } } })
  assert.equal(toConfig("model-map", { ...emptyFields(), mapping: "fast zcode/GLM-5.2" }).ok, false)
  assert.equal(toConfig("model-map", { ...emptyFields(), mapping: "" }).ok, false)
})

test("param-override：set 每行「字段 = JSON 值」，非 JSON 按字符串；delete 逗号分隔", () => {
  assert.deepEqual(toConfig("param-override", { ...emptyFields(), set: "max_tokens = 1024\nservice_tier = flex", remove: "temperature, top_p" }),
    { ok: true, config: { set: { max_tokens: 1024, service_tier: "flex" }, delete: ["temperature", "top_p"] } })
  assert.equal(toConfig("param-override", emptyFields()).ok, false)
})

test("system-prompt 与 word-guard 去掉空白项；全空时报错", () => {
  assert.deepEqual(toConfig("system-prompt", { ...emptyFields(), prompt: "  始终用中文回答 " }), { ok: true, config: { prompt: "始终用中文回答" } })
  assert.equal(toConfig("system-prompt", { ...emptyFields(), prompt: "  " }).ok, false)
  assert.deepEqual(toConfig("word-guard", { ...emptyFields(), block: "机密项目，内部代号", mask: "" }), { ok: true, config: { block: ["机密项目", "内部代号"] } })
  assert.equal(toConfig("word-guard", emptyFields()).ok, false)
})

test("think-tags 只有 strip / wrap 两种", () => {
  assert.deepEqual(toConfig("think-tags", { ...emptyFields(), mode: "wrap" }), { ok: true, config: { mode: "wrap" } })
})

test("fromConfig 与 toConfig 互逆，可回填编辑表单", () => {
  const cases = [
    ["model-map", { mapping: { fast: "zcode/GLM-5.2" } }],
    ["param-override", { set: { max_tokens: 1024 }, delete: ["temperature"] }],
    ["system-prompt", { prompt: "中文" }],
    ["word-guard", { block: ["a"], mask: ["b"] }],
    ["think-tags", { mode: "strip" }],
  ]
  for (const [kind, config] of cases) assert.deepEqual(toConfig(kind, fromConfig(kind, config)), { ok: true, config })
})

test("summary 给列表一行概要", () => {
  assert.equal(summary("model-map", { mapping: { a: "x/y", b: "x/z" } }), "2 条改名")
  assert.equal(summary("word-guard", { block: ["a"], mask: ["b", "c"] }), "拦截 1 个词 · 遮盖 2 个词")
  assert.equal(summary("think-tags", { mode: "strip" }), "剥掉思考段")
})

test("reorder 上移下移并重排 sort，越界不动", () => {
  const rows = [{ name: "a", sort: 0 }, { name: "b", sort: 1 }, { name: "c", sort: 2 }]
  assert.deepEqual(reorder(rows, "c", -1).map((row) => [row.name, row.sort]), [["a", 0], ["c", 1], ["b", 2]])
  assert.deepEqual(reorder(rows, "a", -1).map((row) => row.name), ["a", "b", "c"])
})

test("JS 钩子：路径必填、超时是正整数，回填与概要只显示文件名", () => {
  const fields = { ...emptyFields(), path: "  D:/hooks/ctx.hooks.mjs ", timeout: "1500" }
  assert.deepEqual(toConfig("js-hook", fields), { ok: true, config: { path: "D:/hooks/ctx.hooks.mjs", timeoutMs: 1500 } })
  assert.equal(toConfig("js-hook", emptyFields()).ok, false)
  assert.equal(toConfig("js-hook", { ...fields, timeout: "1.5" }).ok, false)
  assert.equal(fromConfig("js-hook", { path: "/a/b.mjs", timeoutMs: 800 }).timeout, "800")
  assert.equal(summary("js-hook", { path: ["D:", "hooks", "ctx.hooks.mjs"].join(String.fromCharCode(92)) }), "ctx.hooks.mjs")
})
