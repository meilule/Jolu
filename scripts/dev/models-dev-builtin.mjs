// 从 models.dev 生成网关内置的模型元数据兜底表（backend/src/gateway/models_dev_builtin.json）。
// 用法：node scripts/dev/models-dev-builtin.mjs [已下载的 api.json 路径]
// 不给路径时直接下载 https://models.dev/api.json。只收主要厂商，只留网关用到的字段；
// 表头记下生成日期，表里的数字都来自 models.dev，不手填。
import { readFile, writeFile } from "node:fs/promises"
import { resolve } from "node:path"

const VENDORS = ["anthropic", "openai", "google", "deepseek", "moonshotai", "zhipuai", "zai", "xai", "alibaba", "minimax", "mistral"]
const LEVELS = ["none", "minimal", "low", "medium", "high", "xhigh", "max"]

const source = process.argv[2]
const raw = source ? await readFile(source, "utf8") : await (await fetch("https://models.dev/api.json")).text()
const data = JSON.parse(raw)
const models = {}
for (const vendor of VENDORS) {
  for (const [id, meta] of Object.entries(data[vendor]?.models ?? {})) {
    if (models[id]) continue
    const efforts = (meta.reasoning_options ?? []).filter((o) => o.type === "effort").flatMap((o) => o.values ?? []).filter((v) => LEVELS.includes(v))
    models[id] = {
      context_window: meta.limit?.context ?? 0,
      max_output: meta.limit?.output ?? 0,
      label: meta.name ?? "",
      description: meta.description ?? "",
      ...(typeof meta.reasoning === "boolean" ? { reasoning: meta.reasoning } : {}),
      ...(efforts.length ? { efforts } : {}),
      ...(meta.modalities?.input?.length ? { input_modalities: meta.modalities.input } : {}),
      ...(typeof meta.tool_call === "boolean" ? { tool_call: meta.tool_call } : {}),
    }
  }
}
const out = { generated: new Date().toISOString().slice(0, 10), source: "https://models.dev/api.json", models }
const target = resolve("backend/src/gateway/models_dev_builtin.json")
await writeFile(target, JSON.stringify(out) + "\n", "utf8")
console.log(`写入 ${Object.keys(models).length} 个模型 → ${target}`)
