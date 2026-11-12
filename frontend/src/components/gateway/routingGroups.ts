/**
 * 路由分组的纯逻辑（测试见 `routingGroups.test.mjs`）。
 *
 * 路由分组把多个「来源/模型」成员组成一个模型 id `group/<分组id>`，网关按策略在成员间
 * 分摊与失败转移。界面目前是「管理模型来源」弹窗底部的待确认最小版（RoutingGroupsSection）；
 * 按 Jolu.pen 画板确认后移入主界面。
 */

/** 模型目录条目（结构与 `CatalogModelDto` 一致；只依赖字段形状，测试不必引入 Tauri 桥接）。 */
export interface CatalogEntry {
  id: string
  source: string
  slug: string
  model: string
  kind: string
  status: string
}

/** 路由分组（与后端一致；`gatewayApi.groupList` 的返回项）。 */
export interface RoutingGroupDto {
  id: string
  slug: string
  name: string
  strategy: string
  members: RoutingMember[]
  created_ms: number
  /** 「按意图」的判定配置；其他策略缺省 */
  intent?: IntentConfig | null
}

/** 「按意图」判定配置（与后端 `IntentConfig` 一致）。 */
export interface IntentConfig {
  /** 判意图用的模型 id（`来源/模型`，不能是分组） */
  classifier: string
  /** 判定超时毫秒；缺省按后端默认 4000 */
  timeout_ms?: number | null
}

/** 路由分组成员：某个来源（slug）下的一个模型。 */
export interface RoutingMember {
  slug: string
  model: string
  /** 临时暂停到这个时刻（毫秒）；缺省或 null = 未暂停。保存分组时必须原样带回，否则后端会清掉。 */
  paused_until_ms?: number | null
  /** 「轮转」权重（每轮出现几次）；缺省按 1 */
  weight?: number | null
  /** 「按意图」档位：light = 轻档（快问题用的便宜模型）；缺省 = 强档 */
  tier?: string | null
}

/** 成员可固定的推理档位（写在模型名后：`模型:high`）；只认这些词，`qwen:7b` 里的冒号不算 */
export const MEMBER_EFFORTS = ["low", "medium", "high", "xhigh", "max"] as const
const EFFORT_WORDS = ["none", "minimal", ...MEMBER_EFFORTS]

/** 成员模型拆成（基础模型，固定档位） */
export function memberParts(model: string): { base: string; effort: string | null } {
  const at = model.lastIndexOf(":")
  if (at > 0 && EFFORT_WORDS.includes(model.slice(at + 1).toLowerCase())) return { base: model.slice(0, at), effort: model.slice(at + 1).toLowerCase() }
  return { base: model, effort: null }
}

/** 设置某个成员的固定档位（null = 跟随请求） */
export function setMemberEffort(members: RoutingMember[], member: RoutingMember, effort: string | null): RoutingMember[] {
  return members.map((item) => (sameMember(item, member) ? { ...item, model: effort ? `${memberParts(item.model).base}:${effort}` : memberParts(item.model).base } : item))
}

/** 设置某个成员的轮转权重（null 或 1 = 默认） */
export function setMemberWeight(members: RoutingMember[], member: RoutingMember, weight: number | null): RoutingMember[] {
  return members.map((item) => (sameMember(item, member) ? { ...item, weight: weight && weight > 1 ? Math.min(Math.floor(weight), 100) : null } : item))
}

/**
 * 策略文案（下拉与列表行共用）。smart / pace 按额度快照里的重置时间排序；
 * 成员都没有额度数据时退回顺序；额度用满的成员排到最后（不剔除）。
 */
export const ROUTING_STRATEGIES: readonly { value: string; text: string }[] = [
  { value: "order", text: "顺序（失败换下一个）" },
  { value: "rotate", text: "轮转（逐请求换）" },
  { value: "usage", text: "用量最少优先" },
  { value: "smart", text: "智能（重置最近的先用）" },
  { value: "pace", text: "配速（余量最足的先用）" },
  { value: "intent", text: "按意图（小模型分强 / 轻档）" },
]

/** 判定超时范围（毫秒），与后端 `intent::MIN_TIMEOUT_MS` / `MAX_TIMEOUT_MS` 一致 */
export const INTENT_TIMEOUT_RANGE = { min: 500, max: 15000, fallback: 4000 } as const

/** 成员是否轻档（缺省或其他值都算强档，与后端 `GroupMember::is_light` 一致） */
export function isLightMember(member: RoutingMember): boolean {
  return member.tier === "light"
}

/** 设置某个成员的档位（light = 轻档，否则清掉回到强档） */
export function setMemberTier(members: RoutingMember[], member: RoutingMember, light: boolean): RoutingMember[] {
  return members.map((item) => (sameMember(item, member) ? { ...item, tier: light ? "light" : null } : item))
}

/**
 * 「按意图」保存前检查（与后端 `validate_intent` 同规则）：返回第一条问题，没问题为 null。
 * `timeout` 是输入框原文，空 = 用默认值。
 */
export function intentError(members: RoutingMember[], classifier: string, timeout: string): string | null {
  const id = classifier.trim()
  if (!id) return "选一个判意图用的模型。"
  if (id.startsWith("group/")) return "判意图用的模型不能是路由分组。"
  const slash = id.indexOf("/")
  if (slash <= 0 || slash === id.length - 1) return "判意图用的模型要写成「来源/模型」。"
  if (timeout.trim()) {
    const ms = Number(timeout.trim())
    if (!Number.isInteger(ms) || ms < INTENT_TIMEOUT_RANGE.min || ms > INTENT_TIMEOUT_RANGE.max) {
      return `判定超时要在 ${INTENT_TIMEOUT_RANGE.min}–${INTENT_TIMEOUT_RANGE.max} 毫秒之间。`
    }
  }
  if (!members.some(isLightMember) || members.every(isLightMember)) return "强档和轻档都至少要有一个成员。"
  return null
}

