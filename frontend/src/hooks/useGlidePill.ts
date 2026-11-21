import { useCallback, useRef } from "react"

/**
 * 共享滑块高亮（React Bits GlideSelect 的动效）。
 *
 * ── 为什么是 hook 而不是组件 ────────────────────────────────
 * 调用方已有的菜单（MenuFilter / ProviderMenu）样式是按画板调好的，
 * 不该为了加动效去重写 className 或 DOM 结构。这个 hook 只做一件事：
 * 往菜单里插一个绝对定位的滑块，跟着 `[role="menuitemradio"]` 的高亮行移动。
 * 除本文件与一条配套 CSS 外，调用方只多一行 `ref={glide}`。
 *
 * ── 用法 ──────────────────────────────────────────────────
 * ```tsx
 * const glide = useGlidePill()
 * <Menu.Content ref={glide} className={原有类名一字不改}>…</Menu.Content>
 * ```
 *
 * ── 两种运动模式 ──────────────────────────────────────────
 * 鼠标悬停：滑块平移 220ms（一个物体在移动，而不是两块底色交替淡入）。
 * 键盘导航：瞬移 0ms（连按方向键时动画不排队，否则越按越慢）。
 * 指针离开菜单：停在最后停留的行，不回弹。
 * 停在「选中行」上时滑块隐身 —— 那里已有原来的选中底色，叠一层会脏。
 */
/** 菜单里的「一行」。Radix 的 RadioItem 是 menuitemradio，普通 Item 是 menuitem，两种都认 */
const ROW_SELECTOR = '[role="menuitemradio"], [role="menuitem"]'

