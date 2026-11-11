import { useEffect, useRef, useState } from "react"
import { TierPanel } from "./TierPanel"
import type { TierDraft } from "@/lib/tierModels"
import * as RadixDialog from "@radix-ui/react-dialog"
import { TriangleAlert, X } from "lucide-react"
import { cn } from "@/lib/utils"
import type { CatalogModelDto, GatewayGroupDto, GroupKind, WiringStatusDto } from "@/lib/gatewayApi"
import { agentLabel } from "@/lib/gatewayPage"
import { gatewayApi } from "@/lib/gatewayApi"
import { isTauri } from "@/lib/api"
import { useToast } from "@/components/ui/Toast"
import { ConfirmDialog as ActionConfirmDialog } from "@/components/ui/ConfirmDialog"
import { JoluButton } from "@/components/ui/JoluButton"
import type { Connection } from "@/types"

/**
 * 统一网关的弹窗与管理行（沿用模块 J 旧画板的弹窗与 E1–E10 状态）。
 * 来源进出模型池、改名与菜单 id、分组管理在「管理来源」弹窗（SourceManageDialog.tsx）。
 */

export function Modal({ title, icon, tone, children, footer, onClose, busy = false, width = 540 }: { title: string; icon?: React.ReactNode; tone?: "danger"; children: React.ReactNode; footer: React.ReactNode; onClose: () => void; busy?: boolean; width?: number }) {
  const trigger = useRef(document.activeElement)
  return <RadixDialog.Root open onOpenChange={value => { if (!value && !busy) onClose() }}>
    <RadixDialog.Portal>
      <RadixDialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
      <RadixDialog.Content aria-describedby={undefined} aria-busy={busy} style={{ width }}
        onPointerDownOutside={event => event.preventDefault()}
        onEscapeKeyDown={event => { if (busy) event.preventDefault() }}
        onCloseAutoFocus={event => { event.preventDefault(); if (trigger.current instanceof HTMLElement && trigger.current.isConnected) trigger.current.focus() }}
        className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 overflow-y-auto rounded-2xl border border-jolu-border bg-jolu-cc-surface p-6 shadow-dialog outline-none">
        <div className="flex items-center gap-2">
          {icon}
          <RadixDialog.Title className={cn("min-w-0 flex-1 text-base font-semibold", tone === "danger" ? "text-jolu-danger" : "text-jolu-text")}>{title}</RadixDialog.Title>
          <RadixDialog.Close aria-label="关闭" disabled={busy} className="grid size-8 shrink-0 place-items-center rounded-lg text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40"><X aria-hidden className="size-4" /></RadixDialog.Close>
        </div>
        {children}
        <div className="flex flex-wrap justify-end gap-2">{footer}</div>
      </RadixDialog.Content>
    </RadixDialog.Portal>
  </RadixDialog.Root>
}

export function Button({ label, primary, danger, disabled, onClick }: { label: string; primary?: boolean; danger?: boolean; disabled?: boolean; onClick: () => void }) {
  return <JoluButton variant={primary ? (danger ? "danger" : "primary") : "outline"} disabled={disabled} onClick={onClick}>{label}</JoluButton>
}

export function Warning({ children, tone = "danger" }: { children: React.ReactNode; tone?: "danger" | "muted" }) {
  return <div className={cn("flex items-start gap-2 rounded-lg px-3 py-2.5 text-[10px]", tone === "danger" ? "bg-jolu-danger-soft text-jolu-danger" : "bg-jolu-side text-jolu-muted")}><TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" /><span>{children}</span></div>
}

export function MessageDialog({ title, tone, message, action, onClose }: { title: string; tone?: "danger"; message: string; action?: { label: string; onClick: () => void }; onClose: () => void }) {
  return (
    <Modal title={title} tone={tone} icon={<TriangleAlert className={cn("h-4 w-4", tone === "danger" ? "text-jolu-danger" : "text-jolu-muted")} />} onClose={onClose} footer={<><Button label="关闭" onClick={onClose} />{action && <Button label={action.label} primary onClick={action.onClick} />}</>}>
      <Warning>{message}</Warning>
    </Modal>
  )
}

