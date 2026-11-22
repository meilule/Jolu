/**
 * 静态快照用的 Tauri IPC 替身。
 *
 * 快照要复用真实组件，而组件在挂载时读取 `isTauri`（模块级常量）决定走桌面分支还是
 * 浏览器示例分支。为了看到「桌面端真实界面」，快照必须在导入任何业务模块之前把
 * `__TAURI_INTERNALS__` 装好——所以入口用动态 import 顺序保证这一点。
 *
 * 只实现真实用到的协议面：
 * - 普通命令：交给注入的 handler；
 * - `plugin:event|listen / emit / unlisten`：进程内事件总线，配合 `emit` 广播设置变更；
 * - `plugin:window|*`、`plugin:app|*`、`plugin:opener|*`：返回无害的固定值。
 */

type InvokeHandler = (cmd: string, args: Record<string, unknown>) => unknown

interface Listener {
  event: string
  handlerId: number
}

let nextCallbackId = 1
const callbacks = new Map<number, { callback: (data: unknown) => void; once: boolean }>()
const listeners = new Set<Listener>()
/** 未实现的命令：快照会把这些名字打到控制台，便于补齐假数据 */
export const unknownCommands = new Set<string>()

export function installFakeTauri(handler: InvokeHandler, label = "main"): void {
  const transformCallback = (callback: (data: unknown) => void, once = false): number => {
    const id = nextCallbackId++
    callbacks.set(id, { callback, once })
    return id
  }
  const runCallback = (id: number, data: unknown): void => {
    const entry = callbacks.get(id)
    if (!entry) return
    if (entry.once) callbacks.delete(id)
    entry.callback(data)
  }

  function dispatch(event: string, payload: unknown): void {
    for (const listener of [...listeners]) {
      if (listener.event !== event) continue
      runCallback(listener.handlerId, { event, id: listener.handlerId, payload })
    }
  }

  const pluginEvent = (cmd: string, args: Record<string, unknown>): unknown => {
    switch (cmd) {
      case "plugin:event|listen": {
        const handlerId = Number(args.handler)
        listeners.add({ event: String(args.event), handlerId })
        return handlerId
      }
      case "plugin:event|unlisten": {
        for (const listener of [...listeners]) {
          if (listener.event === args.event && listener.handlerId === args.eventId) listeners.delete(listener)
        }
        return null
      }
      case "plugin:event|emit":
      case "plugin:event|emit_to":
        dispatch(String(args.event), args.payload)
        return null
      default:
        return null
    }
  }

  const pluginWindow = (cmd: string, args: Record<string, unknown>): unknown => {
    switch (cmd) {
      case "plugin:window|is_maximized":
      case "plugin:window|is_fullscreen":
      case "plugin:window|is_focused":
      case "plugin:window|is_minimized":
      case ASK_DECORATED:
        return false
      case "plugin:window|is_visible":
      case "plugin:window|is_enabled":
      case "plugin:window|is_resizable":
      case "plugin:window|is_closable":
      case "plugin:window|is_maximizable":
      case "plugin:window|is_minimizable":
        return true
      case "plugin:window|scale_factor":
        return 1
      case "plugin:window|inner_size":
      case "plugin:window|outer_size":
        return { width: 1280, height: 900 }
      case "plugin:window|inner_position":
      case "plugin:window|outer_position":
        return { x: 0, y: 0 }
      case "plugin:window|theme":
        return document.documentElement.classList.contains("dark") ? "dark" : "light"
      case "plugin:window|title":
        return "Jolu"
      case "plugin:window|current_monitor":
      case "plugin:window|primary_monitor":
        return { name: "DISPLAY1", size: { width: 2560, height: 1440 }, position: { x: 0, y: 0 }, scaleFactor: 1 }
      case "plugin:window|available_monitors":
        return []
      default:
        void args
        return null
    }
  }

  const invoke = async (cmd: string, args: Record<string, unknown> = {}): Promise<unknown> => {
    if (cmd.startsWith("plugin:event|")) return pluginEvent(cmd, args)
    if (cmd.startsWith("plugin:window|")) return pluginWindow(cmd, args)
    if (cmd === "plugin:app|version") return "0.1.0"
    if (cmd === "plugin:app|name") return "Jolu"
    if (cmd === "plugin:app|identifier") return "dev.jolu.app"
    if (cmd === "plugin:app|tauri_version") return "2.11.1"
    if (cmd === "plugin:app|bundle_type") return "nsis"
    if (cmd.startsWith("plugin:app|")) return null
    if (cmd === "plugin:opener|open_url") {
      window.open(String(args.url), "_blank", "noopener,noreferrer")
      return null
    }
    if (cmd.startsWith("plugin:opener|")) return null
    if (cmd.startsWith("plugin:updater|")) return null
    return handler(cmd, args)
  }

  window.__TAURI_INTERNALS__ = {
    invoke,
    transformCallback,
    unregisterCallback: (id: number) => callbacks.delete(id),
    runCallback,
    callbacks,
    convertFileSrc: (path: string) => path,
    metadata: { currentWindow: { label }, currentWebview: { windowLabel: label, label } },
  }
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: (_event: string, id: number) => callbacks.delete(id),
  }
  window.__TAURI_SNAPSHOT_EMIT__ = dispatch
}

/** 快照外壳用它广播设置 / 事件变更 */
export function emitSnapshotEvent(event: string, payload: unknown): void {
  window.__TAURI_SNAPSHOT_EMIT__?.(event, payload)
}

declare global {
  interface Window {
    __TAURI_INTERNALS__?: Record<string, unknown>
    __TAURI_SNAPSHOT_EMIT__?: (event: string, payload: unknown) => void
  }
}

const ASK_DECORATED = "plugin:window|is_decorated"
