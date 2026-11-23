/**
 * Agent、模型路由、设置杂项与灵动岛形态的示例数据（静态快照）。
 *
 * Agent 与模型路由在浏览器预览里本来是空态（只有桌面端可用），快照要展示真实界面，
 * 因此这里按后端 DTO 形状造出「已接入 + 可接入 + 暂不支持」三类，与画板 V8-01 / V6 对齐。
 */
import type {
  AgentDiscoveryDto, CatalogModelDto, ClientKeySummaryDto, ContextItemRowDto, ContextSessionRowDto,
  ContextSnapshotDetailDto, ContextSnapshotSummaryDto, ContextToolResultDto, DayOverviewDto, GatewayGroupsDto,
  GatewayStatusDto, MiddlewareDto, ProfileDto, RouteLogDto, RouteLogPageDto, SessionsOverviewDto,
  SourceCapabilityDto, SourceUsageDto, WiringPreviewDto,
} from "@/lib/gatewayApi"
import type { RoutingGroupDto } from "@/components/gateway/routingGroups"
import type { AgentVersionRow, WslProbeDto } from "@/lib/settingsApi"
import type { LibraryAgentDto } from "@/lib/searchSettings"
import type { LibrarySharedStateDto } from "@/lib/librarySharedView"
import type { SearchVendorDto } from "@/lib/searchSettings"
import type { TuneAgentDto } from "@/lib/tuneAdvice"
import type { WslWiringStateDto } from "@/lib/wslWiring"
import type { NetworkDiagnosisDto } from "@/lib/networkSettings"
import type { MaskRecordDto } from "@/components/settings/privacyMasking"
import { CONNECTIONS, NOW } from "./snapshotData"

const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

/* ─────────────────────────── Agent 探测与接线 ─────────────────────────── */

const DISCOVERY: AgentDiscoveryDto[] = [
  { agent: "claude-code", name: "Claude Code", installed: true, state: "installed", evidence: "config_file", cli_found: true, cli_path: "C:/Users/demo/.local/bin/claude" },
  { agent: "codex", name: "Codex", installed: true, state: "installed", evidence: "config_file", cli_found: true, cli_path: "C:/Users/demo/AppData/Local/OpenAI/Codex/bin/codex.exe" },
  { agent: "gemini", name: "Gemini CLI", installed: true, state: "installed", evidence: "config_dir", cli_found: true, cli_path: "C:/Users/demo/AppData/Roaming/npm/gemini.cmd" },
  { agent: "opencode", name: "OpenCode", installed: true, state: "installed", evidence: "config_dir", cli_found: true, cli_path: "C:/Users/demo/.opencode/bin/opencode.exe" },
  { agent: "zcode", name: "ZCode", installed: true, state: "installed", evidence: "config_dir", cli_found: true, cli_path: "C:/Users/demo/.zcode/bin/zcode.cmd" },
  { agent: "kimi", name: "Kimi CLI", installed: true, state: "installed", evidence: "config_file", cli_found: false, cli_path: null },
  { agent: "dsh", name: "DeepSeek Harness", installed: true, state: "installed", evidence: "config_dir", cli_found: true, cli_path: "C:/Users/demo/.bun/bin/dsh.exe" },
  { agent: "qwen", name: "Qwen Code", installed: true, state: "installed", evidence: "config_file", cli_found: true, cli_path: "C:/Users/demo/AppData/Roaming/npm/qwen.cmd" },
  { agent: "workbuddy", name: "WorkBuddy", installed: true, state: "installed", evidence: "config_file", cli_found: false, cli_path: null },
  { agent: "qoder", name: "Qoder", installed: true, state: "installed", evidence: "config_dir", cli_found: true, cli_path: "C:/Users/demo/AppData/Local/Programs/Qoder/qoder.exe" },
  { agent: "qoder-cn", name: "Qoder CN", installed: true, state: "installed", evidence: "config_dir", cli_found: true, cli_path: "C:/Users/demo/AppData/Local/Programs/Qoder CN/qoder.exe" },
  { agent: "pencil", name: "Pencil", installed: true, state: "installed", evidence: "config_file", cli_found: false, cli_path: null },
  { agent: "vscode", name: "VS Code", installed: true, state: "installed", evidence: "bin", cli_found: true, cli_path: "C:/Users/demo/AppData/Local/Programs/Microsoft VS Code/bin/code.cmd" },
  { agent: "continue", name: "Continue", installed: true, state: "installed", evidence: "config_dir", cli_found: false, cli_path: null },
  { agent: "copilot", name: "Copilot CLI", installed: true, state: "installed", evidence: "bin", cli_found: true, cli_path: "C:/Users/demo/AppData/Roaming/npm/copilot.cmd" },
  { agent: "cline", name: "Cline", installed: false, state: "not_found", evidence: null, cli_found: false, cli_path: null },
]

export function demoAgentDiscovery(): AgentDiscoveryDto[] {
  return DISCOVERY
}

