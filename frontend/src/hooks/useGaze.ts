import { useEffect, useRef, type RefObject } from "react"
import { gazeSpringStep, gazeTarget, sphereEyeTransform, type Vec } from "@/lib/botGaze"
import { isTauri, listenEvent } from "@/lib/api"
import { reducedMotion } from "@/lib/motion"

/**
 * 视线跟随：所有 bot 共用一个指针状态，每帧插值逼近目标，直接写 transform，不触发 React 重渲染。
 *
 * 指针来源（设计说明 §6.2）：
 * - 窗口内 `pointermove`：延迟最低，且光标在本窗口内时是唯一真值；
 * - 后端 `island-pointer`：桌面物理坐标，鼠标离开灵动岛窗口后仍能跟随。
 *   换算：减去窗口内容区左上角的物理坐标，再除以缩放比，得到窗口内的 CSS 坐标。
 *
 * 两个来源写同一个 pointer。展开/贴边缩放会移动窗口原点（dock.rs 的
 * resize_free_centered 左右各让一半），origin 刷新（onMoved → 异步 invoke）有
 * 几十毫秒的窗口期，期间后端映射值会系统性偏移——光标明明在卡上，映射却落在
 * 脸的下方，眼睛就一直朝下。裁决规则：光标在本窗口内的 150ms 内，后端推送
 * 一律让位给窗口事件（窗口事件是即时且必然正确的）；光标离开窗口后后端接管。
 */
const pointer = { x: 0, y: 0, on: false }
const listeners = new Set<() => void>()
const notify = () => listeners.forEach((listener) => listener())
let installed = false
/** 最近一次收到后端推送的时刻；推送在跑时，鼠标离开窗口不算「指针消失」 */
let lastGlobal = 0
/** 最近一次窗口内 pointermove 的时刻（webview 真值的新鲜度） */
let lastWindowMove = 0
/** 窗口几何版本号：resize 后 +1，后端映射前若过期则先刷新 origin/scale */
let geoRev = 0
let appliedGeoRev = 0

function install() {
  if (installed) return
  installed = true
  window.addEventListener("pointermove", (event) => {
    pointer.x = event.clientX; pointer.y = event.clientY; pointer.on = true
    lastWindowMove = performance.now()
    notify()
  }, true)
  const lost = () => {
    if (performance.now() - lastGlobal < 500) return
    pointer.on = false
    notify()
  }
  document.documentElement.addEventListener("mouseleave", lost)
  window.addEventListener("blur", lost)
  // 展开形变 / 滚动会移动脸的位置但没有 pointermove：结束时与过程中重新对准
  window.addEventListener("resize", notify, { passive: true })
  document.addEventListener("transitionend", notify, true)
  document.addEventListener("scroll", notify, true)
  if (isTauri) void installGlobal()
}

async function installGlobal() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window")
  const win = getCurrentWindow()
  let origin = { x: 0, y: 0 }
  let scale = window.devicePixelRatio || 1
  const refresh = async () => {
    try {
      const [position, factor] = await Promise.all([win.innerPosition(), win.scaleFactor()])
      origin = { x: position.x, y: position.y }
      scale = factor
    } catch { /* 窗口正在销毁 */ }
  }
  await refresh()
  // 拖动、停靠与缩放都会改变窗口原点
  void win.onMoved(() => { geoRev++; void refresh() })
  void win.onScaleChanged(() => { geoRev++; void refresh() })
  window.addEventListener("resize", () => { geoRev++; void refresh() })
  void listenEvent<{ x: number; y: number }>("island-pointer", (physical) => {
    const now = performance.now()
    lastGlobal = now
    // 光标在本窗口内（刚有过窗口内 pointermove）：窗口事件是即时真值，
    // 后端映射值（origin/scale 缓存在窗口缩放后有失准窗口期）不覆盖它
    if (now - lastWindowMove < 150) return
    // 几何过期（窗口刚缩放/移动）时先取新原点，避免用旧 origin 映射
    const pendingGeo = geoRev !== appliedGeoRev
    const apply = () => {
      appliedGeoRev = geoRev
      pointer.x = (physical.x - origin.x) / scale
      pointer.y = (physical.y - origin.y) / scale
      pointer.on = true
      notify()
    }
    if (pendingGeo) void refresh().then(apply)
    else apply()
  })
}

