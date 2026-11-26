// 从 models.dev 生成网关内置的模型元数据兜底表（backend/src/gateway/models_dev_builtin.json）。
// 用法：node scripts/dev/models-dev-builtin.mjs [已下载的 api.json 路径]
// 不给路径时直接下载 https://models.dev/api.json。只收主要厂商，只留网关用到的字段；
// 表头记下生成日期，表里的数字都来自 models.dev，不手填。
import { readFile, writeFile } from "node:fs/promises"
import { resolve } from "node:path"

const VENDORS = ["anthropic", "openai", "google", "deepseek", "moonshotai", "zhipuai", "zai", "xai", "alibaba", "minimax", "mistral"]
const LEVELS = ["none", "minimal", "low", "medium", "high", "xhigh", "max"]

function parseCost(cost) {
  if (!cost || typeof cost !== "object") return undefined
  const num = (v) => (typeof v === "number" && Number.isFinite(v) && v >= 0 ? v : undefined)
  const input = num(cost.input)
  const output = num(cost.output)
  if (input === undefined && output === undefined) return undefined
  const baseInput = input ?? 0
  const baseOutput = output ?? 0
  const baseRead = num(cost.cache_read) ?? 0
  const baseWrite = num(cost.cache_write)
  const baseWrite1h = num(cost.cache_write_1h)
  const tiers = []
  if (Array.isArray(cost.tiers)) {
    for (const item of cost.tiers) {
      const above = item?.tier?.type === "context" && typeof item?.tier?.size === "number" ? item.tier.size : undefined
      if (!above || above <= 0) continue
      tiers.push({
        above,
        input: num(item.input) ?? baseInput,
        output: num(item.output) ?? baseOutput,
        cache_read: num(item.cache_read) ?? baseRead,
        ...(num(item.cache_write) !== undefined ? { cache_write: num(item.cache_write) } : baseWrite !== undefined ? { cache_write: baseWrite } : {}),
        ...(num(item.cache_write_1h) !== undefined ? { cache_write_1h: num(item.cache_write_1h) } : baseWrite1h !== undefined ? { cache_write_1h: baseWrite1h } : {}),
      })
    }
  }
  if (cost.context_over_200k && typeof cost.context_over_200k === "object" && !tiers.some((t) => t.above === 200000)) {
    const t = cost.context_over_200k
    tiers.push({
      above: 200000,
      input: num(t.input) ?? baseInput,
      output: num(t.output) ?? baseOutput,
      cache_read: num(t.cache_read) ?? baseRead,
      ...(num(t.cache_write) !== undefined ? { cache_write: num(t.cache_write) } : baseWrite !== undefined ? { cache_write: baseWrite } : {}),
      ...(num(t.cache_write_1h) !== undefined ? { cache_write_1h: num(t.cache_write_1h) } : baseWrite1h !== undefined ? { cache_write_1h: baseWrite1h } : {}),
    })
  }
  tiers.sort((a, b) => a.above - b.above)
  return {
    input: baseInput,
    output: baseOutput,
    cache_read: baseRead,
    ...(baseWrite !== undefined ? { cache_write: baseWrite } : {}),
    ...(baseWrite1h !== undefined ? { cache_write_1h: baseWrite1h } : {}),
    ...(tiers.length ? { tiers } : {}),
  }
}

const source = process.argv[2]
const raw = source ? await readFile(source, "utf8") : await (await fetch("https://models.dev/api.json")).text()
const data = JSON.parse(raw)
const models = {}
for (const vendor of VENDORS) {
  for (const [id, meta] of Object.entries(data[vendor]?.models ?? {})) {
    if (models[id]) continue
    const efforts = (meta.reasoning_options ?? []).filter((o) => o.type === "effort").flatMap((o) => o.values ?? []).filter((v) => LEVELS.includes(v))
    const cost = parseCost(meta.cost)
    models[id] = {
      context_window: meta.limit?.context ?? 0,
      max_output: meta.limit?.output ?? 0,
      label: meta.name ?? "",
      description: meta.description ?? "",
      vendor,
      ...(typeof meta.reasoning === "boolean" ? { reasoning: meta.reasoning } : {}),
      ...(efforts.length ? { efforts } : {}),
      ...(meta.modalities?.input?.length ? { input_modalities: meta.modalities.input } : {}),
      ...(typeof meta.tool_call === "boolean" ? { tool_call: meta.tool_call } : {}),
      ...(cost ? { cost } : {}),
    }
  }
}
const out = { generated: new Date().toISOString().slice(0, 10), source: "https://models.dev/api.json", models }
const target = resolve("backend/src/gateway/models_dev_builtin.json")
await writeFile(target, JSON.stringify(out) + "\n", "utf8")
console.log(`写入 ${Object.keys(models).length} 个模型 → ${target}`)
