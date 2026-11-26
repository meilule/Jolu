/**
 * 今日回顾与本机成就纯逻辑（jolu-new.pen R2-I-07）。
 *
 * - 傍晚 18:00 后第一次空闲（30 分钟内没有写入）时弹出一次回顾卡；
 * - 6 枚成就全部基于本机已有真实记录（签到、用量、路由流水、Agent 接入）计算，
 *   只存本机，不联网、不排名、不为成就多调用模型。
 */
import { localDayKey } from "./islandTheater.ts"

export const RECAP_START_HOUR = 18
export const RECAP_IDLE_MS = 30 * 60_000

export type AchievementId =
  | "checkin_streak_7"
  | "cache_master"
  | "three_agents"
  | "zero_failure_day"
  | "night_owl"
  | "relay_king"

export interface AchievementDef {
  id: AchievementId
  name: string
  condition: string
}

export const ACHIEVEMENTS: readonly AchievementDef[] = [
  { id: "checkin_streak_7", name: "连续签到 7 天", condition: "一键签到记录连续 7 天成功" },
  { id: "cache_master", name: "缓存高手", condition: "当天缓存命中 ≥ 90%" },
  { id: "three_agents", name: "三连接入", condition: "接入 3 个 Agent 到模型路由" },
  { id: "zero_failure_day", name: "零失败日", condition: "当天路由流水没有失败请求" },
  { id: "night_owl", name: "夜猫子", condition: "凌晨 2 点后仍有会话在写" },
  { id: "relay_king", name: "接力王", condition: "一天内改道成功 10 次" },
] as const

export interface AchievementRecord {
  /** 首次解锁的本地日期 YYYY-MM-DD */
  unlockedOn: string
}

export type AchievementStore = Partial<Record<AchievementId, AchievementRecord>>

export interface AchievementSignals {
  checkinStreakDays: number
  cacheHitRate: number | null
  todayRequests: number
  wiredAgentCount: number
  todayRouteRequests: number
  todayRouteFailures: number
  wroteAfter2am: boolean
  todayReroutes: number
}

export interface RecapData {
  dateLabel: string
  todayTokens: number | null
  sessionCount: number | null
  cacheSavedUsd: number | null
  checkinClaimed: number | null
  checkinTotal: number | null
  topTool: { name: string; sharePercent: number } | null
  unlockedToday: AchievementDef[]
  allAchievements: Array<AchievementDef & { unlockedOn: string | null }>
}

export function shouldShowDailyRecap(opts: {
  enabled: boolean
  dnd: boolean
  hour: number
  writing: boolean
  inactiveMs: number
  dismissedToday: boolean
}): boolean {
  if (!opts.enabled || opts.dnd || opts.writing || opts.dismissedToday) return false
  return opts.hour >= RECAP_START_HOUR && opts.inactiveMs >= RECAP_IDLE_MS
}

/**
 * 统计一键签到记录的连续成功天数（按本地日历天倒推）。
 */
export function computeCheckinStreak(claimedTimestampsMs: readonly number[], now: Date = new Date()): number {
  if (claimedTimestampsMs.length === 0) return 0
  const days = new Set(claimedTimestampsMs.map((ts) => localDayKey(new Date(ts))))
  const cursor = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  // 今天还没签但昨天签过时，从昨天起算当前连续记录
  if (!days.has(localDayKey(cursor))) {
    cursor.setDate(cursor.getDate() - 1)
    if (!days.has(localDayKey(cursor))) return 0
  }
  let streak = 0
  while (days.has(localDayKey(cursor))) {
    streak += 1
    cursor.setDate(cursor.getDate() - 1)
  }
  return streak
}

/**
 * 按真实记录计算 6 项本机成就并与历史存储合并；仅在首次达成时记录解锁日期。
 */
export function evaluateAchievements(
  signals: AchievementSignals,
  existing: AchievementStore,
  todayKey: string,
): { nextStore: AchievementStore; unlockedToday: AchievementDef[]; changed: boolean } {
  const met: Record<AchievementId, boolean> = {
    checkin_streak_7: signals.checkinStreakDays >= 7,
    cache_master: signals.todayRequests > 0 && signals.cacheHitRate !== null && signals.cacheHitRate >= 0.9,
    three_agents: signals.wiredAgentCount >= 3,
    zero_failure_day: signals.todayRouteRequests > 0 && signals.todayRouteFailures === 0,
    night_owl: signals.wroteAfter2am,
    relay_king: signals.todayReroutes >= 10,
  }

  const nextStore: AchievementStore = { ...existing }
  const unlockedToday: AchievementDef[] = []
  let changed = false

  for (const def of ACHIEVEMENTS) {
    const current = nextStore[def.id]
    if (!current && met[def.id]) {
      nextStore[def.id] = { unlockedOn: todayKey }
      changed = true
    }
    if (nextStore[def.id]?.unlockedOn === todayKey) {
      unlockedToday.push(def)
    }
  }

  return { nextStore, unlockedToday, changed }
}

const WEEKDAYS = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"] as const

export function formatRecapDate(date: Date = new Date()): string {
  return `${date.getMonth() + 1} 月 ${date.getDate()} 日 · ${WEEKDAYS[date.getDay()]}`
}

const ACHIEVEMENTS_KEY = "jolu:island:achievements:v1"
const RECAP_DISMISSED_PREFIX = "jolu:island:recap-dismissed:"
const NIGHT_OWL_PREFIX = "jolu:island:night-owl:"

export function readAchievementStore(storage: Pick<Storage, "getItem"> = localStorage): AchievementStore {
  try {
    const raw = storage.getItem(ACHIEVEMENTS_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw)
    return parsed && typeof parsed === "object" ? (parsed as AchievementStore) : {}
  } catch {
    return {}
  }
}

export function writeAchievementStore(store: AchievementStore, storage: Pick<Storage, "setItem"> = localStorage): void {
  try {
    storage.setItem(ACHIEVEMENTS_KEY, JSON.stringify(store))
  } catch { /* ignore */ }
}

export function clearAchievementStore(storage: Pick<Storage, "removeItem"> = localStorage): void {
  try {
    storage.removeItem(ACHIEVEMENTS_KEY)
  } catch { /* ignore */ }
}

export function isRecapDismissed(day: string, storage: Pick<Storage, "getItem"> = localStorage): boolean {
  try {
    return storage.getItem(`${RECAP_DISMISSED_PREFIX}${day}`) === "1"
  } catch {
    return false
  }
}

export function markRecapDismissed(day: string, storage: Pick<Storage, "setItem"> = localStorage): void {
  try {
    storage.setItem(`${RECAP_DISMISSED_PREFIX}${day}`, "1")
  } catch { /* ignore */ }
}

export function readNightOwlFlag(day: string, storage: Pick<Storage, "getItem"> = localStorage): boolean {
  try {
    return storage.getItem(`${NIGHT_OWL_PREFIX}${day}`) === "1"
  } catch {
    return false
  }
}

export function markNightOwlFlag(day: string, storage: Pick<Storage, "setItem"> = localStorage): void {
  try {
    storage.setItem(`${NIGHT_OWL_PREFIX}${day}`, "1")
  } catch { /* ignore */ }
}
