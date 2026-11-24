import * as Menu from "@radix-ui/react-dropdown-menu"
import { useMemo, useRef, useState } from "react"
import { cn } from "@/lib/utils"
import { lineDiff } from "@/lib/textDiff"
import { isTauri } from "@/lib/api"
import { SKILL_EXTERNAL_CHANGE, skillsApi, type SkillDetailDto, type SkillItemDto, type SkillPlatformDto, type SkillScope } from "@/lib/skillsApi"
import {
  SKILL_PLATFORMS, frontField, hasNameConflict, projectLabel, setFrontField, skillTemplate, targetDirHint, validSkillName,
} from "@/lib/skillsManager"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { useToast } from "@/components/ui/Toast"
import { providerName } from "@/lib/providerCatalog"
import { Field, FieldSelect, INPUT_CLASS, MarkdownPreview, Tag } from "./skillParts"
import { Icon } from "@/icons"
import { Banner, Button, Checkbox, Dialog, StatusPill } from "@/ui"

/** 差异弹窗读取到的磁盘版本 */
type DiskVersion = { text: string; modified: number | null; truncated: boolean }

/** 查看差异：按行对比「磁盘版本」与「当前草稿」。 */
function DiffDialog({ disk, draft, onUseDisk, onKeepDraft, onClose }: {
  disk: DiskVersion
  draft: string
  onUseDisk: () => void
  onKeepDraft: () => void
  onClose: () => void
}) {
  const lines = useMemo(() => lineDiff(disk.text, draft), [disk.text, draft])
  const removed = lines.filter((l) => l.kind === "del").length
  const added = lines.filter((l) => l.kind === "add").length
  return (
    <Dialog
      open
      onOpenChange={(next) => { if (!next) onClose() }}
      width="xl"
      title="查看差异"
      description="按行对比磁盘上的最新版本与当前草稿。"
      actions={
        <>
          <Button kind="ghost" size="md" disabled={disk.truncated} onClick={onUseDisk}>使用磁盘版本</Button>
          <Button kind="primary" size="md" onClick={onKeepDraft}>保留草稿</Button>
        </>
      }
    >
      <div className="flex flex-wrap items-center gap-2 text-xs text-jn-text-3">
        <span className="rounded-md bg-jn-danger-soft px-2 py-0.5 font-mono text-jn-danger">- 磁盘版本</span>
        <span className="rounded-md bg-jn-primary-wash px-2 py-0.5 font-mono text-jn-primary-ink">+ 当前草稿</span>
        <span className="flex-1" />
        <span>{removed || added ? `删除 ${removed} 行 · 新增 ${added} 行` : "两个版本内容相同"}</span>
      </div>
      {disk.truncated && <Banner tone="danger">磁盘上的文件超过 512 KB，只对比了前 512 KB；为避免丢失内容，不能直接使用磁盘版本。</Banner>}
      <div className="max-h-[420px] min-h-[160px] overflow-auto rounded-[14px] border border-jn-line bg-jn-well py-1.5 font-mono text-[12px] leading-5">
        {lines.map((line, i) => (
          <div key={i} className={cn("flex min-w-max whitespace-pre px-3", line.kind === "del" && "bg-jn-danger-soft text-jn-danger", line.kind === "add" && "bg-jn-primary-wash text-jn-primary-ink", line.kind === "same" && "text-jn-text")}>
            <span aria-hidden className="w-4 shrink-0 select-none">{line.kind === "del" ? "-" : line.kind === "add" ? "+" : " "}</span>
            <span className="sr-only">{line.kind === "del" ? "删除：" : line.kind === "add" ? "新增：" : ""}</span>
            {line.text || " "}
          </div>
        ))}
      </div>
    </Dialog>
  )
}

export type EditorTarget =
  | { mode: "new"; platform: string }
  | { mode: "edit"; item: SkillItemDto; detail: SkillDetailDto }

