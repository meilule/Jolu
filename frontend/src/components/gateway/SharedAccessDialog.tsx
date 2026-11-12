import { Fragment, useEffect, useState } from "react"
import { Network, TriangleAlert } from "lucide-react"
import { cn } from "@/lib/utils"
import { gatewayApi, type ClientKeySummaryDto, type IssuedClientKeyDto } from "@/lib/gatewayApi"
import { useToast } from "@/components/ui/Toast"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { Button, Modal } from "./GatewayDialogs"
import { useConnections } from "@/hooks/useConnections"
import { useSettings } from "@/hooks/useSettings"
import { isTauri } from "@/lib/api"
import {
  EXPIRY_CHOICES,
  KEY_ONCE_NOTICE,
  TOKENS_LIMIT_NOTICE,
  BUDGET_FIELDS,
  budgetForm,
  budgetFromForm,
  budgetTags,
  constraintTags,
  spendLines,
  type BudgetForm,
  concurrencyOptions,
  expiresAtFrom,
  isExhausted,
  parseModelList,
  lastUsedText,
  limitText,
  sharingWithoutKeys,
  usageText,
} from "./sharedAccess"

/**
 * 局域网共享与命名客户端 Key（待画板确认）。
 *
 * 后端语义（`gateway/access.rs` + `proxy.rs`）：
 * - 共享开启后网关绑 0.0.0.0，鉴权切**严格**（命名 Key 比对），宽松模式被关掉；
 * - 绑定地址在下次 `proxy::start` 生效，所以开关旁要提示「重启网关监听」；
 * - 观测模式永不共享（透传上游凭证且无鉴权）；
 * - `requests_per_day` 到量直接 429（进 provider 之前），`tokens_per_day` 预扣 + 回冲强制（budget.rs）；
 * - Key 原文只在签发时返回一次。
 */
