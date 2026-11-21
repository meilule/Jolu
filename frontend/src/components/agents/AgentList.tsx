import { useState } from "react"
import { ChevronDown, EyeOff, Plus, RefreshCw, Search, ShieldCheck } from "lucide-react"
import { cn } from "@/lib/utils"
import { AGENT_FILTERS, agentGridRows, type AgentFilter, type AgentRow } from "@/lib/agentPage"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { JoluSwitch } from "@/components/ui/JoluSwitch"
import opencodeLogo from "@/assets/platforms/opencode.svg"
// 多色 / 位图类官方 logo（不适合内嵌单色 path，与 opencode 同走 img）
import vscodeLogo from "@/assets/agents/vscode.svg"
import continueLogo from "@/assets/agents/continue.png"
import pencilLogo from "@/assets/agents/pencil.png"
import piLogo from "@/assets/agents/pi.svg"

/**
 * Agent 页 · 列表态（Jolu.pen「统一网关 · R2 重设计」V8 画板 V8-01 / V8-02）：
 * 三列工具卡片栅格——卡上直接开关接入，底栏「详细」进详情页；
 * 接入状态筛选（全部 / 已接入 / 可接入）+ 搜索；暂不能接入固定折叠条；
 * 底部一行接入安全提示。V8 未画详情态，「详细」沿用既有详情页（V6-03 起）。
 */