export function useGlidePill() {
  const pillRef = useRef<HTMLSpanElement | null>(null)
  const contentRef = useRef<HTMLElement | null>(null)
  const lastRowRef = useRef<HTMLElement | null>(null)
  const cleanupRef = useRef<(() => void) | null>(null)

  /**
   * 把滑块摆到某一行的实际矩形上。
   *
   * 参照系必须是**滑块自己的 offsetParent**，不是 Menu.Content：
   * 菜单里可能套着滚动容器（Skills 的供应商菜单就是 `p-3` + 内层 `overflow-y-auto`），
   * 行比 Content 深好几层；再叠上 `.motion-popover` 的 transform（它会成为新的
   * 包含块），用 Content 当原点算出来的位移会整体偏移一整行甚至更多。
   */
  const place = useCallback((row: HTMLElement | null, jump: boolean) => {
    const pill = pillRef.current
    if (!pill) return
    if (!row) {
      pill.style.opacity = "0"
      return
    }
    const box = pill.offsetParent as HTMLElement | null
    if (!box) return
    /**
     * 位置一律用 offsetTop / offsetLeft 沿 offsetParent 链累加，**不用
     * getBoundingClientRect**：菜单开合动画在祖先上挂了 transform: scale()，
     * 视口坐标量到的是缩放后的值；行和盒子若不在同一层 transform 下，
     * 两个缩放值相减不会抵消，位置会整体偏一行（实测偏 26px）。
     * offset* 是布局坐标，完全不受 transform 影响。
     *
     * 唯一需要 getBoundingClientRect 的场景是盒子自己有 CSS 缩放时换算比例，
     * 这里用 offsetWidth 与 rect.width 的比值兜住。
     */
    const scale = box.offsetWidth > 0 ? box.getBoundingClientRect().width / box.offsetWidth : 1
    const s = scale || 1
    // 沿 offsetParent 链累加，直到 box（不含 box 自身的偏移）
    let top = 0
    let left = 0
    let node: HTMLElement | null = row
    while (node && node !== box) {
      top += node.offsetTop
      left += node.offsetLeft
      const parent: HTMLElement | null = node.offsetParent as HTMLElement | null
      if (!parent) break
      // 跨过带滚动的中间容器时补上滚动量（如菜单内的 overflow-y-auto）
      if (parent !== box && parent.scrollTop) top -= parent.scrollTop
      if (parent !== box && parent.scrollLeft) left -= parent.scrollLeft
      node = parent
    }
    /* 位移时长分两档：
         滑动（悬停）220ms —— 一个物体平移过去的连续感；
         瞬移（打开菜单 / 键盘导航）0ms —— 连按方向键时动画不排队。
       ⚠️ 不能写成 `transitionDuration = jump ? "0ms,..." : ""`：
       赋空串在 CSSOM 里不可靠地等价于「移除」，实测会残留上一次的 0ms，
       滑块从此永久瞬移 —— 用户反馈「没有滑动效果」就是这里。
       必须两个分支都写完整值。 */
    pill.style.transitionDuration = jump
      ? "0ms, 0ms, 0ms, 150ms"
      : "220ms, 160ms, 160ms, 150ms"
    // box 的 padding 是滑块的定位起点（absolute 的 0 点是 padding box 的左上角内侧）
    pill.style.transform = `translate(${left / s + box.scrollLeft}px, ${top / s + box.scrollTop}px)`
    pill.style.width = `${(row.offsetWidth || 0) / s}px`
    pill.style.height = `${(row.offsetHeight || 0) / s}px`
    // 圆角跟随行 —— 滑块比行圆一点或方一点，滑动时边缘会露出来
    pill.style.borderRadius = getComputedStyle(row).borderRadius

    /* 选中行自己有底色，滑块让位。两种菜单的选中标记不同：
         RadioItem（MenuFilter）：data-state="checked"
         ProviderMenu 的普通 Item：className 里挂 bg-jolu-selected
       不能用「计算背景色」判断 —— 高亮行的底色会被 CSS 清成 transparent，循环依赖。
       ⚠️ 曾经还加过「行尾有 svg 就算选中」，但 Skills 的供应商菜单每行都渲染了
       PlatformLogo（SVG），每一行都会被误判成选中行、滑块永远隐身。
       只认属性与类名，不猜结构。 */
    const isChecked = row.getAttribute("data-state") === "checked"
      || /bg-jolu-selected/.test(row.className)
    pill.style.opacity = isChecked ? "0" : "1"
  }, [])

  /** Radix 每次打开都会重建 Content，所以用回调 ref 在挂载时接管 */
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

    /* 注入滑块。放在第一个子节点：行是 position:relative，DOM 里排在滑块之后，
       天然画在它上面，不需要给行加 z-index。 */
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
        // 滑块必须沉到行**下面**：它是一层背景。
        // 之前没写 z-index（auto），而 DOM 里它排在所有行之前 ——
        // 同层叠上下文里「后出现的元素在上」，行（尤其带背景的）会盖住滑块，
        // 表现是滑动时灰条被行割裂、甚至文字被吃掉一截。
        "z-index:0",
        // 与行自己的 data-highlighted 底色一致：菜单是浅底时它才看得见。
        // （之前用 --jolu-side，浅色主题是 #fafafa，与 #ffffff 菜单几乎无差别）
        "background:var(--jolu-stage)",
        "opacity:0",
        // 只写 transition-property / timing-function，时长全部由 place() 每次显式写，
        // 避免「简写 + duration 覆盖」两套来源互相打架
        "transition-property:transform, width, height, opacity",
        "transition-timing-function:var(--motion-ease), var(--motion-ease), var(--motion-ease), ease",
      ].join(";")
    }
    if (pill.parentElement !== node) node.insertBefore(pill, node.firstChild)
    pillRef.current = pill

    /* 标记菜单，让配套 CSS 把「行自己的高亮底色」让位（选中行除外） */
    node.setAttribute("data-glide-menu", "")
    /* 滑块是 absolute，需要一个明确的定位祖先。菜单通常已经是 absolute/fixed，
       但有的实现是 static（靠 .motion-popover 的 transform 碰巧成为包含块）——
       那种情况下 offsetParent 的行为依赖是否有 transform，很脆。显式补上。 */
    if (getComputedStyle(node).position === "static") node.style.position = "relative"

    /* 打开时落到选中行并隐身；找不到选中项就落到第一行 */
    let raf = requestAnimationFrame(() => {
      raf = 0
      const checked = node.querySelector<HTMLElement>(ROW_SELECTOR + '[data-state="checked"]')
      const first = node.querySelector<HTMLElement>(ROW_SELECTOR)
      place(checked ?? first, true)
      // 打开时滑块隐身：选中行自己有底色（ProviderMenu 用的是 bg-jolu-selected）
      if (pill) pill.style.opacity = "0"
    })

    /* 鼠标：pointerover 会冒泡，绑在菜单上就能收到行的事件。
       不能用 pointerenter —— 它不冒泡，绑在容器上收不到子元素的事件。 */
    const onOver = (e: Event) => {
      const row = (e.target as HTMLElement)?.closest?.<HTMLElement>(ROW_SELECTOR)
      if (!row) return
      /* 禁用行：滑块隐身。
         之前是直接 return —— 滑块留在上一行不动，看起来像「两行之间多了一条灰条」
         （用户实测截图）。禁用行本来就没有高亮，滑块不该停在隔壁。 */
      if (row.hasAttribute("data-disabled")) {
        // 不记进 lastRow：离开菜单时不该把滑块恢复到禁用行上
        if (pill) pill.style.opacity = "0"
        return
      }
      if (row === lastRowRef.current) return
      // 非选中行打标记，让 CSS 清掉它自己的高亮底色（滑块来画）；
      // 选中行不打，保留它原有的选中底色，滑块在 place() 里隐身。
      if (row.getAttribute("data-state") !== "checked") row.setAttribute("data-glide-hi", "")
      lastRowRef.current = row
      place(row, false)
    }
    node.addEventListener("pointerover", onOver)

    /* 指针离开整个菜单：停在最后停留的行，不回弹 */
    const onLeave = (e: PointerEvent) => {
      const next = e.relatedTarget as Node | null
      if (next && node.contains(next)) return
      // 停在最后停留的「可用」行；lastRow 只在可用行上更新，所以这里不用再判 disabled
      if (lastRowRef.current) place(lastRowRef.current, false)
    }
    node.addEventListener("pointerleave", onLeave)

    /* 键盘：Radix 改 data-highlighted，属性变化不触发事件，用 MutationObserver。
       300ms 内按过键才认，避免把程序化改动也当键盘操作。 */
    let keyAt = 0
    const onKey = () => { keyAt = performance.now() }
    node.addEventListener("keydown", onKey, true)

    const observer = new MutationObserver(() => {
      if (performance.now() - keyAt > 300) return
      const row = node.querySelector<HTMLElement>(ROW_SELECTOR + '[data-highlighted]:not([data-disabled])')
      if (!row) return
      lastRowRef.current = row
      place(row, true)
    })
    observer.observe(node, { subtree: true, attributes: true, attributeFilter: ["data-highlighted", "data-state"] })

    cleanupRef.current = () => {
      if (raf) cancelAnimationFrame(raf)
      observer.disconnect()
      node.removeEventListener("pointerover", onOver)
      node.removeEventListener("pointerleave", onLeave)
      node.removeEventListener("keydown", onKey, true)
      node.removeAttribute("data-glide-menu")
    }
  }, [place])
}
