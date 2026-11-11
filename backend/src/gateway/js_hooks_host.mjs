// Jolu 网关 JS 钩子宿主（3.8）：由网关用 Node 或 Bun 启动，一行一个 JSON 消息。
// 收：{id, op: "request" | "response", run, hook: {path, version, timeoutMs, options}, body | reply, ctx}
//     {op: "end", run}
// 回：{id, ok: true, body?} 或 {id, ok: false, code}
// 只回固定的结果码，不回钩子抛出的内容（可能带隐私）。
import { pathToFileURL } from "node:url"
import { createInterface } from "node:readline"

const modules = new Map()
const states = new Map()

async function load(hook) {
  const key = `${hook.path}#${hook.version}`
  if (!modules.has(key)) {
    // 版本号随文件修改时间变，改了钩子文件不用重启网关
    modules.set(key, import(`${pathToFileURL(hook.path).href}?v=${encodeURIComponent(hook.version)}`))
  }
  return modules.get(key)
}

function stateOf(run, path) {
  if (!states.has(run)) states.set(run, new Map())
  const perRun = states.get(run)
  if (!perRun.has(path)) perRun.set(path, {})
  return perRun.get(path)
}

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`)
}

async function call(fn, args, timeoutMs, controller) {
  let timer
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => {
      controller.abort()
      reject(Object.assign(new Error("timeout"), { code: "timeout" }))
    }, timeoutMs)
  })
  try {
    return await Promise.race([Promise.resolve().then(() => fn(...args)), timeout])
  } finally {
    clearTimeout(timer)
  }
}

async function handle(message) {
  if (message.op === "end") {
    states.delete(message.run)
    return
  }
  const { id, hook } = message
  let mod
  try {
    mod = await load(hook)
  } catch {
    send({ id, ok: false, code: "load" })
    return
  }
  const controller = new AbortController()
  const ctx = { ...message.ctx, options: hook.options ?? {}, state: stateOf(message.run, hook.path), signal: controller.signal }
  try {
    if (message.op === "request") {
      if (typeof mod.onRequest !== "function") {
        send({ id, ok: true, body: null })
        return
      }
      const result = await call(mod.onRequest, [message.body, ctx], hook.timeoutMs, controller)
      send({ id, ok: true, body: result === undefined ? null : result })
    } else if (message.op === "response") {
      if (typeof mod.onResponse === "function") {
        await call(mod.onResponse, [message.reply, ctx], hook.timeoutMs, controller)
      }
      send({ id, ok: true })
    }
  } catch (error) {
    send({ id, ok: false, code: error?.code === "timeout" ? "timeout" : "threw" })
  }
}

createInterface({ input: process.stdin }).on("line", (line) => {
  let message
  try {
    message = JSON.parse(line)
  } catch {
    return
  }
  void handle(message)
})
process.stdin.on("end", () => process.exit(0))
