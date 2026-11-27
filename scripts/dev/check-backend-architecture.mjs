#!/usr/bin/env node
/**
 * 后端 Rust 架构守卫脚本。退出码非零即失败，用于日常开发与提交前校验。
 *
 * 六条硬门禁（对应后端重构计划 P3）：
 * 1. `no-wildcard-crate-import`：全仓生产代码禁止 `use crate::*;`
 * 2. `commands-only-entrypoint`：`backend/src/commands/` 以外严禁出现 `#[tauri::command]`
 * 3. `pure-service-layer`：`backend/src/service/` 生产代码严禁依赖 `tauri::AppHandle` / `State` / `WebviewWindow`
 * 4. `layer-boundary-and-cycles`：基础层/存储底层禁止反向引用上层业务模块，且跨层依赖图禁止逆向环（Tarjan SCC）
 * 5. `max-file-and-fn-size`：单文件生产代码 ≤ 1,000 行，单函数生产代码 ≤ 180 行
 * 6. `clean-src-root`：`backend/src/` 顶层仅允许白名单入口文件与领域目录，禁止重新堆放散装模块
 *
 * 用法（仓库根目录）：
 *     node scripts/dev/check-backend-architecture.mjs
 */

import { readdir, readFile } from "node:fs/promises"
import path from "node:path"
import { pathToFileURL } from "node:url"

export const MAX_PROD_FILE_LINES = 1000
export const MAX_PROD_FN_LINES = 180

export const ALLOWED_TOP_FILES = new Set([
  "main.rs",
  "lib.rs",
  "platforms.rs",
  "perf_bench.rs",
])

export const ALLOWED_TOP_DIRS = new Set([
  "bin",
  "infra",
  "settings",
  "db",
  "app",
  "commands",
  "service",
  "auth",
  "quota",
  "checkin",
  "collectors",
  "usage",
  "pricing",
  "sessions",
  "handoff",
  "automation",
  "skills",
  "library",
  "agents",
  "ops",
  "proxy",
  "gateway",
])

const PURE_FOUNDATION_FILES = new Set([
  "platforms.rs",
  "settings/types.rs",
  "sessions/types.rs",
  "db/conn.rs",
  "db/schema.rs",
  "infra/redact.rs",
  "infra/filelock.rs",
  "infra/proc.rs",
  "infra/stall.rs",
  "infra/steady.rs",
  "infra/paths.rs",
])

const FORBIDDEN_FROM_FOUNDATION = [
  "commands",
  "service",
  "gateway",
  "automation",
  "skills",
  "handoff",
  "collectors",
  "app",
  "proxy",
  "auth",
  "quota",
  "checkin",
  "usage",
]

function isPureTestFile(rel) {
  const base = path.posix.basename(rel)
  return (
    rel === "perf_bench.rs" ||
    base === "tests.rs" ||
    base.startsWith("tests_") ||
    base.endsWith("_tests.rs") ||
    rel.includes("/tests/")
  )
}

/**
 * 扫描单个 Rust 源文件：剥离 `#[cfg(test)]` 块，统计生产代码行数、每个顶层/impl 函数的生产行数，
 * 并返回拼合后的纯生产源码供正则规则检查。
 */
