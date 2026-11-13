import { EyeOff, Plus, RefreshCw, Search } from "lucide-react"
import { cn } from "@/lib/utils"
import { GROUP_LABELS, agentBadge, agentRows, type AgentRow } from "@/lib/agentPage"

/**
 * Agent 页左栏「工具列表」（画板 V5-1 / V5-2，宽 300）：列表头、搜索、三组工具、已隐藏。
 * 开关直接接入 / 断开；点整行看右侧详情。
 */
export function AgentList({ rows, selected, detectedAt, busy, search, onSearch, hidden, showHidden, onShowHidden, onSelect, onToggle, onRedetect, onAddCustom }: {
  /** 通用接入（P1-1）：添加 Jolu 没有专门适配的工具 */
  onAddCustom: () => void
  rows: AgentRow[]
  selected: string | null
  detectedAt: number | null
  busy: boolean
  search: string
  onSearch: (value: string) => void
  hidden: string[]
  showHidden: boolean
  onShowHidden: (value: boolean) => void
  onSelect: (agent: string) => void
  onToggle: (row: AgentRow) => void
  onRedetect: () => void
}) {
  const { groups, hiddenCount } = agentRows(rows, search, hidden, showHidden)
  const wired = rows.filter((row) => row.group === "wired").length
  const time = detectedAt ? new Date(detectedAt).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" }) : null
  return (
    <aside className="flex w-[300px] shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-jolu-border px-2.5 py-3.5" aria-label="工具列表">
      <div className="flex items-center gap-2 px-1.5 pb-2">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-[13px] font-semibold text-jolu-text">本机检测到 {rows.length} 个工具</span>
          <span className="text-note text-jolu-muted">{time ? `${time} 检测 · ` : ""}{wired} 个已接入</span>
        </div>
        <button type="button" aria-label="重新检测" title="重新检测" disabled={busy} onClick={onRedetect}
          className="jolu-control flex size-[30px] shrink-0 items-center justify-center rounded-lg border border-jolu-border outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
          <RefreshCw aria-hidden className={cn("size-3.5 text-jolu-muted", busy && "animate-spin")} />
        </button>
      </div>
      <label className="flex h-8 shrink-0 items-center gap-2 rounded-lg bg-jolu-side px-2.5">
        <Search aria-hidden className="size-[13px] shrink-0 text-jolu-muted" />
        <input value={search} onChange={(event) => onSearch(event.target.value)} placeholder="搜索工具" aria-label="搜索工具"
          className="min-w-0 flex-1 bg-transparent text-meta text-jolu-text outline-none placeholder:text-jolu-muted" />
      </label>
      {groups.map(({ group, rows: items }) => (
        <div key={group} className="flex flex-col gap-0.5">
          <div className="flex items-center gap-1.5 px-1.5 pb-1 pt-3">
            <span className="text-note font-semibold text-jolu-muted">{GROUP_LABELS[group]}</span>
            <span className="text-note text-jolu-muted">{items.length}</span>
          </div>
          {items.map((row) => (
            <AgentItem key={row.agent} row={row} active={row.agent === selected} busy={busy} hidden={hidden.includes(row.agent)}
              onSelect={() => onSelect(row.agent)} onToggle={() => onToggle(row)} />
          ))}
        </div>
      ))}
      {groups.length === 0 && <p className="px-1.5 py-4 text-meta text-jolu-muted">{search ? "没有匹配的工具" : "还没有检测结果"}</p>}
      <button type="button" onClick={onAddCustom} disabled={busy}
        className="mt-2 flex h-9 shrink-0 items-center justify-center gap-1.5 rounded-lg border border-dashed border-jolu-border text-meta text-jolu-muted outline-none hover:bg-jolu-side hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
        <Plus aria-hidden className="size-3.5" />添加自定义工具
      </button>
      <div className="flex-1" />
      {(hiddenCount > 0 || showHidden) && (
        <div className="flex items-center gap-1.5 border-t border-jolu-table-divider px-2 pt-2">
          <EyeOff aria-hidden className="size-3.5 text-jolu-muted" />
          <span className="text-meta text-jolu-muted">{showHidden ? "正在显示已隐藏的工具" : `已隐藏 ${hiddenCount} 个`}</span>
          <span className="flex-1" />
          <button type="button" onClick={() => onShowHidden(!showHidden)} className="text-meta font-medium text-jolu-on outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">
            {showHidden ? "收起" : "显示"}
          </button>
        </div>
      )}
    </aside>
  )
}

function AgentItem({ row, active, busy, hidden, onSelect, onToggle }: { row: AgentRow; active: boolean; busy: boolean; hidden: boolean; onSelect: () => void; onToggle: () => void }) {
  const badge = agentBadge(row.agent, row.name)
  const blocked = row.group === "blocked"
  return (
    <div className={cn("flex h-12 shrink-0 items-center gap-2.5 rounded-[9px] px-2", active ? "bg-jolu-selected" : "hover:bg-jolu-stage", blocked && "opacity-75", hidden && "opacity-60")}>
      <button type="button" onClick={onSelect} aria-current={active ? "true" : undefined}
        className="flex min-w-0 flex-1 items-center gap-2.5 text-left outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
        <AgentBadge bg={badge.bg} glyph={badge.glyph} size={24} />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-[13px] font-medium text-jolu-text">{row.name}</span>
          <span className={cn("truncate text-note", row.tone === "danger" ? "text-jolu-danger" : row.tone === "warn" ? "text-[#9A5B00]" : "text-jolu-muted")}>{row.subtitle}</span>
        </span>
      </button>
      {row.toggle && <Switch on={row.toggle === "on"} small disabled={busy} label={`${row.toggle === "on" ? "断开" : "接入"} ${row.name}`} onClick={onToggle} />}
    </div>
  )
}

export function AgentBadge({ bg, glyph, size }: { bg: string; glyph: string; size: 24 | 40 }) {
  return (
    <span aria-hidden style={{ background: bg }}
      className={cn("flex shrink-0 items-center justify-center font-bold text-white", size === 24 ? "size-6 rounded-[7px] text-[10px]" : "size-10 rounded-[10px] text-[16px]")}>
      {glyph}
    </span>
  )
}

/** 胶囊开关（画板 V5-1：列表 28×16、详情头 34×20） */
export function Switch({ on, small, disabled, label, onClick }: { on: boolean; small?: boolean; disabled?: boolean; label: string; onClick: () => void }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} disabled={disabled} onClick={onClick}
      className={cn(
        "flex shrink-0 items-center rounded-full p-0.5 outline-none transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:opacity-45",
        small ? "h-4 w-7" : "h-5 w-[34px]",
        on ? "justify-end bg-jolu-accent" : "justify-start bg-jolu-track",
      )}>
      <span className={cn("rounded-full bg-white shadow-sm", small ? "size-3" : "size-4")} />
    </button>
  )
}
