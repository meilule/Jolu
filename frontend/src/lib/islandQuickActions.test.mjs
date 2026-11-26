import assert from "node:assert/strict"
import test from "node:test"
import {
  buildQuickActions,
  computeQuickActionDock,
  pickQuickActionAtPoint,
  QUICK_DOCK_STAGE_PAD,
} from "./islandQuickActions.ts"

const STAGE = { width: 520, height: 640 }

test("R2-I-06 快捷动作固定为 5 项安全操作，且中转开关文案随状态切换", () => {
  const on = buildQuickActions({ relayRunning: true, hasSession: true, canToggleRelay: true })
  assert.equal(on.length, 5)
  assert.equal(on.find((a) => a.id === "toggle_relay")?.label, "停止中转")

  const off = buildQuickActions({ relayRunning: false, hasSession: false, canToggleRelay: true })
  assert.equal(off.find((a) => a.id === "toggle_relay")?.label, "开启中转")
})

test("四个停靠边动作栏始终相对胶囊居中展开且不越界也不遮挡胶囊", () => {
  const items = buildQuickActions({ relayRunning: true, hasSession: true, canToggleRelay: true })

  const topCap = { x: 160, y: 0, width: 200, height: 42 }
  const topDock = computeQuickActionDock(items, "top", topCap, STAGE)
  assert.equal(topDock.orientation, "horizontal")
  assert.equal(topDock.side, "below")
  assert.ok(topDock.dockRect.y > topCap.y + topCap.height, "贴顶应落在胶囊下方")
  assert.equal(
    Math.round(topDock.dockRect.x + topDock.dockRect.width / 2),
    Math.round(topCap.x + topCap.width / 2),
    "贴顶应与胶囊水平居中对齐",
  )

  const bottomCap = { x: 160, y: STAGE.height - 42, width: 200, height: 42 }
  const bottomDock = computeQuickActionDock(items, "bottom", bottomCap, STAGE)
  assert.equal(bottomDock.orientation, "horizontal")
  assert.equal(bottomDock.side, "above")
  assert.ok(bottomDock.dockRect.y + bottomDock.dockRect.height < bottomCap.y, "贴底应落在胶囊上方")

  const leftCap = { x: 0, y: 290, width: 34, height: 58 }
  const leftDock = computeQuickActionDock(items, "left", leftCap, STAGE)
  assert.equal(leftDock.orientation, "vertical")
  assert.equal(leftDock.side, "right")
  assert.ok(leftDock.dockRect.x > leftCap.x + leftCap.width, "贴左应落在胶囊右侧")

  const rightCap = { x: STAGE.width - 34, y: 290, width: 34, height: 58 }
  const rightDock = computeQuickActionDock(items, "right", rightCap, STAGE)
  assert.equal(rightDock.orientation, "vertical")
  assert.equal(rightDock.side, "left")
  assert.ok(rightDock.dockRect.x + rightDock.dockRect.width < rightCap.x, "贴右应落在胶囊左侧")

  const leftPeekCap = { x: 0, y: 300, width: 355, height: 40 }
  const leftPeekDock = computeQuickActionDock(items, "left", leftPeekCap, STAGE)
  assert.equal(leftPeekDock.orientation, "horizontal")
  assert.equal(leftPeekDock.side, "below")
  assert.ok(leftPeekDock.dockRect.y >= leftPeekCap.y + leftPeekCap.height, "贴左宽胶囊探出时动作栏应在下方不遮挡")

  const rightPeekCap = { x: STAGE.width - 355, y: 300, width: 355, height: 40 }
  const rightPeekDock = computeQuickActionDock(items, "right", rightPeekCap, STAGE)
  assert.equal(rightPeekDock.orientation, "horizontal")
  assert.equal(rightPeekDock.side, "below")
  assert.ok(rightPeekDock.dockRect.y >= rightPeekCap.y + rightPeekCap.height, "贴右宽胶囊探出时动作栏应在下方不遮挡")

  for (const layout of [topDock, bottomDock, leftDock, rightDock, leftPeekDock, rightPeekDock]) {
    assert.ok(layout.dockRect.x >= QUICK_DOCK_STAGE_PAD)
    assert.ok(layout.dockRect.y >= QUICK_DOCK_STAGE_PAD)
    assert.ok(layout.dockRect.x + layout.dockRect.width <= STAGE.width - QUICK_DOCK_STAGE_PAD)
    assert.ok(layout.dockRect.y + layout.dockRect.height <= STAGE.height - QUICK_DOCK_STAGE_PAD)
  }
})

test("5 个动作格互不重叠，落在格内精确命中、落在动作栏外返回 null", () => {
  const items = buildQuickActions({ relayRunning: true, hasSession: true, canToggleRelay: true })
  const cap = { x: 160, y: 24, width: 200, height: 42 }
  const layout = computeQuickActionDock(items, null, cap, STAGE)

  for (let i = 0; i < layout.slots.length; i++) {
    const a = layout.slots[i].stageRect
    assert.equal(pickQuickActionAtPoint(a.x + a.width / 2, a.y + a.height / 2, layout), layout.slots[i].item.id)
    for (let j = i + 1; j < layout.slots.length; j++) {
      const b = layout.slots[j].stageRect
      const overlapX = a.x < b.x + b.width && a.x + a.width > b.x
      const overlapY = a.y < b.y + b.height && a.y + a.height > b.y
      assert.ok(!(overlapX && overlapY), `slot ${i} and ${j} should not overlap`)
    }
  }

  // 胶囊中心与动作栏外部返回 null
  assert.equal(pickQuickActionAtPoint(cap.x + cap.width / 2, cap.y + cap.height / 2, layout), null)
  assert.equal(pickQuickActionAtPoint(layout.dockRect.x - 10, layout.dockRect.y + 10, layout), null)
  assert.equal(pickQuickActionAtPoint(layout.dockRect.x + 20, layout.dockRect.y + layout.dockRect.height + 10, layout), null)
})

test("禁用项不会命中", () => {
  const items = buildQuickActions({ relayRunning: false, hasSession: false, canToggleRelay: false })
  const layout = computeQuickActionDock(items, "top", { x: 160, y: 0, width: 200, height: 42 }, STAGE)
  const relaySlot = layout.slots.find((s) => s.item.id === "toggle_relay")
  assert.ok(relaySlot)
  assert.equal(
    pickQuickActionAtPoint(
      relaySlot.stageRect.x + relaySlot.stageRect.width / 2,
      relaySlot.stageRect.y + relaySlot.stageRect.height / 2,
      layout,
    ),
    null,
  )
})
