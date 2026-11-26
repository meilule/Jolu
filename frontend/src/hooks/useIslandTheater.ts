import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { api, isTauri, listenEvent, type IslandLinkageDto, type StatsOverviewDto } from "@/lib/api"
import { checkinApi, type CheckinOverview } from "@/lib/checkinApi"
import { CHECKIN_EDITIONS } from "@/lib/dailyCheckin"
import { EVENTS } from "@/lib/events"
import { gatewayApi } from "@/lib/gatewayApi"
import { providerName } from "@/lib/providerCatalog"
import { rangeToMillis, todayRange } from "@/lib/usageStats"
import type { BotShot } from "@/lib/botExpression"
import type { GatewayActivity } from "@/hooks/useGatewayActivity"
import {
  CACHE_SAVINGS_MIN_USD,
  estimateCacheSavingsUsd,
  HIGH_CONTEXT_RATIO,
  isLateNightHour,
  localDayKey,
  LONG_TASK_MS,
  makeChargeCue,
  makeCoffeeCue,
  makeConfettiCue,
  makeFlagCue,
  makeGiftCue,
  makePiggyCue,
  makeRelayCue,
  makeUtensilsCue,
  nextTokenMilestone,
  readSeenTheaterKeys,
  writeSeenTheaterKeys,
  type TheaterCue,
} from "@/lib/islandTheater"
import {
  ACHIEVEMENTS,
  clearAchievementStore,
  computeCheckinStreak,
  evaluateAchievements,
  formatRecapDate,
  isRecapDismissed,
  markNightOwlFlag,
  markRecapDismissed,
  readAchievementStore,
  readNightOwlFlag,
  shouldShowDailyRecap,
  writeAchievementStore,
  type AchievementStore,
  type RecapData,
} from "@/lib/islandRecap"

export interface IslandTheaterInput {
  linkage: Required<IslandLinkageDto>
  dnd: boolean
  platform: string
  platformName: string
  connectionName: string | null
  writing: boolean
  activeSessionId: string | null
  shot: BotShot | null
  inactiveMs: number
  todayTokens: number | null
  gateway: GatewayActivity | null
  /** 分身不重复弹今日回顾与晨间签到，只有本体弹 */
  isClone: boolean
}

/**
 * 灵动岛 R2-I-05（bot 道具与小剧场）与 R2-I-07（今日回顾与 6 枚本机成就）实时状态引擎。
 * 所有判断完全基于已有本地数据与事件，零额外模型调用、零随机假事件。
 */