/** 已接入的工具（WiringStatus）：画板 V8-01 的「已接入」卡 */
const WIRED_AGENTS = [
  {
    agent: "claude-code", wired: true, config_path: "~/.claude/settings.json",
    notice: "重启 Claude Code 后生效", reason: null, state: "wired" as const, drift: null,
    codex_mode: null, current_model: "jolu/claude-momo/claude-sonnet-4-5",
    current_resolved: "claude-sonnet-4-5",
  },
  {
    agent: "codex", wired: true, config_path: "~/.codex/config.toml",
    notice: "重启 Codex 后生效", reason: null, state: "wired" as const, drift: null,
    codex_mode: "oauth" as const, current_model: null, current_resolved: null,
  },
  {
    agent: "opencode", wired: true, config_path: "~/.config/opencode/opencode.json",
    notice: null, reason: null, state: "wired" as const, drift: null,
    codex_mode: null, current_model: null, current_resolved: null,
  },
  {
    agent: "gemini", wired: true, config_path: "~/.gemini/settings.json",
    notice: null, reason: null, state: "wired" as const, drift: "model_changed" as const,
    codex_mode: null, current_model: "gemini-2.5-pro", current_resolved: null,
  },
  {
    agent: "kimi", wired: false, config_path: "~/.kimi/config.toml",
    notice: null, reason: null, state: "idle" as const, drift: null,
    codex_mode: null, current_model: null, current_resolved: null,
  },
  {
    agent: "dsh", wired: true, config_path: "~/.dsh/profiles/default/cordis.patch.yml",
    notice: "重启 DeepSeek Harness 后生效", reason: null, state: "wired" as const, drift: null,
    codex_mode: null, current_model: null, current_resolved: null,
  },
  {
    agent: "qwen", wired: false, config_path: "~/.qwen/settings.json",
    notice: null, reason: null, state: "idle" as const, drift: null,
    codex_mode: null, current_model: null, current_resolved: null,
  },
  {
    agent: "workbuddy", wired: false, config_path: "~/.workbuddy/models.json",
    notice: null, reason: null, state: "idle" as const, drift: null,
    codex_mode: null, current_model: null, current_resolved: null,
  },
  {
    agent: "qoder", wired: true, config_path: "~/.qoder/config.json",
    notice: null, reason: null, state: "wired" as const, drift: null,
    codex_mode: null, current_model: null, current_resolved: null,
  },
  {
    agent: "qoder-cn", wired: false, config_path: "~/.qoder-cn/config.json",
    notice: null, reason: null, state: "idle" as const, drift: null,
    codex_mode: null, current_model: null, current_resolved: null,
  },
  {
    agent: "pencil", wired: false, config_path: "~/.pencil/config.json",
    notice: null, reason: null, state: "idle" as const, drift: null,
    codex_mode: null, current_model: null, current_resolved: null,
  },
]

export function demoGatewayStatus(): GatewayStatusDto {
  return {
    mode: "relay", enabled: true, port: 12731, circuit_threshold: 3, auto_escape: true,
    default_connection: "conn-claude-momo",
    wiring: WIRED_AGENTS,
  }
}

export function demoAgentVersions(): AgentVersionRow[] {
  const row = (agent: string, name: string, version: string | null, command: string | null, note = ""): AgentVersionRow => ({
    agent, name, found: version !== null, version, raw: version, update_command: command, update_note: note,
  })
  return [
    row("claude-code", "Claude Code", "2.0.31", "claude update"),
    row("codex", "Codex", "0.169.0", "npm install -g @openai/codex"),
    row("gemini", "Gemini CLI", "0.21.4", "npm install -g @google/gemini-cli"),
    row("opencode", "OpenCode", "1.4.2", "opencode upgrade"),
    row("zcode", "ZCode", "1.8.3", null, "随客户端自动更新"),
    row("kimi", "Kimi CLI", null, "npm install -g @moonshot/kimi-cli"),
    row("dsh", "DeepSeek Harness", "0.9.1", "bun add -g deepseek-harness"),
    row("qwen", "Qwen Code", "0.8.6", "npm install -g @qwen-code/qwen-code"),
    row("workbuddy", "WorkBuddy", null, null, "未检测到命令行"),
    row("qoder", "Qoder", "0.6.12", null, "随客户端自动更新"),
    row("qoder-cn", "Qoder CN", "0.6.12", null, "随客户端自动更新"),
    row("pencil", "Pencil", null, null, "未检测到命令行"),
    row("vscode", "VS Code", "1.106.2", null, "随客户端自动更新"),
    row("continue", "Continue", null, null, "未检测到命令行"),
    row("copilot", "Copilot CLI", "0.0.352", "npm install -g @github/copilot"),
  ]
}

/* ─────────────────────────── 模型目录与路由 ─────────────────────────── */

function model(platform: string, slug: string, source: string, id: string, kind: "auth" | "api", context: number): CatalogModelDto {
  return {
    id: `${platform}/${id}`, source, slug, model: id, kind, status: "ready",
    context_window: context, max_output: Math.round(context / 8),
  }
}

