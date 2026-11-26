import { LoadingSurface, type SkeletonLayout } from "@/components/ui/LoadingSurface"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { forwardRef, type ReactNode } from "react"
import { Check, ChartNoAxesCombined, ChevronDown, LoaderCircle, RefreshCw, RotateCcw, Square, SquareCheck } from "lucide-react"
import { cn } from "@/lib/utils"
import { useGlidePill } from "@/hooks/useGlidePill"
import { usagePlatformOptions } from "@/lib/providerCatalog"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { UNATTRIBUTED_ACCOUNT, accountLabel, accountsTriggerLabel } from "@/lib/usageFilters"
import { JoluButton } from "@/components/ui/JoluButton"
import type { Connection, PlatformId } from "@/types"

/** 统计卡片：24px 圆角、1px 描边、表面投影（R1-04 / R1-C） */
export function Card({ className, children }: { className?: string; children: ReactNode }) {
  return <section className={cn("min-w-0 rounded-jn-card border border-jn-line bg-jn-surface shadow-jn-surface", className)}>{children}</section>
}

export function CardTitle({ title, note, children }: { title: string; note?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex min-h-5 items-center justify-between gap-3">
      <h3 className="truncate text-sm font-semibold text-jn-text">{title}</h3>
      {children ?? (note && <span className="shrink-0 text-xs text-jn-text-3">{note}</span>)}
    </div>
  )
}

/** 四项指标卡（R1-04）：24px 圆角白卡、14px 标题、28px 粗体数字、底部说明 */
export function MetricCard({ icon, label, value, note, noteTitle }: { icon?: ReactNode; label: string; value: string; note: ReactNode; noteTitle?: string }) {
  return (
    <Card className="flex min-h-[112px] flex-col gap-2 px-[18px] pt-[18px] pb-3.5">
      <span className="flex min-w-0 items-center gap-1.5 text-xs font-medium text-jn-text-2">
        {icon && <span aria-hidden className="grid size-4 shrink-0 place-items-center text-jn-text-3 [&_svg]:size-3.5">{icon}</span>}
        <span className="truncate">{label}</span>
      </span>
      <span className="tnum truncate text-[28px] font-bold leading-none tracking-tight text-jn-text">{value}</span>
      {/* 费用说明带未收录模型名，单行放不下；两行后仍超出的部分交给 title 悬停 */}
      <span className="line-clamp-2 text-[11px] leading-relaxed text-jn-text-3" title={noteTitle}>{note}</span>
    </Card>
  )
}

const FILTER_CLASS =
  "jn-interactive inline-flex h-9 shrink-0 items-center gap-2 rounded-full border border-jn-line bg-jn-surface px-3.5 text-xs font-medium text-jn-text shadow-jn-surface outline-none hover:bg-jn-well data-[state=open]:bg-jn-well disabled:cursor-not-allowed disabled:opacity-45"

/** 筛选按钮（R1-04 / R1-24）：白底胶囊 + 旋转箭头 */
export const FilterTrigger = forwardRef<HTMLButtonElement, React.ButtonHTMLAttributes<HTMLButtonElement> & { leading?: ReactNode; trailingChevron?: boolean }>(
  function FilterTrigger({ leading, trailingChevron, className, children, ...props }, ref) {
    return (
      <button ref={ref} type="button" className={cn(FILTER_CLASS, className)} {...props}>
        {leading && <span aria-hidden className="flex shrink-0 items-center text-jn-text-3">{leading}</span>}
        <span className="max-w-[180px] truncate">{children}</span>
        {(!leading || trailingChevron) && <ChevronDown aria-hidden className="jn-chevron size-3.5 shrink-0 text-jn-text-3" />}
      </button>
    )
  },
)

