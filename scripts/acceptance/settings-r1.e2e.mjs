// 设置页（jolu-new.pen R1-12～17）浏览器预览验收：
// 真实 SettingsPage + 浏览器默认设置（isTauri=false），逐分类截图并核对分组与关键行。
// 用法：pnpm --dir frontend exec vite --port 5197 --strictPort
//       node scripts/acceptance/settings-r1.e2e.mjs
// 依赖：output/acceptance/node 里的 playwright-core 与本机 Edge。截图在 output/acceptance/settings-r1/。
import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { mkdir } from "node:fs/promises"
import { resolve } from "node:path"

const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [resolve("output/acceptance/node")] }))
const base = process.env.JOLU_PREVIEW_URL || "http://127.0.0.1:5197"
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(new URL(base).hostname))
const output = resolve("output/acceptance/settings-r1")
await mkdir(output, { recursive: true })

const fixture = `
import React from '/node_modules/.vite/deps/react.js'; import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import { SettingsPage } from '/src/views/SettingsPage.tsx';
import { ToastProvider } from '/src/components/ui/Toast.tsx';
import { UpdateProvider } from '/src/components/panel/UpdateContext.tsx';
import { PageActionsProvider, PageHeader, Sidebar, WindowShell } from '/src/ui/index.ts';
import '/src/index.css';
const view=React.createElement(SettingsPage,{section:null,navigationRequest:0,onOpenSkills(){}});
const shell=React.createElement(PageActionsProvider,null,
  React.createElement(WindowShell,{
    sidebar:React.createElement(Sidebar,{collapsed:false,logo:React.createElement('span',{className:'px-2 text-sm font-bold text-jn-text'},'Jolu')},
      React.createElement('div',{className:'px-3 text-[11px] text-jn-text-3'},'导航占位')),
    header:React.createElement(PageHeader,{title:'设置'}),
  },view));
ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(ToastProvider,null,React.createElement(UpdateProvider,null,React.createElement('div',{className:'h-screen w-full bg-jn-stage'},shell))));
`

const browser = await chromium.launch({ channel: "msedge", headless: true })
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
const errors = []
const noise = (text) => /WebSocket|\[vite\]|Failed to load resource|not-tauri/.test(text)
const visible = async (text) => assert.ok(await page.getByText(text).first().isVisible(), `缺少：${text}`)
const shot = async (name) => {
  await page.waitForTimeout(400)
  await page.screenshot({ path: resolve(output, name) })
}

page.on("pageerror", (error) => errors.push(error.message))
page.on("console", (msg) => { if (msg.type() === "error" && !noise(msg.text())) errors.push(msg.text()) })

try {
  await page.route("**/s1-check*", (r) => r.fulfill({
    contentType: "text/html",
    body: `<!doctype html><html><head><meta charset="UTF-8"></head><body><div id="root"></div><script type="module">
import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true;
await import('/s1-fixture');</script></body></html>`,
  }))
  await page.route("**/s1-fixture", (r) => r.fulfill({ contentType: "application/javascript", body: fixture }))
  await page.goto(`${base}/s1-check`)
  await page.getByText("启动偏好").waitFor({ timeout: 15000 })

  const tabs = [
    ["常规", "r1-12-general.png", ["启动偏好", "开机启动", "静默启动", "默认打开页面", "数据刷新", "自动刷新间隔", "立即刷新"]],
    ["外观", "r1-13-appearance.png", ["界面主题", "浅色", "深色", "跟随系统", "阅读与布局", "界面缩放", "列表密度", "动效偏好", "减少动效"]],
    ["网络与代理", "r1-14-network.png", ["应用联网", "CLI 本地代理", "隐私脱敏", "连接方式", "失败自动回退直连"]],
    ["数据与存储", "r1-15-data.png", ["存储位置", "历史数据保留", "导入与导出", "自动清理规则", "手动清理历史", "导入已有数据", "导出本地数据"]],
    ["通知", "r1-16-notifications.png", ["任务提醒", "额度与余额提醒", "免打扰", "低余额提醒", "提醒阈值"]],
    ["关于与更新", "r1-17-about.png", ["应用更新", "帮助与项目", "自动检查更新", "检查间隔", "更新日志", "GitHub", "问题反馈"]],
  ]
  for (const [tab, name, texts] of tabs) {
    await page.getByRole("button", { name: tab }).click()
    await page.getByText(texts[0]).first().waitFor()
    for (const text of texts) await visible(text)
    await shot(name)
  }

  assert.deepEqual(errors, [], `控制台有错误：\n${errors.join("\n")}`)
  console.log(`通过：设置 6 个分类截图，见 ${output}`)
} finally {
  await browser.close()
}
