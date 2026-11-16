import { useEffect, useRef } from "react"
import { createLatestGate, type LatestGate } from "@/lib/latestGate"

/** 组件内的「只认最新一次请求」闸门（见 `lib/latestGate.ts`）；组件卸载时作废进行中的请求，
 *  顺带避免卸载后再写状态。 */
export function useLatestGate(): LatestGate {
  const gate = useRef<LatestGate | null>(null)
  if (gate.current === null) gate.current = createLatestGate()
  useEffect(() => {
    const current = gate.current
    return () => current?.invalidate()
  }, [])
  return gate.current
}
