import { invoke } from "./api"

/** 网关接线状态（与后端 `WiringStatus` 一致）。 */
/** 档位自选集合（批 4，与后端 `wiring_store::TierModels` 一致）；null 档 = 跟随主模型 */
export interface TierModels {
  opus?: string | null
  sonnet?: string | null
  haiku?: string | null
  fable?: string | null
  autoMode?: string | null
}

export interface WiringStatusDto {
  agent: string
  wired: boolean
  config_path: string | null
  notice: string | null
  reason: string | null
  /** 接线记录阶段：idle / pending / wired；旧版后端不返回 */
  state?: "idle" | "pending" | "wired" | null
  /** 漂移：pending / removed / changed / foreign */
  drift?: "pending" | "removed" | "changed" | "foreign" | null
  /** Codex 检测到的登录方式 */
  codex_mode?: "oauth" | "provider" | null
}

export interface GatewayGroupDto {
  id: string
  name: string
  sort: number
  members: string[]
}

export interface GatewayGroupsDto {
  sources: GatewayGroupDto[]
  agents: GatewayGroupDto[]
}

export type GroupKind = "source" | "agent"

export interface CatalogModelDto {
  id: string
  source: string
  slug: string
  model: string
  kind: "auth" | "api" | string
  status: string
  /** models.dev 元数据（P4.2）：缺失表示目录里没有这个模型的资料 */
  context_window?: number
  max_output?: number
  label?: string
}

/** 命名客户端 Key 的列表条目（与后端 `ClientKeySummary` 一致，不含原文）。 */
export interface ClientKeySummaryDto {
  name: string
  masked: string
  tokens_per_day: number | null
  requests_per_day: number | null
  created_ms: number
  /** 过期时间（毫秒）；null = 永不过期 */
  expires_ms: number | null
  /** 模型白名单；null = 不限 */
  models: string[] | null
  /** 只读：只放行查询类请求 */
  read_only: boolean
  last_used_ms: number | null
  today_requests: number
  /** 同时请求数上限；null = 不限（超出的请求排队，不拒绝） */
  max_concurrency?: number | null
}

/** 路由分组成员（与后端 `GroupMember` 一致）：`paused_until_ms` 缺省 = 未暂停，保存时须原样带回。 */
export interface GroupMemberDto {
  slug: string
  model: string
  paused_until_ms?: number | null
}

/** 网关中间件（与后端 `gateway::middleware::Middleware` 一致）。 */
export interface MiddlewareDto {
  name: string
  kind: string
  config: Record<string, unknown>
  enabled: boolean
  sort: number
}

/** 签发结果：`key` 是原文，仅此一次可见。 */
export interface IssuedClientKeyDto {
  name: string
  key: string
  tokens_per_day: number | null
  requests_per_day: number | null
  expires_ms: number | null
  models: string[] | null
  read_only: boolean
}

export interface RouteLogDto {
  /** 自增流水 id：分页的稳定行标识（旧版后端不返回） */
  id?: number
  ts: number
  agent: string | null
  requested_model: string
  source: string | null
  upstream_model: string | null
  status_code: number | null
  result: "ok" | "rejected" | "failed" | string
  duration_ms: number | null
  connection_id?: string | null
  session_platform?: string | null
  session_id?: string | null
  /** 同一次网关请求的编号：分组成员转移时各次尝试共享它（进程内递增，重启后从头计；旧行为 null） */
  request_id?: number | null
}

/** 路由流水的一页（与后端 `RouteLogPage` 一致）。 */
export interface RouteLogPageDto {
  rows: RouteLogDto[]
  page: number
  page_size: number
  /** 查询上界内、当前保留范围内的总行数 */
  total: number
  /** 没有数据时为 0 */
  page_count: number
  /** 本次浏览的查询上界；翻页时带回，刷新时传 null 重新建立 */
  upper_id: number | null
}

/** 在途请求（与后端 `LiveRequest` 一致）。 */
export interface LiveRequestDto {
  request_id: number
  ts: number
  agent: string | null
  requested_model: string
  source: string
  connection_id: string | null
  session_platform: string | null
  session_id: string | null
}

/** 会话经网关到某个连接的历史关联（保留范围内）。 */
export interface SessionLinkDto {
  session_platform: string
  session_id: string
  connection_id: string | null
  source: string | null
  requested_model: string
  upstream_model: string | null
  agent: string | null
  last_ts: number
  requests: number
}

export interface SessionTokensDto {
  fresh_input: number | null
  cache_read: number | null
  cache_write: number | null
  output: number | null
  /** 只有四个分项都可信时才有值 */
  total: number | null
  /** known：分项完整；partial：有分项缺失；missing：会话线程里还没有这条会话 */
  state: "known" | "partial" | "missing"
}

export interface GatewaySessionDto {
  platform: string
  session_id: string
  agent: string | null
  title: string | null
  cwd: string | null
  thread_last_ts: number | null
  gateway_last_ts: number | null
  /** 会话线程状态；还没采集到时为 null */
  thread_status: "running" | "done" | "failed" | "recent" | "usage_only" | string | null
  tokens: SessionTokensDto
  links: SessionLinkDto[]
}

