// Agent 页与模型路由页（Jolu.pen「统一网关 · R2 重设计」V5-1 / V5-2 / V5-3 / V5-4）预览验收：
// 真实 AgentsView / GatewayView + 合成 API；不连接真实桌面、账号或上游，不读写任何工具配置。
// 用法：pnpm --dir frontend exec vite --port 5197 --strictPort；在仓库根目录 node scripts/acceptance/agents-routing-r2.e2e.mjs
// 依赖：output/acceptance/node 里的 playwright-core 与本机 Edge。截图在 output/acceptance/agents-routing-r2/。
import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { mkdir } from "node:fs/promises"
import { resolve } from "node:path"
const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [resolve("output/acceptance/node")] }))
const base = process.env.JOLU_PREVIEW_URL || "http://127.0.0.1:5197"
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(new URL(base).hostname))
const output = resolve("output/acceptance/agents-routing-r2")
await mkdir(output, { recursive: true })

const connections = [
  ["c1", "工作账号", "claude", "auth", "claude-work", true],
  ["c2", "ZCode 个人套餐", "zcode", "auth", "zcode", true],
  ["c3", "Kimi Key", "custom", "api", "kimi", true],
  ["c4", "DeepSeek Key", "custom", "api", "deepseek", false],
].map(([id, baseName, platformId, kind, slug, serve]) => ({
  id, baseName, name: baseName, platformId, kind, slug, serve_enabled: serve, label: "", masked: "", status: "connected",
  lastSyncText: "", baseUrl: null, model: null, effort: null, context1m: null, protocol: null,
}))

