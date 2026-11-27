import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { EVENTS } from "@/lib/events"
import { isTauri, listenEvent } from "@/lib/api"
import { gatewayApi, type SessionsOverviewDto } from "@/lib/gatewayApi"
import {
  applyLiveEvent, buildSessionViews, emptyLive, liveBySession, liveFromSnapshot, liveUnattributed, nextPriorityExpiry, sessionKey,
  updatePriority, type LiveEvent, type LiveState, type PriorityState, type SessionView, type SourceLive, type TokenFreshness,
} from "@/lib/gatewaySessions"

/** 定时刷新会话与 Token；超过 STALE_MS 没有成功刷新时标为过期 */
const REFRESH_MS = 15_000
const STALE_MS = 60_000
/** 快照返回前到达的事件缓存条数 */
const BUFFER_LIMIT = 300

export interface GatewaySessionsState {
  /** 首次读取尚未返回 */
  loading: boolean
  /** 没有任何成功快照时的错误；有过成功快照后失败只体现在 freshness */
  error: string | null
  freshness: TokenFreshness
  views: SessionView[]
  /** 当前在途的路由请求总数（含无会话标识的请求） */
  inFlightCount: number
  /** 没有会话标识的在途请求，按来源 id 汇总（来源卡显示运行中） */
  sourceLive: Map<string, SourceLive>
  /** 快照里是否已有数据（用于区分「真实为空」与「还没读到」） */
  loaded: boolean
  refresh: () => void
}

/**
 * 统一网关「最近会话」数据：先订阅实时开始 / 结束事件，再读快照；快照只吸收序号更大的事件。
 * 遇到快照里没有的会话或请求结束时，延迟刷新一次，补上标题、项目与 Token。
 */
export function useGatewaySessions(): GatewaySessionsState {
  const [overview, setOverview] = useState<SessionsOverviewDto | null>(null)
  const [live, setLive] = useState<LiveState>(emptyLive)
  const [priority, setPriority] = useState<PriorityState>({})
  const [error, setError] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)
  const [stale, setStale] = useState(false)
  const [loading, setLoading] = useState(isTauri)
  const buffer = useRef<LiveEvent[]>([])
  const pending = useRef(false)
  const again = useRef(false)
  const lastOk = useRef(0)
  const known = useRef(new Set<string>())
  const followUp = useRef<number | null>(null)
  const followUpAt = useRef<number>(0)

  const load = useCallback(async () => {
    if (!isTauri) return
    if (pending.current) { again.current = true; return }
    pending.current = true
    try {
      const next = await gatewayApi.sessionsOverview()
      buffer.current = buffer.current.filter((item) => typeof item.event.seq !== "number" || item.event.seq > next.seq)
      known.current = new Set(next.sessions.map((s) => sessionKey(s.platform, s.session_id)))
      setOverview(next)
      setLive(liveFromSnapshot(next.seq, next.in_flight, buffer.current))
      setError(null)
      setFailed(false)
      setStale(false)
      lastOk.current = Date.now()
    } catch (reason) {
      setFailed(true)
      setError(String(reason))
    } finally {
      pending.current = false
      setLoading(false)
      if (again.current) { again.current = false; void load() }
    }
  }, [])

  const scheduleFollowUp = useCallback((delay: number) => {
    const due = Date.now() + delay
    if (followUp.current && followUpAt.current <= due) return
    if (followUp.current) window.clearTimeout(followUp.current)
    followUpAt.current = due
    followUp.current = window.setTimeout(() => { followUp.current = null; followUpAt.current = 0; void load() }, delay)
  }, [load])

  useEffect(() => {
    if (!isTauri) return
    let stopped = false
    const offs: Array<() => void> = []
    const listen = (promise: Promise<() => void>) => void promise.then((off) => { if (stopped) off(); else offs.push(off) }).catch(() => {})
    const push = (item: LiveEvent) => {
      buffer.current = [...buffer.current, item].slice(-BUFFER_LIMIT)
      setLive((state) => applyLiveEvent(state, item))
    }
    listen(listenEvent(EVENTS.GATEWAY_ROUTE_START, (event) => {
      push({ kind: "start", event })
      // 新会话第一次出现：尽快补全标题、项目与 Token
      if (event.session_platform && event.session_id && !known.current.has(sessionKey(event.session_platform, event.session_id))) scheduleFollowUp(400)
    }))
    listen(listenEvent(EVENTS.GATEWAY_ROUTE, (event) => {
      push({ kind: "end", event })
      if (event.session_id) scheduleFollowUp(800)
    }))
    void load()
    const timer = window.setInterval(() => {
      if (document.hidden) return
      void load()
      if (lastOk.current && Date.now() - lastOk.current > STALE_MS) setStale(true)
    }, REFRESH_MS)
    const onVisible = () => { if (!document.hidden) void load() }
    document.addEventListener("visibilitychange", onVisible)
    return () => {
      stopped = true
      offs.forEach((off) => off())
      window.clearInterval(timer)
      document.removeEventListener("visibilitychange", onVisible)
      if (followUp.current) window.clearTimeout(followUp.current)
    }
  }, [load, scheduleFollowUp])

  const liveMap = useMemo(() => liveBySession(live), [live])
  const sourceLive = useMemo(() => liveUnattributed(live), [live])

  // 运行优先：进入运行记下时刻，结束后保留一小段时间再回到按活动排序
  useEffect(() => {
    setPriority((prev) => updatePriority(prev, [...liveMap.entries()].filter(([, v]) => v.count > 0).map(([k]) => k), Date.now()))
  }, [liveMap])
  const expiry = nextPriorityExpiry(priority)
  useEffect(() => {
    if (expiry === null) return
    const timer = window.setTimeout(() => {
      setPriority((prev) => updatePriority(prev, [...liveMap.keys()], Date.now()))
    }, Math.max(0, expiry - Date.now()) + 16)
    return () => window.clearTimeout(timer)
  }, [expiry, liveMap])

  const views = useMemo(() => buildSessionViews(overview?.sessions ?? [], liveMap, priority), [overview, liveMap, priority])
  const inFlightCount = useMemo(() => Object.keys(live.requests).length, [live])

  return {
    loading,
    error: overview ? null : error,
    freshness: failed ? "failed" : stale ? "stale" : "fresh",
    views,
    inFlightCount,
    sourceLive,
    loaded: overview !== null,
    refresh: () => void load(),
  }
}