export function analyzeRustSource(source) {
  const lines = source.split(/\r?\n/)
  let prodLines = 0
  const prodLineTexts = []
  const functions = []

  let depth = 0
  let inBlockComment = 0
  let inRawStr = null
  let inStr = false

  let cfgTestPending = false
  let testSkipDepth = null
  let pendingFn = null // { startLine, name, baseDepth }
  let activeFn = null // { startLine, name, baseDepth }

  for (let idx = 0; idx < lines.length; idx++) {
    const lineno = idx + 1
    const line = lines[idx]
    const stripped = line.trim()

    if (testSkipDepth === null && inBlockComment === 0 && inRawStr === null && !inStr) {
      if (
        /^#\[cfg\(test\)\]/.test(stripped) ||
        /^#\[cfg\(all\([^)]*\btest\b[^)]*\)\)\]/.test(stripped)
      ) {
        cfgTestPending = true
      }
    }

    if (
      !cfgTestPending &&
      testSkipDepth === null &&
      activeFn === null &&
      pendingFn === null &&
      inBlockComment === 0 &&
      inRawStr === null &&
      !inStr
    ) {
      const m = line.match(
        /^\s*(?:pub(?:\([^)]+\))?\s+)?(?:async\s+)?(?:const\s+)?fn\s+([a-zA-Z0-9_]+)/,
      )
      if (m && depth <= 1) {
        pendingFn = { startLine: lineno, name: m[1], baseDepth: depth }
      }
    }

    const inProdAtLineStart = testSkipDepth === null && !cfgTestPending

    let i = 0
    const n = line.length
    while (i < n) {
      const ch = line[i]
      if (inBlockComment > 0) {
        if (line.slice(i, i + 2) === "/*") {
          inBlockComment++
          i += 2
          continue
        }
        if (line.slice(i, i + 2) === "*/") {
          inBlockComment--
          i += 2
          continue
        }
        i++
        continue
      }
      if (inRawStr !== null) {
        if (ch === '"' && line.slice(i + 1, i + 1 + inRawStr) === "#".repeat(inRawStr)) {
          i += 1 + inRawStr
          inRawStr = null
          continue
        }
        i++
        continue
      }
      if (inStr) {
        if (ch === "\\") {
          i += 2
          continue
        }
        if (ch === '"') {
          inStr = false
        }
        i++
        continue
      }
      if (line.slice(i, i + 2) === "//") {
        break
      }
      if (line.slice(i, i + 2) === "/*") {
        inBlockComment++
        i += 2
        continue
      }
      if (ch === "r" || ch === "b") {
        const rm = line.slice(i).match(/^b?r(#*)"/)
        if (rm) {
          inRawStr = rm[1].length
          i += rm[0].length
          continue
        }
      }
      if (ch === '"') {
        inStr = true
        i++
        continue
      }
      if (ch === "'") {
        const cm = line.slice(i).match(/^'(?:\\.|[^'\\])'/)
        if (cm) {
          i += cm[0].length
          continue
        }
      }
      if (ch === "{") {
        if (cfgTestPending && testSkipDepth === null) {
          testSkipDepth = depth
          cfgTestPending = false
        }
        if (pendingFn !== null && activeFn === null && depth === pendingFn.baseDepth) {
          activeFn = pendingFn
          pendingFn = null
        }
        depth++
      } else if (ch === "}") {
        depth--
        if (testSkipDepth !== null && depth === testSkipDepth) {
          testSkipDepth = null
        }
        if (activeFn !== null && depth === activeFn.baseDepth) {
          functions.push({
            name: activeFn.name,
            startLine: activeFn.startLine,
            endLine: lineno,
            lines: lineno - activeFn.startLine + 1,
          })
          activeFn = null
        }
      } else if (ch === ";") {
        if (cfgTestPending && depth === 0) {
          cfgTestPending = false
        }
        if (pendingFn !== null && depth === pendingFn.baseDepth) {
          pendingFn = null
        }
      }
      i++
    }

    if (inProdAtLineStart && testSkipDepth === null) {
      prodLines++
      prodLineTexts.push(line)
    }
  }

  return {
    prodLines,
    prodText: prodLineTexts.join("\n"),
    functions,
  }
}

async function collectRsFiles(dir, baseDir = dir, out = []) {
  const entries = await readdir(dir, { withFileTypes: true })
  for (const entry of entries) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      await collectRsFiles(full, baseDir, out)
    } else if (entry.name.endsWith(".rs")) {
      const rel = path.relative(baseDir, full).split(path.sep).join("/")
      out.push({ full, rel })
    }
  }
  return out
}

/** Tarjan 强连通分量检测，找出所有节点数 >= 2 的环。 */
export function findCycles(adj) {
  let index = 0
  const indices = new Map()
  const lowlink = new Map()
  const onStack = new Set()
  const stack = []
  const sccs = []

  function strongConnect(v) {
    indices.set(v, index)
    lowlink.set(v, index)
    index++
    stack.push(v)
    onStack.add(v)

    for (const w of adj.get(v) ?? []) {
      if (!indices.has(w)) {
        strongConnect(w)
        lowlink.set(v, Math.min(lowlink.get(v), lowlink.get(w)))
      } else if (onStack.has(w)) {
        lowlink.set(v, Math.min(lowlink.get(v), indices.get(w)))
      }
    }

    if (lowlink.get(v) === indices.get(v)) {
      const comp = []
      while (true) {
        const w = stack.pop()
        onStack.delete(w)
        comp.push(w)
        if (w === v) break
      }
      if (comp.length >= 2) {
        sccs.push(comp.sort())
      }
    }
  }

  for (const node of adj.keys()) {
    if (!indices.has(node)) {
      strongConnect(node)
    }
  }
  return sccs
}

