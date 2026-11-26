import { listenEvent } from "./api"
import { EVENTS } from "@/lib/events"
import { isUsageChange, throttleTrailing, USAGE_REFRESH_THROTTLE_MS } from "./usageRefresh"

/**
 * 订阅某平台统计数据的变化，按 [`USAGE_REFRESH_THROTTLE_MS`] 合并连续变化。
 * 返回的取消函数同时清掉尚未执行的补刷新。
 */
export function onUsageChanged(platform: string, refresh: () => void, wait = USAGE_REFRESH_THROTTLE_MS): () => void {
  const throttled = throttleTrailing(refresh, wait)
  let stopped = false
  const offs: Array<() => void> = []
  const keep = (unlisten: () => void) => {
    if (stopped) unlisten()
    else offs.push(unlisten)
  }
  void listenEvent(EVENTS.LIVE_USAGE, (usage) => {
    if (!stopped && isUsageChange(usage, platform)) throttled()
  }).then(keep).catch(() => {})
  void listenEvent(EVENTS.PRICING_CHANGED, () => {
    if (!stopped) refresh()
  }).then(keep).catch(() => {})
  return () => {
    stopped = true
    throttled.cancel()
    offs.forEach((off) => off())
  }
}
