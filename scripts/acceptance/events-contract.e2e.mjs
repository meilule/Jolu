// 事件契约真实桌面验收（docs/specs/2026-11-16-baseline-and-contracts.md §4–§5 的改动）。
// 通过 WebView2 远程调试口驱动隔离运行的开发版应用：
//   1. 设置快照带修订号：get_settings 有 revision；保存一次（set_theme 写回当前主题）后修订号加一；
//   2. 模型目录变化立即重读：停在 Agent 页时，在隔离库里建一个路由分组（发 gateway-catalog-changed），
//      前端应在 1.5 秒内发出 gateway_catalog 请求——不必等 15 秒轮询；验收后删掉这个分组；
//   3. warm-up 结果命令可用：warmup_last_outcome 在隔离实例里返回 null（本次启动没跑过）。
// 只在隔离目录里写：建 / 删一个名为 jolu-acceptance-events 的路由分组、把主题写回原值。
// 前置与用法同 agents-discovery-cache.e2e.mjs（README「隔离桌面流程」）。
import { mkdirSync, writeFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { DEV_URL, connect, invoke, isolatedIsland, log, sleep, waitFor } from "./cdp.mjs"

const OUT = resolve(dirname(fileURLToPath(import.meta.url)), "../../output/acceptance/events-contract")
const SLUG = "jolu-acceptance-events"
const isMain = (p) => p.url().startsWith(DEV_URL) && !p.url().includes("window=") && !p.url().includes("startup")

const { pages } = await connect()
const problems = []
const report = {}
let exitCode = 0
try {
  const { island, database } = await isolatedIsland(pages)
  log("隔离数据库：", database)
  await invoke(island, "menu_action", { id: "open_main" })
  const main = await waitFor("主窗口页面", async () => pages().find(isMain), 60000)
  await main.waitForSelector('nav[aria-label="主导航"]', { timeout: 60000 })
  mkdirSync(OUT, { recursive: true })

  // 1. 修订号
  const before = await invoke(main, "get_settings")
  await invoke(main, "set_theme", { theme: before.theme })
  const after = await invoke(main, "get_settings")
  report.revision = { before: before.revision, after: after.revision, theme: after.theme }
  log("修订号：", JSON.stringify(report.revision))
  if (typeof before.revision !== "number") problems.push("get_settings 没有 revision")
  else if (after.revision !== before.revision + 1) problems.push(`保存一次后修订号应加一：${before.revision} → ${after.revision}`)
  if (after.theme !== before.theme) problems.push("主题没有写回原值")

  // 2. 目录变化事件 → 前端立即重读
  const catalogRequests = []
  main.on("request", (request) => {
    try {
      const url = new URL(request.url())
      if (url.hostname === "ipc.localhost" && decodeURIComponent(url.pathname.slice(1)) === "gateway_catalog") catalogRequests.push(Date.now())
    } catch {}
  })
  await main.locator('nav[aria-label="主导航"]').getByRole("button", { name: "Agent", exact: true }).click()
  await sleep(2_000) // 等挂载时那一轮读取结束
  try { await invoke(main, "gateway_routing_group_delete", { slug: SLUG }) } catch {}
  const mark = catalogRequests.length
  const startedAt = Date.now()
  await invoke(main, "gateway_routing_group_create", {
    slug: SLUG, name: "验收用分组", strategy: "order",
    members: [{ slug: "zcode", model: "glm-5.3", paused_until_ms: null, weight: null, tier: null }], intent: null,
  })
  await sleep(1_500)
  const reloads = catalogRequests.slice(mark).map((at) => at - startedAt)
  report.catalog = { reloadsWithin1500ms: reloads.length, firstAfterMs: reloads[0] ?? null }
  log("目录变化后的重读：", JSON.stringify(report.catalog))
  if (reloads.length === 0) problems.push("建分组（gateway-catalog-changed）后 1.5 秒内没有重读 gateway_catalog")
  await invoke(main, "gateway_routing_group_delete", { slug: SLUG })

  // 3. warm-up 结果命令
  report.warmup = await invoke(main, "warmup_last_outcome")
  log("warmup_last_outcome：", JSON.stringify(report.warmup))
  if (report.warmup !== null) problems.push("隔离实例本次启动没跑过 warm-up，应返回 null")

  await main.screenshot({ path: resolve(OUT, "agents.png") })
  writeFileSync(resolve(OUT, "report.json"), JSON.stringify({ ...report, problems }, null, 2))
  if (problems.length) {
    exitCode = 1
    problems.forEach((problem) => log("问题：", problem))
  } else {
    log("通过：修订号随保存递增，目录变化立即重读，warm-up 结果命令可用")
  }
} catch (error) {
  exitCode = 1
  log("失败：", error.stack || String(error))
}
process.exit(exitCode)
