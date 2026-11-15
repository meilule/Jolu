import { useState } from "react"
import { ChevronRight, EyeOff, Plus, RefreshCw, Search } from "lucide-react"
import { cn } from "@/lib/utils"
import { AGENT_PLATFORM, agentRows, type AgentRow } from "@/lib/agentPage"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import opencodeLogo from "@/assets/platforms/opencode.svg"

/**
 * Agent 页 · 列表态（Jolu.pen「统一网关 · R2 重设计」V6 画板 Ck1N5）：
 * 大标题 + 搜索/检测行 + 「已接入」行卡（点行或「配置」进详情）+「可接入」卡片栅格 +
 * 「暂不能接入」折叠行。展开态画板只给了收起样式，展开内容沿用详情页的
 * 「暂时不能接入」说明，标「待确认」。
 */

export type AgentCounts = {
  skills: number | null
  sessions: number | null
  automation: number | null
  routes: number | null
}

export function AgentDirectory({ rows, modelsCount, busy, search, onSearch, hidden, showHidden, onShowHidden, onSelect, onRedetect, onAddCustom }: {
  rows: AgentRow[]
  /** /model 菜单模型数（已接入行的「N 个菜单模型」） */
  modelsCount: number
  busy: boolean
  search: string
  onSearch: (value: string) => void
  hidden: string[]
  showHidden: boolean
  onShowHidden: (value: boolean) => void
  onSelect: (agent: string) => void
  onRedetect: () => void
  onAddCustom: () => void
}) {
  const { groups } = agentRows(rows, search, hidden, showHidden)
  const wired = groups.find((entry) => entry.group === "wired")?.rows ?? []
  const available = groups.find((entry) => entry.group === "available")?.rows ?? []
  const blocked = groups.find((entry) => entry.group === "blocked")?.rows ?? []
  const hiddenCount = rows.filter((row) => hidden.includes(row.agent)).length
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-6 overflow-y-auto px-8 pb-8 pt-7">
      <div className="flex items-start justify-between gap-4">
        <div className="flex flex-col gap-1.5">
          <h1 className="text-[24px] font-semibold text-jolu-text">你的 AI 工具</h1>
          <p className="text-meta text-jolu-muted">为每个工具选择模型，接入前先看配置将怎样改变。</p>
        </div>
        <button type="button" onClick={onAddCustom} disabled={busy}
          className="flex h-10 shrink-0 items-center gap-1.5 rounded-lg bg-jolu-text px-4 text-meta font-semibold text-jolu-bg outline-none hover:opacity-90 focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
          <Plus aria-hidden className="size-4" />添加工具
        </button>
      </div>

      <div className="flex items-center justify-between gap-3">
        <label className="flex h-10 w-[420px] max-w-full items-center gap-2.5 rounded-lg bg-jolu-side px-3.5">
          <Search aria-hidden className="size-4 shrink-0 text-jolu-muted" />
          <input value={search} onChange={(event) => onSearch(event.target.value)} placeholder="搜索工具" aria-label="搜索工具"
            className="min-w-0 flex-1 bg-transparent text-meta text-jolu-text outline-none placeholder:text-jolu-muted" />
        </label>
        <div className="flex shrink-0 items-center gap-2.5">
          <button type="button" disabled={busy} onClick={onRedetect} aria-label="重新检测"
            className="flex h-10 items-center gap-2 rounded-lg border border-jolu-border px-3.5 text-meta text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
            <RefreshCw aria-hidden className={cn("size-3.5 text-jolu-muted", busy && "animate-spin")} />重新检测
          </button>
          {(hiddenCount > 0 || showHidden) && (
            <button type="button" onClick={() => onShowHidden(!showHidden)} aria-pressed={showHidden}
              className="flex h-10 items-center gap-2 rounded-lg border border-jolu-border px-3.5 text-meta text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
              <EyeOff aria-hidden className="size-3.5 text-jolu-muted" />{showHidden ? "收起已隐藏" : `已隐藏 ${hiddenCount}`}
            </button>
          )}
        </div>
      </div>

      {wired.length > 0 && (
        <section className="flex flex-col gap-3">
          <div className="flex items-center justify-between">
            <SectionTitle label="已接入" count={wired.length} />
            <span className="text-note text-jolu-muted">点击工具查看与调整配置</span>
          </div>
          <div className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
            {wired.map((row, index) => (
              <WiredRow key={row.agent} row={row} modelsCount={modelsCount} first={index === 0} onSelect={() => onSelect(row.agent)} />
            ))}
          </div>
        </section>
      )}

      {available.length > 0 && (
        <section className="flex flex-col gap-3">
          <SectionTitle label="可接入" count={available.length} />
          <div className="grid grid-cols-3 gap-4">
            {available.map((row) => (
              <AvailableCard key={row.agent} row={row} busy={busy} onSelect={() => onSelect(row.agent)} />
            ))}
          </div>
        </section>
      )}

      {blocked.length > 0 && <BlockedSection rows={blocked} />}

      {groups.length === 0 && <p className="py-8 text-center text-meta text-jolu-muted">{search ? "没有匹配的工具" : "还没有检测结果，点「重新检测」扫描本机工具"}</p>}
    </div>
  )
}

