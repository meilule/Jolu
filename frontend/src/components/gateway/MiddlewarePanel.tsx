import { useEffect, useState } from "react"
import { cn } from "@/lib/utils"
import { gatewayApi, type MiddlewareDto } from "@/lib/gatewayApi"
import { useToast } from "@/components/ui/Toast"
import { useLatestGate } from "@/hooks/useLatestGate"
import { Icon, type IconName } from "@/icons"
import { Button, ConfirmDialog, Dialog, FormField, IconButton, Input, Select, Switch, Tag } from "@/ui"
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
  "rounded-[14px] border border-transparent bg-jn-well px-3 py-2 text-[13px] text-jn-text outline-none focus:border-jn-primary focus:bg-jn-surface"

/** 每类规则的卡片图标（按语义选型；R1-22 画板只给了几类示例）。 */
const KIND_ICONS: Record<MiddlewareKind, IconName> = {
  "model-map": "blocks",
  "param-override": "edit",
  "system-prompt": "bot",
  "word-guard": "shield",
  "think-tags": "activity",
  "js-hook": "navSettings",
}

/**
 * 模型路由 · 中间件 —— 设计稿 R1-22（浅色）。
 *
 * 结构：流程链卡（隐私脱敏 → 中间件规则 · N 条 → 选择模型 → 发送请求）+
 * 规则卡列表（序号、图标、名称 / 类型、摘要、上下移、开关、编辑、删除）。
 * 「添加规则」在页头（R1-22 的「页面操作」槽），通过 `addSignal` 通知本面板打开新建表单。
 *
 * 规则的编辑表单 R1-35 画成弹窗；本期先把内容与交互整理到新令牌与新组件，
 * 打开方式仍沿用就地表单（标「待确认」——改成弹窗需要先把 form 状态提出来）。
 */
