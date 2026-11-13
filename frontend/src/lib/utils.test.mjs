import assert from "node:assert/strict"
import test from "node:test"
import { cn } from "./utils.ts"

test("语义字号 text-note / text-meta 不会被当成颜色合并掉", () => {
  assert.equal(cn("truncate text-note", "text-jolu-muted"), "truncate text-note text-jolu-muted")
  assert.equal(cn("text-meta", "text-jolu-danger"), "text-meta text-jolu-danger")
  assert.equal(cn("text-meta", "text-[13px]"), "text-[13px]", "与另一个字号冲突时后者覆盖")
})
