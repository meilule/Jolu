import { useContentMotion } from "@/lib/motion"
import { EVENTS } from "@/lib/events"
import { useCallback, useEffect, useRef, useState } from "react"
import { api, isTauri, listenEvent, openUrl, type MainIntentDto } from "@/lib/api"
import { cn } from "@/lib/utils"
import { PRIMARY_PAGES, SETTINGS_PAGE, pageForIntent, pageLabel, readSidebarCollapsed, resolveOpenPage, writeSidebarCollapsed, type MainPage } from "@/lib/mainNavigation"
import { settingsApi } from "@/lib/settingsApi"
import { applyAppearance, normalizeDensity } from "@/lib/appearance"
import { useSettings } from "@/hooks/useSettings"
import { Icon } from "@/icons"
import { MotionProvider, TactileButton } from "@/motion"
import { Button, Dialog, NavItem, PageActionsProvider, PageHeader, SHELL_HEIGHT, SHELL_WIDTH, Sidebar, Tooltip, WindowShell, usesOwnScroll } from "@/ui"
import { Wordmark } from "@/components/brand/Wordmark"
import { NAV_ICONS } from "@/ui/shell/navIcons"
import { AppIcon } from "@/components/brand/AppIcon"
import { ToastProvider, ToastViewport } from "@/components/ui/Toast"
import { PrivacyFirstNotice } from "@/components/settings/PrivacyRecordsDialog"
import { TipProvider } from "@/components/ui/Tip"
import { HeaderActions, REPO_URL } from "@/components/panel/HeaderActions"
import { SidebarIslandCard } from "@/components/panel/SidebarIslandCard"
import { AppNotices } from "@/components/panel/AppNotices"
import type { NoticeTarget } from "@/lib/appNotices"
import { dateKey } from "@/lib/usageStats"
import { UpdateProvider } from "@/components/panel/UpdateContext"
import { UpdateDialog } from "@/components/panel/UpdateDialog"
import { createLazyPage } from "@/components/panel/lazyPage"
import type { SessionTarget } from "./SessionsView"
import type { AutomationPrefill, AutomationTarget } from "./AutomationView"
import type { UsageRequest } from "./UsageView"
import type { PlatformId } from "@/types"

const IslandView = createLazyPage(() => import("./IslandView").then((m) => ({ default: m.IslandView })), "灵动岛", "island")
const ProvidersView = createLazyPage(() => import("./ProvidersView").then((m) => ({ default: m.ProvidersView })), "供应商", "providers", true)
const CheckinView = createLazyPage(() => import("./CheckinView").then((m) => ({ default: m.CheckinView })), "一键签到", "cards")
const SessionsView = createLazyPage(() => import("./SessionsView").then((m) => ({ default: m.SessionsView })), "会话线程", "sessions")
const AutomationView = createLazyPage(() => import("./AutomationView").then((m) => ({ default: m.AutomationView })), "自动化", "tasks")
const SkillsView = createLazyPage(() => import("./SkillsView").then((m) => ({ default: m.SkillsView })), "Skills 管理", "table")
const UsageView = createLazyPage(() => import("./UsageView").then((m) => ({ default: m.UsageView })), "用量统计", "stats")
const GatewayView = createLazyPage(() => import("./GatewayView").then((m) => ({ default: m.GatewayView })), "模型路由", "settings", true)
const AgentsView = createLazyPage(() => import("./AgentsView").then((m) => ({ default: m.AgentsView })), "Agent", "settings", true)
const SettingsPage = createLazyPage(() => import("./SettingsPage").then((m) => ({ default: m.SettingsPage })), "设置", "settings")

/**
 * 主面板窗口（Jolu.pen 模块 A 画板 01–04）：30px 系统标题栏 + 左侧导航 + 内容区。
 * 标题栏、侧栏与内容页头固定，只有主内容区滚动。关闭只隐藏窗口，后台采集继续运行。
 */
