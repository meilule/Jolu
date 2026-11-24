/**
 * 静态快照的假数据（全部是设计示例，不对应任何真实账号或本机状态）。
 *
 * 数值取自现有浏览器预览的示例（`mock/`）与设计稿画板示例，风格保持一致；
 * 时间以「快照生成时刻」为基准推出来，保证列表里的「几分钟前」永远自然。
 */
import type {
  ApiUsageStateDto, AppSettings, BalanceStateDto, CollectionStatusDto, ConnectionDto, LiveUsageDto,
  LoginDetectionDto, QuotaStateDto, QuotaWindowDto, SourceBreakdownDto, SourceMetrics,
} from "@/lib/api"

const MIN = 60_000
const DAY = 24 * 60 * MIN

export const NOW = Date.now()
export const HOUR = 60 * MIN

const at = (daysAgo: number, h: number, m: number): number => {
  const d = new Date(NOW)
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() - daysAgo, h, m).getTime()
}
const iso = (ms: number): string => new Date(ms).toISOString()

/* ─────────────────────────── 设置 ─────────────────────────── */

export function demoSettings(): AppSettings {
  return {
    revision: 7,
    silent_startup: true,
    claude_long_context: "",
    auto_check_updates: true,
    update_interval_minutes: 30,
    cost_currency: "USD",
    island_platform: "claude",
    island_kind: "auth",
    island_connection_id: "conn-claude-momo",
    island_connection_name: "Momo",
    island_source_id: "local:claude",
    always_on_top: true,
    theme: "light",
    island_opacity: 100,
    island_scale: 100,
    island_shrink_scale: 100,
    island_style: "classic",
    refresh_minutes: 5,
    balance_alert_threshold: 5,
    balance_alert_currency: "USD",
    dock_enabled: true,
    island_horizontal_dock_full: true,
    retention_days: 180,
    dock: { edge: null, offset: 0, monitor: "DISPLAY1" },
    dnd: false,
    island_visible: true,
    island_clones: [],
    island_linkage: { strict_writing: false, signals: true, follow_main: false, zcode_provider: null, auto_focus: false },
    proxy_enabled: false,
    proxy_port: 12731,
    proxy_fallback_direct: true,
    proxy_claude_upstream: null,
    proxy_codex_upstream: null,
    default_page: "last",
    last_page: "providers",
    main_zoom: 100,
    list_density: "comfortable",
    reduce_motion: false,
    network_mode: "system",
    network_proxy_url: null,
    notify_task_resumed: true,
    notify_task_finished: true,
    notify_quota_alerts: true,
    privacy_masking: true,
    privacy_actions: { phone: "warn" },
    privacy_allow: ["localhost", "127.0.0.1"],
    zcode_free_first: false,
    gateway_allow_ips: [],
    search_apis: [],
    keep_awake: false,
    warmup_enabled: false,
    warmup_hour: 9,
    warmup_last_date: null,
    otlp_endpoint: null,
  }
}

/* ─────────────────────────── 供应商与连接 ─────────────────────────── */