export function demoCatalog(): CatalogModelDto[] {
  return [
    model("claude", "claude-momo", "Momo", "claude-sonnet-4-5", "auth", 200_000),
    model("claude", "claude-momo", "Momo", "claude-opus-4-1", "auth", 200_000),
    model("claude", "claude-momo", "Momo", "claude-haiku-4-5", "auth", 200_000),
    model("claude", "claude-team", "Team", "claude-sonnet-4-5", "auth", 200_000),
    model("codex", "codex-alex", "Alex", "gpt-5", "auth", 272_000),
    model("codex", "codex-alex", "Alex", "gpt-5-mini", "auth", 272_000),
    model("codex", "codex-alex", "Alex", "gpt-5-codex", "auth", 272_000),
    model("gemini", "gemini-work", "Gemini 工作 Key", "gemini-2.5-flash", "api", 1_000_000),
    model("gemini", "gemini-work", "Gemini 工作 Key", "gemini-2.5-pro", "api", 1_000_000),
    model("custom", "deepseek", "DeepSeek", "deepseek-v4.1", "api", 128_000),
    model("zcode", "zcode-zai", "ZCode", "glm-5.3", "auth", 200_000),
    model("zcode", "zcode-zai", "ZCode", "glm-5.3-air", "auth", 128_000),
    model("qoder", "qoder-cn", "Qoder CN", "qoder-sonnet", "auth", 200_000),
  ]
}

export function demoRoutingGroups(): RoutingGroupDto[] {
  return [
    {
      id: "rg-daily", slug: "daily", name: "日常编码", strategy: "smart", created_ms: NOW - 12 * DAY,
      members: [
        { slug: "claude-momo", model: "claude-sonnet-4-5", weight: null, tier: null },
        { slug: "codex-alex", model: "gpt-5", weight: null, tier: null },
        { slug: "gemini-work", model: "gemini-2.5-flash", weight: 2, tier: "light" },
      ],
      intent: null,
    },
    {
      id: "rg-light", slug: "light", name: "轻量问答", strategy: "intent", created_ms: NOW - 5 * DAY,
      members: [
        { slug: "claude-momo", model: "claude-haiku-4-5", weight: null, tier: "light" },
        { slug: "deepseek", model: "deepseek-v4.1", weight: null, tier: "light" },
        { slug: "codex-alex", model: "gpt-5", weight: null, tier: null },
      ],
      intent: { classifier: "zcode-zai/glm-5.3-air", timeout_ms: 4000 },
    },
  ]
}

export function demoGatewayGroups(): GatewayGroupsDto {
  return {
    sources: [
      { id: "g-src-sub", name: "订阅账号", sort: 0, members: ["claude-momo", "claude-team", "codex-alex"] },
      { id: "g-src-key", name: "API Key", sort: 1, members: ["gemini-work", "deepseek"] },
    ],
    agents: [
      { id: "g-agent-cli", name: "命令行工具", sort: 0, members: ["claude-code", "codex", "gemini", "opencode", "dsh"] },
      { id: "g-agent-ide", name: "编辑器", sort: 1, members: ["qoder", "pencil"] },
    ],
  }
}

export function demoSourceCapabilities(): SourceCapabilityDto[] {
  return CONNECTIONS.map((c) => ({
    connection_id: c.id,
    can_serve: c.status === "connected" && c.kind !== null,
    state: c.status === "connected" ? "ready" : c.status === "expired" ? "needs_auth" : "unsupported",
    reason: c.status === "expired" ? "登录已失效，需要重新连接" : "",
  }))
}

export function demoSourceUsage(): SourceUsageDto[] {
  return [
    { connection_id: "conn-claude-momo", last_ts: NOW - 3 * MIN, requests: 486, failed: 2 },
    { connection_id: "conn-claude-team", last_ts: NOW - 42 * MIN, requests: 212, failed: 1 },
    { connection_id: "conn-codex-alex", last_ts: NOW - 22 * MIN, requests: 268, failed: 0 },
    { connection_id: "conn-gemini-key", last_ts: NOW - 2 * HOUR, requests: 86, failed: 3 },
    { connection_id: "conn-custom-deepseek", last_ts: NOW - 5 * HOUR, requests: 41, failed: 1 },
  ]
}

export function demoDayOverview(range = "today"): DayOverviewDto {
  const factor = range === "7d" ? 7 : range === "30d" ? 28 : 1
  return {
    today: 846 * factor, yesterday: 712 * factor, reroutes: 14 * factor, errors: 3 * factor,
    by_agent: { "claude-code": 412 * factor, codex: 268 * factor, gemini: 96 * factor, opencode: 42 * factor, dsh: 28 * factor },
    partial: false, identity_incomplete: false,
    last_error: {
      id: 9_812, ts: NOW - 26 * MIN, agent: "gemini", requested_model: "gemini-2.5-pro",
      source: "gemini-work", upstream_model: "gemini-2.5-pro", status_code: 504, result: "failed",
      duration_ms: 30_000, connection_id: "conn-gemini-key", session_platform: "gemini",
      session_id: "snap-gemini-1", request_id: 4_128, resolved_from: null, profile: null,
    },
  }
}

