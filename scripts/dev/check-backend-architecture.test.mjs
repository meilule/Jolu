import assert from "node:assert/strict"
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  analyzeRustSource,
  checkBackendArchitecture,
  findCycles,
} from "./check-backend-architecture.mjs"

async function withTempSrc(fn) {
  const dir = await mkdtemp(path.join(os.tmpdir(), "jolu-arch-test-"))
  try {
    await fn(dir)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
}

test("analyzeRustSource 忽略 #[cfg(test)] 模块并准确计量生产函数行数", () => {
  const sample = `
pub fn short_fn() -> i32 {
    1 + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn huge_test_ignored() {
        ${"let _ = 1;\n".repeat(250)}
    }
}
`
  const res = analyzeRustSource(sample)
  assert.ok(res.prodLines < 10, `expected < 10 prod lines, got ${res.prodLines}`)
  assert.equal(res.functions.length, 1)
  assert.equal(res.functions[0].name, "short_fn")
  assert.equal(res.functions[0].lines, 3)
})

test("checkBackendArchitecture 捕获全部六类架构违规且对合规工程零误报", async () => {
  await withTempSrc(async (src) => {
    await mkdir(path.join(src, "commands"), { recursive: true })
    await mkdir(path.join(src, "service"), { recursive: true })
    await mkdir(path.join(src, "db"), { recursive: true })

    // 合规最小工程
    await writeFile(path.join(src, "main.rs"), "fn main() {}\n")
    await writeFile(path.join(src, "lib.rs"), "pub mod commands;\npub mod service;\npub mod db;\n")
    await writeFile(path.join(src, "platforms.rs"), "pub const OK: bool = true;\n")
    await writeFile(
      path.join(src, "commands", "demo.rs"),
      "#[tauri::command]\npub fn ping() -> &'static str { crate::service::demo::pong() }\n",
    )
    await writeFile(
      path.join(src, "service", "demo.rs"),
      "pub fn pong() -> &'static str { crate::db::conn::ok() }\n",
    )
    await writeFile(path.join(src, "db", "conn.rs"), "pub fn ok() -> &'static str { \"ok\" }\n")

    const clean = await checkBackendArchitecture(src)
    assert.deepEqual(clean.violations, [])

    // 注入 6 类违规
    await writeFile(path.join(src, "stray_root.rs"), "pub fn bad() {}\n")
    await writeFile(
      path.join(src, "commands", "wild.rs"),
      "use crate::*;\n#[tauri::command]\npub fn w() {}\n",
    )
    await writeFile(
      path.join(src, "service", "leaky.rs"),
      "use tauri::AppHandle;\n#[tauri::command]\npub fn stray_cmd(_a: AppHandle) {}\n",
    )
    await writeFile(
      path.join(src, "db", "schema.rs"),
      "pub fn bad_dep() { let _ = crate::gateway::foo(); }\n",
    )
    const longFn = "pub fn monster() {\n" + "    let _x = 1;\n".repeat(185) + "}\n"
    await writeFile(path.join(src, "service", "big.rs"), longFn)

    const dirty = await checkBackendArchitecture(src)
    const rules = new Set(dirty.violations.map((v) => v.rule))
    assert.ok(rules.has("clean-src-root"))
    assert.ok(rules.has("no-wildcard-crate-import"))
    assert.ok(rules.has("commands-only-entrypoint"))
    assert.ok(rules.has("pure-service-layer"))
    assert.ok(rules.has("layer-boundary-and-cycles"))
    assert.ok(rules.has("max-file-and-fn-size"))
  })
})

test("findCycles 使用 Tarjan SCC 检出双向或多跳环", () => {
  const adj = new Map([
    ["a", new Set(["b"])],
    ["b", new Set(["c"])],
    ["c", new Set(["a"])],
    ["d", new Set(["a"])],
  ])
  const cycles = findCycles(adj)
  assert.deepEqual(cycles, [["a", "b", "c"]])
})
