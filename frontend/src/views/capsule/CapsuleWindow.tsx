import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { FollowingBar, SignalRing } from "@/components/island/IslandLinkageBits"
import { CapsuleIsland } from "@/components/island/CapsuleIsland"
import { IslandExprLab } from "@/components/island/IslandExprLab"
import { LazyConnectionSwitcher } from "@/components/island/LazyConnectionSwitcher"
import { DockControls, FreeControls } from "@/components/island/PreviewControls"
import { QuickActionWheel } from "@/components/island/QuickActionWheel"
import { RecapCard } from "@/components/island/RecapCard"
import { useIslandHitReporter } from "@/hooks/useIslandHitRegion"
import { useIslandTheater } from "@/hooks/useIslandTheater"
import { useIslandWindowModel } from "@/hooks/useIslandWindowModel"
import { useIslandWindowShell } from "@/hooks/useIslandWindowShell"
import { api, isTauri, type IslandHitRectDto } from "@/lib/api"
import type { BotExpr, BotProp, BotShot } from "@/lib/botExpression"
import { checkinApi } from "@/lib/checkinApi"
import { CHECKIN_EDITIONS } from "@/lib/dailyCheckin"
import { gatewayApi } from "@/lib/gatewayApi"
import {
  buildQuickActions,
  computeQuickActionDock,
  LONG_PRESS_MS,
  LONG_PRESS_TOLERANCE,
  pickQuickActionAtPoint,
  type QuickActionDockLayout,
  type QuickActionId,
} from "@/lib/islandQuickActions"
import type { TheaterAction } from "@/lib/islandTheater"
import { capsuleUsesEdgeTab } from "@/lib/islandViewState"
import { LiveCountsContext } from "@/lib/liveCounts"

/**
 * 胶囊灵动岛舞台窗口（island-rendering-tech.md + jolu-new.pen R2-I-01～07）：
 * - 固定尺寸透明舞台窗口 + 后端轮廓命中鼠标穿透；
 * - 完整集成长按快捷动作轮盘（R2-I-06）、看向主窗口方位（R2-I-06）、bot 道具小剧场（R2-I-05）与今日回顾成就卡（R2-I-07）。
 */
const PEEK_DELAY_MS = 400