function SectionTitle({ label, count }: { label: string; count: number }) {
  return (
    <span className="flex items-center gap-2">
      <span className="text-[15px] font-semibold text-jolu-text">{label}</span>
      <span className="rounded-md bg-jolu-track px-1.5 py-0.5 text-note font-semibold tabular-nums text-jolu-muted">{count}</span>
    </span>
  )
}

/** 已接入行（画板 Ck1N5）：logo + 名称状态 ｜ 主模型全名 + 档位/菜单说明 ｜ 「↗ 配置」。 */
function WiredRow({ row, modelsCount, first, onSelect }: { row: AgentRow; modelsCount: number; first: boolean; onSelect: () => void }) {
  const claude = row.agent === "claude-code"
  const model = row.wiring?.current_resolved || row.wiring?.current_model || ""
  const second = claude
    ? `${claude ? 4 : 0} 个档位 · ${modelsCount} 个菜单模型`
    : row.wiring?.codex_mode === "oauth"
      ? "保留 ChatGPT 登录"
      : row.subtitle
  return (
    <button type="button" onClick={onSelect}
      className={cn("flex items-center gap-4 px-5 py-4 text-left outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-jolu-accent", !first && "border-t border-jolu-table-divider")}>
      <AgentBadge agent={row.agent} name={row.name} size={40} />
      <span className="flex w-[240px] shrink-0 flex-col gap-1">
        <span className="truncate text-[15px] font-semibold text-jolu-text">{row.name}</span>
        <span className={cn("truncate text-note", row.tone === "danger" ? "text-jolu-danger" : row.tone === "warn" ? "text-[#9A5B00]" : "text-jolu-on")}>{row.subtitle}</span>
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="truncate text-meta font-medium text-jolu-text" title={model || undefined}>{model || "未选择模型"}</span>
        <span className="truncate text-note text-jolu-muted">{second}</span>
      </span>
      <span className="flex h-10 shrink-0 items-center gap-1.5 rounded-lg border border-jolu-border px-4 text-meta font-medium text-jolu-text">
        <ArrowUpRightMini />配置
      </span>
    </button>
  )
}

