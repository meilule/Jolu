import assert from "node:assert/strict"
import test from "node:test"
import {
  cycleDynamicFocus,
  emptyDynamicFollowState,
  FOCUS_END_HOLD_MS,
  stepDynamicFollow,
  TURN_QUOTA_REFRESH_COOLDOWN_MS,
} from "./islandDynamicFollow.ts"

const PLATFORM_PLATFORM = { connection_id: null, name: null, confidence: "platform" }

function brief(platform, overrides = {}) {
  return {
    platform,
    running_count: 1,
    recent_count: 1,
    running_tokens: 4200,
    today_tokens: 18000,
    last_seen_ms: 1000,
    attribution: PLATFORM_PLATFORM,
    sessions: [],
    ...overrides,
  }
}

test("1A: locks onto first active agent, exposes concurrent peers without jitter, and hands off after hold", () => {
  let state = emptyDynamicFollowState()

  // 1) Claude starts writing → immediately locks onto claude
  let out = stepDynamicFollow(state, {
    enabled: true,
    strictWriting: false,
    configuredPlatform: "codex",
    overview: [brief("claude", { running_tokens: 3100 })],
    activeAccounts: [],
    hookAlerts: [],
    currentConnectionId: "conn-claude-1",
    nowMs: 10_000,
  })
  state = out.state
  assert.equal(out.targetPlatform, "claude")
  assert.equal(out.concurrentPeers.length, 0)

  // 2) Codex starts writing concurrently → primary stays locked on claude, codex appears in concurrentPeers
  out = stepDynamicFollow(state, {
    enabled: true,
    strictWriting: false,
    configuredPlatform: "codex",
    overview: [
      brief("codex", { running_tokens: 12_400, last_seen_ms: 12_000 }),
      brief("claude", { running_tokens: 5_200, last_seen_ms: 11_500 }),
    ],
    activeAccounts: [],
    hookAlerts: [],
    currentConnectionId: "conn-claude-1",
    nowMs: 12_000,
  })
  state = out.state
  assert.equal(out.targetPlatform, "claude", "anti-jitter lock must prevent jumping to codex mid-turn")
  assert.equal(out.concurrentPeers.length, 1)
  assert.equal(out.concurrentPeers[0].platform, "codex")
  assert.equal(out.concurrentPeers[0].runningTokens, 12_400)

  // 3) User clicks concurrent bubble → cycles focus to codex
  state = cycleDynamicFocus(state, ["claude", "codex"], (p) => p.toUpperCase(), 12_500)
  out = stepDynamicFollow(state, {
    enabled: true,
    strictWriting: false,
    configuredPlatform: "claude",
    overview: [brief("codex"), brief("claude")],
    activeAccounts: [],
    hookAlerts: [],
    currentConnectionId: "conn-codex-1",
    nowMs: 12_600,
  })
  state = out.state
  assert.equal(out.targetPlatform, "codex")
  assert.equal(out.activeCue?.label, "已切换至 CODEX")

  // 4) Codex finishes turn while Claude is still running → holds for FOCUS_END_HOLD_MS + triggers throttled quota refresh
  out = stepDynamicFollow(state, {
    enabled: true,
    strictWriting: false,
    configuredPlatform: "claude",
    overview: [brief("claude")],
    activeAccounts: [],
    hookAlerts: [],
    currentConnectionId: "conn-codex-1",
    nowMs: 20_000,
  })
  state = out.state
  assert.equal(out.targetPlatform, "codex", "holds briefly before handing off")
  assert.equal(out.refreshQuotaConnectionId, "conn-codex-1")

  // Within cooldown, another end does not re-trigger quota refresh
  out = stepDynamicFollow(
    state,
    {
      enabled: true,
      strictWriting: false,
      configuredPlatform: "claude",
      overview: [brief("claude")],
      activeAccounts: [],
      hookAlerts: [],
      currentConnectionId: "conn-codex-1",
      nowMs: 20_000 + FOCUS_END_HOLD_MS + 10,
    },
    (p) => p.toUpperCase(),
  )
  state = out.state
  assert.equal(out.targetPlatform, "claude", "smoothly hands off to remaining active agent after hold")
  assert.equal(out.activeCue?.label, "已接力 · CLAUDE")
  assert.ok(TURN_QUOTA_REFRESH_COOLDOWN_MS >= 30_000)
})

