import assert from "node:assert/strict"
import test from "node:test"
import * as catalog from "./providerCatalog.ts"
import { accountLabel, accountsFor } from "./usageFilters.ts"
import {
  PROVIDERS,
  canConnect,
  canStartVerify,
  canEnterManualModel,
  connectionBadge,
  defaultMode,
  editSaveState,
  fallsBackToManualModel,
  filterModels,
  modelBusyAfter,
  modelFetchErrorText,
  protocolBaseUrlHint,
  protocolLabel,
  providerEntry,
  providerName,
  requestGate,
  showsModeTabs,
  singleModeReason,
  validateConnectionName,
} from "./providerCatalog.ts"

test("统计与会话筛选按平台去重，预设和地区版本不会传入无效 ID", () => {
  const options = catalog.providerPlatformOptions()
  assert.deepEqual(options.map((option) => option.value), [...new Set(PROVIDERS.map((entry) => entry.platformId))])
  assert.equal(options.find((option) => option.value === "zcode").label, "ZCode")
  assert.equal(options.find((option) => option.value === "custom").label, "自定义供应商")
  assert.ok(!options.some((option) => option.value === "zcode-bigmodel" || option.value === "deepseek"))
})

test("供应商目录保留自定义与现有平台，地区版本独立展示", () => {
  // ZCode 拆成国内 / 国际两条（各有上游与账号家族），所以名称与 edition 清单都多一项。
  // 预设库插在 ZCode 与 Qoder 之间（P4.1），Qoder / WorkBuddy / Trae 的地区版本仍在最后
  assert.deepEqual(PROVIDERS.map((p) => p.name), [
    "自定义供应商", "Claude Code", "Codex", "Gemini", "Grok", "OpenCode",
    "ZCode 国内版", "ZCode 国际版",
    "Qoder CN", "Qoder", "WorkBuddy", "WorkBuddy AI",
    "DeepSeek", "Kimi（月之暗面）", "MiniMax", "StepFun（阶跃星辰）", "Qwen（通义千问）",
    "OpenRouter", "Together", "Fireworks", "SiliconFlow（硅基流动）", "Groq", "Ollama", "LM Studio（本机）",
    "Trae CN", "Trae",
  ])
  // 预设库（P4.1）都映射到 custom 平台：只预填地址与协议，不做厂商特化
  const presets = PROVIDERS.filter((p) => p.id.startsWith("deepseek") || ["kimi", "minimax", "stepfun", "qwen", "openrouter", "together", "fireworks", "siliconflow", "groq", "lmstudio"].includes(p.id))
  assert.ok(presets.length >= 11, `预设库应有 11 个以上条目，实际 ${presets.length}`)
  for (const p of presets) {
    assert.equal(p.platformId, "custom", `${p.id} 预设必须映射到 custom 平台`)
    assert.equal(p.subscription, "none", `${p.id} 预设没有订阅登录`)
    assert.ok(p.baseUrl?.value.startsWith("http"), `${p.id} 预设要预填地址`)
  }
  const ollama = providerEntry("ollama")
  assert.equal(ollama.platformId, "ollama", "Ollama 需要独立平台，才能保存本机订阅登录")
  assert.equal(ollama.subscription, "ready")
  assert.equal(ollama.apiKey, "ready")
  assert.deepEqual(ollama.baseUrlChoices.map((choice) => [choice.label, choice.value]), [
    ["本机 · OpenAI 兼容", "http://127.0.0.1:11434/v1"],
    ["Cloud · OpenAI 兼容", "https://ollama.com/v1"],
  ])
  assert.deepEqual(PROVIDERS.filter(p => p.edition).map(p => [p.id, p.platformId, p.edition]), [
    ["zcode-bigmodel", "zcode", "bigmodel"], ["zcode", "zcode", "zai"],
    ["qoder-cn", "qoder", "cn"], ["qoder", "qoder", "global"],
    ["workbuddy", "workbuddy", "cn"], ["workbuddy-ai", "workbuddy", "ai"],
    ["trae-cn", "trae", "cn"], ["trae", "trae", "global"],
  ])
})