export function RenameDialog({ connection, busy, onClose, onSave }: { connection: Connection; busy: boolean; onClose: () => void; onSave: (name: string, slug: string) => void }) {
  const [name, setName] = useState(connection.baseName)
  const [slug, setSlug] = useState(connection.slug ?? "")
  const reserved = ["route", "auto", "jolu", "group", "default"].includes(slug.trim())
  const invalid = !!slug && (!/^[a-z0-9-]+$/.test(slug) || slug.startsWith("-") || reserved)
  return (
    <Modal busy={busy} title="重命名来源" onClose={onClose} footer={<><Button label="取消" disabled={busy} onClick={onClose} /><Button label="保存" primary disabled={busy || invalid || !name.trim()} onClick={() => onSave(name.trim(), slug.trim())} /></>}>
      <div className="flex flex-col gap-1">
        <span className="text-[9.5px] text-jolu-muted">显示名称</span>
        <input value={name} onChange={(event) => setName(event.target.value)} className="jolu-control h-9 rounded-lg border border-jolu-border bg-jolu-side px-3 text-[11px] text-jolu-text outline-none focus:border-jolu-accent" />
        <span className="text-[8.5px] text-jolu-muted">只改显示名；已接入的工具不受影响。</span>
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-[9.5px] text-jolu-muted">菜单 id</span>
        <input value={slug} onChange={(event) => setSlug(event.target.value)} className={cn("h-9 rounded-lg border bg-jolu-side px-3 text-[11px] text-jolu-text outline-none", invalid ? "border-jolu-danger" : "border-jolu-border focus:border-jolu-accent")} />
        {invalid ? <span className="text-[8.5px] text-jolu-danger">{reserved ? `「${slug}」是保留字，建议改用 ${slug}-src` : "只能使用小写字母、数字和连字符，不能以连字符开头。"}</span> : <span className="text-[8.5px] text-jolu-muted">Agent 配置里存的是这个 id；改名不会改它，改 id 后旧 id 会返回 404 并说明新名字。</span>}
      </div>
    </Modal>
  )
}

/**
 * Agent 接线。
 *
 * Claude Code 的主模型（magpie writeTiers 语义）：接线写 settings.json 顶层 `model`、
 * 全部档位 env（opus/sonnet/haiku/fable/small-fast 默认跟随主模型），并移除
 * `ANTHROPIC_MODEL`——它压过 /model 的选择，写了切换只在本会话生效。
 * 断开时按快照还原用户原来的全部模型配置。不选模型也能接（只写地址和令牌）。
 */
/**
 * 模型元数据（models.dev：上下文窗口、显示名、推理档位）的现状与手动刷新（2.8，待确认：无画板）。
 * 平时 7 天自动刷新一次；从没拉到过时用内置快照。
 */
function ModelsMetaRefresh() {
  const [status, setStatus] = useState<[number, number] | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!isTauri) return
    gatewayApi.modelsDevStatus().then(setStatus).catch(() => setStatus(null))
  }, [])
  if (!isTauri) return null
  const [fetched, count] = status ?? [0, 0]
  const text = !status ? "读取中…" : fetched === 0 ? `内置快照（${count} 个模型）` : `${new Date(fetched).toLocaleString("zh-CN", { hour12: false })} 拉取（${count} 个模型）`
  return (
    <span className="flex items-center gap-1.5 text-[8.5px] text-jolu-muted">
      <span className="min-w-0 truncate">模型元数据：{error ?? text}</span>
      <button type="button" disabled={busy} className="shrink-0 text-jolu-on outline-none hover:underline disabled:opacity-50"
        onClick={async () => {
          setBusy(true)
          setError(null)
          try {
            const [total, at] = await gatewayApi.modelsDevRefresh()
            setStatus([at, total])
          } catch (reason) {
            setError(`刷新失败：${String(reason)}`)
          } finally {
            setBusy(false)
          }
        }}>
        {busy ? "刷新中…" : "立即刷新"}
      </button>
    </span>
  )
}

