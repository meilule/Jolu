// Agent 与模型路由（jolu-new.pen R1）浏览器预览验收：
// 真实 AgentsView / GatewayView + 合成 API；不连接真实桌面、账号或上游，不读写任何工具配置。
// 用法：pnpm --dir frontend exec vite --port 5197 --strictPort
//       node scripts/acceptance/agents-routing-r2.e2e.mjs
// 依赖：output/acceptance/node 里的 playwright-core 与本机 Edge。截图在 output/acceptance/agents-routing-r1/。
import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { mkdir } from "node:fs/promises"
import { resolve } from "node:path"

const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [resolve("output/acceptance/node")] }))
const base = process.env.JOLU_PREVIEW_URL || "http://127.0.0.1:5197"
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(new URL(base).hostname))
const output = resolve("output/acceptance/agents-routing-r1")
await mkdir(output, { recursive: true })

const connections = [
  ["c1", "Momo", "claude", "auth", "claude-momo", true],
  ["c2", "ZCode 个人套餐", "zcode", "auth", "zcode", true],
  ["c3", "Kimi Key", "custom", "api", "kimi", true],
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
import { ToastProvider } from '/src/components/ui/Toast.tsx';
import { PageActionsProvider, PageHeader, Sidebar, WindowShell } from '/src/ui/index.ts';
import '/src/index.css';
const now=Date.now();
const wiring=[
  {agent:'claude-code',wired:true,state:'wired',config_path:'~/.claude/settings.json',notice:'新会话生效；已在运行的会话需重开',current_model:'claude-sonnet-4-5',current_resolved:'claude-momo/claude-sonnet-4-5'},
  {agent:'codex',wired:true,state:'wired',config_path:'~/.codex/config.toml',notice:'需重启 Codex 后生效',codex_mode:'oauth',current_model:'gpt-5'},
  {agent:'gemini',wired:true,state:'wired',drift:'model_changed',config_path:'~/.gemini/.env',current_model:'gemini-2.5-pro'},
  {agent:'opencode',wired:false,config_path:'~/.config/opencode/opencode.json'},
  {agent:'zcode',wired:false,config_path:null,notice:null,reason:'尚未安装或未生成配置文件'},
];
gatewayApi.status=async()=>({mode:'relay',enabled:true,port:12731,circuit_threshold:5,auto_escape:true,default_connection:'c2',wiring});
gatewayApi.catalog=async()=>[
  {id:'m1',slug:'claude-momo',model:'claude-sonnet-4-5',source:'Claude',kind:'auth',status:'connected',context_window:200000},
  {id:'m2',slug:'claude-momo',model:'claude-opus-4-5',source:'Claude',kind:'auth',status:'connected',context_window:200000},
  {id:'m3',slug:'zcode',model:'glm-5.3',source:'ZCode',kind:'auth',status:'connected',context_window:128000},
  {id:'m4',slug:'kimi',model:'kimi-k2.5',source:'Kimi',kind:'api',status:'connected',context_window:262144},
];
gatewayApi.groups=async()=>({sources:[],agents:[]});
gatewayApi.groupList=async()=>[
  {id:'g1',slug:'coding',name:'编码主力',strategy:'usage',members:[{slug:'claude-momo',model:'claude-sonnet-4-5'},{slug:'zcode',model:'glm-5.3'},{slug:'kimi',model:'kimi-k2.5'}],created_ms:1},
  {id:'g2',slug:'light',name:'轻量问答',strategy:'order',members:[{slug:'zcode',model:'glm-5.3'},{slug:'kimi',model:'kimi-k2.5'}],created_ms:2},
];
gatewayApi.profilesList=async()=>[{name:'日常编码',agents:{},created_ms:1}];
gatewayApi.sourceCapabilities=async()=>[];
gatewayApi.dayOverview=async()=>({today:846,yesterday:712,reroutes:14,errors:3,by_agent:{'claude-code':612},partial:false,identity_incomplete:false,last_error:null});
gatewayApi.routeLogPage=async()=>({rows:[
  {id:5,ts:now-60000,agent:'claude-code',requested_model:'claude-sonnet-4-5',upstream_model:'claude-sonnet-4-5',source:'claude-momo',connection_id:'c1',result:'ok',status_code:200,duration_ms:900},
  {id:4,ts:now-120000,agent:'codex',requested_model:'gpt-5',upstream_model:'gpt-5',source:'claude-momo',connection_id:'c1',result:'ok',status_code:200,duration_ms:1700},
  {id:3,ts:now-180000,agent:'gemini',requested_model:'gemini-2.5-pro',source:'zcode',connection_id:'c2',result:'failed',status_code:502,duration_ms:3400},
],page:1,page_size:10,total:3,page_count:1,upper_id:5});
gatewayApi.contextSessions=async()=>[];
gatewayApi.agentDiscovery=async()=>[
  {agent:'claude-code',name:'Claude Code',installed:true,state:'installed',evidence:'bin',cli_found:true,cli_path:'~/.local/bin/claude'},
  {agent:'codex',name:'Codex',installed:true,state:'installed',evidence:'bin',cli_found:true,cli_path:null},
  {agent:'gemini',name:'Gemini CLI',installed:true,state:'installed',evidence:'bin',cli_found:true,cli_path:null},
  {agent:'opencode',name:'OpenCode',installed:true,state:'installed',evidence:'bin',cli_found:true,cli_path:'~/.local/bin/opencode'},
  {agent:'zcode',name:'ZCode',installed:false,state:'not_found',evidence:null,cli_found:false,cli_path:null}];
gatewayApi.wiringApplied=async()=>({model:'zcode/glm-5.3',subagent_model:null,tiers:{sonnet:'claude-momo/claude-sonnet-4-5',haiku:'zcode/glm-5.3'}});
const diff=(path,lines,keys,existed=true)=>({path,existed,keys,lines});
gatewayApi.wiringPreview=async()=>({mode:'wired',files:[diff('~/.claude/settings.json',[{op:'…',text:'3 行未改动'},{op:'-',text:'    "ANTHROPIC_BASE_URL": "https://api.anthropic.com",'},{op:'+',text:'    "ANTHROPIC_BASE_URL": "http://127.0.0.1:12731",'},{op:'+',text:'    "ANTHROPIC_AUTH_TOKEN": "jolu-••••••••3f2a",'},{op:' ',text:'  },'}],9)]});
settingsApi.agentVersions=async()=>[{agent:'claude-code',name:'Claude Code',found:true,version:'2.3.1',raw:null,update_command:null,update_note:''}];
gatewayApi.customAgents=async()=>[{id:'custom-agent:crush',name:'Crush',slug:'crush',config_path:'~/.config/crush/crush.json',format:'json',base_url_key:'providers.jolu.base_url',auth_key:'providers.jolu.api_key',model_key:'models.large.model',base_suffix:'/v1',installed:true,wired:false,drift:null,model:null}];
gatewayApi.customAgentPreview=async()=>({mode:'wire',files:[diff('~/.config/crush/crush.json',[{op:'+',text:'  "providers": {'},{op:'+',text:'    "jolu": { "base_url": "http://127.0.0.1:12731/v1" }'},{op:'+',text:'  }'}],2,false)]});
gatewayApi.middlewares=async()=>[
  {name:'隐私脱敏',kind:'word-guard',config:{block:'',mask:'sk-,token'},enabled:true,sort:0},
  {name:'长上下文自动改写',kind:'model-map',config:{fast:'zcode/glm-5.3'},enabled:true,sort:1},
  {name:'限制输出上限',kind:'param-override',config:{set:'max_tokens=4096'},enabled:false,sort:2},
];
gatewayApi.clientKeys=async()=>[
  {name:'笔记本 · 只读',masked:'jolu-…3f81',models:null,read_only:true,today_requests:42,requests_per_day:500,expires_ms:null,last_used_ms:now-18*60000,budget:null,accounts:[],max_concurrency:null},
  {name:'办公台式',masked:'jolu-…9ac2',models:['group/coding','zcode/glm-5.3'],read_only:false,today_requests:168,requests_per_day:null,expires_ms:now+40*86400000,last_used_ms:now-3*3600000,budget:null,accounts:['c1','c2'],max_concurrency:2},
];
gatewayApi.sessionsOverview=async()=>({seq:1,at:now,in_flight:[],sessions:[]});
gatewayApi.lanAddress=async()=>'192.168.1.23';
gatewayApi.modelsDevStatus=async()=>[now-86400000,120];
gatewayApi.allowIpsFromEnv=async()=>false;
const view=new URLSearchParams(location.search).get('view')||'agents';
const node=view==='agents'?React.createElement(AgentsView,{onNavigate(){}}):React.createElement(GatewayView,{onNavigate(){},onOpenConnection(){}});
const shell=React.createElement(PageActionsProvider,null,
  React.createElement(WindowShell,{
    sidebar:React.createElement(Sidebar,{collapsed:false,logo:React.createElement('span',{className:'px-2 text-sm font-bold text-jn-text'},'Jolu')},
      React.createElement('div',{className:'px-3 text-[11px] text-jn-text-3'},'导航占位')),
    header:React.createElement(PageHeader,{title:view==='agents'?'Agent':'模型路由'}),
  },node));
ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(ToastProvider,null,React.createElement('div',{className:'h-screen w-full bg-jn-stage'},shell)));
`

const browser = await chromium.launch({ channel: "msedge", headless: true })
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
const errors = []
const noise = (text) => /WebSocket|\[vite\]|Failed to load resource/.test(text)
const shot = async (name) => {
  await page.waitForTimeout(500)
  await page.screenshot({ path: resolve(output, name) })
}
const visible = async (text) => assert.ok(await page.getByText(text).first().isVisible(), `缺少：${text}`)

page.on("pageerror", (error) => errors.push(error.message))
page.on("console", (msg) => { if (msg.type() === "error" && !noise(msg.text())) errors.push(msg.text()) })

try {
  await page.addInitScript(() => {
    const callbacks = new Map()
    let next = 1
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      unregisterListener() {},
      transformCallback(cb) { const id = next++; callbacks.set(id, cb); return id },
      unregisterCallback(id) { callbacks.delete(id) },
      invoke(cmd, args) {
        if (cmd === "plugin:event|listen") return Promise.resolve(args.handler)
        if (cmd === "plugin:event|unlisten") return Promise.resolve()
        if (cmd === "get_settings") return Promise.resolve({ claude_long_context: "", gateway_shared_lan: true, gateway_allow_ips: [] })
        return Promise.reject(new Error(`合成环境禁止原生调用：${cmd}`))
      },
    }
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  })
  await page.route("**/src/hooks/useConnections.ts*", (r) => r.fulfill({
    contentType: "application/javascript",
    body: `const connections=${JSON.stringify(connections)}; const reload=async()=>{}; export function useConnections(){return {connections,reload,loading:false,error:null}}`,
  }))
  await page.route("**/r1-check*", (r) => r.fulfill({
    contentType: "text/html",
    body: `<!doctype html><html><head><meta charset="UTF-8"></head><body><div id="root"></div><script type="module">