export function AgentDirectory({ rows, selected, modelsCount, tierCount, busy, search, onSearch, hidden, showHidden, onShowHidden, onSelect, onToggle, onRedetect, onAddCustom }: {
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
  onRedetect: () => void
  onAddCustom: () => void
}) {
  const [filter, setFilter] = useState<AgentFilter>("all")
  const [blockedOpen, setBlockedOpen] = useState(false)
  const grid = agentGridRows(rows, filter, search, hidden, showHidden)
  return (
    // 子元素必须 shrink-0：flex-col 默认允许压缩，压缩后内容溢出被裁、滚动容器反而不触发滚动
    <div className="flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto px-8 pb-6 pt-6 [&>*]:shrink-0">
      {/* 窄窗口时按钮组折行（flex-wrap）：并排会把标题压到换行（2026-11-17 尺寸扫描） */}
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-3">
        <h1 className="whitespace-nowrap text-[24px] font-semibold text-jolu-text">工具</h1>
        <div className="flex shrink-0 items-center gap-2.5">
          <button type="button" disabled={busy} onClick={onRedetect}
            className="flex h-10 items-center gap-2 whitespace-nowrap rounded-lg border border-jolu-border bg-jolu-bg px-3.5 text-meta text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
            <RefreshCw aria-hidden className={cn("size-3.5 text-jolu-muted", busy && "animate-spin")} />重新检测
          </button>
          <button type="button" onClick={onAddCustom} disabled={busy}
            className="flex h-10 items-center gap-2 whitespace-nowrap rounded-lg bg-jolu-text px-3.5 text-meta font-semibold text-jolu-bg outline-none hover:opacity-90 focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
            <Plus aria-hidden className="size-4" />添加工具
          </button>
        </div>
      </div>

      {/* 接入状态筛选 + 搜索（画板 V8-01 筛选行）。窄窗口折行：chips 一行、搜索占满余宽 */}
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
        <div className="flex items-center gap-0.5 rounded-lg bg-jolu-side p-0.5" role="group" aria-label="接入状态筛选">
          {AGENT_FILTERS.map((item) => (
            <button key={item.id} type="button" aria-pressed={filter === item.id} onClick={() => setFilter(item.id)}
              className={cn("h-8 rounded-md px-3.5 text-meta outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
                filter === item.id ? "bg-jolu-bg font-medium text-jolu-text shadow-sm" : "text-jolu-muted hover:text-jolu-text")}>
              {item.label}
              <span className="ml-1 tabular-nums">{
                item.id === "all" ? grid.rows.length + grid.blocked.length
                  : item.id === "wired" ? rows.filter((row) => row.group === "wired").length
                    : rows.filter((row) => row.group === "available").length
              }</span>
            </button>
          ))}
        </div>
        <div className="flex min-w-0 items-center gap-2.5">
          {(grid.hiddenCount > 0 || showHidden) && (
            <button type="button" onClick={() => onShowHidden(!showHidden)} aria-pressed={showHidden}
              className="flex h-9 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-lg border border-jolu-border px-3 text-meta text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
              <EyeOff aria-hidden className="size-3.5 text-jolu-muted" />{showHidden ? "收起已隐藏" : `已隐藏 ${grid.hiddenCount}`}
            </button>
          )}
          {/* 搜索框：与 chips 同行时分到剩余宽（250 封顶），折行后独占整行 */}
          <label className="flex h-10 w-full min-w-0 max-w-[250px] flex-1 items-center gap-2.5 rounded-lg border border-jolu-border bg-jolu-bg px-3">
            <Search aria-hidden className="size-4 shrink-0 text-jolu-muted" />
            <input value={search} onChange={(event) => onSearch(event.target.value)} placeholder="搜索工具" aria-label="搜索工具"
              className="min-w-0 flex-1 bg-transparent text-meta text-jolu-text outline-none placeholder:text-jolu-muted" />
          </label>
        </div>
      </div>

      {/* 三列工具卡片栅格（画板 V8-01：卡 328×218）。按容器宽度降列：三列需要约 1000px
          内容宽（328×3 + 间距 + 页边距），不够就 2 列、再窄 1 列——否则卡内标题与状态行
          被裁（2026-11-17 尺寸扫描）。容器查询在侧栏收起 / 展开时都准确；
          @container 放外层壳（容器查询量的是最近容器祖先，不能和栅格同一元素）。 */}
      <div className="@container" key={`${filter}:${search}:${showHidden}`}>
        <div className="agent-card-grid grid gap-4 grid-cols-1 @[680px]:grid-cols-2 @[1000px]:grid-cols-3">
          {grid.rows.map((row) => (
            <ToolCard key={row.agent} row={row} selected={selected === row.agent} modelsCount={modelsCount} tierCount={tierCount} busy={busy}
              onSelect={() => onSelect(row.agent)} onToggle={() => onToggle(row)} />
          ))}
        </div>
        {grid.rows.length === 0 && (
          <p className="rounded-xl border border-dashed border-jolu-border px-4 py-10 text-center text-meta text-jolu-muted">
            {search ? "没有匹配的工具" : filter === "wired" ? "还没有已接入的工具" : "这个筛选下没有工具"}
          </p>
        )}
      </div>

      {/* 暂不能接入折叠条（画板 V8-01：48 高，右端查看原因） */}
      {grid.blocked.length > 0 && (
        <div className="flex flex-col overflow-hidden rounded-xl bg-jolu-side">
          <button type="button" aria-expanded={blockedOpen} onClick={() => setBlockedOpen((value) => !value)}
            className="flex h-12 shrink-0 items-center gap-2.5 px-5 text-left outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-jolu-accent">
            <ChevronDown aria-hidden className={cn("size-3.5 shrink-0 text-jolu-muted transition-transform", !blockedOpen && "-rotate-90")} />
            <span className="text-[13px] font-semibold text-jolu-text">暂不能接入</span>
            <span className="rounded-md bg-jolu-bg px-1.5 py-0.5 text-note font-semibold tabular-nums text-jolu-muted">{grid.blocked.length}</span>
            <span className="flex-1" />
            <span className="text-note text-jolu-muted">{blockedOpen ? "收起" : "原因"}</span>
          </button>
          {blockedOpen && (
      <div className="grid grid-cols-3 gap-3 bg-jolu-bg p-3">
              {grid.blocked.map((row) => (
                <div key={row.agent} className="flex items-center gap-3 rounded-lg border border-jolu-border px-4 py-3">
                  <AgentBadge agent={row.agent} name={row.name} size={34} />
                  <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className="truncate text-[13px] font-semibold text-jolu-text">{row.name}</span>
                    <span className="truncate text-note text-jolu-muted">{row.subtitle}</span>
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* 接入安全提示（画板 V8-01 底部一行） */}
      <div className="flex items-center gap-2 px-1 text-note text-jolu-muted">
        <ShieldCheck aria-hidden className="size-3.5 shrink-0" />
        <span>开启前预览配置改动，关闭时可还原 Jolu 管理的字段。</span>
      </div>
    </div>
  )
}

/** 工具卡（画板 V8-01）：上身 = 标识 + 名称/状态 + 开关；摘要三行；底栏 = 状态 + 「详细」。 */
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
  return (
    <div className={cn("agent-card flex min-h-[218px] flex-col overflow-hidden rounded-xl border border-jolu-border bg-jolu-bg", selected && "agent-card-selected")}>
      <div className="flex flex-1 flex-col gap-3.5 px-5 pt-4">
        {/* 身份与开关 */}
        <div className="flex items-center gap-3">
          <AgentBadge agent={row.agent} name={row.name} size={38} />
          <button type="button" onClick={onSelect}
            className="flex min-w-0 flex-1 flex-col gap-0.5 text-left outline-none focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-jolu-accent">
            <span className="truncate text-[15px] font-semibold text-jolu-text">{row.name}</span>
            {/* 只有需要处理时（漂移 / 配置文件被删等）才占第二行；CLI 不在 PATH 与「可以接入」并存，
                说明已在摘要区给出，不在标题下重复一遍 */}
            {row.tone === "danger" && <span className="truncate text-[11px] text-jolu-danger">{row.subtitle}</span>}
          </button>
          {/* 画板 V8：暂不支持的也有开关形状（track 底），只是禁用不可拨 */}
          <Switch on={row.toggle === "on"} disabled={busy || !row.toggle} label={row.toggle ? `${row.toggle === "on" ? "断开" : "接入"} ${row.name}` : `${row.name} 暂不支持接入`} onClick={onToggle} />
        </div>
        {/* 模型摘要 */}
        <button type="button" onClick={onSelect} className="agent-card-summary flex min-h-[70px] flex-1 flex-col items-start justify-center gap-1 rounded-lg bg-jolu-side px-3.5 py-2.5 text-left outline-none focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-jolu-accent">
          {wired ? (
            <>
              <span className="text-[11px] text-jolu-muted">新会话默认</span>
              <span className="max-w-full truncate text-[14px] font-semibold text-jolu-text" title={model || undefined}>{model || "工具自己上次的选择"}</span>
              <span className="max-w-full truncate text-[11px] text-jolu-muted">
                {claude ? `${tierCount} 个档位 · ${modelsCount} 个菜单模型` : row.wiring?.codex_mode === "oauth" ? "保留 ChatGPT 登录" : `${modelsCount} 个菜单模型`}
              </span>
            </>
          ) : row.supported ? (
            <>
              <span className="text-[11px] text-jolu-muted">现在</span>
              <span className="max-w-full truncate text-[14px] font-semibold text-jolu-text" title={row.subtitle}>{row.subtitle}</span>
              <span className="max-w-full truncate text-[11px] text-jolu-muted">接入后可用 {modelsCount} 个模型</span>
            </>
          ) : (
            <>
              <span className="text-[11px] text-jolu-muted">暂无专用接入</span>
              <span className="max-w-full truncate text-[14px] font-semibold text-jolu-text">按「添加工具」通用接入</span>
              <span className="max-w-full truncate text-[11px] text-jolu-muted">{row.subtitle}</span>
            </>
          )}
        </button>
      </div>
      {/* 底栏：状态 + 详细 */}
      <div className="mt-3.5 flex h-[42px] shrink-0 items-center gap-2 border-t border-jolu-table-divider px-4">
        <span aria-hidden className={cn("size-[13px] shrink-0 rounded-full", wired ? (row.tone === "danger" ? "bg-jolu-danger" : row.tone === "warn" ? "bg-[#9A5B00]" : "bg-jolu-on") : "bg-jolu-muted")} />
        <span className={cn("truncate text-[12px] font-medium", wired ? (row.tone === "danger" ? "text-jolu-danger" : row.tone === "warn" ? "text-[#9A5B00]" : "text-jolu-on") : "text-jolu-muted")}>
          {wired ? row.subtitle : row.supported ? "可接入" : "暂不支持"}
        </span>
        <span className="flex-1" />
        <button type="button" onClick={onSelect} title={row.supported ? undefined : "Jolu 还没有它的专用接入，可按「添加工具」走通用接入"}
          className="flex h-[30px] shrink-0 items-center gap-1 rounded-lg px-2 text-[12px] font-semibold text-jolu-on outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
          详细
          <svg aria-hidden viewBox="0 0 14 14" className="size-3.5" fill="none" stroke="currentColor" strokeWidth="1.5">
            <path d="M5 3.5L9 7l-4 3.5" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
      </div>
    </div>
  )
}

/**
 * 工具标识：Jolu 支持的工具一律用官方 logo（鼠鼠 2026-11-13 要求）。
 * 三条来源：单色矢量走 `PlatformLogo`（kimi / qwen / dsh(DeepSeek) 复用厂商矢量；cline /
 * copilot / goose / zed 见 PlatformLogo 的「Agent 工具标识」注释）；多色 / 位图官方资产走
 * img（OpenCode 官网 favicon、VS Code 官方品牌包、Factory 与 pen.dev 站点图标、Continue 的
 * GitHub 组织头像）；Crush 没有干净的官方图形（Charm 家只有照片底的 touch 素材），暂用首字母。
 * 自定义工具没有官方 logo，同样用首字母小方块。
 */
const PLATFORM_OF: Readonly<Record<string, string>> = {
  "claude-code": "claude",
  codex: "codex",
  gemini: "gemini",
  zcode: "zcode",
  kimi: "kimi",
  qwen: "qwen",
  cline: "cline",
  copilot: "copilot",
  // DeepSeek Harness 是 DeepSeek 家的编码 harness，用 DeepSeek 品牌图形
  dsh: "deepseek",
  qoder: "qoder",
  "qoder-cn": "qoder-cn",
  workbuddy: "workbuddy",
}

/** 有官方位图 / 多色资产的工具：这些图形自带配色或底色，不适合做单色 path，直接 img（同 opencode） */
const AGENT_IMAGES: Readonly<Record<string, string>> = {
  vscode: vscodeLogo,
  continue: continueLogo,
  pencil: pencilLogo,
  // Pi 官方 logo（pi.dev 首页的 pi-logo-mark，三色几何图形）
  pi: piLogo,
}

export function AgentBadge({ agent, name, size }: { agent: string; name: string; size: 24 | 34 | 38 | 40 }) {
  const boxes: Record<number, string> = { 24: "size-6 rounded-[7px]", 34: "size-[34px] rounded-[9px]", 38: "size-[38px] rounded-[10px]", 40: "size-10 rounded-[10px]" }
  const inners: Record<number, number> = { 24: 20, 34: 26, 38: 28, 40: 30 }
  const box = boxes[size] ?? boxes[40]
  const inner = inners[size] ?? 30
  const platform = PLATFORM_OF[agent]
  const image = AGENT_IMAGES[agent]
  if (agent === "opencode") {
    return <img src={opencodeLogo} alt="" aria-hidden width={size} height={size} className={cn("shrink-0", box)} />
  }
  if (image) {
    return <img src={image} alt="" aria-hidden width={size} height={size} className={cn("shrink-0 object-contain", box)} />
  }
  if (platform) {
    return (
      <span aria-hidden className={cn("flex shrink-0 items-center justify-center border border-jolu-border bg-jolu-side", box)}>
        <PlatformLogo platform={platform} size={inner} />
      </span>
    )
  }
  const glyph = (name.trim()[0] ?? "?").toUpperCase()
  return (
    <span aria-hidden style={{ background: "#615CED" }}
      className={cn("flex shrink-0 items-center justify-center font-bold text-white", box, size >= 34 ? "text-[14px]" : "text-[10px]")}>
      {glyph}
    </span>
  )
}

/**
 * 胶囊开关 —— 现已统一到 JoluSwitch（含液态动效）。
 *
 * 保留原有的 `on / small / large / onClick` 签名，只是内部转发，
 * 所以 Agent 列表、详情、模型路由的那些调用点一行都不用改。
 *
 * 尺寸映射：`small` → JoluSwitch 的 compact（30×18）；其余走默认（36×32 命中区 /
 * 20px 轨道）。原来的 `large`（44×24）在 JoluSwitch 里没有对应档位 ——
 * 统一成默认尺寸，宁可少一档，也不给一个只在这一处存在的尺寸。
 *
 * `onClick` 与 `onChange` 的差异：原实现是「点一下取反」，JoluSwitch 收的是
 * 「新值」。这里把 onClick 包成 onChange = () => onClick()，语义完全一致。
 */
export function Switch({ on, small, disabled, label, onClick }: { on: boolean; small?: boolean; large?: boolean; disabled?: boolean; label: string; onClick: () => void }) {
  return (
    <JoluSwitch
      liquid
      compact={small}
      on={on}
      disabled={disabled}
      label={label}
      onChange={() => onClick()}
    />
  )
}
