import { useState, type ChangeEvent } from "react"
import type { CustomAgentDto, CustomAgentInputDto, WiringPreviewDto } from "@/lib/gatewayApi"
import { CUSTOM_AGENT_PRESETS, CUSTOM_EXAMPLES, type CustomAgentPreset } from "@/lib/agentPage"
import { Icon } from "@/icons"
import { Button, Dialog, FormField, Input, Select, StatusPill, Switch, Tabs } from "@/ui"
import { AgentBadge } from "./AgentBadge"
import { DiffView } from "./DiffView"

/**
 * 通用接入的自定义工具详情（P1-1，画板未画，标「待确认」；外壳对齐 R1-09 的工具头）。
 *
 * 只写三个键：地址、令牌、模型（可选）；断开时只还原仍是 Jolu 写下的值的键。
 * 详情从抽屉改成整页（与内置工具一致），返回按钮与列表状态都由 `AgentsView` 持有。
 */
export function CustomAgentPanel({ agent, models, routing, preview, previewError, busy, model, onModel, onBack, onWire, onUnwire, onRemove }: {
  agent: CustomAgentDto
  models: string[]
  routing: boolean
  preview: WiringPreviewDto | null
  previewError: string | null
  busy: boolean
  model: string
  onModel: (model: string) => void
  onBack: () => void
  onWire: () => void
  onUnwire: () => void
  onRemove: () => void
}) {
  const managed = agent.wired || Boolean(agent.drift)
  const files = preview?.files ?? []
  const keys = [
    { label: "地址", key: agent.base_url_key, value: `http://127.0.0.1:端口${agent.base_suffix}` },
    { label: "令牌", key: agent.auth_key, value: `jolu-custom-${agent.slug}.…（Jolu 生成的接线令牌）` },
    ...(agent.model_key ? [{ label: "模型", key: agent.model_key, value: "你在下面选的模型" }] : []),
  ]
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3.5 overflow-y-auto px-0.5 pb-2 pt-0.5 [&>*]:shrink-0">
      <div className="flex items-center gap-3.5">
        <button type="button" onClick={onBack}
          className="jn-interactive inline-flex h-8 shrink-0 items-center gap-1 rounded-full bg-jn-surface pr-3 pl-2 text-[12.5px] font-medium text-jn-text-2 shadow-jn-surface">
          <Icon name="prev" size="sm" />
          全部工具
        </button>
        <AgentBadge agent={agent.id} name={agent.name} size={40} />
        <div className="flex min-w-0 max-w-[420px] flex-none flex-col gap-0.5">
          <span className="truncate text-[20px] leading-7 font-bold text-jn-text">{agent.name}</span>
          <span className="truncate text-[12px] text-jn-text-3">{agent.format.toUpperCase()} · {agent.config_path}</span>
        </div>
        <StatusPill tone={managed ? "success" : "neutral"}>{managed ? "已接入" : "未接入"}</StatusPill>
        <span className="min-w-0 flex-1" />
        {agent.wired && <Switch checked disabled={busy} label={`断开 ${agent.name}`} onChange={() => onUnwire()} />}
      </div>

      {agent.drift && (
        <div className="flex items-center gap-2.5 rounded-[16px] bg-jn-danger-soft px-3.5 py-[11px]">
          <Icon name="warning" size="sm" className="shrink-0 text-jn-danger" />
          <span className="min-w-0 flex-1 text-[12.5px] font-medium text-jn-text">
            {agent.drift === "removed" ? "配置文件被删除了" : "配置里的地址已被改成别的值"}，这个工具的请求不再经过模型路由。
          </span>
          <Button kind="ghost" size="sm" disabled={busy} onClick={onWire}>重新接入</Button>
          <Button kind="ghost" size="sm" disabled={busy} onClick={onUnwire}>放弃管理</Button>
        </div>
      )}

      {!managed && (
        <section className="flex flex-col gap-2.5 rounded-jn-card border border-jn-line bg-jn-surface p-4 shadow-jn-surface">
          <span className="text-[15px] font-semibold text-jn-text">接入 {agent.name}</span>
          <div className="flex flex-wrap items-center gap-2.5">
            {agent.model_key && (
              <div className="w-[260px]">
                <Select width="full" label="写入的模型" value={model || null} disabled={busy} onChange={onModel}
                  placeholder="不写模型（保留工具自己的）"
                  options={models.map((entry) => ({ value: entry, label: entry }))} />
              </div>
            )}
            <Button kind="primary" icon="connect" disabled={busy || !agent.installed} onClick={onWire}>接入 {agent.name}</Button>
          </div>
          {!routing && <span className="text-[12px] text-jn-text-3">模型路由现在没开，点「接入」会先开启模型路由，再写入配置。</span>}
        </section>
      )}

      <div className="flex flex-col gap-2.5">
        <span className="text-[13px] font-semibold text-jn-text">{managed ? "Jolu 写了这些键" : "接入会做什么"}</span>
        {keys.map((item) => (
          <div key={item.key} className="flex items-center gap-2.5">
            <span className="flex size-[26px] shrink-0 items-center justify-center rounded-[7px] bg-jn-primary-soft">
              <Icon name="file" size="sm" className="text-jn-primary-ink" />
            </span>
            <span className="text-[12px] text-jn-text"><code className="font-mono">{item.key}</code> ← {item.value}</span>
          </div>
        ))}
        <div className="flex items-center gap-2.5">
          <span className="flex size-[26px] shrink-0 items-center justify-center rounded-[7px] bg-jn-primary-soft">
            <Icon name="shield" size="sm" className="text-jn-primary-ink" />
          </span>
          <span className="text-[12px] text-jn-text">这三个键的原值先存快照；文件里的其它内容不动，写前备份到应用数据目录</span>
        </div>
        <div className="flex items-center gap-2.5">
          <span className="flex size-[26px] shrink-0 items-center justify-center rounded-[7px] bg-jn-primary-soft">
            <Icon name="undo" size="sm" className="text-jn-primary-ink" />
          </span>
          <span className="text-[12px] text-jn-text">断开时只还原仍是 Jolu 写下的值的键；接入后你自己改过的不会被还原</span>
        </div>
      </div>

      {previewError && <p className="text-[12px] text-jn-danger">预览读取失败：{previewError}</p>}
      {files.map((file) => <DiffView key={file.path} file={file} caption={preview?.mode === "wired" ? "断开时还原成左侧（-）的样子" : file.existed ? "写入预览 · 还没有改动" : "写入预览 · 会新建这个文件"} />)}

      <div className="flex items-center gap-2">
        <span className="text-[12px] text-jn-text-3">通用接入</span>
        <span className="min-w-0 flex-1" />
        {managed ? (
          <Button kind="danger" icon="undo" disabled={busy} onClick={onUnwire}>断开并还原</Button>
        ) : (
          <Button kind="danger" icon="delete" disabled={busy} onClick={onRemove}>删除这个工具</Button>
        )}
      </div>
    </div>
  )
}