function routeRow(index: number): RouteLogDto {
  const agents = ["claude-code", "codex", "gemini", "opencode", "dsh", "qoder"]
  const agent = agents[index % agents.length]!
  const ok = index % 11 !== 3
  return {
    id: 10_000 - index,
    ts: NOW - index * 4 * MIN,
    agent,
    requested_model: index % 3 === 0 ? "claude-sonnet-4-5" : index % 3 === 1 ? "gpt-5" : "gemini-2.5-flash",
    source: index % 3 === 0 ? "claude-momo" : index % 3 === 1 ? "codex-alex" : "gemini-work",
    upstream_model: index % 3 === 0 ? "claude-sonnet-4-5" : index % 3 === 1 ? "gpt-5" : "gemini-2.5-flash",
    status_code: ok ? 200 : 429,
    result: ok ? "ok" : "failed",
    duration_ms: ok ? 900 + (index % 7) * 420 : 180,
    connection_id: index % 3 === 0 ? "conn-claude-momo" : index % 3 === 1 ? "conn-codex-alex" : "conn-gemini-key",
    session_platform: index % 2 === 0 ? "claude" : "codex",
    session_id: index % 2 === 0 ? "snap-claude-1" : "snap-codex-1",
    request_id: 4_128 - index,
    resolved_from: null,
    profile: index % 9 === 0 ? "写作" : null,
  }
}

export function demoRouteLog(): RouteLogDto[] {
  return Array.from({ length: 60 }, (_, i) => routeRow(i))
}

export function demoRouteLogPage(page = 1, pageSize = 50): RouteLogPageDto {
  const all = demoRouteLog()
  return {
    rows: all.slice((page - 1) * pageSize, page * pageSize),
    page, page_size: pageSize, total: 412, page_count: Math.ceil(412 / pageSize), upper_id: all[0]!.id!,
  }
}

export function demoTodayCounts() {
  return { since: new Date(new Date(NOW).getFullYear(), new Date(NOW).getMonth(), new Date(NOW).getDate()).getTime(), at: NOW - 5_000, ok: 828, rejected: 12, failed: 6 }
}

export function demoSessionsOverview(): SessionsOverviewDto {
  return {
    seq: 412,
    at: NOW - 4_000,
    in_flight: [
      { request_id: 4_128, ts: NOW - 22_000, agent: "claude-code", requested_model: "claude-sonnet-4-5", source: "claude-momo", connection_id: "conn-claude-momo", session_platform: "claude", session_id: "snap-claude-1" },
      { request_id: 4_127, ts: NOW - 48_000, agent: "codex", requested_model: "gpt-5", source: "codex-alex", connection_id: "conn-codex-alex", session_platform: "codex", session_id: "snap-codex-1" },
    ],
    sessions: [
      {
        platform: "claude", session_id: "snap-claude-1", agent: "claude-code", title: "排查缓存 Token 统计",
        cwd: "D:/develop/jolu", thread_last_ts: NOW - 3 * MIN, gateway_last_ts: NOW - 3 * MIN, thread_status: "running",
        tokens: { fresh_input: 9_000, cache_read: 21_000, cache_write: 1_400, output: 4_100, total: 35_500, state: "known" },
        links: [{ session_platform: "claude", session_id: "snap-claude-1", connection_id: "conn-claude-momo", source: "claude-momo", requested_model: "claude-sonnet-4-5", upstream_model: "claude-sonnet-4-5", agent: "claude-code", last_ts: NOW - 3 * MIN, requests: 12 }],
      },
      {
        platform: "codex", session_id: "snap-codex-1", agent: "codex", title: "优化 Jolu 的会话界面",
        cwd: "D:/develop/jolu", thread_last_ts: NOW - 22 * MIN, gateway_last_ts: NOW - 22 * MIN, thread_status: "running",
        tokens: { fresh_input: 12_400, cache_read: 8_600, cache_write: 0, output: 5_200, total: 26_200, state: "known" },
        links: [{ session_platform: "codex", session_id: "snap-codex-1", connection_id: "conn-codex-alex", source: "codex-alex", requested_model: "gpt-5", upstream_model: "gpt-5", agent: "codex", last_ts: NOW - 22 * MIN, requests: 9 }],
      },
      {
        platform: "gemini", session_id: "snap-gemini-1", agent: "gemini", title: "梳理登录引导文案",
        cwd: "D:/develop/jolu-docs", thread_last_ts: NOW - 11 * HOUR, gateway_last_ts: NOW - 11 * HOUR, thread_status: "recent",
        tokens: { fresh_input: 2_400, cache_read: null, cache_write: null, output: 900, total: null, state: "partial" },
        links: [{ session_platform: "gemini", session_id: "snap-gemini-1", connection_id: "conn-gemini-key", source: "gemini-work", requested_model: "gemini-2.5-flash", upstream_model: "gemini-2.5-flash", agent: "gemini", last_ts: NOW - 11 * HOUR, requests: 4 }],
      },
      {
        platform: "claude", session_id: "snap-claude-2", agent: "claude-code", title: "整理 Skills 管理方案",
        cwd: "D:/develop/jolu", thread_last_ts: NOW - 3 * HOUR, gateway_last_ts: NOW - 3 * HOUR, thread_status: "done",
        tokens: { fresh_input: 3_200, cache_read: 7_400, cache_write: 600, output: 1_900, total: 13_100, state: "known" },
        links: [{ session_platform: "claude", session_id: "snap-claude-2", connection_id: "conn-claude-momo", source: "claude-momo", requested_model: "claude-sonnet-4-5", upstream_model: "claude-sonnet-4-5", agent: "claude-code", last_ts: NOW - 3 * HOUR, requests: 6 }],
      },
    ],
  }
}

