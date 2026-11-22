#!/usr/bin/env node
/**
 * 真实播放抓帧：不暂停动画，按墙上时间连续截图，记录每帧的相对时刻。
 * 用来证明「打开文件就能看到动效」，而不是靠定格关键帧。
 *
 * 用法：node scripts/dev/capture-startup-frames.mjs [--dark]
 */
import { spawn } from "node:child_process"
import { existsSync } from "node:fs"
import { mkdir, writeFile, rm, readdir } from "node:fs/promises"
import { resolve, dirname } from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const FILE = resolve(ROOT, "output/startup-preview.html")
const OUT = resolve(ROOT, "output/startup-frames")
const PROFILE = resolve(ROOT, "output/.frames-chrome")
const DARK = process.argv.includes("--dark")

const CHROME = [
  process.env.JOLU_CHROME,
  "C:/Users/林凯东/AppData/Local/ms-playwright/chromium_headless_shell-1247/chrome-headless-shell-win64/chrome-headless-shell.exe",
  "C:/Program Files/Google/Chrome/Application/chrome.exe",
].find((p) => p && existsSync(p))
if (!CHROME) throw new Error("找不到 Chromium")

await rm(OUT, { recursive: true, force: true })
await mkdir(OUT, { recursive: true })
await rm(PROFILE, { recursive: true, force: true })

const port = 9411
const chrome = spawn(CHROME, [
  "--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${PROFILE}`,
  "--allow-file-access-from-files", "--hide-scrollbars", "--window-size=1340,820", "about:blank",
], { stdio: "ignore" })
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

let targets
for (let i = 0; i < 60; i++) { try { targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json(); break } catch { await sleep(200) } }
const ws = new WebSocket(targets.find((t) => t.type === "page").webSocketDebuggerUrl)
let id = 1
const pending = new Map()
ws.addEventListener("message", (e) => {
  const m = JSON.parse(e.data)
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m.result); pending.delete(m.id) }
})
const send = (method, params = {}) => new Promise((r) => { const i = id++; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })) })
await new Promise((r) => ws.addEventListener("open", r, { once: true }))
await send("Page.enable")
await send("Runtime.enable")
await send("Emulation.setDeviceMetricsOverride", { width: 1340, height: 820, deviceScaleFactor: 1, mobile: false })

const query = DARK ? "?theme=dark" : ""
await send("Page.navigate", { url: `file:///${FILE.replaceAll("\\", "/")}${query}` })
// 让页面完成首次布局（字体与图标都是内联的，几十毫秒足够），然后立刻开始抓，
// 不能再多等——多等就会错过 welcome 的起笔
await sleep(90)

const frames = []
const started = Date.now()
// 连续抓 14 帧，覆盖 0–1.9s
for (let i = 0; i < 14; i++) {
  const at = Date.now() - started
  const { data } = await send("Page.captureScreenshot", { format: "png" })
  const name = `f${String(i).padStart(2, "0")}_${String(at).padStart(4, "0")}ms${DARK ? "-dark" : ""}.png`
  await writeFile(resolve(OUT, name), Buffer.from(data, "base64"))
  frames.push({ name, at })
  await sleep(60)
}

// 拼成一张横向 filmstrip，方便一眼看完整个动效
const strip = `<!doctype html><meta charset="utf-8">
<body style="margin:0;background:#1a1b1e;font:11px/1.4 ui-monospace,monospace;color:#ddd">
<div style="padding:10px 14px;font-size:13px">Jolu 启动页 · 真实播放抓帧（${DARK ? "深色" : "浅色"}）</div>
<div style="display:grid;grid-template-columns:repeat(7,1fr);gap:6px;padding:0 14px 14px">
${frames.map((f) => `<div><img src="${f.name}" style="width:100%;display:block;border-radius:5px"><div style="padding-top:3px">+${f.at}ms</div></div>`).join("")}
</div></body>`
await writeFile(resolve(OUT, "strip.html"), strip, "utf8")

await send("Emulation.setDeviceMetricsOverride", { width: 1720, height: 1000, deviceScaleFactor: 1, mobile: false })
await send("Page.navigate", { url: `file:///${resolve(OUT, "strip.html").replaceAll("\\", "/")}` })
await sleep(700)
const { data } = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true })
await writeFile(resolve(ROOT, `output/startup-filmstrip${DARK ? "-dark" : ""}.png`), Buffer.from(data, "base64"))

console.log("帧:", frames.map((f) => `+${f.at}ms`).join(" "))
console.log("filmstrip →", `output/startup-filmstrip${DARK ? "-dark" : ""}.png`)

ws.close(); chrome.kill()
for (let i = 0; i < 10; i++) { try { await rm(PROFILE, { recursive: true, force: true }); break } catch { await sleep(300) } }
void readdir
