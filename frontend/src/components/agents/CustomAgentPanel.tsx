import { useState } from "react"
import { FilePlus2, Plug, ShieldCheck, Trash2, TriangleAlert, Undo2 } from "lucide-react"
import { cn } from "@/lib/utils"
import type { CustomAgentDto, CustomAgentInputDto, WiringPreviewDto } from "@/lib/gatewayApi"
import { CUSTOM_EXAMPLES, agentBadge } from "@/lib/agentPage"
import { Button, Modal } from "@/components/gateway/GatewayDialogs"
import { AgentBadge, Switch } from "./AgentList"
import { DiffView } from "./DiffView"

const selectClass = "h-8 min-w-0 rounded-lg border border-jolu-border bg-jolu-bg px-2.5 text-meta text-jolu-text outline-none focus:border-jolu-accent disabled:opacity-50"

/**
 * 通用接入的自定义工具详情（P1-1，无画板，待确认；沿用 V5-1 / V5-2 的版式）。
 * 只写三个键：地址、令牌、模型（可选）；断开时只还原仍是 Jolu 写下的值的键。
 */
export function CustomAgentPanel({ agent, models, routing, preview, previewError, busy, model, onModel, onWire, onUnwire, onRemove }: {
  agent: CustomAgentDto
  models: string[]
  routing: boolean
  preview: WiringPreviewDto | null
  previewError: string | null
  busy: boolean
  model: string
  onModel: (model: string) => void
  onWire: () => void
  onUnwire: () => void
  onRemove: () => void
}) {
  const badge = agentBadge(agent.id, agent.name)
  const managed = agent.wired || Boolean(agent.drift)
  const files = preview?.files ?? []
  const keys = [
    { label: "地址", key: agent.base_url_key, value: `http://127.0.0.1:端口${agent.base_suffix}` },
    { label: "令牌", key: agent.auth_key, value: `jolu-custom-${agent.slug}.…（Jolu 生成的接线令牌）` },
    ...(agent.model_key ? [{ label: "模型", key: agent.model_key, value: "你在下面选的模型" }] : []),
  ]
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-3.5 overflow-y-auto px-7 py-5">
      <div className="flex items-center gap-3">
        <AgentBadge bg={badge.bg} glyph={badge.glyph} size={40} />
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-[17px] font-semibold text-jolu-text">{agent.name}</span>
            <span className="rounded-full bg-jolu-track px-2 py-0.5 text-note font-semibold text-jolu-muted">{managed ? "自定义 · 已接入" : "自定义 · 未接入"}</span>
          </div>
          <span className="truncate text-meta text-jolu-muted">{agent.format.toUpperCase()} · {agent.config_path}</span>
        </div>
        {agent.wired && <Switch on disabled={busy} label={`断开 ${agent.name}`} onClick={onUnwire} />}
      </div>

      {agent.drift && (
        <div className="flex items-center gap-2.5 rounded-[10px] bg-jolu-status-error-bg px-3.5 py-2.5">
          <TriangleAlert aria-hidden className="size-3.5 shrink-0 text-jolu-danger" />
          <span className="min-w-0 flex-1 text-meta text-jolu-text">{agent.drift === "removed" ? "配置文件被删除了" : "配置里的地址已被改成别的值"}，这个工具的请求不再经过模型路由。</span>
          <button type="button" disabled={busy} onClick={onWire} className="h-7 shrink-0 rounded-md border border-jolu-border bg-jolu-bg px-2.5 text-meta text-jolu-text disabled:opacity-45">重新接入</button>
          <button type="button" disabled={busy} onClick={onUnwire} className="h-7 shrink-0 rounded-md border border-jolu-border bg-jolu-bg px-2.5 text-meta text-jolu-text disabled:opacity-45">放弃管理</button>
        </div>
      )}

      {!managed && (
        <div className="flex flex-col gap-2.5 rounded-xl border border-jolu-border p-4">
          <span className="text-[13px] font-semibold text-jolu-text">接入 {agent.name}</span>
          <div className="flex flex-wrap items-center gap-2.5">
            {agent.model_key && (
              <select aria-label="写入的模型" value={model} disabled={busy} onChange={(event) => onModel(event.target.value)} className={cn(selectClass, "w-[260px]")}>
                <option value="">不写模型（保留工具自己的）</option>
                {models.map((entry) => <option key={entry} value={entry}>{entry}</option>)}
              </select>
            )}
            <button type="button" disabled={busy || !agent.installed} onClick={onWire}
              className="flex h-8 items-center gap-1.5 rounded-lg bg-jolu-accent px-3.5 text-meta font-semibold text-[#071B10] outline-none disabled:opacity-45">
              <Plug aria-hidden className="size-3.5" />
              接入 {agent.name}
            </button>
          </div>
          {!routing && <span className="text-meta text-jolu-muted">模型路由现在没开，点「接入」会先开启模型路由，再写入配置。</span>}
        </div>
      )}

      <div className="flex flex-col gap-2.5">
        <span className="text-[13px] font-semibold text-jolu-text">{managed ? "Jolu 写了这些键" : "接入会做什么"}</span>
        {keys.map((item) => (
          <div key={item.key} className="flex items-center gap-2.5">
            <span className="flex size-[26px] shrink-0 items-center justify-center rounded-[7px] bg-jolu-selected"><FilePlus2 aria-hidden className="size-3.5 text-jolu-on" /></span>
            <span className="text-meta text-jolu-text"><code className="font-mono">{item.key}</code> ← {item.value}</span>
          </div>
        ))}
        <div className="flex items-center gap-2.5">
          <span className="flex size-[26px] shrink-0 items-center justify-center rounded-[7px] bg-jolu-selected"><ShieldCheck aria-hidden className="size-3.5 text-jolu-on" /></span>
          <span className="text-meta text-jolu-text">这三个键的原值先存快照；文件里的其它内容不动，写前备份到应用数据目录</span>
        </div>
        <div className="flex items-center gap-2.5">
          <span className="flex size-[26px] shrink-0 items-center justify-center rounded-[7px] bg-jolu-selected"><Undo2 aria-hidden className="size-3.5 text-jolu-on" /></span>
          <span className="text-meta text-jolu-text">断开时只还原仍是 Jolu 写下的值的键；接入后你自己改过的不会被还原</span>
        </div>
      </div>

      {previewError && <p className="text-meta text-jolu-danger">预览读取失败：{previewError}</p>}
      {files.map((file) => <DiffView key={file.path} file={file} caption={preview?.mode === "wired" ? "断开时还原成左侧（-）的样子" : file.existed ? "写入预览 · 还没有改动" : "写入预览 · 会新建这个文件"} />)}

      <div className="flex items-center gap-2">
        <span className="text-meta text-jolu-muted">通用接入（待确认：没有画板）</span>
        <span className="flex-1" />
        {managed ? (
          <button type="button" disabled={busy} onClick={onUnwire}
            className="flex h-8 items-center gap-1.5 rounded-lg border border-jolu-border px-3 text-meta text-jolu-danger outline-none hover:bg-jolu-side disabled:opacity-45">
            <Undo2 aria-hidden className="size-3.5" />断开并还原
          </button>
        ) : (
          <button type="button" disabled={busy} onClick={onRemove}
            className="flex h-8 items-center gap-1.5 rounded-lg border border-jolu-border px-3 text-meta text-jolu-danger outline-none hover:bg-jolu-side disabled:opacity-45">
            <Trash2 aria-hidden className="size-3.5" />删除这个工具
          </button>
        )}
      </div>
    </div>
  )
}