function ArrowUpRightMini() {
  return (
    <svg aria-hidden viewBox="0 0 14 14" className="size-3 text-jolu-muted" fill="none" stroke="currentColor" strokeWidth="1.5">
      <path d="M4 10L10 4M10 4H5.5M10 4V8.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}

/** 可接入卡（画板 Ck1N5）：logo + 名称、说明、「查看接入预览 →」。 */
function AvailableCard({ row, busy, onSelect }: { row: AgentRow; busy: boolean; onSelect: () => void }) {
  return (
    <button type="button" onClick={onSelect} disabled={busy}
      className="flex flex-col gap-2.5 rounded-xl border border-jolu-border px-5 py-5 text-left outline-none hover:border-jolu-accent focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
      <span className="flex items-center gap-3">
        <AgentBadge agent={row.agent} name={row.name} size={40} />
        <span className="truncate text-[15px] font-semibold text-jolu-text">{row.name}</span>
      </span>
      <span className="truncate text-meta text-jolu-muted">{row.subtitle}</span>
      <span className="flex items-center gap-1.5 text-meta font-medium text-jolu-on">
        查看接入预览
        <ChevronRight aria-hidden className="size-3.5" />
      </span>
    </button>
  )
}

/** 暂不能接入（画板 Ck1N5 只画了收起行；展开内容沿用详情说明，标「待确认」）。 */
function BlockedSection({ rows }: { rows: AgentRow[] }) {
  const [open, setOpen] = useState(false)
  return (
    <section className="flex flex-col gap-3">
      <button type="button" aria-expanded={open} onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2 rounded-xl border border-jolu-border px-5 py-3.5 text-left outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
        <ChevronRight aria-hidden className={cn("size-4 shrink-0 text-jolu-muted transition-transform", open && "rotate-90")} />
        <span className="text-[15px] font-semibold text-jolu-text">暂不能接入</span>
        <span className="rounded-md bg-jolu-track px-1.5 py-0.5 text-note font-semibold tabular-nums text-jolu-muted">{rows.length}</span>
        <span className="flex-1" />
        <span className="text-note text-jolu-muted">{open ? "收起" : "展开查看原因与解决方式"}</span>
      </button>
      {open && (
        <div className="grid grid-cols-3 gap-4">
          {rows.map((row) => (
            <div key={row.agent} className="flex flex-col gap-2.5 rounded-xl border border-jolu-border px-5 py-4">
              <span className="flex items-center gap-3">
                <AgentBadge agent={row.agent} name={row.name} size={40} />
                <span className="min-w-0 flex-1 truncate text-[15px] font-semibold text-jolu-text">{row.name}</span>
              </span>
              <span className="text-meta text-jolu-muted">{row.subtitle}</span>
              <span className="text-note text-jolu-muted">
                {row.discovery?.state === "cli_missing"
                  ? "装回它的命令行程序后点「重新检测」。"
                  : row.discovery?.state === "not_found"
                    ? "本机没有检测到；装好并运行过一次后再检测。"
                    : "满足条件后再点「重新检测」。"}
              </span>
            </div>
          ))}
        </div>
      )}
    </section>
  )
}

/**
 * 工具标识：Jolu 支持的工具一律用官方 logo（鼠鼠 2026-11-13 要求）。
 * Claude Code / Codex / Gemini CLI / ZCode 复用 `PlatformLogo` 的官方图形；OpenCode 用官网
 * favicon（`assets/platforms/opencode.svg`）。自定义工具没有官方 logo，用首字母小方块。
 */
export function AgentBadge({ agent, name, size }: { agent: string; name: string; size: 24 | 40 }) {
  const platform = AGENT_PLATFORM[agent]
  const box = size === 24 ? "size-6 rounded-[7px]" : "size-10 rounded-[10px]"
  const inner = size === 24 ? 20 : 30
  if (agent === "opencode") {
    return <img src={opencodeLogo} alt="" aria-hidden width={size} height={size} className={cn("shrink-0", box)} />
  }
  if (platform) {
    return (
      <span aria-hidden className={cn("flex shrink-0 items-center justify-center border border-jolu-border bg-jolu-bg", box)}>
        <PlatformLogo platform={platform as Parameters<typeof PlatformLogo>[0]["platform"]} size={inner} />
      </span>
    )
  }
  const glyph = (name.trim()[0] ?? "?").toUpperCase()
  return (
    <span aria-hidden style={{ background: "#615CED" }}
      className={cn("flex shrink-0 items-center justify-center font-bold text-white", box, size === 24 ? "text-[10px]" : "text-[16px]")}>
      {glyph}
    </span>
  )
}

/** 胶囊开关：列表小号（28×16）、详情头大号（34×20）、局域网共享特大（40×22）。 */
export function Switch({ on, small, large, disabled, label, onClick }: { on: boolean; small?: boolean; large?: boolean; disabled?: boolean; label: string; onClick: () => void }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} disabled={disabled} onClick={onClick}
      className={cn(
        "flex shrink-0 items-center rounded-full p-0.5 outline-none transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:opacity-45",
        small ? "h-4 w-7" : large ? "h-6 w-11" : "h-5 w-[34px]",
        on ? "justify-end bg-jolu-accent" : "justify-start bg-jolu-track",
      )}>
      <span className={cn("rounded-full bg-white shadow-sm", small ? "size-3" : large ? "size-5" : "size-4")} />
    </button>
  )
}
