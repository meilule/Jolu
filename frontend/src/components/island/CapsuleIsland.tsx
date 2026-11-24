import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { EVENTS } from "@/lib/events"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { ZcodeBucketQuota } from "@/components/providers/ZcodeBucketQuota"
import { Bot, type BotRing } from "./Bot"
import { AnimatedTokens } from "./TokenDelta"
import { SessionList } from "./SessionList"
import { SourceFooter, type IslandData } from "./UsageIsland"
import { LazyConnectionSwitcher } from "./LazyConnectionSwitcher"
import { AgentStack, GatewaySection, RelayRoute, wiredAgents } from "./CapsuleGateway"
import { IslandContour, type IslandBubbleSpec } from "./IslandContour"
import { useBotShots } from "@/hooks/useBotShots"
import type { GatewayActivity } from "@/hooks/useGatewayActivity"
import { botExpression, BOT_NAMES, islandAmbientGlow, type BotExpr, type BotProp } from "@/lib/botExpression"
import { carriedDeltaSegment, type CarriedDeltaSource } from "@/lib/carriedDelta"
import { quotaRowsFor } from "@/lib/authQuotaRows"
import { capsuleQuotaChips, CHIP_NAME_MAX_WIDTH, compactQuotaNumber, type CapsuleChipWindow } from "@/lib/capsuleQuotaChips"
import { islandBucketQuotaView } from "@/lib/islandQuotaBuckets"
import { quotaRings } from "@/lib/quotaRings"
import { edgeCornerRadii } from "@/lib/squircle"
import { api, compactTokens, isTauri, listenEvent, type IslandHitRectDto } from "@/lib/api"
import { sourceBreakdownView } from "@/lib/islandPresentation"
import { useLiveCount } from "@/lib/liveCounts"
import { islandIdFromSearch } from "@/lib/islandProfile"
import { reducedMotion } from "@/lib/motion"
import { windowChipLabel, windowRingColor, windowShortLabel } from "@/lib/providers/registry"
import { cn } from "@/lib/utils"
import type { PlatformId, QuotaWindow } from "@/types"
import "./capsule.css"

/**
 * 胶囊灵动岛 R2（docs/specs/2026-11-24-island-rendering-tech.md）：
 * - C 硬件材质：实体深黑表面 + 1px 内描边 + 顶部内高光 + 两层真实阴影 + 底部状态光晕；
 * - B 连续轮廓：一条由 `SPRING.island` 驱动的超椭圆路径 + SVG Goo 液体分离气泡；
 * - 四边统一「从屏幕边缘长出来」：贴哪条边，靠边两角平滑变直角，悬停 400ms 沿同路径长出。
 */
const CAP_H = 40
const EDGE_TAB_H = 36
const EXPANDED_W = 380

type CapsuleView = "card" | "capsule" | "edgeTab"

const windowLabel = (q: QuotaWindow) => windowShortLabel(q.key) ?? q.windowName
const resetText = (q: QuotaWindow) => q.resetCountdown?.replace(/^还剩\s*/, "") ?? null

function ringColor(q: QuotaWindow, platform: PlatformId): string {
  const p = q.usedPercent
  if (p !== null && p >= 100) return "var(--pn-capsule-red)"
  if (p !== null && p >= 80) return "var(--pn-capsule-orange)"
  return windowRingColor(q.key, platform)
}

function LiveTokens({ countKey, tokens, text, source, unknown = false }: {
  countKey?: string
  tokens: number | null
  text: string | null
  source: CarriedDeltaSource | null
  unknown?: boolean
}) {
  const animated = useLiveCount(countKey)
  const value = animated ?? tokens
  const name = source === "session" ? "本轮写入" : "今日累计写入"
  const compact = value == null ? null : `+${compactQuotaNumber(value)}`
  const title = value == null
    ? unknown
      ? "本轮正在写入，读数未知（该来源不返回本轮 Token，也没有今日累计可兜底）"
      : `${name}暂无数据`
    : source === "session"
      ? `${name} ${compact}（本轮累计 ${value.toLocaleString("zh-CN")} Token；会话结束后停在这个读数，不再增长）`
      : `${name} ${compact}（今日已落库 ${value.toLocaleString("zh-CN")} Token，含今天所有会话）`
  return (
    <span
      className="cap-live cap-num cap-mono"
      data-token-value={value ?? undefined}
      data-carried-source={source ?? undefined}
      data-carried-unknown={unknown ? "true" : undefined}
      aria-label={title}
      title={title}
    >
      {compact ?? text ?? (unknown ? "+—" : "—")}
    </span>
  )
}

