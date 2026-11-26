import assert from "node:assert/strict"
import test from "node:test"
import { existsSync, readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const here = dirname(fileURLToPath(import.meta.url))
const repo = resolve(here, "../../../..")
const logoSource = readFileSync(resolve(here, "PlatformLogo.tsx"), "utf8")
const catalogSource = readFileSync(resolve(repo, "frontend/src/lib/providerCatalog.ts"), "utf8")
const badgeSource = readFileSync(resolve(repo, "frontend/src/components/agents/AgentBadge.tsx"), "utf8")
const discoverySource = readFileSync(resolve(repo, "backend/src/agent_discovery.rs"), "utf8")

function objectKeys(source, name) {
  const block = source.match(new RegExp(`const ${name}[^=]*=\\s*\\{([\\s\\S]*?)\\n\\}`))?.[1]
  assert.ok(block, `找不到 ${name} 映射`)
  return [...block.matchAll(/^\s*(?:"([^"]+)"|([a-z][\w-]*))\s*:/gm)].map((m) => m[1] ?? m[2])
}

test("供应商目录里的每个条目都有官方图标，不落首字母兜底", () => {
  const providerIds = [...catalogSource.matchAll(/^\s*\{ id: "([^"]+)"/gm)].map((m) => m[1])
  assert.ok(providerIds.length >= 26, `供应商条目解析异常：${providerIds.length}`)

  const covered = new Set([
    "custom",
    ...objectKeys(logoSource, "IMAGE_SOURCES"),
    ...objectKeys(logoSource, "SHAPES"),
    ...objectKeys(logoSource, "PLATFORM_ALIASES"),
  ])
  for (const id of providerIds) {
    assert.ok(covered.has(id), `${id} 没有登记官方图形，会显示首字母兜底`)
  }
})

test("所有图片型厂商标识都存在且是有效 SVG/PNG", () => {
  const imports = [...logoSource.matchAll(/^import (\w+) from "@\/assets\/platforms\/([^"]+)"/gm)]
  assert.ok(imports.length >= 16)
  for (const [, , file] of imports) {
    const path = resolve(repo, "frontend/src/assets/platforms", file)
    assert.ok(existsSync(path), `缺少厂商标识资产 ${file}`)
    if (file.endsWith(".svg")) {
      const svg = readFileSync(path, "utf8")
      assert.match(svg, /<svg[\s>]/, `${file} 不是有效 SVG`)
      assert.match(svg, /<title>[^<]+<\/title>/, `${file} 缺少可访问标题`)
    } else {
      assert.ok(readFileSync(path).length > 100, `${file} 资产为空`)
    }
  }
})

test("后端检测到的 18 家工具都有产品图标或平台图形", () => {
  const specs = discoverySource.match(/const SPECS: \[Spec; 18\] = \[([\s\S]*?)\n\]/)?.[1]
  assert.ok(specs, "找不到 agent_discovery SPECS")
  const agents = [...specs.matchAll(/Spec \{ agent: "([^"]+)"/g)].map((m) => m[1])
  assert.equal(agents.length, 18)

  const productImages = new Set(objectKeys(badgeSource, "AGENT_IMAGES"))
  const platforms = new Set(objectKeys(badgeSource, "PLATFORM_OF"))
  for (const agent of agents) {
    assert.ok(productImages.has(agent) || platforms.has(agent), `${agent} 没有登记图标`)
  }
})

test("OpenCode 不再退化为 OC 文本标", () => {
  assert.doesNotMatch(logoSource, />OC</)
  assert.match(logoSource, /opencodeLogo/)
})

test("Antigravity 使用官方 SVG 图标且 AgentBadge 映射无重复键", () => {
  assert.match(logoSource, /import antigravityLogo from "@\/assets\/platforms\/antigravity\.svg"/)
  assert.ok(objectKeys(logoSource, "IMAGE_SOURCES").includes("antigravity"))
  const platformKeys = objectKeys(badgeSource, "PLATFORM_OF")
  assert.equal(
    platformKeys.filter((k) => k === "antigravity").length,
    1,
    "AgentBadge PLATFORM_OF 中 antigravity 只能出现一次",
  )
  assert.equal(new Set(platformKeys).size, platformKeys.length, "AgentBadge PLATFORM_OF 不得有重复键")
})
