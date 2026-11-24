import { useCallback, useEffect, useRef, useState } from "react"
import { FollowingBar, SignalRing } from "@/components/island/IslandLinkageBits"
import { CapsuleIsland } from "@/components/island/CapsuleIsland"
import { IslandExprLab } from "@/components/island/IslandExprLab"
import { LazyConnectionSwitcher } from "@/components/island/LazyConnectionSwitcher"
import { DockControls, FreeControls } from "@/components/island/PreviewControls"
import { useIslandHitReporter } from "@/hooks/useIslandHitRegion"
import { useIslandWindowModel } from "@/hooks/useIslandWindowModel"
import { useIslandWindowShell } from "@/hooks/useIslandWindowShell"
import { useSettings } from "@/hooks/useSettings"
import { api, isTauri, type IslandHitRectDto } from "@/lib/api"
import type { BotExpr } from "@/lib/botExpression"
import { capsuleUsesEdgeTab } from "@/lib/islandViewState"
import { LiveCountsContext } from "@/lib/liveCounts"

/**
 * 胶囊灵动岛舞台窗口（docs/specs/2026-11-24-island-rendering-tech.md §3–§6）。
 *
 * - 固定尺寸透明舞台窗口，形变期间不再调用 `resize_island`（`useIslandWindowSize` 已退役）；
 * - 透明区域默认鼠标穿透，由 `useIslandHitReporter` 将轮廓矩形上报给后端 `island_pointer`；
 * - 四条边统一「从屏幕边缘长出来」：停靠态零边缘留白、靠边两角 0px，悬停 400ms 沿同路径长出。
 */
const PEEK_DELAY_MS = 400

