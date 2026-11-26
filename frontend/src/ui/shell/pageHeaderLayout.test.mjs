import test from "node:test"
import assert from "node:assert/strict"
import { resolveHeaderLayout, WIDE_TABS_MIN_CONTAINER } from "./pageHeaderLayout.ts"

const CHROME_NORMAL = 278 // 4 icon buttons + window controls
const CHROME_WITH_UPDATE = 338 // 3 icon buttons + v0.1.11 pill + window controls

test("用量统计在 1280 默认窗口（容器 1034px）下，三页签稳定落在标题下方且操作留在顶行右端", () => {
  const overview = resolveHeaderLayout({
    containerWidth: 1034,
    titleWidth: 104,
    tabsWidth: 276,
    actionsWidth: 106, // 预算与价目
    chromeWidth: CHROME_WITH_UPDATE,
    hasTabs: true,
    wideTabs: true,
    hasActions: true,
  })
  const calls = resolveHeaderLayout({
    containerWidth: 1034,
    titleWidth: 104,
    tabsWidth: 276,
    actionsWidth: 212, // 预算与价目 + 导出 CSV
    chromeWidth: CHROME_WITH_UPDATE,
    hasTabs: true,
    wideTabs: true,
    hasActions: true,
  })
  assert.deepEqual(overview, { tabsRow: "below", actionsRow: "inline" })
  assert.deepEqual(calls, { tabsRow: "below", actionsRow: "inline" })
})

test("用量统计在宽屏 / 最大化窗口（>= 1180px）下，页签与两个操作按钮及版本徽章同排不跳动", () => {
  const overview = resolveHeaderLayout({
    containerWidth: WIDE_TABS_MIN_CONTAINER,
    titleWidth: 104,
    tabsWidth: 276,
    actionsWidth: 106,
    chromeWidth: CHROME_WITH_UPDATE,
    hasTabs: true,
    wideTabs: true,
    hasActions: true,
  })
  const calls = resolveHeaderLayout({
    containerWidth: WIDE_TABS_MIN_CONTAINER,
    titleWidth: 104,
    tabsWidth: 276,
    actionsWidth: 212,
    chromeWidth: CHROME_WITH_UPDATE,
    hasTabs: true,
    wideTabs: true,
    hasActions: true,
  })
  assert.deepEqual(overview, { tabsRow: "inline", actionsRow: "inline" })
  assert.deepEqual(calls, { tabsRow: "inline", actionsRow: "inline" })
})

test("即使在 >= 1180px，若缩放或长文案导致单行真实总宽超限，仍自动降到标题下方而不挤出窗口按钮", () => {
  const layout = resolveHeaderLayout({
    containerWidth: 1180,
    titleWidth: 120,
    tabsWidth: 520,
    actionsWidth: 240,
    chromeWidth: CHROME_WITH_UPDATE,
    hasTabs: true,
    wideTabs: true,
    hasActions: true,
  })
  assert.deepEqual(layout, { tabsRow: "below", actionsRow: "inline" })
})

test("最小窗口宽度（900px → 容器 654px）调用明细含更新徽章时，页面操作落到第二行右侧、窗口按钮锁定顶行", () => {
  const layout = resolveHeaderLayout({
    containerWidth: 654,
    titleWidth: 104,
    tabsWidth: 276,
    actionsWidth: 212,
    chromeWidth: CHROME_WITH_UPDATE,
    hasTabs: true,
    wideTabs: true,
    hasActions: true,
  })
  assert.deepEqual(layout, { tabsRow: "below", actionsRow: "row2" })
})

test("极窄宽度下连第二行也放不下页签 + 操作时，页面操作落到第三行而不重叠", () => {
  const layout = resolveHeaderLayout({
    containerWidth: 460,
    titleWidth: 104,
    tabsWidth: 276,
    actionsWidth: 212,
    chromeWidth: CHROME_WITH_UPDATE,
    hasTabs: true,
    wideTabs: true,
    hasActions: true,
  })
  assert.deepEqual(layout, { tabsRow: "below", actionsRow: "row3" })
})

test("无页签但有 3 个页面操作的 Skills 页：默认宽单行，最小窗口 654px 时页面操作下沉到第二行", () => {
  const wide = resolveHeaderLayout({
    containerWidth: 1034,
    titleWidth: 130,
    tabsWidth: 0,
    actionsWidth: 325,
    chromeWidth: CHROME_WITH_UPDATE,
    hasTabs: false,
    wideTabs: false,
    hasActions: true,
  })
  const narrow = resolveHeaderLayout({
    containerWidth: 654,
    titleWidth: 130,
    tabsWidth: 0,
    actionsWidth: 325,
    chromeWidth: CHROME_WITH_UPDATE,
    hasTabs: false,
    wideTabs: false,
    hasActions: true,
  })
  assert.deepEqual(wide, { tabsRow: "inline", actionsRow: "inline" })
  assert.deepEqual(narrow, { tabsRow: "inline", actionsRow: "row2" })
})

test("普通 2 页签页面（供应商 / 自动化）：默认宽保持单行，窄窗口时自动下沉页签", () => {
  const normal = resolveHeaderLayout({
    containerWidth: 1034,
    titleWidth: 78,
    tabsWidth: 180,
    actionsWidth: 98,
    chromeWidth: CHROME_NORMAL,
    hasTabs: true,
    wideTabs: false,
    hasActions: true,
  })
  const narrow = resolveHeaderLayout({
    containerWidth: 654,
    titleWidth: 78,
    tabsWidth: 180,
    actionsWidth: 98,
    chromeWidth: CHROME_WITH_UPDATE,
    hasTabs: true,
    wideTabs: false,
    hasActions: true,
  })
  assert.deepEqual(normal, { tabsRow: "inline", actionsRow: "inline" })
  assert.deepEqual(narrow, { tabsRow: "below", actionsRow: "inline" })
})

test("首帧未测量（containerWidth=0）时给出保守初值，wideTabs 默认下方避免首帧跳动", () => {
  assert.deepEqual(
    resolveHeaderLayout({
      containerWidth: 0,
      titleWidth: 0,
      tabsWidth: 0,
      actionsWidth: 0,
      chromeWidth: 0,
      hasTabs: true,
      wideTabs: true,
      hasActions: true,
    }),
    { tabsRow: "below", actionsRow: "inline" },
  )
})
