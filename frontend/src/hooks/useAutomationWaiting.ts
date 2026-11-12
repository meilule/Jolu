import { useEffect, useState } from "react"
import { isTauri, listenEvent } from "@/lib/api"
import { AUTOMATION_CHANGED, automationApi } from "@/lib/automationApi"

/** 兜底轮询：后端每次改运行状态都会发 `automation-changed`，轮询只防漏收事件 */
const POLL_MS = 120_000

/**
 * 有任务在等额度恢复的账号 id（4.2：灵动岛「等额度」表情的数据源）。
 * 读不到时返回空集合——不知道就不显示在等，不把失败当成「在等」。
 */
export function useAutomationWaiting(enabled = true): ReadonlySet<string> {
  const [ids, setIds] = useState<ReadonlySet<string>>(() => new Set())

  useEffect(() => {
    if (!enabled || !isTauri) return
    let alive = true
    const load = () => {
      automationApi.waitingConnections()
        .then((list) => { if (alive) setIds(new Set(list)) })
        .catch(() => { if (alive) setIds(new Set()) })
    }
    load()
    let off: (() => void) | null = null
    void listenEvent(AUTOMATION_CHANGED, load).then((unlisten) => {
      if (alive) off = unlisten
      else unlisten()
    }).catch(() => {})
    const timer = window.setInterval(load, POLL_MS)
    return () => {
      alive = false
      off?.()
      window.clearInterval(timer)
    }
  }, [enabled])

  return ids
}
