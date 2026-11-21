import type {
  ContextKind, ContextPartDto, ContextSnapshotSummaryDto, ContextToolResultDto,
} from "./gatewayApi.ts"
import { compactTokens } from "./gatewaySessions.ts"

/**
 * 模型路由 · 会话观察（Jolu.pen V10-03 起）的纯逻辑；测试见 `contextObservation.test.mjs`。
 *
 * 纪律（V10-13）：类别颜色与顺序固定；分类是本地估算，Skills、记忆与文件只计入有本次来源证据的内容，其余保留为未分类；
 * 未知、真实的零、不支持、采集失败分别表达；估算与实测的偏差明确提示，不凑平；正文未采集时不补写预览。
 */

export interface KindMeta {
  kind: ContextKind
  /** 图例与表格里的名字 */
  label: string
  /** 筛选标签上的名字（「工具」比「工具定义」短） */
  chip: string
  /** 色块（画板 V10-05 的区块颜色变量） */
  fill: string
  /** 色块上的文字颜色（深色块用背景色字） */
  ink: string
}

/**
 * 类别色板（2026-11-21 重定）。
 *
 * 之前借用 jolu-* 语义色，后四个（pressed/track/selected/stage）在浅色主题里
 * 都是浅灰绿，矩形图里完全分不清 —— 这是"难看"的主因。
 * 现在按色相跳跃定色，校验口径同 --chart-*：明度带、色相间隔 ≥40°、
 * 红绿色盲相邻对可分辨。fill/ink 都走 CSS 变量，深色主题自动换一套提亮的。
 *
 * 顺序即视觉权重：框架类（system/unknown）用中性灰，内容类用彩色。
 */
export const KINDS: readonly KindMeta[] = [
  { kind: "system", label: "系统提示词", chip: "系统提示词", fill: "bg-[var(--ctx-system)]", ink: "text-[var(--ctx-ink-light)]" },
  { kind: "tools", label: "工具定义", chip: "工具", fill: "bg-[var(--ctx-tools)]", ink: "text-[var(--ctx-ink-light)]" },
  { kind: "skills", label: "Skills", chip: "Skills", fill: "bg-[var(--ctx-skills)]", ink: "text-[var(--ctx-ink-dark)]" },
  { kind: "memory", label: "记忆", chip: "记忆", fill: "bg-[var(--ctx-memory)]", ink: "text-[var(--ctx-ink-light)]" },
  { kind: "files", label: "文件", chip: "文件", fill: "bg-[var(--ctx-files)]", ink: "text-[var(--ctx-ink-dark)]" },
  { kind: "results", label: "工具结果", chip: "工具结果", fill: "bg-[var(--ctx-results)]", ink: "text-[var(--ctx-ink-light)]" },
  { kind: "chat", label: "会话", chip: "会话", fill: "bg-[var(--ctx-chat)]", ink: "text-[var(--ctx-ink-dark)]" },
  { kind: "unknown", label: "未分类", chip: "未分类", fill: "bg-[var(--ctx-unknown)]", ink: "text-[var(--ctx-ink-light)]" },
]

export const kindMeta = (kind: string): KindMeta => KINDS.find((meta) => meta.kind === kind) ?? KINDS[KINDS.length - 1]

/** 八类按固定顺序补齐（没有的为 0），总量为各类之和 */
export function composition(parts: ContextPartDto[] | null): { kind: ContextKind; tokens: number; share: number | null }[] {
  const total = (parts ?? []).reduce((sum, part) => sum + part.tokens, 0)
  return KINDS.map(({ kind }) => {
    const tokens = parts?.find((part) => part.kind === kind)?.tokens ?? 0
    return { kind, tokens, share: total > 0 ? tokens / total : null }
  })
}

/* ───────────────────────── 来源证据 ───────────────────────── */

/** 每一项的来源证据（V10-11）：从解析时记下的 tag 说明它为什么被归到这一类 */
export function evidenceOf(kind: string, tag: string): string {
  if (kind === "memory") {
    return ({ user: "用户全局指令", project: "项目指令", local: "本机指令", memory: "自动记忆", reminder: "随请求注入的系统提醒" } as Record<string, string>)[tag] ?? "指令文件"
  }
  if (kind === "skills") return tag === "loaded" ? "本次加载证据 · Skill 读取结果" : "技能列表 · 可按需加载的 Skills"
  if (kind === "files") {
    if (tag === "read") return "读文件工具返回 · 来源证据已识别"
    if (tag === "shell") return "Shell 命令读取 · 来源证据已识别"
    if (tag === "image") return "图片或文档"
    return "文件"
  }
  if (kind === "system") return "Agent 自带的提示词"
  return ""
}

