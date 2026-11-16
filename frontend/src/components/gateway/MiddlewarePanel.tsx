import { useEffect, useState } from "react"
import { ArrowDown, ArrowUp, FlaskConical, Pencil, ShieldCheck, Sparkles, Target, Send } from "lucide-react"
import { cn } from "@/lib/utils"
import { gatewayApi, type MiddlewareDto } from "@/lib/gatewayApi"
import { useToast } from "@/components/ui/Toast"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { SelectField } from "@/components/ui/SelectField"
import { Switch } from "@/components/agents/AgentList"
import { useLatestGate } from "@/hooks/useLatestGate"
import {
  MIDDLEWARE_KINDS,
  emptyFields,
  fromConfig,
  reorder,
  summary,
  toConfig,
  type MiddlewareFields,
  type MiddlewareKind,
} from "./middlewareForm"

const fieldClass =
  "rounded-lg border border-jolu-border bg-jolu-bg px-2 py-1.5 text-meta text-jolu-text outline-none focus:border-jolu-accent"

/** 每类规则的卡片图标（无 V6 画板细节，按语义选型，标「待确认」）。 */
const KIND_ICONS: Record<MiddlewareKind, React.ElementType> = {
  "model-map": Sparkles,
  "param-override": Pencil,
  "system-prompt": Sparkles,
  "word-guard": ShieldCheck,
  "think-tags": Pencil,
  "js-hook": Pencil,
}

/**
 * 模型路由 · 中间件（Jolu.pen「统一网关 · R2 重设计」V6 画板 wEsPa）：
 * 流程链卡（隐私脱敏 → 中间件处理 → 选择模型 → 发送请求）+ 规则卡列表
 * （序号、名称、类型 badge、摘要、上下移、启用、编辑、更多）+「保存前自动试跑」说明卡。
 * 规则的编辑表单画板未给出，沿用旧交互就地展开，标「待确认」。
 */