export interface GazeInput {
  base: Vec
  amp: number
  /** 球面眼睛的静息几何：眼距与垂直偏移（px）。缺省时退回整组平移的旧路径 */
  gapPx?: number
  cyPx?: number
}

export function useGaze(
  faceRef: RefObject<SVGCircleElement | null>,
  eyesRef: RefObject<SVGGElement | null>,
  input: GazeInput,
  faceSize: number,
) {
  // 视线状态跨渲染保留：表情切换只改目标，眼睛从当前位置平滑过去，不跳回中位
  const latest = useRef(input)
  latest.current = input
  const current = useRef<Vec>({ x: 0, y: 0 })
  const vel = useRef<Vec>({ x: 0, y: 0 })
  const prev = useRef(0)
  const kickRef = useRef<() => void>(() => {})

  useEffect(() => {
    install()
    let frame = 0
    const step = () => {
      frame = 0
      const face = faceRef.current, eyes = eyesRef.current
      if (!face || !eyes) return
      const rect = face.getBoundingClientRect()
      const reduced = reducedMotion()
      const target = gazeTarget(
        { cx: rect.left + rect.width / 2, cy: rect.top + rect.height / 2, width: rect.width },
        pointer.on && !reduced ? { x: pointer.x, y: pointer.y } : null,
        latest.current,
      )
      // 球面眼睛（EyeTracker 移植）：每只眼是球面上的点，透视变形由各自 transform 承担；
      // 表情切换瞬间 .cbot-eye-p 可能还没挂上，退回整组平移的旧路径
      const eyePs = reduced ? null : eyes.querySelectorAll<SVGGElement>(".cbot-eye-p")
      const spherical = eyePs && eyePs.length === 2 && latest.current.gapPx != null
      const now = performance.now()
      const dt = prev.current ? Math.min(2.5, (now - prev.current) / 16.67) : 1
      let settled: boolean
      if (reduced) {
        current.current = { ...target }
        vel.current = { x: 0, y: 0 }
        settled = true
      } else {
        const result = gazeSpringStep(current.current, vel.current, target, dt)
        current.current = result.next
        vel.current = result.vel
        settled = Math.abs(target.x - result.next.x) < 0.0005 && Math.abs(target.y - result.next.y) < 0.0005 &&
                  Math.abs(result.vel.x) < 0.0005 && Math.abs(result.vel.y) < 0.0005
      }
      if (settled) { current.current = { ...target }; vel.current = { x: 0, y: 0 } }
      if (spherical && eyePs) {
        const faceGeo = { gapPx: latest.current.gapPx ?? 0, cyPx: latest.current.cyPx ?? faceSize / 2, sizePx: faceSize }
        eyePs[0].setAttribute("transform", sphereEyeTransform(0, current.current, faceGeo))
        eyePs[1].setAttribute("transform", sphereEyeTransform(1, current.current, faceGeo))
        eyes.style.transform = ""
      } else {
        eyes.style.transform = `translate(${current.current.x * faceSize}px, ${current.current.y * faceSize}px)`
      }
      prev.current = settled ? 0 : now
      if (!settled) frame = requestAnimationFrame(step)
    }
    const kick = () => { if (!frame) frame = requestAnimationFrame(step) }
    kickRef.current = kick
    listeners.add(kick)
    kick()
    return () => { listeners.delete(kick); cancelAnimationFrame(frame); prev.current = 0; kickRef.current = () => {} }
  }, [faceRef, eyesRef, faceSize])

  // 每次渲染后重新对准：表情偏移、胶囊尺寸或位置可能变了
  useEffect(() => { kickRef.current() })
}
