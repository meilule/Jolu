/**
 * 动效令牌的单一性检查：`motion/tokens.ts` 与 `theme/motion.css` 必须同值。
 *
 * ## 这条测试为什么值得留
 *
 * 两套运行时（CSS transition 与 Motion）本来就要两份定义，没有共享的可能。
 * 没有这层检查时，改了一边忘了另一边不会有任何症状——直到某个组件的菜单
 * 比另一个慢 40ms，这种问题很难复现也很难归因。
 *
 * 只有这一条同步检查，不覆盖具体数值是否「好看」——那是设计稿的事。
 */
import test from "node:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { dirname, join } from "node:path"

import { DURATION, EASE } from "./tokens.ts"

const here = dirname(fileURLToPath(import.meta.url))
const css = readFileSync(join(here, "..", "theme", "motion.css"), "utf8")

/** 从 CSS 里读一个自定义属性的值（只认 `:root` 那一组，忽略 reduce 覆盖）。 */
function cssVar(name) {
  const rootStart = css.indexOf(":root {")
  const rootEnd = css.indexOf("}", rootStart)
  assert.ok(rootStart >= 0 && rootEnd > rootStart, "theme/motion.css 缺少 :root 块")
  const block = css.slice(rootStart, rootEnd)
  const match = new RegExp(`${name}\\s*:\\s*([^;]+);`).exec(block)
  assert.ok(match, `theme/motion.css 的 :root 缺少 ${name}`)
  return match[1].trim()
}

test("CSS 与 TS 的动效时长同值", () => {
  // CSS 用 ms，TS 用秒；这里换算后比对，避免两处单位不同被误判
  const pairs = [
    ["--motion-feedback", DURATION.feedback],
    ["--motion-pop", DURATION.pop],
    ["--motion-pop-out", DURATION.popOut],
    ["--motion-dialog", DURATION.dialog],
    ["--motion-slide", DURATION.slide],
  ]
  for (const [varName, seconds] of pairs) {
    const raw = cssVar(varName)
    const ms = Number(raw.replace("ms", ""))
    assert.equal(ms, Math.round(seconds * 1000), `${varName} 与 tokens.ts 不一致`)
  }
})

test("CSS 与 TS 的缓动曲线同值", () => {
  const standard = /cubic-bezier\(([^)]+)\)/.exec(cssVar("--motion-ease-standard"))
  assert.ok(standard, "--motion-ease-standard 不是 cubic-bezier")
  const parsed = standard[1].split(",").map((n) => Number(n.trim()))
  assert.deepEqual(parsed, [...EASE.standard], "--motion-ease-standard 与 tokens.ts 不一致")

  const exit = /cubic-bezier\(([^)]+)\)/.exec(cssVar("--motion-ease-exit"))
  assert.ok(exit, "--motion-ease-exit 不是 cubic-bezier")
  assert.deepEqual(exit[1].split(",").map((n) => Number(n.trim())), [...EASE.exit])
})

test("减少动效只在 theme/ 里定义一次", () => {
  // 以前这条媒体查询散在 9 个文件里，改一处漏一处。新架构只允许出现在
  // theme/motion.css；本测试只检查 motion.css 自己确实有，扫描全仓库的
  // 那份检查放在 ESLint / 提交前脚本里更合适（见 docs/specs 前端架构 §6）。
  assert.ok(
    css.includes("prefers-reduced-motion: reduce"),
    "theme/motion.css 应有减少动效的兜底",
  )
})