export function useIslandTheater(input: IslandTheaterInput) {
  const {
    linkage,
    dnd,
    platform,
    platformName,
    connectionName,
    writing,
    activeSessionId,
    shot,
    inactiveMs,
    todayTokens,
    gateway,
    isClone,
  } = input

  const [cue, setCue] = useState<TheaterCue | null>(null)
  const cueTimer = useRef(0)
  const [checkinData, setCheckinData] = useState<CheckinOverview | null>(null)
  const [todayStats, setTodayStats] = useState<StatsOverviewDto | null>(null)
  const [todayGatewayReroutes, setTodayGatewayReroutes] = useState(0)
  const [todayGatewayErrors, setTodayGatewayErrors] = useState(0)
  const [todayGatewayRequests, setTodayGatewayRequests] = useState(0)
  const [achievements, setAchievements] = useState<AchievementStore>(() => readAchievementStore())
  const [recapOpen, setRecapOpen] = useState(false)
  const [recapDismissedDay, setRecapDismissedDay] = useState(() => isRecapDismissed(localDayKey()))
  const [clockTick, setClockTick] = useState(() => Date.now())

  const showCue = useCallback((next: TheaterCue) => {
    window.clearTimeout(cueTimer.current)
    setCue(next)
    if (next.durationMs != null) {
      cueTimer.current = window.setTimeout(() => {
        setCue((curr) => (curr?.id === next.id ? null : curr))
      }, next.durationMs)
    }
  }, [])

  const dismissCue = useCallback(() => {
    window.clearTimeout(cueTimer.current)
    setCue(null)
  }, [])

  useEffect(() => () => window.clearTimeout(cueTimer.current), [])

  // 每分钟更新一次时钟（深夜咖啡、跨日重置与 18:00 今日回顾判定）
  useEffect(() => {
    const timer = window.setInterval(() => setClockTick(Date.now()), 60_000)
    return () => window.clearInterval(timer)
  }, [])

  const nowDate = useMemo(() => new Date(clockTick), [clockTick])
  const dayKey = localDayKey(nowDate)
  const hour = nowDate.getHours()

  // 跨日同步 dismissed 标记
  useEffect(() => {
    setRecapDismissedDay(isRecapDismissed(dayKey))
  }, [dayKey])

  // 记录凌晨 2 点后仍在写（夜猫子成就）
  useEffect(() => {
    if (writing && hour >= 2 && hour < 5) {
      markNightOwlFlag(dayKey)
    }
  }, [writing, hour, dayKey])

  // ① 读取今日签到总览（本体启动时读一次，签到变化事件时刷新）
  useEffect(() => {
    if (!isTauri || isClone || (!linkage.props_theater && !linkage.daily_recap)) return
    let disposed = false
    const load = () => {
      void checkinApi.overview().then((ov) => { if (!disposed) setCheckinData(ov) }).catch(() => {})
    }
    load()
    const off = listenEvent(EVENTS.DAILY_CHECKIN_CHANGED, load)
    return () => { disposed = true; void off.then((un) => un()).catch(() => {}) }
  }, [isClone, linkage.props_theater, linkage.daily_recap])

  // ② 晨间签到 · 抱礼物（当天第一次发现有未签到账号时出现一次）
  useEffect(() => {
    if (!linkage.props_theater || dnd || isClone || !checkinData) return
    const manualAccounts = checkinData.accounts.filter(
      (a) => a.connection_status !== "paused" && CHECKIN_EDITIONS.some((e) => e.manual && e.platform === a.platform && e.edition === a.edition),
    )
    if (manualAccounts.length === 0) return
    const claimedTodayIds = new Set(
      checkinData.records
        .filter((r) => r.outcome.state === "claimed" && localDayKey(new Date(r.created_ms)) === dayKey)
        .map((r) => r.connection_id),
    )
    const unclaimedCount = manualAccounts.filter((a) => !claimedTodayIds.has(a.id)).length
    if (unclaimedCount <= 0) {
      setCue((curr) => (curr?.prop === "gift" ? null : curr))
      return
    }
    const seen = readSeenTheaterKeys(dayKey)
    if (seen.has("gift")) return
    seen.add("gift")
    writeSeenTheaterKeys(dayKey, seen)
    showCue(makeGiftCue(unclaimedCount))
  }, [checkinData, linkage.props_theater, dnd, isClone, dayKey, showCue])

  // ③ 额度重置 · 充能
  const prevShotRef = useRef<BotShot | null>(null)
  useEffect(() => {
    if (shot === "reset" && prevShotRef.current !== "reset" && linkage.props_theater && !dnd) {
      showCue(makeChargeCue(connectionName || platformName))
    }
    prevShotRef.current = shot
  }, [shot, linkage.props_theater, dnd, connectionName, platformName, showCue])

  // ④ 里程碑彩带（今日 Token 第一次越过 1M / 10M）
  useEffect(() => {
    if (!linkage.milestone_confetti || dnd || isClone) return
    const total = todayStats?.tokens.total ?? todayStats?.token_coverage.known_total ?? todayTokens
    const seen = readSeenTheaterKeys(dayKey)
    const milestone = nextTokenMilestone(total, seen)
    if (!milestone) return
    seen.add(milestone.key)
    writeSeenTheaterKeys(dayKey, seen)
    showCue(makeConfettiCue(milestone.key, milestone.label))
  }, [todayTokens, todayStats, linkage.milestone_confetti, dnd, isClone, dayKey, showCue])

  // ⑤ 长任务完成 · 举牌（writing 持续 ≥ 5 分钟后结束）
  const writingStartedAt = useRef<number | null>(null)
  const writingSessionRef = useRef<{ platform: string; sessionId: string | null; name: string } | null>(null)
  useEffect(() => {
    if (writing) {
      if (writingStartedAt.current === null) {
        writingStartedAt.current = Date.now()
      }
      writingSessionRef.current = { platform, sessionId: activeSessionId, name: platformName }
    } else if (writingStartedAt.current !== null) {
      const elapsed = Date.now() - writingStartedAt.current
      const meta = writingSessionRef.current
      writingStartedAt.current = null
      if (elapsed >= LONG_TASK_MS && linkage.props_theater && !dnd && meta) {
        showCue(makeFlagCue(meta.name, elapsed, meta.platform, meta.sessionId))
      }
    }
  }, [writing, platform, activeSessionId, platformName, linkage.props_theater, dnd, showCue])

  // ⑥ 网关上下文吃撑（≥ 80% 上下文窗口，仅在网关转发中低频巡检）
  useEffect(() => {
    if (!isTauri || isClone || !linkage.props_theater || dnd || !gateway?.relay) return
    let disposed = false
    const checkContext = () => {
      void gatewayApi.contextSessions().then((rows) => {
        if (disposed || !rows.length) return
        const seen = readSeenTheaterKeys(dayKey)
        const high = rows.find((r) => {
          const tokens = r.input_tokens ?? 0
          const win = r.window_tokens ?? 0
          return win > 0 && tokens / win >= HIGH_CONTEXT_RATIO && !seen.has(`ctx:${r.session_id}`)
        })
        if (!high || !high.window_tokens || !high.input_tokens) return
        seen.add(`ctx:${high.session_id}`)
        writeSeenTheaterKeys(dayKey, seen)
        showCue(makeUtensilsCue(high.input_tokens / high.window_tokens, high.session_platform, high.session_id))
      }).catch(() => {})
    }
    checkContext()
    const timer = window.setInterval(checkContext, 60_000)
    return () => { disposed = true; window.clearInterval(timer) }
  }, [isClone, linkage.props_theater, dnd, gateway?.relay, dayKey, showCue])

  // ⑦ 网关改道接力（同 requestId 从旧来源切到新来源）
  const lastRequestSource = useRef<{ requestId: number; source: string } | null>(null)
  useEffect(() => {
    const shown = gateway?.activity.shown
    if (!shown || !shown.source) return
    const prev = lastRequestSource.current
    if (prev && prev.requestId === shown.requestId && prev.source !== shown.source) {
      if (linkage.props_theater && !dnd) {
        showCue(makeRelayCue(prev.source, shown.source))
      }
    }
    lastRequestSource.current = { requestId: shown.requestId, source: shown.source }
  }, [gateway?.activity.shown?.requestId, gateway?.activity.shown?.source, linkage.props_theater, dnd, showCue])

  // ⑧ 定期汇总今日真实统计（供缓存省钱存钱罐、里程碑彩带、今日回顾与成就使用，每 2 分钟刷新一次）
  const refreshTodaySummary = useCallback(() => {
    if (!isTauri || isClone) return
    const range = rangeToMillis(todayRange(new Date()))
    void api.statsOverview({ ...range, platform: null, connections: [] }).then((ov) => {
      setTodayStats(ov)
      // 缓存省钱存钱罐：有真实 cache_read 与已计价模型时用保守 90% 折扣反推估算节省额
      if (linkage.props_theater && !dnd) {
        const cacheRead = ov.tokens.cache_read ?? ov.tokens.cache_read_known ?? 0
        const pricedCost = ov.cost.priced_requests > 0 ? ov.cost.amount : 0
        const totalTok = ov.tokens.total ?? ov.token_coverage.known_total ?? 0
        if (cacheRead > 0 && pricedCost > 0 && totalTok > 0) {
          const avgPerM = (pricedCost / totalTok) * 1_000_000
          const saved = estimateCacheSavingsUsd(cacheRead, avgPerM * 1.6, avgPerM * 0.16)
          if (saved !== null && saved >= CACHE_SAVINGS_MIN_USD) {
            const bucket = `piggy:${Math.floor(saved * 2)}`
            const seen = readSeenTheaterKeys(localDayKey())
            if (!seen.has(bucket)) {
              seen.add(bucket)
              writeSeenTheaterKeys(localDayKey(), seen)
              showCue(makePiggyCue(saved))
            }
          }
        }
      }
    }).catch(() => {})

    void gatewayApi.dayOverview("today").then((day) => {
      setTodayGatewayRequests(day.today)
      setTodayGatewayReroutes(day.reroutes)
      setTodayGatewayErrors(day.errors)
    }).catch(() => {})
  }, [isClone, linkage.props_theater, dnd, showCue])

  useEffect(() => {
    if (!isTauri || isClone || (!linkage.props_theater && !linkage.milestone_confetti && !linkage.daily_recap)) return
    refreshTodaySummary()
    const timer = window.setInterval(refreshTodaySummary, 120_000)
    return () => window.clearInterval(timer)
  }, [isClone, linkage.props_theater, linkage.milestone_confetti, linkage.daily_recap, refreshTodaySummary])

  // ⑨ 评估 6 枚本机成就（纯基于已有记录，不额外调用模型）
  const wiredAgentCount = useMemo(
    () => (gateway?.status?.wiring ?? []).filter((w) => w.wired).length,
    [gateway?.status?.wiring],
  )
  const checkinStreakDays = useMemo(() => {
    if (!checkinData) return 0
    const claimed = checkinData.records
      .filter((r) => r.outcome.state === "claimed")
      .map((r) => r.created_ms)
    return computeCheckinStreak(claimed, nowDate)
  }, [checkinData, nowDate])

  const evalResult = useMemo(() => {
    const cacheHitRate = todayStats?.cache_hit_rate ?? todayStats?.token_coverage.cache_hit_rate ?? null
    return evaluateAchievements(
      {
        checkinStreakDays,
        cacheHitRate,
        todayRequests: todayStats?.requests ?? 0,
        wiredAgentCount,
        todayRouteRequests: todayGatewayRequests,
        todayRouteFailures: todayGatewayErrors,
        wroteAfter2am: readNightOwlFlag(dayKey),
        todayReroutes: todayGatewayReroutes,
      },
      achievements,
      dayKey,
    )
  }, [
    checkinStreakDays,
    todayStats,
    wiredAgentCount,
    todayGatewayRequests,
    todayGatewayErrors,
    todayGatewayReroutes,
    achievements,
    dayKey,
  ])

  useEffect(() => {
    if (!evalResult.changed) return
    setAchievements(evalResult.nextStore)
    writeAchievementStore(evalResult.nextStore)
  }, [evalResult])

  // ⑩ 傍晚 18:00 后第一次空闲 30 分钟触发今日回顾卡（R2-I-07）
  useEffect(() => {
    if (isClone || recapOpen) return
    const trigger = shouldShowDailyRecap({
      enabled: linkage.daily_recap,
      dnd,
      hour,
      writing,
      inactiveMs,
      dismissedToday: recapDismissedDay,
    })
    if (trigger) {
      refreshTodaySummary()
      setRecapOpen(true)
    }
  }, [isClone, recapOpen, linkage.daily_recap, dnd, hour, writing, inactiveMs, recapDismissedDay, refreshTodaySummary])

  const dismissRecapToday = useCallback(() => {
    markRecapDismissed(dayKey)
    setRecapDismissedDay(true)
    setRecapOpen(false)
  }, [dayKey])

  const resetAchievements = useCallback(() => {
    clearAchievementStore()
    setAchievements({})
  }, [])

  // 深夜咖啡静默陪伴（无高优先一次性剧场时生效）
  const quietCoffee = useMemo(() => {
    if (!linkage.props_theater || !writing || !isLateNightHour(hour)) return null
    return makeCoffeeCue(platformName, nowDate)
  }, [linkage.props_theater, writing, hour, platformName, nowDate])

  const activeCue = cue ?? quietCoffee

  // 汇总 RecapCard 数据（浏览器预览允许用示例摘要展示卡片结构，桌面端完全用真实本机数据）
  const recapData = useMemo<RecapData>(() => {
    const manualAccounts = checkinData?.accounts.filter(
      (a) => a.connection_status !== "paused" && CHECKIN_EDITIONS.some((e) => e.manual && e.platform === a.platform && e.edition === a.edition),
    ) ?? []
    const claimedIds = new Set(
      (checkinData?.records ?? [])
        .filter((r) => r.outcome.state === "claimed" && localDayKey(new Date(r.created_ms)) === dayKey)
        .map((r) => r.connection_id),
    )
    const totalTok = todayStats ? (todayStats.tokens.total ?? todayStats.token_coverage.known_total) : (!isTauri ? 1_860_000 : todayTokens)
    const sessionCount = todayStats ? todayStats.sessions.length || todayStats.requests : (!isTauri ? 32 : null)
    const cacheRead = todayStats?.tokens.cache_read ?? todayStats?.tokens.cache_read_known ?? 0
    const pricedCost = todayStats && todayStats.cost.priced_requests > 0 ? todayStats.cost.amount : 0
    const avgPerM = totalTok && totalTok > 0 && pricedCost > 0 ? (pricedCost / totalTok) * 1_000_000 : null
    const cacheSavedUsd = !isTauri
      ? 4.1
      : avgPerM !== null
        ? estimateCacheSavingsUsd(cacheRead, avgPerM * 1.6, avgPerM * 0.16)
        : null

    let topTool: { name: string; sharePercent: number } | null = null
    if (todayStats && todayStats.providers.length > 0) {
      const sorted = [...todayStats.providers].sort((a, b) => (b.tokens ?? 0) - (a.tokens ?? 0))
      const sum = sorted.reduce((acc, p) => acc + (p.tokens ?? 0), 0)
      if (sum > 0 && sorted[0].tokens) {
        topTool = { name: providerName(sorted[0].platform), sharePercent: Math.round((sorted[0].tokens / sum) * 100) }
      }
    } else if (!isTauri) {
      topTool = { name: "Claude Code", sharePercent: 62 }
    }

    return {
      dateLabel: formatRecapDate(nowDate),
      todayTokens: totalTok,
      sessionCount,
      cacheSavedUsd,
      checkinClaimed: checkinData ? claimedIds.size : (!isTauri ? 3 : null),
      checkinTotal: checkinData ? manualAccounts.length : (!isTauri ? 3 : null),
      topTool,
      unlockedToday: evalResult.unlockedToday,
      allAchievements: ACHIEVEMENTS.map((def) => ({
        ...def,
        unlockedOn: evalResult.nextStore[def.id]?.unlockedOn ?? null,
      })),
    }
  }, [checkinData, todayStats, todayTokens, dayKey, nowDate, evalResult])

  return {
    activeCue,
    dismissCue,
    recapOpen,
    setRecapOpen,
    dismissRecapToday,
    recapData,
    resetAchievements,
    canViewRecap: hour >= 18 || !isTauri,
  }
}