export function CapsuleWindow() {
  const menuBusy = useRef(false)
  const capsuleHoverRef = useRef(false)
  const lastRectsRef = useRef<IslandHitRectDto[]>([])
  const { report: reportHitRegion, cancel: cancelHitReport } = useIslandHitReporter()
  const shell = useIslandWindowShell(menuBusy)
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

  const { setIslandHorizontalDockFull } = useSettings()
  const model = useIslandWindowModel({ capsule: true, mode, peek, dragging })
  const [previewExpr, setPreviewExpr] = useState<BotExpr | null>(null)
  const [previewBubble, setPreviewBubble] = useState(false)
  const [previewLightBg, setPreviewLightBg] = useState(false)
  const {
    data,
    settings,
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

  // 通知后端展开态变化：替代旧版按窗口高度 > 150px 取消首次安装引导的逻辑（§3）
  useEffect(() => {
    if (!isTauri) return
    void api.islandSetExpanded(mode === "expanded").catch(() => {})
  }, [mode])

  useEffect(() => () => cancelHitReport(), [cancelHitReport])

  // 下拉菜单打开或拖动中：立即上报强制交互态，防止菜单被穿透点击
  const interactive = connectionMenuOpen || dragging
  useEffect(() => {
    if (!isTauri || lastRectsRef.current.length === 0) return
    reportHitRegion(lastRectsRef.current, interactive, true)
  }, [interactive, reportHitRegion])

  const onHitRects = useCallback((rects: IslandHitRectDto[], immediate = false) => {
    lastRectsRef.current = rects
    reportHitRegion(rects, interactive, immediate)
  }, [interactive, reportHitRegion])

  const horizontalDockFull = settings.island_horizontal_dock_full !== false
  const horizontalDockFullRef = useRef(horizontalDockFull)
  horizontalDockFullRef.current = horizontalDockFull

  useEffect(() => {
    if (!horizontalDockFull && !capsuleHoverRef.current && peekRef.current) {
      setPeek(false)
    }
  }, [horizontalDockFull, peekRef, setPeek])

  // 停靠收起为小图标时，悬停 400ms 沿离开屏幕边方向长出完整内容，移开立即收回（§5）
  const onCapsuleHover = useCallback((over: boolean) => {
    capsuleHoverRef.current = over
    if (peekTimer.current) { window.clearTimeout(peekTimer.current); peekTimer.current = null }
    if (!over) {
      if (peekRef.current) setPeek(false)
      return
    }
    if (draggingRef.current || !dockedRef.current || expandedRef.current) return
    if (horizontalDockFullRef.current && (edgeRef.current === "top" || edgeRef.current === "bottom")) return
    peekTimer.current = window.setTimeout(() => setPeek(true), PEEK_DELAY_MS)
  }, [peekTimer, peekRef, draggingRef, dockedRef, expandedRef, edgeRef, setPeek])

  const onCapsulePointerMove = useCallback((event: React.PointerEvent) => {
    if (!dockedRef.current || !peekRef.current || expandedRef.current) return
    const target = event.target as HTMLElement | null
    if (target?.closest("[data-capsule-island]")) return
    setPeek(false)
  }, [dockedRef, peekRef, expandedRef, setPeek])

  const dockEdge = mode === "docked" ? edge : mode === "expanded" ? openDockEdge.current : null
  // 拖动离边时提前松开视觉直角，进入吸附区时再贴平（§5 拖离边缘直角平滑变回圆角）
  const visualDockEdge = dragging ? (snapEdge ?? null) : dockEdge
  // 贴边是否收起为小图标：开启「上下贴边保持完整胶囊」时，贴顶 / 贴底常显完整胶囊，仅左右收起
  const edgeTab = capsuleUsesEdgeTab(dockEdge, mode, peek, horizontalDockFull)
  const scale = settings.island_scale / 100
  const reduced = Boolean(settings.reduce_motion)

  // 舞台内锚点：停靠方向零余量紧贴屏幕边，自由态顶部留 24px 完整展示两层硬件阴影
  const anchorClass =
    dockEdge === "top" ? "top-0 left-1/2 -translate-x-1/2"
      : dockEdge === "bottom" ? "bottom-0 left-1/2 -translate-x-1/2"
        : dockEdge === "left" ? "left-0 top-1/2 -translate-y-1/2"
          : dockEdge === "right" ? "right-0 top-1/2 -translate-y-1/2"
            : "top-6 left-1/2 -translate-x-1/2"

  const stageBody = (
    <div
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
          <div className="relative">
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
              zoom={scale}
              reduced={reduced}
              previewExpr={previewExpr}
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
          </div>
          {mode === "expanded" && (
            <div className="mt-2 flex justify-center">
              <FollowingBar following={data.following} />
            </div>
          )}
        </div>
      </div>
    </div>
  )

  return (
    <LiveCountsContext.Provider value={live.counts}>
      {isTauri ? (
        stageBody
      ) : (
        <div className="relative flex min-h-[720px] w-full flex-col items-center">
          {/* 浏览器预览的 520×660 模拟舞台（可切浅/深桌面背景对照 R2 硬件质感与四边长出） */}
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

          <div className="panel fixed bottom-4 left-1/2 z-50 flex -translate-x-1/2 flex-wrap items-center gap-2 rounded-[12px] border border-jn-line bg-jn-surface px-3.5 py-2 text-[11px] shadow-jn-surface">
            {dockEdge
              ? <DockControls edge={edge} onEdge={(e) => { setEdge(e); setMode("docked") }} onUndock={undock} />
              : <FreeControls mode={mode} onMode={setMode} onDock={(e) => { setEdge(e); setMode("docked") }} />}
            <span className="h-3.5 w-px bg-jn-line" />
            <button
              type="button"
              className={`rounded border px-2 py-0.5 ${horizontalDockFull ? "border-jn-primary text-jn-primary-ink" : "border-jn-line text-jn-text-2"}`}
              onClick={() => void setIslandHorizontalDockFull(!horizontalDockFull)}
            >
              上下贴边完整胶囊：{horizontalDockFull ? "开" : "关"}
            </button>
            <button
              type="button"
              className={`rounded border px-2 py-0.5 ${previewBubble ? "border-jn-primary text-jn-primary-ink" : "border-jn-line text-jn-text-2"}`}
              onClick={() => setPreviewBubble((v) => !v)}
            >
              液体气泡：{previewBubble ? "开" : "关"}
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
            <IslandExprLab onPreview={setPreviewExpr} />
          </div>
        </div>
      )}
    </LiveCountsContext.Provider>
  )
}
