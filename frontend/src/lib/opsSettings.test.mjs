import assert from "node:assert/strict"
import test from "node:test"

const {
  versionLabel,
  hasRunnableUpdate,
  warmupStateText,
  normalizeWarmupHour,
  otlpEndpointHint,
  OTLP_SCOPE_NOTICE,
} = await import("./opsSettings.ts")

test("未安装与版本读不出来必须分开表达", () => {
  assert.equal(versionLabel({ found: false, version: null }), "未安装")
  assert.equal(versionLabel({ found: true, version: null }), "已安装 · 版本未能读取")
  assert.equal(versionLabel({ found: true, version: "2.0.31" }), "v2.0.31")
})

test("只有能直接执行的命令才算可复制", () => {
  assert.equal(hasRunnableUpdate({ update_command: "npm install -g @anthropic-ai/claude-code" }), true)
  assert.equal(hasRunnableUpdate({ update_command: "  " }), false)
  assert.equal(hasRunnableUpdate({ update_command: null }), false)
})

test("warm-up 状态区分未开启、今天已完成与等待时间点", () => {
  assert.match(warmupStateText(false, null, "2026-11-09"), /未开启/)
  assert.match(warmupStateText(false, null, "2026-11-09"), /消耗额度/, "必须提示会消耗额度")
  assert.equal(warmupStateText(true, "2026-11-09", "2026-11-09"), "今天已完成")
  assert.equal(warmupStateText(true, "2026-11-08", "2026-11-09"), "等待今天的时间点")
  assert.equal(warmupStateText(true, null, "2026-11-09"), "等待今天的时间点")
})

test("小时越界被夹到 0–23，非法值退回默认 9", () => {
  assert.equal(normalizeWarmupHour(99), 23)
  assert.equal(normalizeWarmupHour(-5), 0)
  assert.equal(normalizeWarmupHour(12.7), 12)
  assert.equal(normalizeWarmupHour(Number.NaN), 9)
})

test("OTLP 端点提示补全 /v1/metrics 且不重复拼接", () => {
  assert.match(otlpEndpointHint(null), /不会离开本机/)
  assert.match(otlpEndpointHint("   "), /不会离开本机/)
  assert.match(otlpEndpointHint("localhost:4318"), /http/)
  assert.equal(otlpEndpointHint("http://localhost:4318"), "http://localhost:4318/v1/metrics")
  assert.equal(otlpEndpointHint("http://localhost:4318/"), "http://localhost:4318/v1/metrics")
  assert.equal(
    otlpEndpointHint("http://localhost:4318/v1/metrics"),
    "http://localhost:4318/v1/metrics",
  )
})

test("导出范围说明写明不含会话与凭据", () => {
  assert.match(OTLP_SCOPE_NOTICE, /不含会话 ID/)
  assert.match(OTLP_SCOPE_NOTICE, /Key/)
})