/** 路径的最后一段 */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean)
  return parts.length > 0 ? parts[parts.length - 1] : path
}

/* ───────────────────────── 明细表 ───────────────────────── */

/** 系统提示词里各段的可读名（解析时按开头标签命名，没有标签的叫 prompt / harness） */
const SYSTEM_NAMES: Record<string, string> = {
  prompt: "Agent 提示词",
  harness: "运行中追加的系统消息",
  billing: "计费标识",
  environment: "环境信息",
  environment_context: "环境信息",
  context: "运行上下文",
}

export type Expand = "turns" | "tools" | "calls" | "unknown" | null

export interface ChildRow {
  key: string
  name: string
  sub: string | null
  tokens: number
  share: number | null
}

export interface TableRow {
  key: string
  kind: ContextKind
  name: string
  /** 名称下面那行：来源证据、路径、聚合说明 */
  subtitle: string
  /** 悬停显示的完整名称 / 路径 */
  title: string
  /** 次数；没有意义时为 null（表格显示「—」） */
  count: number | null
  countText: string
  tokens: number
  share: number | null
  /** 能展开成什么：对话轮次、工具定义、单次工具返回、未分类的类型 */
  expand: Expand
  /** 工具结果展开时查哪个工具 */
  tool: string | null
  children: ChildRow[]
}

const share = (tokens: number, total: number) => (total > 0 ? tokens / total : null)

/**
 * 把八类组成摊成明细表（V10-03 / 05 / 09 / 11）：
 * - 对话整体一行，按轮次展开；工具定义整体一行，按工具展开；
 * - 工具结果每个工具一行，展开看单次返回；未分类整体一行，按类型展开；
 * - 系统提示词、Skills、记忆、文件每项一行，带来源证据。
 */
