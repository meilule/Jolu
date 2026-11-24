import { useRef, type KeyboardEvent } from "react"
import { cn } from "@/lib/utils"
import { listTime, sessionKey, sessionTitle, statusLabel, visibleRows, type ProjectGroup } from "@/lib/sessionThreads"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { providerName } from "@/lib/providerCatalog"
import { Card } from "@/components/usage/usageControls"
import type { PlatformId } from "@/types"
import { Icon } from "@/icons"
import { Button } from "@/ui"

/**
 * 项目 → 会话两级列表（R1-05）。左右键展开收起，上下键移动，Enter 查看。
 * 列表独立滚动；切换会话不改变筛选与展开状态。
 */
export function SessionTree({ groups, expanded, selected, searching, total, onToggle, onSelect, onClear }: {
  groups: ProjectGroup[]
  expanded: ReadonlySet<string>
  selected: string | null
  searching: boolean
  total: number
  onToggle: (project: string, open?: boolean) => void
  onSelect: (key: string) => void
  onClear: () => void
}) {
  const listRef = useRef<HTMLDivElement>(null)
  const rows = visibleRows(groups, expanded)
  const shown = rows.filter((r) => r.kind === "session").length

  const focusRow = (index: number) => {
    const target = listRef.current?.querySelectorAll<HTMLElement>("[data-tree-row]")[index]
    target?.focus()
    target?.scrollIntoView({ block: "nearest" })
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(listRef.current?.querySelectorAll<HTMLElement>("[data-tree-row]") ?? [])]
    const index = items.indexOf(document.activeElement as HTMLElement)
    if (index < 0) return
    const row = rows[index]
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault()
      focusRow(Math.min(rows.length - 1, Math.max(0, index + (event.key === "ArrowDown" ? 1 : -1))))
    } else if (event.key === "ArrowRight" && row.kind === "project") {
      event.preventDefault()
      onToggle(row.key, true)
    } else if (event.key === "ArrowLeft") {
      event.preventDefault()
      if (row.kind === "project") onToggle(row.key, false)
      else focusRow(rows.findIndex((r) => r.kind === "project" && r.key === row.project))
    } else if (event.key === "Enter") {
      event.preventDefault()
      if (row.kind === "session") onSelect(row.key)
      else onToggle(row.key)
    }
  }

  return (
    <Card className="flex h-full w-[320px] shrink-0 flex-col gap-1.5 p-3">
      <div className="flex items-center justify-between px-1 pb-1">
        <h3 className="text-sm font-semibold text-jn-text">{searching ? "匹配的项目与会话" : "项目与会话"}</h3>
        <span className="tnum text-[11px] text-jn-text-3">{groups.length} 项目 · {total} 条</span>
      </div>
      <div ref={listRef} role="tree" aria-label="项目与会话" onKeyDown={onKeyDown} className="-mx-1 flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto px-1">
        {groups.map((g) => {
          const open = expanded.has(g.key)
          return (
            <div key={g.key} role="group" className="flex flex-col gap-0.5">
              <button
                type="button"
                data-tree-row
                role="treeitem"
                aria-expanded={open}
                onClick={() => onToggle(g.key)}
                className="jn-interactive flex h-8 shrink-0 items-center gap-1.5 rounded-[10px] px-2 text-left outline-none hover:bg-jn-well"
              >
                <Icon name={open ? "expand" : "next"} size="xs" className="shrink-0 text-jn-text-3" />
                <Icon name={open ? "openFolder" : "folder"} size="sm" className="shrink-0 text-jn-text-3" />
                <span className="min-w-0 flex-1 truncate text-xs font-semibold text-jn-text" title={g.path ?? undefined}>{g.name}</span>
                <span className="tnum rounded-full bg-jn-well px-1.5 py-0.5 text-[10.5px] font-medium text-jn-text-3">{g.sessions.length}</span>
              </button>
              {open && (
                <div className="ml-2.5 flex flex-col gap-0.5 border-l border-jn-line pl-2">
                  {g.sessions.map((s) => {
                    const key = sessionKey(s)
                    const active = key === selected
                    return (
                      <button
                        key={key}
                        type="button"
                        data-tree-row
                        role="treeitem"
                        aria-selected={active}
                        onClick={() => onSelect(key)}
                        className={cn(
                          "jn-interactive flex h-10 shrink-0 flex-col justify-center gap-0.5 rounded-[10px] px-2 text-left outline-none [.density-compact_&]:h-9",
                          active ? "bg-jn-primary-wash" : "hover:bg-jn-well",
                        )}
                      >
                        <span className="flex w-full items-center gap-1.5">
                          <PlatformLogo platform={s.platform as PlatformId} size={13} />
                          <span className={cn("min-w-0 flex-1 truncate text-xs", active ? "font-semibold text-jn-primary-ink" : "text-jn-text")}>{sessionTitle(s)}</span>
                          <span className="tnum shrink-0 text-[10px] text-jn-text-3">{listTime(s.last_ts)}</span>
                        </span>
                        <span className="flex gap-2 pl-[19px] text-[10.5px]">
                          <span className="text-jn-text-3">{providerName(s.platform)}</span>
                          <span className={s.status === "running" ? "text-jn-primary-ink" : s.status === "failed" ? "text-jn-danger" : "text-jn-text-3"}>{statusLabel(s.status)}</span>
                        </span>
                      </button>
                    )
                  })}
                </div>
              )}
            </div>
          )
        })}
      </div>
      {searching ? (
        <div className="flex flex-col items-start gap-2 pt-1">
          <span className="text-[11px] text-jn-text-3">仅显示匹配会话，自动展开对应项目。</span>
          <Button kind="ghost" size="sm" icon="retry" onClick={onClear}>清除搜索与筛选</Button>
        </div>
      ) : (
        <span className="px-1 pt-1 text-[10.5px] text-jn-text-3">已展开 {shown} 条会话 · 点击项目展开更多</span>
      )}
    </Card>
  )
}
