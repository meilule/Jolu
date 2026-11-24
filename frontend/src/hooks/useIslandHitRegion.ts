import { useCallback, useRef } from "react"
import { api, isTauri, type IslandHitRectDto } from "@/lib/api"

/**
 * 向后端 `island_pointer` 上报胶囊可见轮廓矩形（含四角圆角），用于精确鼠标穿透（§3）。
 *
 * - 1px 量化去重：静止时零 IPC 开销；
 * - 动画期间最多约 20Hz（50ms）节流上报，动画停止时立即补报最终精确轮廓，
 *   避免 60Hz 弹簧帧把 IPC 打满。
 */
export function useIslandHitReporter() {
  const lastSignature = useRef("")
  const lastSentAt = useRef(0)
  const trailingTimer = useRef(0)
  const pendingPayload = useRef<{ rects: IslandHitRectDto[]; interactive: boolean } | null>(null)

  const sendNow = useCallback((rects: IslandHitRectDto[], interactive: boolean) => {
    if (!isTauri) return
    const quantized: IslandHitRectDto[] = rects
      .filter((r) => r.width > 1 && r.height > 1)
      .map((r) => ({
        x: Math.round(r.x),
        y: Math.round(r.y),
        width: Math.round(r.width),
        height: Math.round(r.height),
        radii: [
          Math.round(r.radii[0]),
          Math.round(r.radii[1]),
          Math.round(r.radii[2]),
          Math.round(r.radii[3]),
        ],
      }))
    const signature = `${interactive ? 1 : 0}|${quantized.map((r) => `${r.x},${r.y},${r.width},${r.height},${r.radii.join(":")}`).join(";")}`
    if (signature === lastSignature.current) return
    lastSignature.current = signature
    lastSentAt.current = performance.now()
    void api.setIslandHitRegion(quantized, interactive).catch(() => {})
  }, [])

  const report = useCallback((rects: IslandHitRectDto[], interactive = false, immediate = false) => {
    if (!isTauri) return
    const elapsed = performance.now() - lastSentAt.current
    if (immediate || elapsed >= 50) {
      if (trailingTimer.current) {
        window.clearTimeout(trailingTimer.current)
        trailingTimer.current = 0
      }
      pendingPayload.current = null
      sendNow(rects, interactive)
      return
    }
    pendingPayload.current = { rects, interactive }
    if (!trailingTimer.current) {
      trailingTimer.current = window.setTimeout(() => {
        trailingTimer.current = 0
        const next = pendingPayload.current
        pendingPayload.current = null
        if (next) sendNow(next.rects, next.interactive)
      }, 50 - elapsed)
    }
  }, [sendNow])

  const cancel = useCallback(() => {
    if (trailingTimer.current) {
      window.clearTimeout(trailingTimer.current)
      trailingTimer.current = 0
    }
    pendingPayload.current = null
  }, [])

  return { report, cancel }
}