export function tableRows(parts: ContextPartDto[] | null): TableRow[] {
  const list = parts ?? []
  const total = list.reduce((sum, part) => sum + part.tokens, 0)
  const rows: TableRow[] = []
  for (const part of list) {
    const kind = part.kind
    if (kind === "chat") {
      const turns = part.items.reduce((sum, it) => sum + (it.name === "earlier" ? it.n ?? 0 : 1), 0)
      const numbers = part.items.filter((it) => it.name !== "earlier").map((it) => Number(it.name)).filter(Number.isFinite)
      const last = numbers.length > 0 ? Math.max(...numbers) : turns
      rows.push({
        key: "chat", kind, name: "会话内容", subtitle: turns > 0 ? `第 1–${last} 轮 · 按轮次展开` : "按轮次展开", title: "会话内容",
        count: turns, countText: `${turns} 轮`, tokens: part.tokens, share: share(part.tokens, total), expand: "turns", tool: null,
        children: part.items.map((it) => ({
          key: `turn:${it.name}`, name: it.name === "earlier" ? `更早的 ${it.n ?? 0} 轮` : `第 ${it.name} 轮`, sub: null, tokens: it.tokens, share: share(it.tokens, total),
        })),
      })
      continue
    }
    if (kind === "tools") {
      const count = part.items.reduce((sum, it) => sum + (it.tag === "more" ? it.n ?? 0 : it.n && it.n > 0 ? it.n : 1), 0)
      rows.push({
        key: "tools", kind, name: "工具定义", subtitle: `本次实际提供的 ${count} 个工具`, title: "工具定义",
        count, countText: String(count), tokens: part.tokens, share: share(part.tokens, total), expand: "tools", tool: null,
        children: part.items.map((it) => ({
          key: `tool:${it.tag}:${it.name}`,
          name: it.tag === "more" ? `其余 ${it.n ?? 0} 项` : it.name,
          sub: it.tag === "mcp" ? `MCP 服务器 · ${it.n ?? 1} 个工具` : it.tag === "namespace" ? `命名空间 · ${it.n ?? 1} 个工具` : null,
          tokens: it.tokens, share: share(it.tokens, total),
        })),
      })
      continue
    }
    if (kind === "unknown") {
      rows.push({
        key: "unknown", kind, name: "未分类内容", subtitle: "缺少来源标识 · 不推断为技能或文件", title: "未分类内容",
        count: null, countText: "—", tokens: part.tokens, share: share(part.tokens, total), expand: "unknown", tool: null,
        children: part.items.map((it) => ({ key: `unknown:${it.name}`, name: it.name, sub: it.tag === "role" ? "未识别的消息角色" : "未识别的内容类型", tokens: it.tokens, share: share(it.tokens, total) })),
      })
      continue
    }
    for (const it of part.items) {
      if (kind === "results") {
        const more = it.tag === "more"
        const count = more ? null : it.n && it.n > 0 ? it.n : 1
        rows.push({
          key: `results:${it.name}`, kind, name: more ? `其余 ${it.n ?? 0} 个工具的结果` : `${it.name} 工具结果`, title: it.name,
          subtitle: more ? "已合并显示" : `${count} 次 · 本次输入中保留的工具返回`,
          count, countText: count == null ? "—" : String(count), tokens: it.tokens, share: share(it.tokens, total),
          expand: more ? null : "calls", tool: more ? null : it.name, children: [],
        })
        continue
      }
      if (it.tag === "more") {
        rows.push({
          key: `${kind}:more`, kind, name: `其余 ${it.n ?? 0} 项`, subtitle: "已合并显示", title: "",
          count: null, countText: "—", tokens: it.tokens, share: share(it.tokens, total), expand: null, tool: null, children: [],
        })
        continue
      }
      const evidence = evidenceOf(kind, it.tag)
      const pathLike = kind === "memory" || kind === "files"
      const name = kind === "system" ? SYSTEM_NAMES[it.name] ?? it.name
        : kind === "memory" && it.name !== "reminders" ? baseName(it.name) : it.name === "reminders" ? "系统提醒" : it.name === "skills" && kind === "skills" ? "技能列表" : it.name
      rows.push({
        key: `${kind}:${it.tag}:${it.name}`, kind, name, title: it.name,
        subtitle: pathLike && it.name !== "reminders" && name !== it.name ? `${evidence} · ${it.name}` : evidence,
        count: 1, countText: "1", tokens: it.tokens, share: share(it.tokens, total), expand: null, tool: null, children: [],
      })
    }
  }
  return rows
}

export type SortMode = "tokens" | "kind"

/** 按类别与名称 / 路径 / 证据搜索，排序（占用从大到小，或按类别固定顺序） */
export function visibleRows(rows: TableRow[], kind: ContextKind | null, search: string, sort: SortMode = "tokens"): TableRow[] {
  const needle = search.trim().toLowerCase()
  const order = (k: string) => KINDS.findIndex((meta) => meta.kind === k)
  return rows
    .filter((row) => !kind || row.kind === kind)
    .filter((row) => !needle || [row.name, row.title, row.subtitle, ...row.children.map((c) => c.name)].join("\n").toLowerCase().includes(needle))
    .sort((a, b) => (sort === "kind" ? order(a.kind) - order(b.kind) : 0) || b.tokens - a.tokens || a.name.localeCompare(b.name))
}

/** 表格上方的计数：全部时「已识别 N 项」；工具结果「工具结果 · N 类」；其余「文件 · N 项」 */
export function countLabel(rows: TableRow[], kind: ContextKind | null): string {
  if (!kind) return `已识别 ${rows.length} 项`
  const meta = kindMeta(kind)
  return kind === "results" ? `${meta.label} · ${rows.length} 类` : `${meta.label} · ${rows.length} 项`
}

/* ───────────────────────── 单次工具返回（V10-05） ───────────────────────── */

export interface CallLine {
  key: string
  index: number
  target: string
  tokens: string
  summary: string
  masked: boolean
}

/** 某个工具的单次返回，新的在前（编号按在请求里出现的顺序，从 1 起）；默认只显示前 `limit` 条 */
export function callLines(calls: ContextToolResultDto[], limit: number | null): { lines: CallLine[]; total: number; tokens: number } {
  const numbered = calls.map((call, i) => ({ call, index: i + 1 })).reverse()
  const shown = limit == null ? numbered : numbered.slice(0, limit)
  return {
    total: calls.length,
    tokens: calls.reduce((sum, call) => sum + call.tokens, 0),
    lines: shown.map(({ call, index }) => ({
      key: `${index}`,
      index,
      target: call.target ?? call.tool,
      tokens: `${compactTokens(call.tokens)} Token`,
      summary: call.summary,
      masked: call.masked,
    })),
  }
}

