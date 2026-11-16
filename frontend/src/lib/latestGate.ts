/**
 * 「只认最新一次请求」的闸门（P1「防旧响应回填」的统一工具）。
 *
 * 用在会被反复调用的加载函数上（定时轮询、事件、按钮共用一个 `reload`）：前一次请求还没回来
 * 又发了一次时，先发的结果后到会把新结果盖掉。每次请求先 `begin()` 领号，拿到结果后用返回的
 * 检查函数确认自己仍是最新一号，不是就丢弃。
 *
 * ```ts
 * const isLatest = gate.begin()
 * const value = await api.load()
 * if (!isLatest()) return
 * setValue(value)
 * ```
 *
 * 只在 effect 里发、靠清理函数置 `cancelled` 的请求不需要它（依赖一变，旧 effect 的清理已经
 * 作废了旧请求）；组件里用 `hooks/useLatestGate.ts`，卸载时整体作废。
 */
export interface LatestGate {
  /** 开始一次请求，返回「这次是否仍是最新」的检查函数 */
  begin(): () => boolean
  /** 作废所有进行中的请求（卸载、或明确不再需要之前的结果时） */
  invalidate(): void
}

export function createLatestGate(): LatestGate {
  let latest = 0
  return {
    begin() {
      const ticket = ++latest
      return () => ticket === latest
    },
    invalidate() {
      latest++
    },
  }
}