/* ─────────────────────────── 上下文观察（V10） ─────────────────────────── */

function contextRow(index: number): ContextSessionRowDto {
  const seeds = [
    { platform: "claude", id: "snap-claude-1", agent: "claude-code", title: "排查缓存 Token 统计", input: 168_400, window: 200_000, cache: 121_000 },
    { platform: "codex", id: "snap-codex-1", agent: "codex", title: "优化 Jolu 的会话界面", input: 92_000, window: 272_000, cache: 61_000 },
    { platform: "gemini", id: "snap-gemini-1", agent: "gemini", title: "梳理登录引导文案", input: 41_200, window: null, cache: 0 },
    { platform: "claude", id: "snap-claude-2", agent: "claude-code", title: "整理 Skills 管理方案", input: 63_800, window: 200_000, cache: 38_400 },
  ]
  const seed = seeds[index % seeds.length]!
  return {
    id: 512 - index,
    session_platform: seed.platform,
    session_id: seed.id,
    request_id: 4_128 - index,
    ts: NOW - index * 4 * MIN,
    state: "ready",
    protocol: seed.platform === "gemini" ? "gemini" : seed.platform === "codex" ? "responses" : "anthropic",
    input_tokens: seed.input,
    output_tokens: 1_200 + index * 240,
    cache_read: seed.cache,
    window_tokens: seed.window,
    estimated: seed.input,
    counted: true,
    error: null,
    agent: seed.agent,
    requested_model: seed.platform === "claude" ? "claude-sonnet-4-5" : seed.platform === "codex" ? "gpt-5" : "gemini-2.5-flash",
    upstream_model: seed.platform === "claude" ? "claude-sonnet-4-5" : seed.platform === "codex" ? "gpt-5" : "gemini-2.5-flash",
    source: seed.platform === "claude" ? "claude-momo" : seed.platform === "codex" ? "codex-alex" : "gemini-work",
    route_result: "ok",
    status_code: 200,
    duration_ms: 1_400 + index * 300,
    connection_id: seed.platform === "claude" ? "conn-claude-momo" : seed.platform === "codex" ? "conn-codex-alex" : "conn-gemini-key",
    connection_name: seed.platform === "claude" ? "Momo" : seed.platform === "codex" ? "Alex" : "Gemini 工作 Key",
    connection_platform: seed.platform,
    last_ready_ts: NOW - index * 4 * MIN,
    snapshots: 20 - index,
  }
}

export function demoContextSessions(): ContextSessionRowDto[] {
  return [0, 1, 2, 3].map(contextRow)
}

