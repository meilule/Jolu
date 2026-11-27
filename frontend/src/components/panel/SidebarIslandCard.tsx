/**
 * SidebarIslandCard —— 设计稿 R1-C「分区 · 1 · 外壳 / 侧栏底部 · 灵动岛迷你状态卡」。
 *
 * - 展开态：浅绿底（`jn-primary-wash`）、圆角 16、内边距 12；首行状态点 + 灵动岛关注连接 + 额度百分比，
 *   中段 6px 额度进度条（未知时虚线空轨，绝不伪造 0%），底部一行状态说明；
 * - 收起态（R1-06，72px 侧栏）：居中一枚 40px `QuotaRing`，悬停 Tooltip 显示连接名与状态；
 * - 点击跳到主窗口「灵动岛」页。
 */
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { useSettings } from "@/hooks/useSettings"
import { useConnections } from "@/hooks/useConnections"
import { useQuota } from "@/hooks/useQuota"
import { PLATFORMS } from "@/lib/platforms"
import { TactileButton } from "@/motion"
import { Progress, QuotaRing, Tooltip } from "@/ui"

export function SidebarIslandCard({
  collapsed,
  onOpenIsland,
}: {
  collapsed: boolean
  onOpenIsland: () => void
}) {
  const { settings } = useSettings()
  const { connections } = useConnections()
  const target = settings.island_connection_id
    ? connections.find((c) => c.id === settings.island_connection_id) ?? null
    : null
  const canQueryQuota = Boolean(isTauri && target && target.kind === "auth" && target.status !== "paused")
  const quota = useQuota(canQueryQuota ? target!.id : null, false, canQueryQuota)

  // 取首个有数值的额度窗口作为主进度显示；无数据时保持 null（未知态）
  const firstWindow =
    quota.state?.state === "ok"
      ? quota.state.windows.find((w) => typeof w.used_percent === "number") ?? null
      : null
  const usedPercent =
    firstWindow && typeof firstWindow.used_percent === "number"
      ? Math.max(0, Math.min(100, Math.round(firstWindow.used_percent)))
      : null
  const remaining = usedPercent !== null ? Math.max(0, 100 - usedPercent) : null

  const platformName = target
    ? PLATFORMS.find((p) => p.id === target.platformId)?.name ?? target.platformId
    : null
  const title = target
    ? `${platformName} · ${target.baseName}`
    : settings.island_connection_name || "灵动岛"

  const cloneCount = settings.island_clones?.length ?? 0
  const visible = settings.island_visible
  const subtitle = !visible
    ? "灵动岛已隐藏 · 点击设置"
    : usedPercent !== null
      ? `剩余 ${remaining}%${cloneCount > 0 ? ` · ${cloneCount + 1} 个岛` : ""}`
      : target
        ? `${target.kind === "api" ? "API Key · 额度不可查" : "暂无额度读数"}${cloneCount > 0 ? ` · ${cloneCount + 1} 个岛` : ""}`
        : `灵动岛运行中${cloneCount > 0 ? ` · ${cloneCount + 1} 个岛` : ""}`

  if (collapsed) {
    return (
      <Tooltip content={`${title} · ${subtitle}`} side="right">
        <TactileButton
          variant="icon"
          type="button"
          onClick={onOpenIsland}
          aria-label={`灵动岛状态：${title}`}
          className="jn-interactive flex h-12 w-full items-center justify-center rounded-2xl bg-jn-primary-wash hover:brightness-[0.98]"
        >
          <QuotaRing usedPercent={usedPercent} size="sm" />
        </TactileButton>
      </Tooltip>
    )
  }

  return (
    <TactileButton
      variant="tile"
      type="button"
      onClick={onOpenIsland}
      className="jn-interactive flex w-full flex-col gap-2 rounded-2xl bg-jn-primary-wash p-3 text-left hover:brightness-[0.98]"
    >
      <span className="flex w-full items-center gap-2">
        <span
          aria-hidden
          className={cn(
            "size-[7px] shrink-0 rounded-full",
            !visible
              ? "bg-jn-text-3"
              : usedPercent !== null && usedPercent >= 85
                ? "bg-jn-warning"
                : "bg-jn-primary",
          )}
        />
        <span className="min-w-0 flex-1 truncate text-xs font-semibold text-jn-text">{title}</span>
        <span className="tnum shrink-0 font-mono text-xs font-semibold text-jn-primary-ink">
          {usedPercent !== null ? `${usedPercent}%` : "—"}
        </span>
      </span>
      <Progress percent={usedPercent} highAt={85} />
      <span className="truncate text-[11px] text-jn-text-3">{subtitle}</span>
    </TactileButton>
  )
}
