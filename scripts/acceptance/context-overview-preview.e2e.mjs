// 模型路由 · 概览「运行概览与活跃会话」（Jolu.pen V10-01 浅色 / V10-02 深色）预览验收：
// 真实 GatewayView + 合成 API（不连接桌面、账号或上游）。核对活跃会话表各列与 V10-13 的状态口径，截浅色 / 深色两张图对照画板。
// 用法：pnpm --dir frontend exec vite --port 5198 --strictPort；在仓库根目录 JOLU_PREVIEW_URL=http://localhost:5198 node scripts/acceptance/context-overview-preview.e2e.mjs
// 依赖：output/acceptance/node 里的 playwright-core 与本机 Chrome（没有时用 Edge）。截图在 output/acceptance/context-overview/。
import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { mkdir } from "node:fs/promises"
import { resolve } from "node:path"
const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [resolve("output/acceptance/node")] }))
const base = process.env.JOLU_PREVIEW_URL || "http://localhost:5198"
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(new URL(base).hostname))
const output = resolve("output/acceptance/context-overview")
await mkdir(output, { recursive: true })

const connections = [
  ["c1", "工作账号", "claude", "auth", "claude-work", true],
  ["c2", "个人账号", "workbuddy", "auth", "workbuddy", true],
  ["c3", "主账号", "codex", "auth", "codex", true],
].map(([id, baseName, platformId, kind, slug, serve]) => ({
  id, baseName, name: baseName, platformId, kind, slug, serve_enabled: serve, label: "", masked: "", status: "connected",
  lastSyncText: "", baseUrl: null, model: null, effort: null, context1m: null, protocol: null,
}))

const fixture = `
import React from '/node_modules/.vite/deps/react.js'; import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import { GatewayView } from '/src/views/GatewayView.tsx';
import { gatewayApi } from '/src/lib/gatewayApi.ts';
import { ToastProvider } from '/src/components/ui/Toast.tsx'; import '/src/index.css';
const now=Date.now();
const link=(platform,id,agent,source,model)=>({session_platform:platform,session_id:id,connection_id:null,source,requested_model:source+'/'+model,upstream_model:model,agent,last_ts:now-8000,requests:3});
const session=(platform,id,agent,title,cwd,links)=>({platform,session_id:id,agent,title,cwd,thread_last_ts:now-60000,gateway_last_ts:now-8000,thread_status:null,tokens:{},links});
gatewayApi.sessionsOverview=async()=>({seq:1,at:now,in_flight:[
  {request_id:41,ts:now-2000,agent:'claude-code',requested_model:'claude-work/claude-sonnet-4-5',source:'claude-work',connection_id:'c1',session_platform:'claude',session_id:'ses-claude'},
],sessions:[
  session('claude','ses-claude','claude-code','接线修复','D:\\\\work\\\\Jolu',[link('claude','ses-claude','claude-code','claude-work','claude-sonnet-4-5')]),
  session('codex','ses-codex','codex','文档与验收','D:\\\\work\\\\Jolu',[link('codex','ses-codex','codex','codex','gpt-5.2')]),
  session('opencode','opencode:ses-oc','opencode','界面优化','D:\\\\work\\\\Jolu',[link('opencode','opencode:ses-oc','opencode','workbuddy','deepseek-v4.1-flash')]),
  session('claude','ses-new','claude-code','路由实验',null,[]),
]});
const snap=(platform,id,agent,over)=>({id:Math.floor(Math.random()*1e6),session_platform:platform,session_id:id,request_id:1,ts:now-8000,state:'ready',protocol:'anthropic',
  input_tokens:null,output_tokens:12,cache_read:null,window_tokens:null,estimated:null,counted:true,error:null,agent,requested_model:null,upstream_model:null,source:null,
  route_result:'ok',status_code:200,duration_ms:1200,connection_id:null,connection_name:null,connection_platform:null,last_ready_ts:now-8000,snapshots:4,...over});
gatewayApi.contextSessions=async()=>[
  snap('claude','ses-claude','claude-code',{input_tokens:168000,cache_read:162000,window_tokens:200000,upstream_model:'claude-sonnet-4-5',connection_name:'工作账号',connection_platform:'claude',ts:now-2000}),
  snap('codex','ses-codex','codex',{input_tokens:74200,cache_read:60300,window_tokens:400000,upstream_model:'gpt-5.2',connection_name:'主账号',connection_platform:'codex',ts:now-16000}),
  snap('opencode','opencode:ses-oc','opencode',{input_tokens:631000,cache_read:null,window_tokens:null,upstream_model:'deepseek-v4.1-flash',connection_name:'个人账号',connection_platform:'workbuddy',state:'unsupported',ts:now-30000}),
];
gatewayApi.status=async()=>({mode:'relay',enabled:true,port:24711,circuit_threshold:5,auto_escape:true,default_connection:'c1',wiring:[]});
gatewayApi.catalog=async()=>[];
gatewayApi.groupList=async()=>[];
gatewayApi.dayOverview=async()=>({today:248,yesterday:216,reroutes:4,errors:2,by_agent:{},partial:false,last_error:null});
gatewayApi.agentDiscovery=async()=>[];
gatewayApi.groups=async()=>({sources:[],agents:[]});
gatewayApi.sourceCapabilities=async()=>[];
const rows=[];for(let h=0;h<12;h++){for(let i=0;i<3+h;i++){rows.push({id:rows.length+1,ts:now-(11-h)*3600000-i*60000,agent:'claude-code',requested_model:'m',source:'claude-work',upstream_model:null,status_code:200,result:'ok',duration_ms:800+i*20,request_id:null})}}
gatewayApi.routeLogPage=async()=>({rows,page:1,page_size:240,total:rows.length,page_count:1,upper_id:null});
gatewayApi.routeLog=async()=>[];
gatewayApi.middlewares=async()=>[];
gatewayApi.clientKeys=async()=>[];
gatewayApi.lanAddress=async()=>null;
if(new URLSearchParams(location.search).get('theme')==='dark') document.documentElement.classList.add('dark');
ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(ToastProvider,null,React.createElement('div',{className:'h-screen bg-jolu-bg'},React.createElement(GatewayView,{onNavigate(){}}))));
`

