import { useEffect, useState } from "react"
import { Copy, Eye, GitFork, Globe, KeyRound, Laptop, ListFilter, Plus } from "lucide-react"
import { cn } from "@/lib/utils"
import { gatewayApi, type ClientKeySummaryDto, type IssuedClientKeyDto } from "@/lib/gatewayApi"
import { useConnections } from "@/hooks/useConnections"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { Switch } from "@/components/agents/AgentList"
import {
  BUDGET_FIELDS, budgetForm, budgetFromForm, concurrencyOptions, expiresAtFrom, EXPIRY_CHOICES,
  isExhausted, isExpired, KEY_ONCE_NOTICE, lastUsedText, limitText, parseModelList, TOKENS_LIMIT_NOTICE,
  type BudgetForm,
} from "./sharedAccess"

const KEY_COLUMNS = "grid grid-cols-[minmax(0,1.5fr)_minmax(0,1.1fr)_150px_150px_110px_56px] gap-3"

/**
 * 模型路由 · 对外访问（Jolu.pen「统一网关 · R2 重设计」V6 画板 RMna2）：
 * 局域网共享大卡、「连接地址与示例 / 客户端 Key」两个页签、客户端 Key 表格与签发。
 * 有效期 / 模型白名单 / 只读 / 每日请求与 Token 上限都是已有后端能力；「今日已用满」
 * 的判定沿用 `sharedAccess.isExhausted`。
 */
