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
  members: { slug: string; model: string }[]
  created_ms: number
}

/** 路由分组成员：某个来源（slug）下的一个模型。 */
export interface RoutingMember {
  slug: string
  model: string
}

/** 策略文案（下拉与列表行共用；smart / pace 依赖额度快照，当前按顺序执行）。 */
export const ROUTING_STRATEGIES: readonly { value: string; text: string }[] = [
  { value: "order", text: "顺序（失败换下一个）" },
  { value: "rotate", text: "轮转（逐请求换）" },
  { value: "usage", text: "用量最少优先" },
  { value: "smart", text: "智能（待额度快照，暂按顺序）" },
  { value: "pace", text: "配速（待额度快照，暂按顺序）" },
]

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
  return members.some((item) => item.slug === member.slug && item.model === member.model)
}

/** 勾选 / 取消勾选一个成员：已存在则移除，否则追加到末尾。 */
export function toggleRoutingMember(members: RoutingMember[], member: RoutingMember): RoutingMember[] {
  return hasRoutingMember(members, member)
    ? members.filter((item) => !(item.slug === member.slug && item.model === member.model))
    : [...members, member]
}

/** 成员里不在可选目录中的条目（目录刷新后可能已下架；编辑时保留显示，可取消勾选移除）。 */
export function offCatalogMembers(members: RoutingMember[], models: CatalogEntry[]): RoutingMember[] {
  const selectable = selectableCatalog(models)
  return members.filter((member) => !selectable.some((item) => item.slug === member.slug && item.model === member.model))
}

/** 分组 id 校验：小写字母数字连字符、不以连字符开头；返回错误文案，合法或未填时为 null（未填由按钮禁用负责）。 */
export function routingSlugError(slug: string): string | null {
  if (!slug) return null
  if (!/^[a-z0-9-]+$/.test(slug)) return "只能使用小写字母、数字和连字符。"
  if (slug.startsWith("-")) return "不能以连字符开头。"
  return null
}
