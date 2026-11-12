import test from "node:test"
import assert from "node:assert/strict"
import { toQuotaWindows } from "./quotaMap.ts"
import { quotaRowsFor } from "./authQuotaRows.ts"
import { quotaRings } from "./quotaRings.ts"
import { windowRingColor, windowRingGeometry, providerDescriptor } from "./providers/registry.ts"
import { isPlanCovered } from "./planCoverage.ts"
import { dockQuotaView } from "./dockQuotaView.ts"
import { capsuleQuotaChips } from "./capsuleQuotaChips.ts"
import { remainingDisplay } from "./remainingDisplay.ts"
import { KNOWN_PLATFORM_IDS } from "../types.ts"
import catalog from "./platformCatalog.json" with { type: "json" }

/**
 * 4.3「新增描述符无需改共享组件」：一个从未登记的虚构供应商 `acme-gateway`，带着三个没声明过的
 * 窗口 key，走一遍灵动岛与供应商卡片用到的全部纯函数，结果整体快照比对。
 * 守护的语义：不抛错、不补假的 5h / 7d 占位、未知不画环也不当 0、配色与几何落到缺省、文案走兜底。
 */
const PLATFORM = "acme-gateway"
const NOW = Date.parse("2026-11-12T08:00:00Z")
const dto = [
  { key: "acme-hourly", window_name: "每小时请求", used_percent: 35, amount_text: "35 / 100", resets_at: "2026-11-12T09:00:00Z" },
  { key: "acme-daily", window_name: "每日 Token", used_percent: 92, amount_text: null, resets_at: null },
  { key: "acme-credit", window_name: "赠送额度", used_percent: null, amount_text: null, remaining_text: "剩余 12 元", resets_at: null },
]

test("虚构供应商：共享展示逻辑不改代码也能给出完整结果（渲染快照）", () => {
  assert.equal(providerDescriptor(PLATFORM), null, "前提：注册表里没有它")
  const windows = toQuotaWindows(dto, NOW)
  const snapshot = {
    rows: quotaRowsFor(PLATFORM, windows).map((w) => [w.key, w.windowName, w.usedPercent, w.badgeTone]),
    rings: quotaRings(PLATFORM, windows).map((r) => [r.window.key, r.geometry, windowRingColor(r.window.key, PLATFORM)]),
    geometry: windowRingGeometry("acme-hourly", PLATFORM),
    plan: isPlanCovered(windows),
    dock: dockQuotaView({ kind: "auth", connected: true, quotaQueried: true, quotaLoading: false, quotaState: "ok", windows }).type,
    chips: capsuleQuotaChips(windows.map((w) => ({ key: w.key, name: w.windowName, usedPercent: w.usedPercent, amountText: w.usedText }))),
    remaining: remainingDisplay(PLATFORM, "auth", { state: "ok", windows: dto, plan: null }).title,
  }
  assert.deepEqual(snapshot.rows, [
    ["acme-hourly", "每小时请求", 35, "neutral"],
    ["acme-daily", "每日 Token", 92, "neutral"],
    ["acme-credit", "赠送额度", null, "neutral"],
  ], "原样三行，没有凭空多出的 5 小时 / 7 天占位")
  assert.deepEqual(snapshot.rings.map(([key, geometry]) => [key, geometry]), [["acme-daily", "arc"], ["acme-hourly", "arc"]], "未知窗口不画环；最紧的在前")
  assert.ok(snapshot.rings.every(([, , color]) => typeof color === "string" && color.length > 0), "配色落到缺省色")
  assert.equal(snapshot.geometry, "arc")
  assert.equal(snapshot.plan, false, "没声明成套餐窗口，不按 5h / 7d 套餐口径画停靠条")
  assert.equal(snapshot.dock, "unlimited")
  assert.equal(snapshot.chips.chips.length, 1, "三个窗口折成最紧的一个 + N")
  assert.equal(snapshot.chips.more, 2)
  assert.equal(snapshot.remaining, "剩余额度", "文案走兜底")
})

test("已知平台常量与前后端共用的平台目录一致", () => {
  assert.deepEqual([...KNOWN_PLATFORM_IDS].sort(), catalog.map((item) => item.id).sort())
})
