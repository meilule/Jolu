// 模型路由 · 会话观察（Jolu.pen V10-03～V10-14）预览验收：真实 GatewayView + 合成 API（不连接桌面、账号或上游）。
// 从概览「查看上下文」进入，核对链路、两张占用卡、组成（堆叠条 / 矩形图 / 筛选联动）、明细表展开（轮次、工具定义、单次工具返回与脱敏摘要）、
// Skills 来源证据、历史请求选择与跟随暂停（新请求只计数、恢复才切换）、各采集状态；截浅色 / 深色图对照画板。
// 用法：pnpm --dir frontend exec vite --port 5198 --strictPort；在仓库根目录 JOLU_PREVIEW_URL=http://localhost:5198 node scripts/acceptance/context-observation-preview.e2e.mjs
// 依赖：output/acceptance/node 里的 playwright-core 与本机 Chrome（没有时用 Edge）。截图在 output/acceptance/context-observation/。
import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { mkdir } from "node:fs/promises"
import { resolve } from "node:path"
const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [resolve("output/acceptance/node")] }))
const base = process.env.JOLU_PREVIEW_URL || "http://localhost:5198"
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(new URL(base).hostname))
const output = resolve("output/acceptance/context-observation")
await mkdir(output, { recursive: true })

const fixture = `
import React from '/node_modules/.vite/deps/react.js'; import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import { GatewayView } from '/src/views/GatewayView.tsx';
import { gatewayApi } from '/src/lib/gatewayApi.ts';
import { ToastProvider } from '/src/components/ui/Toast.tsx'; import '/src/index.css';
const now=Date.now();
const link=(platform,id,agent,source,model)=>({session_platform:platform,session_id:id,connection_id:null,source,requested_model:source+'/'+model,upstream_model:model,agent,last_ts:now-8000,requests:126});
const session=(platform,id,agent,title,cwd,links)=>({platform,session_id:id,agent,title,cwd,thread_last_ts:now-60000,gateway_last_ts:now-8000,thread_status:null,tokens:{},links});
gatewayApi.sessionsOverview=async()=>({seq:1,at:now,in_flight:[],sessions:[
  session('claude','ses-claude','claude-code','Jolu 界面优化','D:/develop/Jolu',[link('claude','ses-claude','claude-code','workbuddy','deepseek-v4.1-flash-sg')]),
  session('opencode','opencode:ses-oc','opencode','路由实验','D:/develop/Jolu',[link('opencode','opencode:ses-oc','opencode','kimi','kimi-k2')]),
  session('codex','ses-new','codex','没有快照的会话','D:/develop/Jolu',[]),
]});
const summary=(id,over)=>({id,session_platform:'claude',session_id:'ses-claude',request_id:id,ts:now-(130-id)*60000,state:'ready',protocol:'anthropic',
  input_tokens:631000,output_tokens:2800,cache_read:602000,window_tokens:1000000,estimated:600000,counted:true,error:null,agent:'claude-code',
  requested_model:'group/jolu-dev',upstream_model:'deepseek-v4.1-flash-sg',source:'workbuddy',route_result:'ok',status_code:200,duration_ms:18400,
  connection_id:'c2',connection_name:'个人账号',connection_platform:'workbuddy',...over});
let history=[summary(126),summary(125,{route_result:'failed',input_tokens:null,cache_read:null,estimated:590000,counted:false}),summary(124,{input_tokens:612000})];
window.__addRequest=()=>{const id=history[0].id+1;history=[summary(id),...history]};
gatewayApi.contextHistory=async(platform,sid)=>sid==='ses-claude'?history:sid==='opencode:ses-oc'?[{...summary(77),session_platform:'opencode',session_id:sid,state:'unsupported',estimated:null,counted:false,cache_read:null}]:[];
const parts=[
  {kind:'system',tokens:35300,items:[{name:'prompt',tag:'',tokens:35000},{name:'billing',tag:'',tokens:300}]},
  {kind:'tools',tokens:27100,items:[{name:'Bash',tag:'',tokens:9000},{name:'linear',tag:'mcp',n:3,tokens:18000},{name:'Read',tag:'',tokens:100}]},
  {kind:'skills',tokens:16600,items:[{name:'surgical-code-change',tag:'loaded',tokens:8400},{name:'skills',tag:'skills',tokens:8200}]},
  {kind:'memory',tokens:14200,items:[{name:'D:\\\\develop\\\\Jolu\\\\AGENTS.md',tag:'project',tokens:12000},{name:'reminders',tag:'reminder',tokens:2200}]},
  {kind:'files',tokens:112400,items:[{name:'backend/src/gateway/route.rs',tag:'read',tokens:18400},{name:'frontend/src/views/AgentsView.tsx',tag:'read',tokens:13200},{name:'more',tag:'more',n:10,tokens:80800}]},
  {kind:'results',tokens:206800,items:[{name:'Bash',tag:'result',n:5,tokens:88800},{name:'Read',tag:'result',n:83,tokens:76200},{name:'linear',tag:'result',n:12,tokens:41800}]},
  {kind:'chat',tokens:180000,items:[{name:'earlier',tag:'turns',n:39,tokens:120000},...Array.from({length:23},(_,i)=>({name:String(40+i),tag:'turn',tokens:i===22?5000:2500}))]},
  {kind:'unknown',tokens:38600,items:[{name:'hologram',tag:'type',tokens:38600}]},
];
gatewayApi.contextSnapshot=async(platform,sid,id)=>{
  const row=(await gatewayApi.contextHistory(platform,sid)).find(r=>r.id===(id??history[0].id));
  if(!row) return null;
  if(row.state!=='ready') return {...row,parts:null,tools:[]};
  return {...row,parts,tools:[{tool:'Bash',count:5,tokens:88800}]};
};
gatewayApi.contextToolItems=async(id,tool)=>tool!=='Bash'?[]:['pnpm --dir frontend typecheck','git diff --stat','rg --files frontend/src','cat ~\\\\proj\\\\.env','cargo test'].map((t,i)=>({tool:'Bash',target:t,kind:'results',tokens:600+i*400,lines:12-i,exit_code:0,masked:t.includes('~'),summary:'退出码 0，输出 '+(12-i)+' 行',captured:false}));
const ctx=(platform,id,agent,over)=>({...summary(126),session_platform:platform,session_id:id,agent,last_ready_ts:now,snapshots:3,...over});
gatewayApi.contextSessions=async()=>[ctx('claude','ses-claude','claude-code',{}),ctx('opencode','opencode:ses-oc','opencode',{id:77,state:'unsupported',cache_read:null})];
gatewayApi.status=async()=>({mode:'relay',enabled:true,port:24711,circuit_threshold:5,auto_escape:true,default_connection:'c1',wiring:[]});
gatewayApi.catalog=async()=>[];
gatewayApi.groupList=async()=>[{id:'g1',slug:'jolu-dev',name:'Jolu 开发 · 优先高质量',strategy:'order',members:[],created_ms:1}];
gatewayApi.dayOverview=async()=>({today:248,yesterday:216,reroutes:4,errors:2,by_agent:{},partial:false,last_error:null});
gatewayApi.agentDiscovery=async()=>[];
gatewayApi.groups=async()=>({sources:[],agents:[]});
gatewayApi.sourceCapabilities=async()=>[];
gatewayApi.routeLogPage=async()=>({rows:[],page:1,page_size:240,total:0,page_count:0,upper_id:null});
gatewayApi.routeLog=async()=>[];
gatewayApi.middlewares=async()=>[];
gatewayApi.clientKeys=async()=>[];
gatewayApi.lanAddress=async()=>null;
if(new URLSearchParams(location.search).get('theme')==='dark') document.documentElement.classList.add('dark');
ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(ToastProvider,null,React.createElement('div',{className:'h-screen overflow-auto bg-jolu-bg'},React.createElement(GatewayView,{onNavigate(){}}))));
`

