import { test } from "node:test"
import assert from "node:assert/strict"

import { netSummary, rowState } from "./wslWiring.ts"

const state = (over = {}) => ({ net: { distro: "Ubuntu", home: "/root", mode: "nat", host_ip: "172.30.224.1" }, net_error: null, rows: [], agents: ["claude-code", "codex", "opencode"], lan_shared: false, ...over })

test("网络模式决定能不能接", () => {
  assert.equal(netSummary(state()).canWire, false, "NAT 且没开局域网共享")
  assert.match(netSummary(state()).text, /mirrored/)
  assert.equal(netSummary(state({ lan_shared: true })).canWire, true)
  assert.match(netSummary(state({ lan_shared: true })).text, /172\.30\.224\.1/)
  assert.equal(netSummary(state({ net: { distro: "U", home: "/root", mode: "mirrored", host_ip: null } })).canWire, true)
  assert.equal(netSummary(state({ net: null, net_error: "超时" })).canWire, false)
  assert.equal(netSummary(state({ lan_shared: true, net: { distro: "U", home: "/root", mode: "nat", host_ip: null } })).canWire, false, "读不到主机地址")
})

test("工具状态：未接入 / 已接入 / 被改走", () => {
  assert.equal(rowState(undefined).wired, false)
  const row = { distro: "Ubuntu", agent: "codex", path: "p", base: "b", client_key: "wsl-ubuntu-codex", intact: true, wired_ms: 1 }
  assert.equal(rowState(row).text, "已接入 · 专用 Key wsl-ubuntu-codex")
  assert.equal(rowState({ ...row, intact: false }).tone, "warn")
})
