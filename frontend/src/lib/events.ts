/**
 * 后端发给前端的事件：名字与载荷类型的唯一来源。
 *
 * 与 `backend/src/events.rs` 一一对应（后端测试 `names_match_frontend_table` 核对两边名字集合一致，
 * 改名、增删要两边一起改）。订阅一律用 `listenEvent(EVENTS.X, handler)`，载荷类型按事件名自动推断，
 * 写错名字编译就报错。目录与含义见 `docs/specs/2026-11-16-baseline-and-contracts.md` §4。
 */
import type {
  AgentHookAlertDto,
  AppSettings,
  CollectionStatusDto,
  DockChangedEvent,
  DockHintEvent,
  LiveUsageDto,
  LocalActiveAccountDto,
  MenuSourceEvent,
  UpdateProgressDto,
} from "@/lib/api"
import type { RouteEndEvent, RouteStartEvent } from "@/lib/gatewayActivity"
import type { ChatEventDto } from "@/lib/sessionHandoff"

export const EVENTS = {
  SETTINGS_CHANGED: "settings-changed",
  CONNECTIONS_CHANGED: "connections-changed",
  REFRESH_REQUESTED: "refresh-requested",
  COLLECTION_STATUS: "collection-status",
  LIVE_USAGE: "live-usage",
  GATEWAY_ROUTE_START: "gateway-route-start",
  GATEWAY_ROUTE: "gateway-route",
  GATEWAY_WIRING_CHANGED: "gateway-wiring-changed",
  GATEWAY_CATALOG_CHANGED: "gateway-catalog-changed",
  AUTOMATION_CHANGED: "automation-changed",
  WARMUP_FINISHED: "warmup-finished",
  DAILY_CHECKIN_CHANGED: "daily-checkin-changed",
  LIBRARY_CHANGED: "library-changed",
  PRIVACY_FIRST_MASK: "privacy-first-mask",
  HANDOFF_CHAT: "handoff-chat",
  DOCK_CHANGED: "dock-changed",
  DOCK_HINT: "dock-hint",
  ISLAND_POINTER: "island-pointer",
  ISLAND_BLUR: "island-blur",
  ISLAND_REFRESH: "island-refresh",
  MAIN_INTENT: "main-intent",
  APP_NOTICE: "app-notice",
  UPDATE_DOWNLOAD_PROGRESS: "update-download-progress",
  MENU_REOPEN: "menu-reopen",
  MENU_STATE_CHANGED: "menu-state-changed",
  TRAY_SUMMARY_OPEN: "tray-summary-open",
  TRAY_SUMMARY_CLOSE: "tray-summary-close",
  STARTUP_ERROR: "startup-error",
  MAIN_CONTEXT: "main-context",
  PRICING_CHANGED: "pricing-changed",
  ACTIVE_ACCOUNT_CHANGED: "active-account-changed",
  AGENT_HOOK_CHANGED: "agent-hook-changed",
} as const

/** 最近一次 warm-up 的结果（`warmup-finished` 载荷，也是 `warmup_last_outcome` 的返回） */
export interface WarmupOutcomeDto {
  status: "ok" | "skipped" | "failed"
  /** 实际执行用的 agent；跳过时为 null */
  platform: string | null
  /** 直接给用户看的说明 */
  detail: string
  finished_ms: number
}

/** 主窗口当前在看的页面、连接与物理中心点（`set_main_context` / `main_context`）；离开有连接的页面时 connection_id 为 null */
export interface MainContextDto {
  page: string
  connection_id: string | null
  main_center?: [number, number] | null
}

/** 压缩会话对话式修订的流式片段 */
export interface HandoffChatEvent {
  job_id: string
  event: ChatEventDto
}

/** 事件名 → 载荷类型。没有载荷的事件后端发 `()`，前端收到 `null`。 */
export interface EventPayloads {
  /** 整份设置快照，带修订号（见 `lib/settingsRevision.ts`） */
  "settings-changed": AppSettings
  "connections-changed": null
  /** `null` = 全部刷新；连接 id = 只刷这个连接 */
  "refresh-requested": string | null
  "collection-status": CollectionStatusDto
  "live-usage": LiveUsageDto
  "gateway-route-start": RouteStartEvent
  "gateway-route": RouteEndEvent
  "gateway-wiring-changed": null
  /** 模型目录可能变了：重读 `gateway_catalog` */
  "gateway-catalog-changed": null
  "automation-changed": null
  "warmup-finished": WarmupOutcomeDto
  /** 签到结果变化的连接 id */
  "daily-checkin-changed": string
  "library-changed": null
  "privacy-first-mask": null
  "handoff-chat": HandoffChatEvent
  "dock-changed": DockChangedEvent
  "dock-hint": DockHintEvent
  /** 屏幕物理坐标 */
  "island-pointer": { x: number; y: number }
  /** 失焦的岛 id；本体为 null */
  "island-blur": string | null
  "island-refresh": null
  "main-intent": null
  "app-notice": null
  "update-download-progress": UpdateProgressDto
  "menu-reopen": MenuSourceEvent
  "menu-state-changed": null
  "tray-summary-open": null
  "tray-summary-close": null
  /** 启动失败的说明文字 */
  "startup-error": string
  /** 主窗口当前在看哪个连接（4.2b「跟随主窗口」） */
  "main-context": MainContextDto
  "pricing-changed": null
  /** 本机 CLI 生效凭证变化（3A 外部切号跟随 + 陌生 Key 防呆） */
  "active-account-changed": LocalActiveAccountDto[]
  /** 受控轻量级 Agent Hooks 状态回执变化（4B） */
  "agent-hook-changed": AgentHookAlertDto[]
}

export type EventName = keyof EventPayloads

// 两张表必须覆盖同一组名字：少写或多写都在这里编译失败
type EventsTableNames = (typeof EVENTS)[keyof typeof EVENTS]
const _sameNames: [EventsTableNames] extends [EventName] ? ([EventName] extends [EventsTableNames] ? true : never) : never = true
void _sameNames
