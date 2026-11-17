import { cn } from "@/lib/utils"
import type { IslandData } from "./UsageIsland"

/**
 * 灵动岛联动的两块小界面（4.2b；无画板，待确认）：
 * - `SignalRing`：额度 80% / 用尽 / 网关拒绝 / 新会话时，沿岛的内侧描一圈 1.5 秒（不撑大窗口）；
 *   免打扰或减少动效时只亮颜色、不播动画；
 * - `FollowingBar`：本体岛正在跟随主窗口看的连接，带「解除」。
 */
export function SignalRing({ signal, round }: { signal: IslandData["signal"]; round: string }) {
  if (!signal) return null
  const tone = signal.kind === "quota-exhausted" || signal.kind === "rejected" ? "danger" : signal.kind === "quota-80" ? "warn" : "info"
  return (
    <span
      key={signal.at}
      role="status"
      aria-label={signal.label}
      title={signal.label}
      className={cn("pointer-events-none absolute inset-0 z-10", round, `island-signal-${tone}`, signal.motion ? "island-signal-motion" : "island-signal-static")}
    />
  )
}

export function FollowingBar({ following }: { following: IslandData["following"] }) {
  if (!following) return null
  return (
    <div className="flex max-w-[280px] items-center gap-2 rounded-lg bg-surface px-2.5 py-1 text-[11px] text-text-secondary shadow-sm">
      <span className="min-w-0 truncate" title={following.label}>{following.label}</span>
      <button type="button" onClick={following.onUnfollow} onPointerDown={(event) => event.stopPropagation()}
        className="shrink-0 font-medium text-text-primary underline-offset-2 outline-none hover:underline focus-visible:outline-2">
        解除
      </button>
    </div>
  )
}