export function WiringDialog({ agents, models, modelChoice, onModelChoice, subagentChoice, onSubagentChoice, tierDraft, onTierDraft, onApplyTiers, profiles, onProfileApply, onProfileSave, onProfileDelete, busy, onClose, onWire, onUnwire, onAccept }: {
  agents: WiringStatusDto[]
  /** 可选的 `<来源>/<模型>` 列表（来自模型目录，只含已接入来源；context 为 models.dev 上下文窗口） */
  models: { id: string; context?: number }[]
  modelChoice: string
  onModelChoice: (value: string) => void
  /** Claude Code 的 subagent 独立模型；空串 = 跟随主模型 */
  subagentChoice: string
  onSubagentChoice: (value: string) => void
  /** Claude Code 档位自选（批 4）草稿与应用 */
  tierDraft: TierDraft
  onTierDraft: (next: TierDraft) => void
  onApplyTiers: () => void
  /** Profile（配置一键切换）：保存当前全部接线的主模型，应用时逐个重放 */
  profiles: import("@/lib/gatewayApi").ProfileDto[]
  onProfileApply: (name: string) => void
  onProfileSave: (name: string) => void
  onProfileDelete: (name: string) => void
  busy: boolean
  onClose: () => void
  onWire: (agent: string) => void
  onUnwire: (agent: string) => void
  onAccept: (agent: string) => void
}) {
  const [profileName, setProfileName] = useState("")
  const [profileOpen, setProfileOpen] = useState(false)
  const claude = agents.find((entry) => entry.agent === "claude-code")
  return (
    <Modal busy={busy} title="Agent 接线" onClose={onClose} footer={<Button label="关闭" disabled={busy} onClick={onClose} />}>
      <p className="text-[9.5px] text-jolu-muted">状态按配置文件里实际写入的值判断；重新应用不会重新拍快照。</p>
      {claude && (
        <div className="flex flex-col gap-1 rounded-lg border border-jolu-border bg-jolu-side px-2.5 py-2">
          <span className="text-[9.5px] text-jolu-text">Claude Code 主模型</span>
          {models.length === 0 ? (
            <span className="text-[8.5px] text-jolu-warn">还没有可用模型：先在上面接入来源，再点来源区的「拉取」拿清单。</span>
          ) : (
            <select
              value={modelChoice}
              disabled={busy}
              onChange={(event) => onModelChoice(event.target.value)}
              className="h-8 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[11px] text-jolu-text outline-none focus:border-jolu-accent"
            >
              {models.map((model) => (
                <option key={model.id} value={model.id}>
                  {model.context ? `${model.id} · ${model.context >= 1_000_000 ? `${(model.context / 1_000_000).toFixed(1).replace(/\.0$/, "")}M` : `${Math.round(model.context / 1000)}K`}` : model.id}
                </option>
              ))}
            </select>
          )}
          <span className="text-[8.5px] text-jolu-muted">
            {modelChoice
              ? "写入 settings.json 顶层 model 与全部档位（opus/sonnet/haiku/fable/small-fast 跟随主模型）；ANTHROPIC_MODEL 会被移除，/model 切换不再被覆盖。断开时还原你原来的全部模型配置。"
              : "不选则只接地址和令牌，你的模型配置原样保留；Claude Code 会发自带模型名，需要「默认来源」兜底才能路由。"}
          </span>
          {/* subagent 独立模型：留空跟随主模型（Claude Code 缺省），这与 magpie 的 Quiet 字段同语义 */}
          <span className="mt-1 text-[9.5px] text-jolu-text">Subagent 模型</span>
          <select
            value={subagentChoice}
            disabled={busy}
            onChange={(event) => onSubagentChoice(event.target.value)}
            className="h-8 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[11px] text-jolu-text outline-none focus:border-jolu-accent"
          >
            <option value="">跟随主模型</option>
            {models.map((model) => <option key={model.id} value={model.id}>{model.id}</option>)}
          </select>
          <span className="text-[8.5px] text-jolu-muted">
            Claude Code 的子代理任务（Task 工具）用它；留空即跟随主模型，也是 Claude Code 的缺省行为。
          </span>
          <ModelsMetaRefresh />
          {models.length > 0 && (
            <TierPanel models={models} mainModel={modelChoice} subagent={subagentChoice} draft={tierDraft} onDraft={onTierDraft}
              busy={busy} wired={claude.wired} onApply={onApplyTiers} />
          )}
        </div>
      )}
      <div className="flex flex-col gap-1.5">
        {agents.map((entry) => (
          <div key={entry.agent} className="flex items-center gap-2 rounded-lg border border-jolu-border bg-jolu-bg px-2.5 py-2">
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="text-[10.5px] text-jolu-text">{agentLabel(entry.agent)}</span>
              <span className={cn("truncate text-[8.5px]", entry.drift === "unreachable" ? "text-jolu-danger" : entry.wired ? "text-jolu-on" : entry.drift ? "text-jolu-danger" : "text-jolu-muted")}>
                {entry.drift === "unreachable" ? "已接入，但网关没在运行：请求会失败" : entry.wired ? "已接入" : entry.drift === "foreign" ? "已被另一个 Jolu 接管" : entry.drift === "removed" ? "配置里已移除" : entry.drift === "changed" ? "配置被改写" : entry.drift === "pending" ? "上次接线未完成" : entry.reason ?? "未接入"}
              </span>
            </div>
            {entry.wired ? <Button label="断开" disabled={busy} onClick={() => onUnwire(entry.agent)} /> : entry.drift ? (<><Button label="重新应用" primary disabled={busy} onClick={() => onWire(entry.agent)} /><Button label="接受现状" disabled={busy} onClick={() => onAccept(entry.agent)} /></>) : (<Button label="接线" primary disabled={busy || (!entry.config_path && !!entry.reason)} onClick={() => onWire(entry.agent)} />)}
          </div>
        ))}
      </div>
      <span className="text-[8.5px] text-jolu-muted">同一工具的第二份安装（WSL、便携版）暂不支持，只在主安装上接线。</span>
      {/* Profile：把全部接线的主模型存成一个快照，一次切回（待画板确认前的最小可用版） */}
      <div className="flex flex-col gap-1 rounded-lg border border-jolu-border bg-jolu-side px-2.5 py-2">
        <button type="button" onClick={() => setProfileOpen((open) => !open)}
          className="flex items-center justify-between text-[9.5px] text-jolu-text outline-none">
          <span>Profiles · 配置一键切换（待确认）</span>
          <span className="text-jolu-muted">{profileOpen ? "收起 ▲" : `展开 ▼（已有 ${profiles.length} 个）`}</span>
        </button>
        {profileOpen && (
          <div className="flex flex-col gap-1">
            <div className="flex items-center gap-1.5">
              <input value={profileName} onChange={(event) => setProfileName(event.target.value)} placeholder="Profile 名称（保存当前接线）"
                className="h-8 min-w-0 flex-1 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[11px] text-jolu-text outline-none focus:border-jolu-accent" />
              <Button label="保存当前" disabled={busy || profileName.trim() === ""} onClick={() => { onProfileSave(profileName.trim()); setProfileName("") }} />
            </div>
            {profiles.length === 0 ? (
              <span className="text-[8.5px] text-jolu-muted">还没有 Profile：接好各工具的模型后，保存成一套，之后一键切回。</span>
            ) : (
              profiles.map((profile) => (
                <div key={profile.name} className="flex items-center gap-2 rounded-lg bg-jolu-bg px-2 py-1.5">
                  <span className="min-w-0 flex-1 truncate text-[10.5px] text-jolu-text">{profile.name}</span>
                  <span className="shrink-0 text-[8.5px] text-jolu-muted" title={profile.extras?.favorites?.length ? `常用：${profile.extras.favorites.join("、")}` : undefined}>
                    {Object.keys(profile.agents).length} 个工具
                    {profile.extras?.agents && Object.keys(profile.extras.agents).length > 0 ? " · 含档位" : ""}
                    {profile.extras?.default_connection ? " · 含默认来源" : ""}
                  </span>
                  <Button label="应用" primary disabled={busy} onClick={() => onProfileApply(profile.name)} />
                  <Button label="删除" disabled={busy} onClick={() => onProfileDelete(profile.name)} />
                </div>
              ))
            )}
            <span className="text-[8.5px] text-jolu-muted">应用只影响「当前已接线」的工具；配置被外部改写的会跳过并说明。</span>
          </div>
        )}
      </div>
    </Modal>
  )
}