export function CapsuleWindow() {
  const menuBusy = useRef(false)
  const stageRef = useRef<HTMLDivElement>(null)
  const capsuleHoverRef = useRef(false)
  const lastRectsRef = useRef<IslandHitRectDto[]>([])
  const dragBlockedRef = useRef(false)
  const suppressPokeClickRef = useRef(false)
  const { report: reportHitRegion, cancel: cancelHitReport } = useIslandHitReporter()
  const shell = useIslandWindowShell(menuBusy, dragBlockedRef)
  const {
    mode,
    edge,
    peek,
    dragging,
    connectionMenuOpen,
    snapEdge,
    setMode,
    setEdge,
    setPeek,
    setConnectionMenuOpen,
    openDockEdge,
    draggingRef,
    dockedRef,
    expandedRef,
    edgeRef,
    peekRef,
    peekTimer,
    snapIndicator,
    onPointerDown,
    contextMenu,
    undock,
    collapseExpanded,
  } = shell

  const model = useIslandWindowModel({ capsule: true, mode, peek, dragging })
  const [previewExpr, setPreviewExpr] = useState<BotExpr | null>(null)
  const [previewProp, setPreviewProp] = useState<BotProp>("none")
  const [previewBubble, setPreviewBubble] = useState(false)
  const [previewLightBg, setPreviewLightBg] = useState(false)
  const [botActivity, setBotActivity] = useState<{ shot: BotShot | null; inactiveMs: number }>({ shot: null, inactiveMs: 0 })
  const [quickToast, setQuickToast] = useState<string | null>(null)
  const quickToastTimer = useRef(0)

  const {
    data,
    settings,
    linkage,
    mainWindowCenter,
    profile,
    cloneId,
    connections,
    connectionsLoading,
    active,
    refreshActive,
    quota,
    balance,
    gateway,
    live,
  } = model
  menuBusy.current = model.refreshingRef.current || quota.loading || balance.loading

  const showQuickFeedback = useCallback((text: string) => {
    window.clearTimeout(quickToastTimer.current)
    setQuickToast(text)
    quickToastTimer.current = window.setTimeout(() => setQuickToast(null), 2400)
  }, [])
  useEffect(() => () => window.clearTimeout(quickToastTimer.current), [])

  // 通知后端展开态变化
  useEffect(() => {
    if (!isTauri) return
    void api.islandSetExpanded(mode === "expanded").catch(() => {})
  }, [mode])

  useEffect(() => () => cancelHitReport(), [cancelHitReport])

  const dockEdge = mode === "docked" ? edge : mode === "expanded" ? openDockEdge.current : null
  const visualDockEdge = dragging ? (snapEdge ?? null) : dockEdge
  const horizontalFull = settings.island_horizontal_dock_full ?? true
  const horizontalFullRef = useRef(horizontalFull)
  horizontalFullRef.current = horizontalFull
  const edgeTab = capsuleUsesEdgeTab(dockEdge, mode, peek, horizontalFull)
  const scale = settings.island_scale / 100
  const reduced = Boolean(settings.reduce_motion)

  // ---------- R2-I-05 道具剧场 + R2-I-07 今日回顾与成就 ----------
  const runningCount = data.sessions?.filter((s) => s.state === "running").length ?? 0
  const writing = data.working ?? (!data.sessionStatusUnknown && runningCount > 0)
  const theater = useIslandTheater({
    linkage,
    dnd: settings.dnd,
    platform: data.platform,
    platformName: data.platformName,
    connectionName: active?.name ?? null,
    writing,
    activeSessionId: data.sessions?.[0]?.id ?? null,
    shot: botActivity.shot,
    inactiveMs: botActivity.inactiveMs,
    todayTokens: live.todayTokens ?? null,
    gateway: gateway.relay ? gateway : null,
    isClone: Boolean(cloneId),
  })

  const handleTheaterAction = useCallback((action: TheaterAction) => {
    theater.dismissCue()
    if (!isTauri) {
      if (action.kind !== "none") showQuickFeedback(`预览：触发「${action.kind}」跳转`)
      return
    }
    if (action.kind === "checkin") void api.openMainSection("checkin").catch(() => {})
    else if (action.kind === "usage") void api.openMainUsage(data.platform, data.usageConnectionId ?? null).catch(() => {})
    else if (action.kind === "gateway") void api.openMainSection("gateway").catch(() => {})
    else if (action.kind === "session") void api.openMainSession(action.platform, action.sessionId).catch(() => {})
  }, [theater, data.platform, data.usageConnectionId, showQuickFeedback])

  // ---------- R2-I-06 长按 bot 快捷动作栏 ----------
  const [wheel, setWheel] = useState<{
    open: boolean
    holding: boolean
    layout: QuickActionDockLayout | null
    activeId: QuickActionId | null
  }>({
    open: false,
    holding: false,
    layout: null,
    activeId: null,
  })
  const longPressTimer = useRef(0)

  const quickActions = useMemo(
    () => buildQuickActions({
      relayRunning: Boolean(gateway.relay),
      hasSession: Boolean(data.sessions?.[0]),
      canToggleRelay: Boolean(gateway.status) || !isTauri,
    }),
    [gateway.relay, gateway.status, data.sessions],
  )

  const executeQuickAction = useCallback((id: QuickActionId) => {
    setWheel({ open: false, holding: false, layout: null, activeId: null })
    dragBlockedRef.current = false

    if (!isTauri) {
      const label = quickActions.find((a) => a.id === id)?.label ?? id
      showQuickFeedback(`已执行：${label}`)
      return
    }

    if (id === "checkin") {
      void (async () => {
        try {
          const overview = await checkinApi.overview()
          const targets = overview.accounts.filter(
            (a) => a.connection_status !== "paused" && CHECKIN_EDITIONS.some((e) => e.manual && e.platform === a.platform && e.edition === a.edition),
          )
          if (targets.length === 0) {
            void api.openMainSection("checkin")
            return
          }
          showQuickFeedback("正在检查签到状态…")
          let claimed = 0
          for (const account of targets) {
            const st = await checkinApi.status(account.id).catch(() => null)
            if (st?.outcome.state === "ready" && st.account_key) {
              const res = await checkinApi.claim(account.id, st.account_key).catch(() => null)
              if (res?.outcome.state === "claimed") claimed += 1
            }
          }
          showQuickFeedback(claimed > 0 ? `已签到 ${claimed} 个账号` : "今日没有待签到账号")
        } catch {
          void api.openMainSection("checkin")
        }
      })()
      return
    }

    if (id === "refresh") {
      void api.islandAction("refresh", cloneId)
        .then(() => showQuickFeedback("已触发额度刷新"))
        .catch(() => showQuickFeedback("刷新失败"))
      return
    }

    if (id === "toggle_relay") {
      const nextMode = gateway.relay ? "observe" : "relay"
      void gatewayApi.setMode(nextMode, gateway.status?.default_connection ?? null)
        .then(() => showQuickFeedback(nextMode === "relay" ? "已开启模型中转" : "已停止模型中转"))
        .catch(() => showQuickFeedback("切换中转失败"))
      return
    }

    if (id === "open_session") {
      const top = data.sessions?.[0]
      if (top) void api.openMainSession(data.platform, top.id).catch(() => {})
      else void api.openMainSection("sessions").catch(() => {})
      return
    }

    if (id === "new_automation") {
      const top = data.sessions?.[0]
      void api.openMainAutomationNew(data.platform, top?.id ?? null).catch(() => {})
    }
  }, [quickActions, cloneId, gateway.relay, gateway.status?.default_connection, data.sessions, data.platform, showQuickFeedback])

  const closeWheel = useCallback(() => {
    setWheel({ open: false, holding: false, layout: null, activeId: null })
    dragBlockedRef.current = false
  }, [])

  const onBotFacePointerDown = useCallback((event: React.PointerEvent) => {
    if (event.button !== 0 || event.detail > 1) return
    const fallbackEl = event.currentTarget as Element
    const startX = event.clientX
    const startY = event.clientY
    let opened = false
    let draggedAfterOpen = false
    let activeLayout: QuickActionDockLayout | null = null
    let stageOrigin = { left: 0, top: 0 }
    let latestActive: QuickActionId | null = null

    const onMove = (ev: PointerEvent) => {
      if (!opened) {
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) > LONG_PRESS_TOLERANCE) {
          cleanup()
        }
        return
      }
      if (!activeLayout) return
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) > 10) {
        draggedAfterOpen = true
      }
      const px = ev.clientX - stageOrigin.left
      const py = ev.clientY - stageOrigin.top
      const picked = pickQuickActionAtPoint(px, py, activeLayout)
      latestActive = picked
      setWheel((curr) => (curr.activeId === picked ? curr : { ...curr, activeId: picked }))
    }

    const onUp = () => {
      const wasOpen = opened
      const chosen = latestActive
      const didDrag = draggedAfterOpen
      cleanup()
      if (wasOpen) {
        if (chosen) {
          executeQuickAction(chosen)
        } else if (didDrag) {
          closeWheel()
        } else {
          // 长按后直接原地松手：保持动作栏打开，方便看清后直接点选
          dragBlockedRef.current = false
          setWheel((curr) => (curr.open ? { ...curr, holding: false } : curr))
        }
      }
    }

    const cleanup = () => {
      window.clearTimeout(longPressTimer.current)
      longPressTimer.current = 0
      window.removeEventListener("pointermove", onMove)
      window.removeEventListener("pointerup", onUp)
      window.removeEventListener("pointercancel", onUp)
    }

    longPressTimer.current = window.setTimeout(() => {
      const stageEl = stageRef.current
      if (!stageEl) return
      const sRect = stageEl.getBoundingClientRect()
      const capEl = stageEl.querySelector<HTMLElement>("[data-capsule-island]")
      const cRect = capEl ? capEl.getBoundingClientRect() : fallbackEl.getBoundingClientRect()
      stageOrigin = { left: sRect.left, top: sRect.top }
      activeLayout = computeQuickActionDock(
        quickActions,
        dockEdge,
        {
          x: cRect.left - sRect.left,
          y: cRect.top - sRect.top,
          width: cRect.width,
          height: cRect.height,
        },
        { width: sRect.width, height: sRect.height },
      )
      opened = true
      dragBlockedRef.current = true
      suppressPokeClickRef.current = true
      setWheel({ open: true, holding: true, layout: activeLayout, activeId: null })
    }, LONG_PRESS_MS)

    window.addEventListener("pointermove", onMove)
    window.addEventListener("pointerup", onUp)
    window.addEventListener("pointercancel", onUp)
  }, [quickActions, dockEdge, executeQuickAction, closeWheel])

  useEffect(() => () => window.clearTimeout(longPressTimer.current), [])
  useEffect(() => { closeWheel() }, [dockEdge, mode, closeWheel])

  const wheelOpenRef = useRef(false)
  wheelOpenRef.current = wheel.open
  useEffect(() => {
    if (wheel.open) return
    if (dockedRef.current && peekRef.current && !expandedRef.current && !capsuleHoverRef.current) {
      setPeek(false)
    }
  }, [wheel.open, dockedRef, peekRef, expandedRef, setPeek])

  // 强制交互态：下拉、拖动、轮盘或今日回顾卡展开时关闭鼠标穿透
  const interactive = connectionMenuOpen || dragging || wheel.open || theater.recapOpen
  useEffect(() => {
    if (!isTauri || lastRectsRef.current.length === 0) return
    reportHitRegion(lastRectsRef.current, interactive, true)
  }, [interactive, reportHitRegion])

  const onHitRects = useCallback((rects: IslandHitRectDto[], immediate = false) => {
    lastRectsRef.current = rects
    reportHitRegion(rects, interactive, immediate)
  }, [interactive, reportHitRegion])

  const onCapsuleHover = useCallback((over: boolean) => {
    capsuleHoverRef.current = over
    if (peekTimer.current) { window.clearTimeout(peekTimer.current); peekTimer.current = null }
    if (!over) {
      if (wheelOpenRef.current) return
      if (peekRef.current) setPeek(false)
      return
    }
    if (draggingRef.current || !dockedRef.current || expandedRef.current) return
    if (horizontalFullRef.current && (edgeRef.current === "top" || edgeRef.current === "bottom")) return
    peekTimer.current = window.setTimeout(() => setPeek(true), PEEK_DELAY_MS)
  }, [peekTimer, peekRef, draggingRef, dockedRef, expandedRef, edgeRef, setPeek])

  const onCapsulePointerMove = useCallback((event: React.PointerEvent) => {
    if (!dockedRef.current || !peekRef.current || expandedRef.current || wheelOpenRef.current) return
    const target = event.target as HTMLElement | null
    if (target?.closest("[data-capsule-island]")) return
    setPeek(false)
  }, [dockedRef, peekRef, expandedRef, setPeek])

  const anchorClass =
    dockEdge === "top" ? "top-0 left-1/2 -translate-x-1/2"
      : dockEdge === "bottom" ? "bottom-0 left-1/2 -translate-x-1/2"
        : dockEdge === "left" ? "left-0 top-1/2 -translate-y-1/2"
          : dockEdge === "right" ? "right-0 top-1/2 -translate-y-1/2"
            : "top-6 left-1/2 -translate-x-1/2"

  const stageBody = (
    <div
      ref={stageRef}
      style={{ opacity: settings.island_opacity / 100 }}
      className={`relative h-full w-full ${dragging ? "island-dragging" : ""}`}
      onContextMenu={contextMenu}
      onPointerDown={onPointerDown}
      onPointerMove={onCapsulePointerMove}
      onDragStart={(e) => e.preventDefault()}
    >
      {snapIndicator}
      <div className="absolute inset-0" style={{ zoom: scale }}>
        <div className={`absolute w-max ${anchorClass}`}>
          <div className="relative flex flex-col items-center">
            <SignalRing signal={data.signal} round={mode === "expanded" ? "rounded-[28px]" : "rounded-full"} />
            <CapsuleIsland
              data={data}
              open={mode === "expanded"}
              mini={edgeTab}
              connectionMenuOpen={connectionMenuOpen}
              onOpenChange={(next) => {
                if (next) {
                  openDockEdge.current = mode === "docked" ? edge : profile.dock.edge ?? null
                  setMode("expanded")
                  setConnectionMenuOpen(false)
                } else {
                  collapseExpanded(capsuleHoverRef.current)
                }
              }}
              dragging={dragging}
              refreshing={refreshActive}
              dockEdge={dockEdge}
              visualDockEdge={visualDockEdge}
              gateway={gateway.relay ? gateway : null}
              linkage={linkage}
              lookAtPhysical={mainWindowCenter}
              theaterCue={theater.activeCue}
              onTheaterAction={handleTheaterAction}
              onBotFacePointerDown={onBotFacePointerDown}
              suppressPokeClickRef={suppressPokeClickRef}
              onBotActivity={setBotActivity}
              canViewRecap={theater.canViewRecap && !cloneId}
              onOpenRecap={() => {
                collapseExpanded(false)
                theater.setRecapOpen(true)
              }}
              zoom={scale}
              reduced={reduced}
              previewExpr={previewExpr}
              previewProp={previewProp}
              previewBubble={previewBubble}
              onHoverChange={onCapsuleHover}
              onHitRects={onHitRects}
              connectionSwitcher={
                <LazyConnectionSwitcher
                  connections={connections}
                  selectedId={active?.id}
                  loading={connectionsLoading}
                  onOpenChange={setConnectionMenuOpen}
                  onSelect={isTauri ? (id) => api.setIslandConnection(id, cloneId) : undefined}
                />
              }
            />

            {quickToast && (
              <div
                role="status"
                data-quick-toast
                className={dockEdge === "bottom" ? "cap-quick-toast cap-quick-toast-top" : "cap-quick-toast"}
              >
                {quickToast}
              </div>
            )}

            {theater.recapOpen && (
              <div className="mt-3">
                <RecapCard
                  recap={theater.recapData}
                  onOpenUsage={() => {
                    theater.dismissRecapToday()
                    if (isTauri) void api.openMainUsage(null, null).catch(() => {})
                    else showQuickFeedback("预览：打开今日用量")
                  }}
                  onDismiss={theater.dismissRecapToday}
                  onClearAchievements={theater.resetAchievements}
                />
              </div>
            )}
          </div>
          {mode === "expanded" && (
            <div className="mt-2 flex justify-center">
              <FollowingBar following={data.following} />
            </div>
          )}
        </div>
      </div>

      {/* R2-I-06 长按 bot 弹出的 5 项安全快捷动作栏 */}
      <QuickActionWheel
        open={wheel.open}
        layout={wheel.layout}
        activeId={wheel.activeId}
        holding={wheel.holding}
        onHoverId={(activeId) => setWheel((curr) => ({ ...curr, activeId }))}
        onSelect={executeQuickAction}
        onCancel={closeWheel}
      />
    </div>
  )

  return (
    <LiveCountsContext.Provider value={live.counts}>
      {isTauri ? (
        stageBody
      ) : (
        <div className="relative flex min-h-[720px] w-full flex-col items-center">
          <div
            data-testid="island-preview-stage"
            className="relative h-[640px] w-[520px] max-w-full overflow-hidden rounded-[20px] border border-jn-line shadow-jn-surface transition-colors"
            style={{
              background: previewLightBg
                ? "linear-gradient(135deg, #e7edf0 0%, #dce7e2 52%, #eef2f3 100%)"
                : "linear-gradient(135deg, #121820 0%, #1a2420 52%, #111518 100%)",
            }}
          >
            {stageBody}
          </div>

          <div className="panel mt-3 flex flex-wrap items-center gap-2 rounded-[12px] border border-jn-line bg-jn-surface px-3.5 py-2 text-[11px] shadow-jn-surface">
            {dockEdge
              ? <DockControls edge={edge} onEdge={(e) => { setEdge(e); setMode("docked") }} onUndock={undock} />
              : <FreeControls mode={mode} onMode={setMode} onDock={(e) => { setEdge(e); setMode("docked") }} />}
            <span className="h-3.5 w-px bg-jn-line" />
            <button
              type="button"
              className={`rounded border px-2 py-0.5 ${previewBubble ? "border-jn-primary text-jn-primary-ink" : "border-jn-line text-jn-text-2"}`}
              onClick={() => setPreviewBubble((v) => !v)}
            >
              液体气泡：{previewBubble ? "开" : "关"}
            </button>
            <button
              type="button"
              className={`rounded border px-2 py-0.5 ${theater.recapOpen ? "border-jn-primary text-jn-primary-ink" : "border-jn-line text-jn-text-2"}`}
              onClick={() => theater.setRecapOpen((v) => !v)}
            >
              今日回顾：{theater.recapOpen ? "开" : "关"}
            </button>
            <button
              type="button"
              className="rounded border border-jn-line px-2 py-0.5 text-jn-text-2"
              onClick={() => setPreviewLightBg((v) => !v)}
            >
              桌面壁纸：{previewLightBg ? "浅色" : "深色"}
            </button>
          </div>

          <div className="panel fixed right-4 top-16 z-50 max-h-[calc(100vh-6rem)] w-[300px] overflow-y-auto rounded-[14px] border border-jn-line bg-jn-surface p-3 shadow-jn-surface">
            <IslandExprLab onPreview={setPreviewExpr} onPreviewProp={setPreviewProp} />
          </div>
        </div>
      )}
    </LiveCountsContext.Provider>
  )
}
