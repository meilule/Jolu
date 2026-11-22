/**
 * 快照的后端命令路由：把每个 `invoke` 命令映射到示例数据。
 *
 * 设计原则：
 * - 读命令返回有内容的数据（页面才有东西可看）；
 * - 写命令返回成功并「就地改状态」，这样点开关 / 切页签也能看到反馈；
 * - 没登记的写命令返回 null，不抛错——快照不承担数据校验职责；
 * - 没登记的读命令返回空数组而不是 undefined，避免调用方 `Cannot read of undefined`。
 *
 * 未登记的命令会记录在 `unknownCommands`，快照页脚可以显示出来，便于补齐。
 */
import type { AppSettings } from "@/lib/api"
import { CONNECTIONS, DEMO_FX, NOW, HOUR, demoApiUsage, demoBalance, demoCandidates, demoCollectionStatus, demoConnectionKind, demoCostEstimate, demoLiveUsage, demoLocalCodexQuota, demoLocalGrokQuota, demoLocalZcodeQuota, demoLoginDetection, demoQuota, demoSettings, demoSourceBreakdown, demoSourceMetrics } from "./snapshotData"
import { DEMO_CONTINUE_CHECK, demoMessages, demoStatsActivity, demoStatsCalls, demoStatsOverview, demoSessions } from "./snapshotUsage"
import {
  DEMO_AUTOMATION_MODELS, demoAutomationAccounts, demoAutomationOverview, demoAutomationQuota, demoAutomationRunDetail,
  demoAutomationRuns, demoAutomationSkills, demoAutomationTask, demoCheckinClaim, demoCheckinOverview, demoCheckinStatus,
  demoSkillDetail, demoSkillFile, demoSkillImportScan, demoSkillsOverview,
} from "./snapshotModules"
import {
  DEMO_CLEANUP_PREVIEW, DEMO_IMPORT_PREVIEW, DEMO_IMPORT_SUMMARY, DEMO_OTLP_RESULT, DEMO_UPDATE_CHECK, DEMO_WARMUP_OUTCOME,
  demoAgentVersions, demoCatalog, demoClientKeys, demoContextHistory, demoContextItems,
  demoContextSessions, demoContextSnapshot, demoContextToolItems, demoDataInfo, demoDayOverview, demoGatewayGroups,
  demoGatewayStatus, demoLibraryList, demoLibrarySharedState, demoMiddlewares, demoNetworkDiagnosis, demoPrivacyRecords,
  demoProfiles, demoRouteLog, demoRouteLogPage, demoRoutingGroups, demoSearchVendors, demoSessionsOverview,
  demoSourceCapabilities, demoSourceUsage, demoStorageCounts, demoTodayCounts, demoTuneAdvice, demoWiringPreview,
  demoWslProbe, demoWslWiringState,
} from "./snapshotGateway"

type Args = Record<string, unknown>

const num = (value: unknown): number => (typeof value === "number" ? value : Number(value) || 0)
const str = (value: unknown): string => (typeof value === "string" ? value : "")

/** 快照内可变状态：设置与工具接入 */
const state: {
  settings: AppSettings
  wiring: Record<string, boolean>
  checkin: Set<string>
} = {
  settings: demoSettings(),
  wiring: Object.fromEntries(demoGatewayStatus().wiring.map((entry) => [entry.agent, entry.wired])),
  checkin: new Set(),
}

export function snapshotSettings(): AppSettings {
  return state.settings
}

/** 快照外壳的样式控制：改设置并返回最新快照（由入口广播 settings-changed） */
export function setSnapshotDock(edge: "top" | "bottom" | "left" | "right" | null): AppSettings {
  return patchSettings({ dock: { ...state.settings.dock, edge } })
}

export function setSnapshotStyle(style: "classic" | "capsule"): AppSettings {
  return patchSettings({ island_style: style })
}

export function setSnapshotTheme(theme: "light" | "dark"): AppSettings {
  return patchSettings({ theme })
}

