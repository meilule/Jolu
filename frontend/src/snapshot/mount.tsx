import { useCallback, useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { Moon, Sun } from "lucide-react"
import { cn } from "@/lib/utils"
import "@/index.css"
import "./snapshot.css"
import { AppIcon } from "@/components/brand/AppIcon"
import { emitSnapshotEvent } from "./fakeTauri"
import { setSnapshotDock, setSnapshotStyle, setSnapshotTheme, snapshotSettings } from "./snapshotCommands"

/**
 * 静态快照外壳：在同一个页面里预览主面板的全部模块，外加灵动岛窗口的两种样式。
 *
 * 外壳**不参与样式重构**——它只是一层「看板边框」，页面本体仍是主面板窗口里的真实结构。
 * 因此它自己只用中性灰与描边，不与 `jolu-*` 设计变量争抢注意力。
 */

type Mode = "panel" | "island"

const MODES: { id: Mode; label: string }[] = [
  { id: "panel", label: "主面板" },
  { id: "island", label: "灵动岛" },
]

export function mountSnapshot() {
  createRoot(document.getElementById("root")!).render(<SnapshotApp />)
}

function SnapshotApp() {
  const [mode, setMode] = useState<Mode>("panel")
  const [dark, setDarkState] = useState(() => new URLSearchParams(location.search).get("theme") === "dark")
  const [style, setStyle] = useState<"classic" | "capsule">(() => (snapshotSettings().island_style === "capsule" ? "capsule" : "classic"))
  const [dock, setDock] = useState<"top" | "bottom" | "left" | "right" | null>(null)

  useEffect(() => {
    const root = document.documentElement
    root.classList.toggle("dark", dark)
    root.classList.remove("island-window")
  }, [dark])

  const toggleTheme = useCallback(() => {
    setDarkState((current) => {
      const next = !current
      emitSnapshotEvent("settings-changed", setSnapshotTheme(next ? "dark" : "light"))
      return next
    })
  }, [])

  const changeStyle = useCallback((next: "classic" | "capsule") => {
    setStyle(next)
    emitSnapshotEvent("settings-changed", setSnapshotStyle(next))
  }, [])

  const changeDock = useCallback((next: "top" | "bottom" | "left" | "right" | null) => {
    setDock(next)
    emitSnapshotEvent("settings-changed", setSnapshotDock(next))
    emitSnapshotEvent("dock-changed", { island: null, dock: { edge: next, offset: 0, monitor: "DISPLAY1" } })
  }, [])

  return (
    <div className="min-h-dvh bg-[#f5f5f6] dark:bg-[#0b0b0c]">
      <SnapshotBar
        mode={mode}
        onMode={setMode}
        dark={dark}
        onTheme={toggleTheme}
        style={style}
        onStyle={changeStyle}
        dock={dock}
        onDock={changeDock}
      />
      <div className="mx-auto w-full max-w-[1400px] px-6 pb-10 pt-4">
        {mode === "panel" ? <PanelStage /> : <IslandStage style={style} />}
      </div>
    </div>
  )
}

function SnapshotBar({ mode, onMode, dark, onTheme, style, onStyle, dock, onDock }: {
  mode: Mode
  onMode: (mode: Mode) => void
  dark: boolean
  onTheme: () => void
  style: "classic" | "capsule"
  onStyle: (style: "classic" | "capsule") => void
  dock: "top" | "bottom" | "left" | "right" | null
  onDock: (dock: "top" | "bottom" | "left" | "right" | null) => void
}) {
  return (
    <header className="sticky top-0 z-30 border-b border-black/10 bg-white/85 backdrop-blur dark:border-white/10 dark:bg-[#141416]/85">
      <div className="mx-auto flex h-14 w-full max-w-[1400px] items-center gap-3 px-6">
        <AppIcon size={22} />
        <span className="text-[13px] font-semibold text-neutral-900 dark:text-neutral-100">Jolu 界面快照</span>
        <span className="text-[11px] text-neutral-500 dark:text-neutral-400">静态示例 · 用于样式对齐</span>
        <span className="flex-1" />
        <div className="flex items-center gap-0.5 rounded-lg bg-neutral-100 p-0.5 dark:bg-neutral-800" role="group" aria-label="窗口类型">
          {MODES.map((item) => (
            <button
              key={item.id}
              type="button"
              aria-pressed={mode === item.id}
              onClick={() => onMode(item.id)}
              className={cn(
                "h-7 rounded-md px-3 text-[12px] outline-none",
                mode === item.id
                  ? "bg-white font-medium text-neutral-900 shadow-sm dark:bg-neutral-700 dark:text-neutral-50"
                  : "text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100",
              )}
            >
              {item.label}
            </button>
          ))}
        </div>
        {mode === "island" && (
          <>
            <span className="mx-1 h-5 w-px bg-neutral-200 dark:bg-neutral-700" />
            <div className="flex items-center gap-0.5 rounded-lg bg-neutral-100 p-0.5 dark:bg-neutral-800" role="group" aria-label="灵动岛样式">
              {([["classic", "经典"], ["capsule", "胶囊"]] as const).map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  aria-pressed={style === id}
                  onClick={() => onStyle(id)}
                  className={cn(
                    "h-7 rounded-md px-3 text-[12px] outline-none",
                    style === id
                      ? "bg-white font-medium text-neutral-900 shadow-sm dark:bg-neutral-700 dark:text-neutral-50"
                      : "text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100",
                  )}
                >
                  {label}
                </button>
              ))}
            </div>
            <div className="flex items-center gap-0.5 rounded-lg bg-neutral-100 p-0.5 dark:bg-neutral-800" role="group" aria-label="停靠">
              {([[null, "自由"], ["top", "上"], ["bottom", "下"], ["left", "左"], ["right", "右"]] as const).map(([id, label]) => (
                <button
                  key={String(id)}
                  type="button"
                  aria-pressed={dock === id}
                  onClick={() => onDock(id)}
                  className={cn(
                    "h-7 rounded-md px-2.5 text-[12px] outline-none",
                    dock === id
                      ? "bg-white font-medium text-neutral-900 shadow-sm dark:bg-neutral-700 dark:text-neutral-50"
                      : "text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100",
                  )}
                >
                  {label}
                </button>
              ))}
            </div>
          </>
        )}
        <span className="mx-1 h-5 w-px bg-neutral-200 dark:bg-neutral-700" />
        <button
          type="button"
          onClick={onTheme}
          aria-label={dark ? "切换到浅色" : "切换到深色"}
          className="grid size-8 place-items-center rounded-lg border border-neutral-200 bg-white text-neutral-600 outline-none hover:bg-neutral-50 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-300"
        >
          {dark ? <Sun className="size-4" /> : <Moon className="size-4" />}
        </button>
      </div>
    </header>
  )
}

