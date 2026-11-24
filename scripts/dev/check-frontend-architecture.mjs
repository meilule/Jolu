#!/usr/bin/env node
/**
 * 新版前端的架构检查。退出码非零即失败，用于提交前。
 *
 * ## 为什么不用 ESLint
 *
 * 前端架构文档 §8 把「是否引入 ESLint」列为待确认。在拍板之前，这里用一段
 * 几十行的扫描把同样的三条规则落地——等真接 ESLint 时，这几条会变成
 * `no-restricted-imports` 的配置，脚本随之删掉。
 *
 * ## 检查范围只覆盖新目录
 *
 * 规则从 `ui/` `icons/` `motion/` `features/` `theme/` 开始生效。旧的
 * `components/` `views/` `hooks/` 是待替换代码（80 个文件各自 import lucide），
 * 把它们纳入检查只会制造一片红，看不出新代码有没有守规矩。旧目录随页面替换
 * 逐页消失，替换完成后这份白名单也要一起删掉。
 *
 * ## 三条规则
 *
 * 1. **图标只从 `@/icons` 拿。** 直接 import `lucide-react` 会让同一个语义
 *    出现两种画法（「刷新」曾有 RefreshCw 和 RotateCw 两版）。
 * 2. **动效只从 `@/motion` 拿。** 页面直接写 `<motion.div>` + 字面量参数，
 *    会让时长与曲线散落在各处；令牌必须同源（见 motion/tokens.test.mjs）。
 * 3. **不写色值字面量。** `#16A862`、`text-[#fff]`、`bg-[#000]` 这类写法规避了
 *    令牌，深色主题下不会跟着变。
 *
 * 用法（仓库根目录）：
 *     node scripts/dev/check-frontend-architecture.mjs
 */

import { readdir, readFile } from "node:fs/promises"
import { existsSync } from "node:fs"
import path from "node:path"

const ROOT = path.resolve(import.meta.dirname, "../..")
const SRC = path.join(ROOT, "frontend", "src")

/** 规则生效的目录（相对 frontend/src）。 */
const ENFORCED_DIRS = ["ui", "icons", "motion", "features", "theme"]

/** 允许出现的地方：注册表本身、令牌生成结果。 */
const ALLOW = new Set([
  "icons/ui.ts", // 语义注册表就是 lucide 的唯一引用点
  "motion/tokens.ts", // 动效令牌的定义处
  "motion/components.tsx", // 动效封装层是 framer-motion 的唯一引用点
])

const RULES = [
  {
    id: "no-direct-icon-import",
    // 页面直引图标库
    test: (text) => /from\s+["']lucide-react["']/.test(text),
    message: '直接 import lucide-react：改用 `import { Icon } from "@/icons"`，语义名见 icons/ui.ts',
  },
  {
    id: "no-direct-motion-import",
    // 页面直引动效库
    test: (text) => /from\s+["'](?:framer-)?motion(?:\/react)?["']/.test(text),
    message: '直接 import 动效库：改用 `import { … } from "@/motion"`，时长与曲线见 motion/tokens.ts',
  },
  {
    id: "no-hex-color-literal",
    // 六位或三位十六进制色值；排除注释行与 CSS 变量定义
    test: (text) => {
      const withoutComments = text
        .split("\n")
        .filter((line) => !line.trim().startsWith("//") && !line.trim().startsWith("*"))
        .join("\n")
      return /#[0-9a-fA-F]{6}\b/.test(withoutComments) || /#[0-9a-fA-F]{3}\b/.test(withoutComments)
    },
    message: "出现十六进制色值：改用 jn-* 令牌（bg-jn-primary / text-jn-text-2…），深色主题才会跟着变",
  },
]

/** 递归收集源码文件（跳过测试与快照）。 */
async function collect(dir, out = []) {
  const entries = await readdir(dir, { withFileTypes: true })
  for (const entry of entries) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      if (entry.name === "node_modules" || entry.name === "snapshot") continue
      await collect(full, out)
    } else if (/\.(ts|tsx)$/.test(entry.name) && !entry.name.endsWith(".test.ts")) {
      out.push(full)
    }
  }
  return out
}

const violations = []
const scanned = []

for (const dirName of ENFORCED_DIRS) {
  const dir = path.join(SRC, dirName)
  if (!existsSync(dir)) continue // 还没建起来的目录不算失败
  for (const file of await collect(dir)) {
    const rel = path.relative(SRC, file).split(path.sep).join("/")
    if (ALLOW.has(rel)) continue
    const text = await readFile(file, "utf8")
    scanned.push(rel)
    for (const rule of RULES) {
      if (rule.test(text)) {
        violations.push({ file: rel, rule: rule.id, message: rule.message })
      }
    }
  }
}

if (violations.length === 0) {
  console.log(`架构检查通过（扫描 ${scanned.length} 个文件，规则生效目录：${ENFORCED_DIRS.join(" / ")}）`)
  process.exit(0)
}

console.error(`架构检查发现 ${violations.length} 处问题：\n`)
for (const v of violations) {
  console.error(`  ${v.file}`)
  console.error(`    [${v.rule}] ${v.message}\n`)
}
process.exit(1)
