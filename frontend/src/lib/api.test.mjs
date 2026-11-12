import test from "node:test"
import assert from "node:assert/strict"

test("编辑连接把必填 ID 和其他字段按 Tauri 命令签名传递", async () => {
  const calls = []
  globalThis.window = { __TAURI_INTERNALS__: { invoke: async (command, args) => { calls.push({ command, args }) } } }
  try {
    const { api } = await import("./api.ts")
    await api.updateConnection({
      id: "connection-1",
      name: "Claude",
      base_url: "https://example.test/api",
      secret: "key",
      model: "model-1",
      effort: "medium",
      context_1m: true,
    })
    assert.deepEqual(calls, [{
      command: "update_connection",
      args: {
        id: "connection-1",
        name: "Claude",
        baseUrl: "https://example.test/api",
        secret: "key",
        model: "model-1",
        effort: "medium",
        context1m: true,
      },
    }])
  } finally {
    delete globalThis.window
  }
})

test("导入预览、确认导入与按日期导出按 Tauri 命令签名传参", async () => {
  const calls = []
  globalThis.window = { __TAURI_INTERNALS__: { invoke: async (command, args) => { calls.push({ command, args }) } } }
  try {
    const { api } = await import("./api.ts")
    await api.previewImportFile()
    await api.confirmImport("token-1")
    await api.discardImport("token-2")
    await api.exportDataFile("claude", 1000, 2000)
    await api.exportDataFile(null, null, null)
    assert.deepEqual(calls.map((c) => [c.command, c.args ?? null]), [
      ["preview_import_file", {}],
      ["confirm_import", { token: "token-1" }],
      ["discard_import", { token: "token-2" }],
      ["export_data_file", { platform: "claude", startMs: 1000, endMs: 2000 }],
      ["export_data_file", { platform: null, startMs: null, endMs: null }],
    ])
  } finally {
    delete globalThis.window
  }
})

test("限流头观测按 ok 渲染并标 derived，官方结果原样返回（4.3）", async () => {
  const { normalizeQuota } = await import("./api.ts")
  const windows = [{ key: "rl-requests", window_name: "请求数 · 上次请求观测 10:32", used_percent: 60, amount_text: "剩余 4 / 10", resets_at: null }]
  assert.deepEqual(normalizeQuota({ state: "observed", windows, observed_at_ms: 5, reason: "API Key 不提供订阅额度" }),
    { state: "ok", windows, plan: null, confidence: "derived", observed_at_ms: 5, reason: "API Key 不提供订阅额度" })
  const official = { state: "ok", windows, plan: "Pro" }
  assert.equal(normalizeQuota(official), official)
  const failed = { state: "failed", reason: "x" }
  assert.equal(normalizeQuota(failed), failed)
})
