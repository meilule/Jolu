import { useCallback, useRef } from "react"

const DEFAULT_ROW_SELECTOR = '[role="menuitemradio"], [role="menuitem"], [role="menuitemcheckbox"]'

export interface GlidePillOptions {
  /** 参与滑动高亮的子项选择器，默认匹配 Radix 菜单项 */
  selector?: string
  /** 滑块底色 CSS 值，默认 var(--jn-well) */
  background?: string
  /** 为 true 时，悬停在当前已激活项（aria-selected/aria-pressed/aria-current）上让滑块隐身，但仍同步坐标以便连续跨过激活项 */
  skipActive?: boolean
}

function isReducedMotion(): boolean {
  if (typeof document !== "undefined" && document.documentElement.classList.contains("reduce-motion")) return true
  if (typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) return true
  return false
}

function isRowActive(row: HTMLElement): boolean {
  return (
    row.getAttribute("aria-selected") === "true" ||
    row.getAttribute("aria-pressed") === "true" ||
    row.getAttribute("aria-current") === "page"
  )
}

export function useGlidePill(options: GlidePillOptions = {}) {
  const selector = options.selector ?? DEFAULT_ROW_SELECTOR
  const background = options.background ?? "var(--jn-well)"
  const skipActive = options.skipActive ?? false
  const isMenu = selector === DEFAULT_ROW_SELECTOR

  const pillRef = useRef<HTMLSpanElement | null>(null)
  const contentRef = useRef<HTMLElement | null>(null)
  const lastRowRef = useRef<HTMLElement | null>(null)
  const cleanupRef = useRef<(() => void) | null>(null)

  const place = useCallback((row: HTMLElement | null, jump: boolean) => {
    const pill = pillRef.current
    if (!pill) return
    if (!row) {
      pill.style.opacity = "0"
      return
    }
    const box = pill.offsetParent as HTMLElement | null
    if (!box) return

    let top = 0
    let left = 0
    let node: HTMLElement | null = row
    while (node && node !== box) {
      top += node.offsetTop
      left += node.offsetLeft
      const parent: HTMLElement | null = node.offsetParent as HTMLElement | null
      if (!parent) break
      if (parent !== box && parent.scrollTop) top -= parent.scrollTop
      if (parent !== box && parent.scrollLeft) left -= parent.scrollLeft
      node = parent
    }

    const instant = jump || isReducedMotion()
    // 必须两个分支都写完整值，赋空串在 CSSOM 中可能残留上一次的 0ms
    pill.style.transitionDuration = instant
      ? "0ms, 0ms, 0ms, 120ms"
      : "200ms, 160ms, 160ms, 120ms"
    pill.style.transform = `translate(${left}px, ${top}px)`
    pill.style.width = `${row.offsetWidth || 0}px`
    pill.style.height = `${row.offsetHeight || 0}px`
    pill.style.borderRadius = getComputedStyle(row).borderRadius

    const hidden =
      row.hasAttribute("data-glide-static") ||
      /\bbg-jn-primary-wash\b/.test(row.className) ||
      (skipActive && isRowActive(row))
    pill.style.opacity = hidden ? "0" : "1"
  }, [skipActive])

  return useCallback((node: HTMLElement | null) => {
    cleanupRef.current?.()
    cleanupRef.current = null

    if (!node) {
      contentRef.current = null
      pillRef.current = null
      return
    }

    contentRef.current = node
    lastRowRef.current = null

    let pill = node.querySelector<HTMLSpanElement>("[data-glide-pill]")
    if (!pill) {
      pill = document.createElement("span")
      pill.setAttribute("data-glide-pill", "")
      pill.setAttribute("aria-hidden", "true")
      pill.style.cssText = [
        "position:absolute",
        "left:0",
        "top:0",
        "pointer-events:none",
        "z-index:0",
        "opacity:0",
        "transition-property:transform, width, height, opacity",
        "transition-timing-function:var(--motion-ease-standard), var(--motion-ease-standard), var(--motion-ease-standard), ease",
      ].join(";")
    }
    pill.style.background = background
    if (pill.parentElement !== node) node.insertBefore(pill, node.firstChild)
    pillRef.current = pill

    node.setAttribute(isMenu ? "data-glide-menu" : "data-glide-container", "")
    if (getComputedStyle(node).position === "static") node.style.position = "relative"

    let raf = requestAnimationFrame(() => {
      raf = 0
      if (lastRowRef.current) return
      const checked = node.querySelector<HTMLElement>(`${selector}[data-state="checked"]`)
        ?? node.querySelector<HTMLElement>(`${selector}[aria-checked="true"]`)
        ?? node.querySelector<HTMLElement>(`${selector}[aria-selected="true"]`)
        ?? node.querySelector<HTMLElement>(`${selector}[aria-current="page"]`)
      const first = node.querySelector<HTMLElement>(selector)
      place(checked ?? first, true)
      if (pill) pill.style.opacity = "0"
    })

    const onOver = (e: Event) => {
      const row = (e.target as HTMLElement)?.closest?.<HTMLElement>(selector)
      if (!row || !node.contains(row)) return
      if (row.hasAttribute("data-disabled") || row.hasAttribute("disabled") || row.getAttribute("aria-disabled") === "true") {
        if (pill) pill.style.opacity = "0"
        return
      }
      const firstEnter = lastRowRef.current === null
      const sameRow = row === lastRowRef.current
      if (sameRow && pill?.style.opacity === "1") return
      lastRowRef.current = row
      place(row, firstEnter)
    }
    node.addEventListener("pointerover", onOver)

    const onLeave = (e: PointerEvent) => {
      const next = e.relatedTarget as Node | null
      if (next && node.contains(next)) return
      lastRowRef.current = null
      if (pill) pill.style.opacity = "0"
    }
    node.addEventListener("pointerleave", onLeave)

    let keyAt = 0
    const onKey = () => { keyAt = performance.now() }
    node.addEventListener("keydown", onKey, true)

    const observer = new MutationObserver(() => {
      if (skipActive && lastRowRef.current) {
        if (isRowActive(lastRowRef.current)) {
          if (pill) pill.style.opacity = "0"
        }
        return
      }
      if (!isMenu || performance.now() - keyAt > 300) return
      const row = node.querySelector<HTMLElement>(`${selector}[data-highlighted]:not([data-disabled])`)
      if (!row) return
      lastRowRef.current = row
      place(row, true)
    })
    observer.observe(node, {
      subtree: true,
      attributes: true,
      attributeFilter: isMenu
        ? ["data-highlighted", "data-state"]
        : ["aria-selected", "aria-pressed", "aria-current"],
    })

    cleanupRef.current = () => {
      if (raf) cancelAnimationFrame(raf)
      observer.disconnect()
      node.removeEventListener("pointerover", onOver)
      node.removeEventListener("pointerleave", onLeave)
      node.removeEventListener("keydown", onKey, true)
      node.removeAttribute(isMenu ? "data-glide-menu" : "data-glide-container")
    }
  }, [background, isMenu, place, selector, skipActive])
}
