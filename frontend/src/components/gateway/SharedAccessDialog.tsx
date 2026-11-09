import { useState } from "react"
import { Network, TriangleAlert } from "lucide-react"
import { cn } from "@/lib/utils"
import { gatewayApi, type ClientKeySummaryDto, type IssuedClientKeyDto } from "@/lib/gatewayApi"
import { useToast } from "@/components/ui/Toast"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { Button, Modal } from "./GatewayDialogs"
import {
  EXPIRY_CHOICES,
  KEY_ONCE_NOTICE,
  TOKENS_LIMIT_NOTICE,
  constraintTags,
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

      {/* 共享开关 */}
      <div className="flex flex-col gap-1.5 rounded-lg border border-jolu-border bg-jolu-side px-3 py-2.5">
        <label className="flex items-center gap-2.5">
          <input type="checkbox" checked={shared} disabled={busy} onChange={(event) => onSharedChange(event.target.checked)}
            className="size-4 accent-jolu-accent" />
          <span className="min-w-0 flex-1">
            <span className="block text-[10.5px] font-medium text-jolu-text">启用局域网共享</span>
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
        <span className="text-[10.5px] font-medium text-jolu-text">签发客户端 Key</span>
        <div className="flex flex-wrap items-center gap-1.5">
          <input value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })}
            placeholder="名称，例如 书房台式机"
            className="h-8 min-w-[140px] flex-1 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[11px] text-jolu-text outline-none focus:border-jolu-accent" />
          <input value={form.requests} onChange={(event) => setForm({ ...form, requests: event.target.value })}
            inputMode="numeric" placeholder="每日请求上限"
            className="h-8 w-[118px] rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[11px] text-jolu-text outline-none focus:border-jolu-accent" />
          <input value={form.tokens} onChange={(event) => setForm({ ...form, tokens: event.target.value })}
            inputMode="numeric" placeholder="每日 Token 上限"
            className="h-8 w-[118px] rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[11px] text-jolu-text outline-none focus:border-jolu-accent" />
          <Button label={issuing ? "签发中…" : "签发"} primary disabled={busy || issuing || form.name.trim() === ""} onClick={() => void issue()} />
        </div>
        {/* 待确认：有效期 / 模型白名单 / 只读三项约束在 Jolu.pen 中同样没有画板 */}
        <div className="flex flex-wrap items-center gap-1.5">
          <select value={form.expiryDays ?? ""} aria-label="有效期"
            onChange={(event) => setForm({ ...form, expiryDays: event.target.value === "" ? null : Number(event.target.value) })}
            className="h-8 w-[104px] rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[11px] text-jolu-text outline-none focus:border-jolu-accent">
            {EXPIRY_CHOICES.map((choice) => (
              <option key={choice.label} value={choice.days ?? ""}>{choice.days === null ? choice.label : `有效 ${choice.label}`}</option>
            ))}
          </select>
          <input value={form.models} onChange={(event) => setForm({ ...form, models: event.target.value })}
            placeholder="模型白名单，逗号分隔，例如 zcode/GLM-5.2"
            className="h-8 min-w-[180px] flex-1 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[11px] text-jolu-text outline-none focus:border-jolu-accent" />
          <label className="flex h-8 items-center gap-1.5 text-[10.5px] text-jolu-text">
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
          <span className="text-[10.5px] font-medium text-jolu-on">{issued.name} 的 Key</span>
          <code className="break-all rounded-md bg-jolu-bg px-2 py-1.5 font-mono text-[10.5px] text-jolu-text">{issued.key}</code>
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
          <span className="text-[10.5px] font-medium text-jolu-text">客户端 Key</span>
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
            <div className="grid grid-cols-[minmax(0,1.4fr)_minmax(0,1.2fr)_88px_96px_88px_56px] items-center gap-2 bg-jolu-side px-2.5 py-1.5 text-[8.5px] font-medium text-jolu-muted">
              <span>名称</span><span>Key</span><span>每日上限</span><span>今日用量</span><span>最近使用</span><span />
            </div>
            {rows.map((row) => (
              <div key={row.name}
                className="grid grid-cols-[minmax(0,1.4fr)_minmax(0,1.2fr)_88px_96px_88px_56px] items-center gap-2 border-t border-jolu-border px-2.5 py-1.5 text-[10px]">
                <span className="flex min-w-0 flex-col">
                  <span className="truncate text-jolu-text" title={row.name}>{row.name}</span>
                  {constraintTags(row, Date.now()).length > 0 && (
                    <span className="flex flex-wrap gap-1" title={row.models?.join("、")}>
                      {constraintTags(row, Date.now()).map((tag) => (
                        <span key={tag.text} className={cn("text-[8.5px]", tag.danger ? "text-jolu-danger" : "text-jolu-muted")}>{tag.text}</span>
                      ))}
                    </span>
                  )}
                </span>
                <code className="truncate font-mono text-[9.5px] text-jolu-muted" title={row.masked}>{row.masked}</code>
                <span className="text-jolu-text">{limitText(row.requests_per_day)}</span>
                <span className={cn("tnum", isExhausted(row) ? "text-jolu-danger" : "text-jolu-text")} title={isExhausted(row) ? "今日额度已用满，后续请求会被拒绝" : undefined}>
                  {usageText(row)}
                </span>
                <span className="truncate text-jolu-muted">{lastUsedText(row.last_used_ms, Date.now())}</span>
                <button type="button" disabled={busy} onClick={() => setRevoking(row.name)}
                  className="justify-self-end text-[9px] text-jolu-danger outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-danger disabled:opacity-50">
                  撤销
                </button>
              </div>
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
