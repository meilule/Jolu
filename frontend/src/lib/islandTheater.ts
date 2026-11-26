/**
 * bot 道具与小剧场纯逻辑（jolu-new.pen R2-I-05）。
 *
 * 所有道具仅由真实状态与真实事件推出，不随机出现；每日一次性事件按本地日期在 localStorage 去重。
 */
import type { BotExpr, BotProp } from "./botExpression.ts"

export const MILESTONE_1M = 1_000_000
export const MILESTONE_10M = 10_000_000
export const CACHE_SAVINGS_MIN_USD = 0.5
export const HIGH_CONTEXT_RATIO = 0.8
export const LONG_TASK_MS = 5 * 60_000

export type TheaterAction =
  | { kind: "none" }
  | { kind: "checkin" }
  | { kind: "usage" }
  | { kind: "gateway" }
  | { kind: "session"; platform: string; sessionId: string }

export interface TheaterCue {
  id: string
  prop: BotProp
  /** 建议联动显示的表情（遵循 R2-I-05：无嘴巴、纯眼睛） */
  expr: BotExpr
  title: string
  /** 是否属于静默陪伴（如深夜咖啡：不弹提醒、不打断） */
  quiet?: boolean
  /** 显示时长（ms）；null 表示跟条件持续（如深夜写作期间端着咖啡） */
  durationMs: number | null
  action: TheaterAction
}

export function localDayKey(date: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`
}

/** 00:00–05:00 属于深夜加班时段（R2-I-05 咖啡道具） */
export function isLateNightHour(hour: number): boolean {
  return hour >= 0 && hour < 5
}

/**
 * 计算今日 Token 跨过的最高未播报里程碑档位（1M / 10M）。
 * 每个档位每天只触发一次。
 */
export function nextTokenMilestone(
  todayTokens: number | null | undefined,
  seen: ReadonlySet<string>,
): { key: "1m" | "10m"; label: string } | null {
  if (typeof todayTokens !== "number" || !Number.isFinite(todayTokens)) return null
  if (todayTokens >= MILESTONE_10M && !seen.has("10m")) {
    return { key: "10m", label: "今天第 10,000,000 个 Token" }
  }
  if (todayTokens >= MILESTONE_1M && !seen.has("1m")) {
    return { key: "1m", label: "今天第 1,000,000 个 Token" }
  }
  return null
}

/**
 * 估算缓存读取省下的美元费用（按同类主流模型 90% 缓存读取折扣的保守下界，
 * 当模型已返回真实单价差时直接按真实单价差计算）。
 * 无法估算或没有缓存读取时返回 null（不伪造金额）。
 */
export function estimateCacheSavingsUsd(
  cacheReadTokens: number | null | undefined,
  freshInputPricePerM?: number | null,
  cachedInputPricePerM?: number | null,
): number | null {
  if (typeof cacheReadTokens !== "number" || !Number.isFinite(cacheReadTokens) || cacheReadTokens <= 0) {
    return null
  }
  if (
    typeof freshInputPricePerM === "number" &&
    typeof cachedInputPricePerM === "number" &&
    freshInputPricePerM > cachedInputPricePerM &&
    cachedInputPricePerM >= 0
  ) {
    const saved = (cacheReadTokens / 1_000_000) * (freshInputPricePerM - cachedInputPricePerM)
    return Number.isFinite(saved) && saved > 0 ? saved : null
  }
  return null
}

/** 构造 8 种 R2-I-05 道具剧场提示的工厂，统一文案与时长规范。 */
export function makeGiftCue(readyCount: number): TheaterCue {
  return {
    id: "gift",
    prop: "gift",
    expr: "happy",
    title: `${readyCount} 个账号可签到`,
    durationMs: 6500,
    action: { kind: "checkin" },
  }
}

export function makeChargeCue(label: string): TheaterCue {
  return {
    id: `charge:${label}`,
    prop: "charge",
    expr: "reset",
    title: `${label} 额度已重置`,
    durationMs: 4200,
    action: { kind: "none" },
  }
}

export function makeConfettiCue(tier: "1m" | "10m", label: string): TheaterCue {
  return {
    id: `confetti:${tier}`,
    prop: "confetti",
    expr: "happy",
    title: label,
    durationMs: 4500,
    action: { kind: "usage" },
  }
}

export function makePiggyCue(savedUsd: number): TheaterCue {
  return {
    id: `piggy:${Math.floor(savedUsd * 2)}`,
    prop: "piggy",
    expr: "happy",
    title: `缓存帮你省了 ≈ $${savedUsd.toFixed(2)}`,
    durationMs: 4800,
    action: { kind: "usage" },
  }
}

export function makeUtensilsCue(ratio: number, platform: string, sessionId: string): TheaterCue {
  return {
    id: `utensils:${sessionId}`,
    prop: "utensils",
    expr: "surprise",
    title: `上下文 ${Math.round(ratio * 100)}% · 建议压缩`,
    durationMs: 6500,
    action: { kind: "session", platform, sessionId },
  }
}

export function makeFlagCue(tool: string, durationMs: number, platform: string, sessionId: string | null): TheaterCue {
  const minutes = Math.max(5, Math.round(durationMs / 60_000))
  return {
    id: `flag:${sessionId ?? tool}:${minutes}`,
    prop: "flag",
    expr: "greet",
    title: `${tool} 完成了 · 用时 ${minutes} 分钟`,
    durationMs: 6000,
    action: sessionId ? { kind: "session", platform, sessionId } : { kind: "none" },
  }
}

export function makeCoffeeCue(tool: string, now: Date = new Date()): TheaterCue {
  const pad = (n: number) => String(n).padStart(2, "0")
  return {
    id: "coffee",
    prop: "coffee",
    expr: "sleep",
    title: `${pad(now.getHours())}:${pad(now.getMinutes())} · ${tool} 还在写`,
    quiet: true,
    durationMs: null,
    action: { kind: "none" },
  }
}

export function makeRelayCue(fromSource: string, toSource: string): TheaterCue {
  return {
    id: `relay:${fromSource}->${toSource}`,
    prop: "medal",
    expr: "happy",
    title: `${toSource} 接住了 ${fromSource} 的请求`,
    durationMs: 3600,
    action: { kind: "gateway" },
  }
}

const SEEN_STORAGE_PREFIX = "jolu:island:theater-seen:"

export function readSeenTheaterKeys(day: string, storage: Pick<Storage, "getItem"> = localStorage): Set<string> {
  try {
    const raw = storage.getItem(`${SEEN_STORAGE_PREFIX}${day}`)
    if (!raw) return new Set()
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) ? new Set(parsed.filter((v): v is string => typeof v === "string")) : new Set()
  } catch {
    return new Set()
  }
}

export function writeSeenTheaterKeys(day: string, keys: ReadonlySet<string>, storage: Pick<Storage, "setItem"> = localStorage): void {
  try {
    storage.setItem(`${SEEN_STORAGE_PREFIX}${day}`, JSON.stringify([...keys]))
  } catch {
    // 浏览器隐私模式等环境写不进不影响当前运行
  }
}
