/**
 * 长按 bot 快捷动作栏（jolu-new.pen R2-I-06）：纯几何布局与命中判定。
 *
 * 五个动作均为安全、可逆的主窗口既有操作：全部签到、刷新额度、停止/开启中转、打开会话、建自动化。
 * 危险操作（删除账号、断开接入等）绝不放入这里。
 */
import type { IconName } from "@/icons"
import type { DockEdge } from "@/types"

export type QuickActionId = "checkin" | "refresh" | "toggle_relay" | "open_session" | "new_automation"

export interface QuickActionItem {
  id: QuickActionId
  label: string
  icon: IconName
  disabled?: boolean
}

export interface StageRect {
  x: number
  y: number
  width: number
  height: number
}

export interface StageSize {
  width: number
  height: number
}

export interface QuickActionSlot {
  item: QuickActionItem
  index: number
  /** 相对动作栏容器左上角的偏移（逻辑像素） */
  x: number
  y: number
  width: number
  height: number
  /** 相对舞台容器左上角的矩形 */
  stageRect: StageRect
}

export type QuickDockSide = "below" | "above" | "right" | "left"

export interface QuickActionDockLayout {
  orientation: "horizontal" | "vertical"
  side: QuickDockSide
  dockRect: StageRect
  itemsHeight: number
  slots: QuickActionSlot[]
}

/** 长按触发时长（ms）：足够短以显灵动，同时避免与单击戳一戳或直接拖动冲突 */
export const LONG_PRESS_MS = 380
/** 长按蓄力期间允许的手指/鼠标微动容差（px） */
export const LONG_PRESS_TOLERANCE = 6
/** 动作栏与胶囊之间的呼吸间距（px） */
export const QUICK_DOCK_GAP = 10
/** 动作栏距舞台四周的安全留白（px） */
export const QUICK_DOCK_STAGE_PAD = 12

const TRAY_PAD = 6
const H_TILE_W = 62
const H_TILE_H = 56
const H_TILE_GAP = 4
const V_ROW_W = 158
const V_ROW_H = 36
const V_ROW_GAP = 3
const FOOTER_H = 26

export function buildQuickActions(opts: {
  relayRunning: boolean
  hasSession: boolean
  canToggleRelay: boolean
}): QuickActionItem[] {
  return [
    { id: "checkin", label: "全部签到", icon: "navCheckin" },
    { id: "refresh", label: "刷新额度", icon: "refresh" },
    {
      id: "toggle_relay",
      label: opts.relayRunning ? "停止中转" : "开启中转",
      icon: "power",
      disabled: !opts.canToggleRelay,
    },
    { id: "open_session", label: "打开会话", icon: "navSessions" },
    { id: "new_automation", label: "建自动化", icon: "bot" },
  ]
}

function clamp(v: number, min: number, max: number): number {
  if (max < min) return min
  return Math.max(min, Math.min(max, v))
}

/**
 * 以胶囊本体为中心对称排布快捷动作栏：
 * - 贴顶 / 自由：居中落在胶囊正下方（自由态贴近舞台底边时自动翻到上方）；
 * - 贴底：居中落在胶囊正上方；
 * - 贴左：垂直居中落在胶囊正右侧（竖排）；
 * - 贴右：垂直居中落在胶囊正左侧（竖排）。
 * 所有坐标钳在舞台安全留白内，不越界也不遮挡胶囊。
 */