test("连接供应商优先使用后端区域，本机来源可回退，未知区域不猜", () => {
  const variants = [
    ["qoder", "cn", "qoder-cn", "cn"], ["qoder", "global", "qoder", "global"],
    ["workbuddy", "cn", "workbuddy", "main"], ["workbuddy", "ai", "workbuddy-ai", "ai"],
    ["trae", "cn", "trae-cn", "cn"], ["trae", "global", "trae", "ide"],
  ]
  for (const [platformId, edition, expected, source] of variants) {
    assert.equal(catalog.connectionProviderId({ id: "browser:opaque", platformId, edition }), expected)
    assert.equal(catalog.connectionProviderId({ id: `monitor:native:${platformId}:${source}`, platformId }), expected)
  }
  assert.equal(catalog.connectionProviderId({ id: "monitor:native:qoder:cn", platformId: "qoder", edition: "global" }), "qoder")
  for (const platformId of ["qoder", "workbuddy", "trae"]) {
    assert.equal(catalog.connectionProviderId({ id: "browser:opaque", platformId }), null)
    assert.equal(catalog.connectionProviderId({ id: "monitor:native:qoder:cn", platformId, edition: "invalid" }), null)
    assert.equal(catalog.connectionProviderEntry({ platformId }).name, `${providerEntry(platformId).name} · 区域未知`)
  }
  assert.equal(catalog.connectionProviderId({ platformId: "codex" }), "codex")
  assert.equal(catalog.connectionProviderId({ id: "monitor:native:trae:cn", platformId: "qoder" }), null)
})

test("统计账号标签显示版本名称，统计供应商筛选仍按平台家族", () => {
  const accounts = [
    { id: "browser:cn", platformId: "qoder", edition: "cn", baseName: "国内" },
    { id: "browser:global", platformId: "qoder", edition: "global", baseName: "海外" },
  ]
  assert.equal(accountLabel(accounts[0]), "国内 · Qoder CN")
  assert.equal(accountLabel(accounts[1]), "海外 · Qoder")
  assert.equal(accountsFor(accounts, "qoder").length, 2)
})

test("连接方式：两种都实现才显示页签，待接入不能连接", () => {
  assert.equal(showsModeTabs(providerEntry("claude")), true)
  assert.equal(showsModeTabs(providerEntry("zcode")), true)
  assert.equal(defaultMode(providerEntry("zcode")), "subscription")
  assert.equal(singleModeReason(providerEntry("zcode")), null)
  assert.match(singleModeReason(providerEntry("trae")), /暂不支持 API Key/)
  assert.equal(singleModeReason(providerEntry("codex")), null)
  assert.equal(canConnect(providerEntry("opencode")), true)
  assert.equal(canConnect(providerEntry("trae")), true)
  assert.equal(providerEntry("gemini").baseUrl.locked, true)
})

test("数量徽标", () => {
  assert.equal(connectionBadge(0), "尚未连接")
  assert.equal(connectionBadge(2), "2 个账号")
})

test("连接名称去空白、同平台同方式不得重名", () => {
  const existing = [{ platformId: "codex", kind: "api", baseName: "工作 API" }]
  assert.equal(validateConnectionName("  ", existing, "codex", "api"), "请填写连接名称")
  assert.match(validateConnectionName(" 工作 API ", existing, "codex", "api"), /同名/)
  assert.equal(validateConnectionName("工作 API", existing, "codex", "auth"), null)
  assert.equal(validateConnectionName("工作 API", existing, "claude", "api"), null)
})

test("连接重名仅限制同一供应商版本与方式", () => {
  const existing = [{ id: "browser:cn", platformId: "qoder", edition: "cn", kind: "auth", baseName: "工作账号" }]
  assert.match(validateConnectionName("工作账号", existing, "qoder-cn", "auth"), /同名/)
  assert.equal(validateConnectionName("工作账号", existing, "qoder", "auth"), null)
})

test("模型搜索不区分大小写并保留顺序", () => {
  const models = ["coding-main", "Coding-Fast", "reasoning-pro"]
  assert.deepEqual(filterModels(models, "coding"), ["coding-main", "Coding-Fast"])
  assert.deepEqual(filterModels(models, " "), models)
})

test("自定义供应商只有 API Key 方式，可以连接且不显示方式页签", () => {
  const custom = providerEntry("custom")
  assert.equal(custom.id, "custom")
  assert.equal(canConnect(custom), true)
  assert.equal(showsModeTabs(custom), false)
  assert.equal(defaultMode(custom), "apiKey")
  assert.equal(singleModeReason(custom), null)
  assert.equal(custom.baseUrl, undefined)
})

test("自定义供应商协议：Base URL 示例与显示名", () => {
  assert.equal(protocolLabel("openai"), "OpenAI 兼容")
  assert.equal(protocolLabel("anthropic"), "Anthropic 兼容")
  assert.equal(protocolLabel(null), "未记录协议")
  assert.match(protocolBaseUrlHint("openai"), /\/v1$/)
  assert.doesNotMatch(protocolBaseUrlHint("anthropic"), /\/v1$/)
})

