/**
 * 灵动岛动态跟随纯状态机（1A 智能跟随与防抖焦点锁 + 3A 外部切号与陌生 Key 防呆 + 4B 等待确认高优提醒）。
 *
 * 所有时间判定均通过显式 `nowMs` 注入，零定时器、零 DOM/Tauri 依赖，可由同名 `.test.mjs` 确定性测试。
 */

import type {
  AgentHookAlertDto,
  LocalActiveAccountDto,
  PlatformLiveBriefDto,
} from "./api"

/** 焦点 Agent 本轮结束后的防抖驻留时长：避免多工具交替停顿时主胶囊来回跳切 */
export const FOCUS_END_HOLD_MS = 2_400
/** 外部切号或自动接力时的胶囊轻提示时长 */
export const SWITCH_CUE_MS = 1_500
/** 会话回合结束触发额度补查的每连接冷却窗口 */
export const TURN_QUOTA_REFRESH_COOLDOWN_MS = 45_000

export interface ConcurrentAgentPeer {
  platform: string
  runningCount: number
  runningTokens: number | null
  todayTokens: number | null
  connectionName: string | null
}

export interface UnregisteredLocalKey {
  platform: string
  kind: "auth" | "api"
  masked: string
  baseUrl: string | null
  sourceAgent: string | null
}

export interface SwitchCue {
  kind: "agent" | "account" | "unregistered"
  label: string
  untilMs: number
}

export interface DynamicFollowState {
  focusedPlatform: string | null
  holdUntilMs: number | null
  externalPlatform: string | null
  accountFingerprints: Record<string, string>
  accountBaselineReady: boolean
  switchCue: SwitchCue | null
  lastQuotaRefreshAt: Record<string, number>
}

export function emptyDynamicFollowState(): DynamicFollowState {
  return {
    focusedPlatform: null,
    holdUntilMs: null,
    externalPlatform: null,
    accountFingerprints: {},
    accountBaselineReady: false,
    switchCue: null,
    lastQuotaRefreshAt: {},
  }
}

export interface DynamicFollowStepInput {
  enabled: boolean
  strictWriting: boolean
  configuredPlatform: string
  overview: readonly PlatformLiveBriefDto[]
  activeAccounts: readonly LocalActiveAccountDto[]
  hookAlerts: readonly AgentHookAlertDto[]
  /** 当前展示连接的 id（用于回合结束节流补查额度） */
  currentConnectionId: string | null
  nowMs: number
}

export interface DynamicFollowStepOutput {
  state: DynamicFollowState
  /** 智能跟随决议出的目标平台 */
  targetPlatform: string
  /** 智能跟随决议出的目标已存连接 id（未命中或陌生 Key 时为 null） */
  targetConnectionId: string | null
  /** 3A 陌生 Key 防呆：当前平台在本机生效了一把尚未保存到 Jolu 的外部 Key / 登录 */
  unregisteredLocal: UnregisteredLocalKey | null
  /** 1A 并发多 Agent 气泡：除当前主胶囊锁定的平台外，仍在后台并发写入的其它平台 */
  concurrentPeers: ConcurrentAgentPeer[]
  /** 4B 最高优等待确认 / 完成提醒 */
  approvalAlert: AgentHookAlertDto | null
  /** 1.5s 切换轻提示（过期后为 null） */
  activeCue: SwitchCue | null
  /** 本步刚结束一轮运行且冷却已过、需要触发一次额度刷新的连接 id */
  refreshQuotaConnectionId: string | null
}

function accountFingerprint(item: LocalActiveAccountDto): string {
  return `${item.kind}|${item.state}|${item.connection_id ?? ""}|${item.masked}|${item.base_url ?? ""}|${item.source_agent ?? ""}`
}

function isPlatformActivelyWriting(brief: PlatformLiveBriefDto, strictWriting: boolean): boolean {
  if (brief.running_count > 0) return true
  if (strictWriting) return false
  return brief.recent_count > 0
}

function attributedConnectionId(brief: PlatformLiveBriefDto | undefined): string | null {
  if (!brief) return null
  const { attribution } = brief
  if (!attribution || attribution.confidence === "platform") return null
  return attribution.connection_id?.trim() || null
}