export function demoContextSnapshot(platform: string, sessionId: string, snapshotId: number | null): ContextSnapshotDetailDto | null {
  const base = demoContextSessions().find((row) => row.session_platform === platform && row.session_id === sessionId)
    ?? demoContextSessions()[0]!
  const total = base.input_tokens ?? 168_400
  const parts = [
    { kind: "system" as const, tokens: Math.round(total * 0.06), items: [
      { name: "系统提示", tag: "system", tokens: Math.round(total * 0.05) },
      { name: "项目约定（CLAUDE.md）", tag: "memory", tokens: Math.round(total * 0.01) },
    ] },
    { kind: "tools" as const, tokens: Math.round(total * 0.18), items: [
      { name: "Read", tag: "tool", tokens: Math.round(total * 0.04) },
      { name: "Edit", tag: "tool", tokens: Math.round(total * 0.03) },
      { name: "Bash", tag: "tool", tokens: Math.round(total * 0.05) },
      { name: "Grep / Glob", tag: "tool", tokens: Math.round(total * 0.06) },
    ] },
    { kind: "skills" as const, tokens: Math.round(total * 0.07), items: [
      { name: "frontend-design", tag: "SKILL.md", tokens: Math.round(total * 0.04) },
      { name: "test-runner", tag: "SKILL.md", tokens: Math.round(total * 0.03) },
    ] },
    { kind: "memory" as const, tokens: Math.round(total * 0.03), items: [{ name: "AGENTS.md", tag: "memory", tokens: Math.round(total * 0.03) }] },
    { kind: "files" as const, tokens: Math.round(total * 0.22), items: [
      { name: "frontend/src/views/UsageView.tsx", tag: "file", tokens: Math.round(total * 0.12) },
      { name: "frontend/src/lib/usageStats.ts", tag: "file", tokens: Math.round(total * 0.1) },
    ] },
    { kind: "results" as const, tokens: Math.round(total * 0.16), items: [
      { name: "Bash · node --test", tag: "tool_result", n: 3, tokens: Math.round(total * 0.09) },
      { name: "Read · UsageView.tsx", tag: "tool_result", n: 2, tokens: Math.round(total * 0.07) },
    ] },
    { kind: "chat" as const, tokens: Math.round(total * 0.24), items: [{ name: "对话消息", tag: "chat", n: 14, tokens: Math.round(total * 0.24) }] },
    { kind: "unknown" as const, tokens: Math.round(total * 0.04), items: [{ name: "未能归类", tag: "unknown", tokens: Math.round(total * 0.04) }] },
  ]
  return {
    ...base,
    id: snapshotId ?? base.id,
    parts,
    tools: [
      { tool: "Read", count: 8, tokens: Math.round(total * 0.05) },
      { tool: "Bash", count: 5, tokens: Math.round(total * 0.09) },
      { tool: "Edit", count: 3, tokens: Math.round(total * 0.03) },
    ],
  }
}

export function demoContextItems(kind: string): ContextItemRowDto[] {
  const detail = demoContextSnapshot("claude", "snap-claude-1", 512)!
  const part = detail.parts?.find((p) => p.kind === kind) ?? detail.parts?.[0]
  const total = detail.input_tokens ?? 1
  return (part?.items ?? []).map((item) => ({
    kind: (part?.kind ?? "unknown") as ContextItemRowDto["kind"],
    name: item.name, tag: item.tag, n: item.n ?? null, tokens: item.tokens,
    share: item.tokens / total,
  }))
}

export function demoContextToolItems(): ContextToolResultDto[] {
  return [
    { tool: "Bash", target: "node --test frontend/src/lib/*.test.mjs", kind: "results", tokens: 8_400, lines: 42, exit_code: 0, masked: false, summary: "通过 128 项测试", captured: true },
    { tool: "Bash", target: "pnpm --dir frontend typecheck", kind: "results", tokens: 2_100, lines: 6, exit_code: 0, masked: false, summary: "类型检查通过", captured: true },
    { tool: "Read", target: "frontend/src/views/UsageView.tsx", kind: "results", tokens: 6_200, lines: 220, exit_code: null, masked: false, summary: "读取 220 行", captured: true },
  ]
}

export function demoContextHistory(): ContextSnapshotSummaryDto[] {
  return [0, 1, 2].map((index) => {
    const row = contextRow(index)
    return { ...row, id: row.id }
  })
}

/* ─────────────────────────── 对外访问与中间件 ─────────────────────────── */

export function demoClientKeys(): ClientKeySummaryDto[] {
  return [
    {
      name: "笔记本 · 只读", masked: "jolu-…3f81", tokens_per_day: 2_000_000, requests_per_day: 500,
      created_ms: NOW - 6 * DAY, expires_ms: null, models: null, read_only: true, last_used_ms: NOW - 18 * MIN,
      today_requests: 42, max_concurrency: 2,
      budget: { tokens_per_day: 2_000_000, cost_per_day: 5 }, accounts: null,
      spend: [[820_000, 1.24], [4_100_000, 6.2], [12_800_000, 19.4]],
    },
    {
      name: "办公台式", masked: "jolu-…9ac2", tokens_per_day: null, requests_per_day: null,
      created_ms: NOW - 21 * DAY, expires_ms: NOW + 40 * DAY, models: ["claude-sonnet-4-5", "gpt-5"], read_only: false,
      last_used_ms: NOW - 3 * HOUR, today_requests: 168, max_concurrency: null,
      accounts: ["conn-claude-momo", "conn-codex-alex"], spend: [[1_200_000, 2.1], [6_400_000, 11.8], [22_100_000, 41.3]],
    },
    {
      name: "临时演示（已用满）", masked: "jolu-…7d14", tokens_per_day: null, requests_per_day: 100,
      created_ms: NOW - 2 * DAY, expires_ms: NOW - DAY, models: null, read_only: false, last_used_ms: NOW - 32 * HOUR,
      today_requests: 0, max_concurrency: null, accounts: null, spend: [[0, 0], [120_000, 0.4], [120_000, 0.4]],
    },
  ]
}