/* ───────────────────────── 两张占用卡 ───────────────────────── */

export interface UsageCard {
  value: string
  ratio: number | null
  /** 上游实测 / 本地估算 / 未知 */
  basis: string
  note: string
}

/** 上下文占用（V10-03 左卡）：输入优先用上游实测；窗口未知时隐藏比例与剩余空间（V10-13） */
export function contextCard(s: ContextSnapshotSummaryDto): UsageCard {
  const used = s.input_tokens ?? s.estimated
  const basis = s.input_tokens != null ? "上游实测" : s.estimated != null ? "本地估算" : "未知"
  if (used == null) return { value: "尚未采集", ratio: null, basis, note: "这次请求没有可用的输入量" }
  const window = s.window_tokens
  if (window == null || window <= 0) {
    return { value: `${compactTokens(used)} / 容量未知`, ratio: null, basis, note: "模型窗口容量尚未确认，不显示占比与剩余空间" }
  }
  const remaining = Math.max(0, window - used)
  return { value: `${compactTokens(used)} / ${compactTokens(window)}`, ratio: used / window, basis, note: `剩余上下文空间 ${compactTokens(remaining)} · 窗口容量由模型元数据提供` }
}

/** 提示词缓存（V10-03 右卡）：没给写「未提供」，给了 0 就显示 0 */
export function cacheCard(s: ContextSnapshotSummaryDto): UsageCard {
  if (s.cache_read == null) return { value: "未提供", ratio: null, basis: "", note: "上游没有返回缓存读取数" }
  const input = s.input_tokens
  return {
    value: compactTokens(s.cache_read),
    ratio: input != null && input > 0 ? s.cache_read / input : null,
    basis: "",
    note: "读取量已包含在输入中，不重复相加",
  }
}

/** 估算与实测的偏差（V10-13：明确提示，不凑平）；没有实测或没有估算时为 null */
export function deviationNote(s: ContextSnapshotSummaryDto): string | null {
  if (!s.counted || s.input_tokens == null || s.estimated == null || s.input_tokens <= 0) return null
  const diff = (s.estimated - s.input_tokens) / s.input_tokens
  if (Math.abs(diff) < 0.005) return `本地估算 ${compactTokens(s.estimated)}，与上游实测一致`
  return `本地估算 ${compactTokens(s.estimated)}，比上游实测${diff > 0 ? "多" : "少"} ${(Math.abs(diff) * 100).toFixed(1)}%；组成已按实测等比例换算`
}

/** 运行摘要那一行 */
export function runSummary(s: ContextSnapshotSummaryDto): { status: string; tone: "on" | "danger" | "muted"; parts: string[] } {
  const status = s.route_result === "ok" ? "已完成" : s.route_result === "rejected" ? "被拒绝" : s.route_result ? "请求失败" : s.state === "analyzing" ? "处理中" : "结果未知"
  const tone = s.route_result === "ok" ? "on" : s.route_result ? "danger" : "muted"
  const parts = [clock(s.ts)]
  if (s.duration_ms != null) parts.push(`完整用时 ${(s.duration_ms / 1000).toFixed(1)}s`)
  return { status, tone, parts }
}

export function ioSummary(s: ContextSnapshotSummaryDto): string {
  const input = s.input_tokens != null ? compactTokens(s.input_tokens) : "未提供"
  const output = s.output_tokens != null ? compactTokens(s.output_tokens) : "未提供"
  return `输入 ${input}　/　输出 ${output} Token`
}