export function MiddlewarePanel({ busy, run, onOpenPrivacy }: {
  busy: boolean
  run: (work: () => Promise<unknown>, successTitle?: string) => Promise<boolean>
  /** 跳到设置页的隐私脱敏分区 */
  onOpenPrivacy: () => void
}) {
  const toast = useToast()
  const [rows, setRows] = useState<MiddlewareDto[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [editing, setEditing] = useState<{ original: string | null; name: string; kind: MiddlewareKind; fields: MiddlewareFields } | null>(null)
  const [removing, setRemoving] = useState<string | null>(null)

  // 挂载与每次操作之后都会调 load：只认最新一次的结果（见 lib/latestGate.ts）
  const loadGate = useLatestGate()
  const load = async () => {
    const isLatest = loadGate.begin()
    setError(null)
    try {
      const next = await gatewayApi.middlewares()
      if (isLatest()) setRows(next)
    } catch (reason) {
      if (isLatest()) setError(String(reason))
    }
  }
  useEffect(() => { void load() }, [])

  const startNew = () => setEditing({ original: null, name: "", kind: "model-map", fields: emptyFields() })
  const startEdit = (row: MiddlewareDto) =>
    setEditing({ original: row.name, name: row.name, kind: row.kind as MiddlewareKind, fields: fromConfig(row.kind as MiddlewareKind, row.config) })

  const saveEditing = async () => {
    if (!editing) return
    const name = editing.name.trim()
    if (!name) return toast.danger("请填写名称", "名称用于区分不同的规则")
    const parsed = toConfig(editing.kind, editing.fields)
    if (!parsed.ok) return toast.danger("配置不完整", parsed.error)
    const current = rows?.find((row) => row.name === editing.original)
    const sort = current?.sort ?? (rows?.length ?? 0)
    const ok = await run(async () => {
      await gatewayApi.saveMiddleware({ name, kind: editing.kind, config: parsed.config, enabled: current?.enabled ?? true, sort })
      // 改名 = 新建一条再删旧的（后端按名称幂等）
      if (editing.original && editing.original !== name) await gatewayApi.deleteMiddleware(editing.original)
      await load()
    }, editing.original ? "已保存中间件" : "已添加规则")
    if (ok) setEditing(null)
  }

  const toggle = (row: MiddlewareDto) =>
    void run(async () => {
      await gatewayApi.saveMiddleware({ ...row, enabled: !row.enabled })
      await load()
    }, row.enabled ? `已停用「${row.name}」` : `已启用「${row.name}」`)

  const move = (row: MiddlewareDto, delta: -1 | 1) => {
    if (!rows) return
    const next = reorder(rows, row.name, delta)
    const changed = next.filter((item) => rows.find((old) => old.name === item.name)?.sort !== item.sort)
    if (changed.length === 0) return
    void run(async () => {
      for (const item of changed) await gatewayApi.saveMiddleware(item)
      await load()
    }, "已调整顺序")
  }

  const ordered = rows ? [...rows].sort((a, b) => a.sort - b.sort) : []
  const kindOf = (id: string) => MIDDLEWARE_KINDS.find((kind) => kind.id === id)

  return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 px-8 pb-8 pt-2">
      <div className="flex items-start justify-between gap-4">
        <div className="flex flex-col gap-1.5">
          <h2 className="text-[20px] font-semibold text-jolu-text">按你的规则处理请求</h2>
          <p className="text-meta text-jolu-muted">规则按顺序执行；每一项都可以独立启用、编辑和调整顺序。</p>
        </div>
        <button type="button" disabled={busy} onClick={startNew}
          className="flex h-10 shrink-0 items-center gap-1.5 rounded-lg bg-jolu-text px-4 text-meta font-semibold text-jolu-bg outline-none hover:opacity-90 focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
          <PlusIcon />添加规则
        </button>
      </div>

      <section className="flex flex-wrap items-center gap-2 rounded-xl border border-jolu-border bg-jolu-side px-4 py-3">
        <FlowChip icon={<ShieldCheck aria-hidden className="size-3.5" />} label="隐私脱敏" onClick={onOpenPrivacy} />
        <FlowArrow />
        <FlowChip icon={<Sparkles aria-hidden className="size-3.5" />} label="中间件处理" active />
        <FlowArrow />
        <FlowChip icon={<Target aria-hidden className="size-3.5" />} label="选择模型" />
        <FlowArrow />
        <FlowChip icon={<Send aria-hidden className="size-3.5" />} label="发送请求" />
        <span className="flex-1" />
        <button type="button" onClick={onOpenPrivacy} className="text-meta font-medium text-jolu-on outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">
          管理隐私设置 ↗
        </button>
      </section>

      {error && <div className="rounded-xl bg-jolu-status-error-bg px-4 py-2.5 text-meta text-jolu-danger">读取失败：{error}</div>}
      {rows !== null && rows.length === 0 && !editing && (
        <div className="rounded-xl border border-dashed border-jolu-border px-4 py-8 text-center text-meta text-jolu-muted">
          还没有规则。点「添加规则」后，下一个经过模型路由的请求即生效。
        </div>
      )}

      <div className="flex flex-col gap-3">
        {ordered.map((row, index) => {
          const kind = kindOf(row.kind)
          const Icon = KIND_ICONS[row.kind as MiddlewareKind] ?? Sparkles
          const editingRow = editing?.original === row.name
          return (
            <section key={row.name} className="flex flex-col gap-3 rounded-xl border border-jolu-border px-4 py-3.5">
              <div className="flex items-center gap-3.5">
                <span className="w-7 shrink-0 text-note tabular-nums text-jolu-muted">{String(index + 1).padStart(2, "0")}</span>
                <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-jolu-side"><Icon aria-hidden className="size-4 text-jolu-muted" /></span>
                <span className="flex min-w-0 items-center gap-2.5">
                  <span className={cn("truncate text-[15px] font-semibold", row.enabled ? "text-jolu-text" : "text-jolu-muted")}>{row.name}</span>
                  <span className="shrink-0 rounded-md bg-jolu-track px-2 py-0.5 text-note text-jolu-muted">{kind?.label ?? row.kind}</span>
                </span>
                <span className="hidden min-w-0 flex-1 truncate text-meta text-jolu-muted md:block" title={summary(row.kind, row.config)}>{summary(row.kind, row.config)}</span>
                <span className="flex shrink-0 items-center gap-1">
                  <button type="button" aria-label={`上移 ${row.name}`} disabled={busy || index === 0} onClick={() => move(row, -1)}
                    className="grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40">
                    <ArrowUp aria-hidden className="size-3.5" />
                  </button>
                  <button type="button" aria-label={`下移 ${row.name}`} disabled={busy || index === ordered.length - 1} onClick={() => move(row, 1)}
                    className="grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40">
                    <ArrowDown aria-hidden className="size-3.5" />
                  </button>
                </span>
                <Switch on={row.enabled} disabled={busy} label={`${row.enabled ? "停用" : "启用"} ${row.name}`} onClick={() => toggle(row)} />
                <button type="button" disabled={busy} onClick={() => (editingRow ? setEditing(null) : startEdit(row))}
                  className={cn(
                    "flex h-9 shrink-0 items-center gap-1.5 rounded-lg border px-3.5 text-meta font-medium outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45",
                    editingRow ? "border-jolu-accent bg-jolu-selected text-jolu-on" : "border-jolu-border text-jolu-text hover:bg-jolu-side",
                  )}>
                  <Pencil aria-hidden className="size-3.5 text-jolu-muted" />编辑
                </button>
                <button type="button" aria-label={`更多操作 ${row.name}`} disabled={busy} onClick={() => setRemoving(row.name)}
                  className="grid size-9 shrink-0 place-items-center rounded-lg border border-jolu-border outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
                  <EllipsisDot />
                </button>
              </div>
              <span className="min-w-0 truncate pl-[54px] text-meta text-jolu-muted md:hidden">{summary(row.kind, row.config)}</span>
              {editingRow && editing && (
                <div className="flex flex-col gap-2.5 rounded-lg border border-jolu-border bg-jolu-side px-3.5 py-3" data-rule-editor>
                  <div className="flex flex-wrap items-center gap-2">
                    <input value={editing.name} onChange={(event) => setEditing({ ...editing, name: event.target.value })} placeholder="名称，例如 快模型改名" aria-label="规则名称"
                      className={cn(fieldClass, "h-9 min-w-[160px] flex-1")} />
                    <SelectField label="规则类型" value={editing.kind}
                      onValueChange={(value) => setEditing({ ...editing, kind: value as MiddlewareKind, fields: emptyFields() })}
                      className="h-9 w-[150px] rounded-[10px]"
                      options={MIDDLEWARE_KINDS.map((item) => ({ value: item.id, label: item.label, description: item.hint }))} />
                  </div>
                  <span className="text-note text-jolu-muted">{kindOf(editing.kind)?.hint}</span>
                  <KindFields kind={editing.kind} fields={editing.fields} onChange={(fields) => setEditing({ ...editing, fields })} />
                  <div className="flex items-center gap-2">
                    <button type="button" disabled={busy || !editing.name.trim()} onClick={() => void saveEditing()}
                      className="h-8 rounded-lg bg-jolu-accent px-3.5 text-meta font-semibold text-[#071B10] disabled:opacity-45">保存</button>
                    <button type="button" disabled={busy} onClick={() => setEditing(null)} className="text-meta text-jolu-muted hover:text-jolu-text">取消</button>
                  </div>
                </div>
              )}
            </section>
          )
        })}
      </div>

      {editing && editing.original === null && (
        <div className="flex flex-col gap-2.5 rounded-xl border border-jolu-border px-4 py-3.5" data-new-rule>
          <span className="text-[15px] font-semibold text-jolu-text">新规则</span>
          <div className="flex flex-wrap items-center gap-2">
            <input autoFocus value={editing.name} onChange={(event) => setEditing({ ...editing, name: event.target.value })} placeholder="名称，例如 快模型改名" aria-label="规则名称"
              className={cn(fieldClass, "h-9 min-w-[160px] flex-1")} />
            <SelectField label="规则类型" value={editing.kind}
              onValueChange={(value) => setEditing({ ...editing, kind: value as MiddlewareKind, fields: emptyFields() })}
              className="h-9 w-[150px] rounded-[10px]"
              options={MIDDLEWARE_KINDS.map((item) => ({ value: item.id, label: item.label, description: item.hint }))} />
          </div>
          <span className="text-note text-jolu-muted">{kindOf(editing.kind)?.hint}</span>
          <KindFields kind={editing.kind} fields={editing.fields} onChange={(fields) => setEditing({ ...editing, fields })} />
          <div className="flex items-center gap-2">
            <button type="button" disabled={busy || !editing.name.trim()} onClick={() => void saveEditing()}
              className="h-8 rounded-lg bg-jolu-accent px-3.5 text-meta font-semibold text-[#071B10] disabled:opacity-45">添加</button>
            <button type="button" disabled={busy} onClick={() => setEditing(null)} className="text-meta text-jolu-muted hover:text-jolu-text">取消</button>
          </div>
        </div>
      )}

      <section className="flex items-start gap-3 rounded-xl border border-jolu-border bg-jolu-side px-5 py-4">
        <FlaskConical aria-hidden className="mt-0.5 size-4 shrink-0 text-jolu-muted" />
        <span className="flex flex-col gap-1">
          <span className="text-[15px] font-semibold text-jolu-text">保存前自动试跑</span>
          <span className="text-meta text-jolu-muted">配置通过样例请求校验后才会保存。运行时某项出错会跳过该项；命中拦截规则则拒绝请求。</span>
        </span>
      </section>

      <p className="text-note text-jolu-muted">可添加：{MIDDLEWARE_KINDS.map((kind) => kind.label).join("、")}</p>

      {removing && (
        <ConfirmDialog title={`删除「${removing}」？`} description="删除后下一个请求起不再执行这个规则。" confirmLabel="删除" danger
          onClose={() => setRemoving(null)}
          onConfirm={async () => {
            const name = removing
            setRemoving(null)
            await run(async () => {
              await gatewayApi.deleteMiddleware(name)
              await load()
            }, `已删除「${name}」`)
          }} />
      )}
    </div>
  )
}