export function MiddlewarePanel({ busy, addSignal = 0, run, onOpenPrivacy }: {
  busy: boolean
  /** 页头「添加规则」的序号；每次 +1 打开新建表单 */
  addSignal?: number
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

  const startEdit = (row: MiddlewareDto) =>
    setEditing({ original: row.name, name: row.name, kind: row.kind as MiddlewareKind, fields: fromConfig(row.kind as MiddlewareKind, row.config) })

  // 页头「添加规则」：序号变化即打开新建表单
  useEffect(() => {
    if (addSignal > 0) setEditing({ original: null, name: "", kind: "model-map", fields: emptyFields() })
  }, [addSignal])

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
  const activeCount = ordered.filter((row) => row.enabled).length

  return (
    <div className="flex min-w-0 flex-col gap-3.5">
      {/* 流程链（R1-22）：隐私脱敏 → 中间件规则 → 选择模型 → 发送请求 */}
      <section className="flex flex-wrap items-center gap-2 rounded-jn-card border border-jn-line bg-jn-surface px-4 py-3 shadow-jn-surface">
        <FlowChip icon="shield" label="隐私脱敏" onClick={onOpenPrivacy} />
        <Icon name="next" size="sm" className="shrink-0 text-jn-text-3" />
        <FlowChip icon="navSettings" label={`中间件规则 · ${activeCount} 条`} active />
        <Icon name="next" size="sm" className="shrink-0 text-jn-text-3" />
        <FlowChip icon="blocks" label="选择模型" />
        <Icon name="next" size="sm" className="shrink-0 text-jn-text-3" />
        <FlowChip icon="external" label="发送请求" />
        <span className="min-w-0 flex-1" />
        <button type="button" onClick={onOpenPrivacy} className="text-[12.5px] font-semibold text-jn-primary-ink">管理隐私设置</button>
      </section>

      {error && <div className="rounded-[16px] bg-jn-danger-soft px-4 py-2.5 text-[12px] text-jn-danger">读取失败：{error}</div>}
      {rows !== null && rows.length === 0 && !editing && (
        <div className="rounded-jn-card border border-dashed border-jn-line px-4 py-8 text-center text-[12px] text-jn-text-3">
          还没有规则。点页头「添加规则」后，下一个请求即生效。
        </div>
      )}

      <div className="flex flex-col gap-2.5">
        {ordered.map((row, index) => {
          const kind = kindOf(row.kind)
          const icon = KIND_ICONS[row.kind as MiddlewareKind] ?? "blocks"
          const editingRow = editing?.original === row.name
          return (
            <section key={row.name} className="flex flex-col gap-3 rounded-jn-card border border-jn-line bg-jn-surface py-3.5 pr-3.5 pl-[18px] shadow-jn-surface">
              <div className="flex items-center gap-3.5">
                <span className="w-6 shrink-0 text-[12px] font-semibold tabular-nums text-jn-text-3">{String(index + 1).padStart(2, "0")}</span>
                <span className={cn("flex size-9 shrink-0 items-center justify-center rounded-[12px]", row.enabled ? "bg-jn-primary-wash" : "bg-jn-well")}>
                  <Icon name={icon} size="md" className={row.enabled ? "text-jn-primary" : "text-jn-text-3"} />
                </span>
                <span className="flex min-w-0 flex-1 flex-col gap-[3px]">
                  <span className="flex min-w-0 items-center gap-2">
                    <span className={cn("truncate text-[14px] font-bold", row.enabled ? "text-jn-text" : "text-jn-text-2")}>{row.name}</span>
                    <Tag>{kind?.label ?? row.kind}</Tag>
                  </span>
                  <span className="truncate text-[12px] text-jn-text-3" title={summary(row.kind, row.config)}>{summary(row.kind, row.config)}</span>
                </span>
                <IconButton icon="collapse" label={`上移 ${row.name}`} size="sm" disabled={busy || index === 0} onClick={() => move(row, -1)} />
                <IconButton icon="expand" label={`下移 ${row.name}`} size="sm" disabled={busy || index === ordered.length - 1} onClick={() => move(row, 1)} />
                <Switch checked={row.enabled} disabled={busy} label={`${row.enabled ? "停用" : "启用"} ${row.name}`} onChange={() => toggle(row)} />
                <Button kind={editingRow ? "soft" : "ghost"} size="sm" icon="edit" disabled={busy} onClick={() => startEdit(row)}>
                  编辑
                </Button>
                <IconButton icon="delete" label={`删除规则 ${row.name}`} size="sm" disabled={busy} onClick={() => setRemoving(row.name)} />
              </div>
            </section>
          )
        })}
      </div>

      {/* R1-35 编辑 / 添加中间件规则弹窗 */}
      <Dialog
        open={Boolean(editing)}
        onOpenChange={(open) => { if (!open && !busy) setEditing(null) }}
        preventClose={busy}
        width="md"
        title={editing?.original ? `编辑规则 · ${editing.original}` : "添加规则"}
        description="保存前自动用样例请求校验；运行时出错跳过该项，命中拦截规则则拒绝请求。"
        actions={
          <>
            <Button kind="ghost" size="md" disabled={busy} onClick={() => setEditing(null)}>取消</Button>
            <Button kind="primary" size="md" icon="check" loading={busy} disabled={busy || !editing?.name.trim()} onClick={() => void saveEditing()}>
              {editing?.original ? "保存" : "添加"}
            </Button>
          </>
        }
      >
        {editing && (
          <div className="flex flex-col gap-3.5" data-rule-editor={editing.original ? "" : undefined} data-new-rule={editing.original === null ? "" : undefined}>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <FormField label="规则名称" required>
                <Input autoFocus size="lg" value={editing.name} onChange={(event) => setEditing({ ...editing, name: event.target.value })} placeholder="例如 快模型改名" aria-label="规则名称" />
              </FormField>
              <FormField label="规则类型" required>
                <Select width="full" label="规则类型" value={editing.kind}
                  onChange={(value) => setEditing({ ...editing, kind: value as MiddlewareKind, fields: emptyFields() })}
                  options={MIDDLEWARE_KINDS.map((item) => ({ value: item.id, label: item.label, hint: item.hint }))} />
              </FormField>
            </div>
            {kindOf(editing.kind)?.hint && (
              <span className="-mt-1 text-xs text-jn-text-3">{kindOf(editing.kind)?.hint}</span>
            )}
            <KindFields kind={editing.kind} fields={editing.fields} onChange={(fields) => setEditing({ ...editing, fields })} />
          </div>
        )}
      </Dialog>

      <section className="flex items-center gap-3 rounded-[16px] bg-jn-well px-4 py-2.5">
        <Icon name="info" size="sm" className="shrink-0 text-jn-text-3" />
        <span className="min-w-0 flex-1 text-[12px] text-jn-text-3">保存前用样例请求校验；运行时出错跳过该项，命中拦截规则则拒绝请求。</span>
      </section>

      {removing && (
        <ConfirmDialog
          open
          onOpenChange={(open) => { if (!open) setRemoving(null) }}
          title={`删除「${removing}」？`}
          description="删除后下一个请求起不再执行这个规则。"
          confirmText="删除"
          loading={busy}
          onConfirm={() => {
            const name = removing
            setRemoving(null)
            void run(async () => {
              await gatewayApi.deleteMiddleware(name)
              await load()
            }, `已删除「${name}」`)
          }}
        />
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
          <Input size="lg" value={fields.remove} onChange={(event) => onChange({ ...fields, remove: event.target.value })} placeholder="要删除的字段，逗号分隔，例如 temperature, top_p" />
        </>
      )
    case "system-prompt":
      return area("prompt", "例如：始终用中文回答", 4)
    case "word-guard":
      return (
        <>
          <Input size="lg" value={fields.block} onChange={(event) => onChange({ ...fields, block: event.target.value })} placeholder="拦截词，逗号分隔：命中整条请求拒发" />
          <Input size="lg" value={fields.mask} onChange={(event) => onChange({ ...fields, mask: event.target.value })} placeholder="遮盖词，逗号分隔：替换成 ***" />
        </>
      )
    case "think-tags":
      return (
        <div className="w-[200px]">
          <Select width="full" label="思考段处理方式" value={fields.mode}
            onChange={(value) => onChange({ ...fields, mode: value as "strip" | "wrap" })}
            options={[{ value: "strip", label: "剥掉思考段" }, { value: "wrap", label: "包进思考标签" }]} />
        </div>
      )
    case "js-hook":
      return (
        <>
          <Input size="lg" value={fields.path} onChange={(event) => onChange({ ...fields, path: event.target.value })} placeholder="钩子脚本完整路径，例如 D:\\hooks\\context.hooks.mjs" />
          <Input size="lg" value={fields.timeout} onChange={(event) => onChange({ ...fields, timeout: event.target.value })} placeholder="超时毫秒（可空，默认 1000，范围 100–30000）" className="w-[280px]" />
        </>
      )
  }
}

function FlowChip({ icon, label, active, onClick }: { icon: IconName; label: string; active?: boolean; onClick?: () => void }) {
  return (
    <button type="button" onClick={onClick} disabled={!onClick} aria-current={active ? "step" : undefined}
      className={cn(
        "jn-interactive flex h-9 items-center gap-[7px] rounded-[18px] pr-3.5 pl-3 text-[12.5px] font-semibold",
        active ? "bg-jn-primary-soft text-jn-primary-ink" : "bg-jn-well text-jn-text-2",
        onClick && "hover:bg-jn-track",
      )}>
      <Icon name={icon} size="sm" className={active ? "text-jn-primary" : "text-jn-text-2"} />
      {label}
    </button>
  )
}