export const CONNECTIONS: ConnectionDto[] = [
  {
    id: "conn-claude-momo", platform: "claude", kind: "auth", name: "Momo", label: "Claude Max",
    masked: "", status: "connected", last_sync_text: "刚刚同步", base_url: null, model: null, effort: null,
    context_1m: null, protocol: null, account_email: "momo@example.com", serve_enabled: true, slug: "claude-momo",
    start_channel: null,
  },
  {
    id: "conn-claude-team", platform: "claude", kind: "auth", name: "Team", label: "Claude Team",
    masked: "", status: "connected", last_sync_text: "3 分钟前", base_url: null, model: null, effort: null,
    context_1m: null, protocol: null, account_email: "team@example.com", serve_enabled: false, slug: "claude-team",
    start_channel: null,
  },
  {
    id: "conn-codex-alex", platform: "codex", kind: "auth", name: "Alex", label: "ChatGPT Plus",
    masked: "", status: "connected", last_sync_text: "刚刚同步", base_url: null, model: null, effort: null,
    context_1m: null, protocol: null, account_email: "alex@example.com", serve_enabled: true, slug: "codex-alex",
    start_channel: null,
  },
  {
    id: "conn-zcode", platform: "zcode", kind: "auth", edition: "zai", name: "ZCode", label: "Coding Plan",
    masked: "", status: "connected", last_sync_text: "5 分钟前", base_url: null, model: null, effort: null,
    context_1m: null, protocol: null, account_email: "kai@example.com", serve_enabled: false, slug: "zcode-zai",
    start_channel: null,
  },
  {
    id: "conn-gemini-key", platform: "gemini", kind: "api", name: "Gemini 工作 Key", label: "API Key",
    masked: "sk-…9f2a", status: "connected", last_sync_text: "12 分钟前",
    base_url: "https://generativelanguage.googleapis.com/v1beta/openai", model: "gemini-2.5-flash",
    effort: null, context_1m: null, protocol: "openai", serve_enabled: true, slug: "gemini-work", start_channel: null,
  },
  {
    id: "conn-custom-deepseek", platform: "custom", kind: "api", name: "DeepSeek", label: "API Key",
    masked: "sk-…41c7", status: "connected", last_sync_text: "20 分钟前",
    base_url: "https://api.deepseek.com/v1", model: "deepseek-v4.1", effort: null, context_1m: null,
    protocol: "openai", serve_enabled: true, slug: "deepseek", start_channel: null,
  },
  {
    id: "conn-qoder-cn", platform: "qoder", kind: "auth", edition: "cn", name: "Qoder CN", label: "Pro",
    masked: "", status: "connected", last_sync_text: "1 小时前", base_url: null, model: null, effort: null,
    context_1m: null, protocol: null, account_email: "momo@example.com", serve_enabled: false, slug: "qoder-cn",
    start_channel: null,
  },
  {
    id: "conn-trae-global", platform: "trae", kind: "auth", edition: "global", name: "Trae", label: "Free",
    masked: "", status: "expired", last_sync_text: "2 天前", base_url: null, model: null, effort: null,
    context_1m: null, protocol: null, account_email: "alex@example.com", serve_enabled: false, slug: null,
    start_channel: null,
  },
]

export function demoConnectionKind(platform: string): { kind: "auth" | "api" | null; label: string | null } {
  const row = CONNECTIONS.find((c) => c.platform === platform && c.status === "connected")
  return row ? { kind: row.kind, label: row.label } : { kind: null, label: null }
}

export function demoCandidates() {
  return [
    { platform: "claude", kind: "auth" as const, suggested_name: "本机 Claude 登录", masked: "sk-…4d1e" },
    { platform: "codex", kind: "auth" as const, suggested_name: "本机 Codex 登录", masked: "sk-…7b02" },
  ]
}

export function demoLoginDetection(platform: string): LoginDetectionDto {
  return {
    found: platform === "claude" || platform === "codex",
    masked: platform === "claude" ? "sk-…4d1e" : platform === "codex" ? "sk-…7b02" : null,
    account: platform === "claude" ? "momo@example.com" : platform === "codex" ? "alex@example.com" : null,
    plan: platform === "claude" ? "Claude Max" : platform === "codex" ? "ChatGPT Plus" : null,
    sources: platform === "claude" ? ["~/.claude/settings.json", "keychain"] : platform === "codex" ? ["~/.codex/auth.json"] : [],
    message: null,
  }
}

/* ─────────────────────────── 额度 / 余额 ─────────────────────────── */

const fiveHour = (used: number, minutes: number): QuotaWindowDto => ({
  key: "5h", window_name: "5 小时窗口", used_percent: used,
  amount_text: `已用 ${Math.round((used / 100) * 12)}k / 12k 消息`,
  remaining_text: null, resets_at: iso(NOW + minutes * MIN),
})

const week = (used: number, days: number): QuotaWindowDto => ({
  key: "7d", window_name: "7 天窗口", used_percent: used,
  amount_text: `已用 ${Math.round((used / 100) * 750)}k / 750k 消息`,
  remaining_text: null, resets_at: iso(NOW + days * DAY),
})

