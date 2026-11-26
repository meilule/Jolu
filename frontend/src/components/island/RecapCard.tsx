import { useState } from "react"
import { Icon } from "@/icons"
import { compactTokens } from "@/lib/api"
import type { RecapData } from "@/lib/islandRecap"
import { cn } from "@/lib/utils"

/**
 * R2-I-07 · 今日回顾与成就卡。
 * 复用灵动岛深色硬件卡片视觉语言；所有数字取自本机真实统计，缺项显示「—」；
 * 成就只存在本机，可一键清除后重新累计。
 */
export function RecapCard({
  recap,
  onOpenUsage,
  onDismiss,
  onClearAchievements,
}: {
  recap: RecapData
  onOpenUsage: () => void
  onDismiss: () => void
  onClearAchievements: () => void
}) {
  const [showBadges, setShowBadges] = useState(false)
  const [confirmClear, setConfirmClear] = useState(false)
  const unlockedCount = recap.allAchievements.filter((a) => a.unlockedOn !== null).length

  return (
    <section
      role="dialog"
      aria-label="今日回顾"
      data-island-recap
      className="cap-recap"
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      <header className="cap-recap-hd">
        <div className="flex flex-col gap-0.5">
          <span className="cap-recap-title">今天辛苦了</span>
          <span className="cap-t3">{recap.dateLabel}</span>
        </div>
        <button
          type="button"
          aria-label="关闭今日回顾"
          className="cap-recap-close"
          onClick={onDismiss}
        >
          <Icon name="close" size="xs" />
        </button>
      </header>

      <div className="cap-recap-grid">
        <div className="cap-recap-tile">
          <b className="cap-num cap-mono">{recap.todayTokens === null ? "—" : compactTokens(recap.todayTokens)}</b>
          <span>Token</span>
        </div>
        <div className="cap-recap-tile">
          <b className="cap-num cap-mono">{recap.sessionCount === null ? "—" : recap.sessionCount}</b>
          <span>次会话</span>
        </div>
        <div className="cap-recap-tile">
          <b className={cn("cap-num cap-mono", recap.cacheSavedUsd !== null && "cap-g")}>
            {recap.cacheSavedUsd === null ? "—" : `≈ $${recap.cacheSavedUsd.toFixed(2)}`}
          </b>
          <span>缓存省下</span>
        </div>
        <div className="cap-recap-tile">
          <b className="cap-num cap-mono">
            {recap.checkinClaimed === null || recap.checkinTotal === null
              ? "—"
              : `${recap.checkinClaimed} / ${recap.checkinTotal}`}
          </b>
          <span>账号已签到</span>
        </div>
      </div>

      {recap.topTool && (
        <div className="cap-recap-top">
          <span className="cap-t2">最常用 · <b className="text-[var(--pn-capsule-text)]">{recap.topTool.name}</b></span>
          <span className="cap-num cap-mono cap-g font-semibold">{recap.topTool.sharePercent}%</span>
        </div>
      )}

      {recap.unlockedToday.length > 0 && (
        <div className="cap-recap-unlocked">
          <span className="cap-t3">今日解锁</span>
          <div className="flex flex-wrap gap-1.5">
            {recap.unlockedToday.map((item) => (
              <span key={item.id} className="cap-recap-badge-pill">
                <Icon name="spark" size="xs" />
                {item.name}
              </span>
            ))}
          </div>
        </div>
      )}

      <div className="cap-recap-ach">
        <button
          type="button"
          className="cap-recap-ach-toggle"
          aria-expanded={showBadges}
          onClick={() => { setShowBadges((v) => !v); setConfirmClear(false) }}
        >
          <span>成就徽章 · {unlockedCount} / {recap.allAchievements.length}</span>
          <Icon name={showBadges ? "collapse" : "expand"} size="xs" />
        </button>
        {showBadges && (
          <div className="cap-recap-ach-list">
            {recap.allAchievements.map((item) => (
              <div key={item.id} className={cn("cap-recap-ach-row", item.unlockedOn && "cap-recap-ach-on")}>
                <span className="cap-recap-ach-dot" />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center justify-between gap-2">
                    <span className="truncate font-semibold">{item.name}</span>
                    <span className="shrink-0 text-[10px] text-[var(--pn-capsule-text-3)]">
                      {item.unlockedOn ? `${item.unlockedOn.slice(5)} 解锁` : "未解锁"}
                    </span>
                  </div>
                  <p className="m-0 text-[10px] text-[var(--pn-capsule-text-3)]">{item.condition}</p>
                </div>
              </div>
            ))}
            <div className="flex items-center justify-between pt-1 text-[10px] text-[var(--pn-capsule-text-3)]">
              <span>仅存本机，不联网、不排名</span>
              {unlockedCount > 0 && (
                confirmClear ? (
                  <span className="flex items-center gap-2">
                    <button type="button" className="cap-r font-semibold" onClick={() => { onClearAchievements(); setConfirmClear(false) }}>
                      确认清除
                    </button>
                    <button type="button" onClick={() => setConfirmClear(false)}>取消</button>
                  </span>
                ) : (
                  <button type="button" className="hover:text-[var(--pn-capsule-text)]" onClick={() => setConfirmClear(true)}>
                    清除记录
                  </button>
                )
              )}
            </div>
          </div>
        )}
      </div>

      <footer className="cap-recap-foot">
        <button type="button" className="cap-recap-btn cap-recap-btn-soft" onClick={onOpenUsage}>
          看今日用量
        </button>
        <button type="button" className="cap-recap-btn cap-recap-btn-primary" onClick={onDismiss}>
          明天见
        </button>
      </footer>
    </section>
  )
}