test("3A: external account switch follows matched connection and safeguards unregistered local key", () => {
  let state = emptyDynamicFollowState()

  // Initial baseline
  let out = stepDynamicFollow(state, {
    enabled: true,
    strictWriting: false,
    configuredPlatform: "claude",
    overview: [],
    activeAccounts: [
      {
        platform: "claude",
        kind: "auth",
        state: "matched",
        connection_id: "c-momo",
        connection_name: "Momo · Max",
        masked: "sk-ant-****1111",
        account_email: "momo@example.com",
        base_url: null,
        updated_ms: 1000,
      },
    ],
    hookAlerts: [],
    currentConnectionId: "c-momo",
    nowMs: 1000,
  })
  state = out.state
  assert.equal(out.targetConnectionId, "c-momo")
  assert.equal(out.unregisteredLocal, null)

  // External CC Switch switches Claude to an unregistered API key!
  out = stepDynamicFollow(state, {
    enabled: true,
    strictWriting: false,
    configuredPlatform: "claude",
    overview: [],
    activeAccounts: [
      {
        platform: "claude",
        kind: "api",
        state: "unregistered",
        connection_id: null,
        connection_name: null,
        masked: "sk-ant-****9f2a",
        account_email: null,
        base_url: "https://relay.unknown.test",
        updated_ms: 2000,
      },
    ],
    hookAlerts: [],
    currentConnectionId: "c-momo",
    nowMs: 2000,
  })
  state = out.state
  assert.equal(out.targetConnectionId, null, "must clear old connection so old quota rings are never shown")
  assert.deepEqual(out.unregisteredLocal, {
    platform: "claude",
    kind: "api",
    masked: "sk-ant-****9f2a",
    baseUrl: "https://relay.unknown.test",
    sourceAgent: null,
  })
  assert.equal(out.activeCue?.kind, "unregistered")

  // User clicks one-click enrollment on Dynamic Island → backend saves connection and emits matched state
  out = stepDynamicFollow(state, {
    enabled: true,
    strictWriting: false,
    configuredPlatform: "claude",
    overview: [],
    activeAccounts: [
      {
        platform: "claude",
        kind: "api",
        state: "matched",
        connection_id: "claude-enrolled-1",
        connection_name: "Claude Code · relay.unknown.test",
        masked: "sk-ant-****9f2a",
        account_email: null,
        base_url: "https://relay.unknown.test",
        source_agent: "Claude Code",
        updated_ms: 2500,
      },
    ],
    hookAlerts: [],
    currentConnectionId: null,
    nowMs: 2500,
  })
  assert.equal(out.unregisteredLocal, null, "unregistered warning disappears immediately after enrollment")
  assert.equal(out.targetConnectionId, "claude-enrolled-1")
  assert.equal(out.activeCue?.kind, "account")
  assert.equal(out.activeCue?.label, "已跟随 · Claude Code · relay.unknown.test")
})

test("3A: multi-CLI unregistered key carries source_agent label for Qwen Code / OpenCode", () => {
  let state = emptyDynamicFollowState()
  let out = stepDynamicFollow(state, {
    enabled: true,
    strictWriting: false,
    configuredPlatform: "claude",
    overview: [],
    activeAccounts: [
      {
        platform: "claude",
        kind: "auth",
        state: "matched",
        connection_id: "c-1",
        connection_name: "Main",
        masked: "sk-ant-****0001",
        account_email: null,
        base_url: null,
        updated_ms: 1000,
      },
    ],
    hookAlerts: [],
    currentConnectionId: "c-1",
    nowMs: 1000,
  })
  state = out.state

  out = stepDynamicFollow(state, {
    enabled: true,
    strictWriting: false,
    configuredPlatform: "claude",
    overview: [],
    activeAccounts: [
      {
        platform: "claude",
        kind: "auth",
        state: "matched",
        connection_id: "c-1",
        connection_name: "Main",
        masked: "sk-ant-****0001",
        account_email: null,
        base_url: null,
        updated_ms: 1000,
      },
      {
        platform: "custom",
        kind: "api",
        state: "unregistered",
        connection_id: null,
        connection_name: null,
        masked: "sk-****5555",
        account_email: null,
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        source_agent: "Qwen Code",
        updated_ms: 2000,
      },
    ],
    hookAlerts: [],
    currentConnectionId: "c-1",
    nowMs: 2000,
  })
  assert.equal(out.targetPlatform, "custom")
  assert.equal(out.unregisteredLocal?.sourceAgent, "Qwen Code")
  assert.equal(out.activeCue?.label, "Qwen Code · 未收录 · sk-****5555")
})

test("4B: waiting_approval hook alert takes highest priority and guides idle island to waiting platform", () => {
  const state = emptyDynamicFollowState()
  const out = stepDynamicFollow(state, {
    enabled: true,
    strictWriting: false,
    configuredPlatform: "claude",
    overview: [],
    activeAccounts: [],
    hookAlerts: [
      {
        platform: "codex",
        kind: "turn_complete",
        session_id: "s-1",
        summary: "本轮已完成",
        updated_ms: 5000,
      },
      {
        platform: "claude",
        kind: "waiting_approval",
        session_id: "s-2",
        summary: "等待确认 Bash 权限",
        updated_ms: 4800,
      },
    ],
    currentConnectionId: null,
    nowMs: 5100,
  })
  assert.equal(out.approvalAlert?.kind, "waiting_approval")
  assert.equal(out.approvalAlert?.platform, "claude")
  assert.equal(out.targetPlatform, "claude")
})