const browser = await chromium.launch({ channel: "chrome", headless: true }).catch(() => chromium.launch({ channel: "msedge", headless: true }))
const errors = []
const page = await browser.newPage({ viewport: { width: 1072, height: 1100 } })
const visible = async (text) => {
  const matches = await page.getByText(text, { exact: true }).all()
  return (await Promise.all(matches.map((m) => m.isVisible()))).some(Boolean)
}
const expectText = async (texts, where) => {
  for (const text of texts) assert.ok(await visible(text), `${where}：缺少「${text}」`)
}
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
    body: "const connections=[]; const reload=async()=>{}; export function useConnections(){return {connections,reload,loading:false,error:null}}",
  }))
  await page.route("**/obs-check*", (r) => r.fulfill({ contentType: "text/html", body: `<!doctype html><html><head><meta charset="UTF-8"></head><body class="bg-jolu-bg"><div id="root"></div><script type="module">
import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true;
await import('/obs-fixture');</script></body></html>` }))
  await page.route("**/obs-fixture", (r) => r.fulfill({ contentType: "application/javascript", body: fixture }))

  for (const theme of ["light", "dark"]) {
    await page.goto(`${base}/obs-check?theme=${theme}`)
    await page.locator("#active-sessions").getByText("Jolu 界面优化").waitFor()
    await page.locator("#active-sessions div.grid", { hasText: "Jolu 界面优化" }).getByRole("button", { name: "查看上下文" }).click()
    await page.getByText("上下文组成", { exact: true }).waitFor()
    await page.getByText("系统提示词 35.3K").waitFor()
    await expectText([
      "当前会话 · 126 次请求", "最近一次请求 · #126", "跟随最新",
      "group/jolu-dev", "Jolu 开发 · 优先高质量", "deepseek-v4.1-flash-sg · 个人账号",
      "● 已完成", "完整用时 18.4s", "输入 631K　/　输出 2.8K Token",
      "上下文占用", "上游实测", "631K / 1M", "63.1%", "剩余上下文空间 369K · 窗口容量由模型元数据提供",
      "提示词缓存", "602K", "占输入 95.4%", "读取量已包含在输入中，不重复相加",
      "工具定义 27.1K", "工具结果 206.8K", "会话 180K", "未分类 38.6K",
      "会话内容", "第 1–62 轮 · 按轮次展开", "62 轮", "Bash 工具结果", "工具定义", "本次实际提供的 5 个工具",
      "未分类内容", "缺少来源标识 · 不推断为技能或文件", "AGENTS.md", "本次加载证据 · Skill 读取结果",
    ], `${theme} 会话观察`)
    await page.screenshot({ path: resolve(output, `v10-0${theme === "light" ? 3 : 4}-observation-${theme}.png`), fullPage: true })
  }

  // V10-05：展开 Bash 工具结果，默认 3 条、可看全部；脱敏摘要 / 正文未采集；展开即暂停跟随
  await page.getByRole("button", { name: /Bash 工具结果/ }).click()
  await page.getByText("本次输入保留 5 条返回 · 合计 7K Token（按实测换算）").waitFor()
  await expectText(["cargo test", "cat ~\\proj\\.env", "跟随已暂停", "恢复跟随最新 →"], "工具结果展开")
  assert.equal(await page.getByText("pnpm --dir frontend typecheck").count(), 0, "默认只显示最近 3 条")
  assert.ok(await page.getByText("已脱敏摘要（本机路径、用户名或凭证已替换）　/　正文未采集").first().isVisible())
  await page.getByRole("button", { name: "查看全部 5 条 →" }).click()
  await page.getByText("pnpm --dir frontend typecheck").waitFor()
  await page.screenshot({ path: resolve(output, "v10-05-tool-results.png"), fullPage: true })

  // 恢复跟随：收起展开
  await page.getByRole("button", { name: "恢复跟随最新 →" }).click()
  await page.getByText("跟随最新", { exact: true }).waitFor()
  assert.equal(await page.getByText("pnpm --dir frontend typecheck").count(), 0, "恢复跟随后收起展开")

  // V10-09：矩形图 + 点选「文件」筛选联动
  await page.getByRole("button", { name: "矩形图" }).click()
  await page.locator("button[title^='文件 ·']").click()
  await page.getByText("已选：文件 · 112.4K（17.8%）　/　点击空白或「全部」清除筛选").waitFor()
  await expectText(["文件 · 3 项", "backend/src/gateway/route.rs", "其余 10 项"], "文件筛选")
  await page.screenshot({ path: resolve(output, "v10-09-treemap-files.png"), fullPage: true })
  await page.getByRole("button", { name: "全部", exact: true }).click()

  // V10-11：Skills 筛选
  await page.getByRole("group", { name: "按类别筛选" }).getByRole("button", { name: "Skills" }).click()
  await expectText(["Skills · 2 项", "surgical-code-change", "技能列表"], "Skills 筛选")
  await page.getByRole("group", { name: "按类别筛选" }).getByRole("button", { name: "全部" }).click()

  // V10-07：选历史请求 #125（失败、用量未提供），暂停；新请求只计数；恢复才切到最新
  await page.getByRole("button", { name: /最近一次请求|历史请求/ }).click()
  await expectText(["选择本次会话的一次请求", "最新 · 已选 · 已完成 · 输入 631K", "请求失败 · 估算 590K"], "历史下拉")
  await page.getByRole("option", { name: /#125/ }).click()
  await page.getByText("历史请求 · #125").waitFor()
  await expectText(["● 请求失败", "590K / 1M", "本地估算"], "历史请求 #125")
  await page.evaluate(() => window.__addRequest())
  await page.getByText("已有 1 次新请求，内容保持不变。", { exact: false }).waitFor({ timeout: 8000 })
  assert.ok(await visible("历史请求 · #125"), "新请求不替换已选快照")
  await page.screenshot({ path: resolve(output, "v10-07-history-paused.png"), fullPage: true })
  await page.getByRole("button", { name: "恢复跟随最新 →" }).click()
  await page.getByText("最近一次请求 · #127").waitFor()

  // V10-13：不支持分析的来源；没有快照的会话
  await page.getByRole("button", { name: "运行概览" }).click()
  await page.locator("#active-sessions div.grid", { hasText: "路由实验" }).getByRole("button", { name: "查看上下文" }).click()
  await page.getByText("此来源暂不支持内容分析").waitFor()
  await page.getByRole("button", { name: "运行概览" }).click()
  await page.locator("#active-sessions div.grid", { hasText: "没有快照的会话" }).getByRole("button", { name: "查看上下文" }).click()
  await page.getByText("这个会话还没有经过 Jolu 的模型路由。等待下一次请求；不显示历史累计用量作为上下文。").waitFor()

  const unexpected = await page.evaluate(() => window.unexpectedNativeCalls)
  assert.deepEqual(unexpected, [], `合成环境出现未处理的原生调用：${unexpected.join(", ")}`)
  assert.deepEqual(errors, [], `控制台错误：\n${errors.join("\n")}`)
  console.log(`通过：V10-03～V10-14 会话观察。截图在 ${output}`)
} finally {
  await browser.close()
}
