import { useState } from "react"
import { AGENT_FILTERS, agentGridRows, type AgentFilter, type AgentRow } from "@/lib/agentPage"
import { cn } from "@/lib/utils"
import { Icon } from "@/icons"
import { Collapse } from "@/motion"
import { AgentBadge } from "./AgentBadge"
import { Button, Input, StatusPill, Switch as JnSwitch, Tabs, type StatusTone } from "@/ui"

/**
 * Agent 页 · 列表态 —— 设计稿 R1-08「Agent（浅色）」。
 *
 * 结构（自顶向下）：
 * 1. **说明行**：左侧一句副标题 + 右侧接入状态大分段 + 搜索；
 * 2. **工具栅格**：三列工具卡（容器宽度不够时降为两列 / 一列）；
 * 3. **暂不能接入**折叠条（画板未画，沿用 V8 的信息结构，标「待确认」）；
 * 4. **接入安全提示**一行。
 *
 * 工具卡（画板 328×218 量级）：标识 + 名称 + 开关；模型摘要三行；
 * 底栏 = 状态胶囊 + 「详细」。卡片不再用 `.agent-card` 旧 CSS 类，
 * 悬停浮起与选中描边都走 `jn-*` 令牌。
 *
 * 「重新检测 / 添加工具」在页头（设计稿 R1-08 的「页面操作」槽），
 * 由 `AgentsView` 通过 `PageActions` 送进页头，不在本文件渲染。
 */