function KindFields({ kind, fields, onChange }: { kind: MiddlewareKind; fields: MiddlewareFields; onChange: (fields: MiddlewareFields) => void }) {
  const area = (key: keyof MiddlewareFields, placeholder: string, rows = 3) => (
    <textarea value={fields[key]} rows={rows} placeholder={placeholder} onChange={(event) => onChange({ ...fields, [key]: event.target.value })}
      className={cn(fieldClass, "w-full resize-y font-mono")} />
  )
  switch (kind) {
    case "model-map":
      return area("mapping", "每行一条：请求名 = 来源/模型\n例如 fast = zcode/GLM-5.2")
    case "param-override":
      return (
        <>
          {area("set", "每行一项：字段 = 值（数字、布尔或 JSON 原样写）\n例如 max_tokens = 1024")}
          <input value={fields.remove} onChange={(event) => onChange({ ...fields, remove: event.target.value })} placeholder="要删除的字段，逗号分隔，例如 temperature, top_p"
            className={cn(fieldClass, "h-9 w-full")} />
        </>
      )
    case "system-prompt":
      return area("prompt", "例如：始终用中文回答", 4)
    case "word-guard":
      return (
        <>
          <input value={fields.block} onChange={(event) => onChange({ ...fields, block: event.target.value })} placeholder="拦截词，逗号分隔：命中整条请求拒发"
            className={cn(fieldClass, "h-9 w-full")} />
          <input value={fields.mask} onChange={(event) => onChange({ ...fields, mask: event.target.value })} placeholder="遮盖词，逗号分隔：替换成 ***"
            className={cn(fieldClass, "h-9 w-full")} />
        </>
      )
    case "think-tags":
      return (
        <SelectField label="思考段处理方式" value={fields.mode}
          onValueChange={(value) => onChange({ ...fields, mode: value as "strip" | "wrap" })}
          className="h-9 w-[200px] rounded-[10px]"
          options={[{ value: "strip", label: "剥掉思考段" }, { value: "wrap", label: "包进思考标签" }]} />
      )
    case "js-hook":
      return (
        <>
          <input value={fields.path} onChange={(event) => onChange({ ...fields, path: event.target.value })} placeholder="钩子脚本完整路径，例如 D:\hooks\context.hooks.mjs"
            className={cn(fieldClass, "h-9 w-full")} />
          <input value={fields.timeout} onChange={(event) => onChange({ ...fields, timeout: event.target.value })} placeholder="超时毫秒（可空，默认 1000，范围 100–30000）"
            className={cn(fieldClass, "h-9 w-[280px]")} />
        </>
      )
  }
}

