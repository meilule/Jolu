import { useEffect, useState } from "react"
import { gatewayApi, type ClientKeySummaryDto, type IssuedClientKeyDto } from "@/lib/gatewayApi"
import { useConnections } from "@/hooks/useConnections"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import { useLatestGate } from "@/hooks/useLatestGate"
import { Icon } from "@/icons"
import { Button, Checkbox, ConfirmDialog, Dialog, FormField, IconButton, Input, Select, StatusPill, Switch, Tabs } from "@/ui"
import { cn } from "@/lib/utils"
import {
  BUDGET_FIELDS, budgetForm, budgetFromForm, concurrencyOptions, expiresAtFrom, EXPIRY_CHOICES,
  isExhausted, isExpired, KEY_ONCE_NOTICE, lastUsedText, limitText, parseModelList, TOKENS_LIMIT_NOTICE,
  type BudgetForm,
} from "./sharedAccess"

const KEY_COLUMNS = "grid grid-cols-[minmax(180px,1.2fr)_minmax(120px,1fr)_minmax(140px,1fr)_minmax(120px,0.9fr)_96px_36px] gap-3"

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

  // 挂载与每次操作之后都会调 load：只认最新一次的结果（见 lib/latestGate.ts）
  const loadGate = useLatestGate()
  const load = async () => {
    const isLatest = loadGate.begin()
    setError(null)
    try {
      const next = await gatewayApi.clientKeys()
      if (isLatest()) setRows(next)
    } catch (reason) {
      if (isLatest()) setError(String(reason))
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
    <div className="flex min-w-0 flex-col gap-3.5">
      <section className="flex items-center gap-3.5 rounded-jn-card border border-jn-line bg-jn-surface px-5 py-[18px] shadow-jn-surface">
        <span className="flex size-[42px] shrink-0 items-center justify-center rounded-[14px] bg-jn-well">
          <Icon name="external" size="lg" className="text-jn-text-2" />
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-[15px] font-bold text-jn-text">局域网共享</span>
          <span className="text-[12px] text-jn-text-3">开启后同一网络里的设备可以用客户端 Key 访问本机路由；下次启动中转时生效</span>
        </span>
        <StatusPill tone={shared ? "success" : "neutral"}>{shared ? "已开启" : "已关闭"}</StatusPill>
        <Switch checked={shared} disabled={busy} label="局域网共享" onChange={onSharedChange} />
      </section>

      <Tabs style="segment-lg" className="self-start" value={chip} onChange={setChip}
        items={[{ key: "keys", label: "客户端 Key" }, { key: "address", label: "连接地址与示例" }]} />

      {chip === "keys" ? (
        <section className="flex flex-col gap-3">
          <div className="flex items-center gap-2">
            <span className="text-[14px] font-bold text-jn-text">客户端 Key</span>
            {rows !== null && <span className="flex h-5 items-center rounded-full bg-jn-well px-2 text-[11px] font-bold text-jn-text-2">{rows.length}</span>}
            <span className="min-w-0 flex-1" />
            <Button kind="primary" size="sm" icon="add" disabled={busy} onClick={() => { setIssued(null); setIssuing(true) }}>签发 Key</Button>
          </div>

          {/* R1-34 签发客户端 Key 弹窗 */}
          <Dialog
            open={issuing}
            onOpenChange={(open) => { if (!open && !busy) { setIssuing(false); setIssued(null) } }}
            preventClose={busy}
            width="md"
            title="签发客户端 Key"
            description="局域网设备接入时使用；本机回环访问不需要 Key。"
            footerStart={issued ? KEY_ONCE_NOTICE : `留空即不限 · ${TOKENS_LIMIT_NOTICE}`}
            actions={
              issued ? (
                <Button kind="primary" size="md" icon="check" onClick={() => { setIssuing(false); setIssued(null) }}>我已保存</Button>
              ) : (
                <>
                  <Button kind="ghost" size="md" disabled={busy} onClick={() => setIssuing(false)}>取消</Button>
                  <Button kind="primary" size="md" icon="key" loading={busy} disabled={busy || form.name.trim() === ""} onClick={() => void issue()}>签发</Button>
                </>
              )
            }
          >
            <div className="flex flex-col gap-3.5" data-issue-form>
              <FormField label="名称" required hint="名称用于区分不同设备，例如「书房台式机」">
                <Input size="lg" value={form.name} disabled={busy || Boolean(issued)} onChange={(event) => setForm({ ...form, name: event.target.value })} placeholder="书房台式机" aria-label="Key 名称" />
              </FormField>
              <div className="grid grid-cols-2 gap-3">
                <FormField label="每日请求上限">
                  <Input size="lg" value={form.requests} disabled={busy || Boolean(issued)} onChange={(event) => setForm({ ...form, requests: event.target.value })} inputMode="numeric" placeholder="留空即不限" aria-label="每日请求上限" />
                </FormField>
                <FormField label="每日 Token 上限">
                  <Input size="lg" value={form.tokens} disabled={busy || Boolean(issued)} onChange={(event) => setForm({ ...form, tokens: event.target.value })} inputMode="numeric" placeholder="留空即不限" aria-label="每日 Token 上限" />
                </FormField>
              </div>
              <FormField label="有效期">
                <Tabs
                  style="segment"
                  className="self-start"
                  value={form.expiryDays === null ? "" : String(form.expiryDays)}
                  onChange={(value) => { if (!busy && !issued) setForm({ ...form, expiryDays: value === "" ? null : Number(value) }) }}
                  items={EXPIRY_CHOICES.map((choice) => ({
                    key: choice.days === null ? "" : String(choice.days),
                    label: choice.label,
                  }))}
                />
              </FormField>
              <FormField label="模型白名单" hint="逗号分隔，留空 = 全部模型">
                <Input size="lg" value={form.models} disabled={busy || Boolean(issued)} onChange={(event) => setForm({ ...form, models: event.target.value })} placeholder="group/daily, gpt-5" aria-label="模型白名单" />
              </FormField>
              <Checkbox
                checked={form.readOnly}
                disabled={busy || Boolean(issued)}
                label="只读（只能查询，例如读取模型列表）"
                onChange={(checked) => setForm({ ...form, readOnly: checked })}
              />
              {issued && (
                <div className="flex flex-col gap-2 rounded-[16px] border border-jn-primary-soft bg-jn-primary-wash p-3.5" data-issued-key>
                  <span className="flex items-center gap-1.5 text-xs font-semibold text-jn-primary-ink">
                    <Icon name="success" size="sm" />
                    已签发「{issued.name}」
                  </span>
                  <div className="flex items-center gap-2 rounded-xl bg-jn-surface px-3 py-2">
                    <code className="min-w-0 flex-1 break-all font-mono text-xs font-medium text-jn-text">{issued.key}</code>
                    <Button kind="soft" size="sm" icon="copy" onClick={() => copyKey(issued.key)}>{copied ? "已复制" : "复制 Key"}</Button>
                  </div>
                </div>
              )}
            </div>
          </Dialog>

          <div className="overflow-x-auto rounded-jn-card border border-jn-line bg-jn-surface p-3 shadow-jn-surface">
          <div className="flex min-w-[740px] flex-col gap-0.5">
            <div className={cn(KEY_COLUMNS, "items-center px-2.5 pb-1.5 text-[12px] font-medium text-jn-text-3")}>
              <span>客户端</span><span>访问范围</span><span>今日请求</span><span>状态 / 有效期</span><span>最近使用</span><span />
            </div>
            {error && <div className="px-2.5 py-4 text-[12px] text-jn-danger">读取失败：{error}</div>}
            {rows !== null && rows.length === 0 && !error && (
              <p className="px-2.5 py-8 text-center text-[12px] text-jn-text-3">
                还没有客户端 Key。局域网设备接入前，先点「签发 Key」创建一个；本机回环访问不需要 Key。
              </p>
            )}
            {rows?.map((row) => {
              const now = Date.now()
              const exhausted = isExhausted(row)
              const expired = isExpired(row, now)
              const editingRow = editing?.name === row.name
              const tone = expired || exhausted ? "danger" : "success"
              return (
                <div key={row.name} className={cn(KEY_COLUMNS, "items-center rounded-[14px] px-2.5 py-2 odd:bg-jn-well")}>
                  <span className="flex min-w-0 items-center gap-2.5">
                    <span className="flex size-8 shrink-0 items-center justify-center rounded-[11px] bg-jn-surface">
                      <RowIcon row={row} />
                    </span>
                    <span className="flex min-w-0 flex-col gap-px">
                      <span className="truncate text-[13px] font-semibold text-jn-text" title={row.name}>{row.name}</span>
                      <code className="truncate font-mono text-[11px] text-jn-text-3" title={row.masked}>{row.masked}</code>
                    </span>
                  </span>
                  <span className="min-w-0 truncate text-[12.5px] text-jn-text-2" title={row.models?.join("、")}>{scopeText(row)}</span>
                  <span className="flex min-w-0 flex-col gap-1">
                    <span className={cn("text-[12.5px] font-semibold tabular-nums", exhausted ? "text-jn-danger" : "text-jn-text")}>
                      {row.today_requests} / {limitText(row.requests_per_day)}
                    </span>
                    {row.requests_per_day !== null && (
                      <span className="h-1.5 w-[130px] overflow-hidden rounded-full bg-jn-track">
                        <span className={cn("block h-full rounded-full", exhausted ? "bg-jn-danger" : "bg-jn-primary")}
                          style={{ width: `${Math.min(100, Math.round((row.today_requests / row.requests_per_day) * 100))}%` }} />
                      </span>
                    )}
                  </span>
                  <span className="flex min-w-0 flex-col gap-1">
                    <StatusPill tone={tone} className="self-start">{expired ? "已过期" : exhausted ? "今日已用满" : "有效"}</StatusPill>
                    <span className="text-[11px] text-jn-text-3">{row.expires_ms === null ? "永不过期" : `剩余 ${daysLeft(row.expires_ms, now)} 天`}</span>
                  </span>
                  <span className="min-w-0 truncate text-[12px] text-jn-text-3">{lastUsedText(row.last_used_ms, now)}</span>
                  <span className="justify-self-end">
                    <IconButton icon="more" label={`调整 ${row.name}`} size="sm" disabled={busy}
                      onClick={() => setEditing(editingRow ? null : { name: row.name, form: budgetForm(row.budget), accounts: row.accounts ?? [], concurrency: row.max_concurrency ?? null, error: null })} />
                  </span>
                  {editingRow && editing && (
                    <div className={cn(KEY_COLUMNS, "col-span-full flex flex-col gap-2.5 rounded-[14px] bg-jn-stage px-3.5 py-3")} data-limits-editor>
                      <div className="col-span-full flex flex-col gap-1.5">
                        <span className="text-[11px] text-jn-text-3">每日上限与同时请求数 · 留空即不限</span>
                        <div className="grid grid-cols-4 gap-2">
                          {BUDGET_FIELDS.map((field) => (
                            <label key={field.key} className="flex flex-col gap-1">
                              <span className="text-[11px] text-jn-text-3">{field.label}</span>
                              <Input size="md" aria-label={`${row.name} ${field.label}`} inputMode={field.money ? "decimal" : "numeric"}
                                value={editing.form[field.key]}
                                onChange={(event) => setEditing({ ...editing, form: { ...editing.form, [field.key]: event.target.value }, error: null })} />
                            </label>
                          ))}
                          <label className="flex flex-col gap-1">
                            <span className="text-[11px] text-jn-text-3">同时请求数</span>
                            <Select width="full" label={`${row.name} 的同时请求数`} value={String(editing.concurrency ?? 0)} disabled={busy}
                              onChange={(value) => setEditing({ ...editing, concurrency: Number(value) || null })}
                              options={concurrencyOptions(editing.concurrency).map((option) => ({ value: String(option.value), label: option.label }))} />
                          </label>
                        </div>
                        <span className="text-[11px] text-jn-text-3">只能使用这些账号（全不勾 = 不限）</span>
                        <AccountChecks accounts={editing.accounts} onToggle={(id) => setEditing({
                          ...editing,
                          accounts: editing.accounts.includes(id) ? editing.accounts.filter((item) => item !== id) : [...editing.accounts, id],
                        })} />
                        {editing.error && <span className="text-[11px] text-jn-danger">{editing.error}</span>}
                        <div className="flex items-center gap-2">
                          <Button kind="primary" size="sm" disabled={busy} onClick={async () => {
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
                          }}>保存限额</Button>
                          <Button kind="ghost" size="sm" disabled={busy} onClick={() => setEditing(null)}>取消</Button>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              )
            })}
          </div>
          </div>

          <p className="flex items-center gap-2 text-[11px] text-jn-text-3">
            <Icon name="shield" size="sm" className="shrink-0" />
            Key 原文只在签发时显示一次。行尾菜单可调整限额、账号范围或撤销。
          </p>
        </section>
      ) : (
        <AddressChip port={port} shared={shared} models={models} copy={copy} />
      )}

      {revoking && (
        <ConfirmDialog
          open
          onOpenChange={(open) => { if (!open) setRevoking(null) }}
          title={`撤销「${revoking}」？`}
          description="删除后这个 Key 立即失效，使用它的设备会收到 401；历史路由流水的归属保留。"
          confirmText="撤销"
          loading={busy}
          onConfirm={() => {
            const name = revoking
            setRevoking(null)
            void run(async () => {
              await gatewayApi.revokeClientKey(name)
              await load()
            }, `已撤销「${name}」`)
          }}
        />
      )}
    </div>
  )
}

/** 行首图标按访问范围给语义：分组 → 分叉、白名单 → 文件、只读 → 眼睛，其余 = 一般客户端。 */
function RowIcon({ row }: { row: ClientKeySummaryDto }) {
  const icon = row.models?.some((model) => model.startsWith("group/")) ? "blocks" : row.models?.length ? "file" : row.read_only ? "eye" : "users"
  return <Icon name={icon} size="sm" className="text-jn-text-2" />
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
      <section className="flex flex-col overflow-hidden rounded-xl border border-jn-line">
        <div className="flex items-baseline gap-2 bg-jn-surface px-4 py-2.5">
          <span className="text-[13px] font-semibold text-jn-text">地址</span>
          <span className="text-[12px] text-jn-text-3">本机不需要 Key；局域网请求必须带启用的客户端 Key</span>
        </div>
        <AddressRow label="端口" value={String(port)} note="改端口还没有命令（待确认）" />
        <AddressRow label="本机" value={`http://127.0.0.1:${port}`} mono action={<CopyBtn label="复制本机地址" onClick={() => copy(`http://127.0.0.1:${port}`)} />} />
        <LanRow port={port} shared={shared} copy={copy} />
      </section>

      <section className="flex flex-col overflow-hidden rounded-xl border border-jn-line">
        <div className="flex items-center gap-2 bg-jn-surface px-4 py-2.5">
          <span className="text-[13px] font-semibold text-jn-text">接入示例</span>
          <span className="min-w-0 flex-1 truncate text-[12px] text-jn-text-3">按模型与协议生成</span>
          <div className="w-[220px] shrink-0">
            <Select variant="filter" width="full" label="示例模型" value={model || null} onChange={setModel}
              placeholder="（模型池为空）"
              options={models.map((entry) => ({ value: entry, label: entry }))} />
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-3 px-4 pt-3">
          {(["anthropic", "openai", "gemini"] as const).map((value) => (
            <button key={value} type="button" aria-pressed={protocol === value} onClick={() => setProtocol(value)}
              className={cn("text-[12px] outline-none", protocol === value ? "font-semibold text-jn-primary-ink" : "text-jn-text-3 hover:text-jn-text")}>
              {value === "openai" ? "OpenAI" : value === "anthropic" ? "Anthropic" : "Gemini"}
            </button>
          ))}
          <span className="flex-1" />
          {(["powershell", "shell", "curl", "python", "node"] as const).map((value) => (
            <button key={value} type="button" aria-pressed={shell === value} onClick={() => setShell(value)}
              className={cn("text-[12px] outline-none", shell === value ? "font-semibold text-jn-primary-ink" : "text-jn-text-3 hover:text-jn-text")}>
              {value === "powershell" ? "PowerShell" : value === "shell" ? "Shell" : value}
            </button>
          ))}
        </div>
        <div className="relative m-4 mt-3 rounded-lg bg-jn-surface px-3.5 py-3">
          <pre className="overflow-x-auto whitespace-pre font-mono text-[11px] leading-5 text-jn-text">{code}</pre>
          <span className="absolute top-2 right-2">
            <IconButton icon="copy" label="复制示例" size="sm" onClick={() => copy(code)} />
          </span>
        </div>
      </section>

      <AllowIpsCard />
    </div>
  )
}

function AddressRow({ label, value, note, mono, action }: { label: string; value: string; note?: string; mono?: boolean; action?: React.ReactNode }) {
  return (
    <div className="flex min-h-11 items-center gap-3 border-t border-jn-line px-4 py-2">
      <span className="w-14 shrink-0 text-[12px] text-jn-text-3">{label}</span>
      <span className={cn("min-w-0 truncate text-[12px] text-jn-text", mono && "font-mono")}>{value}</span>
      {note && <span className="min-w-0 truncate text-[11px] text-jn-text-3">{note}</span>}
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
  return <Button kind="ghost" size="sm" icon="copy" onClick={onClick} title={label}>复制</Button>
}

function AccountChecks({ accounts, onToggle }: { accounts: string[]; onToggle: (id: string) => void }) {
  const { connections } = useConnections()
  return (
    <div className="flex flex-wrap gap-x-4 gap-y-1.5">
      {connections.map((connection) => (
        <label key={connection.id} className="flex items-center gap-1.5 text-[12px] text-jn-text">
          <input type="checkbox" checked={accounts.includes(connection.id)} onChange={() => onToggle(connection.id)} className="size-3.5 accent-[var(--jn-primary)]" />
          {connection.name}
        </label>
      ))}
      {connections.length === 0 && <span className="text-[11px] text-jn-text-3">还没有连接</span>}
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
    <section className="flex flex-col gap-1.5 rounded-xl border border-jn-line px-4 py-3.5">
      <span className="text-[13px] font-semibold text-jn-text">允许连入的来源地址</span>
      <span className="text-[11px] text-jn-text-3">每行一个 IP 或网段 · 留空则局域网内都可连 · 本机始终可连</span>
      {fromEnv && <span className="text-[11px] text-jn-warning">已由环境变量 JOLU_ALLOW_IPS 指定，这里保存的不会生效。</span>}
      <textarea aria-label="允许连入的来源地址" rows={3} value={text} spellCheck={false}
        onChange={(event) => setDraft(event.target.value)}
        className="rounded-lg border border-jn-line bg-jn-surface px-3 py-2 font-mono text-[12px] text-jn-text outline-none focus:border-jn-primary" />
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
          }} className="h-8 rounded-lg bg-jn-primary px-3.5 text-[12px] font-semibold text-jn-on-primary">保存白名单</button>
          <button type="button" onClick={() => setDraft(null)} className="text-[12px] text-jn-text-3 hover:text-jn-text">撤销</button>
        </div>
      )}
    </section>
  )
}

/** 接入示例：Key 一律写占位（局域网换成你的客户端 Key），不把任何真实 Key 放进页面。 */
export function example(protocol: "anthropic" | "openai" | "gemini", shell: "powershell" | "shell" | "curl" | "python" | "node", base: string, model: string, shared: boolean): string {
  const key = shared ? "<你的客户端 Key>" : "jn-primary-ink"
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