export function demoMiddlewares(): MiddlewareDto[] {
  return [
    { name: "隐私脱敏", kind: "word-guard", config: { mask: ["AKIA", "sk-"] }, enabled: true, sort: 0 },
    { name: "长上下文自动改写", kind: "model-map", config: { mapping: { "claude-long": "claude-momo/claude-sonnet-4-5" } }, enabled: true, sort: 1 },
    { name: "限制输出上限", kind: "param-override", config: { set: { max_tokens: 8192 } }, enabled: false, sort: 2 },
  ]
}

export function demoProfiles(): ProfileDto[] {
  return [
    { name: "写作", agents: { "claude-code": "claude-momo/claude-sonnet-4-5", codex: "codex-alex/gpt-5" }, created_ms: NOW - 9 * DAY, extras: { default_connection: "conn-claude-momo", favorites: ["claude-momo/claude-sonnet-4-5"] } },
    { name: "前端开发", agents: { "claude-code": "claude-momo/claude-opus-4-1", codex: "codex-alex/gpt-5-codex", gemini: "gemini-work/gemini-2.5-pro" }, created_ms: NOW - 3 * DAY, },
  ]
}

export function demoWiringPreview(mode: "wire" | "wired"): WiringPreviewDto {
  return {
    mode,
    files: [
      {
        path: "~/.claude/settings.json", existed: true, keys: mode === "wire" ? 3 : 1,
        lines: mode === "wire"
          ? [
              { op: " ", text: "{" },
              { op: " ", text: "  \"env\": {" },
              { op: "+", text: "    \"ANTHROPIC_BASE_URL\": \"http://127.0.0.1:12731\",", },
              { op: "+", text: "    \"ANTHROPIC_AUTH_TOKEN\": \"jolu-claude-code-…\"," },
              { op: " ", text: "    \"ANTHROPIC_MODEL\": \"claude-sonnet-4-5\"" },
              { op: " ", text: "  }," },
              { op: "+", text: "  \"model\": \"jolu/claude-momo/claude-sonnet-4-5\"," },
              { op: " ", text: "  \"includeCoAuthoredBy\": false" },
              { op: " ", text: "}" },
            ]
          : [
              { op: " ", text: "{" },
              { op: "…", text: "（未改动 6 行）" },
              { op: "-", text: "  \"model\": \"jolu/claude-momo/claude-sonnet-4-5\"," },
              { op: " ", text: "  \"includeCoAuthoredBy\": false" },
              { op: " ", text: "}" },
            ],
      },
    ],
  }
}

/* ─────────────────────────── 设置杂项 ─────────────────────────── */

export function demoDataInfo() {
  return { path: "C:/Users/demo/AppData/Roaming/jolu", size_bytes: 428_600_000 }
}

export function demoStorageCounts() {
  return { requests: 128_460, sessions: 3_812 }
}

export function demoNetworkDiagnosis(): NetworkDiagnosisDto {
  return {
    proxy: null,
    endpoints: [
      { connection_name: "Momo", platform: "claude", probe: { ok: true, latency_ms: 42, error: null, skipped: false } },
      { connection_name: "Alex", platform: "codex", probe: { ok: true, latency_ms: 68, error: null, skipped: false } },
      { connection_name: "Gemini 工作 Key", platform: "gemini", probe: { ok: true, latency_ms: 121, error: null, skipped: false } },
      { connection_name: "DeepSeek", platform: "custom", probe: { ok: false, latency_ms: null, error: "连接超时", skipped: false } },
    ],
  }
}

export function demoSearchVendors(): SearchVendorDto[] {
  return [
    { id: "tavily", name: "Tavily", key_optional: false, needs_url: false },
    { id: "exa", name: "Exa", key_optional: false, needs_url: false },
    { id: "searxng", name: "SearXNG（自建）", key_optional: true, needs_url: true },
  ]
}

export function demoLibraryList(): LibraryAgentDto[] {
  return [
    { agent: "claude-code", installed: ["jolu-search"], config_present: true, supported: true, note: null },
    { agent: "codex", installed: [], config_present: true, supported: true, note: null },
    { agent: "gemini", installed: [], config_present: false, supported: true, note: "尚未生成配置文件" },
    { agent: "qoder", installed: [], config_present: false, supported: false, note: "格式待确认" },
  ]
}

export function demoLibrarySharedState(): LibrarySharedStateDto {
  return {
    instructions: { text: "始终用中文回答。改动前先读相关文件；不要新增依赖，除非说明理由。", agents: ["claude-code", "codex"] },
    instruction_targets: [
      { agent: "claude-code", path: "~/.claude/CLAUDE.md", written: true, changed: false },
      { agent: "codex", path: "~/.codex/AGENTS.md", written: true, changed: false },
      { agent: "gemini", path: null, written: false, changed: false },
    ],
    subagents: [
      {
        name: "code-reviewer", description: "审查改动并给出可执行的修改建议", prompt: "你是一个严格但务实的代码审查者……",
        model: "claude-sonnet-4-5", agents: ["claude-code"],
        targets: [{ agent: "claude-code", path: "~/.claude/agents/code-reviewer.md", written: true, changed: false }],
      },
    ],
    subagent_agents: ["claude-code", "codex", "gemini", "opencode", "qwen", "droid"],
  }
}

