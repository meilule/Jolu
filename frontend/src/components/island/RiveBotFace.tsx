import { useEffect, useRef, useState } from "react"
import {
  BOT_EXPR_INDEX,
  BOT_EYE_TONE_INDEX,
  BOT_PROP_INDEX,
  BOT_SHOT_INDEX,
  botEyeTone,
  type BotExpr,
  type BotProp,
  type BotShot,
} from "@/lib/botExpression"

/**
 * Rive bot 脸部运行时桥接（docs/specs/2026-11-24-island-rendering-tech.md §6）。
 *
 * - 只画额度环里面的「脸与道具」，额度环仍由 `Bot.tsx` 用 SVG 精准绘制；
 * - `.riv` 资源通过 Vite glob 按需探查 `frontend/src/island/assets/bot.riv`：
 *   资产尚未交付或 Rive 运行时加载失败时，自动记一条诊断日志并通知 `Bot.tsx` 回退到 SVG 眼睛；
 * - 运行时包 `@rive-app/react-canvas` 仅在 `.riv` 存在时才动态 import，绝不进入主窗口首屏包。
 */

const STATE_MACHINE = "BotStateMachine"

// Vite 构建期扫描资产：交付 bot.riv 后自动生效，未交付时为空对象且不报构建错
const RIV_ASSETS = import.meta.glob<string>("../../island/assets/bot.riv", {
  query: "?url",
  import: "default",
})

let loggedFallbackOnce = false
function logFallbackOnce(reason: string) {
  if (loggedFallbackOnce) return
  loggedFallbackOnce = true
  console.info(`[灵动岛·Rive] ${reason}，已自动回退到 SVG 眼睛`)
}

export function hasRiveBotAsset(): boolean {
  return Object.keys(RIV_ASSETS).length > 0
}

export interface RiveBotFaceProps {
  size: number
  expr: BotExpr
  certainty?: "confirmed" | "inferred" | "unknown" | "idle"
  prop?: BotProp
  shot?: BotShot | null
  pokeSeq?: number
  gazeRef?: { current: { x: number; y: number } }
  small: boolean
  reduced: boolean
  onReadyChange?: (ready: boolean) => void
}

interface RiveRuntimeInput {
  name: string
  value: number | boolean
  fire?: () => void
}

interface RiveCanvasInstance {
  stateMachineInputs: (name: string) => RiveRuntimeInput[] | null | undefined
  cleanup?: () => void
}

interface RiveCanvasModule {
  Rive: new (opts: Record<string, unknown>) => RiveCanvasInstance
  Layout?: new (opts: Record<string, unknown>) => unknown
  Fit?: { Contain: unknown }
  Alignment?: { Center: unknown }
}

/**
 * 若 `bot.riv` 资产已交付且 Rive 运行时可用，挂载 Canvas 并同步 §6.2 全部 8 组状态机输入；
 * 否则返回 null 让 `Bot.tsx` 透明显示 SVG 兜底眼睛。
 */