export function computeQuickActionDock(
  items: readonly QuickActionItem[],
  dockEdge: DockEdge | null,
  capsuleRect: StageRect,
  stageSize: StageSize,
): QuickActionDockLayout {
  const count = items.length
  const capCenterX = capsuleRect.x + capsuleRect.width / 2
  const capCenterY = capsuleRect.y + capsuleRect.height / 2
  const capBottom = capsuleRect.y + capsuleRect.height
  const capRight = capsuleRect.x + capsuleRect.width

  const vWidth = TRAY_PAD * 2 + V_ROW_W
  const preferVertical = dockEdge === "left" || dockEdge === "right"
  // 贴左 / 贴右探出为宽胶囊时，侧边余宽不足以放下竖排栏，自动切换为紧贴胶囊下方的横排栏以避免遮挡胶囊尾部
  const verticalFits =
    dockEdge === "left"
      ? capRight + QUICK_DOCK_GAP + vWidth + QUICK_DOCK_STAGE_PAD <= stageSize.width
      : dockEdge === "right"
        ? capsuleRect.x - QUICK_DOCK_GAP - vWidth >= QUICK_DOCK_STAGE_PAD
        : false
  const vertical = preferVertical && verticalFits
  const orientation = vertical ? "vertical" : "horizontal"

  const innerW = vertical
    ? V_ROW_W
    : count > 0 ? count * H_TILE_W + (count - 1) * H_TILE_GAP : H_TILE_W
  const itemsHeight = vertical
    ? (count > 0 ? count * V_ROW_H + (count - 1) * V_ROW_GAP : V_ROW_H)
    : H_TILE_H

  const width = TRAY_PAD * 2 + innerW
  const height = TRAY_PAD + itemsHeight + TRAY_PAD + FOOTER_H

  let side: QuickDockSide
  let rawX: number
  let rawY: number

  if (vertical && dockEdge === "left") {
    side = "right"
    rawX = capRight + QUICK_DOCK_GAP
    rawY = capCenterY - height / 2
  } else if (vertical && dockEdge === "right") {
    side = "left"
    rawX = capsuleRect.x - QUICK_DOCK_GAP - width
    rawY = capCenterY - height / 2
  } else if (dockEdge === "left" || dockEdge === "right") {
    const fitsBelow = capBottom + QUICK_DOCK_GAP + height + QUICK_DOCK_STAGE_PAD <= stageSize.height
    side = fitsBelow ? "below" : "above"
    rawX = dockEdge === "left" ? capsuleRect.x + 8 : capRight - width - 8
    rawY = fitsBelow ? capBottom + QUICK_DOCK_GAP : capsuleRect.y - QUICK_DOCK_GAP - height
  } else if (dockEdge === "bottom") {
    side = "above"
    rawX = capCenterX - width / 2
    rawY = capsuleRect.y - QUICK_DOCK_GAP - height
  } else {
    const fitsBelow = capBottom + QUICK_DOCK_GAP + height + QUICK_DOCK_STAGE_PAD <= stageSize.height
    if (dockEdge === "top" || fitsBelow) {
      side = "below"
      rawX = capCenterX - width / 2
      rawY = capBottom + QUICK_DOCK_GAP
    } else {
      side = "above"
      rawX = capCenterX - width / 2
      rawY = capsuleRect.y - QUICK_DOCK_GAP - height
    }
  }

  const x = Math.round(clamp(rawX, QUICK_DOCK_STAGE_PAD, stageSize.width - width - QUICK_DOCK_STAGE_PAD))
  const y = Math.round(clamp(rawY, QUICK_DOCK_STAGE_PAD, stageSize.height - height - QUICK_DOCK_STAGE_PAD))
  const dockRect: StageRect = { x, y, width, height }

  const slots: QuickActionSlot[] = items.map((item, index) => {
    const sx = vertical ? TRAY_PAD : TRAY_PAD + index * (H_TILE_W + H_TILE_GAP)
    const sy = vertical ? TRAY_PAD + index * (V_ROW_H + V_ROW_GAP) : TRAY_PAD
    const sw = vertical ? V_ROW_W : H_TILE_W
    const sh = vertical ? V_ROW_H : H_TILE_H
    return {
      item,
      index,
      x: sx,
      y: sy,
      width: sw,
      height: sh,
      stageRect: { x: x + sx, y: y + sy, width: sw, height: sh },
    }
  })

  return { orientation, side, dockRect, itemsHeight, slots }
}

/**
 * 判定舞台坐标 `(px, py)` 命中的快捷动作。
 * 落在动作区外（含胶囊区与舞台空白）或禁用项上返回 null。
 */
export function pickQuickActionAtPoint(
  px: number,
  py: number,
  layout: QuickActionDockLayout,
): QuickActionId | null {
  if (!Number.isFinite(px) || !Number.isFinite(py) || layout.slots.length === 0) return null
  const { dockRect, itemsHeight, slots } = layout
  const itemsBottom = dockRect.y + TRAY_PAD * 2 + itemsHeight
  if (px < dockRect.x || px > dockRect.x + dockRect.width || py < dockRect.y || py > itemsBottom) {
    return null
  }

  // 直接落在格内优先命中；落在格间 3~4px 缝隙时平滑吸附最近格，避免滑动闪烁
  let nearest: QuickActionSlot | null = null
  let nearestDist = Infinity
  for (const slot of slots) {
    const r = slot.stageRect
    if (px >= r.x && px <= r.x + r.width && py >= r.y && py <= r.y + r.height) {
      return slot.item.disabled ? null : slot.item.id
    }
    const cx = r.x + r.width / 2
    const cy = r.y + r.height / 2
    const d = Math.hypot(px - cx, py - cy)
    if (d < nearestDist) {
      nearestDist = d
      nearest = slot
    }
  }
  if (!nearest || nearest.item.disabled) return null
  return nearest.item.id
}