/** 将文件路径映射为架构分层节点，检查跨层循环依赖。 */
function layerOf(rel) {
  if (rel === "main.rs" || rel === "lib.rs" || rel.startsWith("bin/")) return "entry"
  if (rel.startsWith("commands/")) return "commands"
  if (
    rel.startsWith("app/") &&
    rel !== "app/state.rs" &&
    rel !== "app/events.rs" &&
    rel !== "app/notify.rs" &&
    rel !== "app/toast_link.rs" &&
    rel !== "app/mod.rs"
  ) {
    return "app"
  }
  if (rel.startsWith("service/")) return "service"
  if (rel === "db/conn.rs" || rel === "db/schema.rs") return "db_core"
  if (PURE_FOUNDATION_FILES.has(rel)) return "foundation"
  return "domain"
}

function targetLayerOfCrateSeg(seg, subseg) {
  if (seg === "commands") return "commands"
  if (
    seg === "app" &&
    !["state", "events", "notify", "toast_link", ""].includes(subseg)
  ) {
    return "app"
  }
  if (seg === "service") return "service"
  if (seg === "db" && (subseg === "conn" || subseg === "schema")) return "db_core"
  if (
    seg === "platforms" ||
    ["redact", "filelock", "proc", "stall", "steady"].includes(seg) ||
    (seg === "settings" && subseg === "types") ||
    (seg === "sessions" && subseg === "types") ||
    (seg === "infra" && ["redact", "filelock", "proc", "stall", "steady", "paths"].includes(subseg))
  ) {
    return "foundation"
  }
  return "domain"
}