/**
 * 添加自定义工具的表单（P1-1，画板未画，标「待确认」）。
 *
 * 用 R1-C 的 `Dialog` + `FormField` + `Input`：旧 `Modal` 与 `Button` 来自模型路由的
 * 旧弹窗外壳，令牌是 `jolu-*` 的，新页面不再依赖它们。
 */
export function CustomAgentDialog({ busy, onClose, onSave }: { busy: boolean; onClose: () => void; onSave: (input: CustomAgentInputDto) => Promise<boolean> }) {
  const [format, setFormat] = useState<"json" | "toml" | "env">("json")
  const [form, setForm] = useState({ name: "", config_path: "", base_url_key: "", auth_key: "", model_key: "", base_suffix: "/v1" })
  const [preset, setPreset] = useState<CustomAgentPreset | null>(null)
  const example = CUSTOM_EXAMPLES[format]
  /** 选一个工具预设：格式与三个键一次填好，用户只需核对（未验证的会提示核对） */
  const applyPreset = (next: CustomAgentPreset | null) => {
    setPreset(next)
    if (!next) return
    setFormat(next.format)
    setForm({
      name: next.name,
      config_path: next.configPath,
      base_url_key: next.baseKey,
      auth_key: next.authKey,
      model_key: next.modelKey,
      base_suffix: next.suffix,
    })
  }
  const set = (key: keyof typeof form) => (event: ChangeEvent<HTMLInputElement>) => setForm((current) => ({ ...current, [key]: event.target.value }))
  const ready = Boolean(form.name.trim() && form.config_path.trim() && form.base_url_key.trim() && form.auth_key.trim())
  const field = (key: keyof typeof form, label: string, placeholder: string, hint?: string) => (
    <FormField label={label} required={key === "name" || key === "config_path" || key === "base_url_key" || key === "auth_key"} hint={hint}>
      <Input spellCheck={false} autoComplete="off" aria-label={label} value={form[key]} onChange={set(key)} placeholder={placeholder} className="font-mono" />
    </FormField>
  )
  return (
    <Dialog
      open
      onOpenChange={(open) => { if (!open && !busy) onClose() }}
      title="添加自定义工具"
      width="md"
      actions={
        <>
          <Button kind="ghost" disabled={busy} onClick={onClose}>取消</Button>
          <Button kind="primary" loading={busy} disabled={busy || !ready} onClick={() => void onSave({
            name: form.name.trim(), config_path: form.config_path.trim(), format,
            base_url_key: form.base_url_key.trim(), auth_key: form.auth_key.trim(),
            model_key: form.model_key.trim() || null, base_suffix: form.base_suffix.trim() || null,
          }).then((ok) => { if (ok) onClose() })}>添加</Button>
        </>
      }
    >
      <FormField label="从已支持的工具开始">
        <Select width="full" label="从已支持的工具开始" value={preset?.id ?? ""}
          onChange={(value) => applyPreset(CUSTOM_AGENT_PRESETS.find((item) => item.id === value) ?? null)}
          placeholder="自己填（不套用预设）"
          options={[{ value: "", label: "自己填（不套用预设）" }, ...CUSTOM_AGENT_PRESETS.map((item) => ({ value: item.id, label: item.name + (item.verified ? "" : " · 键名待核对"), hint: item.configPath }))]} />
      </FormField>
      {preset && !preset.verified && (
        <span className="flex items-start gap-1.5 text-[11px] text-jn-warning">
          <Icon name="warning" size="xs" className="mt-px shrink-0" />
          这个工具的键名是按常见格式推测的，接入前请对照它的官方文档核对；键名不对会写坏配置（Jolu 会先备份）。
        </span>
      )}
      {field("name", "名称", "例如 Crush")}
      <FormField label="配置文件格式" required>
        <Tabs style="segment" value={format} onChange={setFormat}
          items={[{ key: "json", label: "JSON" }, { key: "toml", label: "TOML" }, { key: "env", label: "env" }]} />
      </FormField>
      {field("config_path", "配置文件路径", example.path, "绝对路径，或以 ~ 开头")}
      {field("base_url_key", "地址键", example.base, format === "env" ? "变量名" : "用点分隔的路径；中间缺的层级会自动建")}
      {field("auth_key", "令牌键", example.auth)}
      {field("model_key", "模型键", example.model, "可不填：不填就不改工具自己的模型")}
      {field("base_suffix", "地址后缀", example.suffix, "OpenAI 兼容的工具通常要 /v1；Anthropic 兼容的留空")}
    </Dialog>
  )
}