const fixture = `
import React from '/node_modules/.vite/deps/react.js'; import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import { AgentsView } from '/src/views/AgentsView.tsx';
import { GatewayView } from '/src/views/GatewayView.tsx';
import { gatewayApi } from '/src/lib/gatewayApi.ts';
import { settingsApi } from '/src/lib/settingsApi.ts';
import { skillsApi } from '/src/lib/skillsApi.ts';
import { automationApi } from '/src/lib/automationApi.ts';
import { api } from '/src/lib/api.ts';
import { ToastProvider } from '/src/components/ui/Toast.tsx'; import '/src/index.css';
const now=Date.now();
const wiring=[
  {agent:'claude-code',wired:true,state:'wired',config_path:'~/.claude/settings.json',notice:'新会话生效；已在运行的会话需重开',reason:null},
  {agent:'codex',wired:true,state:'wired',config_path:'~/.codex/config.toml',notice:'需重启 Codex 后生效',reason:null,codex_mode:'oauth'},
  {agent:'gemini',wired:false,state:'wired',drift:'changed',config_path:'~/.gemini/.env',notice:null,reason:null},
  {agent:'opencode',wired:false,config_path:'~/.config/opencode/opencode.json',notice:null,reason:null},
  {agent:'zcode',wired:false,config_path:null,notice:null,reason:'尚未安装或未生成配置文件'},
];
gatewayApi.status=async()=>({mode:'relay',enabled:true,port:15721,circuit_threshold:5,auto_escape:true,default_connection:'c2',wiring});
gatewayApi.catalog=async()=>['claude-work/claude-opus-5-5','claude-work/claude-sonnet-5-5','claude-work/claude-haiku-4-5','zcode/glm-5.3','zcode/glm-5.3-flash','kimi/kimi-k2.5'].map(id=>({id,source:id.split('/')[0],slug:id.split('/')[0],model:id.split('/')[1],kind:'api',status:'connected'}));
gatewayApi.groupList=async()=>[{id:'g1',slug:'daily-coding',name:'日常编码',strategy:'order',members:[{slug:'claude-work',model:'claude-sonnet-5-5'},{slug:'zcode',model:'glm-5.3'}],created_ms:1},{id:'g2',slug:'fast',name:'快',strategy:'order',members:[{slug:'zcode',model:'glm-5.3-flash'}],created_ms:2}];
gatewayApi.profilesList=async()=>[{name:'日常编码',agents:{},created_ms:1},{name:'省额度',agents:{},created_ms:2}];
gatewayApi.dayOverview=async()=>({today:1284,yesterday:1146,reroutes:17,errors:2,by_agent:{'claude-code':612,codex:540},partial:false,last_error:{id:9,ts:now-60000,agent:'codex',requested_model:'group/fast',source:'zcode',upstream_model:null,status_code:502,result:'failed',duration_ms:900}});
gatewayApi.agentDiscovery=async()=>[
  {agent:'claude-code',name:'Claude Code',installed:true,state:'installed',evidence:'bin',cli_found:true,cli_path:'~/.local/bin/claude'},
  {agent:'codex',name:'Codex',installed:true,state:'installed',evidence:'bin',cli_found:true,cli_path:null},
  {agent:'gemini',name:'Gemini CLI',installed:true,state:'installed',evidence:'bin',cli_found:true,cli_path:null},
  {agent:'opencode',name:'OpenCode',installed:true,state:'installed',evidence:'bin',cli_found:true,cli_path:'~/.local/bin/opencode'},
  {agent:'zcode',name:'ZCode',installed:false,state:'not_found',evidence:null,cli_found:false,cli_path:null}];
gatewayApi.wiringApplied=async()=>({model:'zcode/glm-5.3',subagent_model:null,tiers:{sonnet:'claude-work/claude-sonnet-5-5',haiku:'zcode/glm-5.3-flash'}});
const diff=(path,lines,keys,existed=true)=>({path,existed,keys,lines});
gatewayApi.wiringPreview=async(agent)=>agent==='claude-code'
  ? {mode:'wired',files:[diff('~/.claude/settings.json',[{op:'…',text:'3 行未改动'},{op:'-',text:'    "ANTHROPIC_BASE_URL": "https://api.anthropic.com",'},{op:'+',text:'    "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721",'},{op:'+',text:'    "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "131072",'},{op:' ',text:'  },'}],9)]}
  : {mode:'wire',files:[diff('~/.config/opencode/opencode.json',[{op:' ',text:'"provider": {'},{op:' ',text:'  "deepseek": { … 原样保留 … },'},{op:'+',text:'  "jolu": {'},{op:'+',text:'    "npm": "@ai-sdk/openai-compatible",'},{op:'+',text:'    "options": { "baseURL": "http://127.0.0.1:15721/v1", "apiKey": "jolu-opencode" }'},{op:'+',text:'  }'},{op:' ',text:'}'}],3)]};
settingsApi.agentVersions=async()=>[{agent:'claude-code',name:'Claude Code',found:true,version:'2.3.1',raw:null,update_command:null,update_note:''},{agent:'opencode',name:'OpenCode',found:true,version:'1.4.2',raw:null,update_command:null,update_note:''}];
skillsApi.overview=async(p)=>({platforms:[],items:Array.from({length:7},(_,i)=>({platform:p,enabled:true})),roots:[],projects:[],errors:[],scanned_at:now});
api.sessionsList=async()=>({sessions:Array.from({length:12},()=>({})),files:12});
automationApi.overview=async()=>({tasks:[{platform:'claude'},{platform:'claude'}],enabled:2,running:0,running_note:null,waiting:0,next_reset:null,done_today:0,failed_today:0});
gatewayApi.groups=async()=>({sources:[],agents:[]});
gatewayApi.sourceCapabilities=async()=>[];
gatewayApi.routeLog=async()=>[{id:3,ts:now-60000,agent:'claude-code',requested_model:'group/daily-coding',source:'Claude 订阅',status_code:200,result:'ok',duration_ms:3400},{id:2,ts:now-120000,agent:'codex',requested_model:'claude-haiku-4.5',source:'ZCode',status_code:200,result:'ok',duration_ms:900}];
gatewayApi.routeLogPage=async()=>({rows:[],page:1,page_size:10,total:0,page_count:0,upper_id:null});
gatewayApi.middlewares=async()=>[{name:'a',kind:'model-map',config:{},enabled:true,sort:0},{name:'b',kind:'word-guard',config:{},enabled:false,sort:1}];
gatewayApi.clientKeys=async()=>[];
gatewayApi.sessionsOverview=async()=>({seq:1,at:now,in_flight:[],sessions:[]});
gatewayApi.lanAddress=async()=>'192.168.1.23';
gatewayApi.allowIpsFromEnv=async()=>false;
const view=new URLSearchParams(location.search).get('view')||'agents';
const node=view==='agents'?React.createElement(AgentsView,{onNavigate(){}}):React.createElement(GatewayView,{onNavigate(){}});
ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(ToastProvider,null,React.createElement('div',{className:'h-screen bg-jolu-bg'},node)));
`

