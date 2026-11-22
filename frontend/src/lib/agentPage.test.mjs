import assert from "node:assert/strict"
import test from "node:test"
import { agentRow as makeAgentRow, buildAgentRows, agentRows, agentGridRows, customAgentRow, customPresetFor, isCustomAgent, CUSTOM_AGENT_PRESETS, longContextMode, longContextText, parseTokens, readHidden, sessionRecordCapability, sessionRecordNote, writeHidden, wireExplanation } from "./agentPage.ts"

const agentRow = (agent, wiring, discovery, model = null, supported = true) => makeAgentRow(agent, wiring, discovery, model, supported)

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

  // 对齐 magpie：配置在、CLI 不在 PATH 仍「可以接入」（接线只写配置文件）；
  // 真卸载残留已由后端 agent_dir_has_content 过滤
  const cliMissing = agentRow("opencode", wiring({ agent: "opencode" }), { ...found("cli_missing"), agent: "opencode", name: "OpenCode" })
  assert.equal(cliMissing.group, "available")
  assert.equal(cliMissing.toggle, "off", "有专用接线的给开关")
  assert.match(cliMissing.subtitle, /CLI 未在 PATH/)
  // 无专用接线的（cline）：可接入但「暂不支持」，不给会失败的开关
  const noWiring = agentRow("cline", wiring({ agent: "cline" }), { ...found("cli_missing"), agent: "cline", name: "Cline" }, null, false)
  assert.equal(noWiring.group, "available")
  assert.equal(noWiring.supported, false)
  assert.equal(noWiring.toggle, null)
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
  // 预设随着「原生接线」增加而减少（qwen 2026-11-21 接了之后从 3 降到 2）。
  // 断言只保「别被删光」；具体有哪几家由下面的 verified 检查与 customPresetFor 覆盖。
  assert.ok(presets.length >= 2, `预设至少保留 2 家（goose / continue），实际 ${presets.length}`)
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
  assert.deepEqual(verified, [], `剩下的预设都是推测的键名，实际 ${verified}`)
  // 有专用接线的工具不再出现在预设里
  for (const id of ["kimi", "crush", "pencil", "qoder", "qoder-cn", "qwen", "workbuddy"]) assert.equal(customPresetFor(id), null, id)
})

test("customPresetFor 只认已知 id", () => {
  assert.equal(customPresetFor("continue")?.name, "Continue")
  assert.equal(customPresetFor("不存在的工具"), null)
})


test("V8 栅格筛选：blocked 恒定折叠、顺序先已接入再可接入、隐藏单独计数", () => {
  const wired = agentRow("claude-code", { wired: true, drift: null, state: "ok" }, null, "opus")
  const available = agentRow("codex", null, { agent: "codex", name: "Codex", state: "installed" }, null)
  const blocked = agentRow("qwen", null, { agent: "qwen", name: "Qwen Code", state: "not_found" }, null)
  const rows = [available, blocked, wired]
  // 全部：栅格里只有 wired + available（blocked 走折叠区），wired 在前
  const all = agentGridRows(rows, "all", "", [], false)
  assert.deepEqual(all.rows.map((r) => r.agent), ["claude-code", "codex"])
  assert.deepEqual(all.blocked.map((r) => r.agent), ["qwen"])
  // 已接入筛选
  assert.deepEqual(agentGridRows(rows, "wired", "", [], false).rows.map((r) => r.agent), ["claude-code"])
  // 可接入筛选 + 搜索
  assert.deepEqual(agentGridRows(rows, "available", "code", [], false).rows.map((r) => r.agent), ["codex"])
  // 隐藏：默认不显示，计数返回
  const hiddenOne = agentGridRows(rows, "all", "", ["codex"], false)
  assert.deepEqual(hiddenOne.rows.map((r) => r.agent), ["claude-code"])
  assert.equal(hiddenOne.hiddenCount, 1)
  assert.deepEqual(agentGridRows(rows, "all", "", ["codex"], true).rows.map((r) => r.agent), ["claude-code", "codex"])
})

test("V8 暂不支持：非可接线且已检测的工具在栅格里、开关禁用", () => {
  // workbuddy 2026-11-21 起有原生接线，改用仍未接的 cline 做「暂不支持」示例
  const workbuddy = agentRow("cline", null, { agent: "cline", name: "Cline", state: "installed" }, null, false)
  assert.equal(workbuddy.group, "available")
  assert.equal(workbuddy.supported, false)
  assert.equal(workbuddy.toggle, null)
  assert.equal(workbuddy.subtitle, "已检测到 · 暂无专用接入")
  // 可接线的仍是开关可用
  const zcode = agentRow("zcode", null, { agent: "zcode", name: "ZCode", state: "installed" }, null)
  assert.equal(zcode.supported, true)
  assert.equal(zcode.toggle, "off")
  // 栅格包含暂不支持卡（不走折叠区）
  const grid = agentGridRows([workbuddy, zcode], "all", "", [], false)
  // 同组保持输入顺序（都是 available）
  assert.deepEqual(grid.rows.map((r) => r.agent), ["cline", "zcode"])
  assert.deepEqual(grid.blocked, [])
})

