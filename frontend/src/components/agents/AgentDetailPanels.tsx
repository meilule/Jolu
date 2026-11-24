import { useState } from "react"
import { Icon } from "@/icons"
import { longContextMode, parseTokens, sessionRecordNote, wireExplanation, type AgentRow } from "@/lib/agentPage"
import type { AutoChoice } from "@/lib/tierModels"
import { Button, IconButton, Input, SettingGroup, SettingRow, Select, Tabs, Tag } from "@/ui"
import { DiffView } from "./DiffView"
import type { AgentDetailProps } from "./AgentDetail"

/**
 * Agent 详情里除「模型设置」以外的页签与弹层：
 * - 配置方案菜单（R1-09 未画，沿用 V5 的方案功能，标「待确认」）；
 * - 高级（R1-32「Agent · 详情 · 高级」）；
 * - 写入改动（R1-33「Agent · 详情 · 写入改动」）；
 * - 接入前预览 / 暂不支持 / 不能接入（R1 未画，沿用 V5-2 内容，标「待确认」）。
 */

/* ─────────────────────────── 配置方案（待确认） ─────────────────────────── */

/** 配置方案下拉（画板 R1-09 未画；保留 V5 的方案功能，放在页签行右端）。 */
export function ProfileMenu({ profiles, busy, onProfileApply, onProfileSave, onProfileDelete }: AgentDetailProps) {
  const [open, setOpen] = useState(false)
  const [naming, setNaming] = useState<string | null>(null)
  return (
    <div className="relative">
      <Button kind="ghost" size="sm" disabled={busy} aria-expanded={open} onClick={() => { setOpen((value) => !value); setNaming(null) }}>
        配置方案
        <Icon name="expand" size="xs" className="text-jn-text-3" />
      </Button>
      {open && (
        <div role="menu" className="absolute right-0 top-9 z-20 w-64 rounded-[14px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface">
          <p className="px-2.5 pb-1.5 text-[11px] text-jn-text-3">把这套方案应用到各已接入的工具</p>
          {profiles.length === 0 && naming === null && <p className="px-2.5 py-1.5 text-[12px] text-jn-text-3">还没有保存过方案</p>}
          {profiles.map((profile) => (
            <div key={profile.name} className="flex items-center gap-1 rounded-[10px] px-2.5 py-1 hover:bg-jn-well">
              <button type="button" role="menuitem" disabled={busy} onClick={() => { setOpen(false); onProfileApply(profile.name) }}
                className="min-w-0 flex-1 truncate text-left text-[12px] text-jn-text">
                {profile.name}
              </button>
              <IconButton icon="close" label={`删除方案 ${profile.name}`} size="sm" disabled={busy} onClick={() => onProfileDelete(profile.name)} />
            </div>
          ))}
          {naming === null ? (
            <button type="button" role="menuitem" disabled={busy} onClick={() => setNaming("")}
              className="w-full rounded-[10px] px-2.5 py-1.5 text-left text-[12px] font-semibold text-jn-primary-ink hover:bg-jn-well">
              ＋ 保存当前配置为新方案
            </button>
          ) : (
            <form className="flex items-center gap-1.5 px-2 py-1.5" onSubmit={(event) => { event.preventDefault(); if (naming.trim()) { onProfileSave(naming.trim()); setNaming(null); setOpen(false) } }}>
              <Input autoFocus size="md" value={naming} onChange={(event) => setNaming(event.target.value)} placeholder="方案名称" aria-label="方案名称" className="h-8 w-32" />
              <button type="submit" className="text-[12px] font-semibold text-jn-primary-ink">保存</button>
              <button type="button" onClick={() => setNaming(null)} className="text-[12px] text-jn-text-3">取消</button>
            </form>
          )}
        </div>
      )}
    </div>
  )
}

/* ─────────────────────────── 高级（R1-32） ─────────────────────────── */