/** 链路：请求模型 → 路由分组（请求的是 group/ 时）→ 实际模型 / 来源账号 */
export function routeChain(s: ContextSnapshotSummaryDto, groupName: (slug: string) => string | null): { requested: string; group: string | null; actual: string } {
  const requested = s.requested_model ?? "—"
  const slug = requested.startsWith("group/") ? requested.slice("group/".length) : null
  const group = slug ? groupName(slug) ?? slug : null
  const model = s.upstream_model ?? (s.requested_model ? s.requested_model.replace(/^[^/]+\//, "") : null)
  const account = s.connection_name ?? s.source
  return { requested, group, actual: [model, account].filter(Boolean).join(" · ") || "—" }
}

/* ───────────────────────── 跟随最新（V10-07 / V10-13） ───────────────────────── */

export interface FollowState {
  following: boolean
  /** 正在看的快照；跟随时就是最新一条 */
  selectedId: number | null
  /** 暂停时最新的那一条：之后新增的请求数从它往后算 */
  pausedAt: number | null
}

export const initialFollow: FollowState = { following: true, selectedId: null, pausedAt: null }

/** 历史列表更新（新的在前）：跟随时切到最新；暂停时不动 */
export function onHistory(state: FollowState, history: { id: number }[]): FollowState {
  const latest = history[0]?.id ?? null
  if (state.following) return { ...state, selectedId: latest }
  if (state.selectedId != null && !history.some((h) => h.id === state.selectedId)) {
    // 正在看的那条已被清理（每个会话只留 20 条）：仍保持暂停，但退回最新，免得指向不存在的快照
    return { ...state, selectedId: latest }
  }
  return state
}

/** 选旧请求、展开条目、主动筛选：暂停跟随 */
export function pause(state: FollowState, history: { id: number }[], select?: number): FollowState {
  const latest = history[0]?.id ?? null
  return {
    following: false,
    selectedId: select ?? state.selectedId ?? latest,
    pausedAt: state.following ? latest : state.pausedAt ?? latest,
  }
}

export function resume(history: { id: number }[]): FollowState {
  return { following: true, selectedId: history[0]?.id ?? null, pausedAt: null }
}

/** 暂停以来新增的请求数 */
export function newSince(state: FollowState, history: { id: number }[]): number {
  if (state.following || state.pausedAt == null) return 0
  return history.filter((h) => h.id > (state.pausedAt as number)).length
}

/** 历史下拉的一行：「最新 · 已完成 · 输入 648K」「请求失败 · 用量未提供」 */
export function historyLabel(s: ContextSnapshotSummaryDto, latest: boolean, selected: boolean): string {
  const status = s.route_result === "ok" ? "已完成" : s.route_result ? "请求失败" : s.state === "analyzing" ? "处理中" : "结果未知"
  const usage = s.input_tokens != null ? `输入 ${compactTokens(s.input_tokens)}` : s.estimated != null ? `估算 ${compactTokens(s.estimated)}` : "用量未提供"
  return [latest ? "最新" : null, selected ? "已选" : null, status, usage].filter(Boolean).join(" · ")
}
/* ───────────────────────── 矩形图（V10-09，2026-11-21 重做） ───────────────────────── */

export interface Tile {
  kind: ContextKind | null
  tokens: number
  /** 0–1 比例，相对整个画布（已用区 + 剩余区共同构成 1） */
  x: number
  y: number
  w: number
  h: number
  /** 剩余空间块：只画底色与「剩余 N」，不进图例、不参与筛选 */
  free?: boolean
  /** 面积被抬到最小可点值（真实占比更小），文案需标注 */
  floored?: boolean
}

/* 参考 magpie internal/gui/assets/context.js 的 tilesOf，三处关键设计：
   1) 画布分成「已用区 + 剩余区」，剩余区是一块明显的浅色 —— 一眼看出还有多少余量。
      之前只画已用内容，20% 满的窗口看起来也是满的。
   2) 已用区长度的曲线是 share^0.35 而不是线性：窗口只有 8% 满时，
      已用区仍占画布 41%，里面的块才读得出来；窗口满了仍然是满的。
   3) 每块强制最小可点面积（TILE_FLOOR）—— 否则小类被挤成细条，既读不出也点不到。 */
/** 最小可点面积（真实 px²）。magpie 用 700 —— 一个约 26×26 的方块，手指/鼠标够得着 */
const TILE_FLOOR = 700
/** 已用区长度的曲线指数：0.35（magpie 实测值） */
const USED_CURVE = 0.35

type Rect = { x: number; y: number; w: number; h: number }

/** squarified treemap（Bruls），在给定矩形里按 v 分配；坐标是 0–1 比例 */
function squarify(vals: number[], rect: Rect): Rect[] {
  const out: Rect[] = []
  const total = vals.reduce((s, v) => s + Math.max(v, 0), 0)
  if (!total || rect.w <= 0 || rect.h <= 0) return out
  const k = (rect.w * rect.h) / total
  let { x, y, w, h } = rect
  const worst = (row: number[], side: number) => {
    let sum = 0, hi = 0, lo = Infinity
    for (const v of row) { const s = v * k; sum += s; hi = Math.max(hi, s); lo = Math.min(lo, s) }
    return Math.max((side * side * hi) / (sum * sum), (sum * sum) / (side * side * lo))
  }
  const lay = (row: number[]) => {
    const sum = row.reduce((acc, v) => acc + v * k, 0)
    if (w >= h) {
      const cw = sum / h
      let cy = y
      for (let i = 0; i < row.length; i++) {
        // 最后一块吃满剩余高度，而不是按自己的值再算一遍：逐块累加 `cy += ch` 会
        // 让浮点误差攒到末块上，最后一块的 y + h 可能略大于 1，渲染时就超出画布
        // （现场：绿色「会话」块从底边溢出来）。
        const last = i === row.length - 1
        const ch = last ? y + h - cy : (row[i] * k) / cw
        out.push({ x, y: cy, w: cw, h: Math.max(0, ch) })
        cy += ch
      }
      x += cw; w -= cw
    } else {
      const rh = sum / w
      let cx = x
      for (let i = 0; i < row.length; i++) {
        // 同理：末块宽度吃满剩余，避免累加误差让它越过右边界
        const last = i === row.length - 1
        const cw = last ? x + w - cx : (row[i] * k) / rh
        out.push({ x: cx, y, w: Math.max(0, cw), h: rh })
        cx += cw
      }
      y += rh; h -= rh
    }
  }
  let row: number[] = []
  for (const v of vals) {
    if (v <= 0) continue
    const side = Math.min(w, h)
    if (!row.length || worst([...row, v], side) <= worst(row, side)) { row.push(v); continue }
    lay(row); row = [v]
  }
  if (row.length) lay(row)
  return out
}

/**
 * 上下文组成矩形图。坐标是 0–1 比例。
 *
 * @param window 上下文窗口总量。给了就画「已用 + 剩余」两块；不给就退化成纯占比图
 *               （与旧行为一致，用于窗口容量未知的模型）。
 */
export function treemap(
  items: { kind: ContextKind; tokens: number }[],
  window?: number | null,
  canvasW = 900,
  canvasH = 200,
): Tile[] {
  const data = items.filter((it) => it.tokens > 0).sort((a, b) => b.tokens - a.tokens)
  const used = data.reduce((sum, it) => sum + it.tokens, 0)
  if (used <= 0) return []
  const hasRoom = window != null && window > used

  /* 已用区长度：share^0.35。8% 满 → 41% 画布；满 → 100%。
     没有窗口数据时整块画布都给已用内容。 */
  const usedLen = hasRoom ? Math.max(Math.pow(used / window!, USED_CURVE), 0.18) : 1
  const usedRect: Rect = { x: 0, y: 0, w: usedLen, h: 1 }

  /* 最小可点面积：把过小的块的「虚拟面积」抬到刚好超过 TILE_FLOOR。
     面积要按**已用区的实际面积**算，不是整个画布：窗口只用到 3% 时 `usedLen` 被
     下限抬到 0.18，已用区只有 900×200×0.18 = 32,400px²。按整块画布算会把下限
     低估 5 倍，小类仍然被压成 2px 高的细条（现场：绿色的「会话」块薄到渲染成负高度、
     看起来从底边溢出来）。 */
  const area = Math.max(1, canvasW * canvasH * usedLen)
  const sum = data.reduce((s, it) => s + it.tokens, 0)
  const lo = (sum * TILE_FLOOR) / area
  const values = data.map((it) => Math.max(it.tokens, lo))

  const rects = squarify(values, usedRect)
  const tiles: Tile[] = rects.map((r, i) => ({
    kind: data[i].kind,
    // 展示用真实 token；面积可能因为 TILE_FLOOR 被抬高过
    tokens: data[i].tokens,
    x: r.x, y: r.y, w: r.w, h: r.h,
    floored: data[i].tokens < lo ? true : undefined,
  }))

  if (hasRoom) {
    tiles.push({
      kind: null,
      tokens: (window as number) - used,
      x: usedLen, y: 0, w: 1 - usedLen, h: 1,
      free: true,
    })
  }
  return tiles
}

function clock(ts: number): string {
  return new Date(ts).toLocaleTimeString("zh-CN", { hour12: false })
}