export interface SessionsOverviewDto {
  /** 在途快照对应的事件序号：只应用序号更大的实时事件 */
  seq: number
  at: number
  in_flight: LiveRequestDto[]
  sessions: GatewaySessionDto[]
}

/** 网关总览（与后端 `GatewayStatus` 一致；不含任何凭证）。 */
export interface GatewayStatusDto {
  mode: "observe" | "relay" | string
  enabled: boolean
  port: number
  circuit_threshold: number
  auto_escape: boolean
  default_connection: string | null
  wiring: WiringStatusDto[]
}

export type GatewayMode = "observe" | "relay"

export interface SourceCapabilityDto {
  connection_id: string
  can_serve: boolean
  state: "ready" | "needs_auth" | "unsupported"
  reason: string
}

/** 批量加入 / 移出模型池的单项结果 */
export interface ServeBatchItemDto {
  connection_id: string
  ok: boolean
  error: string | null
}

/** 来源经网关的使用情况（路由流水保留范围内） */
export interface SourceUsageDto {
  connection_id: string
  last_ts: number
  requests: number
  failed: number
}

export const gatewayApi = {
  sourceCapabilities: () => invoke<SourceCapabilityDto[]>("gateway_source_capabilities"),
  accessKey: (refresh: boolean) => invoke<string>("gateway_access_key", { refresh }),
  copyKey: () => invoke<void>("gateway_copy_key"),
  status: () => invoke<GatewayStatusDto>("gateway_status"),
  /** 切换模式；切回 observe 时后端会先还原全部接线 */
  setMode: (mode: GatewayMode, defaultConnection: string | null) =>
    invoke<unknown>("gateway_set_mode", { mode, defaultConnection }),
  /**
   * 设置 / 清除默认来源。无前缀模型名只能靠它兜底，否则会被网关拒绝（未带来源前缀）。
   * 只接受已在模型池中的连接。
   */
  setDefaultSource: (connectionId: string | null) => invoke<string | null>("gateway_set_default_source", { connectionId }),
  /** 实时拉取某来源的模型清单并落库；订阅类来源缓存为空时界面靠它脱离「0 模型」。 */
  refreshModels: (connectionId: string) => invoke<string[]>("gateway_refresh_models", { connectionId }),
  /* ── 局域网共享与命名客户端 Key（P8.3 后端已就绪） ── */

  /** 局域网共享开关；绑定地址在网关下次启动监听时生效。 */
  setSharedLan: (on: boolean) => invoke<{ gateway_shared_lan?: boolean }>("gateway_shared_lan_set", { on }),
  /** 客户端 Key 列表：只有掩码与限额，原文不出后端。 */
  clientKeys: () => invoke<ClientKeySummaryDto[]>("gateway_client_keys_list"),
  /** 签发客户端 Key：**原文只返回这一次**（关闭后无法再查看）。 */
  issueClientKey: (
    name: string,
    requestsPerDay?: number | null,
    tokensPerDay?: number | null,
    options?: { expiresMs?: number | null; models?: string[] | null; readOnly?: boolean },
  ) =>
    invoke<IssuedClientKeyDto>("gateway_client_key_issue", {
      name,
      requestsPerDay: requestsPerDay ?? null,
      tokensPerDay: tokensPerDay ?? null,
      expiresMs: options?.expiresMs ?? null,
      models: options?.models ?? null,
      readOnly: options?.readOnly ?? false,
    }),
  /** 撤销客户端 Key：删除立即失效，历史流水归属保留。 */
  revokeClientKey: (name: string) => invoke<void>("gateway_client_key_revoke", { name }),
  /** 命名 Key 的同时请求数上限：null = 不限，1–1000 */
  setClientKeyConcurrency: (name: string, cap: number | null) => invoke<void>("gateway_client_key_concurrency_set", { name, cap }),

  /** 网关中间件：按 sort 顺序在选路前改写请求体；保存前后端会试跑配置，跑不通拒绝保存。 */
  middlewares: () => invoke<MiddlewareDto[]>("gateway_middleware_list"),
  saveMiddleware: (row: MiddlewareDto) =>
    invoke<MiddlewareDto>("gateway_middleware_save", { name: row.name, kind: row.kind, config: row.config, enabled: row.enabled, sort: row.sort }),
  deleteMiddleware: (name: string) => invoke<void>("gateway_middleware_delete", { name }),

  /** Profile（配置一键切换）：保存当前已接线 agent 的主模型快照，应用时逐个重放接线。 */
  profilesList: () => invoke<{ name: string; agents: Record<string, string>; created_ms: number }[]>("gateway_profiles_list"),
  profilesSave: (name: string) => invoke<{ name: string; agents: Record<string, string>; created_ms: number }>("gateway_profiles_save", { name }),
  profilesDelete: (name: string) => invoke<boolean>("gateway_profiles_delete", { name }),
  profilesApply: (name: string) =>
    invoke<{ switched: string[]; skipped: string[]; message: string }>("gateway_profiles_apply", { name }),
  /** 路由分组（`group/<slug>`）：成员按策略分摊与转移。界面为待确认的最小可用版。 */
  groupList: () => invoke<{ id: string; slug: string; name: string; strategy: string; members: GroupMemberDto[]; created_ms: number }[]>("gateway_routing_group_list"),
  groupCreate: (slug: string, name: string, strategy: string, members: GroupMemberDto[]) =>
    invoke<unknown>("gateway_routing_group_create", { slug, name, strategy, members }),
  groupUpdate: (slug: string, patch: { name?: string; strategy?: string; members?: GroupMemberDto[] }) =>
    invoke<unknown>("gateway_routing_group_update", { slug, ...patch }),
  groupDelete: (slug: string) => invoke<unknown>("gateway_routing_group_delete", { slug }),
  /** 连接进出模型池 */
  setServe: (connectionId: string, enabled: boolean) =>
    invoke<void>("gateway_set_serve", { connectionId, enabled }),
  /** 批量进出模型池：逐项返回结果，单个失败不影响其他来源 */
  setServeMany: (connectionIds: string[], enabled: boolean) =>
    invoke<ServeBatchItemDto[]>("gateway_set_serve_many", { connectionIds, enabled }),
  /** 各来源最近使用时间与请求数 */
  sourceUsage: () => invoke<SourceUsageDto[]>("gateway_source_usage"),
  /** 接入 / 断开单个工具 */
  /** 接入 / 断开工具。`model` 只对 Claude Code 有意义（`<来源>/<模型>`），不传则不改模型映射。 */
  setWiring: (agent: string, wired: boolean, model?: string | null, subagentModel?: string | null, tiers?: TierModels) =>
    invoke<string>("gateway_wiring_set", { agent, wired, model: model ?? null, subagentModel: subagentModel ?? null, tiers: tiers ?? null }),
  /** 档位自选（批 4）：每档的最终写入值（`model[:effort]`，可带 [1m]）；某档 null = 跟随主模型。
   *  对应后端 `wiring_store::TierModels`；Auto 判别写 `CLAUDE_CODE_AUTO_MODE_MODEL`，「跟随子代理」由调用方填子代理的值。 */
  setWiringTiers: (agent: string, tiers: TierModels | null) =>
    invoke<string>("gateway_wiring_set", { agent, wired: true, tiers }),
  /** 接线记录里实际写入的主模型 / 子代理 / 档位（只对已接线的工具有值），给接线弹窗回填 */
  wiringApplied: (agent: string) =>
    invoke<{ model: string | null; subagent_model: string | null; tiers: TierModels | null } | null>("gateway_wiring_applied", { agent }),
  /** 接受漂移：只清接线记录，不改用户的配置文件 */
  acceptWiring: (agent: string) => invoke<void>("gateway_wiring_accept", { agent }),
  /** 逃生门：还原全部接线并停网关 */
  escape: () => invoke<string>("gateway_escape"),
  /** 来源分组 / Agent 分组（只影响排序） */
  groups: () => invoke<GatewayGroupsDto>("gateway_groups"),
  createGroup: (kind: GroupKind, name: string) => invoke<GatewayGroupDto>("gateway_group_create", { kind, name }),
  renameGroup: (kind: GroupKind, id: string, name: string) => invoke<void>("gateway_group_rename", { kind, id, name }),
  deleteGroup: (kind: GroupKind, id: string) => invoke<void>("gateway_group_delete", { kind, id }),
  moveToGroup: (kind: GroupKind, member: string, groupId: string | null) =>
    invoke<void>("gateway_group_move", { kind, member, groupId }),
  reorderGroups: (kind: GroupKind, ids: string[]) => invoke<void>("gateway_group_reorder", { kind, ids }),
  /** 改菜单 id；旧 id 之后请求会得到 404 说明 */
  setSlug: (connectionId: string, slug: string) => invoke<string>("gateway_set_slug", { connectionId, slug }),
  /** 模型清单（只读缓存，不打上游） */
  catalog: () => invoke<CatalogModelDto[]>("gateway_catalog"),
  /** 最近 20 条路由流水（灵动岛用） */
  routeLog: () => invoke<RouteLogDto[]>("gateway_route_log"),
  /** 路由流水分页；upperId 为 null 时以最新一条建立查询范围 */
  routeLogPage: (page: number, pageSize: number, upperId: number | null) =>
    invoke<RouteLogPageDto>("gateway_route_log_page", { page, pageSize, upperId }),
  /** 模型来源 → 最近会话：在途快照 + 经网关会话 + 权威 Token */
  sessionsOverview: () => invoke<SessionsOverviewDto>("gateway_sessions_overview"),
  /** 今日（本地零点起）经网关的请求数 */
  todayCounts: () => invoke<{ since: number; at: number; ok: number; rejected: number; failed: number }>("gateway_today_counts"),
}