function FlowChip({ icon, label, active, onClick }: { icon: React.ReactNode; label: string; active?: boolean; onClick?: () => void }) {
  return (
    <button type="button" onClick={onClick} disabled={!onClick} aria-current={active ? "step" : undefined}
      className={cn(
        "flex h-9 items-center gap-2 rounded-lg px-3.5 text-meta font-medium outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
        active ? "bg-jolu-selected text-jolu-on" : "border border-jolu-border bg-jolu-bg text-jolu-text hover:bg-jolu-side",
      )}>
      {icon}
      {label}
    </button>
  )
}

function FlowArrow() {
  return (
    <svg aria-hidden viewBox="0 0 14 14" className="size-3.5 shrink-0 text-jolu-border" fill="none" stroke="currentColor" strokeWidth="1.5">
      <path d="M3 7h8M8 4l3 3-3 3" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}

function PlusIcon() {
  return (
    <svg aria-hidden viewBox="0 0 14 14" className="size-3.5" fill="none" stroke="currentColor" strokeWidth="1.6">
      <path d="M7 2.5v9M2.5 7h9" strokeLinecap="round" />
    </svg>
  )
}

function EllipsisDot() {
  return (
    <svg aria-hidden viewBox="0 0 16 16" className="size-4 text-jolu-muted" fill="currentColor">
      <circle cx="3.2" cy="8" r="1.4" /><circle cx="8" cy="8" r="1.4" /><circle cx="12.8" cy="8" r="1.4" />
    </svg>
  )
}