function patchSettings(patch: Partial<AppSettings>): AppSettings {
  state.settings = { ...state.settings, ...patch, revision: (state.settings.revision ?? 0) + 1 }
  return state.settings
}

function statusWithWiring() {
  const base = demoGatewayStatus()
  return {
    ...base,
    wiring: base.wiring.map((entry) => {
      const wired = state.wiring[entry.agent] ?? entry.wired
      return { ...entry, wired, state: wired ? "wired" : "idle", drift: wired ? entry.drift : null }
    }),
  }
}

/**
 * 命令路由。返回值可以直接是数据，也可以是 Promise（少数命令刻意延迟，用来观察加载态）。
 */
export function routeCommand(cmd: string, args: Args): unknown {
  switch (cmd) {
    /* ─────────── 设置 ─────────── */
    case "get_settings": return state.settings
    case "set_theme": return patchSettings({ theme: args.theme as AppSettings["theme"] })
    case "set_default_page": return patchSettings({ default_page: str(args.page) })
    case "remember_page": state.settings = { ...state.settings, last_page: str(args.page) }; return null
    case "set_display_preferences":
      return patchSettings({
        island_opacity: num(args.opacity), island_scale: num(args.scale),
        island_shrink_scale: num(args.shrinkScale), refresh_minutes: num(args.refreshMinutes),
      })
    case "set_appearance_preferences":
      return patchSettings({ main_zoom: num(args.zoom), list_density: str(args.density), reduce_motion: Boolean(args.reduceMotion) })
    case "set_network": return patchSettings({ network_mode: str(args.mode), network_proxy_url: (args.proxyUrl as string | null) ?? null })
    case "set_proxy": return patchSettings({ proxy_enabled: Boolean(args.enabled), proxy_port: num(args.port), proxy_fallback_direct: Boolean(args.fallbackDirect) })
    case "set_task_notify":
      return patchSettings({
        notify_task_resumed: args.resumed === null ? state.settings.notify_task_resumed : Boolean(args.resumed),
        notify_task_finished: args.finished === null ? state.settings.notify_task_finished : Boolean(args.finished),
        notify_quota_alerts: args.quota === null ? state.settings.notify_quota_alerts : Boolean(args.quota),
      })
    case "set_island_visibility": return patchSettings({ island_visible: Boolean(args.visible) })
    case "island_topmost": return patchSettings({ always_on_top: Boolean(args.on) })
    case "set_dock_enabled": return patchSettings({ dock_enabled: Boolean(args.on) })
    case "set_dnd": return patchSettings({ dnd: Boolean(args.on) })
    case "set_island_style": return patchSettings({ island_style: str(args.style) as AppSettings["island_style"] })
    case "set_island_linkage": return patchSettings({ island_linkage: args.linkage as AppSettings["island_linkage"] })
    case "set_island_platform": return patchSettings({ island_platform: str(args.platform) })
    case "set_island_connection": return patchSettings({ island_connection_id: (args.id as string | null) ?? null })
    case "set_island_source": return patchSettings({ island_source_id: (args.id as string | null) ?? null })
    case "set_auto_check_updates": return patchSettings({ auto_check_updates: Boolean(args.on) })
    case "set_update_interval": return patchSettings({ update_interval_minutes: num(args.minutes) })
    case "set_cost_currency": return patchSettings({ cost_currency: str(args.currency) })
    case "set_silent_startup": return patchSettings({ silent_startup: Boolean(args.on) })
    case "set_autostart": return Boolean(args.on)
    case "autostart_status": return true
    case "set_retention_days": return patchSettings({ retention_days: (args.days as number | null) ?? null })
    case "set_balance_alert": return patchSettings({ balance_alert_threshold: (args.threshold as number | null) ?? null, balance_alert_currency: str(args.currency) })
    case "set_keep_awake": return patchSettings({ keep_awake: Boolean(args.on) })
    case "set_warmup": return patchSettings({ warmup_enabled: Boolean(args.enabled), warmup_hour: num(args.hour) })
    case "set_otlp_endpoint": return patchSettings({ otlp_endpoint: (args.endpoint as string | null) ?? null })
    case "privacy_masking_set": return patchSettings({ privacy_masking: Boolean(args.on) })
    case "privacy_policy_set": return patchSettings({ privacy_actions: args.actions as Record<string, string>, privacy_allow: args.allow as string[] })
    case "privacy_records": return demoPrivacyRecords()
    case "privacy_records_clear": return null
    case "zcode_free_first_set":
    case "zcode_set_channel": return state.settings
    case "search_vendors": return demoSearchVendors()
    case "search_apis_set": return patchSettings({ search_apis: args.apis as AppSettings["search_apis"] })
    case "search_try": return [
      { title: "Jolu · 统一模型入口", url: "https://example.com/jolu", snippet: "把多个订阅与 API Key 汇成一个模型入口，按分组分摊与失败转移。" },
      { title: "Agent 接线：Claude Code settings.json", url: "https://example.com/claude-settings", snippet: "ANTHROPIC_BASE_URL 与 ANTHROPIC_AUTH_TOKEN 写在 env 里。" },
    ]
    case "library_list": return demoLibraryList()
    case "libraryinstall":
    case "library_install": return { agent: str(args.agent), name: "jolu-search", created: true }
    case "library_remove": return true
    case "library_shared_state": return demoLibrarySharedState()
    case "library_instructions_save":
    case "library_subagent_save": return [{ agent: "claude-code", path: "~/.claude/CLAUDE.md", action: "written", reason: null }]
    case "library_subagent_remove": return [{ agent: "claude-code", path: null, action: "removed", reason: null }]
    case "warmup_last_outcome": return DEMO_WARMUP_OUTCOME
    case "otlp_export_usage": return DEMO_OTLP_RESULT
    case "agent_versions": return demoAgentVersions()
    case "wsl_probe_agents": return demoWslProbe()
    case "wsl_wiring_state": return demoWslWiringState()
    case "wsl_wire": return "已接入"
    case "wsl_unwire": return "已断开"
    case "tune_advice": return demoTuneAdvice()
    case "storage_counts": return demoStorageCounts()
    case "data_info": return demoDataInfo()
    case "open_data_directory": return null
    case "diagnose_network": return demoNetworkDiagnosis()
    case "proxy_status": return {
      running: false, port: 12731, fallback_reason: null, last_error: null,
      claude_upstream: null, codex_upstream: null, claude_taken_over: false, codex_taken_over: false,
    }
    case "fx_rate": return DEMO_FX

    /* ─────────── 更新 ─────────── */
    case "check_app_update_available":
    case "download_app_update": return DEMO_UPDATE_CHECK
    case "cancel_app_update_download": return null
    case "install_downloaded_update_and_restart": return null

    /* ─────────── 供应商与连接 ─────────── */
    case "list_connections": return CONNECTIONS
    case "current_connections": return ["conn-claude-momo", "conn-codex-alex"]
    case "connection_kind": return demoConnectionKind(str(args.platform))
    case "discover_connections": return demoCandidates()
    case "detect_local_login": return demoLoginDetection(str(args.platform))
    case "providers_import_scan": return {
      drafts: [
        { reference: "cc-switch:1", source: "cc-switch", from: "settings.json · env", name: "工作账号", baseUrl: "https://api.anthropic.com", protocol: "anthropic", keyMasked: "sk-…4d1e" },
        { reference: "claude-code:1", source: "claude-code", from: "~/.claude/settings.json", name: "个人账号", baseUrl: "https://api.anthropic.com", protocol: "anthropic", keyMasked: "sk-…9b20" },
        { source: "codex", error: "配置文件被其他程序占用" },
      ],
    }
    case "providers_import_apply": return ["工作账号", "个人账号"]
    case "fetch_credentials": return { ok: true, masked: "sk-…4d1e", message: "已更新本机配置里的 Key", scanned: ["~/.claude/settings.json"], changed: true }
    case "replace_api_key": return null
    case "rename_connection": return null
    case "update_connection": return null
    case "remove_connection": return null
    case "add_connection": return `conn-${Date.now()}`
    case "enable_connection": return "已写入对应工具的配置文件"
    case "set_connection_paused": return null
    case "read_local_connections": return { added: 1, existing: 1, warnings: [] }
    case "start_browser_authorization": return "snap-auth"
    case "browser_authorization_status": return { state: "saved", detection: demoLoginDetection("trae"), message: null }
    case "cancel_browser_authorization": return null
    case "save_browser_connection": return `conn-${Date.now()}`
    case "fetch_remote_models": return ["deepseek-v4.1", "deepseek-v4.1-mini", "deepseek-r2"]
    case "verify_custom_model": return null
    case "connection_quota": return demoQuota(str(args.id))
    case "connection_balance": return demoBalance(str(args.id))
    case "connection_api_usage": return demoApiUsage(str(args.id))
    case "local_codex_quota": return demoLocalCodexQuota()
    case "local_zcode_quota": return demoLocalZcodeQuota()
    case "local_grok_quota": return demoLocalGrokQuota()

    /* ─────────── 用量 ─────────── */
    case "stats_overview": return demoStatsOverview(num((args.filter as Args)?.start))
    case "stats_activity": return demoStatsActivity()
    case "stats_calls": return demoStatsCalls(num(args.page))
    case "export_calls_csv": return { path: "C:/Users/demo/Downloads/jolu-calls.csv", count: 846 }
    case "source_metrics": return demoSourceMetrics()
    case "cost_estimate": return demoCostEstimate()
    case "island_source_breakdown": return demoSourceBreakdown()

    /* ─────────── 会话 ─────────── */
    case "sessions_list": return demoSessions()
    case "session_messages": return demoMessages(str(args.platform), str(args.sessionId))
    case "session_continue_check": return DEMO_CONTINUE_CHECK
    case "session_continue": return null
    case "scan_local_sessions": return { files_scanned: 42, records_inserted: 12, claude_found: true, codex_found: true, errors: [] }
    case "collection_status": return demoCollectionStatus()
    case "live_usage": return demoLiveUsage(str(args.platform))

    /* ─────────── 签到 ─────────── */
    case "daily_checkin_overview": return demoCheckinOverview()
    case "daily_checkin_status": return demoCheckinStatus(str(args.id))
    case "daily_checkin_claim": return demoCheckinClaim(str(args.id))

    /* ─────────── Skills ─────────── */
    case "skills_overview": return demoSkillsOverview(str(args.platform))
    case "skill_detail": {
      const overview = demoSkillsOverview("claude")
      const path = str(args.path)
      const item = overview.items.find((entry) => entry.path === path) ?? overview.items[0]!
      return demoSkillDetail(item)
    }
    case "skill_file": {
      const overview = demoSkillsOverview("claude")
      const path = str(args.path)
      const item = overview.items.find((entry) => entry.path === path) ?? overview.items[0]!
      return demoSkillFile(item, str(args.rel))
    }
    case "skill_set_enabled": {
      const enabled = Boolean(args.enabled)
      return `${str(args.path)}${enabled ? "" : "（已停用）"}`
    }
    case "skill_save": return { path: str((args.input as Args)?.original_path), modified_ms: Date.now(), backup: null }
    case "skill_copy": return { target: "~/.claude/skills/frontend-design", copied: true, conflict: false, issue: null }
    case "skill_remove": return "已移到应用数据目录的备份区"
    case "skills_import_scan": return demoSkillImportScan(str(args.project) || "C:/Users/demo/.claude/skills")
    case "skills_import": return { imported: ["api-review"], skipped: ["frontend-design"], failed: [] }
    case "skills_import_git": return {
      checkout_dir: "C:/Users/demo/AppData/Local/Temp/jolu-skill-git-1",
      scan: demoSkillImportScan("C:/Users/demo/.claude/skills"),
    }
    case "skills_import_git_cleanup": return null
    case "skills_set_projects": return args.projects as string[]
    case "skills_ignore_project": return null
    case "skill_open_dir": return null
    case "skills_pick_folder":
    case "automation_pick_dir": return null
    case "skills_save_dir_settings": return null

    /* ─────────── 自动化 ─────────── */
    case "automation_overview": return demoAutomationOverview()
    case "automation_waiting_connections": return ["demo-momo"]
    case "automation_task": return demoAutomationTask(num(args.id))
    case "automation_runs": return demoAutomationRuns(num(args.page))
    case "automation_run": return demoAutomationRunDetail(num(args.id))
    case "automation_accounts": return demoAutomationAccounts(str(args.platform) as "claude" | "codex")
    case "automation_models": return DEMO_AUTOMATION_MODELS[str(args.platform) as "claude" | "codex"] ?? []
    case "automation_quota": return demoAutomationQuota()
    case "automation_tools": return [{ platform: "claude", found: true }, { platform: "codex", found: true }]
    case "automation_skills": {
      const platform = str(args.platform) as "claude" | "codex"
      return demoAutomationSkills(platform, (args.cwd as string | null) ?? null)
    }
    case "automation_create": return 30
    case "automation_update": return null
    case "automation_action": return null
    case "automation_delete": return null

    /* ─────────── Agent 与模型路由 ─────────── */
    case "gateway_status": return statusWithWiring()
    case "gateway_set_mode": return { mode: str(args.mode), enabled: args.mode === "relay" }
    case "gateway_set_default_source": return args.connectionId ?? null
    case "gateway_wiring_set": {
      const agent = str(args.agent)
      state.wiring[agent] = Boolean(args.wired)
      return `已${args.wired ? "接入" : "断开"} ${agent}`
    }
    case "gateway_wiring_applied": return {
      model: "jolu/claude-momo/claude-sonnet-4-5", subagent_model: "jolu/claude-momo/claude-haiku-4-5",
      tiers: { opus: "jolu/claude-momo/claude-opus-4-1", sonnet: "jolu/claude-momo/claude-sonnet-4-5", haiku: "jolu/claude-momo/claude-haiku-4-5", fable: null, autoMode: null },
    }
    case "gateway_wiring_preview": return demoWiringPreview("wire")
    case "gateway_wiring_preflight": return []
    case "gateway_wiring_accept": return null
    case "gateway_escape": return "已还原全部接线并停止模型路由"
    case "gateway_catalog": return demoCatalog()
    case "gateway_groups": return demoGatewayGroups()
    case "gateway_routing_group_list": return demoRoutingGroups()
    case "gateway_routing_group_create":
    case "gateway_routing_group_update":
    case "gateway_routing_group_delete": return null
    case "gateway_group_create": return { id: `g-${Date.now()}`, name: str(args.name), sort: 9, members: [] }
    case "gateway_group_rename":
    case "gateway_group_delete":
    case "gateway_group_move":
    case "gateway_group_reorder": return null
    case "gateway_set_slug": return str(args.slug)
    case "gateway_source_capabilities": return demoSourceCapabilities()
    case "gateway_source_usage": return demoSourceUsage()
    case "gateway_set_serve": return null
    case "gateway_set_serve_many": return (args.connectionIds as string[]).map((id) => ({ connection_id: id, ok: true, error: null }))
    case "gateway_refresh_models": return demoCatalog().map((m) => m.model)
    case "gateway_day_overview": return demoDayOverview(str(args.range) || "today")
    case "gateway_route_log": return demoRouteLog()
    case "gateway_route_log_page": return demoRouteLogPage(num(args.page), num(args.pageSize) || 50)
    case "gateway_today_counts": return demoTodayCounts()
    case "gateway_sessions_overview": return demoSessionsOverview()
    case "gateway_context_sessions": return demoContextSessions()
    case "gateway_context_snapshot": return demoContextSnapshot(str(args.platform), str(args.sessionId), (args.snapshotId as number | null) ?? null)
    case "gateway_context_items": return demoContextItems(str(args.kind))
    case "gateway_context_tool_items": return demoContextToolItems()
    case "gateway_context_history": return demoContextHistory()
    case "gateway_client_keys_list": return demoClientKeys()
    case "gateway_client_key_issue": return {
      name: str(args.name), key: "jolu-8f27c1d4e9b0a3f6", tokens_per_day: (args.tokensPerDay as number | null) ?? null,
      requests_per_day: (args.requestsPerDay as number | null) ?? null, expires_ms: (args.expiresMs as number | null) ?? null,
      models: (args.models as string[] | null) ?? null, read_only: Boolean(args.readOnly),
    }
    case "gateway_client_key_revoke":
    case "gateway_client_key_limits_set":
    case "gateway_client_key_concurrency_set": return null
    case "gateway_lan_address": return "192.168.1.24"
    case "gateway_allow_ips_from_env": return false
    case "gateway_allow_ips_set": return null
    case "gateway_shared_lan_set": return { gateway_shared_lan: Boolean(args.on) }
    case "gateway_access_key": return "jolu-local-access-7d21"
    case "gateway_copy_key": return null
    case "gateway_middleware_list": return demoMiddlewares()
    case "gateway_middleware_save": return demoMiddlewares()[0]
    case "gateway_middleware_delete": return null
    case "gateway_profiles_list": return demoProfiles()
    case "gateway_profiles_save": return { name: str(args.name), agents: {}, created_ms: Date.now() }
    case "gateway_profiles_delete": return true
    case "gateway_profiles_apply": return { switched: ["claude-code", "codex"], skipped: [], message: "已应用配置方案" }
    case "gateway_models_dev_status": return [1_284, NOW - 6 * HOUR]
    case "gateway_models_dev_refresh": return [1_312, NOW]
    case "gateway_long_context_set": return str(args.value)
    case "custom_agents_list": return []
    case "custom_agents_add": return { id: "custom-1", name: str((args.input as Args)?.name), slug: "custom", config_path: "", format: "json", base_url_key: "baseURL", auth_key: "apiKey", model_key: "model", base_suffix: "", installed: true, wired: false, drift: null, model: null }
    case "custom_agents_remove":
    case "custom_agents_wire":
    case "custom_agents_unwire": return null
    case "custom_agents_preview": return demoWiringPreview("wire")

    /* ─────────── 导入 / 清理 ─────────── */
    case "preview_import_file": return DEMO_IMPORT_PREVIEW
    case "confirm_import": return DEMO_IMPORT_SUMMARY
    case "discard_import": return null
    case "cleanup_preview":
    case "cleanup_history": return { ...DEMO_CLEANUP_PREVIEW, cutoff_ms: Date.now() - num(args.days) * 86_400_000 }
    case "export_data_file": return "C:/Users/demo/Downloads/jolu-data.json"

    /* ─────────── 主面板与灵动岛操作 ─────────── */
    case "main_panel_ready": return null
    case "take_main_intent": return null
    case "take_app_notices": return []
    case "main_drag":
    case "island_drag":
    case "dock_undock":
    case "island_position":
    case "island_action":
    case "island_menu":
    case "menu_action":
    case "menu_show":
    case "menu_close":
    case "menu_fit": return cmd === "menu_fit" ? { root_x: 0, root_y: 0, root_width: 240, sub_width: 240, side: "right" } : null
    case "menu_refreshing": return false
    case "resize_island": return null
    case "set_main_context": return null
    case "main_context": return null
    case "open_main_section":
    case "open_main_session":
    case "open_main_usage":
    case "open_zcode_client": return null
    case "retry_startup": return null

    default:
      return undefined
  }
}