/** 工具按钮：刷新、导出 CSV、重置（R1 胶囊形态） */
export function ToolButton({ icon, children, bordered, busy, className, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement> & { icon: ReactNode; bordered?: boolean; busy?: boolean }) {
  return (
    <button
      type="button"
      aria-busy={busy || undefined}
      className={cn(
        "jn-interactive inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full px-3.5 text-xs font-semibold text-jn-text outline-none disabled:cursor-not-allowed disabled:opacity-45",
        bordered ? "border border-jn-line bg-jn-surface shadow-jn-surface hover:bg-jn-well" : "bg-jn-well hover:bg-jn-track",
        className,
      )}
      {...props}
    >
      <span aria-hidden className={cn("grid size-4 place-items-center text-jn-text-2 [&_svg]:size-3.5", busy && "[&_svg]:animate-spin")}>{icon}</span>
      {children}
    </button>
  )
}

export function RefreshButton({ loading, onClick }: { loading: boolean; onClick: () => void }) {
  return <ToolButton bordered icon={<RefreshCw />} busy={loading} disabled={loading} onClick={onClick}>刷新</ToolButton>
}

export function ResetButton({ onClick, disabled }: { onClick: () => void; disabled?: boolean }) {
  return <ToolButton icon={<RotateCcw />} onClick={onClick} disabled={disabled}>重置</ToolButton>
}

const MENU_CLASS = "jn-popover z-[140] min-w-[var(--radix-dropdown-menu-trigger-width)] max-w-[320px] overflow-y-auto overscroll-contain rounded-[14px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface"
const ITEM_CLASS = "relative flex cursor-pointer items-center gap-2 rounded-[10px] px-2.5 py-2 text-xs text-jn-text select-none outline-none data-[highlighted]:bg-jn-well data-[disabled]:opacity-40"

export interface MenuOption {
  value: string
  label: string
  icon?: ReactNode
  meta?: string
}

/** 单选下拉：全部供应商 / 全部模型 / 全部状态（方案 A：选中带图标与主色描边，菜单内展示图标与计数） */
export function MenuFilter({ value, options, onChange, label, trailingChevron, active, menuClassName }: {
  value: string
  options: MenuOption[]
  onChange: (value: string) => void
  label: string
  trailingChevron?: boolean
  active?: boolean
  menuClassName?: string
}) {
  // 共享滑块动效：只往菜单里插一个滑块，菜单与行的类名一字不改
  const glide = useGlidePill()
  const selected = options.find((option) => option.value === value) ?? options[0]
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger asChild>
        <FilterTrigger
          aria-label={label}
          leading={selected?.icon}
          trailingChevron={trailingChevron}
          className={active ? "border-jn-primary text-jn-text" : undefined}
        >
          {selected?.label}
        </FilterTrigger>
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content ref={glide} align="start" sideOffset={6} collisionPadding={12} className={cn(MENU_CLASS, menuClassName)} style={{ maxHeight: "min(360px, var(--radix-dropdown-menu-content-available-height))" }}>
          <Menu.RadioGroup value={value} onValueChange={onChange}>
            {options.map((option) => (
              <Menu.RadioItem key={option.value} value={option.value} className={cn(ITEM_CLASS, "pr-7 data-[state=checked]:text-jn-primary-ink")}>
                {option.icon && <span aria-hidden className="grid size-4 shrink-0 place-items-center">{option.icon}</span>}
                <span className="min-w-0 flex-1 truncate">{option.label}</span>
                {option.meta && <span className="tnum shrink-0 font-mono text-[11px] text-jn-text-3">{option.meta}</span>}
                <Menu.ItemIndicator className="absolute right-2"><Check aria-hidden className="size-3.5" /></Menu.ItemIndicator>
              </Menu.RadioItem>
            ))}
          </Menu.RadioGroup>
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

export const ALL = "__all__"

export function providerOptions(counts?: ReadonlyMap<string, number>, totalCount?: number): MenuOption[] {
  const base = usagePlatformOptions().map((option, index) => ({
    ...option,
    index,
    count: counts?.get(option.value) ?? 0,
  }))
  if (counts && counts.size > 0) {
    base.sort((a, b) => (b.count > 0 ? 1 : 0) - (a.count > 0 ? 1 : 0) || b.count - a.count || a.index - b.index)
  }
  return [
    { value: ALL, label: "全部供应商", meta: totalCount && totalCount > 0 ? `${totalCount} 次` : undefined },
    ...base.map((option) => ({
      value: option.value,
      label: option.label,
      icon: <PlatformLogo platform={option.value as PlatformId} size={15} />,
      meta: option.count > 0 ? `${option.count} 次` : undefined,
    })),
  ]
}

/** 账号筛选：多选（画板 38「账号筛选」）；勾选「全部账号」即清空选择 */
export function AccountFilter({ accounts, value, onChange }: {
  accounts: readonly Connection[]
  value: readonly string[]
  onChange: (next: string[]) => void
}) {
  const glide = useGlidePill()
  const toggle = (id: string) => onChange(value.includes(id) ? value.filter((v) => v !== id) : [...value, id])
  const row = (id: string, label: string, checked: boolean, onSelect: () => void) => (
    <Menu.CheckboxItem
      key={id}
      checked={checked}
      onSelect={(event) => {
        event.preventDefault()
        onSelect()
      }}
      className={cn(ITEM_CLASS, "h-7", checked && "bg-jn-primary-wash data-[highlighted]:bg-jn-primary-wash")}
    >
      {checked ? <SquareCheck aria-hidden className="size-[15px] text-jn-primary-ink" /> : <Square aria-hidden className="size-[15px] text-jn-text-3" />}
      <span className="truncate">{label}</span>
    </Menu.CheckboxItem>
  )
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger asChild>
        <FilterTrigger aria-label="账号筛选">{accountsTriggerLabel(value, accounts)}</FilterTrigger>
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content ref={glide} align="start" sideOffset={6} collisionPadding={12} className={cn(MENU_CLASS, "w-[260px]")} style={{ maxHeight: "min(360px, var(--radix-dropdown-menu-content-available-height))" }}>
          {row("all", "全部账号", value.length === 0, () => onChange([]))}
          {accounts.map((account) => row(account.id, accountLabel(account), value.includes(account.id), () => toggle(account.id)))}
          {row(UNATTRIBUTED_ACCOUNT, "未归属账号", value.includes(UNATTRIBUTED_ACCOUNT), () => toggle(UNATTRIBUTED_ACCOUNT))}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

// providerName 已移到 `lib/providerCatalog.ts`：它是纯逻辑（平台 id → 显示名），
// 放在组件目录里无法按项目规范配 .test.mjs；调用方一律从 lib 直接导入。

/** 首次加载骨架（画板 38「正在读取本地用量」） */
export function LoadingCard({ title = "正在读取本地用量", layout = "table" }: { title?: string; layout?: SkeletonLayout }) {
  return <LoadingSurface label={title} layout={layout} />
}

/** 筛选无结果（画板 38）；没有可清除的筛选时不显示按钮 */
export function EmptyCard({ onClear }: { onClear?: () => void }) {
  return (
    <Card className="flex flex-col items-start gap-3 p-4">
      <ChartNoAxesCombined aria-hidden className="size-[26px] text-jn-text-3" />
      <span className="text-base font-semibold text-jn-text">这个范围内暂无用量</span>
      <span className="text-xs text-jn-text-3">尝试扩大日期范围，或清除账号筛选。</span>
      {onClear && <ToolButton icon={<RotateCcw />} onClick={onClear}>清除筛选</ToolButton>}
    </Card>
  )
}

/** 读取失败：没有上次数据时整块显示；有上次数据时只在顶部提示并保留数据 */
export function ErrorNotice({ error, onRetry, stale }: { error: string; onRetry: () => void; stale: boolean }) {
  return (
    <div role="alert" className="flex items-center justify-between gap-3 rounded-[10px] bg-jn-danger-soft px-4 py-2.5 text-xs text-jn-danger">
      <span className="min-w-0 truncate">{stale ? `刷新失败，暂时显示上次数据：${error}` : `本地用量读取失败：${error}`}</span>
      <JoluButton size="sm" variant="danger-soft" onClick={onRetry}>重试</JoluButton>
    </div>
  )
}

export function Spinner() {
  return <LoaderCircle aria-hidden className="size-4 animate-spin text-jn-text-3" />
}
