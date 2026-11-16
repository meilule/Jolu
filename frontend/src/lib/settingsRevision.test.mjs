import test from "node:test"
import assert from "node:assert/strict"
import { acceptsRevision, nextApplied } from "./settingsRevision.ts"

test("乱序到达的旧快照被丢弃，新快照与同一份快照照常接受", () => {
  assert.equal(acceptsRevision(5, 6), true)
  assert.equal(acceptsRevision(5, 5), true, "同一份快照重复到达无害")
  assert.equal(acceptsRevision(5, 4), false, "后到的旧快照不能盖掉新的")
})

test("没有修订号的快照不在这里判，也不改变已应用的修订号", () => {
  assert.equal(acceptsRevision(5, undefined), true)
  assert.equal(nextApplied(5, undefined), 5)
})

test("已应用修订号只增不减", () => {
  assert.equal(nextApplied(0, 3), 3)
  assert.equal(nextApplied(7, 3), 7)
  assert.equal(nextApplied(7, 9), 9)
})

test("模拟两次推送乱序：先到 7、后到 6，最终停在 7", () => {
  let applied = 0
  let shown = null
  for (const snapshot of [{ revision: 7, theme: "dark" }, { revision: 6, theme: "light" }]) {
    if (!acceptsRevision(applied, snapshot.revision)) continue
    applied = nextApplied(applied, snapshot.revision)
    shown = snapshot.theme
  }
  assert.equal(applied, 7)
  assert.equal(shown, "dark")
})