/** 添加自定义工具的表单（P1-1，待确认：没有画板） */
export function CustomAgentDialog({ busy, onClose, onSave }: { busy: boolean; onClose: () => void; onSave: (input: CustomAgentInputDto) => Promise<boolean> }) {
  const [format, setFormat] = useState<"json" | "toml" | "env">("json")
  const [form, setForm] = useState({ name: "", config_path: "", base_url_key: "", auth_key: "", model_key: "", base_suffix: "/v1" })
  const example = CUSTOM_EXAMPLES[format]
  const set = (key: keyof typeof form) => (event: React.ChangeEvent<HTMLInputElement>) => setForm((current) => ({ ...current, [key]: event.target.value }))
  const ready = form.name.trim() && form.config_path.trim() && form.base_url_key.trim() && form.auth_key.trim()
  const field = (key: keyof typeof form, label: string, placeholder: string, note?: string) => (
    <label className="flex flex-col gap-1.5">
      <span className="text-meta font-medium text-jolu-text">{label}</span>
      <input value={form[key]} onChange={set(key)} placeholder={placeholder} spellCheck={false} autoComplete="off"
        className="h-9 rounded-lg border border-jolu-border bg-jolu-bg px-3 font-mono text-meta text-jolu-text outline-none focus:border-jolu-accent" />
      {note && <span className="text-note text-jolu-muted">{note}</span>}
    </label>
  )
  return (
    <Modal title="添加自定义工具" busy={busy} onClose={onClose} width={560}
      footer={<>
        <Button label="取消" disabled={busy} onClick={onClose} />
        <Button label="添加" primary disabled={busy || !ready} onClick={() => void onSave({
          name: form.name.trim(), config_path: form.config_path.trim(), format,
          base_url_key: form.base_url_key.trim(), auth_key: form.auth_key.trim(),
          model_key: form.model_key.trim() || null, base_suffix: form.base_suffix.trim() || null,
        }).then((ok) => { if (ok) onClose() })} />
      </>}>
      <p className="text-meta text-jolu-muted">Jolu 没有专门适配的工具，可以照它的文档填写配置文件和键名。待确认：没有画板。</p>
      {field("name", "名称 *", "例如 Crush")}
      <div className="flex flex-col gap-1.5">
        <span className="text-meta font-medium text-jolu-text">配置文件格式 *</span>
        <div className="flex items-center gap-0.5 self-start rounded-lg bg-jolu-track p-0.5" role="group" aria-label="配置文件格式">
          {(["json", "toml", "env"] as const).map((value) => (
            <button key={value} type="button" aria-pressed={format === value} onClick={() => setFormat(value)}
              className={cn("h-7 rounded-md px-3 text-meta outline-none", format === value ? "bg-jolu-bg font-medium text-jolu-text shadow-sm" : "text-jolu-muted")}>
              {value === "env" ? "env" : value.toUpperCase()}
            </button>
          ))}
        </div>
      </div>
      {field("config_path", "配置文件路径 *", example.path, "绝对路径，或以 ~ 开头")}
      {field("base_url_key", "地址键 *", example.base, format === "env" ? "变量名" : "用点分隔的路径；中间缺的层级会自动建")}
      {field("auth_key", "令牌键 *", example.auth)}
      {field("model_key", "模型键", example.model, "可不填：不填就不改工具自己的模型")}
      {field("base_suffix", "地址后缀", example.suffix, "OpenAI 兼容的工具通常要 /v1；Anthropic 兼容的留空")}
    </Modal>
  )
}
