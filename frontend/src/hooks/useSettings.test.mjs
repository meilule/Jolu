import test from "node:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { stripTypeScriptTypes } from "node:module"
import { acceptsRevision, nextApplied } from "../lib/settingsRevision.ts"

// 执行实际 hook 的窗口级存储逻辑；不模拟其 publish/revision/reload 实现。
// React hook 以下的 UI 绑定不参与此测试，API 与事件只以可控 Promise 注入。
function settingsStore(load) {
  const source = readFileSync(new URL("./useSettings.ts", import.meta.url), "utf8")
    .split("export function useSettings()")[0]
    .replace(/^import [^\r\n]*\r?\n/gm, "")
  const code = stripTypeScriptTypes(source, { mode: "strip" })
  let pushed
  // 修订号把关用真实实现（不替身），事件名只需要设置这一个
  const make = new Function("api", "isTauri", "listenEvent", "EVENTS", "acceptsRevision", "nextApplied", `${code}\nreturn { reload: reloadShared, start: ensureSubscribed, snapshot: () => shared };`)
  const store = make({ getSettings: load }, true, async (_event, receive) => { pushed = receive; return () => {} }, { SETTINGS_CHANGED: "settings-changed" }, acceptsRevision, nextApplied)
  return { ...store, push: (value) => pushed(value) }
}

const settle = () => new Promise((resolve) => setImmediate(resolve))

test("较旧的设置读取失败不能覆盖成功推送", async () => {
  const request = Promise.withResolvers()
  const store = settingsStore(() => request.promise)
  store.start()
  await settle()
  store.push({ theme: "dark" })
  request.reject(new Error("old read failure"))
  await settle()
  assert.equal(store.snapshot().settings.theme, "dark")
  assert.equal(store.snapshot().error, null)
  assert.equal(store.snapshot().loading, false)
})

test("设置同步恢复后清除此前读取错误", async () => {
  const store = settingsStore(() => Promise.reject(new Error("offline")))
  store.start()
  await settle()
  assert.match(store.snapshot().error, /offline/)
  store.push({ theme: "light" })
  assert.equal(store.snapshot().error, null)
  assert.equal(store.snapshot().ready, true)
})

test("并发重新读取只接受最新请求，旧请求结束不提前解除加载", async () => {
  const old = Promise.withResolvers()
  const latest = Promise.withResolvers()
  let count = 0
  const store = settingsStore(() => (++count === 1 ? old.promise : latest.promise))
  const first = store.reload()
  const second = store.reload()
  old.resolve({ theme: "old" })
  await first
  assert.equal(store.snapshot().loading, true)
  assert.notEqual(store.snapshot().settings.theme, "old")
  latest.resolve({ theme: "latest" })
  await second
  assert.equal(store.snapshot().settings.theme, "latest")
  assert.equal(store.snapshot().loading, false)
})

test("最新设置先返回时，迟到的旧成功结果仍不能倒退状态", async () => {
  const old = Promise.withResolvers()
  const latest = Promise.withResolvers()
  let count = 0
  const store = settingsStore(() => (++count === 1 ? old.promise : latest.promise))
  const first = store.reload()
  const second = store.reload()
  latest.resolve({ theme: "latest" })
  await second
  old.resolve({ theme: "old" })
  await first
  assert.equal(store.snapshot().settings.theme, "latest")
})

test("推送乱序到达：后到的旧修订快照不能盖掉新的", async () => {
  const store = settingsStore(() => new Promise(() => {}))
  store.start()
  await settle()
  store.push({ theme: "dark", revision: 7 })
  store.push({ theme: "light", revision: 6 })
  assert.equal(store.snapshot().settings.theme, "dark")
  store.push({ theme: "system", revision: 8 })
  assert.equal(store.snapshot().settings.theme, "system", "更新的修订照常接受")
})

test("初次读取晚于推送返回、且修订更旧时，不能倒退", async () => {
  const request = Promise.withResolvers()
  const store = settingsStore(() => request.promise)
  store.start()
  await settle()
  store.push({ theme: "dark", revision: 5 })
  request.resolve({ theme: "light", revision: 4 })
  await settle()
  assert.equal(store.snapshot().settings.theme, "dark")
})