test("工具卡片使用后端状态清单判断接入能力", () => {
  const rows = buildAgentRows(
    [wiring({ agent: "new-agent" })],
    [{ ...found(), agent: "new-agent", name: "New Agent" }, { ...found(), agent: "other", name: "Other" }],
    null,
  )
  assert.deepEqual(rows.map((row) => row.agent), ["new-agent", "other"])
  assert.equal(rows[0].supported, true)
  assert.equal(rows[0].toggle, "off")
  assert.equal(rows[1].supported, false)
  assert.equal(rows[1].toggle, null)
})

// 前端不再有硬编码清单（`buildAgentRows` 直接按后端 `status.wiring` 建行），
// 所以「卡片与注册表脱节」的表现变成了另一头：**后端 status 漏了某家**——
// 注册表里能接、`gateway_status` 却不返回它，卡片直接不出现，用户永远看不到。
// 这里直接读后端源码比对：注册表的 id 集合必须与 status 的构造来源一致。
test("后端 status.wiring 必须覆盖接线注册表的全部 id", async () => {
  const { readFileSync } = await import("node:fs")
  const { fileURLToPath } = await import("node:url")
  const { dirname, resolve } = await import("node:path")
  const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../../..")

  // 注册表的 11 条 AgentSpec → id
  const specs = readFileSync(resolve(repo, "backend/src/gateway/wiring_specs.rs"), "utf8")
  const registered = [...specs.matchAll(/AgentSpec\s*\{([\s\S]*?)\n\s*\},/g)]
    .map((block) => block[1].match(/id:\s*"([^"]+)"/)?.[1])
    .filter((id) => typeof id === "string")
  const declared = (specs.match(/(?<!struct )(?<!impl )AgentSpec\s*\{/g) ?? []).length
  assert.equal(registered.length, declared, `只解析出 ${registered.length} 个 id，源码里有 ${declared} 条 AgentSpec——正则漏了排版`)
  assert.ok(registered.length >= 10, `注册表解析异常（${registered.length} 个）`)

  // status 的接线列表必须由 register 派生，不能另有一份手写清单：
  // `service/gateway/status.rs` 里应调用 `wiring::agents()`。
  const status = readFileSync(resolve(repo, "backend/src/service/gateway/status.rs"), "utf8")
  assert.match(status, /wiring::agents\(\)/, "gateway_status 的接线列表必须从注册表派生，不能手写")
  assert.doesNotMatch(
    status,
    /wiring:\s*vec!\[/,
    "gateway_status 里出现了手写的接线清单——加一家会漏掉，卡片直接不出现",
  )

  // 前端不能重新长出一份硬编码清单
  const page = readFileSync(resolve(repo, "frontend/src/lib/agentPage.ts"), "utf8")
  assert.doesNotMatch(
    page,
    /WIRABLE_AGENTS\s*=/,
    "前端又出现了 WIRABLE_AGENTS 硬编码清单：改由后端 status.wiring 派生，加一家不用改前端",
  )
})

// 活跃会话只列真的读到会话的平台。接了一家却永远看不到它，用户会以为坏了。
// 这三家要明确说明，其余默认能观测。
test("会话记录能力：不支持的要明说，能观测的不占位置", () => {
  assert.equal(sessionRecordCapability("pencil"), "no-record", "Pencil 只有模型配置，没有会话存储")
  assert.equal(sessionRecordCapability("qwen"), "not-yet")
  assert.equal(sessionRecordCapability("workbuddy"), "not-yet")
  // 能读到会话的 8 家都不该有说明（不占位置）
  for (const agent of ["claude-code", "codex", "zcode", "gemini", "opencode", "kimi", "pi", "dsh", "qoder"]) {
    assert.equal(sessionRecordCapability(agent), "observable", agent)
    assert.equal(sessionRecordNote(agent), null, `${agent} 不该有说明`)
  }
  // 说明文案要能区分「本来就没有」与「还没接」
  assert.match(sessionRecordNote("pencil"), /不产生会话记录/)
  assert.match(sessionRecordNote("qwen"), /还没接入/)
})
