// T7 真实桌面验收（docs/plans/2026-11-16-agent-discovery-fix.md）：Agent 页跨挂载记住上次探测结果。
// 通过 WebView2 远程调试口驱动隔离运行的开发版应用。`window.__TAURI_INTERNALS__.invoke` 不可写，
// 改为监听 Tauri 2 在 Windows 上的 IPC 请求（`http://ipc.localhost/<命令>`，请求体带参数）计次、看 force；
// 网络事件的时间戳经 CDP 回报有上百毫秒延迟，耗时改在页面内直接 invoke 计时。步骤：
//   1. 首次进入 Agent 页：记录页面文字随时间的变化；
//   2. 在「供应商」与「Agent」之间来回切 3 次：每次进入的第一帧文字应与稳定后的文字一致（先显示上次结果，
//      不闪空），后台仍调用 agent_discover（不节流）。开发模式的 React StrictMode 会让 effect 执行两遍，
//      所以开发版是 2 次、正式版是 1 次；
//   3. 点「重新检测」：agent_discover 带 force: true；
//   4. 页面内计时：IPC 基线（data_info）、命中缓存、过取戳窗口、force 全量重找。
// 只读：不接入、不断开任何工具，不改设置。探测读的是本机真实工具目录（只读）。
// 前置：按 README「隔离桌面流程」启动应用，并在能解析 `playwright` 或 `playwright-core` 的目录下运行。
// 用法：node <仓库>/scripts/acceptance/agents-discovery-cache.e2e.mjs（JOLU_CDP / JOLU_DEV_URL 见 cdp.mjs）
import { mkdirSync, writeFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { DEV_URL, connect, invoke, isolatedIsland, log, sleep, waitFor } from "./cdp.mjs"

const OUT = resolve(dirname(fileURLToPath(import.meta.url)), "../../output/acceptance/agents-discovery-cache")
const OBSERVE_MS = 2_000
const isMain = (p) => p.url().startsWith(DEV_URL) && !p.url().includes("window=") && !p.url().includes("startup")

const { pages } = await connect()
let exitCode = 0
const problems = []
try {
  const { island, database } = await isolatedIsland(pages)
  log("隔离数据库：", database)
  await invoke(island, "menu_action", { id: "open_main" })
  const main = await waitFor("主窗口页面", async () => pages().find(isMain), 60000)
  await main.waitForSelector('nav[aria-label="主导航"]', { timeout: 60000 })
  // 刷新主窗口页面：清掉前端模块里记住的上次结果，保证第 1 步是真正的首次挂载
  await main.reload()
  await main.waitForSelector('nav[aria-label="主导航"]', { timeout: 60000 })
  mkdirSync(OUT, { recursive: true })

  // 只记录这两个命令的 IPC 请求：命令名、force 参数、耗时（请求发出 → 响应结束）
  const WATCHED = ["agent_discover", "agent_versions"]
  const calls = []
  const byRequest = new Map()
  const commandOf = (url) => { try { const u = new URL(url); return u.hostname === "ipc.localhost" ? decodeURIComponent(u.pathname.slice(1)) : null } catch { return null } }
  main.on("request", (request) => {
    const cmd = commandOf(request.url())
    if (!cmd || !WATCHED.includes(cmd) || request.method() !== "POST") return
    let force
    try { force = JSON.parse(request.postData() ?? "{}").force } catch {}
    const record = { cmd, force, t0: Date.now(), t1: null, ok: null }
    calls.push(record)
    byRequest.set(request, record)
  })
  const finish = (ok) => (request) => {
    const record = byRequest.get(request)
    if (record) { record.t1 = Date.now(); record.ok = ok && (request.failure() == null) }
  }
  main.on("requestfinished", finish(true))
  main.on("requestfailed", finish(false))
  const callsSince = (mark) => calls.slice(mark).map((call) => ({ cmd: call.cmd, force: call.force, ok: call.ok }))

  // 在页面里完成「点导航 → 每帧记录文字」，避免 CDP 往返吃掉第一帧
  const enter = async (label, observe) => {
    const mark = calls.length
    const entry = await enterInPage(label, observe)
    await sleep(200)
    return { ...entry, calls: callsSince(mark) }
  }
  const enterInPage = (label, observe) => main.evaluate(async ([label, observe]) => {
    const nav = document.querySelector('nav[aria-label="主导航"]')
    const button = [...nav.querySelectorAll("button, a")].find((node) => node.textContent.trim() === label || node.getAttribute("aria-label") === label || node.textContent.includes(label))
    if (!button) throw new Error("导航里找不到：" + label)
    const started = performance.now()
    button.click()
    const text = () => {
      const group = document.querySelector('[aria-label="接入状态筛选"]')
      // 筛选计数 + 每张卡的开关状态：都取决于探测结果
      const switches = [...document.querySelectorAll('[role="switch"]')].map((node) => `${node.getAttribute("aria-label")}=${node.getAttribute("aria-checked")}${node.disabled ? "/disabled" : ""}`)
      return group ? `${group.innerText.replace(/\s+/g, " ")} | ${switches.join(",")}` : null
    }
    const frames = []
    let last
    while (performance.now() - started < observe) {
      await new Promise((done) => requestAnimationFrame(done))
      const now = text()
      if (now !== last) {
        frames.push({ t: Math.round(performance.now() - started), text: now })
        last = now
      }
    }
    return { label, frames }
  }, [label, observe])

  const summarize = (entry) => {
    const visible = entry.frames.filter((frame) => frame.text)
    const discover = entry.calls.filter((call) => call.cmd === "agent_discover")
    const versions = entry.calls.filter((call) => call.cmd === "agent_versions")
    return {
      firstVisibleMs: visible[0]?.t ?? null,
      changesAfterVisible: Math.max(0, visible.length - 1),
      firstText: visible[0]?.text ?? null,
      settledText: visible.at(-1)?.text ?? null,
      discover: discover.map((call) => ({ force: call.force ?? false, ok: call.ok })),
      versions: versions.length,
    }
  }
  // 开发模式（StrictMode）每次挂载跑两遍 effect
  const maxDiscoverPerEntry = DEV_URL.startsWith("http://localhost") ? 2 : 1

  // 先站到别的页面，保证第一次进入 Agent 页是真正的首次挂载
  await enter("供应商", 800)
  const report = { first: null, reentries: [], redetect: null }

  report.first = summarize(await enter("Agent", OBSERVE_MS * 2))
  log("首次进入：", JSON.stringify(report.first))
  await main.screenshot({ path: resolve(OUT, "first.png") })

  for (let round = 1; round <= 3; round++) {
    await enter("供应商", 600)
    const entry = summarize(await enter("Agent", OBSERVE_MS))
    report.reentries.push(entry)
    log(`第 ${round} 次切回：`, JSON.stringify(entry))
    if (entry.firstText !== entry.settledText) problems.push(`第 ${round} 次切回：第一帧与稳定后的内容不同（仍有闪变）`)
    if (entry.discover.length < 1 || entry.discover.length > maxDiscoverPerEntry) problems.push(`第 ${round} 次切回：agent_discover 调用 ${entry.discover.length} 次，预期 1–${maxDiscoverPerEntry} 次（不节流的后台刷新）`)
    if (entry.discover.some((call) => call.force)) problems.push(`第 ${round} 次切回：不该带 force`)
  }
  await main.screenshot({ path: resolve(OUT, "reentry.png") })

  // 「重新检测」：带 force，清缓存后全量重找
  const mark = calls.length
  await main.getByRole("button", { name: "重新检测" }).click()
  await waitFor("重新检测完成", async () => main.evaluate(() => !document.querySelector("button .animate-spin")), 20000)
  await sleep(300)
  const redetectCalls = callsSince(mark).filter((call) => call.cmd === "agent_discover")
  report.redetect = redetectCalls
  log("重新检测：", JSON.stringify(redetectCalls))
  if (!redetectCalls.some((call) => call.force === true && call.ok)) problems.push("「重新检测」没有带 force: true 调 agent_discover")

  // 页面内计时（含 IPC 往返）：先 force 清缓存，再测命中缓存、过取戳窗口
  report.timingMs = await main.evaluate(async () => {
    const time = async (cmd, args) => { const t = performance.now(); await window.__TAURI_INTERNALS__.invoke(cmd, args); return Math.round((performance.now() - t) * 10) / 10 }
    const ipcBaseline = await time("data_info", {})
    const force = await time("agent_discover", { force: true })
    const cached = [await time("agent_discover", {}), await time("agent_discover", {}), await time("agent_discover", {})]
    await new Promise((done) => setTimeout(done, 2300))
    const afterWindow = await time("agent_discover", {})
    return { ipcBaseline, force, cached, afterWindow }
  })
  log("页面内计时（ms）：", JSON.stringify(report.timingMs))
  if (Math.max(...report.timingMs.cached, report.timingMs.afterWindow) > 50) problems.push("命中缓存或过窗口后的探测超过 50ms")

  writeFileSync(resolve(OUT, "report.json"), JSON.stringify({ ...report, problems }, null, 2))
  if (problems.length) {
    exitCode = 1
    problems.forEach((problem) => log("问题：", problem))
  } else {
    log("通过：切回 Agent 页第一帧即为探测后的内容，后台各刷新一次，「重新检测」带 force")
  }
} catch (error) {
  exitCode = 1
  log("失败：", error.stack || String(error))
}
process.exit(exitCode)