export function CapsuleIsland({
  data,
  open,
  mini = false,
  onOpenChange,
  connectionSwitcher,
  connectionMenuOpen = false,
  dragging = false,
  refreshing = false,
  dockEdge = null,
  visualDockEdge,
  gateway = null,
  zoom = 1,
  reduced = false,
  onHoverChange,
  onHitRects,
  previewExpr = null,
  previewProp = "none",
  previewBubble = false,
}: {
  data: IslandData
  gateway?: GatewayActivity | null
  /** 贴边收起态（四边统一「从屏幕边缘长出来」）：只露出 bot + 额度环 */
  mini?: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
  connectionSwitcher?: ReactNode
  connectionMenuOpen?: boolean
  dragging?: boolean
  refreshing?: boolean
  /** 逻辑停靠边 */
  dockEdge?: "top" | "bottom" | "left" | "right" | null
  /** 视觉停靠边（拖动离边时提前为 null，让直角平滑变回圆角） */
  visualDockEdge?: "top" | "bottom" | "left" | "right" | null
  zoom?: number
  reduced?: boolean
  onHoverChange?: (over: boolean) => void
  /** 向舞台窗口上报当前轮廓与气泡的窗口内命中矩形 */
  onHitRects?: (rects: IslandHitRectDto[], immediate?: boolean) => void
  previewExpr?: BotExpr | null
  previewProp?: BotProp
  /** 浏览器预览开关：强制展示液体分离气泡以便验收 goo 形变 */
  previewBubble?: boolean
}) {
  const showQuotas = data.kind === "auth" && !data.unavailable
  const quotaRows = useMemo(() => quotaRowsFor(data.platform, data.quotas).slice(0, 2), [data.platform, data.quotas])
  const follow = data.relayQuota ?? null
  const ringPlatform = follow?.platform ?? data.platform
  const ringWindows = useMemo(
    () => (follow ? quotaRings(follow.platform, follow.windows)
      : data.kind === "auth" && !data.unavailable ? quotaRings(data.platform, data.quotas) : []),
    [follow, data.kind, data.unavailable, data.platform, data.quotas],
  )
  const ringsFor = (aria: boolean): BotRing[] => ringWindows.map((ring) => {
    const q = ring.window
    const reset = resetText(q)
    return {
      p: q.usedPercent,
      geometry: ring.geometry,
      color: ringColor(q, ringPlatform),
      warn: q.usedPercent !== null && q.usedPercent >= 80,
      aria: aria ? `${follow ? `正在调用「${follow.name}」· ` : ""}${windowLabel(q)}额度${q.usedPercent === null ? "未知" : `已用 ${Math.round(q.usedPercent)}%`}${reset ? `，${reset} 后重置` : ""}。按 Enter 固定显示` : undefined,
    }
  })
  const quotaUsed = useMemo(() => (showQuotas ? Object.fromEntries(quotaRows.map((q) => [q.key, q.usedPercent])) : {}), [showQuotas, quotaRows])

  const chipWindows = useMemo<CapsuleChipWindow[]>(() => {
    const windows = data.quotas ?? []
    if (windows.length === 0) return []
    const named = windows.some((w) => windowChipLabel(w.key) !== null)
    const source = named ? quotaRows : windows
    return source.map((w) => ({
      key: w.key,
      name: windowChipLabel(w.key) ?? w.windowName,
      usedPercent: w.usedPercent,
      amountText: w.usedText,
      remainingText: w.remainingText ?? null,
      reset: resetText(w),
    }))
  }, [data.quotas, quotaRows])
  const { chips: quotaChipList, more: quotaChipMore } = useMemo(() => capsuleQuotaChips(chipWindows), [chipWindows])
  const bucketView = useMemo(
    () => islandBucketQuotaView({
      platform: data.platform,
      kind: data.kind,
      windows: data.quotas,
      queryFailed: data.queryFailed,
      unavailable: Boolean(data.unavailable),
    }),
    [data.platform, data.kind, data.quotas, data.queryFailed, data.unavailable],
  )

  const running = data.sessions?.filter((s) => s.state === "running").length ?? 0
  const working = !data.sessionStatusUnknown && running > 0
  const writing = data.working ?? working
  const showDelta = data.deltaPhase !== undefined && data.deltaPhase !== "idle"
  const { shot, inactiveMs } = useBotShots({
    deltaPhase: data.deltaPhase, open, quotaUsed, active: writing || showDelta,
    identity: `${data.platform}:${data.kind}:${data.connectionId ?? ""}`,
  })

  const relayOn = Boolean(gateway?.relay)
  const relay = relayOn ? gateway!.activity.shown : null
  const agents = useMemo(() => (relayOn ? wiredAgents(gateway!.status, gateway!.activity) : []), [relayOn, gateway])
  const drifted = agents.filter((w) => w.drift).length
  const segment = carriedDeltaSegment({
    value: data.carriedDelta ?? data.deltaTokens ?? null,
    source: data.carriedDelta != null ? data.carriedSource ?? null : null,
    text: data.deltaText ?? null,
    writing,
    unavailable: Boolean(data.unavailable),
    relay: Boolean(relay),
  })

  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!relayOn) return
    setNow(Date.now())
    const timer = window.setInterval(() => setNow(Date.now()), 30_000)
    const dot = relay?.phase === "ok" ? window.setTimeout(() => setNow(Date.now()), 10_100) : 0
    return () => { window.clearInterval(timer); window.clearTimeout(dot) }
  }, [relayOn, relay?.requestId, relay?.phase])

  const [hover, setHover] = useState(false)
  const [focusRing, setFocusRing] = useState<number | null>(null)
  const [pinRing, setPinRing] = useState<number | null>(null)
  const derived = botExpression({
    connected: data.connected ?? data.status?.tone === "success",
    queryFailed: showQuotas && Boolean(data.queryFailed),
    quotaUsed: quotaRows.map((q) => q.usedPercent),
    dragging, waiting: Boolean(data.automationWaiting), refreshing, working: writing, shot, inactiveMs: relay ? 0 : inactiveMs,
    docked: Boolean(dockEdge) && !open, hover,
    relay: relay ? (relay.phase === "failed" ? "rejected" : relay.phase) : null,
  })
  const expr = previewExpr ?? derived.expr
  const reason = previewExpr ? `表情库预览：${BOT_NAMES[previewExpr]}` : derived.reason
  const isReduced = reduced || reducedMotion()
  const glow = islandAmbientGlow(expr, data.writingLevel)
  const ticker = writing && showDelta && data.deltaCountKey ? { countKey: data.deltaCountKey, label: "本段 Token", sub: `${running} 个会话写入中` } : null

  const focused = pinRing ?? focusRing
  const focusWindow = focused !== null ? ringWindows[focused]?.window : undefined
  const display: [string, string, string?] | null = focusWindow && focusWindow.usedPercent !== null
    ? [`${Math.round(focusWindow.usedPercent)}%`, windowLabel(focusWindow), resetText(focusWindow) ? `${resetText(focusWindow)} 后重置` : undefined]
    : null

  // ---------- 连续轮廓尺寸与四角圆角计算（§4 / §5） ----------
  const activeVisualEdge = visualDockEdge !== undefined ? visualDockEdge : dockEdge
  const view: CapsuleView = open ? "card" : mini ? "edgeTab" : "capsule"
  const mainRef = useRef<HTMLSpanElement>(null)
  const exRef = useRef<HTMLDivElement>(null)
  const [mainW, setMainW] = useState(0)
  const [exH, setExH] = useState(0)

  useLayoutEffect(() => {
    const main = mainRef.current, ex = exRef.current
    if (!main || !ex) return
    const measure = () => {
      setMainW(Math.ceil(main.getBoundingClientRect().width))
      setExH(Math.ceil(ex.offsetHeight))
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(main)
    observer.observe(ex)
    return () => observer.disconnect()
  }, [])

  // 舞台窗口固定高度上限（保留投影边距），卡片超出时改为内部滚动
  const stageMaxH = Math.max(CAP_H, ((isTauri ? window.innerHeight : 660) - 72) / (zoom || 1))
  const fullCardH = exH + 2
  const cardH = Math.max(CAP_H, Math.min(fullCardH, stageMaxH))
  const scroll = view === "card" && fullCardH > stageMaxH

  // 完整胶囊宽 = 左 4 + bot 32 + 间距 10 + 内容 + 右 14
  const capsuleW = Math.max(48, mainW + 60)
  // 四边统一的「贴边长出来」收起态：只露 bot 28 + 边距（写入中加 6px 脉冲点）
  const edgeTabW = writing ? 52 : 40
  const shellW = view === "card" ? EXPANDED_W : view === "edgeTab" ? edgeTabW : capsuleW
  const shellH = view === "card" ? cardH : view === "edgeTab" ? EDGE_TAB_H : CAP_H
  const baseRadius = view === "card" ? 28 : view === "edgeTab" ? 18 : 20
  const targetRadii = edgeCornerRadii(baseRadius, activeVisualEdge)

  // ---------- 交互 ----------
  const collapse = useCallback(() => { setPinRing(null); setFocusRing(null); onOpenChange(false) }, [onOpenChange])
  useEffect(() => { if (!open) { setPinRing(null); setFocusRing(null) } }, [open])
  const leaveTimer = useRef(0)
  const onRingHover = (index: number | null) => {
    window.clearTimeout(leaveTimer.current)
    if (index !== null) setFocusRing(index)
    else leaveTimer.current = window.setTimeout(() => setFocusRing(null), 300)
  }
  useEffect(() => () => window.clearTimeout(leaveTimer.current), [])
  const togglePin = (index: number) => setPinRing((current) => (current === index ? null : index))

  useEffect(() => {
    if (!open) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || connectionMenuOpen) return
      if (pinRing !== null) setPinRing(null)
      else collapse()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [open, pinRing, collapse, connectionMenuOpen])

  useEffect(() => {
    if (!open || connectionMenuOpen) return
    const onDown = (event: PointerEvent) => {
      const target = event.target as HTMLElement | null
      if (!target) return
      if (target.closest("[data-capsule-island]")) return
      if (target.closest("[role=menu], [role=listbox], [role=dialog], [data-radix-popper-content-wrapper], .panel")) return
      collapse()
    }
    document.addEventListener("pointerdown", onDown, true)
    return () => document.removeEventListener("pointerdown", onDown, true)
  }, [open, connectionMenuOpen, collapse])

  const dragRef = useRef(dragging)
  dragRef.current = dragging

  useEffect(() => {
    if (!open || connectionMenuOpen) return
    let menuAt = 0
    let disposed = false
    let off: (() => void) | null = null
    const onMenu = () => { menuAt = Date.now() }
    const onBlur = () => { if (!dragRef.current && Date.now() - menuAt > 800) collapse() }
    window.addEventListener("island-menu-open", onMenu)
    window.addEventListener("blur", onBlur)
    const self = islandIdFromSearch(window.location.search)
    void listenEvent(EVENTS.ISLAND_BLUR, (id) => { if ((id ?? null) === self) onBlur() }).then((un) => { if (disposed) un(); else off = un }).catch(() => {})
    return () => {
      disposed = true
      off?.()
      window.removeEventListener("island-menu-open", onMenu)
      window.removeEventListener("blur", onBlur)
    }
  }, [open, connectionMenuOpen, collapse])

  // ---------- 液体分离气泡（§4.4 / R2-I-01）：多会话 / 自动化等额度 / 接线偏移 ----------
  const bubbleSpec = useMemo<IslandBubbleSpec | null>(() => {
    if (view !== "capsule") return { visible: false, width: 40, side: dockEdge === "right" ? "left" : "right", children: null }
    const side: "left" | "right" = dockEdge === "right" ? "left" : "right"
    if (data.automationWaiting) {
      return {
        visible: true,
        width: 82,
        side,
        children: (
          <span className="flex items-center gap-1.5" role="status" aria-label="自动化任务等待额度恢复">
            <span className="cap-wdot" />
            <span>等额度</span>
          </span>
        ),
      }
    }
    if (writing && running >= 2) {
      return {
        visible: true,
        width: 40,
        side,
        children: (
          <span role="status" aria-label={`${running} 个会话运行中`}>
            <span className="cap-spinner" />
            <b className="cap-num">{running}</b>
          </span>
        ),
      }
    }
    if (drifted > 0) {
      return {
        visible: true,
        width: 118,
        side,
        children: (
          <span className="flex items-center gap-1.5" role="status">
            <span className="cap-wdot" />
            <span>{drifted} 个接线需处理</span>
          </span>
        ),
      }
    }
    if (previewBubble) {
      return {
        visible: true,
        width: 40,
        side,
        children: (
          <span role="status" aria-label="2 个会话运行中（预览）">
            <span className="cap-spinner" />
            <b className="cap-num">2</b>
          </span>
        ),
      }
    }
    return { visible: false, width: 40, side, children: null }
  }, [view, dockEdge, data.automationWaiting, writing, running, drifted, previewBubble])

  // ---------- 内容装配 ----------
  const attributed = data.activeConnection ?? null
  const plat = (
    <span
      className={cn("cap-plat", attributed?.confidence === "mapped" && "cap-dim")}
      data-cap-attribution={attributed?.confidence ?? "platform"}
      title={attributed ? `${data.platformName} · ${attributed.name}` : undefined}
    >
      <PlatformLogo platform={data.platform} size={13} />
      {attributed ? attributed.name : data.platformName}
    </span>
  )
  const quotaChips = quotaChipList.length > 0
    ? (
      <>
        {quotaChipList.map((chip) => (
          <span key={chip.key} className="cap-q" title={chip.title}>
            <i style={{ maxWidth: CHIP_NAME_MAX_WIDTH, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{chip.name}</i>
            <b className={cn("cap-num", chip.tone === "warn" && "cap-o", chip.tone === "danger" && "cap-r")}>{chip.value}</b>
          </span>
        ))}
        {quotaChipMore > 0 && (
          <span className="cap-t3" title={`另有 ${quotaChipMore} 个额度窗口，展开卡片查看`}>+{quotaChipMore}</span>
        )}
      </>
    )
    : null
  const dashOnly = <span className="cap-num cap-t3 text-[13px]">—</span>

  const collapsedMain = relay ? (
    <RelayRoute relay={relay} />
  ) : data.unavailable ? (
    <>
      <span className="cap-plat cap-dim"><PlatformLogo platform={data.platform} size={13} />{data.platformName}</span>
      <span className="cap-wdot" /><span className="cap-t2">{data.unavailable.title} · 双击查看</span>
    </>
  ) : data.queryFailed && showQuotas ? (
    <>{plat}<span className="cap-r text-xs font-semibold">查询失败 · 双击查看</span></>
  ) : showQuotas ? (
    <>{plat}{quotaChipList.length > 0 ? quotaChips : dashOnly}</>
  ) : data.localMetrics ? (
    <>{plat}<span className="cap-q"><i>本机今日</i><b className="cap-num">{data.localMetrics.token ?? "—"}</b></span></>
  ) : (
    <>
      {plat}
      <span className="cap-chip cap-b">API</span>
      <b className="cap-num cap-mono">{data.apiToday?.token ?? "—"}</b>
      <span className="cap-t3">Token</span>
      {data.apiToday?.cost && <><span className="cap-hair" /><b className={cn("cap-num", data.apiToday.costEstimated && "cap-o")}>{data.apiToday.cost}</b></>}
    </>
  )

  const statusColor = data.unavailable ? "var(--pn-capsule-orange)" : showQuotas && data.queryFailed ? "var(--pn-capsule-red)"
    : refreshing ? "var(--pn-capsule-cyan)" : data.status?.tone === "danger" ? "var(--pn-capsule-red)"
    : data.status?.tone === "warn" ? "var(--pn-capsule-orange)" : "var(--pn-capsule-green)"
  const statusText = data.unavailable ? data.unavailable.title : showQuotas && data.queryFailed ? "额度查询失败 · 请刷新重试"
    : refreshing ? "正在刷新…" : data.status?.label ?? "—"

  const legend = data.unavailable ? (
    <p className="cap-note">{data.unavailable.hint}</p>
  ) : showQuotas ? (
    ringWindows.map((ring, index) => {
      const q = ring.window
      const on = focused === index
      const reset = resetText(q)
      return (
        <button
          key={q.key}
          type="button"
          className={cn("cap-lg", on && "on", pinRing === index && "pin")}
          aria-pressed={pinRing === index}
          data-cap-ring={q.key}
          onPointerEnter={() => onRingHover(index)}
          onPointerLeave={() => onRingHover(null)}
          onFocus={() => onRingHover(index)}
          onBlur={() => onRingHover(null)}
          onClick={() => togglePin(index)}
          onDoubleClick={(event) => event.stopPropagation()}
        >
          <span className="cap-sw" style={{ background: ringColor(q, data.platform) }} />
          <span className="cap-t">
            <span className="cap-w">{windowLabel(q)} · {index === 0 ? "外圈" : "内圈"}</span>
            <span className="cap-p cap-num">{q.usedPercent === null ? "—" : Math.round(q.usedPercent)}<small>{q.usedPercent === null ? "" : "%"}</small></span>
            <span className="cap-r2">{reset ? `${reset} 后重置` : "未提供重置时间"}</span>
          </span>
        </button>
      )
    })
  ) : bucketView ? (
    <ZcodeBucketQuota view={bucketView} plan={null} size="island" columns={1} />
  ) : data.localMetrics ? (
    <>
      <div className="cap-met"><span>本机今日 Token</span><b className="cap-num">{data.localMetrics.token ?? "—"}</b></div>
      <div className="cap-met"><span>今日上报积分</span><b className="cap-num">{data.localMetrics.credits ?? "—"}</b></div>
    </>
  ) : (
    <>
      <div className="cap-met"><span>{data.apiToday?.tokenLabel ?? "今日 Token"}</span><b className="cap-num">{data.apiToday?.token ?? "—"}</b></div>
      <div className="cap-met"><span>{data.apiToday?.costEstimated ? "本机费用（估算）" : "今日费用"}</span><b className={cn("cap-num", data.apiToday?.costEstimated && "cap-o")}>{data.apiToday?.cost ?? "—"}</b></div>
    </>
  )

  const notes = [
    showQuotas && data.quotaMessage,
    data.localMetrics?.message,
    !showQuotas && !data.localMetrics && data.apiMessage,
    !showQuotas && !data.localMetrics && data.connectionLabel && `连接：${data.connectionLabel}`,
    data.balanceText && `网关余额 ${data.balanceText}${data.balanceTimeText ? `（${data.balanceTimeText}）` : ""}`,
    data.todayTokenNote,
  ].filter(Boolean) as string[]

  const sessionTotal = data.sessions?.length ?? 0
  const breakdown = sourceBreakdownView(data.sourceBreakdown ?? null, compactTokens)
  const sectionTitle = sessionTotal > 0 ? (data.sessionSectionTitle ?? `运行中的会话 · ${sessionTotal}`) : "本轮 Token"

  return (
    <IslandContour
      width={shellW}
      height={shellH}
      radii={targetRadii}
      dockEdge={activeVisualEdge}
      open={open}
      edgeTab={view === "edgeTab"}
      dragging={dragging}
      scroll={scroll}
      glow={glow}
      reduced={isReduced}
      ariaLabel={`${data.platformName} 灵动岛`}
      title={reason}
      expr={expr}
      bubble={bubbleSpec}
      onHitRects={onHitRects}
      onDoubleClick={(event) => {
        if (!open) { onOpenChange(true); return }
        if ((event.target as HTMLElement).closest("button, [role=button], [role=combobox], input, a")) return
        collapse()
      }}
      onPointerEnter={() => { setHover(true); onHoverChange?.(true) }}
      onPointerLeave={() => { setHover(false); onHoverChange?.(false) }}
    >
      {/* 收缩 / 贴边长出层（常驻，不卸载） */}
      <div
        className="cap-layer cap-cl cap-toggle"
        role="button"
        tabIndex={open ? -1 : 0}
        aria-expanded={false}
        aria-label={`${data.platformName}${attributed ? `（${attributed.name}）` : ""}，${BOT_NAMES[expr]}。${view === "edgeTab" ? "悬停 400ms 探出，双击打开卡片" : "按 Enter 展开"}`}
        inert={open}
        onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); onOpenChange(true) } }}
      >
        <Bot
          certainty={data.writingLevel}
          size={view === "edgeTab" ? 28 : 32}
          ringWidth={view === "edgeTab" ? 2 : 2}
          ringGap={view === "edgeTab" ? 1.2 : 1.5}
          inset={view === "edgeTab" ? 2 : 3}
          rings={ringsFor(false)}
          expr={expr}
          prop={view === "edgeTab" ? "none" : previewProp}
          shot={shot}
          reduced={isReduced}
          pokeable={!dragging}
          dockEdge={dockEdge}
          ticker={open || view === "edgeTab" || relay ? null : ticker}
        />
        {view === "edgeTab" && writing && <span className="cap-pulse" />}
        <span ref={mainRef} className="cap-main">
          {collapsedMain}
          {segment.visible && (
            <>
              <span className="cap-hair" />
              <LiveTokens
                countKey={writing ? data.carriedCountKey : undefined}
                tokens={segment.value}
                text={data.deltaText ?? null}
                source={segment.source}
                unknown={segment.unknown}
              />
            </>
          )}
          {relayOn && agents.length > 0 && <>{!relay && <span className="cap-hair" />}<AgentStack agents={agents} activity={gateway!.activity} now={now} /></>}
        </span>
        {relay?.phase === "in" && <span className="cap-flow" />}
      </div>

      {/* 展开卡片层（常驻，轮廓先动 80ms 后轻模糊淡入） */}
      <div ref={exRef} className="cap-layer cap-ex" inert={!open} style={connectionMenuOpen ? { minHeight: 240 } : undefined}>
        <div className="cap-sec cap-hd">
          <div className="cap-id">
            <div className="cap-nm">
              <PlatformLogo platform={data.platform} size={15} />
              {data.platformName}
              <span className={cn("cap-chip", data.localMetrics ? "" : data.kind === "auth" ? "cap-g" : "cap-b")}>
                {data.localMetrics ? "本机" : data.kind === "auth" ? "Auth" : "API"}
              </span>
            </div>
            <div className="cap-st"><i style={{ background: statusColor }} /><span className="truncate">{statusText}</span></div>
          </div>
          {open && (connectionSwitcher ?? <LazyConnectionSwitcher />)}
        </div>

        <div className="cap-sec cap-hero">
          <div className="cap-orb-l">
            <Bot
              certainty={data.writingLevel}
              size={140}
              ringWidth={9}
              ringGap={5}
              inset={28}
              rings={ringsFor(true)}
              expr={expr}
              prop={previewProp}
              shot={shot}
              reduced={isReduced}
              pokeable={!dragging}
              dockEdge={dockEdge}
              ticker={open ? ticker : null}
              focus={focused}
              display={display}
              interactive={showQuotas}
              onRingHover={onRingHover}
              onRingClick={togglePin}
              pinnedRing={pinRing}
            />
          </div>
          <div className="cap-legend">{legend}</div>
        </div>

        {(sessionTotal > 0 || showDelta) && (
          <div className="cap-sec cap-col">
            <div className="cap-sh">
              <span>{sectionTitle}</span>
              <em>
                <AnimatedTokens
                  tokens={writing ? (data.liveDelta ?? data.deltaTokens) : segment.value}
                  text={data.deltaText}
                  countKey={writing ? data.deltaCountKey : undefined}
                  className="!text-[11px] !text-[var(--pn-capsule-green)]"
                />
              </em>
            </div>
            {sessionTotal > 0 && (
              <SessionList
                sessions={data.sessions!}
                showUnknownTag={data.sessionStatusUnknown}
                showSource
                onOpen={isTauri ? (session) => void api.openMainSession(data.platform, session.id).catch(() => {}) : undefined}
              />
            )}
          </div>
        )}

        {breakdown && (
          <div className="cap-sec cap-col">
            <div className="cap-sh">
              <span>今日按来源</span>
              {breakdown.coverage && <em className="cap-src-cov">{breakdown.coverage}</em>}
            </div>
            <div className="cap-src">
              {breakdown.rows.map((row) => (
                <div key={row.id} className="cap-src-row"><span title={row.name}>{row.name}</span><b className="cap-num">{row.value}</b></div>
              ))}
              {breakdown.more > 0 && <div className="cap-src-row cap-src-more"><span>另有 {breakdown.more} 个来源</span></div>}
            </div>
          </div>
        )}

        {relayOn && <GatewaySection status={gateway!.status!} activity={gateway!.activity} agents={agents} now={now} />}

        <div className="cap-sec cap-foot">
          {data.todayTokenText && showQuotas && (
            <div className="cap-row">
              <span className="cap-l">本地今日 Token</span>
              <span className="cap-v cap-num" title={data.todayTokenNote ?? undefined}>{data.todayTokenText}</span>
            </div>
          )}
          {notes.map((text) => <p key={text} className="cap-note">{text}</p>)}
          {data.sourceText && <SourceFooter text={data.sourceText} />}
          {isTauri && (
            <div className="cap-lnk">
              <span>今日用量明细</span>
              <button type="button" onClick={() => void api.openMainUsage(data.platform, data.usageConnectionId ?? null).catch(() => {})}>
                在主窗口查看
              </button>
            </div>
          )}
        </div>
      </div>
    </IslandContour>
  )
}