const browser = await chromium.launch({ channel: "msedge", headless: true })
const errors = []
const page = await browser.newPage({ viewport: { width: 1072, height: 796 } })
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
        if (cmd === "get_settings") return Promise.resolve({ claude_long_context: "", gateway_shared_lan: false })
        window.unexpectedNativeCalls.push(cmd)
        return Promise.reject(new Error(`合成环境禁止原生调用：${cmd}`))
      },
    }
  })
  await page.route("**/src/hooks/useConnections.ts*", (r) => r.fulfill({
    contentType: "application/javascript",
    body: `const connections=${JSON.stringify(connections)}; const reload=async()=>{}; export function useConnections(){return {connections,reload,loading:false,error:null}}`,
  }))
  await page.route("**/r2-check*", (r) => r.fulfill({ contentType: "text/html", body: `<!doctype html><html><head><meta charset="UTF-8"></head><body class="bg-jolu-bg"><div id="root"></div><script type="module">
import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true;
await import('/r2-fixture');</script></body></html>` }))
  await page.route("**/r2-fixture", (r) => r.fulfill({ contentType: "application/javascript", body: fixture }))

  // V5-1：已接入的 Claude Code
  await page.goto(`${base}/r2-check?view=agents`)
  await page.getByText("本机检测到 5 个工具").waitFor()
  await page.getByText("已接入 · 经模型路由调用模型").waitFor()
  for (const text of ["已接入", "可以接入", "暂时不能接入", "配置被改动，未经路由", "Skills 7", "会话 12", "自动化 2", "今日路由 612", "/model opus", "也跑标题、Explore 等后台小任务", "断开并还原", "日常编码"]) {
    assert.ok(await page.getByText(text, { exact: true }).first().isVisible(), `V5-1 缺少：${text}`)
  }
  await page.screenshot({ path: resolve(output, "v5-1-agent-wired.png") })
  await page.getByRole("button", { name: /写入的改动/ }).click()
  await page.getByText("CLAUDE_CODE_MAX_OUTPUT_TOKENS", { exact: false }).waitFor()
  await page.screenshot({ path: resolve(output, "v5-1-agent-diff.png"), fullPage: true })

  // V5-2：接入前预览（OpenCode）
  await page.getByRole("button", { name: /OpenCode/ }).first().click()
  await page.getByText("接入会做什么").waitFor()
  await page.getByText("写入预览 · 还没有改动").waitFor()
  assert.ok(await page.getByRole("button", { name: "接入 OpenCode" }).isVisible())
  await page.screenshot({ path: resolve(output, "v5-2-agent-preview.png") })

  // V5-3：模型路由概览
  await page.goto(`${base}/r2-check?view=gateway`)
  await page.getByText("模型路由运行中").waitFor()
  for (const text of ["今日请求", "1,284", "较昨天 +12%", "改道", "Agent 收到的错误", "请求怎么走", "需要你处理", "Gemini CLI 已不经过模型路由", "group/daily-coding", "模型池", "对外访问", "1 启用"]) {
    assert.ok(await page.getByText(text, { exact: true }).first().isVisible(), `V5-3 缺少：${text}`)
  }
  await page.screenshot({ path: resolve(output, "v5-3-routing-overview.png") })

  // V5-4：对外访问
  await page.getByRole("button", { name: /对外访问/ }).click()
  await page.getByText("http://192.168.1.23:15721").waitFor()
  assert.ok(await page.getByText("http://127.0.0.1:15721", { exact: true }).isVisible())
  await page.screenshot({ path: resolve(output, "v5-4-routing-access.png"), fullPage: true })

  // 模型池（只读）
  await page.getByRole("button", { name: /模型池/ }).click()
  await page.getByText("未加入模型池").waitFor()
  await page.screenshot({ path: resolve(output, "v5-5-pool.png") })

  assert.deepEqual(errors, [], `页面错误：${errors.join("\n")}`)
  console.log(JSON.stringify({ ok: true, screenshots: output, unmocked: await page.evaluate(() => [...new Set(window.unexpectedNativeCalls)]) }, null, 2))
} catch (error) {
  await page.screenshot({ path: resolve(output, "failure.png"), fullPage: true }).catch(() => undefined)
  console.error("页面错误：", errors, "未合成的原生调用：", await page.evaluate(() => window.unexpectedNativeCalls).catch(() => null))
  throw error
} finally {
  await browser.close()
}
