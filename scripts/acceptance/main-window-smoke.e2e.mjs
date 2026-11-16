// 主窗口冒烟验收：通过 WebView2 远程调试口驱动隔离运行的开发版应用，依次打开侧栏八个页面。
// 每页检查：没有「读取失败」「扫描失败」「刷新失败」；加载提示在 10 秒内消失且之后不再出现；期间控制台没有 error、页面没有未捕获异常。
// 加载提示有两段：第一次进入某页时先是懒加载占位「正在加载<页名>」（下载页面代码），挂载后才是页面自己的骨架
// 「正在读取…」。两段都算；「加载结束」要求连续 LOADING_STABLE_MS 都看不到任何加载提示——只看「正在读取」
// 时，会在「正在加载」阶段就误判加载已结束，第一次加载稍慢（冷启动的 Vite 现编译页面、机器正忙）就被误报成
// 「再次出现」（2026-11-16 排查，见 docs/specs/2026-11-16-baseline-and-contracts.md §5）。
// 截图保存到 output/acceptance/main-window/<页面>.png。
// 前置：按 README「隔离桌面流程」启动应用，并在能解析 `playwright` 或 `playwright-core` 的目录下运行。
// 用法：node <仓库>/scripts/acceptance/main-window-smoke.e2e.mjs（JOLU_CDP / JOLU_DEV_URL 见 cdp.mjs）
import { mkdirSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { DEV_URL, connect, invoke, isolatedIsland, log, sleep, waitFor } from "./cdp.mjs"

const OUT = resolve(dirname(fileURLToPath(import.meta.url)), "../../output/acceptance/main-window")
const LOADING_TIMEOUT = 10_000
// 连续这么久看不到加载提示才算加载结束（懒加载占位与骨架之间、点击与挂载之间都有短暂空档）
const LOADING_STABLE_MS = 400
// 加载提示：懒加载占位「正在加载<页名>」与页面骨架「正在读取…」
const LOADING_WORDS = ["正在加载", "正在读取"]
const isLoading = (text) => LOADING_WORDS.some((word) => text.includes(word))
// 读取结束后再观察一会儿，接住稍晚到达的失败提示与控制台报错
const SETTLE_MS = 1_500
// 加载超过这么久在结果里标「偏慢」并附 IPC 耗时（不判失败）
const SLOW_MS = 3_000
// 顺序与文案取自 lib/mainNavigation.ts（Jolu.pen 画板 01）
const PAGES = [
  ["island", "灵动岛"],
  ["providers", "供应商"],
  ["checkin", "一键签到"],
  ["usage", "用量统计"],
  ["sessions", "会话线程"],
  ["automation", "自动化"],
  ["skills", "Skills 管理"],
  ["settings", "设置"],
]

// 主窗口：开发地址下、不带 window= 参数（灵动岛 window=island、菜单 window=menu），也不是启动页
const isMain = (p) => p.url().startsWith(DEV_URL) && !p.url().includes("window=") && !p.url().includes("startup")

// 失败提示：页面级「读取失败」，Skills「扫描失败」，用量 / 额度「刷新失败」（含 toast）
const FAILURE_WORDS = ["读取失败", "扫描失败", "刷新失败"]

const { ctx, pages } = await connect()
const results = []
let exitCode = 0
try {
  const { island, database } = await isolatedIsland(pages)

  // 在打开主窗口之前就监听所有页面，覆盖主窗口首次加载时的报错；只记主窗口的
  let errors = []
  const watched = new WeakSet()
  const watch = (page) => {
    if (watched.has(page)) return
    watched.add(page)
    page.on("console", (m) => { if (m.type() === "error" && isMain(page)) errors.push("console: " + m.text().slice(0, 300)) })
    page.on("pageerror", (e) => { if (isMain(page)) errors.push("pageerror: " + e.message.slice(0, 300)) })
  }
  pages().forEach(watch)
  ctx.on("page", watch)

  // 与托盘「打开主窗口」同一路径：显示并聚焦主窗口（窗口不存在时创建）
  await invoke(island, "menu_action", { id: "open_main" })
  const main = await waitFor("主窗口页面", async () => pages().find(isMain), 60000)
  await main.waitForSelector('nav[aria-label="主导航"]', { timeout: 60000 })
  log("主窗口：", main.url())

  // 可选：模拟慢环境（JOLU_SMOKE_LATENCY_MS=毫秒）：拦截页面懒加载的代码（/src/views/）延迟后放行，
  // 稳定复现「第一次进入某页时加载偏慢」——加载提示的判定必须在慢的时候也不误报。CDP 的网络限速对
  // WebView2 不生效；Tauri 的 IPC 走自定义协议，也拦不到，所以只能放慢页面代码这一段
  const latency = Number(process.env.JOLU_SMOKE_LATENCY_MS ?? 0)
  if (latency > 0) {
    await main.route((url) => url.toString().includes("/src/views/"), async (route) => { await sleep(latency); await route.continue() })
    log(`模拟慢环境：页面代码延迟 ${latency}ms`)
  }

  // 记每页期间的 IPC（Tauri 2 在 Windows 上走 http://ipc.localhost/<命令>）：失败时写出慢的和没返回的，
  // 便于区分「后端慢 / 请求没发 / 页面自己的问题」。时间戳经 CDP 回报有几十到上百毫秒误差，只作量级参考
  const ipc = []
  const ipcOf = new Map()
  main.on("request", (request) => {
    try {
      const url = new URL(request.url())
      if (url.hostname !== "ipc.localhost") return
      const cmd = decodeURIComponent(url.pathname.slice(1))
      if (cmd.startsWith("plugin:event|")) return
      const record = { cmd, t0: Date.now(), t1: null }
      ipc.push(record)
      ipcOf.set(request, record)
    } catch {}
  })
  const ipcDone = (request) => { const record = ipcOf.get(request); if (record) record.t1 = Date.now() }
  main.on("requestfinished", ipcDone)
  main.on("requestfailed", ipcDone)
  // 开启慢环境拦截后，WebView2 的 IPC 不再出现在网络事件里，这时如实写「未记录到」
  const ipcSummary = (since) => {
    const rows = ipc.filter((r) => r.t0 >= since).map((r) => r.t1 === null ? `${r.cmd}（未返回）` : `${r.cmd} ${r.t1 - r.t0}ms`)
    return rows.length ? rows : ["未记录到"]
  }

  mkdirSync(OUT, { recursive: true })
  const nav = main.locator('nav[aria-label="主导航"]')
  const bodyText = () => main.evaluate(() => document.body.innerText)
  // 当前还在显示的加载提示：哪一行、是不是骨架屏的无障碍标签
  const loadingLines = () => main.evaluate((words) => {
    const out = []
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
    while (walker.nextNode()) {
      const node = walker.currentNode
      if (!words.some((word) => node.textContent.includes(word))) continue
      const cls = String(node.parentElement?.className ?? "")
      out.push(`${node.textContent.trim().slice(0, 30)}${cls.includes("sr-only") ? "（骨架屏无障碍标签）" : ""}`)
    }
    return out
  }, LOADING_WORDS)
  /** 等加载提示连续 LOADING_STABLE_MS 都不出现；返回最后一次看到提示的时刻（没看到过则为开始时刻） */
  const waitLoaded = async (since) => {
    let lastSeen = since
    for (;;) {
      const now = Date.now()
      if (isLoading(await bodyText())) lastSeen = Date.now()
      else if (now - lastSeen >= LOADING_STABLE_MS) return lastSeen
      if (now - since > LOADING_TIMEOUT) throw new Error("timeout")
      await sleep(100)
    }
  }

  for (const [id, label] of PAGES) {
    const problems = []
    let loadingMs = null
    let slowNote = ""
    try {
      const started = Date.now()
      const button = nav.getByRole("button", { name: label, exact: true })
      await button.click()
      await waitFor(label + " 成为当前页", async () => (await button.getAttribute("aria-current")) === "page", 5000)
      try {
        loadingMs = (await waitLoaded(started)) - started
        // 慢不算失败，但要看得见：附上本页 IPC 耗时，分清是页面代码慢还是后端慢
        if (loadingMs > SLOW_MS) slowNote = `偏慢｜本页 IPC：${ipcSummary(started).join("，")}`
      } catch {
        problems.push("加载提示超过 10 秒未消失：" + (await loadingLines()).join("；") + "｜本页 IPC：" + ipcSummary(started).join("，"))
      }
      await sleep(SETTLE_MS)
      const text = await bodyText()
      for (const line of text.split("\n").filter((l) => FAILURE_WORDS.some((w) => l.includes(w)))) problems.push("出现失败提示：" + line.trim().slice(0, 200))
      // 已经稳定结束后又出现：写清楚是哪一行、挂在什么元素上，以及本页 IPC 的耗时，不必另写诊断
      if (loadingMs !== null && isLoading(text)) {
        problems.push("加载提示在结束后再次出现：" + (await loadingLines()).join("；") + "｜本页 IPC：" + ipcSummary(started).join("，"))
      }
    } catch (error) {
      problems.push(error.message)
    }
    // 第一页同时带上主窗口首次加载期间的报错；记完再清空，留给下一页
    problems.push(...errors)
    errors = []
    await main.screenshot({ path: resolve(OUT, id + ".png") }).catch((error) => problems.push("截图失败：" + error.message))
    const pass = problems.length === 0
    if (!pass) exitCode = 1
    results.push({ page: id, label, pass, loadingMs, problems })
    results.at(-1).slow = slowNote || undefined
    log(pass ? "通过" : "失败", label, loadingMs === null ? "" : `读取 ${loadingMs}ms`, slowNote, pass ? "" : JSON.stringify(problems))
  }
  console.log("RESULT " + JSON.stringify({ pass: exitCode === 0, database, screenshots: OUT, pages: results }))
} catch (error) {
  exitCode = 1
  console.error("FAILED", error.message)
  console.error("pages:", pages().map((p) => p.url()))
}
process.exit(exitCode)