/** 表单值 → 保存用的判定配置（先过 `intentError`） */
export function intentConfig(classifier: string, timeout: string): IntentConfig {
  return { classifier: classifier.trim(), timeout_ms: timeout.trim() ? Number(timeout.trim()) : null }
}

/**
 * 策略对成员顺序的含义。按额度排的策略（usage / smart / pace）里，顺序只在额度相同时决定先后；
 * 还没有额度数据的成员（比如刚加的账号）排在最后，要等其他成员用到上限附近才会轮到——
 * 这是规则本身，不是故障（magpie `4436b7d` 修过同样的误解：把新账号排第一却一直用不到）。
 */
export function strategyOrderNote(strategy: string): string {
  switch (strategy) {
    case "order": return "按列表顺序用，前一个失败或用满才换下一个；前面的恢复后，会话会回到它。"
    case "rotate": return "每个请求换下一个成员，列表顺序只决定起点。"
    case "usage": return "用量最少的先用；列表顺序只在用量相同时决定先后。"
    case "smart": return "额度最快重置的先用；列表顺序只在重置时间相同时决定先后。还没有额度数据的成员（如刚加的账号）排在最后。"
    case "pace": return "本周期余量最足的先用；列表顺序只在余量相同时决定先后。还没有额度数据的成员排在最后。"
    case "intent": return "每个新提问先让判定模型分强档或轻档，按列表顺序用该档成员，另一档留作兜底；工具调用的中间步骤沿用上次判定，判不出时走强档。每个新提问会多一次判定调用，用量记在路由流水里。"
    default: return ""
  }
}

/** 策略显示文案；未知值原样返回，不悄悄替换成默认策略。 */
export function strategyText(strategy: string): string {
  return ROUTING_STRATEGIES.find((item) => item.value === strategy)?.text ?? strategy
}

/** 成员的模型 id：`来源slug/模型`。 */
export function routingMemberId(member: RoutingMember): string {
  return `${member.slug}/${member.model}`
}

/** 可勾选的成员条目：模型目录里的普通条目；分组自己的条目（slug === "group"）与缺 slug / model 的条目跳过，不嵌套。 */
export function selectableCatalog(models: CatalogEntry[]): CatalogEntry[] {
  return models.filter((item) => item.slug !== "group" && !!item.slug && !!item.model)
}

/** 成员是否已勾选（按 slug + model 匹配）。 */
export function hasRoutingMember(members: RoutingMember[], member: RoutingMember): boolean {
  return members.some((item) => sameMember(item, member))
}

/** 暂停时长选项（分钟）：到期自动恢复，与后端 `GroupMember::is_paused` 一致。 */
export const PAUSE_CHOICES: readonly { minutes: number; text: string }[] = [
  { minutes: 30, text: "30 分钟" },
  { minutes: 60, text: "1 小时" },
  { minutes: 240, text: "4 小时" },
  { minutes: 1440, text: "24 小时" },
]

/** 同一成员：来源相同、基础模型相同（固定档位不影响身份——目录里只有基础模型） */
function sameMember(a: RoutingMember, b: RoutingMember): boolean {
  return a.slug === b.slug && memberParts(a.model).base === memberParts(b.model).base
}

/** 设置 / 清除某个成员的暂停到期时间（null = 恢复）；其他成员原样保留。 */
export function pauseMember(members: RoutingMember[], member: RoutingMember, untilMs: number | null): RoutingMember[] {
  return members.map((item) => (sameMember(item, member) ? { ...item, paused_until_ms: untilMs } : item))
}

/** 成员此刻是否在暂停中（到期视为已恢复）。 */
export function isMemberPaused(member: RoutingMember, now: number): boolean {
  return typeof member.paused_until_ms === "number" && member.paused_until_ms > now
}

/** 暂停中的成员给「暂停至 HH:MM」，否则为空串。 */
export function pausedText(member: RoutingMember, now: number): string {
  if (!isMemberPaused(member, now)) return ""
  const until = new Date(member.paused_until_ms as number)
  return `暂停至 ${until.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}`
}

/** 勾选 / 取消勾选一个成员：已存在则移除，否则追加到末尾。 */
export function toggleRoutingMember(members: RoutingMember[], member: RoutingMember): RoutingMember[] {
  return hasRoutingMember(members, member)
    ? members.filter((item) => !sameMember(item, member))
    : [...members, member]
}

/** 成员里不在可选目录中的条目（目录刷新后可能已下架；编辑时保留显示，可取消勾选移除）。 */
export function offCatalogMembers(members: RoutingMember[], models: CatalogEntry[]): RoutingMember[] {
  const selectable = selectableCatalog(models)
  return members.filter((member) => !selectable.some((item) => sameMember({ slug: item.slug, model: item.model }, member)))
}

/** 分组 id 校验：小写字母数字连字符、不以连字符开头；返回错误文案，合法或未填时为 null（未填由按钮禁用负责）。 */
export function routingSlugError(slug: string): string | null {
  if (!slug) return null
  if (!/^[a-z0-9-]+$/.test(slug)) return "只能使用小写字母、数字和连字符。"
  if (slug.startsWith("-")) return "不能以连字符开头。"
  return null
}