export function RiveBotFace({
  size,
  expr,
  certainty,
  prop = "none",
  shot = null,
  pokeSeq = 0,
  gazeRef,
  small,
  reduced,
  onReadyChange,
}: RiveBotFaceProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const inputsRef = useRef<Map<string, RiveRuntimeInput>>(new Map())
  const [ready, setReady] = useState(false)
  const [failed, setFailed] = useState(() => !hasRiveBotAsset())
  const lastPokeRef = useRef(pokeSeq)
  const lastShotRef = useRef<BotShot | null>(null)

  useEffect(() => {
    onReadyChange?.(ready)
  }, [ready, onReadyChange])

  useEffect(() => {
    const loaders = Object.values(RIV_ASSETS)
    if (loaders.length === 0) {
      logFallbackOnce("未检测到 frontend/src/island/assets/bot.riv")
      setFailed(true)
      setReady(false)
      return
    }

    let disposed = false
    let instance: RiveCanvasInstance | null = null

    const boot = async () => {
      try {
        const assetUrl = await loaders[0]()
        // 动态拼包名：避免包未安装或主窗口构建时被静态打包进主包
        const pkg = "@rive-app/canvas"
        const mod = (await import(/* @vite-ignore */ pkg)) as unknown as RiveCanvasModule
        if (disposed || !canvasRef.current || !mod?.Rive) return

        instance = new mod.Rive({
          src: assetUrl,
          canvas: canvasRef.current,
          autoplay: true,
          stateMachines: STATE_MACHINE,
          layout: mod.Layout && mod.Fit && mod.Alignment
            ? new mod.Layout({ fit: mod.Fit.Contain, alignment: mod.Alignment.Center })
            : undefined,
          onLoad: () => {
            if (disposed || !instance) return
            const list = instance.stateMachineInputs(STATE_MACHINE) ?? []
            const map = new Map<string, RiveRuntimeInput>()
            for (const input of list) map.set(input.name, input)
            inputsRef.current = map
            setFailed(false)
            setReady(true)
          },
          onLoadError: () => {
            if (disposed) return
            logFallbackOnce("bot.riv 加载失败")
            setFailed(true)
            setReady(false)
          },
        })
      } catch {
        if (disposed) return
        logFallbackOnce("Rive 运行时不可用")
        setFailed(true)
        setReady(false)
      }
    }

    void boot()
    return () => {
      disposed = true
      inputsRef.current.clear()
      instance?.cleanup?.()
    }
  }, [])

  // 同步 §6.2 状态机输入：expr / eyeTone / prop / small / reduced
  useEffect(() => {
    if (!ready) return
    const map = inputsRef.current
    const setVal = (name: string, value: number | boolean) => {
      const input = map.get(name)
      if (input) input.value = value
    }
    setVal("expr", BOT_EXPR_INDEX[expr] ?? 0)
    setVal("eyeTone", BOT_EYE_TONE_INDEX[botEyeTone(expr, certainty)])
    setVal("prop", BOT_PROP_INDEX[prop] ?? 0)
    setVal("small", small)
    setVal("reduced", reduced)
  }, [ready, expr, certainty, prop, small, reduced])

  // 触发器：poke（§6.2）
  useEffect(() => {
    if (!ready || pokeSeq === lastPokeRef.current) return
    lastPokeRef.current = pokeSeq
    inputsRef.current.get("poke")?.fire?.()
  }, [ready, pokeSeq])

  // 触发器：shot + shotKind（§6.2）
  useEffect(() => {
    if (!ready) return
    if (shot && shot !== lastShotRef.current) {
      const kind = inputsRef.current.get("shotKind")
      if (kind) kind.value = BOT_SHOT_INDEX[shot]
      inputsRef.current.get("shot")?.fire?.()
    }
    lastShotRef.current = shot
  }, [ready, shot])

  // 同步视线跟随 gazeX / gazeY（仅在 Rive 活跃且未开启减少动效时按需推送）
  useEffect(() => {
    if (!ready || !gazeRef || reduced) return
    let raf = 0
    let prevX = Number.NaN
    let prevY = Number.NaN
    const step = () => {
      const { x, y } = gazeRef.current
      if (Math.abs(x - prevX) > 0.004 || Math.abs(y - prevY) > 0.004) {
        prevX = x
        prevY = y
        const gx = inputsRef.current.get("gazeX")
        const gy = inputsRef.current.get("gazeY")
        if (gx) gx.value = x
        if (gy) gy.value = y
      }
      raf = window.requestAnimationFrame(step)
    }
    raf = window.requestAnimationFrame(step)
    return () => window.cancelAnimationFrame(raf)
  }, [ready, gazeRef, reduced])

  if (failed) return null

  return (
    <canvas
      ref={canvasRef}
      width={Math.round(size * (window.devicePixelRatio || 1))}
      height={Math.round(size * (window.devicePixelRatio || 1))}
      aria-hidden
      style={{
        width: size,
        height: size,
        opacity: ready ? 1 : 0,
        pointerEvents: "none",
      }}
    />
  )
}
