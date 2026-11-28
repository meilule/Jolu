import { test } from "node:test"
import assert from "node:assert/strict"

import {
  isPortBindError,
  parseGatewayPort,
  portChangeSuccessText,
  sanitizePortInput,
} from "./gatewayPort.ts"

test("sanitizePortInput 只保留前 5 位数字", () => {
  assert.equal(sanitizePortInput("12a73b19"), "12731")
  assert.equal(sanitizePortInput("  :18731 "), "18731")
  assert.equal(sanitizePortInput("abc"), "")
})

test("parseGatewayPort 校验端口范围与格式", () => {
  assert.deepEqual(parseGatewayPort("18731"), { ok: true, port: 18731 })
  assert.deepEqual(parseGatewayPort("1024"), { ok: true, port: 1024 })
  assert.deepEqual(parseGatewayPort("65535"), { ok: true, port: 65535 })
  assert.equal(parseGatewayPort("").ok, false)
  assert.equal(parseGatewayPort("80").ok, false)
  assert.equal(parseGatewayPort("65536").ok, false)
  assert.equal(parseGatewayPort("12a34").ok, false)
})

test("isPortBindError 准确识别端口占用与 Windows 10013 保留错误", () => {
  assert.equal(isPortBindError("启动模型入口失败：端口 12731 被系统保留或禁止监听（Windows 上常见于 WSL / Hyper-V / Docker 动态保留端口区间），请更换一个端口后再开启模型路由"), true)
  assert.equal(isPortBindError("启动模型入口失败：监听 127.0.0.1:12731 失败：以一种访问权限不允许的方式做了一个访问套接字的尝试。 (os error 10013)"), true)
  assert.equal(isPortBindError("端口 12731 已被占用；模型入口必须使用固定端口"), true)
  assert.equal(isPortBindError("数据库不可用"), false)
  assert.equal(isPortBindError("网络超时"), false)
})

test("portChangeSuccessText 区分直接改端口与方案 A 还原后改端口", () => {
  assert.equal(
    portChangeSuccessText({ port: 18731, restored: [], skipped: [], failed: [], stopped: false }),
    "监听端口已改为 18731",
  )
  assert.equal(
    portChangeSuccessText({ port: 18731, restored: ["claude-code", "codex"], skipped: [], failed: [], stopped: true }),
    "已还原 2 个工具并停止中转，监听端口已改为 18731",
  )
})
