import { useEffect, useState } from "react"
import { ArrowDown, ArrowUp, Layers } from "lucide-react"
import { cn } from "@/lib/utils"
import { gatewayApi, type MiddlewareDto } from "@/lib/gatewayApi"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { useToast } from "@/components/ui/Toast"
import { Button, Modal } from "./GatewayDialogs"
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
  "rounded-lg border border-jolu-border bg-jolu-bg px-2 py-1.5 text-[12px] text-jolu-text outline-none focus:border-jolu-accent"

/**
 * 网关中间件（待确认：Jolu.pen 没有这个弹窗，外壳与行布局沿用「局域网共享」弹窗）。
 *
 * 选路之前按顺序执行；单个中间件出错只跳过自己。保存时后端先在样例请求上试跑，
 * 跑不通的配置不会写进库，避免「开了但从没生效」。
 */
export function MiddlewareDialog({ busy, onClose, run, inline = false }: {
  /** 内嵌进模型路由页「中间件」检查器（R2 / V5） */
  inline?: boolean
  busy: boolean
  onClose: () => void
  run: (work: () => Promise<unknown>, successTitle?: string) => Promise<boolean>
}) {
  const toast = useToast()
  const [rows, setRows] = useState<MiddlewareDto[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [editing, setEditing] = useState<{ original: string | null; name: string; kind: MiddlewareKind; fields: MiddlewareFields } | null>(null)
  const [removing, setRemoving] = useState<string | null>(null)

  const load = async () => {
    setError(null)
    try {
      setRows(await gatewayApi.middlewares())
    } catch (reason) {
      setError(String(reason))
    }
  }
  useEffect(() => { void load() }, [])

  const startNew = () => setEditing({ original: null, name: "", kind: "model-map", fields: emptyFields() })
  const startEdit = (row: MiddlewareDto) =>
    setEditing({ original: row.name, name: row.name, kind: row.kind as MiddlewareKind, fields: fromConfig(row.kind as MiddlewareKind, row.config) })

  const saveEditing = async () => {
    if (!editing) return
    const name = editing.name.trim()
    if (!name) return toast.danger("请填写名称", "名称用于区分不同的中间件")
    const parsed = toConfig(editing.kind, editing.fields)
    if (!parsed.ok) return toast.danger("配置不完整", parsed.error)
    const current = rows?.find((row) => row.name === editing.original)
    const sort = current?.sort ?? (rows?.length ?? 0)
    const ok = await run(async () => {
      await gatewayApi.saveMiddleware({ name, kind: editing.kind, config: parsed.config, enabled: current?.enabled ?? true, sort })
      // 改名 = 新建一条再删旧的（后端按名称幂等）
      if (editing.original && editing.original !== name) await gatewayApi.deleteMiddleware(editing.original)
      await load()
    }, editing.original ? "已保存中间件" : "已添加中间件")
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
    <Modal title="网关中间件" icon={<Layers aria-hidden className="size-4 text-jolu-muted" />} width={620} busy={busy} onClose={onClose} inline={inline}
      footer={<><Button label="关闭" disabled={busy} onClick={onClose} />{!editing && <Button label="添加中间件" primary disabled={busy} onClick={startNew} />}</>}>
      <p className="text-[9.5px] text-jolu-muted">
        选路之前按从上到下的顺序改写请求；某一条出错只跳过它，请求照常发出。隐私脱敏最先执行，中间件其次。
      </p>

      {error && <div className="rounded-lg bg-jolu-status-error-bg px-2.5 py-2 text-[8.5px] text-jolu-danger">读取失败：{error}</div>}
      {rows !== null && rows.length === 0 && !editing && (
        <div className="rounded-lg border border-dashed border-jolu-border px-3 py-4 text-center text-[9.5px] text-jolu-muted">
          还没有中间件。添加后，下一个经过统一网关的请求即生效。
        </div>
      )}

      {ordered.length > 0 && (
        <div className="overflow-hidden rounded-lg border border-jolu-border">
          {ordered.map((row, index) => (
            <div key={row.name} className={cn("flex items-center gap-2 px-2.5 py-2 text-[11px]", index > 0 && "border-t border-jolu-border")}>
              <input type="checkbox" checked={row.enabled} disabled={busy} onChange={() => toggle(row)} aria-label={`启用 ${row.name}`}
                className="size-3.5 shrink-0 accent-jolu-accent" />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className={cn("truncate", row.enabled ? "text-jolu-text" : "text-jolu-muted")}>{row.name}</span>
                <span className="truncate text-[8.5px] text-jolu-muted">{kindOf(row.kind)?.label ?? row.kind} · {summary(row.kind, row.config)}</span>
              </span>
              <button type="button" disabled={busy || index === 0} onClick={() => move(row, -1)} aria-label="上移"
                className="rounded p-1 text-jolu-muted outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40">
                <ArrowUp className="size-3" />
              </button>
              <button type="button" disabled={busy || index === ordered.length - 1} onClick={() => move(row, 1)} aria-label="下移"
                className="rounded p-1 text-jolu-muted outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40">
                <ArrowDown className="size-3" />
              </button>
              <button type="button" disabled={busy} onClick={() => startEdit(row)}
                className="text-[9px] text-jolu-accent outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-50">编辑</button>
              <button type="button" disabled={busy} onClick={() => setRemoving(row.name)}
                className="text-[9px] text-jolu-danger outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-danger disabled:opacity-50">删除</button>
            </div>
          ))}
        </div>
      )}

      {editing && (
        <div className="flex flex-col gap-2 rounded-lg border border-jolu-border px-3 py-2.5">
          <span className="text-[11.5px] font-medium text-jolu-text">{editing.original ? `编辑「${editing.original}」` : "添加中间件"}</span>
          <div className="flex flex-wrap items-center gap-1.5">
            <input value={editing.name} onChange={(event) => setEditing({ ...editing, name: event.target.value })} placeholder="名称，例如 快模型改名"
              className={cn(fieldClass, "h-8 min-w-[160px] flex-1")} />
            <select value={editing.kind} aria-label="类型" onChange={(event) => setEditing({ ...editing, kind: event.target.value as MiddlewareKind, fields: emptyFields() })}
              className={cn(fieldClass, "h-8 w-[140px]")}>
              {MIDDLEWARE_KINDS.map((kind) => <option key={kind.id} value={kind.id}>{kind.label}</option>)}
            </select>
          </div>
          <span className="text-[8.5px] text-jolu-muted">{kindOf(editing.kind)?.hint}</span>
          <KindFields kind={editing.kind} fields={editing.fields} onChange={(fields) => setEditing({ ...editing, fields })} />
          <div className="flex items-center gap-1.5">
            <Button label="保存" primary disabled={busy} onClick={() => void saveEditing()} />
            <Button label="取消" disabled={busy} onClick={() => setEditing(null)} />
          </div>
        </div>
      )}

      {removing && (
        <ConfirmDialog title={`删除「${removing}」？`} description="删除后下一个请求起不再执行这个中间件。" confirmLabel="删除" danger
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
    </Modal>
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
            className={cn(fieldClass, "h-8 w-full")} />
        </>
      )
    case "system-prompt":
      return area("prompt", "例如：始终用中文回答", 4)
    case "word-guard":
      return (
        <>
          <input value={fields.block} onChange={(event) => onChange({ ...fields, block: event.target.value })} placeholder="拦截词，逗号分隔：命中整条请求拒发"
            className={cn(fieldClass, "h-8 w-full")} />
          <input value={fields.mask} onChange={(event) => onChange({ ...fields, mask: event.target.value })} placeholder="遮盖词，逗号分隔：替换成 ***"
            className={cn(fieldClass, "h-8 w-full")} />
        </>
      )
    case "think-tags":
      return (
        <select value={fields.mode} aria-label="处理方式" onChange={(event) => onChange({ ...fields, mode: event.target.value as "strip" | "wrap" })}
          className={cn(fieldClass, "h-8 w-[180px]")}>
          <option value="strip">剥掉思考段</option>
          <option value="wrap">包进思考标签</option>
        </select>
      )
    case "js-hook":
      return (
        <>
          <input value={fields.path} onChange={(event) => onChange({ ...fields, path: event.target.value })} placeholder="钩子脚本完整路径，例如 D:\hooks\context.hooks.mjs"
            className={cn(fieldClass, "h-8 w-full")} />
          <input value={fields.timeout} onChange={(event) => onChange({ ...fields, timeout: event.target.value })} placeholder="超时毫秒（可空，默认 1000，范围 100–30000）"
            className={cn(fieldClass, "h-8 w-[260px]")} />
        </>
      )
  }
}
