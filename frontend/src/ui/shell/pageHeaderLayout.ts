/**
 * PageHeader 布局决策纯逻辑。
 *
 * 按真实测量到的各槽位宽度决定页签与页面操作放在哪一行，保证：
 * 1. 第一行左端「标题」与右端「全局操作 + 窗口按钮」永远稳固在顶行，绝不被挤换行；
 * 2. 有 `wideTabs` 标记的页面（如 6 页签的模型路由、带多个页头操作的用量统计）在常规宽度下
 *    稳定把页签放在标题正下方，避免切换页签时因某个页签多出一个操作按钮而上下跳动；
 * 3. 极窄窗口（例如 900px 最小宽度）下，页面操作也会优雅落到副行，窗口按钮始终在右上角。
 */

export const TITLE_TABS_GAP = 16
export const MIN_ROW1_SPACER = 24
export const ACTIONS_CHROME_GAP = 8
export const ROW2_GAP = 12
export const WIDE_TABS_MIN_CONTAINER = 1180

export interface HeaderLayoutInput {
  /** 页头可用宽度（px）；0 表示尚未完成首帧测量 */
  containerWidth: number
  /** 标题 h1 固有宽度（px） */
  titleWidth: number
  /** 页签栏固有宽度（px），无页签时为 0 */
  tabsWidth: number
  /** 页面操作区固有宽度（px），无页面操作时为 0 */
  actionsWidth: number
  /** 全局操作 + 窗口控制按钮总宽度（px） */
  chromeWidth: number
  hasTabs: boolean
  wideTabs: boolean
  hasActions: boolean
}

export interface HeaderLayoutResult {
  /** 页签位置：inline = 顶行标题右侧；below = 标题正下方第二行 */
  tabsRow: "inline" | "below"
  /** 页面操作位置：inline = 顶行全局操作左侧；row2 = 第二行；row3 = 第二行放不下时落到第三行 */
  actionsRow: "inline" | "row2" | "row3"
}

export function resolveHeaderLayout(input: HeaderLayoutInput): HeaderLayoutResult {
  const {
    containerWidth,
    titleWidth,
    tabsWidth,
    actionsWidth,
    chromeWidth,
    hasTabs,
    wideTabs,
    hasActions,
  } = input

  if (containerWidth <= 0) {
    return {
      tabsRow: hasTabs && wideTabs ? "below" : "inline",
      actionsRow: "inline",
    }
  }

  const actionsNeed = hasActions ? actionsWidth + ACTIONS_CHROME_GAP : 0

  // 1. 先检查顶行去掉页签后能否放下「标题 + 弹性间隔 + 页面操作 + 全局/窗口按钮」
  const row1WithoutTabs = titleWidth + MIN_ROW1_SPACER + actionsNeed + chromeWidth
  if (hasActions && row1WithoutTabs > containerWidth) {
    if (!hasTabs) {
      return { tabsRow: "inline", actionsRow: "row2" }
    }
    const row2FitsBoth = tabsWidth + ROW2_GAP + actionsWidth <= containerWidth
    return {
      tabsRow: "below",
      actionsRow: row2FitsBoth ? "row2" : "row3",
    }
  }

  // 2. 页面操作可留在顶行；无页签时直接单行
  if (!hasTabs) {
    return { tabsRow: "inline", actionsRow: "inline" }
  }

  // 3. 标记 wide 的页面在常规宽度下统一放标题下方，切页签不上下跳动
  if (wideTabs && containerWidth < WIDE_TABS_MIN_CONTAINER) {
    return { tabsRow: "below", actionsRow: "inline" }
  }

  // 4. 其余情况按真实总宽判断，放得下才与标题同行
  const row1WithTabs =
    titleWidth + TITLE_TABS_GAP + tabsWidth + MIN_ROW1_SPACER + actionsNeed + chromeWidth

  return {
    tabsRow: row1WithTabs <= containerWidth ? "inline" : "below",
    actionsRow: "inline",
  }
}
