/**
 * 组件库统一出口（对应设计稿 R1-C 的六个分区）。
 *
 * 页面只从这里拿组件：`import { Button, Card, DataValue } from "@/ui"`。
 * 不从子目录深引（`@/ui/actions/Button`）——那样内部文件可以随意挪位置，
 * 而页面会记住旧路径。深层引用只在组件库内部使用。
 *
 * ## 目录与分区对应
 *
 * ```
 * shell/      1 · 外壳          WindowShell Sidebar NavItem PageHeader PanelPage
 * actions/    2 · 按钮与操作    Button IconButton TabItem TabBar OptionTile
 * inputs/     3 · 选择与输入    Select Menu Input FormField Switch Checkbox SettingRow
 * data/       4 · 状态与数据    StatusPill Count Tag CapabilityTag DataValue SourceNote
 *                              QuotaRing Progress PlatformLogo MetricCard
 * containers/ 5 · 容器与反馈    Card ListRow Table Pagination Dialog Toast Tooltip
 *                              Banner EmptyState Skeleton BatchBar
 * charts/     6 · 图表          Legend BarRow ChartTooltip
 * ```
 *
 * 当前已实现的组件逐批导出；未实现的在对应分区文件建好后再加到这里。
 * 这样「导出了但还没写」不可能发生——导入不存在的组件会直接编译失败。
 */

// 1 · 外壳
export { NavItem, type NavItemProps } from "./shell/NavItem"
export { PageActions, PageActionsProvider, PageTabs, usePageActions, usePageTabs } from "./shell/PageActions"
export { PageHeader, type PageHeaderProps } from "./shell/PageHeader"
export { Sidebar, type SidebarProps } from "./shell/Sidebar"
export { PANEL_PAGES, usesOwnScroll } from "./shell/PanelPage"
export { WindowControls } from "./shell/WindowControls"
export { SHELL_HEIGHT, SHELL_WIDTH, WindowShell, type WindowShellProps } from "./shell/WindowShell"

// 2 · 按钮与操作
export { Button, type ButtonKind, type ButtonProps, type ButtonSize } from "./actions/Button"
export { IconButton, type IconButtonKind, type IconButtonProps, type IconButtonSize } from "./actions/IconButton"
export { OptionTile, type OptionTileProps } from "./actions/OptionTile"
export { TabItem, Tabs, type TabStyle } from "./actions/Tabs"

// 3 · 选择与输入
export { Checkbox, type CheckboxProps } from "./inputs/Checkbox"
export { FormField, type FormFieldProps } from "./inputs/FormField"
export { Input, type InputProps, type InputSize } from "./inputs/Input"
export { Select, type SelectOption, type SelectProps, type SelectVariant, type SelectWidth } from "./inputs/Select"
export { SettingGroup, SettingRow, type SettingRowProps } from "./inputs/SettingRow"
export { Switch, type SwitchProps } from "./inputs/Switch"

// 4 · 状态与数据
export { CapabilityTag, type CapabilityLevel } from "./data/CapabilityTag"
export { Count, Tag, type CountTone } from "./data/CountTag"
export { DataValue, type DataValueProps, type FormatNumber } from "./data/DataValue"
export { MetricCard, type MetricCardProps } from "./data/MetricCard"
export { Progress, type ProgressLevel, type ProgressProps } from "./data/Progress"
export { QuotaRing, quotaLevel, type QuotaRingLevel, type QuotaRingProps, type QuotaRingSize } from "./data/QuotaRing"
export { SourceNote, type DataSource, type SourceNoteProps } from "./data/SourceNote"
export { StatusPill, type StatusPillProps, type StatusTone } from "./data/StatusPill"

// 5 · 容器与反馈
export { BatchAction, BatchBar, type BatchBarProps } from "./containers/BatchBar"
export { Card, CardTitle, type CardProps, type CardTone } from "./containers/Card"
export {
  ConfirmDialog,
  Dialog,
  type DialogProps,
  type DialogWidth,
} from "./containers/Dialog"
export { ListRow, type ListRowProps, type ListRowState } from "./containers/ListRow"
export { Pagination, type PaginationProps } from "./containers/Pagination"
export { pageItems } from "./containers/pageItems"
export {
  Banner,
  EmptyState,
  Skeleton,
  SkeletonLines,
  type BannerTone,
  type EmptyStateProps,
} from "./containers/States"
export { Table, TableRow, type TableColumn } from "./containers/Table"
export { ToastProvider, useToast, type ToastTone } from "./containers/Toast"
export { Tooltip, TooltipProvider } from "./containers/Tooltip"

// 6 · 图表
export {
  BarRow,
  ChartTooltip,
  Legend,
  VIZ_BG,
  VIZ_VAR,
  vizSeriesAt,
  type BarRowProps,
  type ChartTooltipRow,
  type LegendItemData,
  type VizSeries,
} from "./charts"