/** 主面板：直接渲染真实的主面板窗口（含标题栏、侧栏与内容区） */
function PanelStage() {
  const host = useRef<HTMLDivElement>(null)
  useEffect(() => {
    let disposed = false
    let cleanup: (() => void) | undefined
    void import("@/views/MainPanelWindow").then(({ MainPanelWindow }) => {
      if (disposed || !host.current) return
      const root = createRoot(host.current)
      root.render(<MainPanelWindow />)
      cleanup = () => root.unmount()
    })
    return () => { disposed = true; cleanup?.() }
  }, [])
  return (
    <div className="flex justify-center pt-6">
      <div ref={host} />
    </div>
  )
}

/** 灵动岛窗口：按窗口实际尺寸（400 宽）画一层桌面底，看透明边距与投影 */
function IslandStage({ style }: { style: "classic" | "capsule" }) {
  const host = useRef<HTMLDivElement>(null)
  const rootRef = useRef<{ unmount: () => void } | null>(null)
  useEffect(() => {
    let disposed = false
    void import("@/views/IslandWindow").then(({ IslandWindow }) => {
      if (disposed || !host.current) return
      // 换样式时先卸载旧树再挂新的：直接 replaceChildren 会让 React 在卸载时
      // 找不到自己的节点（removeChild NotFoundError），样式也切不干净
      rootRef.current?.unmount()
      host.current.replaceChildren()
      const root = createRoot(host.current)
      rootRef.current = root
      root.render(<div className="island-stage-shell" data-style={style}><IslandWindow /></div>)
    })
    return () => {
      disposed = true
      rootRef.current?.unmount()
      rootRef.current = null
    }
  }, [style])

  return (
    <div className="flex flex-col items-center gap-4 pt-6">
      <p className="text-[12px] text-neutral-500 dark:text-neutral-400">
        {style === "classic" ? "经典样式（A2）" : "胶囊样式（A3）"} · 双击岛可展开 / 收回
      </p>
      <div className="island-stage flex w-full justify-center">
        <div ref={host} />
      </div>
    </div>
  )
}