test("编辑自定义供应商：更换密钥后必须重新获取模型或验证手填模型才能保存（画板 27）", () => {
  const base = { platformId: "custom", kind: "api", nameChanged: false, nameValid: true, newSecret: "", model: "", modelReady: false }
  // 没有任何改动不能保存；只改名称无需重新验证
  assert.equal(editSaveState(base).canSave, false)
  assert.deepEqual(editSaveState({ ...base, nameChanged: true }), { canSave: true, needsModel: false, reason: null })
  // 更换密钥：需要模型，且模型须由新密钥获取或验证过
  const replaced = { ...base, newSecret: "sk-new" }
  assert.equal(editSaveState(replaced).needsModel, true)
  assert.equal(editSaveState(replaced).canSave, false)
  assert.match(editSaveState(replaced).reason, /重新获取模型/)
  assert.equal(editSaveState({ ...replaced, model: "m-1" }).canSave, false)
  assert.equal(editSaveState({ ...replaced, model: "m-1", modelReady: true }).canSave, true)
  assert.equal(editSaveState({ ...replaced, model: "  ", modelReady: true }).canSave, false)
  // 名称无效时一律不能保存
  assert.equal(editSaveState({ ...replaced, model: "m-1", modelReady: true, nameValid: false }).canSave, false)
  // 其他平台更换密钥仍由后端验证，不要求重新获取模型
  assert.deepEqual(editSaveState({ ...base, platformId: "codex", newSecret: "sk-new" }), { canSave: true, needsModel: false, reason: null })
})

test("自定义供应商获取模型失败：接口不可用时改为手填，密钥被拒时不改（画板 29A 状态 A / 画板 27）", () => {
  assert.equal(fallsBackToManualModel("获取模型失败：HTTP 404 Not Found，请检查 Key 与地址"), true)
  assert.equal(fallsBackToManualModel("网关返回了空模型列表"), true)
  assert.equal(fallsBackToManualModel("无法连接网关：error sending request"), true)
  assert.equal(fallsBackToManualModel("获取模型失败：HTTP 401 Unauthorized，请检查 Key 与地址"), false)
  assert.equal(fallsBackToManualModel("获取模型失败：HTTP 403 Forbidden，请检查 Key 与地址"), false)
  assert.equal(modelFetchErrorText("网关返回了空模型列表"), "获取模型失败：网关返回了空模型列表")
  assert.equal(modelFetchErrorText("获取模型失败：HTTP 404 Not Found"), "获取模型失败：HTTP 404 Not Found")
})

test("请求闸门：新请求或失效后，旧请求的结果一律丢弃（换 Key 期间的验证不能标记新 Key）", () => {
  const gate = requestGate()
  const verifyWithOldKey = gate.begin()
  assert.equal(gate.isCurrent(verifyWithOldKey), true)
  gate.invalidate() // 用户改了 Key
  assert.equal(gate.isCurrent(verifyWithOldKey), false)
  const fetchA = gate.begin()
  const fetchB = gate.begin()
  assert.equal(gate.isCurrent(fetchA), false)
  assert.equal(gate.isCurrent(fetchB), true)
})

test("「手动填写模型」入口：自定义供应商 401 / 403 时不提供，其他平台照旧", () => {
  assert.equal(canEnterManualModel("custom", "获取模型失败：HTTP 401 Unauthorized，请检查 Key 与地址"), false)
  assert.equal(canEnterManualModel("custom", "获取模型失败：HTTP 403 Forbidden，请检查 Key 与地址"), false)
  assert.equal(canEnterManualModel("custom", "网关返回了空模型列表"), true)
  assert.equal(canEnterManualModel("codex", "获取模型失败：HTTP 401 Unauthorized，请检查 Key 与地址"), true)
})

test("模型区忙碌状态：获取与验证互斥，过期请求结束不改状态，任何顺序都不会残留进行中", () => {
  // 复现审查步骤：手填状态下先获取，未返回时开始验证
  let busy = "idle"
  busy = modelBusyAfter(busy, { type: "start", kind: "fetching" })
  assert.equal(canStartVerify(busy), false) // ① 获取进行中不能验证
  busy = modelBusyAfter(busy, { type: "start", kind: "verifying" }) // ② 即使绕过也只保留一种进行中状态
  assert.equal(busy, "verifying")
  busy = modelBusyAfter(busy, { type: "end", current: false }) // 过期的获取结束
  assert.equal(busy, "verifying")
  busy = modelBusyAfter(busy, { type: "end", current: true }) // 验证结束
  assert.equal(busy, "idle")
  assert.equal(canStartVerify(busy), true)
  // 反向：验证中重新获取，过期的验证结束不影响获取
  busy = modelBusyAfter(modelBusyAfter("idle", { type: "start", kind: "verifying" }), { type: "start", kind: "fetching" })
  assert.equal(modelBusyAfter(busy, { type: "end", current: false }), "fetching")
  assert.equal(canStartVerify("verifying"), false)
})

