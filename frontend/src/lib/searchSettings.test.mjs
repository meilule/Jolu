import test from "node:test"
import assert from "node:assert/strict"
import {
  draftsChanged, isMaskedKey, nextVendor, pluginState, pluginStateText, splitKeys, toDrafts, toInputs, validateDrafts,
} from "./searchSettings.ts"

const vendors = [
  { id: "tavily", name: "Tavily", key_optional: false, needs_url: false },
  { id: "you", name: "You.com", key_optional: true, needs_url: false },
  { id: "searxng", name: "SearXNG", key_optional: true, needs_url: true },
]

test("草稿与提交结构互转时掩码原样保留、多 Key 拆开去重", () => {
  const drafts = toDrafts([{ vendor: "tavily", keys: ["tvly••••5678"], url: null, off: false }])
  assert.equal(drafts[0].keysText, "tvly••••5678")
  drafts[0].keysText = "tvly••••5678\ntvly-new, tvly-new"
  assert.deepEqual(toInputs(drafts), [{ vendor: "tavily", keys: ["tvly••••5678", "tvly-new"], url: null, off: false }])
  assert.deepEqual(splitKeys(" a ; b\n\nc,a "), ["a", "b", "c"])
  assert.ok(isMaskedKey("tvly••••5678") && !isMaskedKey("tvly-raw"))
})

test("保存前校验：缺 Key、缺实例地址、地址协议不对", () => {
  assert.equal(validateDrafts([{ vendor: "tavily", keysText: "", url: "", off: false }], vendors), "Tavily 至少要填一把 Key")
  assert.equal(validateDrafts([{ vendor: "you", keysText: "", url: "", off: false }], vendors), null, "You.com 可免 Key")
  assert.equal(validateDrafts([{ vendor: "searxng", keysText: "", url: " ", off: false }], vendors), "SearXNG 需要填写实例地址")
  assert.match(validateDrafts([{ vendor: "searxng", keysText: "", url: "sx.local", off: false }], vendors), /http/)
  assert.match(validateDrafts([{ vendor: "bing", keysText: "k", url: "", off: false }], vendors), /bing/)
})

test("新增行优先选未用过的后端；改动判定只看提交内容", () => {
  assert.equal(nextVendor([{ vendor: "tavily", keysText: "", url: "", off: false }], vendors), "you")
  const saved = [{ vendor: "tavily", keys: ["tvly••••5678"], url: null, off: false }]
  assert.equal(draftsChanged(toDrafts(saved), saved), false)
  assert.equal(draftsChanged([{ vendor: "tavily", keysText: "tvly••••5678 ", url: "", off: false }], saved), false, "多余空白不算改动")
  assert.equal(draftsChanged([{ vendor: "tavily", keysText: "tvly••••5678", url: "", off: true }], saved), true)
})

test("搜索插件状态：不支持、已装、未装且配置文件不存在分别说明", () => {
  const row = { agent: "claude-code", installed: [], config_present: true, supported: true, note: null }
  assert.equal(pluginState(row), "absent")
  assert.equal(pluginStateText({ ...row, config_present: false }), "未安装 · 配置文件尚未创建，安装时会新建")
  assert.equal(pluginState({ ...row, installed: ["jolu-search"] }), "installed")
  assert.equal(pluginState({ ...row, supported: false }), "unsupported")
})