export function AgentDirectory({ rows, selected, modelsCount, tierCount, busy, search, onSearch, hidden, showHidden, onShowHidden, onSelect, onToggle }: {
  rows: AgentRow[]
  selected?: string | null
  /** /model 菜单模型数（可接入卡摘要「N 个模型」） */
  modelsCount: number
  /** Claude Code 摘要里的档位项数（主模型 + 四档 + 子代理，画板示例写 6） */
  tierCount: number
  busy: boolean
  search: string
  onSearch: (value: string) => void
  hidden: string[]
  showHidden: boolean
  onShowHidden: (value: boolean) => void
  onSelect: (agent: string) => void
  onToggle: (row: AgentRow) => void
}) {
  const [filter, setFilter] = useState<AgentFilter>("all")
  const [blockedOpen, setBlockedOpen] = useState(false)
  const grid = agentGridRows(rows, filter, search, hidden, showHidden)
  const counts: Record<AgentFilter, number> = {
    all: grid.rows.length + grid.blocked.length,
    wired: rows.filter((row) => row.group === "wired").length,
    available: rows.filter((row) => row.group === "available").length,
  }
  return (
    // 子元素必须 shrink-0：flex-col 默认允许压缩，压缩后内容溢出被裁、滚动容器反而不触发滚动
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3.5 overflow-y-auto px-0.5 pb-2 pt-0.5 [&>*]:shrink-0">
      {/* 说明行：副标题 + 大分段 + 搜索。窄窗口时整体折行 */}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2.5">
        <span className="whitespace-nowrap text-[15px] font-bold text-jn-text">接入模型路由，为每个工具配置顺手的模型</span>
        <span className="min-w-0 flex-1" />
        <Tabs
          style="segment-lg"
          value={filter}
          onChange={setFilter}
          items={AGENT_FILTERS.map((item) => ({ key: item.id, label: `${item.label} ${counts[item.id]}` }))}
        />
        <Input
          size="md"
          icon="search"
          value={search}
          onChange={(event) => onSearch(event.target.value)}
          placeholder="搜索工具"
          aria-label="搜索工具"
          className="w-[200px] shrink-0"
        />
      </div>

      {/* 隐藏的工具单独计数（画板未画，沿用 V8 的「已隐藏」入口，标「待确认」） */}
      {(grid.hiddenCount > 0 || showHidden) && (
        <div className="flex items-center gap-2">
          <Button kind="ghost" size="sm" icon="eyeOff" aria-pressed={showHidden} onClick={() => onShowHidden(!showHidden)}>
            {showHidden ? "收起已隐藏" : `已隐藏 ${grid.hiddenCount}`}
          </Button>
        </div>
      )}

      {/* 工具卡片栅格：三列需要约 1000px 内容宽（328×3 + 间距 + 页边距），不够就 2 列、再窄 1 列 */}
      <div className="@container">
        <div className="grid grid-cols-1 gap-3.5 @[680px]:grid-cols-2 @[940px]:grid-cols-3">
          {grid.rows.map((row) => (
            <ToolCard key={row.agent} row={row} selected={selected === row.agent} modelsCount={modelsCount} tierCount={tierCount} busy={busy}
              onSelect={() => onSelect(row.agent)} onToggle={() => onToggle(row)} />
          ))}
        </div>
        {grid.rows.length === 0 && (
          <p className="rounded-jn-card border border-dashed border-jn-line px-4 py-10 text-center text-[12px] text-jn-text-3">
            {search ? "没有匹配的工具" : filter === "wired" ? "还没有已接入的工具" : "这个筛选下没有工具"}
          </p>
        )}
      </div>

      {/* 暂不能接入折叠条（画板 V8-01 的信息结构；R1-08 未画，标「待确认」） */}
      {grid.blocked.length > 0 && (
        <div className="flex flex-col overflow-hidden rounded-[16px] bg-jn-well">
          <button type="button" aria-expanded={blockedOpen} onClick={() => setBlockedOpen((value) => !value)}
            className="jn-interactive flex h-12 shrink-0 items-center gap-2.5 px-4 text-left">
            <Icon name="expand" size="sm" className={cn("jn-chevron shrink-0 text-jn-text-3", !blockedOpen && "-rotate-90")} />
            <span className="text-[13px] font-semibold text-jn-text">暂不能接入</span>
            <span className="rounded-full bg-jn-surface px-2 py-0.5 text-[11px] font-semibold tabular-nums text-jn-text-3">{grid.blocked.length}</span>
            <span className="flex-1" />
            <span className="text-[11px] text-jn-text-3">{blockedOpen ? "收起" : "原因"}</span>
          </button>
          <Collapse open={blockedOpen}>
            <div className="grid grid-cols-1 gap-2.5 bg-jn-surface p-3 @[680px]:grid-cols-2 @[940px]:grid-cols-3">
              {grid.blocked.map((row) => (
                <div key={row.agent} className="flex items-center gap-3 rounded-[14px] border border-jn-line px-3.5 py-3">
                  <AgentBadge agent={row.agent} name={row.name} size={34} />
                  <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className="truncate text-[13px] font-semibold text-jn-text">{row.name}</span>
                    <span className="truncate text-[11px] text-jn-text-3">{row.subtitle}</span>
                  </span>
                </div>
              ))}
            </div>
          </Collapse>
        </div>
      )}

      {/* 接入安全提示（画板 R1-08 底部一行） */}
      <div className="flex items-center gap-2 px-0.5 text-[11px] text-jn-text-3">
        <Icon name="shield" size="sm" className="shrink-0" />
        <span>开启前预览配置改动，关闭时可还原 Jolu 管理的字段。</span>
      </div>
    </div>
  )
}