// 本机 Edge 无头启动会直接退出（2026-11-17），优先用 Chrome，Chrome 不可用时退回 Edge
const browser = await chromium.launch({ channel: "chrome", headless: true }).catch(() => chromium.launch({ channel: "msedge", headless: true }))
const errors = []
const page = await browser.newPage({ viewport: { width: 1072, height: 1000 } })
try {
  const noise = (text) => /WebSocket|\[vite\]|Failed to load resource|合成环境禁止原生调用/.test(text)
  page.on("pageerror", (error) => errors.push(error.message))
  page.on("console", (msg) => { if (msg.type() === "error" && !noise(msg.text())) errors.push(msg.text()) })
  await page.addInitScript(() => {
    window.unexpectedNativeCalls = []
    const callbacks = new Map()
    let next = 1
    window.__TAURI_INTERNALS__ = {
      transformCallback(cb) { const id = next++; callbacks.set(id, cb); return id },
      unregisterCallback(id) { callbacks.delete(id) },
      invoke(cmd, args) {
        if (cmd === "plugin:event|listen") return Promise.resolve(args.handler)
        if (cmd === "plugin:event|unlisten") return Promise.resolve()
        if (cmd === "get_settings") return Promise.resolve({ gateway_shared_lan: false })
        window.unexpectedNativeCalls.push(cmd)
        return Promise.reject(new Error(`合成环境禁止原生调用：${cmd}`))
      },
    }
  })
  await page.route("**/src/hooks/useConnections.ts*", (r) => r.fulfill({
    contentType: "application/javascript",
    body: `const connections=${JSON.stringify(connections)}; const reload=async()=>{}; export function useConnections(){return {connections,reload,loading:false,error:null}}`,
  }))
  await page.route("**/ctx-check*", (r) => r.fulfill({ contentType: "text/html", body: `<!doctype html><html><head><meta charset="UTF-8"></head><body class="bg-jolu-bg"><div id="root"></div><script type="module">
import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true;
await import('/ctx-fixture');</script></body></html>` }))
  await page.route("**/ctx-fixture", (r) => r.fulfill({ contentType: "application/javascript", body: fixture }))

  for (const theme of ["light", "dark"]) {
    await page.goto(`${base}/ctx-check?theme=${theme}`)
    await page.getByText("活跃会话", { exact: true }).waitFor()
    const table = page.locator("#active-sessions")
    // 表格与窄容器卡片两种形态同时存在（CSS 隐藏其一），取第一个可见的
    await table.getByText("168K / 200K").first().waitFor()
    for (const text of [
      "运行概览", "最近 12 小时", "需要关注", "Claude Code 上下文已用 84%", "最近一次请求 · 168K / 200K",
      "1 个处理中", "接线修复", "84.0%", "162K · 96.4%", "正在生成",
      "74.2K / 400K", "18.6%", "60.3K · 81.3%",
      "631K / 容量未知", "未提供", "空闲",
      "路由实验", "暂无请求",
    ]) {
      // 同一文字可能也出现在隐藏的下拉选项里：任一处可见即可
      const matches = await page.getByText(text, { exact: true }).all()
      const visible = await Promise.all(matches.map((m) => m.isVisible()))
      assert.ok(visible.some(Boolean), `${theme}：缺少「${text}」`)
    }
    // 容量未知那一行不显示百分比与进度条
    const unknownRow = table.locator("div.grid", { hasText: "631K / 容量未知" })
    assert.equal(await unknownRow.locator("span.rounded-full.bg-jolu-track").count(), 0, "容量未知不应有进度条")
    // 接近上限那一行的进度条用危险色
    const riskRow = table.locator("div.grid", { hasText: "168K / 200K" })
    assert.equal(await riskRow.locator("span.bg-jolu-danger").count(), 1, "84% 应标危险色")
    await page.screenshot({ path: resolve(output, `v10-0${theme === "light" ? 1 : 2}-overview-${theme}.png`), fullPage: true })
  }

  // 搜索与状态筛选
  await page.getByLabel("搜索会话或项目").fill("文档")
  assert.equal(await page.locator("#active-sessions").getByText("接线修复").count(), 0)
  assert.ok(await page.locator("#active-sessions").getByText("文档与验收").first().isVisible())
  await page.getByLabel("搜索会话或项目").fill("")
  await page.getByLabel("按状态筛选").click()
  await page.getByRole("menuitemradio", { name: "接近上限", exact: true }).click()
  assert.ok(await page.locator("#active-sessions").getByText("接线修复").first().isVisible())
  assert.equal(await page.locator("#active-sessions").getByText("文档与验收").count(), 0)
  await page.waitForTimeout(300)
  await page.getByLabel("按状态筛选").click()
  await page.getByRole("menuitemradio", { name: "尚未采集", exact: true }).click()
  assert.ok(await page.locator("#active-sessions").getByText("路由实验").first().isVisible())

  const unexpected = await page.evaluate(() => window.unexpectedNativeCalls)
  assert.deepEqual(unexpected, [], `合成环境出现未处理的原生调用：${unexpected.join(", ")}`)
  assert.deepEqual(errors, [], `控制台错误：\n${errors.join("\n")}`)
  console.log(`通过：V10-01 / V10-02 概览与活跃会话。截图在 ${output}`)
} finally {
  await browser.close()
}
