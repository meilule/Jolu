import assert from "node:assert/strict"
import test from "node:test"
import { agentRow, agentRows, customAgentRow, customPresetFor, isCustomAgent, CUSTOM_AGENT_PRESETS, longContextMode, longContextText, parseTokens, readHidden, writeHidden, wireExplanation } from "./agentPage.ts"

const wiring = (patch = {}) => ({ agent: "claude-code", wired: false, config_path: "~/.claude/settings.json", notice: null, reason: null, ...patch })
const found = (state = "installed") => ({ agent: "claude-code", name: "Claude Code", installed: state === "installed", state, evidence: "bin", cli_found: state === "installed", cli_path: null })

test("工具按 已接入 / 可以接入 / 暂时不能接入 分组（画板 V5-1）", () => {
  const wired = agentRow("claude-code", wiring({ wired: true, state: "wired" }), found(), "zcode/glm-5.3")
  assert.equal(wired.group, "wired")
  assert.equal(wired.subtitle, "zcode/glm-5.3", "已接入时第二行是主模型")
  assert.equal(wired.toggle, "on")

  const drifted = agentRow("gemini", wiring({ agent: "gemini", wired: false, state: "wired", drift: "changed" }), null)
  assert.equal(drifted.group, "wired", "被外部改动仍在已接入组里，等用户处理")
  assert.equal(drifted.subtitle, "配置被改动，未经路由")
  assert.equal(drifted.tone, "danger")
  assert.equal(drifted.toggle, null)

  const available = agentRow("opencode", wiring({ agent: "opencode" }), { ...found(), agent: "opencode", name: "OpenCode" })
  assert.equal(available.group, "available")
  assert.equal(available.toggle, "off")

  const missing = agentRow("codex", wiring({ agent: "codex", reason: "尚未安装或未生成配置文件" }), { ...found("not_found"), agent: "codex", name: "Codex" })
  assert.equal(missing.group, "blocked")
  assert.equal(missing.toggle, null)

  // 桌面客户端（ZCode）：本来就没有 CLI，配置在就能接入，不显示「CLI 未找到」
  const desktop = agentRow("zcode", wiring({ agent: "zcode" }), { ...found("cli_missing"), agent: "zcode", name: "ZCode" })
  assert.equal(desktop.group, "available", "ZCode 是桌面客户端，没有 CLI 也能接入")
  assert.doesNotMatch(desktop.subtitle, /CLI 未找到/)

  // 真 CLI 工具（qwen）：CLI 不在 = 只剩配置残留，归「暂时不能接入」，避免列表堆满没装的工具
  const cliMissing = agentRow("qwen", wiring({ agent: "qwen" }), { ...found("cli_missing"), agent: "qwen", name: "Qwen Code" })
  assert.equal(cliMissing.group, "blocked")
  assert.match(cliMissing.subtitle, /CLI 未找到/)
})

test("列表搜索、隐藏与分组顺序", () => {
  const rows = [
    agentRow("opencode", wiring({ agent: "opencode" }), null),
    agentRow("claude-code", wiring({ wired: true, state: "wired" }), null),
    agentRow("codex", wiring({ agent: "codex", reason: "x" }), null),
  ]
  const all = agentRows(rows, "", [], false)
  assert.deepEqual(all.groups.map((g) => g.group), ["wired", "available", "blocked"])
  const hidden = agentRows(rows, "", ["codex"], false)
  assert.equal(hidden.hiddenCount, 1)
  assert.equal(hidden.groups.some((g) => g.rows.some((r) => r.agent === "codex")), false)
  assert.equal(agentRows(rows, "", ["codex"], true).groups.length, 3, "点「显示」后全部列出")
  assert.deepEqual(agentRows(rows, "CLAUDE", [], false).groups.map((g) => g.rows[0].agent), ["claude-code"])
})

test("长对话三态与大小解析（与后端 parse_tokens 同口径）", () => {
  assert.equal(longContextMode(""), "default")
  assert.equal(longContextMode("full"), "full")
  assert.equal(longContextMode("400k"), "custom")
  assert.equal(longContextText(""), "长对话 272K 压缩")
  assert.equal(parseTokens("272k"), 272_000)
  assert.equal(parseTokens("1.5M"), 1_500_000)
  assert.equal(parseTokens("272,000"), 272_000)
  assert.equal(parseTokens("12"), null)
  assert.equal(parseTokens("abc"), null)
})

test("接入说明每家都写清还原方式；隐藏列表存取失败不抛错", () => {
  for (const agent of ["claude-code", "codex", "gemini", "opencode", "zcode"]) {
    assert.ok(wireExplanation(agent).some((item) => item.icon === "undo"), agent)
  }
  const map = new Map()
  const storage = { getItem: (key) => map.get(key) ?? null, setItem: (key, value) => map.set(key, value) }
  writeHidden(["codex"], storage)
  assert.deepEqual(readHidden(storage), ["codex"])
  const broken = { getItem: () => { throw new Error("blocked") }, setItem: () => { throw new Error("blocked") } }
  assert.deepEqual(readHidden(broken), [])
  assert.doesNotThrow(() => writeHidden(["x"], broken))
})

test("通用接入的自定义工具按接入状态分组（P1-1）", () => {
  const base = { id: "custom-agent:crush", name: "Crush", slug: "crush", config_path: "~/.config/crush/crush.json", format: "json", base_url_key: "a", auth_key: "b", model_key: null, base_suffix: "/v1", installed: true, wired: false, drift: null, model: null }
  assert.equal(customAgentRow(base).group, "available")
  assert.equal(customAgentRow(base).subtitle, "自定义 · JSON")
  assert.equal(customAgentRow({ ...base, wired: true, model: "group/fast" }).subtitle, "group/fast")
  const drifted = customAgentRow({ ...base, drift: "changed" })
  assert.equal(drifted.group, "wired")
  assert.equal(drifted.tone, "danger")
  assert.equal(drifted.toggle, null)
  assert.equal(customAgentRow({ ...base, installed: false }).group, "blocked")
  assert.ok(isCustomAgent("custom-agent:crush"))
  assert.ok(!isCustomAgent("claude-code"))
})

test("自定义工具预设：每条键名完整，未验证的必须标出来", () => {
  const presets = CUSTOM_AGENT_PRESETS
  assert.ok(presets.length >= 6, `预设应有至少 6 家，实际 ${presets.length}`)
  for (const preset of presets) {
    assert.ok(preset.id && preset.name, "id 与名称必填")
    assert.ok(["json", "toml", "env"].includes(preset.format), `${preset.id} 格式合法`)
    assert.ok(preset.configPath.startsWith("~"), `${preset.id} 路径应以 ~ 开头`)
    assert.ok(preset.baseKey && preset.authKey, `${preset.id} 地址键与令牌键必填`)
    // 未验证的要如实标出来，不能假装准确
    assert.equal(typeof preset.verified, "boolean", `${preset.id} 必须声明 verified`)
  }
  // 只有已从纸码/文档确认的才允许 verified: true
  const verified = presets.filter((p) => p.verified).map((p) => p.id)
  assert.deepEqual(verified, ["crush"], `目前只有 Crush 的键名有代码依据，实际 ${verified}`)
})

test("customPresetFor 只认已知 id", () => {
  assert.equal(customPresetFor("crush")?.name, "Crush")
  assert.equal(customPresetFor("不存在的工具"), null)
})