export function demoQuota(connectionId: string): QuotaStateDto {
  const base = {
    state: "ok" as const,
    plan: null as string | null,
    account: {
      subscription_status: "active" as const,
      expires_at: iso(NOW + 21 * DAY),
      subscription_source: "live" as const,
      observed_at: iso(NOW - 2 * MIN),
      reset_credits: null,
      email: null,
    },
  }
  switch (connectionId) {
    case "conn-claude-momo":
      return { ...base, plan: "Claude Max", windows: [fiveHour(41, 212), week(63, 3)] }
    case "conn-claude-team":
      return { ...base, plan: "Claude Team", windows: [fiveHour(18, 260), week(37, 5)] }
    case "conn-codex-alex":
      return {
        ...base, plan: "ChatGPT Plus",
        windows: [
          { key: "5h", window_name: "5 小时窗口", used_percent: 22, amount_text: null, remaining_text: "剩余 78%", resets_at: iso(NOW + 143 * MIN) },
          { key: "7d", window_name: "7 天窗口", used_percent: 44, amount_text: null, remaining_text: "剩余 56%", resets_at: iso(NOW + 4 * DAY) },
        ],
      }
    case "conn-zcode":
      return {
        ...base, plan: "Coding Plan",
        windows: [
          { key: "5h", window_name: "5 小时额度", used_percent: 34, amount_text: null, remaining_text: null, resets_at: iso(NOW + 96 * MIN) },
          { key: "7d", window_name: "周额度", used_percent: 51, amount_text: null, remaining_text: null, resets_at: iso(NOW + 3 * DAY) },
          { key: "1d:glm-5.3", window_name: "GLM-5.3 · 每日额度", used_percent: 62, amount_text: "1.24M / 2.00M", remaining_text: "剩 760K", resets_at: iso(at(0, 24, 0)) },
          { key: "1d:glm-5.3-air", window_name: "GLM-5.3-Air · 每日额度", used_percent: 12, amount_text: "240K / 2.00M", remaining_text: "剩 1.76M", resets_at: iso(at(0, 24, 0)) },
        ],
      }
    case "conn-gemini-key":
      return { ...base, plan: "Gemini API", windows: [{ key: "gemini:gemini-2.5-flash", window_name: "gemini-2.5-flash", used_percent: 23, amount_text: null, remaining_text: "剩余 77%", resets_at: iso(at(1, 0, 0)) }] }
    case "conn-qoder-cn":
      return {
        ...base, plan: "Qoder Pro",
        windows: [
          { key: "qoder-credits", window_name: "Credits · 本月", used_percent: 38, amount_text: null, remaining_text: "剩 620 Credits", resets_at: iso(at(0, 0, 0) + 30 * DAY) },
          { key: "qoder-daily", window_name: "每日请求", used_percent: 7, amount_text: "7 / 100", remaining_text: null, resets_at: iso(at(1, 0, 0)) },
        ],
      }
    case "conn-custom-deepseek":
      return {
        ...base, plan: "DeepSeek API",
        windows: [{ key: "1d", window_name: "今日额度", used_percent: null, amount_text: null, remaining_text: null, resets_at: null }],
      }
    case "conn-trae-global":
      return { state: "unauthorized", reason: "登录已失效，请在供应商页重新连接" }
    default:
      return { state: "unsupported", reason: "该来源不提供额度查询" }
  }
}

export function demoBalance(connectionId: string): BalanceStateDto {
  switch (connectionId) {
    case "conn-gemini-key": return { state: "ok", balance: 42.68, currency: "USD", used: 17.32 }
    case "conn-custom-deepseek": return { state: "ok", balance: 18.4, currency: "USD", used: 6.15 }
    default: return { state: "unsupported", reason: "该连接没有可查询的余额" }
  }
}

export function demoApiUsage(connectionId: string): ApiUsageStateDto {
  if (connectionId !== "conn-gemini-key") return { state: "forbidden", reason: "普通 Key 没有组织用量权限" }
  return {
    state: "ok", input_tokens: 812_400, output_tokens: 268_900, cache_read_tokens: 384_200,
    cache_write_tokens: 0, total_tokens: 1_081_300, cost_usd: 2.38,
    cost_reason: "按官方单价估算", source: "Gemini API · 组织用量",
  }
}