test("Qoder 与 WorkBuddy 密钥在原工具配置",()=>{for(const id of ["qoder","workbuddy"]){const p=providerEntry(id);assert.equal(p.apiKey,"none");assert.equal(singleModeReason(p),`${p.name} 的自定义模型在 ${p.name} 客户端内配置，Jolu 不代管密钥。`)}})
test("ZCode 两个版本各有官方地址与鉴权协议",()=>{
  // 国内版与国际版是两条独立条目（各有 edition 与上游），不再混在一条里。
  const intl=providerEntry("zcode")
  const cn=providerEntry("zcode-bigmodel")
  assert.equal(intl.edition,"zai")
  assert.equal(cn.edition,"bigmodel")
  for(const p of [intl,cn]){
    assert.equal(p.baseUrlChoices.length,2)
    for(const c of p.baseUrlChoices)assert.equal(c.protocol,c.value.endsWith("anthropic")?"anthropic":"openai")
  }
  assert.ok(intl.baseUrlChoices.every(c=>c.value.includes("z.ai")))
  assert.ok(cn.baseUrlChoices.every(c=>c.value.includes("bigmodel.cn")))
})

test("平台显示名：多版本平台只给平台名，单条目平台沿用条目名，未知原样返回", () => {
  // 用量汇总行来自后端 `providers()`，它只按 requests.platform 分组：
  // zcode 一行是国内版 + 国际版的混合总量，绝不能隐式标成其中某一个版本。
  assert.equal(providerName("zcode"), "ZCode")
  for (const version of ["国际版", "国内版"]) {
    assert.ok(!providerName("zcode").includes(version), `平台总量不能带版本后缀 ${version}`)
  }
  // 同样共用 platformId 的其他平台：Qoder / WorkBuddy / Trae 各有两个地区或账号家族
  assert.equal(providerName("qoder"), "Qoder")
  assert.equal(providerName("workbuddy"), "WorkBuddy")
  assert.equal(providerName("trae"), "Trae")
  // 预设库全映射到 custom，汇总行只能是平台级名称
  assert.equal(providerName("custom"), "自定义供应商")

  // 单条目平台：条目即平台，必须仍然返回条目名（不能一刀切成裸平台名）
  assert.equal(providerName("claude"), "Claude Code")
  assert.equal(providerName("codex"), "Codex")
  assert.equal(providerName("gemini"), "Gemini")
  assert.equal(providerName("opencode"), "OpenCode")
  for (const id of ["claude", "codex", "gemini", "grok", "opencode"]) {
    assert.equal(PROVIDERS.filter((p) => p.platformId === id).length, 1, `${id} 应当只有一条条目`)
    assert.equal(providerName(id), PROVIDERS.find((p) => p.platformId === id).name)
  }

  // 传条目 id（不是 platformId）时按条目查，连接页 / Skills 的选择器依赖这一点
  assert.equal(providerName("zcode-bigmodel"), "ZCode 国内版")
  assert.equal(providerName("qoder-cn"), "Qoder CN")
  assert.equal(providerName("workbuddy-ai"), "WorkBuddy AI")
  // 都不认识就原样返回，不臆测名称
  assert.equal(providerName("unknown-platform"), "unknown-platform")
  // 本机采集 CLI 平台（P1/P2 新增）
  assert.equal(providerName("antigravity"), "Antigravity")
  assert.equal(providerName("kimi"), "Kimi Code")
  assert.equal(providerName("pi"), "Pi")
  assert.equal(providerName("dsh"), "DeepSeek Harness")
  const usageOpts = catalog.usagePlatformOptions()
  for (const [id, label] of [["antigravity", "Antigravity"], ["kimi", "Kimi Code"], ["pi", "Pi"], ["dsh", "DeepSeek Harness"]]) {
    assert.equal(usageOpts.find((o) => o.value === id)?.label, label)
  }
})

test("每个 platformId 都登记了平台级名称，不靠条目顺序猜版本", () => {
  // 这条测试保护目录演进：新增一个共用 platformId 的多版本平台却忘了登记
  // PLATFORM_NAMES 时，providerName 会退回裸 platformId——测试在这里失败，
  // 提醒补上中文平台名，而不是默默把混合总量标成某一个版本。
  const platforms = [...new Set(PROVIDERS.map((p) => p.platformId))]
  const multi = platforms.filter((id) => PROVIDERS.filter((p) => p.platformId === id).length > 1)
  assert.deepEqual(multi.sort(), ["custom", "qoder", "trae", "workbuddy", "zcode"].sort())
  for (const id of multi) {
    const name = providerName(id)
    assert.notEqual(name, id, `${id} 是多条目平台，必须有平台级名称`)
    // 平台级名称不能带版本 / 地区后缀，否则等于把混合总量标成单一版本。
    // 注意 Qoder / Trae 的国际版条目名恰好就是裸平台名，所以这里只禁版本后缀，
    // 不去断言「与所有条目名都不同」——那条会误伤合法命名。
    assert.doesNotMatch(name, /国际版|国内版/, `${id} 的平台名不能带版本后缀`)
  }
})
