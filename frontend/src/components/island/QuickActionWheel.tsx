import { useEffect, useRef } from "react"
import { Icon } from "@/icons"
import type { QuickActionDockLayout, QuickActionId } from "@/lib/islandQuickActions"
import { cn } from "@/lib/utils"

/**
 * 长按 bot 快捷动作栏（jolu-new.pen R2-I-06）：
 * - 以胶囊本体居中锚定的一体式深色硬件动作栏，按停靠边水平或竖向向屏内展开；
 * - 内部共享滑块随拖动 / 悬停平滑跟随 5 个安全动作格，不越界也不遮挡胶囊；
 * - 支持「长按拖向动作松手执行」「长按松手后点击执行」「拖离 / 点空白 / Esc 取消」。
 */
export function QuickActionWheel({
  open,
  layout,
  activeId,
  holding,
  onHoverId,
  onSelect,
  onCancel,
}: {
  open: boolean
  layout: QuickActionDockLayout | null
  activeId: QuickActionId | null
  holding: boolean
  onHoverId: (id: QuickActionId | null) => void
  onSelect: (id: QuickActionId) => void
  onCancel: () => void
}) {
  useEffect(() => {
    if (!open) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault()
        onCancel()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [open, onCancel])

  const activeSlot = layout?.slots.find((s) => s.item.id === activeId) ?? null
  const lastSlotRef = useRef(activeSlot)
  const hadActiveRef = useRef(false)
  const jumpGlide = Boolean(activeSlot) && !hadActiveRef.current
  if (activeSlot) {
    lastSlotRef.current = activeSlot
    hadActiveRef.current = true
  } else {
    hadActiveRef.current = false
  }
  useEffect(() => {
    if (!open) {
      lastSlotRef.current = null
      hadActiveRef.current = false
    }
  }, [open])

  if (!open || !layout) return null

  const glideSlot = activeSlot ?? lastSlotRef.current ?? layout.slots[0]
  const vertical = layout.orientation === "vertical"
  const fromOffset =
    layout.side === "above" ? { x: "0px", y: "8px" }
      : layout.side === "right" ? { x: "-8px", y: "0px" }
        : layout.side === "left" ? { x: "8px", y: "0px" }
          : { x: "0px", y: "-8px" }

  const hintText = activeSlot
    ? `${holding ? "松手执行" : "点击执行"} · ${activeSlot.item.label}`
    : holding
      ? "滑向动作松手执行 · 拖离取消"
      : "点选快捷动作 · Esc 或点空白关闭"

  return (
    <>
      {/* 非拖动按住态下，点击空白关闭动作栏 */}
      {!holding && (
        <div
          aria-hidden
          className="cap-dock-backdrop"
          onPointerDown={(e) => {
            e.stopPropagation()
            onCancel()
          }}
        />
      )}

      <div
        role="menu"
        aria-label="快捷动作"
        data-quick-wheel
        data-orientation={layout.orientation}
        data-side={layout.side}
        className={cn("cap-dock", vertical ? "cap-dock-v" : "cap-dock-h")}
        style={{
          left: layout.dockRect.x,
          top: layout.dockRect.y,
          width: layout.dockRect.width,
          height: layout.dockRect.height,
          ["--dock-from-x" as string]: fromOffset.x,
          ["--dock-from-y" as string]: fromOffset.y,
        }}
        onPointerDown={(e) => e.stopPropagation()}
        onDoubleClick={(e) => e.stopPropagation()}
        onPointerLeave={() => { if (!holding) onHoverId(null) }}
      >
        {/* 共享滑块高亮：首次落点瞬移到位，随后随指针在 5 个动作格间平滑滑动，移出时原地淡出 */}
        <span
          aria-hidden
          className="cap-dock-glide"
          style={{
            opacity: activeSlot ? 1 : 0,
            width: glideSlot?.width ?? 62,
            height: glideSlot?.height ?? 56,
            transform: `translate(${glideSlot?.x ?? 6}px, ${glideSlot?.y ?? 6}px)`,
            transitionDuration: jumpGlide ? "0ms, 0ms, 0ms, 120ms" : undefined,
          }}
        />

        {layout.slots.map((slot) => {
          const { item } = slot
          const active = item.id === activeId
          return (
            <button
              key={item.id}
              type="button"
              role="menuitem"
              disabled={item.disabled}
              data-quick-action={item.id}
              data-active={active || undefined}
              style={{
                left: slot.x,
                top: slot.y,
                width: slot.width,
                height: slot.height,
              }}
              className={cn(
                "cap-dock-tile",
                vertical ? "cap-dock-tile-v" : "cap-dock-tile-h",
                active && "cap-dock-tile-on",
              )}
              onPointerEnter={() => { if (!item.disabled && !holding) onHoverId(item.id) }}
              onClick={(e) => {
                e.stopPropagation()
                if (!item.disabled) onSelect(item.id)
              }}
            >
              <span className="cap-dock-icon">
                <Icon name={item.icon} size="xs" />
              </span>
              <span className="cap-dock-label">{item.label}</span>
            </button>
          )
        })}

        <div aria-live="polite" className={cn("cap-dock-footer", activeSlot && "cap-dock-footer-on")}>
          <span aria-hidden className="cap-dock-dot" />
          <span className="truncate">{hintText}</span>
        </div>
      </div>
    </>
  )
}
