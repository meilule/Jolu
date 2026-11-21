// 只读真机验收：Agent 页工具图标落地 + Kimi 详情的「接入前写入预览」。
// 不点任何工具的接入 / 断开开关，不修改任何工具配置；写入预览走生产路径
// （gateway_wiring_preview → plan_wire）但只算不写。截图在 output/acceptance/agent-icons/。
// 连接的是本机 dev 实例（JOLU_CDP 指到它开放的调试端口），不是隔离实例。
import { mkdirSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { connect, DEV_URL, log, sleep, waitFor } from "./cdp.mjs"

// 管道下 node 的 console 是块缓冲，卡住时看不到进度；关键步骤用 writeSync 直写 stderr 不缓冲

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const OUT = join(REPO, "output", "acceptance", "agent-icons")
mkdirSync(OUT, { recursive: true })
const problems = []

const { pages } = await connect()
process.stderr.write("[trace] CDP 已连接\n")
const main = await waitFor("主面板页面", () => pages().find((p) => {
  const url = p.url()
  return url.startsWith(DEV_URL) && !url.includes("window=") && !p.isClosed()
}))
await main.waitForSelector("#root > *", { timeout: 60000 })
process.stderr.write("[trace] 主面板已连接\n")
log("主面板已连接")

// 进 Agent 页：侧栏按钮文本就是「Agent」
await main.getByText("Agent", { exact: true }).first().click()
process.stderr.write("[trace] 已点侧栏 Agent\n")
await main.getByRole("heading", { name: "工具", exact: true }).waitFor({ timeout: 30000 })
process.stderr.write("[trace] 栅格标题已出现\n")
await sleep(800)
log("Agent 页已打开")

/**
 * 图标审计：从工具名的叶子节点向上找最近一层含 img / svg 的祖先（即卡片容器）。
 * 折叠条（「暂不能接入」）默认收起，里面的工具不在 DOM，found=false 是正常状态；
 * 断言只覆盖必须出现在主栅格的可接入工具。
 */
const audit = await main.evaluate(() => {
  const names = ["Claude Code", "Codex", "Gemini CLI", "OpenCode", "ZCode", "Kimi CLI", "Continue", "Pencil",
    "DeepSeek Harness", "Copilot CLI", "VS Code", "Qoder", "Qoder CN", "WorkBuddy", "Qwen Code", "Cline",
    "Droid", "Zed", "Crush", "Goose"]
  const norm = (s) => (s ?? "").replace(/\s+/g, " ").trim()
  return names.map((name) => {
    const heading = [...document.querySelectorAll("*")].find((el) =>
      el.children.length === 0 && norm(el.textContent) === name)
    if (!heading) return { name, found: false }
    let node = heading
    for (let i = 0; i < 8 && node; i++) {
      const el = node.querySelector?.("img, svg")
      if (el) {
        return el.tagName === "IMG"
          ? { name, found: true, kind: "img", src: el.getAttribute("src")?.split("/").pop()?.slice(0, 40) }
          : { name, found: true, kind: "svg" }
      }
      node = node.parentElement
    }
    return { name, found: true, kind: "none" }
  })
})
log("图标审计：")
for (const row of audit) log(" ", JSON.stringify(row))
// 已接入工具 + 已检出并装了本机的工具才一定在主栅格；未检出的（如本机没有 gemini 二进制）
// 进折叠条不在 DOM，不算失败。其余（暂不支持 / 折叠条）只记录不强求
const MUST_OFFICIAL = new Set(["Claude Code", "Codex", "OpenCode", "ZCode", "Kimi CLI"])
for (const row of audit) {
  if (!MUST_OFFICIAL.has(row.name)) continue
  if (!row.found) problems.push(`${row.name} 卡片没找到（应出现在主栅格）`)
  else if (row.kind !== "img" && row.kind !== "svg") problems.push(`${row.name} 没有官方图形（kind=${row.kind}）`)
}
await main.screenshot({ path: join(OUT, "agent-grid.png"), fullPage: false })
log("已截图 agent-grid.png")

// Kimi 卡的「详细」进详情页（DOM 内点按钮，避免 Playwright 宽容器匹配歧义）
process.stderr.write("[trace] 开始点 Kimi 详细\n")
await main.evaluate(() => {
  const norm = (s) => (s ?? "").replace(/\s+/g, " ").trim()
  const heading = [...document.querySelectorAll("*")].find((el) => el.children.length === 0 && norm(el.textContent) === "Kimi CLI")
  if (!heading) throw new Error("Kimi CLI 卡不在 DOM")
  let node = heading
  while (node && ![...node.querySelectorAll("button, a")].some((b) => norm(b.textContent) === "详细")) node = node.parentElement
  if (!node) throw new Error("Kimi CLI 卡里没有「详细」")
  ;[...node.querySelectorAll("button, a")].find((b) => norm(b.textContent) === "详细").click()
})
await main.getByText("全部工具", { exact: true }).first().waitFor({ timeout: 30000 })
process.stderr.write("[trace] 已进 Kimi 详情\n")
try {
  await main.getByText("正在生成写入预览…").waitFor({ state: "detached", timeout: 45000 })
} catch {
  problems.push("写入预览 45 秒没生成完")
}
await sleep(600)
process.stderr.write("[trace] 读预览文本\n")
const previewText = await main.evaluate(() => document.body.innerText)
const hasProvider = previewText.includes("providers.jolu") || previewText.includes("base_url")
const hasModels = previewText.includes("jolu/")
const hasDefault = previewText.includes("default_model")
log("Kimi 写入预览：providers/base_url=", hasProvider, "，jolu/ 模型表=", hasModels, "，default_model=", hasDefault)
if (!hasProvider) problems.push("写入预览里没有 providers.jolu / base_url")
if (!hasModels) problems.push("写入预览里没有 jolu/ 开头的模型表")
if (!hasDefault) problems.push("写入预览里没有 default_model")
await main.screenshot({ path: join(OUT, "kimi-preview.png"), fullPage: false })
log("已截图 kimi-preview.png")

if (problems.length) {
  console.error("验收未过：\n- " + problems.join("\n- "))
  process.exit(1)
}
log("全部通过")
