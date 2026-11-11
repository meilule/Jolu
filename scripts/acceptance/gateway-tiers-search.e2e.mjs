// 档位面板与联网搜索设置的预览验收：真实组件 + 合成 API；不连接桌面、不读写任何工具配置。
// 用法：先启动前端（pnpm dev），在仓库根目录 JOLU_PREVIEW_URL=http://localhost:5173 node scripts/acceptance/gateway-tiers-search.e2e.mjs
// 依赖：output/acceptance/node 里的 playwright-core 与本机 Edge。截图在 output/acceptance/gateway-tiers-search/。
import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { mkdir } from "node:fs/promises"
import { resolve } from "node:path"
const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [resolve("output/acceptance/node")] }))
const base = process.env.JOLU_PREVIEW_URL || "http://127.0.0.1:5197"
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(new URL(base).hostname))
const output = resolve("output/acceptance/gateway-tiers-search")
await mkdir(output, { recursive: true })

const settings = {
  island_platform: "claude", dnd: false, island_visible: true, proxy_enabled: true, theme: "light",
  search_apis: [{ vendor: "tavily", keys: ["tvly••••5678"], url: null, off: false }],
}
const shell = (fixture) => `<!doctype html><html><head><meta charset="UTF-8"></head><body class="bg-jolu-bg"><div id="root"></div><script type="module">
import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true;
await import('/${fixture}');</script></body></html>`

