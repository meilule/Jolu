#!/usr/bin/env node
/**
 * 快照自检：用无头 Chromium 打开单文件快照，逐页截图并收集控制台错误。
 *
 * 用法（仓库根目录）：
 *   node scripts/dev/check-snapshot.mjs [--out output/snapshot-shots] [--keep]
 *
 * 用 Node 内建的 WebSocket 走 CDP，不依赖 playwright / puppeteer。
 * 只读操作：不会写仓库里的任何文件，截图落在 output/（已被 Git 忽略）。
 */
import { spawn } from "node:child_process"
import { existsSync } from "node:fs"
import { mkdir, writeFile, rm } from "node:fs/promises"
import { resolve, dirname, join } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const HTML = resolve(ROOT, "output/jolu-ui-snapshot.html")
const OUT = resolve(ROOT, process.argv.includes("--out")
  ? process.argv[process.argv.indexOf("--out") + 1]
  : "output/snapshot-shots")
const PROFILE = resolve(ROOT, "output/.snapshot-chrome")

const CANDIDATES = [
  process.env.JOLU_CHROME,
  "C:/Users/林凯东/AppData/Local/ms-playwright/chromium_headless_shell-1247/chrome-headless-shell-win64/chrome-headless-shell.exe",
  "C:/Program Files/Google/Chrome/Application/chrome.exe",
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
]
const CHROME = CANDIDATES.find((path) => path && existsSync(path))
if (!CHROME) throw new Error("找不到 Chromium / Chrome / Edge，可用 JOLU_CHROME 指定")
if (!existsSync(HTML)) throw new Error(`先构建快照：pnpm snapshot（缺 ${HTML}）`)

const PAGES = [
  ["island", "灵动岛"],
  ["providers", "供应商"],
  ["checkin", "一键签到"],
  ["usage", "用量统计"],
  ["sessions", "会话线程"],
  ["automation", "自动化"],
  ["skills", "Skills 管理"],
  ["agents", "Agent"],
  ["gateway", "模型路由"],
  ["settings", "设置"],
]

await mkdir(OUT, { recursive: true })
await rm(PROFILE, { recursive: true, force: true })

const port = 9333
const chrome = spawn(CHROME, [
  "--headless=new",
  `--remote-debugging-port=${port}`,
  `--user-data-dir=${PROFILE}`,
  "--allow-file-access-from-files",
  "--hide-scrollbars",
  "--force-device-scale-factor=2",
  "--window-size=1440,1000",
  "about:blank",
], { stdio: ["ignore", "pipe", "pipe"] })

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