export function AdvancedBody(props: AgentDetailProps) {
  const { models, sourceNotes, mainModel, subagent, draft, busy, longContext, onApplyTiers, onLongContext } = props
  const mode = longContextMode(longContext)
  const [custom, setCustom] = useState(mode === "custom" ? longContext : "")
  /** 自定义值没有对应分段：视觉上落在「272K 时压缩」一侧，输入框里的值才是真实状态 */
  const segmentValue = mode === "full" ? "full" : "default"
  const autoValue = draft.auto.kind === "model" ? `model:${draft.auto.model}` : draft.auto.kind
  const setAuto = (value: string) => {
    const auto: AutoChoice = value === "default" ? { kind: "default" } : value === "subagent" ? { kind: "subagent" } : { kind: "model", model: value.slice("model:".length) }
    onApplyTiers({ ...draft, auto }, mainModel, subagent)
  }
  const customValid = parseTokens(custom) !== null
  return (
    <div className="flex flex-col gap-3.5" data-advanced>
      <section className="flex flex-col rounded-jn-card border border-jn-line bg-jn-surface px-5 py-1 shadow-jn-surface">
        <div className="flex h-[50px] items-center gap-2.5">
          <span className="flex size-[30px] items-center justify-center rounded-[10px] bg-jn-primary-wash">
            <Icon name="navSettings" size="sm" className="text-jn-primary" />
          </span>
          <h3 className="text-[14px] font-bold text-jn-text">模型与压缩</h3>
        </div>
        <SettingGroup>
          <SettingRow bare
            title="子代理模型"
            description="Claude Code 派生子代理时使用；留空由 Claude Code 自己挑选"
            control={
              <div className="w-[320px]">
                <Select width="full" label="子代理模型" value={subagent || null} disabled={busy}
                  onChange={(value) => onApplyTiers(draft, mainModel, value)}
                  placeholder="由 Claude Code 自己挑选"
                  options={models.map((model) => ({ value: model, label: model, hint: sourceNotes[model] }))} />
              </div>
            } />
          <SettingRow bare
            title="自动模式模型"
            description="自动模式下 Claude Code 选用的模型"
            control={
              <div className="w-[320px]">
                <Select width="full" label="自动模式模型" value={autoValue} disabled={busy} onChange={setAuto}
                  options={[
                    { value: "default", label: "默认（Claude Code 自己挑）" },
                    { value: "subagent", label: "跟随子代理" },
                    ...models.map((model) => ({ value: `model:${model}`, label: model, hint: sourceNotes[model] })),
                  ]} />
              </div>
            } />
          <SettingRow bare
            title="长对话压缩"
            description="只对窗口大于 200K 的模型生效，窗口未知的模型不写"
            control={
              <div className="flex items-center gap-2">
                <Tabs style="segment" value={segmentValue}
                  onChange={(value) => onLongContext(value === "default" ? "" : "full")}
                  items={[{ key: "default", label: "272K 时压缩" }, { key: "full", label: "完整窗口" }]} />
                <form className="flex items-center gap-1.5" onSubmit={(event) => { event.preventDefault(); if (customValid) onLongContext(custom.trim()) }}>
                  <Input size="md" value={custom} onChange={(event) => setCustom(event.target.value)} placeholder="自定义，如 400k"
                    aria-label="自定义压缩大小" invalid={Boolean(custom) && !customValid} className="h-8 w-[120px]" />
                  <Button kind="ghost" size="sm" type="submit" disabled={busy || !customValid}>应用</Button>
                </form>
              </div>
            } />
          <SettingRow bare
            title="新会话强度"
            description="新会话默认思考强度"
            control={
              <div className="flex items-center gap-2">
                <div className="w-[200px]">
                  <Select width="full" label="新会话强度" value="default" disabled onChange={() => undefined}
                    options={[{ value: "default", label: "默认" }]} />
                </div>
                <Tag>待确认</Tag>
              </div>
            } />
        </SettingGroup>
      </section>
    </div>
  )
}

/* ─────────────────────────── 写入改动（R1-33） ─────────────────────────── */

export function DiffBody({ preview, previewError }: AgentDetailProps) {
  const files = preview?.files ?? []
  const keys = files.reduce((sum, file) => sum + file.keys, 0)
  return (
    <div className="flex flex-col gap-3.5">
      <div className="flex items-center gap-2">
        <Icon name="file" size="sm" className="text-jn-text-2" />
        <span className="text-[13px] font-semibold text-jn-text">
          {files.length ? `改了 ${files.length} 个文件 · ${keys} 个键 · 断开时逐键还原` : "没有检测到 Jolu 的改动"}
        </span>
        <span className="min-w-0 flex-1" />
        <span className="text-[11.5px] text-jn-text-3">密钥已遮住</span>
      </div>
      {previewError && <p className="text-[12px] text-jn-danger">{previewError}</p>}
      {!preview && !previewError && <p className="text-[12px] text-jn-text-3">正在生成写入预览…</p>}
      {files.map((file) => <DiffView key={file.path} file={file} caption="断开时还原成左侧（-）的样子" />)}
    </div>
  )
}

/* ─────────────────────────── 接入前预览（无 R1 画板，标「待确认」） ─────────────────────────── */