export function pickApprovalAlert(alerts: readonly AgentHookAlertDto[]): AgentHookAlertDto | null {
  if (alerts.length === 0) return null
  const priority = (kind: AgentHookAlertDto["kind"]): number => {
    if (kind === "waiting_approval") return 3
    if (kind === "waiting_input") return 2
    if (kind === "turn_complete") return 1
    return 0
  }
  const ranked = [...alerts]
    .filter((a) => priority(a.kind) > 0)
    .sort((a, b) => priority(b.kind) - priority(a.kind) || b.updated_ms - a.updated_ms)
  return ranked[0] ?? null
}

/** 用户点击右侧并发气泡时，在当前所有活跃平台之间轮换焦点锁 */
export function cycleDynamicFocus(
  prev: DynamicFollowState,
  activePlatforms: readonly string[],
  platformLabel: (platform: string) => string,
  nowMs: number,
): DynamicFollowState {
  if (activePlatforms.length === 0) return prev
  const currentIdx = prev.focusedPlatform ? activePlatforms.indexOf(prev.focusedPlatform) : -1
  const nextPlatform = activePlatforms[(currentIdx + 1) % activePlatforms.length]!
  if (nextPlatform === prev.focusedPlatform) return prev
  return {
    ...prev,
    focusedPlatform: nextPlatform,
    holdUntilMs: null,
    switchCue: {
      kind: "agent",
      label: `已切换至 ${platformLabel(nextPlatform)}`,
      untilMs: nowMs + SWITCH_CUE_MS,
    },
  }
}