/** 新建与编辑 Skill（R1-31）：基本信息、SKILL.md 编辑器与实时预览、保存前校验与外部修改横幅。 */
export function SkillEditor({ target, items, platforms, projects, onCancel, onSaved }: {
  target: EditorTarget
  items: SkillItemDto[]
  platforms: SkillPlatformDto[]
  projects: string[]
  onCancel: () => void
  onSaved: (path: string, platform: string) => void
}) {
  const toast = useToast()
  const editing = target.mode === "edit"
  const initialContent = editing ? target.detail.content : skillTemplate("", "")
  const [platform, setPlatform] = useState(editing ? target.item.platform : target.platform === "all" ? "claude" : target.platform)
  const [scopeKey, setScopeKey] = useState(editing ? (target.item.scope === "project" ? `project:${target.item.project}` : target.item.scope) : "global")
  const [name, setName] = useState(editing ? target.item.dir_name : "")
  const [content, setContent] = useState(initialContent)
  const [enable, setEnable] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [external, setExternal] = useState(false)
  const [expected, setExpected] = useState<number | null>(editing ? target.item.modified_ms : null)
  const [leaving, setLeaving] = useState(false)
  const [replacingTemplate, setReplacingTemplate] = useState(false)
  const templateTrigger = useRef<HTMLButtonElement>(null)
  const gutterRef = useRef<HTMLDivElement>(null)
  const [diff, setDiff] = useState<DiskVersion | null>(null)
  const [loadingDiff, setLoadingDiff] = useState(false)
  const lineCount = useMemo(() => Math.max(1, content.split("\n").length), [content])

  const scope: SkillScope = scopeKey.startsWith("project:") ? "project" : (scopeKey as SkillScope)
  const project = scope === "project" ? scopeKey.slice("project:".length) : null
  const description = frontField(content, "description") ?? ""
  const frontName = frontField(content, "name")
  const dirty = content !== initialContent || (!editing && name !== "")

  const problems = useMemo(() => {
    const list: string[] = []
    if (!validSkillName(name)) list.push("名称只能使用小写字母、数字和连字符，最长 64 位")
    if (!description.trim()) list.push("请填写描述")
    if (frontName && frontName !== name) list.push("SKILL.md 里的 name 与名称不一致")
    if (scope === "project" && !project) list.push("请选择项目")
    return list
  }, [name, description, frontName, scope, project])
  const conflict = !editing && validSkillName(name) && hasNameConflict(items, platform, scope, project, name)

  const scopeOptions = [{ value: "global", label: "全局" }, ...projects.map((p) => ({ value: `project:${p}`, label: `项目 · ${projectLabel(p)}` }))]

  const updateName = (value: string) => {
    setName(value)
    if (validSkillName(value)) setContent((text) => setFrontField(text, "name", value))
  }

  const save = async () => {
    if (saving || problems.length || conflict) return
    setSaving(true)
    setError(null)
    try {
      if (!isTauri) {
        toast.success(editing ? "已保存（示例）" : "已创建（示例）", "浏览器预览不会写入文件")
        return onSaved(editing ? target.item.path : `${targetDirHint(platform, scope, project, platforms)}${name}`, platform)
      }
      const result = await skillsApi.save({
        platform,
        scope,
        project,
        name,
        content,
        original_path: editing ? target.item.path : null,
        expected_modified_ms: editing ? expected : null,
        enable,
      })
      toast.success(editing ? "Skill 已保存" : "Skill 已创建", result.backup ? "原文件已备份到应用数据目录" : undefined)
      onSaved(result.path, platform)
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason)
      if (message === SKILL_EXTERNAL_CHANGE) setExternal(true)
      else setError(message)
    } finally {
      setSaving(false)
    }
  }

  const keepDraft = async (latestModified?: number | null) => {
    if (latestModified !== undefined) setExpected(latestModified)
    else if (editing && isTauri) {
      try {
        const latest = await skillsApi.detail(target.item.path)
        setExpected(latest.item.modified_ms)
      } catch (reason) {
        return setError(String(reason))
      }
    }
    setExternal(false)
    setDiff(null)
    toast.info("已保留草稿", "再次保存将覆盖外部修改，原文件会先备份")
  }

  const openDiff = async () => {
    if (!editing) return
    if (!isTauri) return setDiff({ text: initialContent, modified: expected, truncated: false })
    setLoadingDiff(true)
    try {
      const latest = await skillsApi.detail(target.item.path)
      setDiff({ text: latest.content, modified: latest.item.modified_ms, truncated: latest.truncated })
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setLoadingDiff(false)
    }
  }

  const applyDisk = (disk: DiskVersion) => {
    setContent(disk.text)
    setExpected(disk.modified)
    setExternal(false)
    setDiff(null)
    toast.info("已换成磁盘版本", "草稿中的修改已丢弃")
  }

  const cancel = () => (dirty ? setLeaving(true) : onCancel())
  const ok = !problems.length && !conflict

  return (
    <div className="flex w-full flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-col gap-1">
          <h2 className="text-[20px] font-bold text-jn-text">{editing ? "编辑 Skill" : "新建 Skill"}</h2>
          <p className="text-xs text-jn-text-3">{editing ? "修改 SKILL.md 并预览效果；保存前会先备份原文件。" : "从模板开始，编辑内容并预览效果。"}</p>
        </div>
        <div className="flex gap-2">
          <Button kind="ghost" size="sm" disabled={saving} onClick={cancel}>取消</Button>
          <Button kind="primary" size="sm" icon="check" loading={saving} disabled={!ok || saving || external} onClick={() => void save()}>
            {saving ? "保存中…" : "保存 Skill"}
          </Button>
        </div>
      </div>

      <section className="flex flex-col gap-4 rounded-jn-card border border-jn-line bg-jn-surface p-5 shadow-jn-surface">
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <Field label="名称 *">
            <input value={name} disabled={editing || saving} onChange={(e) => updateName(e.target.value.trim())} placeholder="例如 api-review" aria-label="名称" className={INPUT_CLASS} />
          </Field>
          <Field label="描述 *">
            <input value={description} disabled={saving} onChange={(e) => setContent((text) => setFrontField(text, "description", e.target.value))} placeholder="说明何时使用这个 Skill" aria-label="描述" className={INPUT_CLASS} />
          </Field>
        </div>
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <Field label="供应商 *">
            <FieldSelect label="供应商" value={platform} disabled={editing || saving} onChange={setPlatform} options={SKILL_PLATFORMS.map((id) => ({ value: id, label: providerName(id) }))} />
          </Field>
          <Field label="作用范围 *">
            <FieldSelect label="作用范围" value={scopeKey} disabled={editing || saving} onChange={setScopeKey} options={editing && scope !== "global" && scope !== "project" ? [{ value: scopeKey, label: scopeKey }] : scopeOptions} />
          </Field>
        </div>
        <p className="truncate font-mono text-[11.5px] text-jn-text-3">
          {editing ? `文件位置：${target.item.path}` : `保存到 ${targetDirHint(platform, scope, project, platforms)}${name || "<名称>"}/`}
          {!editing && projects.length === 0 && " · 需要项目范围时，先在「目录设置」添加项目目录"}
        </p>
      </section>

      <div className="grid h-[490px] grid-cols-1 gap-4 lg:grid-cols-2">
        <section className="flex min-h-0 min-w-0 flex-col gap-3 rounded-jn-card border border-jn-line bg-jn-surface p-5 shadow-jn-surface">
          <div className="flex h-8 items-center gap-2">
            <span className="font-mono text-[13px] font-semibold text-jn-text">SKILL.md</span>
            {dirty && <StatusPill tone="warning">已修改</StatusPill>}
            <span className="min-w-0 flex-1" />
            {editing && (
              <Button kind="ghost" size="sm" icon="diff" loading={loadingDiff} disabled={saving || loadingDiff} onClick={() => void openDiff()}>
                查看差异
              </Button>
            )}
            <Menu.Root modal={false}>
              <Menu.Trigger ref={templateTrigger} disabled={saving} className="jn-interactive inline-flex h-8 items-center gap-1.5 rounded-full bg-jn-well px-3 text-xs font-medium text-jn-text outline-none hover:bg-jn-track">
                插入模板<Icon name="expand" size="xs" className="jn-chevron text-jn-text-3" />
              </Menu.Trigger>
              <Menu.Portal>
                <Menu.Content align="end" sideOffset={6} className="jn-popover z-[140] w-[220px] rounded-[16px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface">
                  <Menu.Item
                    disabled={saving}
                    className="flex h-8 cursor-pointer select-none items-center gap-2 rounded-[10px] px-2.5 text-xs text-jn-text outline-none data-[highlighted]:bg-jn-well"
                    onSelect={() => { if (content !== skillTemplate(name, description)) setReplacingTemplate(true) }}
                  >
                    <Icon name="fileText" size="sm" className="text-jn-text-3" />基础模板（替换当前内容）
                  </Menu.Item>
                  <span className="block px-2.5 py-1 text-[11px] text-jn-text-3">更多模板待确认</span>
                </Menu.Content>
              </Menu.Portal>
            </Menu.Root>
          </div>
          <div className="flex min-h-0 flex-1 overflow-hidden rounded-[14px] bg-jn-well focus-within:ring-2 focus-within:ring-jn-primary/30">
            <div
              ref={gutterRef}
              aria-hidden
              className="tnum select-none overflow-hidden border-r border-jn-line/70 py-3.5 pr-2.5 pl-3 text-right font-mono text-[12px] leading-5 text-jn-text-3"
            >
              {Array.from({ length: lineCount }, (_, idx) => (
                <div key={idx}>{idx + 1}</div>
              ))}
            </div>
            <textarea
              disabled={saving}
              value={content}
              onChange={(e) => setContent(e.target.value)}
              onScroll={(e) => {
                if (gutterRef.current) gutterRef.current.scrollTop = e.currentTarget.scrollTop
              }}
              spellCheck={false}
              aria-label="SKILL.md 内容"
              className="min-h-0 flex-1 resize-none bg-transparent py-3.5 pr-3.5 pl-3 font-mono text-[12px] leading-5 text-jn-text outline-none"
            />
          </div>
        </section>
        <section className="flex min-h-0 min-w-0 flex-col gap-4 overflow-y-auto rounded-jn-card border border-jn-line bg-jn-surface p-6 shadow-jn-surface">
          <span className="text-xs font-semibold text-jn-primary-ink">实时预览</span>
          <MarkdownPreview source={content} />
        </section>
      </div>

      {external ? (
        <Banner tone="danger">
          <div className="flex flex-wrap items-center gap-3">
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <span className="font-semibold">检测到外部修改</span>
              <span className="text-xs opacity-90">文件已被其他程序修改。当前草稿已保留，查看差异后再决定如何合并。</span>
            </div>
            <div className="flex shrink-0 gap-2">
              <Button kind="soft" size="sm" icon="diff" loading={loadingDiff} disabled={loadingDiff} onClick={() => void openDiff()}>
                查看差异
              </Button>
              <Button kind="ghost" size="sm" icon="fileText" onClick={() => void keepDraft()}>
                保留草稿
              </Button>
            </div>
          </div>
        </Banner>
      ) : (
        <div className={cn("flex min-h-11 items-center gap-2.5 rounded-[14px] px-4 text-xs", ok ? "bg-jn-primary-wash text-jn-primary-ink" : "bg-jn-danger-soft text-jn-danger")}>
          <Icon name={ok ? "success" : "danger"} size="sm" className="shrink-0" />
          <span className="min-w-0 flex-1 truncate">
            {error ?? (ok ? `名称、描述与目标目录校验通过 · ${editing ? "尚未保存" : "尚未写入"}` : conflict ? "目标已有同名 Skill，请换一个名称或选择其他范围" : problems[0])}
          </span>
          {!editing && validSkillName(name) && <Tag tone={conflict ? "danger" : "on"}>{conflict ? "同名冲突" : "无同名冲突"}</Tag>}
        </div>
      )}

      {!editing && (
        <div className="flex items-center gap-2.5 self-start">
          <Checkbox checked={enable} disabled={saving} label="保存后启用" onChange={setEnable} />
          <button type="button" disabled={saving} onClick={() => setEnable(!enable)} className="text-left text-xs text-jn-text outline-none">
            保存后启用
            <span className="ml-1 text-[11.5px] text-jn-text-3">（{platform === "claude" ? "不勾选时放入 Jolu 停用区，工具不会加载，同名副本不受影响" : "不勾选时按这份 SKILL.md 的路径写入 config.toml"}）</span>
          </button>
        </div>
      )}

      {diff && <DiffDialog disk={diff} draft={content} onUseDisk={() => applyDisk(diff)} onKeepDraft={() => void keepDraft(diff.modified)} onClose={() => setDiff(null)} />}

      {replacingTemplate && <ConfirmDialog
        title="替换当前内容？"
        description="基础模板会替换编辑器中的全部内容。尚未保存的修改将丢失，磁盘文件不会改变。"
        confirmLabel="替换模板"
        returnFocusTo={templateTrigger.current}
        danger
        onClose={() => setReplacingTemplate(false)}
        onConfirm={() => setContent(skillTemplate(name, description))}
      />}

      <Dialog
        open={leaving}
        onOpenChange={(next) => { if (!next) setLeaving(false) }}
        width="sm"
        title="离开编辑器？"
        description="当前有尚未保存的修改。"
        actions={
          <>
            <Button kind="ghost" size="sm" onClick={() => setLeaving(false)}>继续编辑</Button>
            <Button kind="danger" size="sm" onClick={onCancel}>放弃修改</Button>
            <Button kind="primary" size="sm" disabled={!ok || saving} onClick={() => { setLeaving(false); void save() }}>保存</Button>
          </>
        }
      />
    </div>
  )
}