export function AccessPanel({ port, shared, busy, onSharedChange, run, models }: {
  port: number
  shared: boolean
  busy: boolean
  onSharedChange: (on: boolean) => void
  run: (work: () => Promise<unknown>, successTitle?: string) => Promise<boolean>
  models: string[]
}) {
  const toast = useToast()
  const [chip, setChip] = useState<"keys" | "address">("keys")
  const [rows, setRows] = useState<ClientKeySummaryDto[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [issuing, setIssuing] = useState(false)
  const [issued, setIssued] = useState<IssuedClientKeyDto | null>(null)
  const [copied, setCopied] = useState(false)
  const emptyForm = { name: "", requests: "", tokens: "", expiryDays: null as number | null, models: "", readOnly: false }
  const [form, setForm] = useState(emptyForm)
  const [editing, setEditing] = useState<{ name: string; form: BudgetForm; accounts: string[]; concurrency: number | null; error: string | null } | null>(null)
  const [revoking, setRevoking] = useState<string | null>(null)

  const load = async () => {
    setError(null)
    try {
      setRows(await gatewayApi.clientKeys())
    } catch (reason) {
      setError(String(reason))
    }
  }
  useEffect(() => { void load() }, [])

  const parseLimit = (raw: string): number | null => {
    const value = raw.trim()
    if (value === "") return null
    const parsed = Number(value)
    return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : null
  }

  const issue = async () => {
    const name = form.name.trim()
    if (name === "") {
      toast.danger("请填写 Key 名称", "名称用于区分不同设备，例如「书房台式机」")
      return
    }
    await run(async () => {
      const created = await gatewayApi.issueClientKey(name, parseLimit(form.requests), parseLimit(form.tokens), {
        expiresMs: expiresAtFrom(form.expiryDays, Date.now()),
        models: parseModelList(form.models),
        readOnly: form.readOnly,
      })
      setIssued(created)
      setForm(emptyForm)
      setIssuing(false)
      await load()
      return created
    }, "已签发客户端 Key")
  }

  const copy = (text: string, note?: string) => {
    void navigator.clipboard.writeText(text).then(() => toast.success("已复制", note)).catch(() => toast.danger("复制失败", "请手动选中复制"))
  }
  const copyKey = (key: string) => {
    void navigator.clipboard.writeText(key).then(() => setCopied(true)).catch(() => toast.danger("复制失败", "请手动选中 Key 复制"))
    window.setTimeout(() => setCopied(false), 2000)
  }

  return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 px-8 pb-8 pt-2">
      <div className="flex flex-col gap-1.5">
        <h2 className="text-[20px] font-semibold text-jolu-text">让其他设备使用你的模型</h2>
        <p className="text-meta text-jolu-muted">独立管理每个客户端的访问权限和使用上限。</p>
      </div>

      <section className={cn(
        "flex items-center gap-3.5 rounded-xl border px-5 py-4",
        shared ? "border-jolu-selected bg-jolu-status-success-bg" : "border-jolu-border",
      )}>
        <span className={cn("flex size-10 shrink-0 items-center justify-center rounded-[10px]", shared ? "bg-jolu-selected" : "bg-jolu-side")}>
          <Globe aria-hidden className={cn("size-5", shared ? "text-jolu-on" : "text-jolu-muted")} />
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="text-[15px] font-semibold text-jolu-text">局域网共享</span>
          <span className="text-meta text-jolu-muted">开关变更在下次启动中转时生效</span>
        </span>
        {shared && <span className="shrink-0 text-meta font-medium text-jolu-on">已开启</span>}
        <Switch on={shared} large disabled={busy} label={shared ? "关闭局域网共享" : "开启局域网共享"} onClick={() => onSharedChange(!shared)} />
      </section>

      <div className="flex items-center gap-2.5">
        <Chip active={chip === "address"} onClick={() => setChip("address")}>连接地址与示例</Chip>
        <Chip active={chip === "keys"} onClick={() => setChip("keys")}>客户端 Key</Chip>
      </div>

      {chip === "keys" ? (
        <section className="flex flex-col gap-3">
          <div className="flex items-center gap-2.5">
            <span className="text-[15px] font-semibold text-jolu-text">客户端 Key</span>
            {rows !== null && <span className="text-meta tabular-nums text-jolu-muted">{rows.length}</span>}
            <span className="flex-1" />
            <button type="button" disabled={busy} onClick={() => setIssuing((value) => !value)}
              className="flex h-10 shrink-0 items-center gap-1.5 rounded-lg bg-jolu-text px-4 text-meta font-semibold text-jolu-bg outline-none hover:opacity-90 focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
              <Plus aria-hidden className="size-4" />签发 Key
            </button>
          </div>

          {issuing && (
            <div className="flex flex-col gap-2.5 rounded-xl border border-jolu-border px-4 py-3.5" data-issue-form>
              <div className="flex flex-wrap items-center gap-2">
                <input value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} placeholder="名称，例如 书房台式机" aria-label="Key 名称"
                  className="h-9 min-w-[160px] flex-1 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-meta text-jolu-text outline-none focus:border-jolu-accent" />
                <input value={form.requests} onChange={(event) => setForm({ ...form, requests: event.target.value })} inputMode="numeric" placeholder="每日请求上限" aria-label="每日请求上限"
                  className="h-9 w-[130px] rounded-lg border border-jolu-border bg-jolu-bg px-3 text-meta text-jolu-text outline-none focus:border-jolu-accent" />
                <input value={form.tokens} onChange={(event) => setForm({ ...form, tokens: event.target.value })} inputMode="numeric" placeholder="每日 Token 上限" aria-label="每日 Token 上限"
                  className="h-9 w-[130px] rounded-lg border border-jolu-border bg-jolu-bg px-3 text-meta text-jolu-text outline-none focus:border-jolu-accent" />
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <select value={form.expiryDays ?? ""} aria-label="有效期" onChange={(event) => setForm({ ...form, expiryDays: event.target.value === "" ? null : Number(event.target.value) })}
                  className="h-9 w-[120px] rounded-lg border border-jolu-border bg-jolu-bg px-2 text-meta text-jolu-text outline-none focus:border-jolu-accent">
                  {EXPIRY_CHOICES.map((choice) => (
                    <option key={choice.label} value={choice.days ?? ""}>{choice.days === null ? choice.label : `有效 ${choice.label}`}</option>
                  ))}
                </select>
                <input value={form.models} onChange={(event) => setForm({ ...form, models: event.target.value })} placeholder="模型白名单，逗号分隔，留空 = 全部" aria-label="模型白名单"
                  className="h-9 min-w-[180px] flex-1 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-meta text-jolu-text outline-none focus:border-jolu-accent" />
                <label className="flex h-9 items-center gap-1.5 text-meta text-jolu-text">
                  <input type="checkbox" checked={form.readOnly} onChange={(event) => setForm({ ...form, readOnly: event.target.checked })} className="size-3.5 accent-[var(--jolu-accent)]" />
                  只读
                </label>
              </div>
              <div className="flex items-center gap-2">
                <button type="button" disabled={busy || form.name.trim() === ""} onClick={() => void issue()}
                  className="flex h-9 items-center rounded-lg bg-jolu-accent px-4 text-meta font-semibold text-[#071B10] outline-none disabled:opacity-45">签发</button>
                <button type="button" disabled={busy} onClick={() => setIssuing(false)} className="text-meta text-jolu-muted outline-none hover:text-jolu-text">取消</button>
                <span className="min-w-0 flex-1 truncate text-note text-jolu-muted">上限与白名单留空表示不限；只读 Key 只能查询（如读模型列表）。{TOKENS_LIMIT_NOTICE}</span>
              </div>
            </div>
          )}

          {issued && (
            <div className="flex flex-col gap-1.5 rounded-xl border border-jolu-on bg-jolu-status-success-bg px-4 py-3.5">
              <span className="text-meta font-medium text-jolu-on">{issued.name} 的 Key</span>
              <code className="break-all rounded-lg bg-jolu-bg px-3 py-2 font-mono text-meta text-jolu-text">{issued.key}</code>
              <div className="flex items-center gap-2">
                <button type="button" onClick={() => copyKey(issued.key)}
                  className="flex h-8 items-center rounded-lg bg-jolu-accent px-3.5 text-meta font-semibold text-[#071B10]">{copied ? "已复制" : "复制 Key"}</button>
                <button type="button" onClick={() => setIssued(null)} className="text-meta text-jolu-muted hover:text-jolu-text">我已保存</button>
                <span className="min-w-0 flex-1 truncate text-note text-jolu-danger">{KEY_ONCE_NOTICE}</span>
              </div>
            </div>
          )}

          <div className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
            <div className={cn(KEY_COLUMNS, "items-center bg-jolu-side px-4 py-2.5 text-note text-jolu-muted")}>
              <span>客户端</span><span>访问范围</span><span>今日请求</span><span>状态 / 有效期</span><span>最近使用</span><span />
            </div>
            {error && <div className="border-t border-jolu-border px-4 py-4 text-meta text-jolu-danger">读取失败：{error}</div>}
            {rows !== null && rows.length === 0 && !error && (
              <p className="border-t border-jolu-table-divider px-4 py-8 text-center text-meta text-jolu-muted">
                还没有客户端 Key。局域网设备接入前，先点「签发 Key」创建一个；本机回环访问不需要 Key。
              </p>
            )}
            {rows?.map((row) => {
              const now = Date.now()
              const exhausted = isExhausted(row)
              const expired = isExpired(row, now)
              const editingRow = editing?.name === row.name
              return (
                <div key={row.name} className={cn(KEY_COLUMNS, "items-center border-t border-jolu-table-divider px-4 py-3")}>
                  <span className="flex min-w-0 items-center gap-2.5">
                    <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-jolu-side">
                      <RowIcon row={row} />
                    </span>
                    <span className="flex min-w-0 flex-col gap-0.5">
                      <span className="truncate text-meta font-medium text-jolu-text" title={row.name}>{row.name}</span>
                      <code className="truncate font-mono text-note text-jolu-muted" title={row.masked}>{row.masked}</code>
                    </span>
                  </span>
                  <span className="min-w-0 truncate text-meta text-jolu-text" title={row.models?.join("、")}>{scopeText(row)}</span>
                  <span className="flex min-w-0 flex-col gap-1">
                    <span className={cn("text-meta tabular-nums", exhausted ? "text-jolu-danger" : "text-jolu-text")}>
                      {row.today_requests} / {limitText(row.requests_per_day)}
                    </span>
                    {row.requests_per_day !== null && (
                      <span className="h-1.5 w-[130px] overflow-hidden rounded-full bg-jolu-track">
                        <span className={cn("block h-full rounded-full", exhausted ? "bg-jolu-danger" : "bg-jolu-on")}
                          style={{ width: `${Math.min(100, Math.round((row.today_requests / row.requests_per_day) * 100))}%` }} />
                      </span>
                    )}
                  </span>
                  <span className="flex min-w-0 flex-col gap-0.5">
                    <span className={cn("text-meta font-medium", expired || exhausted ? "text-jolu-danger" : "text-jolu-on")}>
                      {expired ? "已过期" : exhausted ? "今日已用满" : "有效"}
                    </span>
                    <span className="text-note text-jolu-muted">{row.expires_ms === null ? "永不过期" : `剩余 ${daysLeft(row.expires_ms, now)} 天`}</span>
                  </span>
                  <span className="min-w-0 truncate text-note text-jolu-muted">{lastUsedText(row.last_used_ms, now)}</span>
                  <span className="justify-self-end">
                    <button type="button" aria-label={`调整 ${row.name}`} disabled={busy}
                      onClick={() => setEditing(editingRow ? null : { name: row.name, form: budgetForm(row.budget), accounts: row.accounts ?? [], concurrency: row.max_concurrency ?? null, error: null })}
                      className="grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
                      <EllipsisDot />
                    </button>
                  </span>
                  {editingRow && editing && (
                    <div className={cn(KEY_COLUMNS, "col-span-full flex flex-col gap-2.5 border-t border-jolu-table-divider bg-jolu-side px-4 py-3")} data-limits-editor>
                      <div className="col-span-full flex flex-col gap-1.5">
                        <span className="text-note text-jolu-muted">每日上限（留空即不限；Token 上限请求前按估算预扣、结束后回冲）与同时请求数</span>
                        <div className="grid grid-cols-3 gap-2">
                          {BUDGET_FIELDS.map((field) => (
                            <label key={field.key} className="flex flex-col gap-0.5">
                              <span className="text-note text-jolu-muted">{field.label}</span>
                              <input aria-label={`${row.name} ${field.label}`} inputMode={field.money ? "decimal" : "numeric"}
                                value={editing.form[field.key]}
                                onChange={(event) => setEditing({ ...editing, form: { ...editing.form, [field.key]: event.target.value }, error: null })}
                                className="h-8 rounded-md border border-jolu-border bg-jolu-bg px-2 text-meta text-jolu-text outline-none focus:border-jolu-accent" />
                            </label>
                          ))}
                          <label className="flex flex-col gap-0.5">
                            <span className="text-note text-jolu-muted">同时请求数</span>
                            <select aria-label={`${row.name} 的同时请求数`} value={editing.concurrency ?? 0} disabled={busy}
                              onChange={(event) => setEditing({ ...editing, concurrency: Number(event.target.value) || null })}
                              className="h-8 rounded-md border border-jolu-border bg-jolu-bg px-2 text-meta text-jolu-text outline-none focus:border-jolu-accent">
                              {concurrencyOptions(editing.concurrency).map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                            </select>
                          </label>
                        </div>
                        <span className="text-note text-jolu-muted">只能使用这些账号（全不勾 = 不限）</span>
                        <AccountChecks accounts={editing.accounts} onToggle={(id) => setEditing({
                          ...editing,
                          accounts: editing.accounts.includes(id) ? editing.accounts.filter((item) => item !== id) : [...editing.accounts, id],
                        })} />
                        {editing.error && <span className="text-note text-jolu-danger">{editing.error}</span>}
                        <div className="flex items-center gap-2">
                          <button type="button" disabled={busy} onClick={async () => {
                            const parsed = budgetFromForm(editing.form)
                            if (parsed.error !== undefined) { setEditing({ ...editing, error: parsed.error }); return }
                            try {
                              await gatewayApi.setClientKeyLimits(row.name, parsed.budget, editing.accounts.length ? editing.accounts : null)
                              await gatewayApi.setClientKeyConcurrency(row.name, editing.concurrency)
                              setEditing(null)
                              toast.success("限额已保存", row.name)
                              await load()
                            } catch (reason) {
                              setEditing({ ...editing, error: String(reason) })
                            }
                          }} className="h-8 rounded-lg bg-jolu-accent px-3.5 text-meta font-semibold text-[#071B10] disabled:opacity-45">保存限额</button>
                          <button type="button" disabled={busy} onClick={() => setEditing(null)} className="text-meta text-jolu-muted hover:text-jolu-text">取消</button>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              )
            })}
          </div>

          <p className="flex items-center gap-2 text-note text-jolu-muted">
            <KeyRound aria-hidden className="size-3.5 shrink-0" />
            Key 原文只在签发时显示一次。行尾菜单可调整限额、账号范围或撤销。
          </p>
        </section>
      ) : (
        <AddressChip port={port} shared={shared} models={models} copy={copy} />
      )}

      {revoking && (
        <ConfirmDialog title={`撤销「${revoking}」？`} description="删除后这个 Key 立即失效，使用它的设备会收到 401；历史路由流水的归属保留。"
          confirmLabel="撤销" danger onClose={() => setRevoking(null)}
          onConfirm={async () => {
            const name = revoking
            setRevoking(null)
            await run(async () => {
              await gatewayApi.revokeClientKey(name)
              await load()
            }, `已撤销「${name}」`)
          }} />
      )}
    </div>
  )
}

/** 行首图标按访问范围给语义：分组 → 分叉、白名单 → 清单、只读 → 眼睛，其余 = 一般客户端。 */
function RowIcon({ row }: { row: ClientKeySummaryDto }) {
  if (row.models?.some((model) => model.startsWith("group/"))) return <GitFork aria-hidden className="size-3.5 text-jolu-muted" />
  if (row.models?.length) return <ListFilter aria-hidden className="size-3.5 text-jolu-muted" />
  if (row.read_only) return <Eye aria-hidden className="size-3.5 text-jolu-muted" />
  return <Laptop aria-hidden className="size-3.5 text-jolu-muted" />
}

function scopeText(row: ClientKeySummaryDto): string {
  const scope = row.models === null ? "全部模型" : row.models.length === 0 ? "没有可用模型" : row.models.length === 1 ? row.models[0] : `${row.models.length} 个模型`
  if (row.read_only) return `${scope} · 只读`
  if (row.accounts?.length) return `${scope} · ${row.accounts.length} 个账号`
  return scope
}

function daysLeft(expiresMs: number, now: number): number {
  return Math.max(0, Math.ceil((expiresMs - now) / 86_400_000))
}

function Chip({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button type="button" aria-pressed={active} onClick={onClick}
      className={cn(
        "flex h-9 items-center rounded-lg px-4 text-meta font-medium outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
        active ? "bg-jolu-selected text-jolu-on" : "border border-jolu-border text-jolu-text hover:bg-jolu-side",
      )}>
      {children}
    </button>
  )
}

/** 连接地址与示例页签（沿用 V5-4 的地址卡与接入示例；IP 白名单无 V6 画板，保留在此页签）。 */
function AddressChip({ port, shared, models, copy }: {
  port: number
  shared: boolean
  models: string[]
  copy: (text: string, note?: string) => void
}) {
  const [protocol, setProtocol] = useState<"anthropic" | "openai" | "gemini">("anthropic")
  const [shell, setShell] = useState<"powershell" | "shell" | "curl" | "python" | "node">("powershell")
  const [model, setModel] = useState("")
  const chosen = model || models[0] || "group/你的分组"
  const code = example(protocol, shell, `http://127.0.0.1:${port}`, chosen, shared)
  return (
    <div className="flex flex-col gap-4">
      <section className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
        <div className="flex items-baseline gap-2 bg-jolu-side px-4 py-2.5">
          <span className="text-[13px] font-semibold text-jolu-text">地址</span>
          <span className="text-meta text-jolu-muted">本机回环不需要 Key；局域网请求必须带一个启用的客户端 Key</span>
        </div>
        <AddressRow label="端口" value={String(port)} note="改端口还没有命令（待确认）" />
        <AddressRow label="本机" value={`http://127.0.0.1:${port}`} mono action={<CopyBtn label="复制本机地址" onClick={() => copy(`http://127.0.0.1:${port}`)} />} />
        <LanRow port={port} shared={shared} copy={copy} />
      </section>

      <section className="flex flex-col overflow-hidden rounded-xl border border-jolu-border">
        <div className="flex items-center gap-2 bg-jolu-side px-4 py-2.5">
          <span className="text-[13px] font-semibold text-jolu-text">接入示例</span>
          <span className="min-w-0 flex-1 truncate text-meta text-jolu-muted">点一个模型，示例跟着变</span>
          <select aria-label="示例模型" value={model} onChange={(event) => setModel(event.target.value)}
            className="h-8 max-w-[220px] rounded-md border border-jolu-border bg-jolu-bg px-2 text-note text-jolu-text outline-none">
            {models.length === 0 && <option value="">（模型池为空）</option>}
            {models.map((entry) => <option key={entry} value={entry}>{entry}</option>)}
          </select>
        </div>
        <div className="flex flex-wrap items-center gap-3 px-4 pt-3">
          {(["anthropic", "openai", "gemini"] as const).map((value) => (
            <button key={value} type="button" aria-pressed={protocol === value} onClick={() => setProtocol(value)}
              className={cn("text-meta outline-none", protocol === value ? "font-semibold text-jolu-on" : "text-jolu-muted hover:text-jolu-text")}>
              {value === "openai" ? "OpenAI" : value === "anthropic" ? "Anthropic" : "Gemini"}
            </button>
          ))}
          <span className="flex-1" />
          {(["powershell", "shell", "curl", "python", "node"] as const).map((value) => (
            <button key={value} type="button" aria-pressed={shell === value} onClick={() => setShell(value)}
              className={cn("text-meta outline-none", shell === value ? "font-semibold text-jolu-on" : "text-jolu-muted hover:text-jolu-text")}>
              {value === "powershell" ? "PowerShell" : value === "shell" ? "Shell" : value}
            </button>
          ))}
        </div>
        <div className="relative m-4 mt-3 rounded-lg bg-jolu-side px-3.5 py-3">
          <pre className="overflow-x-auto whitespace-pre font-mono text-note leading-5 text-jolu-text">{code}</pre>
          <button type="button" aria-label="复制示例" onClick={() => copy(code)}
            className="absolute right-2 top-2 flex size-7 items-center justify-center rounded-md hover:bg-jolu-stage">
            <Copy aria-hidden className="size-3.5 text-jolu-muted" />
          </button>
        </div>
      </section>

      <AllowIpsCard />
    </div>
  )
}

function AddressRow({ label, value, note, mono, action }: { label: string; value: string; note?: string; mono?: boolean; action?: React.ReactNode }) {
  return (
    <div className="flex min-h-11 items-center gap-3 border-t border-jolu-table-divider px-4 py-2">
      <span className="w-14 shrink-0 text-meta text-jolu-muted">{label}</span>
      <span className={cn("min-w-0 truncate text-meta text-jolu-text", mono && "font-mono")}>{value}</span>
      {note && <span className="min-w-0 truncate text-note text-jolu-muted">{note}</span>}
      <span className="flex-1" />
      {action}
    </div>
  )
}

function LanRow({ port, shared, copy }: { port: number; shared: boolean; copy: (text: string, note?: string) => void }) {
  const [lan, setLan] = useState<string | null>(null)
  useEffect(() => {
    void gatewayApi.lanAddress().then(setLan).catch(() => setLan(null))
  }, [])
  const value = lan ? `http://${lan}:${port}` : null
  return (
    <AddressRow label="局域网" value={value ?? "没有检测到局域网地址"} mono={!!value}
      note={shared ? undefined : "开启局域网共享后可用"}
      action={value ? <CopyBtn label="复制局域网地址" onClick={() => copy(value)} /> : undefined} />
  )
}

function CopyBtn({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button type="button" onClick={onClick} aria-label={label}
      className="flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-jolu-border px-2.5 text-note text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
      <Copy aria-hidden className="size-3 text-jolu-muted" />复制
    </button>
  )
}

function AccountChecks({ accounts, onToggle }: { accounts: string[]; onToggle: (id: string) => void }) {
  const { connections } = useConnections()
  return (
    <div className="flex flex-wrap gap-x-4 gap-y-1.5">
      {connections.map((connection) => (
        <label key={connection.id} className="flex items-center gap-1.5 text-meta text-jolu-text">
          <input type="checkbox" checked={accounts.includes(connection.id)} onChange={() => onToggle(connection.id)} className="size-3.5 accent-[var(--jolu-accent)]" />
          {connection.name}
        </label>
      ))}
      {connections.length === 0 && <span className="text-note text-jolu-muted">还没有连接</span>}
    </div>
  )
}

/** 允许连入的来源地址（P2-9；无 V6 画板，保留在「连接地址与示例」页签，标「待确认」）。 */
function AllowIpsCard() {
  const toast = useToast()
  const settings = useSettings().settings
  const saved = (settings.gateway_allow_ips ?? []).join("\n")
  const [draft, setDraft] = useState<string | null>(null)
  const [fromEnv, setFromEnv] = useState(false)
  useEffect(() => {
    void gatewayApi.allowIpsFromEnv().then(setFromEnv).catch(() => {})
  }, [])
  const text = draft ?? saved
  return (
    <section className="flex flex-col gap-1.5 rounded-xl border border-jolu-border px-4 py-3.5">
      <span className="text-[13px] font-semibold text-jolu-text">允许连入的来源地址</span>
      <span className="text-note text-jolu-muted">每行一个 IP 或网段（如 192.168.1.20、192.168.1.0/24）；留空 = 局域网内都可连。不在列表里的地址在看 Key 之前就被拒绝，本机始终可连。（待确认：无 V6 画板）</span>
      {fromEnv && <span className="text-note text-jolu-warn">已由环境变量 JOLU_ALLOW_IPS 指定，这里保存的不会生效。</span>}
      <textarea aria-label="允许连入的来源地址" rows={3} value={text} spellCheck={false}
        onChange={(event) => setDraft(event.target.value)}
        className="rounded-lg border border-jolu-border bg-jolu-bg px-3 py-2 font-mono text-meta text-jolu-text outline-none focus:border-jolu-accent" />
      {draft !== null && (
        <div className="flex items-center gap-2">
          <button type="button" onClick={async () => {
            try {
              await gatewayApi.allowIpsSet(draft.split(/[\s,;]+/).filter(Boolean))
              setDraft(null)
              toast.success("白名单已保存", "立即生效")
            } catch (reason) {
              toast.danger("保存白名单失败", String(reason))
            }
          }} className="h-8 rounded-lg bg-jolu-accent px-3.5 text-meta font-semibold text-[#071B10]">保存白名单</button>
          <button type="button" onClick={() => setDraft(null)} className="text-meta text-jolu-muted hover:text-jolu-text">撤销</button>
        </div>
      )}
    </section>
  )
}

/** 接入示例：Key 一律写占位（局域网换成你的客户端 Key），不把任何真实 Key 放进页面。 */
export function example(protocol: "anthropic" | "openai" | "gemini", shell: "powershell" | "shell" | "curl" | "python" | "node", base: string, model: string, shared: boolean): string {
  const key = shared ? "<你的客户端 Key>" : "jolu-local"
  const path = protocol === "openai" ? "/v1/chat/completions" : protocol === "anthropic" ? "/v1/messages" : `/v1beta/models/${model}:generateContent`
  const body = protocol === "gemini"
    ? `{"contents":[{"parts":[{"text":"你好"}]}]}`
    : protocol === "anthropic"
      ? `{"model":"${model}","max_tokens":256,"messages":[{"role":"user","content":"你好"}]}`
      : `{"model":"${model}","messages":[{"role":"user","content":"你好"}]}`
  const header = protocol === "anthropic" ? "x-api-key" : protocol === "gemini" ? "x-goog-api-key" : "Authorization"
  const value = protocol === "openai" ? `Bearer ${key}` : key
  switch (shell) {
    case "powershell":
      if (protocol === "anthropic") return `$env:ANTHROPIC_BASE_URL = "${base}"\n$env:ANTHROPIC_AUTH_TOKEN = "${key}"\nclaude --model ${model}`
      if (protocol === "openai") return `$env:OPENAI_BASE_URL = "${base}/v1"\n$env:OPENAI_API_KEY = "${key}"\ncodex -m ${model}`
      return `$env:GOOGLE_GEMINI_BASE_URL = "${base}"\n$env:GEMINI_API_KEY = "${key}"\ngemini -m ${model}`
    case "shell":
      if (protocol === "anthropic") return `export ANTHROPIC_BASE_URL="${base}"\nexport ANTHROPIC_AUTH_TOKEN="${key}"\nclaude --model ${model}`
      if (protocol === "openai") return `export OPENAI_BASE_URL="${base}/v1"\nexport OPENAI_API_KEY="${key}"\ncodex -m ${model}`
      return `export GOOGLE_GEMINI_BASE_URL="${base}"\nexport GEMINI_API_KEY="${key}"\ngemini -m ${model}`
    case "curl":
      return `curl ${base}${path} \\\n  -H "content-type: application/json" \\\n  -H "${header}: ${value}" \\\n  -d '${body}'`
    case "python":
      return `import requests\n\nr = requests.post(\n    "${base}${path}",\n    headers={"${header}": "${value}"},\n    json=${body},\n)\nprint(r.json())`
    case "node":
      return `const r = await fetch("${base}${path}", {\n  method: "POST",\n  headers: { "content-type": "application/json", "${header}": "${value}" },\n  body: JSON.stringify(${body}),\n})\nconsole.log(await r.json())`
  }
}

function EllipsisDot() {
  return (
    <svg aria-hidden viewBox="0 0 16 16" className="size-4" fill="currentColor">
      <circle cx="3.2" cy="8" r="1.4" /><circle cx="8" cy="8" r="1.4" /><circle cx="12.8" cy="8" r="1.4" />
    </svg>
  )
}