export function demoWslProbe(): WslProbeDto {
  return {
    available: true, reason: null, distros: ["Ubuntu-24.04", "Debian"], default_distro: "Ubuntu-24.04",
    agents: [
      { agent: "claude-code", found: true, path: "/usr/local/bin/claude", config_path: "~/.claude/settings.json", config_exists: true },
      { agent: "codex", found: true, path: "/usr/local/bin/codex", config_path: "~/.codex/config.toml", config_exists: true },
      { agent: "opencode", found: false, path: null, config_path: "~/.config/opencode/opencode.json", config_exists: false },
    ],
  }
}

export function demoWslWiringState(): WslWiringStateDto {
  return {
    net: { distro: "Ubuntu-24.04", home: "/home/demo", mode: "mirrored", host_ip: null },
    net_error: null,
    rows: [
      { distro: "Ubuntu-24.04", agent: "claude-code", path: "~/.claude/settings.json", base: "http://127.0.0.1:12731", client_key: null, intact: true, wired_ms: NOW - 4 * DAY },
    ],
    agents: ["claude-code", "codex", "opencode"],
    lan_shared: false,
  }
}

export function demoTuneAdvice(): TuneAgentDto[] {
  return [
    {
      platform: "claude", calls: 1_284, compact_setting: null,
      compactions: [
        {
          model: "claude-sonnet-4-5", current: 272_000, window: 1_000_000, best: 220_000, cost: 4.82, low: 3.94,
          curve: [
            { value: 120, cost: 5.4, compacts: 18 },
            { value: 180, cost: 4.42, compacts: 11 },
            { value: 220, cost: 3.94, compacts: 8 },
            { value: 272, cost: 4.82, compacts: 6 },
          ],
          facts: { calls: 1_284, streams: 96, compacts: 6, quick: 12, pause: 4, away: 3 },
        },
      ],
      cache: { current: "5 分钟", current_inferred: true, best: "1 小时", cost: 2.16, low: 1.42, facts: { calls: 1_284, streams: 96, compacts: 6, quick: 12, pause: 4, away: 3 } },
    },
    {
      platform: "codex", calls: 812, compact_setting: null,
      compactions: [
        {
          model: "gpt-5", current: 200_000, window: 0, best: 200_000, cost: 2.28, low: 2.28,
          curve: [{ value: 200, cost: 2.28, compacts: 4 }],
          facts: { calls: 812, streams: 64, compacts: 4, quick: 8, pause: 1, away: 2 },
        },
      ],
      cache: null,
    },
  ]
}

export function demoPrivacyRecords(): MaskRecordDto[] {
  return [
    { id: 3, at_ms: NOW - 26 * MIN, entry: "claude-code · claude-sonnet-4-5", blocked: false, masked: 2, hits: [
      { kind: "phone", action: "warn", length: 11, preview: "138****", placeholder: null, left: "联系电话 ", right: " 请回拨" },
      { kind: "email", action: "mask", length: 17, preview: "m***@example.com", placeholder: "[EMAIL_1]", left: "账号 ", right: " 已创建" },
    ] },
    { id: 2, at_ms: NOW - 3 * HOUR, entry: "codex · gpt-5", blocked: true, masked: 1, hits: [
      { kind: "private_key", action: "block", length: 1_872, preview: "-----BEGIN…", placeholder: null, left: "文件内容", right: "" },
    ] },
    { id: 1, at_ms: NOW - DAY, entry: "gemini · gemini-2.5-flash", blocked: false, masked: 1, hits: [
      { kind: "id_card", action: "mask", length: 18, preview: "110***", placeholder: "[ID_1]", left: "证件号 ", right: " 校验" },
    ] },
  ]
}

export const DEMO_WARMUP_OUTCOME = { status: "ok" as const, platform: "claude", detail: "已用 Claude Code 的 haiku 档位发送最小请求", finished_ms: NOW - 9 * HOUR }

export const DEMO_OTLP_RESULT = { endpoint: "https://otlp.example.com/v1/metrics", points: 42, status: 200, response: "OK" }

export const DEMO_UPDATE_CHECK = {
  available: true, current_version: "0.1.1", available_version: "0.2.0",
  notes: "灵动岛新增会话耗时角标；修复代理转发偶发中断；连接管理支持批量导入。",
  pub_date: "2026-11-18",
}

export const DEMO_IMPORT_PREVIEW = {
  token: "snap-preview", file_name: "jolu-backup-20261120.json", valid: 1_200, duplicate: 180, invalid: 2,
  invalid_reasons: ["第 42 条缺 model 字段", "第 907 条时间戳无效"],
}

export const DEMO_IMPORT_SUMMARY = { requests_changed: 1_200, sessions_changed: 180, skipped: 182 }

export const DEMO_CLEANUP_PREVIEW = { cutoff_ms: NOW - 180 * DAY, requests: 12_480, usage_events: 26_900, sessions: 412 }