export function stepDynamicFollow(
  prev: DynamicFollowState,
  input: DynamicFollowStepInput,
  platformLabel: (platform: string) => string = (p) => p,
): DynamicFollowStepOutput {
  const { enabled, strictWriting, configuredPlatform, overview, activeAccounts, hookAlerts, currentConnectionId, nowMs } = input
  const approvalAlert = pickApprovalAlert(hookAlerts)

  // 即使未开启 follow_active，陌生 Key 防呆与 4B 提醒也按当前平台如实报告
  const accountByPlatform = new Map<string, LocalActiveAccountDto>()
  const nextFingerprints: Record<string, string> = {}
  for (const item of activeAccounts) {
    accountByPlatform.set(item.platform, item)
    nextFingerprints[item.platform] = accountFingerprint(item)
  }

  if (!enabled) {
    const currentLocal = accountByPlatform.get(configuredPlatform)
    const unregisteredLocal: UnregisteredLocalKey | null =
      currentLocal && currentLocal.state === "unregistered"
        ? {
            platform: currentLocal.platform,
            kind: currentLocal.kind,
            masked: currentLocal.masked,
            baseUrl: currentLocal.base_url,
            sourceAgent: currentLocal.source_agent ?? null,
          }
        : null
    return {
      state: {
        ...emptyDynamicFollowState(),
        accountFingerprints: nextFingerprints,
        accountBaselineReady: activeAccounts.length > 0 || prev.accountBaselineReady,
      },
      targetPlatform: configuredPlatform,
      targetConnectionId: null,
      unregisteredLocal,
      concurrentPeers: [],
      approvalAlert,
      activeCue: null,
      refreshQuotaConnectionId: null,
    }
  }

  let nextCue: SwitchCue | null = prev.switchCue && prev.switchCue.untilMs > nowMs ? prev.switchCue : null
  let externalPlatform = prev.externalPlatform
  let focusedPlatform = prev.focusedPlatform
  let holdUntilMs = prev.holdUntilMs
  let refreshQuotaConnectionId: string | null = null
  const nextLastRefresh = { ...prev.lastQuotaRefreshAt }

  // 3A 检测本机配置是否刚刚被外部工具（CC Switch / CLI 登录）改写
  let changedAccount: LocalActiveAccountDto | null = null
  if (prev.accountBaselineReady) {
    for (const item of activeAccounts) {
      const oldFp = prev.accountFingerprints[item.platform]
      const newFp = nextFingerprints[item.platform]
      if (oldFp !== newFp) {
        changedAccount = item
        break
      }
    }
  }

  const writingBriefs = overview.filter((b) => isPlatformActivelyWriting(b, strictWriting))
  const briefByPlatform = new Map(overview.map((b) => [b.platform, b]))

  if (changedAccount && (!focusedPlatform || focusedPlatform === changedAccount.platform || !writingBriefs.some((b) => b.platform === focusedPlatform))) {
    externalPlatform = changedAccount.platform
    if (!writingBriefs.some((b) => b.platform === focusedPlatform)) {
      focusedPlatform = null
      holdUntilMs = null
    }
    if (changedAccount.state === "matched" && changedAccount.connection_name) {
      nextCue = {
        kind: "account",
        label: `已跟随 · ${changedAccount.connection_name}`,
        untilMs: nowMs + SWITCH_CUE_MS,
      }
    } else if (changedAccount.state === "unregistered") {
      const prefix = changedAccount.source_agent ? `${changedAccount.source_agent} · 未收录` : "未保存凭证"
      nextCue = {
        kind: "unregistered",
        label: `${prefix} · ${changedAccount.masked}`,
        untilMs: nowMs + SWITCH_CUE_MS,
      }
    }
  }

  // 1A 焦点锁与回合结束防抖接力
  if (focusedPlatform) {
    const stillActive = writingBriefs.some((b) => b.platform === focusedPlatform)
    if (stillActive) {
      holdUntilMs = null
    } else {
      if (holdUntilMs === null) {
        holdUntilMs = nowMs + FOCUS_END_HOLD_MS
        if (currentConnectionId) {
          const lastAt = nextLastRefresh[currentConnectionId]
          if (lastAt === undefined || nowMs - lastAt >= TURN_QUOTA_REFRESH_COOLDOWN_MS) {
            nextLastRefresh[currentConnectionId] = nowMs
            refreshQuotaConnectionId = currentConnectionId
          }
        }
      }
      if (nowMs >= holdUntilMs) {
        holdUntilMs = null
        const handoff = writingBriefs[0] ?? null
        if (handoff) {
          focusedPlatform = handoff.platform
          nextCue = {
            kind: "agent",
            label: `已接力 · ${platformLabel(handoff.platform)}`,
            untilMs: nowMs + SWITCH_CUE_MS,
          }
        } else {
          focusedPlatform = null
        }
      }
    }
  } else if (writingBriefs.length > 0) {
    focusedPlatform = writingBriefs[0]!.platform
    holdUntilMs = null
  }

  // 4B 若有高优等待确认提醒且当前无其它正在运行锁定（或提醒就在其它后台终端），优先确保用户能看见：
  // 若无锁定运行平台，直接切向等待确认的平台；若有锁定运行平台，保留主胶囊并由气泡提示。
  const targetPlatform =
    focusedPlatform ??
    (approvalAlert && (approvalAlert.kind === "waiting_approval" || approvalAlert.kind === "waiting_input")
      ? approvalAlert.platform
      : null) ??
    externalPlatform ??
    configuredPlatform

  const targetBrief = briefByPlatform.get(targetPlatform)
  const targetLocal = accountByPlatform.get(targetPlatform)

  const unregisteredLocal: UnregisteredLocalKey | null =
    targetLocal && targetLocal.state === "unregistered"
      ? {
          platform: targetLocal.platform,
          kind: targetLocal.kind,
          masked: targetLocal.masked,
          baseUrl: targetLocal.base_url,
          sourceAgent: targetLocal.source_agent ?? null,
        }
      : null

  // 目标连接优先级：实时会话已归属连接（2B / Exact / Mapped）> 本机当前匹配连接（3A）
  const targetConnectionId = unregisteredLocal
    ? null
    : attributedConnectionId(targetBrief) ??
      (targetLocal?.state === "matched" ? targetLocal.connection_id : null)

  const concurrentPeers: ConcurrentAgentPeer[] = writingBriefs
    .filter((b) => b.platform !== targetPlatform)
    .map((b) => ({
      platform: b.platform,
      runningCount: b.running_count || b.recent_count,
      runningTokens: b.running_tokens,
      todayTokens: b.today_tokens,
      connectionName: b.attribution.confidence !== "platform" ? b.attribution.name : null,
    }))

  const nextState: DynamicFollowState = {
    focusedPlatform,
    holdUntilMs,
    externalPlatform,
    accountFingerprints: nextFingerprints,
    accountBaselineReady: activeAccounts.length > 0 || prev.accountBaselineReady,
    switchCue: nextCue,
    lastQuotaRefreshAt: nextLastRefresh,
  }

  return {
    state: nextState,
    targetPlatform,
    targetConnectionId,
    unregisteredLocal,
    concurrentPeers,
    approvalAlert,
    activeCue: nextCue,
    refreshQuotaConnectionId,
  }
}