export function demoLocalCodexQuota(): QuotaStateDto {
  return {
    state: "ok", plan: "ChatGPT Plus",
    windows: [
      { key: "5h", window_name: "5 小时窗口", used_percent: 22, amount_text: null, remaining_text: "剩余 78%", resets_at: iso(NOW + 143 * MIN) },
      { key: "7d", window_name: "7 天窗口", used_percent: 44, amount_text: null, remaining_text: "剩余 56%", resets_at: iso(NOW + 4 * DAY) },
    ],
    account: null,
  }
}

export function demoLocalZcodeQuota(): QuotaStateDto {
  return demoQuota("conn-zcode")
}

export function demoLocalGrokQuota(): QuotaStateDto {
  return {
    state: "ok", plan: "SuperGrok",
    windows: [
      { key: "7d", window_name: "每周额度", used_percent: 29, amount_text: null, remaining_text: null, resets_at: iso(NOW + 2 * DAY) },
      { key: "credits", window_name: "Credits", used_percent: 16, amount_text: "84 / 100", remaining_text: "剩 84", resets_at: null },
    ],
    account: null,
  }
}

export const DEMO_FX = { currency: "CNY", rate: 7.12, as_of: "2026-11-22", fallback: false }

/* ─────────────────────────── 灵动岛实时用量 ─────────────────────────── */

export function demoLiveUsage(platform: string): LiveUsageDto {
  const sessions = platform === "claude"
    ? [
        { session_id: "snap-claude-1", title: "排查缓存 Token 统计", delta_tokens: 8_400, last_seen_ms: NOW - 22_000, started_at_ms: NOW - 26 * MIN, running_tokens: 46_200, state: "running" as const },
        { session_id: "snap-claude-2", title: "整理 Skills 管理方案", delta_tokens: 3_200, last_seen_ms: NOW - 3 * MIN, started_at_ms: NOW - 48 * MIN, running_tokens: 18_900, state: "running" as const },
      ]
    : [
        { session_id: `snap-${platform}-1`, title: "优化 Jolu 的会话界面", delta_tokens: 5_200, last_seen_ms: NOW - 90_000, started_at_ms: NOW - 34 * MIN, running_tokens: 21_400, state: "running" as const },
      ]
  const total = sessions.reduce((sum, s) => sum + (s.running_tokens ?? 0), 0)
  return {
    platform, cursor: 42, delta_tokens: sessions.reduce((sum, s) => sum + s.delta_tokens, 0),
    delta_sessions: sessions.map((s) => ({ session_id: s.session_id, title: s.title, delta_tokens: s.delta_tokens, last_seen_ms: s.last_seen_ms, state: s.state, running_tokens: s.running_tokens })),
    active_sessions: sessions, running_tokens: total, active_window_seconds: 90,
    today_tokens: platform === "claude" ? 12_840_000 : 3_260_000, realtime_delta: true, initial: false,
    data_changed: true, attribution: { connection_id: `conn-${platform === "claude" ? "claude-momo" : "codex-alex"}`, name: platform === "claude" ? "Momo" : "Alex", confidence: "exact" },
  }
}

export function demoSourceBreakdown(): SourceBreakdownDto {
  return {
    rows: [
      { connection_id: "conn-claude-momo", name: "Momo", requests: 486, tokens: 8_620_000 },
      { connection_id: "conn-claude-team", name: "Team", requests: 212, tokens: 3_140_000 },
    ],
    total_requests: 742, unattributed_requests: 104,
  }
}

export function demoSourceMetrics(): SourceMetrics {
  return { credits: 1_284.5, credit_requests: 42, context_ratio: 0.62, context_at_ms: NOW - 4 * MIN, reasoning: 86_400, tool: 32_100 }
}

export function demoCostEstimate() {
  return { amount: 1.86, estimated: true, uncovered_tokens: 0, complete: true }
}

export function demoCollectionStatus(): CollectionStatusDto {
  return { ok: true, errors: [], last_success_ms: NOW - 8_000, failed_sources: [], consecutive_failures: 0 }
}
