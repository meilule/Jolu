import { useEffect } from "react"
import { api, isTauri } from "@/lib/api"

/**
 * 主窗口报告「正在看哪个连接」（4.2b #9「跟随主窗口」；无画板，待确认）。
 * 本体岛开了「跟随主窗口」才会用它；后端只在内存里记，值变了才广播 `main-context`。
 * 离开页面（卸载）或不再盯着某个连接时报 null，岛随之回到自己的连接。
 */
export function useMainContextReport(page: string, connectionId: string | null) {
  useEffect(() => {
    if (!isTauri) return
    void api.setMainContext(page, connectionId).catch(() => {})
  }, [page, connectionId])
  useEffect(() => {
    if (!isTauri) return
    return () => { void api.setMainContext(page, null).catch(() => {}) }
  }, [page])
}