/** 工具卡（R1-08）：标题行（标识 + 名称 + 开关）→ 模型摘要 → 底栏（状态 + 详细）。 */
function ToolCard({ row, selected, modelsCount, tierCount, busy, onSelect, onToggle }: {
  row: AgentRow
  selected?: boolean
  modelsCount: number
  tierCount: number
  busy: boolean
  onSelect: () => void
  onToggle: () => void
}) {
  const wired = row.group === "wired"
  const claude = row.agent === "claude-code"
  const model = row.wiring?.current_resolved || row.wiring?.current_model || ""
  const statusTone: StatusTone = !wired ? (row.supported ? "neutral" : "muted") : row.tone === "danger" ? "danger" : row.tone === "warn" ? "warning" : "success"
  return (
    <div className={cn(
      "flex min-h-[218px] flex-col gap-3.5 rounded-jn-card border bg-jn-surface p-[18px] pb-3.5 shadow-jn-surface transition-[border-color,box-shadow,transform] duration-200",
      selected ? "border-jn-primary shadow-[0_0_0_1px_var(--jn-focus)]" : "border-jn-line hover:-translate-y-0.5 hover:border-jn-mint-2",
    )}>
      {/* 身份与开关 */}
      <div className="flex items-center gap-2.5">
        <AgentBadge agent={row.agent} name={row.name} size={40} />
        <button type="button" onClick={onSelect} className="flex min-w-0 flex-1 flex-col gap-0.5 text-left">
          <span className="truncate text-[15px] font-bold text-jn-text">{row.name}</span>
          {/* 只有需要处理时（漂移 / 配置文件被删等）才占第二行 */}
          {row.tone === "danger" && <span className="truncate text-[11px] text-jn-danger">{row.subtitle}</span>}
        </button>
        {/* 暂不支持的也有开关形状（track 底），只是禁用不可拨 */}
        <JnSwitch checked={row.toggle === "on"} disabled={busy || !row.toggle} label={row.toggle ? `${row.toggle === "on" ? "断开" : "接入"} ${row.name}` : `${row.name} 暂不支持接入`} onChange={() => onToggle()} />
      </div>
      {/* 模型摘要三行（画板：标签 11 / 值 14 粗 / 注 11.5） */}
      <button type="button" onClick={onSelect}
        className="jn-interactive flex min-h-[70px] flex-1 flex-col items-start justify-center gap-[3px] rounded-[16px] bg-jn-well px-3.5 py-3 text-left">
        {wired ? (
          <>
            <span className="text-[11px] font-medium text-jn-text-3">新会话默认</span>
            <span className="max-w-full truncate text-[14px] font-bold text-jn-text" title={model || undefined}>{model || "工具自己上次的选择"}</span>
            <span className="max-w-full truncate text-[11.5px] text-jn-text-3">
              {claude ? `${tierCount} 个档位 · ${modelsCount} 个菜单模型` : row.wiring?.codex_mode === "oauth" ? "保留 ChatGPT 登录" : `${modelsCount} 个菜单模型`}
            </span>
          </>
        ) : row.supported ? (
          <>
            <span className="text-[11px] font-medium text-jn-text-3">现在</span>
            <span className="max-w-full truncate text-[14px] font-bold text-jn-text" title={row.subtitle}>{row.subtitle}</span>
            <span className="max-w-full truncate text-[11.5px] text-jn-text-3">接入后可用 {modelsCount} 个模型</span>
          </>
        ) : (
          <>
            <span className="text-[11px] font-medium text-jn-text-3">暂无专用接入</span>
            <span className="max-w-full truncate text-[14px] font-bold text-jn-text">按「添加工具」通用接入</span>
            <span className="max-w-full truncate text-[11.5px] text-jn-text-3">{row.subtitle}</span>
          </>
        )}
      </button>
      {/* 底栏：状态 + 详细 */}
      <div className="flex h-6 shrink-0 items-center gap-2">
        <StatusPill tone={statusTone} className="max-w-[210px] overflow-hidden">{row.status}</StatusPill>
        <span className="min-w-0 flex-1" />
        <button type="button" onClick={onSelect} title={row.supported ? undefined : "Jolu 还没有它的专用接入，可按「添加工具」走通用接入"}
          className="jn-interactive inline-flex h-6 shrink-0 items-center gap-1 rounded-full px-1 text-[12.5px] font-semibold text-jn-primary-ink">
          详细
          <Icon name="next" size="xs" />
        </button>
      </div>
    </div>
  )
}

export { AgentBadge }