export async function checkBackendArchitecture(srcDir) {
  const violations = []

  // Rule 6: clean-src-root
  const topEntries = await readdir(srcDir, { withFileTypes: true })
  for (const entry of topEntries) {
    if (entry.isDirectory()) {
      if (!ALLOWED_TOP_DIRS.has(entry.name)) {
        violations.push({
          file: entry.name + "/",
          rule: "clean-src-root",
          message: `backend/src/ 顶层出现未登记目录 "${entry.name}/"；请归入既定领域目录或更新架构白名单`,
        })
      }
    } else if (
      entry.name.endsWith(".rs") ||
      entry.name.endsWith(".cjs") ||
      entry.name.endsWith(".mjs") ||
      entry.name.endsWith(".js")
    ) {
      if (!ALLOWED_TOP_FILES.has(entry.name)) {
        violations.push({
          file: entry.name,
          rule: "clean-src-root",
          message: `backend/src/ 顶层禁止堆放散装文件 "${entry.name}"；请迁入对应领域子目录`,
        })
      }
    }
  }

  const files = await collectRsFiles(srcDir)
  const layerAdj = new Map([
    ["entry", new Set()],
    ["commands", new Set()],
    ["app", new Set()],
    ["service", new Set()],
    ["domain", new Set()],
    ["db_core", new Set()],
    ["foundation", new Set()],
  ])

  let scannedCount = 0
  for (const { full, rel } of files) {
    if (isPureTestFile(rel)) continue
    scannedCount++
    const raw = await readFile(full, "utf8")
    const { prodLines, prodText, functions } = analyzeRustSource(raw)

    const codeNoLineComments = prodText
      .split("\n")
      .filter((l) => !l.trim().startsWith("//"))
      .join("\n")

    // Rule 1: no-wildcard-crate-import
    if (/\buse\s+crate::\*\s*;/.test(codeNoLineComments)) {
      violations.push({
        file: rel,
        rule: "no-wildcard-crate-import",
        message: "禁止使用 `use crate::*;`，请改为显式路径导入",
      })
    }

    // Rule 2: commands-only-entrypoint
    if (!rel.startsWith("commands/") && /#\[tauri::command\b/.test(codeNoLineComments)) {
      violations.push({
        file: rel,
        rule: "commands-only-entrypoint",
        message: "`#[tauri::command]` 只能出现在 `backend/src/commands/` 命令壳层中",
      })
    }

    // Rule 3: pure-service-layer
    if (rel.startsWith("service/")) {
      if (
        /\btauri::(?:AppHandle|State|WebviewWindow)\b/.test(codeNoLineComments) ||
        /\buse\s+tauri\b/.test(codeNoLineComments)
      ) {
        violations.push({
          file: rel,
          rule: "pure-service-layer",
          message: "`service/` 用例编排层必须保持零 Tauri 依赖（禁止引用 AppHandle / State / WebviewWindow）",
        })
      }
    }

    // Rule 4a: pure foundation files cannot depend on upper layers
    if (PURE_FOUNDATION_FILES.has(rel)) {
      for (const forbidden of FORBIDDEN_FROM_FOUNDATION) {
        const re = new RegExp(`\\bcrate::${forbidden}\\b`)
        if (re.test(codeNoLineComments)) {
          violations.push({
            file: rel,
            rule: "layer-boundary-and-cycles",
            message: `底层基础模块禁止反向依赖上层模块 \`crate::${forbidden}\``,
          })
        }
      }
    }

    // Rule 4b: proxy/deps.rs & proxy/observe.rs must stay decoupled from gateway runtime & commands
    if (rel === "proxy/deps.rs" || rel === "proxy/observe.rs") {
      if (/\bcrate::(?:commands|gateway::(?:server|wire|escape|adapter|wiring))\b/.test(codeNoLineComments)) {
        violations.push({
          file: rel,
          rule: "layer-boundary-and-cycles",
          message: `\`${rel}\` 必须与 \`gateway\` 转发实现及 \`commands\` 解耦，防止 proxy <-> gateway 双向循环复发`,
        })
      }
    }

    // Build cross-layer dependency edges for Tarjan SCC check
    const fromLayer = layerOf(rel)
    for (const m of codeNoLineComments.matchAll(/\bcrate::([a-zA-Z0-9_]+)(?:::([a-zA-Z0-9_]+))?/g)) {
      const toLayer = targetLayerOfCrateSeg(m[1], m[2] ?? "")
      if (fromLayer !== toLayer) {
        layerAdj.get(fromLayer)?.add(toLayer)
      }
    }

    // Also forbid any non-entry/non-commands module from importing `crate::commands`!
    if (fromLayer !== "entry" && fromLayer !== "commands") {
      if (/\bcrate::commands\b/.test(codeNoLineComments)) {
        violations.push({
          file: rel,
          rule: "layer-boundary-and-cycles",
          message: `下层模块（${fromLayer}）禁止反向调用命令壳层 \`crate::commands\`；请下沉至 service/ 或通过回调注入`,
        })
      }
    }

    // Rule 5: max-file-and-fn-size
    if (prodLines > MAX_PROD_FILE_LINES) {
      violations.push({
        file: rel,
        rule: "max-file-and-fn-size",
        message: `单文件生产代码 ${prodLines} 行，超过上限 ${MAX_PROD_FILE_LINES} 行`,
      })
    }
    for (const fn of functions) {
      if (fn.lines > MAX_PROD_FN_LINES) {
        violations.push({
          file: `${rel}:${fn.startLine}`,
          rule: "max-file-and-fn-size",
          message: `函数 \`${fn.name}\` 生产代码 ${fn.lines} 行（L${fn.startLine}-L${fn.endLine}），超过上限 ${MAX_PROD_FN_LINES} 行`,
        })
      }
    }
  }

  // Tarjan SCC check across architectural layers
  const layerCycles = findCycles(layerAdj)
  for (const comp of layerCycles) {
    violations.push({
      file: "backend/src/",
      rule: "layer-boundary-and-cycles",
      message: `检测到跨架构层强连通循环依赖（SCC）：[${comp.join(" <-> ")}]`,
    })
  }

  return { scannedCount, violations }
}

const isDirectRun =
  process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href

if (isDirectRun) {
  const ROOT = path.resolve(import.meta.dirname, "../..")
  const SRC = path.join(ROOT, "backend", "src")
  const { scannedCount, violations } = await checkBackendArchitecture(SRC)
  if (violations.length === 0) {
    console.log(
      `后端架构检查通过（扫描 ${scannedCount} 个生产源码文件，单文件 ≤ ${MAX_PROD_FILE_LINES} 行，单函数 ≤ ${MAX_PROD_FN_LINES} 行，0 跨层环）`,
    )
    process.exit(0)
  }
  console.error(`后端架构检查发现 ${violations.length} 处违规：\n`)
  for (const v of violations) {
    console.error(`  ${v.file}`)
    console.error(`    [${v.rule}] ${v.message}\n`)
  }
  process.exit(1)
}