import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true;
await import('/r1-fixture');</script></body></html>`,
  }))
  await page.route("**/r1-fixture", (r) => r.fulfill({ contentType: "application/javascript", body: fixture }))

  // ── Agent：R1-08 列表 → R1-09 详情 → R1-32 高级 → R1-33 写入改动 ──
  await page.goto(`${base}/r1-check?view=agents`)
  await page.getByText("接入模型路由，为每个工具配置顺手的模型").waitFor({ timeout: 15000 })
  for (const text of ["全部 6", "已接入 3", "可接入 2", "Claude Code", "Codex", "OpenCode", "Crush", "暂不能接入"]) await visible(text)
  await shot("agents-r1-08-list.png")

  await page.getByText("详细", { exact: true }).first().click()
  await page.getByText("新会话默认模型").first().waitFor()
  for (const text of ["模型设置", "高级", "写入改动", "Claude Code 档位", "今天经过路由", "能力验证", "已接入 · 配置一致"]) await visible(text)
  await shot("agents-r1-09-detail.png")

  await page.getByRole("tab", { name: "高级" }).click()
  await page.getByText("模型与压缩").waitFor()
  await visible("子代理模型")
  await shot("agents-r1-32-advanced.png")

  await page.getByRole("tab", { name: "写入改动" }).click()
  await page.getByText("改了 1 个文件 · 9 个键 · 断开时逐键还原").waitFor()
  await shot("agents-r1-33-diff.png")

  await page.getByRole("button", { name: "全部工具" }).click()
  await page.getByText("接入模型路由，为每个工具配置顺手的模型").waitFor()

  // ── 模型路由：R1-01 概览 + R1-18～22 五个页签 ──
  await page.goto(`${base}/r1-check?view=gateway`)
  await page.getByText("本机路由").first().waitFor({ timeout: 15000 })
  for (const text of ["今日请求", "今日 Token", "局域网共享", "中间件链", "来源额度", "需要关注", "响应头耗时", "活跃会话", "路由分组"]) await visible(text)
  await shot("gateway-r1-01-overview.png")

  const tabs = [
    ["路由流水", "route-log", ["全部 Agent", "全部结果", "暂停刷新", "Claude Code", "成功"]],
    ["路由分组", "groups", ["编码主力", "选择策略", "成员模型", "保存更改"]],
    ["模型池", "pool", ["模型池", "Momo", "claude-momo/claude-sonnet-4-5", "管理来源"]],
    ["对外访问", "access", ["局域网共享", "客户端 Key", "笔记本 · 只读", "签发 Key"]],
    ["中间件", "middleware", ["隐私脱敏", "中间件规则 · 2 条", "长上下文自动改写", "保存前用样例请求校验"]],
  ]
  for (const [tab, name, texts] of tabs) {
    await page.getByRole("tab", { name: tab }).click()
    await page.getByText(texts[0]).first().waitFor()
    for (const text of texts) await visible(text)
    await shot(`gateway-${name}.png`)
  }

  assert.deepEqual(errors, [], `控制台有错误：\n${errors.join("\n")}`)
  console.log(`通过：Agent 4 张 + 模型路由 6 张截图，见 ${output}`)
} finally {
  await browser.close()
}