export function PreviewBody({ row, models, sourceNotes, mainModel, onMainModel, routing, preview, previewError, busy, onWire }: AgentDetailProps) {
  const claude = row.agent === "claude-code"
  const files = preview?.mode === "wire" ? preview.files : []
  const icons = { file: "file", shield: "shield", undo: "undo" } as const
  return (
    <div className="flex flex-col gap-4">
      <section className="flex flex-col gap-2.5 rounded-jn-card border border-jn-line bg-jn-surface p-4 shadow-jn-surface">
        <span className="text-[15px] font-semibold text-jn-text">{claude ? "接入后，新会话从哪个模型开始" : `接入 ${row.name}`}</span>
        <div className="flex flex-wrap items-center gap-2.5">
          {claude && (
            <div className="w-[300px]">
              <Select width="full" label="起点模型" value={mainModel || null} disabled={busy} onChange={onMainModel}
                placeholder="不选（保留工具自己的选择）"
                options={models.map((model) => ({ value: model, label: model, hint: sourceNotes[model] }))} />
            </div>
          )}
          <Button kind="primary" icon="connect" disabled={busy} onClick={onWire}>接入 {row.name}</Button>
        </div>
        <span className="text-[12px] text-jn-text-3">
          {claude
            ? `不选也可以接入；模型路由里的 ${models.length} 个模型会出现在它的模型列表里。`
            : `接入后请求经模型路由转发，可用 ${models.length} 个模型。`}
        </span>
      </section>

      <section className="flex flex-col gap-2.5">
        <span className="text-[15px] font-semibold text-jn-text">接入会做什么</span>
        {wireExplanation(row.agent).map((item) => (
          <div key={item.text} className="flex items-center gap-2.5">
            <span className="flex size-[26px] shrink-0 items-center justify-center rounded-[7px] bg-jn-primary-soft">
              <Icon name={icons[item.icon]} size="sm" className="text-jn-primary-ink" />
            </span>
            <span className="text-[12px] text-jn-text">{item.text}</span>
          </div>
        ))}
        {/* 会话记录能力：不支持的就说清楚，别让用户在活跃会话里找不到它却不知道为什么 */}
        {sessionRecordNote(row.agent) && (
          <div className="flex items-center gap-2.5 rounded-[14px] bg-jn-well px-3.5 py-2.5">
            <Icon name="info" size="sm" className="shrink-0 text-jn-text-3" />
            <span className="text-[12px] text-jn-text-3">{sessionRecordNote(row.agent)}</span>
          </div>
        )}
      </section>

      {previewError && <p className="text-[12px] text-jn-danger">写入预览读取失败：{previewError}</p>}
      {!preview && !previewError && <p className="text-[12px] text-jn-text-3">正在生成写入预览…</p>}
      {files.map((file) => <DiffView key={file.path} file={file} caption={file.existed ? "写入预览 · 还没有改动" : "写入预览 · 会新建这个文件"} />)}

      {!routing && (
        <div className="flex items-center gap-2.5 rounded-[14px] bg-jn-well px-3.5 py-2.5">
          <Icon name="info" size="sm" className="shrink-0 text-jn-text-3" />
          <span className="text-[12px] text-jn-text-3">模型路由现在没开，点「接入」会先开启模型路由，再写入配置。</span>
        </div>
      )}
    </div>
  )
}

/** 暂不支持（R1 未画）：探测到但 Jolu 还没有专用接入；指到通用接入。 */
export function UnsupportedBody({ row }: { row: AgentRow }) {
  return (
    <div className="flex flex-col gap-2 rounded-jn-card border border-jn-line bg-jn-surface p-4 shadow-jn-surface">
      <span className="text-[15px] font-semibold text-jn-text">暂无专用接入</span>
      <span className="text-[12px] text-jn-text-3">
        {row.wiring?.config_path ? <>配置位置：<code className="font-mono">{row.wiring.config_path}</code></> : row.subtitle}
      </span>
    </div>
  )
}

export function BlockedBody({ row }: { row: AgentRow }) {
  const cliMissing = row.discovery?.state === "cli_missing"
  return (
    <div className="flex flex-col gap-2 rounded-jn-card border border-jn-line bg-jn-surface p-4 shadow-jn-surface">
      <span className="text-[15px] font-semibold text-jn-text">暂时不能接入</span>
      <span className="text-[12px] text-jn-text-3">
        {cliMissing ? "检测到它的配置，但找不到命令行程序。装回 CLI 后点「重新检测」。" : `${row.subtitle}。装好并运行过一次后，点「重新检测」。`}
      </span>
      {row.wiring?.config_path && <span className="font-mono text-[11px] text-jn-text-3">配置位置：{row.wiring.config_path}</span>}
    </div>
  )
}