export function CodexDialog({ mode, returnFocusTo, onClose, onConfirm }: { mode: "oauth" | "provider"; busy: boolean; returnFocusTo?: HTMLElement | null; onClose: () => void; onConfirm: () => Promise<boolean> }) {
  const oauth = mode === "oauth"
  return (
    <ActionConfirmDialog returnFocusTo={returnFocusTo} title="接入 Codex" description="确认下方接线方式后更新 Codex 本机配置。" confirmLabel="接入" onClose={onClose} onConfirm={onConfirm}>
      <p className="text-[10px] text-jolu-muted">Jolu 会把 Codex 的请求转到本机模型入口；只写入 config.toml。</p>
      <div className={cn("flex flex-col gap-1.5 rounded-lg border p-3", oauth ? "border-jolu-on bg-jolu-status-success-bg" : "border-jolu-border bg-jolu-side")}>
        <span className="text-[11px] font-semibold text-jolu-text">{oauth ? "保留 ChatGPT 登录" : "使用网关 Key"}</span>
        <span className="text-[9px] text-jolu-muted">{oauth ? "检测到 ChatGPT 登录：只改请求地址（openai_base_url），不写 Key、不动 auth.json。" : "没有检测到 ChatGPT 登录：写自定义 provider 和 jolu-codex 令牌，不改其他 provider。"}</span>
      </div>
      {!oauth && <Warning tone="muted">如果以后登录 ChatGPT，重新接线即可切回保留登录的接法。</Warning>}
      <Warning tone="muted">Codex 发 Responses 请求；只支持 Chat Completions 的上游会在请求时返回 404。</Warning>
    </ActionConfirmDialog>
  )
}