async function json(path) {
  for (let i = 0; i < 60; i++) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}${path}`)
      if (response.ok) return response.json()
    } catch { /* 还没起来 */ }
    await sleep(200)
  }
  throw new Error("CDP 未就绪：" + path)
}

const targets = await json("/json/list")
const pageTarget = targets.find((target) => target.type === "page")
if (!pageTarget) throw new Error("没有可用的页面目标")

const ws = new WebSocket(pageTarget.webSocketDebuggerUrl)
let nextId = 1
const pending = new Map()
const events = []

ws.addEventListener("message", (event) => {
  const message = JSON.parse(event.data)
  if (message.id && pending.has(message.id)) {
    const { resolve: done, reject } = pending.get(message.id)
    pending.delete(message.id)
    if (message.error) reject(new Error(JSON.stringify(message.error)))
    else done(message.result)
    return
  }
  if (message.method === "Runtime.consoleAPICalled") {
    const text = message.params.args.map((a) => a.value ?? a.description ?? a.type).join(" ")
    events.push({ kind: "console", level: message.params.type, text })
  }
  if (message.method === "Runtime.exceptionThrown") {
    const detail = message.params.exceptionDetails
    events.push({ kind: "exception", level: "error", text: detail.exception?.description ?? detail.text })
  }
  if (message.method === "Log.entryAdded") {
    const entry = message.params.entry
    if (entry.level === "error" || entry.level === "warning") events.push({ kind: "log", level: entry.level, text: entry.text + (entry.url ? ` (${entry.url})` : "") })
  }
})

const send = (method, params = {}) => new Promise((done, reject) => {
  const id = nextId++
  pending.set(id, { resolve: done, reject })
  ws.send(JSON.stringify({ id, method, params }))
})

await new Promise((done, reject) => {
  ws.addEventListener("open", done, { once: true })
  ws.addEventListener("error", reject, { once: true })
})

await send("Runtime.enable")
await send("Log.enable")
await send("Page.enable")
await send("Emulation.setDeviceMetricsOverride", { width: 1440, height: 1000, deviceScaleFactor: 2, mobile: false })

const evaluate = async (expression) => {
  const result = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.text + " :: " + expression)
  return result.result.value
}

const shot = async (name) => {
  const { data } = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true })
  await writeFile(join(OUT, `${name}.png`), Buffer.from(data, "base64"))
}

const url = pathToFileURL(HTML).href
await send("Page.navigate", { url })
await sleep(2500)

const unknown = await evaluate("JSON.stringify([...window.__SNAPSHOT_UNKNOWN__ ?? []])")
const pageLog = []

// 逐页切换
for (const [id, label] of PAGES) {
  const changed = await evaluate(`(() => {
    const buttons = [...document.querySelectorAll('[aria-label="主导航"] button')]
    const button = buttons.find((b) => b.textContent.trim() === ${JSON.stringify(label)})
    if (!button) return false
    button.click()
    return true
  })()`)
  if (!changed) { pageLog.push(`${id}: 侧栏找不到「${label}」`); continue }
  await sleep(1400)
  const height = await evaluate("document.documentElement.scrollHeight")
  await shot(`panel-${id}`)
  pageLog.push(`${id}: 已截图（页面高 ${height}）`)
}

// 子页签与关键入口：样式重构最容易漏的就是这些二级界面
async function clickText(label, scope = "document") {
  return evaluate(`(() => {
    const root = ${scope === "document" ? "document" : `document.querySelector(${JSON.stringify(scope)})`}
    if (!root) return false
    const nodes = [...root.querySelectorAll('button, [role="tab"], [role="radio"], a')]
    const node = nodes.find((n) => n.textContent.trim() === ${JSON.stringify(label)})
    if (!node) return false
    // RubberSegment 等自绘分段控件只认 pointerdown/pointerup，不响应 .click()；
    // 两种都发一遍，普通按钮与拖拽式控件都能切换
    const box = node.getBoundingClientRect()
    const opts = { bubbles: true, cancelable: true, clientX: box.x + box.width / 2, clientY: box.y + box.height / 2, pointerId: 1, pointerType: "mouse", isPrimary: true, button: 0, buttons: 1 }
    node.dispatchEvent(new PointerEvent("pointerdown", opts))
    node.dispatchEvent(new PointerEvent("pointerup", { ...opts, buttons: 0 }))
    node.click()
    return true
  })()`)
}

const SUBPAGES = [
  ["providers", "账号管理", "providers-accounts"],
  ["checkin", "批量管理", "checkin-batch"],
  ["usage", "用量分析", "usage-analysis"],
  ["usage", "调用明细", "usage-calls"],
  ["automation", "运行记录", "automation-runs"],
  ["gateway", "路由流水", "gateway-log"],
  ["gateway", "路由分组", "gateway-groups"],
  ["gateway", "模型池", "gateway-pool"],
  ["gateway", "对外访问", "gateway-access"],
  ["gateway", "中间件", "gateway-middleware"],
]
for (const [page, label, name] of SUBPAGES) {
  const pageLabel = PAGES.find(([id]) => id === page)[1]
  await clickText(pageLabel, '[aria-label="主导航"]')
  await sleep(900)
  const ok = await clickText(label)
  if (!ok) { pageLog.push(`${name}: 找不到「${label}」`); continue }
  await sleep(1400)
  await shot(name)
  pageLog.push(`${name}: 已截图`)
}

// 设置各分类
for (const [label, name] of [["外观", "settings-appearance"], ["网络与代理", "settings-network"], ["数据与存储", "settings-data"], ["通知", "settings-notifications"], ["关于与更新", "settings-about"]]) {
  await clickText("设置", '[aria-label="主导航"]')
  await sleep(800)
  await clickText(label, '[aria-label="设置分类"]')
  await sleep(1200)
  await shot(name)
  pageLog.push(`${name}: 已截图`)
}

// 深色主题（外壳按钮）
await evaluate(`(() => {
  const button = document.querySelector('[aria-label="切换到深色"]') ?? document.querySelector('[aria-label="切换到浅色"]')
  button?.click()
})()`)
await sleep(600)
for (const id of ["providers", "usage", "gateway", "settings"]) {
  const label = PAGES.find(([page]) => page === id)[1]
  await evaluate(`(() => {
    const buttons = [...document.querySelectorAll('[aria-label="主导航"] button')]
    buttons.find((b) => b.textContent.trim() === ${JSON.stringify(label)})?.click()
  })()`)
  await sleep(1200)
  await shot(`dark-${id}`)
}

// 灵动岛两种样式
for (const [style, name] of [["classic", "灵动岛"], ["capsule", "灵动岛"]]) {
  await evaluate(`[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === ${JSON.stringify(name)})?.click()`)
  await sleep(800)
  const styleButton = style === "classic" ? "经典" : "胶囊"
  await evaluate(`[...document.querySelectorAll('[aria-label="灵动岛样式"] button')].find((b) => b.textContent.trim() === ${JSON.stringify(styleButton)})?.click()`)
  await sleep(1200)
  await shot(`island-${style}`)
}

console.log(pageLog.join("\n"))
const errors = events.filter((event) => event.level === "error")
const warnings = events.filter((event) => event.level === "warning")
console.log(`\n控制台错误 ${errors.length} 条 / 警告 ${warnings.length} 条`)
for (const event of events.slice(0, 40)) console.log(`[${event.kind}/${event.level}] ${event.text.slice(0, 400)}`)
console.log(`\n未登记命令：${unknown}`)

ws.close()
chrome.kill()
if (!process.argv.includes("--keep")) {
  // Windows 上 Chrome 退出后仍会短暂占用日志文件，重试几次再放弃（清理失败不影响结果）
  for (let i = 0; i < 10; i++) {
    try { await rm(PROFILE, { recursive: true, force: true }); break } catch { await sleep(300) }
  }
}
