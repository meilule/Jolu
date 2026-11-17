/**
 * 灵动岛的阈值 / 事件短时信号（4.2b #7，调研 `docs/specs/2026-11-05-island-live-linkage.md` §4；无画板，待确认）。
 * 测试见 `islandSignals.test.mjs`。
 *
 * 只在前端用已有数据合成，不新增后端事件：
 * - quota-80：任一额度窗口的已用比例从 80% 以下升到 80% 及以上；
 * - quota-exhausted：任一额度窗口从未满到 100%；
 * - rejected：网关回了 429 / 402（限流、额度或余额不足）——只有胶囊样式订阅了网关事件；
 * - new-session：会话列表里出现了这一轮之前没见过的会话。
 *
 * 第一次拿到数据只记基线、不报信号（打开岛时不该一下子全闪）；同一种信号 5 秒内只报一次；
 * 每个信号显示 1.5 秒。免打扰或减少动效时界面只改颜色、不播动画（由调用方决定）。
 */

export type SignalKind = "quota-80" | "quota-exhausted" | "rejected" | "new-session"

export interface SignalInput {
  /** 各额度窗口的已用百分比（未知的不传） */
  usedPercents: number[]
  /** 会话 id 列表 */
  sessionIds: string[]
  /** 最近一次网关拒绝的标识（状态码 + 时刻），没有为 null */
  rejection: string | null
}

export interface SignalState {
  /** 上一次的输入；null = 还没有基线 */
  last: SignalInput | null
  /** 每种信号最近一次报出的时刻 */
  firedAt: Partial<Record<SignalKind, number>>
}

export interface ActiveSignal {
  kind: SignalKind
  at: number
}

export const SIGNAL_SHOW_MS = 1_500
export const SIGNAL_MERGE_MS = 5_000
export const QUOTA_WARN = 80

export const SIGNAL_LABEL: Record<SignalKind, string> = {
  "quota-80": "额度已用 80%",
  "quota-exhausted": "额度已用尽",
  rejected: "网关被拒绝（限流或额度不足）",
  "new-session": "新会话开始",
}

export const emptySignals = (): SignalState => ({ last: null, firedAt: {} })

const maxOf = (values: number[]) => (values.length === 0 ? null : Math.max(...values))

/** 比较前后两次输入，返回新的状态与这次要报的信号（按严重程度：用尽 > 拒绝 > 80% > 新会话，只报最严重的一个） */
export function stepSignals(state: SignalState, input: SignalInput, now: number): { state: SignalState; signal: ActiveSignal | null } {
  const last = state.last
  if (!last) return { state: { ...state, last: input }, signal: null }
  const candidates: SignalKind[] = []
  const before = maxOf(last.usedPercents)
  const after = maxOf(input.usedPercents)
  if (after != null && after >= 100 && (before == null || before < 100)) candidates.push("quota-exhausted")
  if (input.rejection && input.rejection !== last.rejection) candidates.push("rejected")
  if (after != null && after >= QUOTA_WARN && after < 100 && (before == null || before < QUOTA_WARN)) candidates.push("quota-80")
  const seen = new Set(last.sessionIds)
  if (input.sessionIds.some((id) => !seen.has(id))) candidates.push("new-session")
  const firedAt = { ...state.firedAt }
  let signal: ActiveSignal | null = null
  for (const kind of candidates) {
    const previous = firedAt[kind]
    if (previous != null && now - previous < SIGNAL_MERGE_MS) continue
    firedAt[kind] = now
    if (!signal) signal = { kind, at: now }
  }
  return { state: { last: input, firedAt }, signal }
}

/** 信号是否还在显示期内 */
export function signalVisible(signal: ActiveSignal | null, now: number): boolean {
  return signal != null && now - signal.at < SIGNAL_SHOW_MS
}