export function GeminiDialog({ returnFocusTo, onClose, onConfirm }: { busy: boolean; returnFocusTo?: HTMLElement | null; onClose: () => void; onConfirm: () => Promise<boolean> }) {
  return (
    <ActionConfirmDialog returnFocusTo={returnFocusTo} title="接入 Gemini CLI" description="接线期间 Gemini 将使用 Jolu 网关 Key，断开时恢复原登录方式。" confirmLabel="确认接入" onClose={onClose} onConfirm={onConfirm}>
      <Warning>接线期间 Gemini 会改用 Jolu 网关 Key，不再使用 Google 账号登录。OAuth 凭据文件不会被删除。</Warning>
      <div className="flex flex-col rounded-lg bg-jolu-side px-3">
        <Line label="当前登录方式" value="Google 账号 · oauth-personal" />
        <Line label="接线期间" value="网关 Key · gemini-api-key" danger />
        <Line label="断开时" value="恢复原登录方式与原 Key" />
      </div>
      <span className="text-[8.5px] text-jolu-muted">原值会存进接线记录；如果断开时还原失败，会在「Agent 接线」里提示并提供重试。</span>
    </ActionConfirmDialog>
  )
}

export function CatalogDialog({ models, onClose }: { models: CatalogModelDto[]; onClose: () => void }) {
  const toast = useToast()
  return (
    <Modal title="模型池里的模型" onClose={onClose} footer={<Button label="关闭" onClick={onClose} />}>
      <p className="text-[9.5px] text-jolu-muted">复制「来源/模型」填入调用工具。订阅模型菜单不代表已完成真实调用验收。</p>
      <div className="flex max-h-[50vh] flex-col gap-1 overflow-y-auto">
        {models.map((model) => (
          <div key={model.id} className="flex items-center gap-2 rounded-lg bg-jolu-side px-2.5 py-1.5">
            <span className="truncate text-[10px] text-jolu-text">{model.id}</span>
            <button type="button" className="shrink-0 text-[9px] text-jolu-on" onClick={() => { void navigator.clipboard.writeText(model.id).then(() => toast.success("模型名已复制"), () => toast.danger("复制失败，请重试")) }}>复制模型名</button>
            <div className="flex-1" />
            <span className={cn("text-[8.5px]", model.status === "connected" ? "text-jolu-muted" : "text-jolu-danger")}>{model.status === "connected" ? model.source : "未连接"}</span>
          </div>
        ))}
      </div>
    </Modal>
  )
}

export function Line({ label, value, danger }: { label: string; value: string; danger?: boolean }) {
  return (
    <div className="flex h-8 items-center gap-2 border-b border-jolu-table-divider text-[9.5px] last:border-0">
      <span className="text-jolu-muted">{label}</span>
      <div className="flex-1" />
      <span className={cn("font-semibold", danger ? "text-jolu-danger" : "text-jolu-text")}>{value}</span>
    </div>
  )
}

/** 分组按置顶优先、再按保存的顺序排列（只影响排列，不改路由、接线和 Agent 菜单）。 */
export function orderGroups(groups: GatewayGroupDto[], pinned: string[], kind: GroupKind): GatewayGroupDto[] {
  return [...groups].sort((a, b) => {
    const aPin = pinned.includes(`${kind}:${a.id}`) ? 0 : 1
    const bPin = pinned.includes(`${kind}:${b.id}`) ? 0 : 1
    return aPin - bPin || a.sort - b.sort
  })
}
