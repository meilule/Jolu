/**
 * 侧栏导航的「页面 → 图标」映射。
 *
 * 单独成模块而不是写进 `MainPanelWindow`：
 * - 它是纯数据，可以被测试直接读；
 * - 图标语义名必须在 `icons/ui.ts` 里存在，映射集中一处才好核对；
 * - 旧外壳（`components/panel/SideNav.tsx`）用 lucide 组件，新外壳用语义名，
 *   两张表在替换期间并存；等旧外壳删掉时这张表就是唯一的。
 */
import type { IconName } from "@/icons"
import type { MainPage } from "@/lib/mainNavigation"

export const NAV_ICONS: Record<MainPage, IconName> = {
  island: "navIsland",
  providers: "navProviders",
  checkin: "navCheckin",
  usage: "navUsage",
  sessions: "navSessions",
  automation: "navAutomation",
  skills: "navSkills",
  agents: "navAgents",
  gateway: "navGateway",
  settings: "navSettings",
}