export function SharedAccessDialog({ shared, busy, onSharedChange, onClose, run }: {
  /** 当前共享开关值（来自 settings.gateway_shared_lan） */
  shared: boolean
  busy: boolean
  /** 切换共享开关：由调用方走统一的 run（加锁 + toast） */
  onSharedChange: (on: boolean) => void
  onClose: () => void
  /** 统一的执行包装：返回是否成功，供弹窗内部自己管 loading */
  run: (work: () => Promise<unknown>, successTitle?: string) => Promise<boolean>
}) {
  const toast = useToast()
  const { connections } = useConnections()
  /** 正在编辑上限的 Key（待确认：无画板，行下展开的最小版） */
  const [editing, setEditing] = useState<{ name: string; form: BudgetForm; accounts: string[]; error: string | null } | null>(null)
  const [rows, setRows] = useState<ClientKeySummaryDto[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [issuing, setIssuing] = useState(false)
  const emptyForm = { name: "", requests: "", tokens: "", expiryDays: null as number | null, models: "", readOnly: false }
  const [form, setForm] = useState(emptyForm)
  const [issued, setIssued] = useState<IssuedClientKeyDto | null>(null)
  const [revoking, setRevoking] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)

  const load = async () => {
    setLoading(true)
    setError(null)
    try {
      setRows(await gatewayApi.clientKeys())
    } catch (reason) {
      setError(String(reason))
    } finally {
      setLoading(false)
    }
  }
  // 首次展开时拉一次；弹窗关闭即销毁，不缓存
  if (rows === null && !loading && !error) void load()

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
    setIssuing(true)
    const ok = await run(async () => {
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
    setIssuing(false)
    if (!ok) return
  }

  const copyKey = async (key: string) => {
    try {
      await navigator.clipboard.writeText(key)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 2000)
      toast.success("已复制", "粘贴到局域网设备的网关地址里")
    } catch {
      toast.danger("复制失败", "请手动选中 Key 复制")
    }
  }

  const danger = sharingWithoutKeys(shared, rows?.length ?? 0)
  const settings = useSettings().settings
  const savedAllow = (settings.gateway_allow_ips ?? []).join("\n")
  const [allowDraft, setAllowDraft] = useState<string | null>(null)
  const [allowError, setAllowError] = useState<string | null>(null)
  const [allowFromEnv, setAllowFromEnv] = useState(false)
  useEffect(() => {
    if (isTauri) void gatewayApi.allowIpsFromEnv().then(setAllowFromEnv).catch(() => {})
  }, [])
  const allowText = allowDraft ?? savedAllow

  return (
    <Modal
      title="局域网共享与客户端 Key"
      icon={<Network aria-hidden className="size-4 text-jolu-muted" />}
      width={620}
      busy={busy}
      onClose={onClose}
      footer={<Button label="关闭" disabled={busy} onClick={onClose} />}
    >
      {/* 待确认：Jolu.pen 未包含此弹窗，样式等鼠鼠确认后补画板。 */}
      <p className="text-[9.5px] text-jolu-muted">
        开启后本机监听 0.0.0.0，局域网设备用下面的客户端 Key 接入；观测模式永不共享。
      </p>

      {/* 来源地址白名单（P2-9，待确认：无画板） */}
      <div className="flex flex-col gap-1.5 rounded-lg border border-jolu-border bg-jolu-side px-3 py-2.5">
        <span className="text-[11.5px] font-medium text-jolu-text">允许连入的来源地址</span>
        <span className="text-[8.5px] text-jolu-muted">每行一个 IP 或网段（如 192.168.1.20、192.168.1.0/24）；留空 = 局域网内都可连。不在列表里的地址在看 Key 之前就被拒绝，本机始终可连。</span>
        {allowFromEnv && <span className="text-[8.5px] text-jolu-warn">已由环境变量 JOLU_ALLOW_IPS 指定，这里保存的不会生效。</span>}
        <textarea aria-label="允许连入的来源地址" rows={3} value={allowText} disabled={busy} spellCheck={false}
          onChange={(event) => { setAllowDraft(event.target.value); setAllowError(null) }}
          className="rounded-md border border-jolu-border bg-jolu-bg px-2 py-1.5 font-mono text-[11px] text-jolu-text outline-none focus:border-jolu-accent" />
        {allowError && <span className="text-[8.5px] text-jolu-danger">{allowError}</span>}
        {allowDraft !== null && allowDraft !== savedAllow && (
          <div className="flex items-center gap-2">
            <Button label="保存白名单" primary disabled={busy} onClick={async () => {
              try {
                await gatewayApi.allowIpsSet(allowDraft.split(/[\s,;]+/).filter(Boolean))
                setAllowDraft(null)
                toast.success("白名单已保存", "立即生效")
              } catch (reason) {
                setAllowError(String(reason))
              }
            }} />
            <Button label="撤销" disabled={busy} onClick={() => { setAllowDraft(null); setAllowError(null) }} />
          </div>
        )}
      </div>

      {/* 共享开关 */}
      <div className="flex flex-col gap-1.5 rounded-lg border border-jolu-border bg-jolu-side px-3 py-2.5">
        <label className="flex items-center gap-2.5">
          <input type="checkbox" checked={shared} disabled={busy} onChange={(event) => onSharedChange(event.target.checked)}
            className="size-4 accent-jolu-accent" />
          <span className="min-w-0 flex-1">
            <span className="block text-[11.5px] font-medium text-jolu-text">启用局域网共享</span>
            <span className="block text-[8.5px] text-jolu-muted">
              开启后鉴权切换为严格校验：只有下面的客户端 Key 能通过。绑定地址在下次开启接入时生效。
            </span>
          </span>
        </label>
        {danger && (
          <div className="flex items-start gap-1.5 rounded-md bg-jolu-status-success-bg px-2 py-1.5">
            <TriangleAlert aria-hidden className="mt-0.5 size-3 shrink-0 text-jolu-danger" />
            <span className="text-[8.5px] text-jolu-danger">
              已开启共享但还没有客户端 Key：局域网设备现在无法接入，请先签发一个。
            </span>
          </div>
        )}
      </div>

      {/* 签发表单 */}
      <div className="flex flex-col gap-1.5 rounded-lg border border-jolu-border px-3 py-2.5">
        <span className="text-[11.5px] font-medium text-jolu-text">签发客户端 Key</span>
        <div className="flex flex-wrap items-center gap-1.5">
          <input value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })}
            placeholder="名称，例如 书房台式机"
            className="h-8 min-w-[140px] flex-1 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[12px] text-jolu-text outline-none focus:border-jolu-accent" />
          <input value={form.requests} onChange={(event) => setForm({ ...form, requests: event.target.value })}
            inputMode="numeric" placeholder="每日请求上限"
            className="h-8 w-[118px] rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[12px] text-jolu-text outline-none focus:border-jolu-accent" />
          <input value={form.tokens} onChange={(event) => setForm({ ...form, tokens: event.target.value })}
            inputMode="numeric" placeholder="每日 Token 上限"
            className="h-8 w-[118px] rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[12px] text-jolu-text outline-none focus:border-jolu-accent" />
          <Button label={issuing ? "签发中…" : "签发"} primary disabled={busy || issuing || form.name.trim() === ""} onClick={() => void issue()} />
        </div>
        {/* 待确认：有效期 / 模型白名单 / 只读三项约束在 Jolu.pen 中同样没有画板 */}
        <div className="flex flex-wrap items-center gap-1.5">
          <select value={form.expiryDays ?? ""} aria-label="有效期"
            onChange={(event) => setForm({ ...form, expiryDays: event.target.value === "" ? null : Number(event.target.value) })}
            className="h-8 w-[104px] rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[12px] text-jolu-text outline-none focus:border-jolu-accent">
            {EXPIRY_CHOICES.map((choice) => (
              <option key={choice.label} value={choice.days ?? ""}>{choice.days === null ? choice.label : `有效 ${choice.label}`}</option>
            ))}
          </select>
          <input value={form.models} onChange={(event) => setForm({ ...form, models: event.target.value })}
            placeholder="模型白名单，逗号分隔，例如 zcode/GLM-5.2"
            className="h-8 min-w-[180px] flex-1 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[12px] text-jolu-text outline-none focus:border-jolu-accent" />
          <label className="flex h-8 items-center gap-1.5 text-[11.5px] text-jolu-text">
            <input type="checkbox" checked={form.readOnly} onChange={(event) => setForm({ ...form, readOnly: event.target.checked })}
              className="size-3.5 accent-jolu-accent" />
            只读
          </label>
        </div>
        <span className="text-[8.5px] text-jolu-muted">
          上限与白名单留空表示不限。只读 Key 只能查询（如读取模型列表），不能发起对话。{TOKENS_LIMIT_NOTICE}
        </span>
      </div>

      {/* 签发成功：Key 原文只此一次 */}
      {issued && (
        <div className="flex flex-col gap-1.5 rounded-lg border border-jolu-on bg-jolu-status-success-bg px-3 py-2.5">
          <span className="text-[11.5px] font-medium text-jolu-on">{issued.name} 的 Key</span>
          <code className="break-all rounded-md bg-jolu-bg px-2 py-1.5 font-mono text-[11.5px] text-jolu-text">{issued.key}</code>
          <div className="flex items-center gap-1.5">
            <Button label={copied ? "已复制" : "复制 Key"} primary onClick={() => void copyKey(issued.key)} />
            <Button label="我已保存" onClick={() => setIssued(null)} />
          </div>
          <span className="text-[8.5px] text-jolu-danger">{KEY_ONCE_NOTICE}</span>
        </div>
      )}

      {/* Key 列表 */}
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between">
          <span className="text-[11.5px] font-medium text-jolu-text">客户端 Key</span>
          <button type="button" onClick={() => void load()} disabled={loading || busy}
            className="text-[9.5px] text-jolu-accent outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-50">
            {loading ? "读取中…" : "刷新"}
          </button>
        </div>
        {error && (
          <div className="rounded-lg bg-jolu-status-error-bg px-2.5 py-2 text-[8.5px] text-jolu-danger">
            读取失败：{error}
          </div>
        )}
        {rows !== null && rows.length === 0 && !error && (
          <div className="rounded-lg border border-dashed border-jolu-border px-3 py-4 text-center text-[9.5px] text-jolu-muted">
            还没有客户端 Key。局域网设备接入前，先在上面签发一个。
          </div>
        )}
        {rows !== null && rows.length > 0 && (
          <div className="overflow-hidden rounded-lg border border-jolu-border">
            <div className="grid grid-cols-[minmax(0,1.4fr)_minmax(0,1.2fr)_72px_88px_96px_88px_56px] items-center gap-2 bg-jolu-side px-2.5 py-1.5 text-[8.5px] font-medium text-jolu-muted">
              <span>名称</span><span>Key</span><span title="超出的请求按到达顺序排队，不拒绝">同时请求</span><span>每日上限</span><span>今日用量</span><span>最近使用</span><span />
            </div>
            {rows.map((row) => (
              <Fragment key={row.name}>
              <div
                className="grid grid-cols-[minmax(0,1.4fr)_minmax(0,1.2fr)_72px_88px_96px_88px_56px] items-center gap-2 border-t border-jolu-border px-2.5 py-1.5 text-[11px]">
                <span className="flex min-w-0 flex-col">
                  <span className="truncate text-jolu-text" title={row.name}>{row.name}</span>
                  {constraintTags(row, Date.now()).length > 0 && (
                    <span className="flex flex-wrap gap-1" title={row.models?.join("、")}>
                      {constraintTags(row, Date.now()).map((tag) => (
                        <span key={tag.text} className={cn("text-[8.5px]", tag.danger ? "text-jolu-danger" : "text-jolu-muted")}>{tag.text}</span>
                      ))}
                    </span>
                  )}
                  {budgetTags(row).length > 0 && (
                    <span className="flex flex-wrap gap-1">
                      {budgetTags(row).map((tag) => <span key={tag} className="text-[8.5px] text-jolu-muted">{tag}</span>)}
                    </span>
                  )}
                  {spendLines(row).length > 0 && (
                    <span className="truncate text-[8.5px] text-jolu-muted" title="按 Key 出账：Token 与按价目估算的费用（未收录价目的模型不计费用）">
                      {spendLines(row).map((line) => `${line.label} ${line.tokens} · ${line.cost}`).join("　")}
                    </span>
                  )}
                </span>
                <code className="truncate font-mono text-[9.5px] text-jolu-muted" title={row.masked}>{row.masked}</code>
                {/* 待确认：同时请求数在 Jolu.pen 没有画板，用原生下拉做最小版 */}
                <select
                  aria-label={`${row.name} 的同时请求数`}
                  value={row.max_concurrency ?? 0}
                  disabled={busy}
                  onChange={async (event) => {
                    const cap = Number(event.target.value)
                    try {
                      await gatewayApi.setClientKeyConcurrency(row.name, cap > 0 ? cap : null)
                      await load()
                    } catch (reason) {
                      setError(String(reason))
                    }
                  }}
                  className="h-6 min-w-0 rounded-md border border-jolu-border bg-jolu-bg px-1 text-[9.5px] text-jolu-text outline-none focus:border-jolu-accent disabled:opacity-50"
                >
                  {concurrencyOptions(row.max_concurrency).map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                </select>
                <span className="text-jolu-text">{limitText(row.requests_per_day)}</span>
                <span className={cn("tnum", isExhausted(row) ? "text-jolu-danger" : "text-jolu-text")} title={isExhausted(row) ? "今日额度已用满，后续请求会被拒绝" : undefined}>
                  {usageText(row)}
                </span>
                <span className="truncate text-jolu-muted">{lastUsedText(row.last_used_ms, Date.now())}</span>
                <span className="flex flex-col items-end gap-0.5 justify-self-end">
                  <button type="button" disabled={busy} onClick={() => setEditing(editing?.name === row.name ? null : { name: row.name, form: budgetForm(row.budget), accounts: row.accounts ?? [], error: null })}
                    className="text-[9px] text-jolu-on outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-50">
                    限额
                  </button>
                  <button type="button" disabled={busy} onClick={() => setRevoking(row.name)}
                    className="text-[9px] text-jolu-danger outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-danger disabled:opacity-50">
                    撤销
                  </button>
                </span>
              </div>
              {editing?.name === row.name && (
                <div className="flex flex-col gap-2 border-t border-jolu-border bg-jolu-side px-2.5 py-2 text-[9.5px]" data-limits-editor>
                  <span className="text-jolu-muted">留空即不限。Token 上限请求前按估算预扣；费用上限按价目估算、已花达到后拒绝（未收录价目的模型不计入）。待确认：暂无设计稿。</span>
                  <div className="grid grid-cols-3 gap-2">
                    {BUDGET_FIELDS.map((field) => (
                      <label key={field.key} className="flex flex-col gap-0.5">
                        <span className="text-jolu-muted">{field.label}</span>
                        <input
                          aria-label={`${row.name} ${field.label}`}
                          inputMode={field.money ? "decimal" : "numeric"}
                          value={editing.form[field.key]}
                          onChange={(event) => setEditing({ ...editing, form: { ...editing.form, [field.key]: event.target.value }, error: null })}
                          className="h-7 rounded-md border border-jolu-border bg-jolu-bg px-2 text-[11px] text-jolu-text outline-none focus:border-jolu-accent"
                        />
                      </label>
                    ))}
                  </div>
                  <div className="flex flex-col gap-1">
                    <span className="text-jolu-muted">只能使用这些账号（都不勾 = 不限）</span>
                    <div className="flex flex-wrap gap-x-3 gap-y-1">
                      {connections.map((connection) => (
                        <label key={connection.id} className="flex items-center gap-1 text-jolu-text">
                          <input
                            type="checkbox"
                            checked={editing.accounts.includes(connection.id)}
                            onChange={(event) => setEditing({
                              ...editing,
                              accounts: event.target.checked ? [...editing.accounts, connection.id] : editing.accounts.filter((id) => id !== connection.id),
                            })}
                          />
                          {connection.name}
                        </label>
                      ))}
                      {connections.length === 0 && <span className="text-jolu-muted">还没有连接</span>}
                    </div>
                  </div>
                  {editing.error && <span className="text-jolu-danger">{editing.error}</span>}
                  <div className="flex items-center gap-2">
                    <Button label="保存限额" primary disabled={busy} onClick={async () => {
                      const parsed = budgetFromForm(editing.form)
                      if (parsed.error !== undefined) { setEditing({ ...editing, error: parsed.error }); return }
                      try {
                        await gatewayApi.setClientKeyLimits(row.name, parsed.budget, editing.accounts.length ? editing.accounts : null)
                        setEditing(null)
                        toast.success("限额已保存", row.name)
                        await load()
                      } catch (reason) {
                        setEditing({ ...editing, error: String(reason) })
                      }
                    }} />
                    <Button label="取消" disabled={busy} onClick={() => setEditing(null)} />
                  </div>
                </div>
              )}
              </Fragment>
            ))}
          </div>
        )}
      </div>

      {revoking && (
        <ConfirmDialog
          title={`撤销「${revoking}」？`}
          description="删除后这个 Key 立即失效，使用它的设备会收到 401；历史路由流水的归属保留。"
          confirmLabel="撤销"
          danger
          onClose={() => setRevoking(null)}
          onConfirm={async () => {
            const name = revoking
            setRevoking(null)
            await run(async () => {
              await gatewayApi.revokeClientKey(name)
              await load()
            }, `已撤销「${name}」`)
          }}
        />
      )}
    </Modal>
  )
}