export function MainPanelWindow({
  initialPage = "island",
  /** true = 作为真实桌面窗口铺满，不要预览用的固定尺寸与投影 */
  embedded = false,
}: {
  initialPage?: MainPage
  embedded?: boolean
}) {
  const [page, setPage] = useState<MainPage>(initialPage)
  // 首次进入才加载；之后保持挂载，使批量签到在切页时仍可继续并保留结果。
  const checkinVisited = useRef(initialPage === "checkin")
  if (page === "checkin") checkinVisited.current = true
  const [collapsed, setCollapsed] = useState(() => readSidebarCollapsed())
  const [navigationRequest, setNavigationRequest] = useState(0)
  /** 托盘 / 菜单要定位的设置分区（旧分区 id，由设置页换算成新分类） */
  const [settingsSection, setSettingsSection] = useState<string | null>(null)
  const [about, setAbout] = useState<string | null>(null)
  const [panelActive, setPanelActive] = useState(true)
  /** 页面之间的定位跳转：用量统计 → 指定会话，会话 → 该会话的调用明细 */
  const [sessionTarget, setSessionTarget] = useState<SessionTarget | null>(null)
  /** 会话线程「创建自动化」带入自动化新建页的会话（画板 72） */
  const [automationPrefill, setAutomationPrefill] = useState<AutomationPrefill | null>(null)
  const [automationTarget, setAutomationTarget] = useState<AutomationTarget | null>(null)
  const [accountTarget, setAccountTarget] = useState<{ connectionId: string | null; nonce: number } | null>(null)
  const clearAccountTarget = useCallback(() => setAccountTarget(null), [])
  const [deferredNotice, setDeferredNotice] = useState<NoticeTarget | null>(null)
  const clearDeferredNotice = useCallback(() => setDeferredNotice(null), [])
  const clearAutomationTarget = useCallback(() => setAutomationTarget(null), [])
  const [usageRequest, setUsageRequest] = useState<UsageRequest | null>(null)
  const [opening, setOpening] = useState(embedded && isTauri)
  const readySent = useRef(false)
  /** 桌面主窗口：首次挂载按「默认打开页面」定页，定好后才通知后端显示窗口，避免先闪一下灵动岛页 */
  const desktop = embedded && isTauri
  const [pageResolved, setPageResolved] = useState(!desktop)
  const initialPending = useRef(desktop)
  /** 「上次访问的页面」：切页后延迟 1 秒写入，期间再切换就重新计时 */
  const rememberTimer = useRef<number | null>(null)
  const pendingPage = useRef<MainPage | null>(null)
  const savedPage = useRef<string | null>(null)
  /** 各页面的滚动位置：切换模块再回来时保持原位（画板 104） */
  const scrollRef = useRef<HTMLDivElement>(null)
  const scrollPositions = useRef(new Map<MainPage, number>())
  // 页面包含 fixed 弹窗，避免祖先 transform 改变弹窗的定位参照。
  const pageMotion = useContentMotion(page, 220, 0, true)
  /** 设置 · 外观（画板 88）：列表密度与减少动效挂在 <html> 上；缩放由后端对 main 窗口 set_zoom */
  const { list_density: listDensity, reduce_motion: reduceMotion } = useSettings().settings

  useEffect(() => {
    const root = document.documentElement
    applyAppearance(root, normalizeDensity(listDensity), reduceMotion)
    // 浏览器预览会在同一文档里切到灵动岛视图，卸载时还原
    return () => applyAppearance(root, "comfortable", false)
  }, [listDensity, reduceMotion])

  const navigate = (next: MainPage) => {
    if (scrollRef.current) scrollPositions.current.set(page, scrollRef.current.scrollTop)
    setPage(next)
  }

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollPositions.current.get(page) ?? 0
  }, [page])

  useEffect(() => {
    if (!embedded || !isTauri || !pageResolved || readySent.current) return
    readySent.current = true
    void api.mainPanelReady().then(() => setOpening(false)).catch((error) => {
      console.error("主面板就绪通知失败", error)
    })
  }, [embedded, pageResolved])

  useEffect(() => {
    if (!desktop || !pageResolved) return
    if (rememberTimer.current !== null) window.clearTimeout(rememberTimer.current)
    rememberTimer.current = null
    if (page === savedPage.current) {
      pendingPage.current = null
      return
    }
    pendingPage.current = page
    rememberTimer.current = window.setTimeout(() => {
      rememberTimer.current = null
      void settingsApi.rememberPage(page).then(() => {
        savedPage.current = page
        if (pendingPage.current === page) pendingPage.current = null
      }).catch((error) => console.error("记录上次访问的页面失败", error))
    }, 1000)
  }, [desktop, page, pageResolved])

  useEffect(() => () => {
    if (rememberTimer.current !== null) window.clearTimeout(rememberTimer.current)
  }, [])

  // 托盘菜单可以直接把主面板带到指定位置
  useEffect(() => {
    const offs: Array<() => void> = []
    let stopped = false
    const finishInitial = () => {
      initialPending.current = false
      if (!stopped) setPageResolved(true)
    }
    const applyIntent = (intent: MainIntentDto | null) => {
      if (!intent || stopped) return
      if (intent.kind === "about") {
        setAbout(intent.version)
        return
      }
      if ((intent.kind === "providers" || intent.kind === "automation") && document.querySelector('[role="dialog"]:not([data-state="closed"]), [role="alertdialog"]:not([data-state="closed"])')) {
        setDeferredNotice(intent.kind === "providers" ? { kind: "account", id: intent.connection_id, nonce: Date.now() } : { kind: "task", id: intent.task_id, nonce: Date.now() })
        return
      }
      initialPending.current = false
      setAbout(null)
      if (intent.kind === "overview") {
        // 后端快照里的 last_page 可能还没写入最近一次切页（1 秒延迟内），以前端记录为准
        const lastPage = pendingPage.current ?? intent.settings.last_page
        if (intent.settings.last_page) savedPage.current = intent.settings.last_page
        setPage(pageForIntent({ kind: "overview", settings: { default_page: intent.settings.default_page, last_page: lastPage } }))
      } else setPage(pageForIntent(intent))
      if (intent.kind === "settings") setSettingsSection(intent.section)
      if (intent.kind === "automation") setAutomationTarget({ taskId: intent.task_id, nonce: Date.now() })
      if (intent.kind === "automation_new") {
        const nonce = Date.now()
        if (intent.platform && intent.session_id) {
          void api.sessionsList({ platform: intent.platform, connection: null, project: null, since: null, search: intent.session_id })
            .then((res) => {
              const matched = res.sessions.find((s) => s.platform === intent.platform && s.session_id === intent.session_id)
              if (!stopped) setAutomationPrefill(matched ? { session: matched, nonce } : { nonce })
            })
            .catch(() => { if (!stopped) setAutomationPrefill({ nonce }) })
        } else {
          setAutomationPrefill({ nonce })
        }
      }
      if (intent.kind === "providers") setAccountTarget({ connectionId: intent.connection_id, nonce: Date.now() })
      if (intent.kind === "sessions") setSessionTarget({ platform: intent.platform, sessionId: intent.session_id, nonce: Date.now() })
      if (intent.kind === "usage") {
        const today = dateKey(new Date())
        setUsageRequest({
          search: "", range: { start: today, end: today }, nonce: Date.now(), tab: "overview",
          platform: (intent.platform as PlatformId | null) ?? null,
          accounts: intent.connection_id ? [intent.connection_id] : [],
        })
      }
      if (intent.kind !== "overview") setNavigationRequest((value) => value + 1)
    }
    const consumeIntent = () => void api.takeMainIntent().then(applyIntent).catch(() => {})
    /** 首次挂载：有托盘 / 第二实例留下的跳转意图就按意图打开，否则（含「关于」）读设置决定打开哪一页 */
    const consumeInitial = () => void api.takeMainIntent().then(async (intent) => {
      applyIntent(intent)
      if (!initialPending.current) return
      const settings = await api.getSettings()
      savedPage.current = settings.last_page
      if (!stopped && initialPending.current) setPage(resolveOpenPage(settings.default_page, settings.last_page))
    }).catch((error) => console.error("读取默认打开页面失败", error)).finally(finishInitial)
    void listenEvent(EVENTS.MAIN_INTENT, consumeIntent).then((un) => {
      if (stopped) un()
      else {
        offs.push(un)
        if (initialPending.current) consumeInitial()
        else consumeIntent()
      }
    }).catch((error) => {
      console.error("主面板跳转监听失败", error)
      finishInitial()
    })
    return () => {
      stopped = true
      offs.forEach((f) => f())
    }
  }, [])

  useEffect(() => {
    if (!isTauri) return
    let stopped = false
    let visibilityRevision = 0
    const offs: Array<() => void> = []
    const update = async () => {
      const currentRevision = ++visibilityRevision
      const { getCurrentWindow } = await import("@tauri-apps/api/window")
      const current = getCurrentWindow()
      const [visible, minimized] = await Promise.all([current.isVisible(), current.isMinimized()])
      if (!stopped && currentRevision === visibilityRevision) setPanelActive(visible && !minimized && document.visibilityState !== "hidden")
    }
    const onVisibility = () => void update()
    document.addEventListener("visibilitychange", onVisibility)
    void import("@tauri-apps/api/window").then(async ({ getCurrentWindow }) => {
      if (stopped) return
      const current = getCurrentWindow()
      const listeners = await Promise.all([
        current.onResized(() => void update()),
        current.onFocusChanged(() => void update()),
      ])
      if (stopped) listeners.forEach((off) => off())
      else offs.push(...listeners)
      await update()
    }).catch(() => {})
    return () => {
      stopped = true
      document.removeEventListener("visibilitychange", onVisibility)
      offs.forEach((off) => off())
    }
    // 就绪命令显示窗口后再读一次；丢弃启动时隐藏窗口的迟到结果。
  }, [opening])

  const toggleSidebar = () => {
    setCollapsed((value) => {
      writeSidebarCollapsed(!value)
      return !value
    })
  }

  return (
    <MotionProvider reduced={reduceMotion}>
    <ToastProvider>
    <PrivacyFirstNotice />
    <AppNotices
      deferredTarget={deferredNotice}
      onDeferredHandled={clearDeferredNotice}
      onOpenAccount={connectionId => { navigate("providers"); setAccountTarget({ connectionId, nonce: Date.now() }) }}
      onOpenTask={taskId => { navigate("automation"); setAutomationTarget({ taskId, nonce: Date.now() }) }}
    />
    <UpdateProvider>
    <TipProvider>
    <div
      data-panel-opening={opening}
      className={
        embedded
          ? "desktop-panel-entry relative flex h-full min-h-0 w-full flex-col overflow-hidden bg-jn-stage"
          : "relative flex flex-col overflow-hidden rounded-[14px] bg-jn-stage"
      }
      style={embedded ? undefined : { width: SHELL_WIDTH, height: SHELL_HEIGHT, maxWidth: "100%" }}
    >
      {/* 外壳按 jolu-new.pen R1-C：侧栏（216/72，Logo 与收起按钮在侧栏顶）+ 页头（72，窗口按钮并入右端）+ 页面区。 */}
      <PageActionsProvider>
      <WindowShell
        sidebar={
          <Sidebar
            collapsed={collapsed}
            logo={<span className={collapsed ? "text-jn-text" : "flex items-center gap-2.5 text-jn-text"}>{collapsed ? <AppIcon size={30} /> : <><AppIcon size={30} /><Wordmark /></>}</span>}
            toggle={
              <Tooltip content={collapsed ? "展开侧栏" : "收起侧栏"} side="right">
                <TactileButton
                  variant="icon"
                  type="button"
                  onClick={toggleSidebar}
                  aria-label={collapsed ? "展开侧栏" : "收起侧栏"}
                  aria-expanded={!collapsed}
                  className="jn-interactive flex size-8 shrink-0 items-center justify-center rounded-[10px] text-jn-text-3 hover:bg-jn-surface/70 hover:text-jn-text"
                >
                  <Icon name="sidebarToggle" size="md" />
                </TactileButton>
              </Tooltip>
            }
            footer={
              <>
                <SidebarIslandCard collapsed={collapsed} onOpenIsland={() => navigate("island")} />
                <NavItem
                  label={SETTINGS_PAGE.label}
                  icon={NAV_ICONS.settings}
                  active={page === SETTINGS_PAGE.id}
                  collapsed={collapsed}
                  onClick={() => navigate(SETTINGS_PAGE.id)}
                />
              </>
            }
          >
            {PRIMARY_PAGES.map((entry) => (
              <NavItem
                key={entry.id}
                label={entry.label}
                icon={NAV_ICONS[entry.id]}
                active={page === entry.id}
                collapsed={collapsed}
                onClick={() => navigate(entry.id)}
              />
            ))}
          </Sidebar>
        }
        header={<PageHeader title={pageLabel(page)} globalActions={<HeaderActions />} />}
      >
          {/* 自带滚动的页面（Agent / 模型路由 / 供应商）用 flex-col + h-full 钉住可视区，外层不滚；
              其余页面用普通块级纵向滚动容器，不让 flex-col 压扁子内容。 */}
          <div ref={scrollRef} data-panel-scroll className={cn(
            "relative min-h-0 min-w-0 flex-1",
            usesOwnScroll(page) ? "flex flex-col overflow-hidden" : "overflow-y-auto overflow-x-hidden",
          )}>
            <div ref={pageMotion} className={cn(
              usesOwnScroll(page) ? "flex h-full min-h-0 flex-1 flex-col overflow-hidden" : "min-w-0 px-0.5 pb-4 pt-0.5",
            )}>
              {page === "island" && <IslandView />}
              {page === "usage" && (
                <UsageView
                  active={panelActive}
                  request={usageRequest}
                  onOpenSession={(target) => {
                    if (target) setSessionTarget({ ...target, nonce: Date.now() })
                    navigate("sessions")
                  }}
                />
              )}
              {page === "sessions" && (
                <SessionsView
                  active={panelActive}
                  target={sessionTarget}
                  onDrillUsage={(s) => {
                    setUsageRequest({ search: s.session_id, range: { start: dateKey(new Date(s.first_ts)), end: dateKey(new Date(s.last_ts)) }, nonce: Date.now() })
                    navigate("usage")
                  }}
                  onCreateAutomation={(session) => {
                    setAutomationPrefill({ session, nonce: Date.now() })
                    navigate("automation")
                  }}
                />
              )}
              {page === "providers" && <ProvidersView accountTarget={accountTarget} onAccountTargetHandled={clearAccountTarget} />}
              {checkinVisited.current && <div hidden={page !== "checkin"}><CheckinView active={page === "checkin" && panelActive} /></div>}
              {page === "agents" && <AgentsView onNavigate={navigate} />}
              {page === "gateway" && <GatewayView onNavigate={navigate} onOpenConnection={(connectionId) => { navigate("providers"); setAccountTarget({ connectionId: connectionId ?? null, nonce: Date.now() }) }} />}
              {page === "settings" && <SettingsPage section={settingsSection} navigationRequest={navigationRequest} onOpenSkills={() => navigate("skills")} />}
              {/* 模块 E · 自动化 */}
              {page === "automation" && (
                <AutomationView
                  active={panelActive}
                  prefill={automationPrefill}
                  target={automationTarget}
                  onTargetHandled={clearAutomationTarget}
                  onOpenSession={(platform, sessionId) => {
                    setSessionTarget({ platform, sessionId, nonce: Date.now() })
                    navigate("sessions")
                  }}
                  onOpenProviders={() => navigate("providers")}
                />
              )}
              {/* 模块 F · Skills 管理 */}
              {page === "skills" && <SkillsView active={panelActive} />}
            </div>
          </div>
      </WindowShell>
      </PageActionsProvider>

      {/* 操作通知固定在主窗口右上角，不随页面滚动或侧栏宽度变化。 */}
      <ToastViewport />
      <Dialog
        open={Boolean(about)}
        onOpenChange={(open) => { if (!open) setAbout(null) }}
        title={
          <span className="flex items-center gap-2.5">
            <AppIcon size={28} />
            <span className="flex flex-col">
              <span className="text-[15px] font-bold text-jn-text">Jolu</span>
              {about && <span className="tnum font-mono text-[11px] font-normal text-jn-text-3">v{about}</span>}
            </span>
          </span>
        }
        width="sm"
        actions={<Button kind="ghost" size="sm" onClick={() => setAbout(null)}>关闭</Button>}
      >
        <p className="text-xs leading-relaxed text-jn-text-2">
          在本机运行的 AI 编程工具管理器。统计来自本地会话记录与服务商接口，凭证只保存在本机。
        </p>
        <TactileButton
          variant="row"
          type="button"
          onClick={() => void openUrl(REPO_URL)}
          className="jn-interactive self-start rounded text-xs font-medium text-jn-primary-ink underline"
        >
          {REPO_URL}
        </TactileButton>
      </Dialog>
      <UpdateDialog onViewTasks={() => navigate("automation")} />
    </div>
    </TipProvider>
    </UpdateProvider>
    </ToastProvider>
    </MotionProvider>
  )
}
