#!/usr/bin/env node
/**
 * 启动页真实渲染验证：在 vite 开发服务上打开 startup.html，按时间点定格截图。
 *
 * 用法：node scripts/dev/check-startup.mjs [--port 5199] [--dark] [--reduced]
 *
 * 检查项：
 *   - 页面在多个时刻的形态（空白 → hello 书写 → Jolu 书写 → 终态）
 *   - 深色主题
 *   - prefers-reduced-motion 下直接呈现终态
 *   - 控制台错误
 */
import { spawn } from "node:child_process"
import { existsSync } from "node:fs"
import { mkdir, writeFile, rm } from "node:fs/promises"
import { resolve, dirname } from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const arg = (name, fallback) => {
  const index = process.argv.indexOf(name)
  return index >= 0 ? process.argv[index + 1] : fallback
}
const PORT = arg("--port", "5199")
const OUT = resolve(ROOT, "output/startup-shots")
const PROFILE = resolve(ROOT, "output/.startup-chrome")
const DARK = process.argv.includes("--dark")
const REDUCED = process.argv.includes("--reduced")
const SUFFIX = (DARK ? "-dark" : "") + (REDUCED ? "-reduced" : "")

const CHROME = [
  process.env.JOLU_CHROME,
  "C:/Users/林凯东/AppData/Local/ms-playwright/chromium_headless_shell-1247/chrome-headless-shell-win64/chrome-headless-shell.exe",
  "C:/Program Files/Google/Chrome/Application/chrome.exe",
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
].find((p) => p && existsSync(p))
if (!CHROME) throw new Error("找不到 Chromium / Chrome / Edge")

// 时间点对齐 startup.css：hello 60ms 起 820ms；Jolu 760/855/950/1040 各 360ms；图标 1360ms 起
const FRAMES = [0.05, 0.3, 0.55, 0.75, 0.95, 1.05, 1.15, 1.35, 1.6]

await mkdir(OUT, { recursive: true })
await rm(PROFILE, { recursive: true, force: true })

const port = DARK ? 9351 : 9350
const chrome = spawn(CHROME, [
  "--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${PROFILE}`,
  "--hide-scrollbars", "--force-device-scale-factor=1", "--window-size=1300,760", "about:blank",
], { stdio: "ignore" })
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

let targets
for (let i = 0; i < 60; i++) {
  try { targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json(); break } catch { await sleep(200) }
}
const page = targets.find((t) => t.type === "page")
const ws = new WebSocket(page.webSocketDebuggerUrl)
let id = 1
const pending = new Map()
const errors = []
ws.addEventListener("message", (e) => {
  const m = JSON.parse(e.data)
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m.result); pending.delete(m.id); return }
  if (m.method === "Runtime.exceptionThrown") errors.push(m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text)
  if (m.method === "Log.entryAdded" && m.params.entry.level === "error") errors.push(m.params.entry.text)
})
const send = (method, params = {}) => new Promise((r) => { const i = id++; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })) })
await new Promise((r) => ws.addEventListener("open", r, { once: true }))
await send("Runtime.enable"); await send("Log.enable"); await send("Page.enable")
await send("Emulation.setDeviceMetricsOverride", { width: 1300, height: 760, deviceScaleFactor: 1, mobile: false })
if (REDUCED) await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] })

const theme = DARK ? "dark" : "light"
await send("Page.navigate", { url: `http://localhost:${PORT}/startup.html?theme=${theme}` })
await sleep(2200)

// 真实流程里 startup.ts 会在首帧加 .ready；这里等它
const ready = await send("Runtime.evaluate", { expression: "document.documentElement.classList.contains('ready')", returnByValue: true })

const shots = []
for (const t of FRAMES) {
  await send("Runtime.evaluate", {
    expression: `(() => { for (const a of document.getAnimations()) { try { a.currentTime = ${t * 1000}; a.pause() } catch {} } })()`,
  })
  await sleep(120)
  const name = `t${String(t).replace(".", "_")}${SUFFIX}`
  const { data } = await send("Page.captureScreenshot", { format: "png" })
  await writeFile(resolve(OUT, `${name}.png`), Buffer.from(data, "base64"))
  shots.push(name)
}

const box = await send("Runtime.evaluate", {
  expression: `(() => {
    const q = (s) => { const el = document.querySelector(s); if (!el) return null; const r = el.getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height)] }
    return JSON.stringify({ welcome: q('.welcome'), wordmark: q('.wordmark'), icon: q('.icon'), ready: document.documentElement.classList.contains('ready') })
  })()`,
  returnByValue: true,
})

console.log("ready:", ready.result.value, "尺寸:", box.result.value)
console.log("截图:", shots.join(", "))
console.log("控制台错误:", errors.length ? errors.slice(0, 5) : "无")

ws.close(); chrome.kill()
for (let i = 0; i < 10; i++) { try { await rm(PROFILE, { recursive: true, force: true }); break } catch { await sleep(300) } }