const browser = await chromium.launch({ channel: "msedge", headless: true })
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 1500 } })
  const errors = []
  page.on("pageerror", (error) => errors.push(error.message))
  await page.addInitScript((initial) => {
    window.calls = []
    const callbacks = new Map()
    const listeners = new Map()
    let next = 1
    let current = initial
    const answers = {
      get_settings: () => current,
      search_vendors: () => [
        { id: "tavily", name: "Tavily", key_optional: false, needs_url: false },
        { id: "you", name: "You.com", key_optional: true, needs_url: false },
        { id: "searxng", name: "SearXNG", key_optional: true, needs_url: true },
      ],
      search_apis_set: (args) => { current = { ...current, search_apis: args.apis.map((api) => ({ ...api, keys: api.keys.map((k) => (k.includes("•") ? k : `${k.slice(0, 4)}••••${k.slice(-4)}`)) })) }; return current },
      search_try: () => [{ title: "Rust 1.90 发布说明", url: "https://blog.rust-lang.org/", snippet: "合成结果" }],
      library_list: () => [
        { agent: "claude-code", installed: ["jolu-search"], config_present: true, supported: true, note: null },
        { agent: "workbuddy", installed: [], config_present: false, supported: true, note: "WorkBuddy 只连接批准过的 server：装好后在 WorkBuddy 的 MCP 设置里打开一次" },
        { agent: "trae", installed: [], config_present: false, supported: false, note: "Trae 的 MCP 配置位置与格式没有可靠来源，暂不支持" },
      ],
    }
    window.__TAURI_INTERNALS__ = {
      transformCallback(cb) { const id = next++; callbacks.set(id, cb); return id },
      unregisterCallback(id) { callbacks.delete(id) },
      invoke(cmd, args) {
        if (cmd === "plugin:event|listen") {
          listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler])
          return Promise.resolve(args.handler)
        }
        if (cmd === "plugin:event|unlisten") return Promise.resolve()
        window.calls.push([cmd, args])
        if (cmd in answers) {
          const answer = answers[cmd](args)
          // 与真实命令一致：保存后广播 settings-changed
          if (cmd === "search_apis_set") queueMicrotask(() => (listeners.get("settings-changed") ?? []).forEach((id) => callbacks.get(id)?.({ event: "settings-changed", id, payload: answer })))
          return Promise.resolve(answer)
        }
        return Promise.reject(new Error(`合成环境未提供：${cmd}`))
      },
    }
  }, settings)

  // —— 档位面板 ——
  await page.route("**/tiers-check", (r) => r.fulfill({ contentType: "text/html", body: shell("tiers-fixture") }))
  await page.route("**/tiers-fixture", (r) => r.fulfill({ contentType: "application/javascript", body: `
import React from '/node_modules/.vite/deps/react.js'; import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import { WiringDialog } from '/src/components/gateway/GatewayDialogs.tsx';
import { emptyTierDraft, tierModelsFrom } from '/src/lib/tierModels.ts';
import '/src/index.css';
const models=['claude-me/claude-opus-4-5','claude-me/claude-haiku-4-5','zcode/glm-5.3-flash'].map(id=>({id}));
function App(){
  const [draft,setDraft]=React.useState(emptyTierDraft());
  window.applied=null;
  return React.createElement(WiringDialog,{agents:[{agent:'claude-code',wired:true,config_path:'C:/synthetic',reason:null,notice:null}],models,modelChoice:'zcode/glm-5.3-flash',onModelChoice(){},subagentChoice:'',onSubagentChoice(){},
    tierDraft:draft,onTierDraft:setDraft,onApplyTiers(){window.applied=tierModelsFrom(draft,'zcode/glm-5.3-flash','')},
    profiles:[],onProfileApply(){},onProfileSave(){},onProfileDelete(){},busy:false,onClose(){},onWire(){},onUnwire(){},onAccept(){}});
}
ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(App));
` }))
  await page.goto(`${base}/tiers-check`)
  await page.getByText("档位（待确认）").click()
  await page.getByLabel("Opus 档模型").selectOption("claude-me/claude-opus-4-5")
  const opusEfforts = await page.getByRole("radiogroup", { name: "Opus 档 effort" }).getByRole("radio").allTextContents()
  assert.deepEqual(opusEfforts, ["默认", "低", "中", "高"], "Opus 4.5 只给到 high")
  await page.getByRole("radiogroup", { name: "Opus 档 effort" }).getByRole("radio", { name: "高" }).click()
  const checked = await page.getByRole("radiogroup", { name: "Opus 档 effort" }).locator('[aria-checked="true"]').allTextContents()
  assert.deepEqual(checked, ["高"], "只有一个 effort 被选中")
  await page.getByLabel("Haiku 档模型").selectOption("claude-me/claude-haiku-4-5")
  assert.ok(await page.getByText("这个模型不收 effort").isVisible(), "Haiku 不给 effort 选项")
  await page.getByLabel("Auto 判别模型").selectOption("subagent")
  await page.waitForTimeout(400) // 等按钮的颜色过渡结束再截图
  await page.screenshot({ path: resolve(output, "tiers.png"), fullPage: true })
  await page.getByRole("button", { name: "应用档位" }).click()
  const applied = await page.evaluate(() => window.applied)
  assert.deepEqual(applied, { opus: "claude-me/claude-opus-4-5:high", haiku: "claude-me/claude-haiku-4-5", autoMode: "zcode/glm-5.3-flash" }, "跟随子代理且子代理跟随主模型时填主模型")

  // —— 联网搜索设置 ——
  await page.route("**/search-check", (r) => r.fulfill({ contentType: "text/html", body: shell("search-fixture") }))
  await page.route("**/search-fixture", (r) => r.fulfill({ contentType: "application/javascript", body: `
import React from '/node_modules/.vite/deps/react.js'; import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import { SearchSection } from '/src/components/settings/SearchSection.tsx';
import { ToastProvider } from '/src/components/ui/Toast.tsx'; import '/src/index.css';
ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(ToastProvider,null,React.createElement('div',{className:'mx-auto flex max-w-[860px] flex-col gap-4 p-6'},React.createElement(SearchSection))));
` }))
  await page.goto(`${base}/search-check`)
  const keys = page.getByLabel("Tavily 的 Key")
  await keys.waitFor()
  assert.equal(await keys.inputValue(), "tvly••••5678", "只显示掩码")
  await keys.fill("tvly••••5678\ntvly-newkey-00001111")
  await page.getByRole("button", { name: "保存" }).click()
  await page.waitForFunction(() => window.calls.some(([cmd]) => cmd === "search_apis_set"))
  const saved = await page.evaluate(() => window.calls.find(([cmd]) => cmd === "search_apis_set")[1].apis)
  assert.deepEqual(saved[0].keys, ["tvly••••5678", "tvly-newkey-00001111"], "掩码原样交回、新 Key 原文只交一次")
  await page.getByLabel("试搜关键词").fill("rust")
  await page.getByRole("button", { name: "试搜" }).click()
  await page.getByText("Rust 1.90 发布说明").waitFor()
  assert.ok(await page.getByText("不支持", { exact: false }).first().isVisible(), "Trae 标为不支持")
  await page.screenshot({ path: resolve(output, "search.png"), fullPage: true })

  assert.deepEqual(errors, [], `页面错误：${errors.join("；")}`)
  console.log("档位面板与联网搜索设置预览验收通过")
} finally {
  await browser.close()
}
