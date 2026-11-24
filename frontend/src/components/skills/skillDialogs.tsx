import * as Menu from "@radix-ui/react-dropdown-menu"
import { useEffect, useRef, useState, type ReactNode } from "react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import {
  skillsApi, type SkillCopyResultDto, type SkillImportAction, type SkillImportScanDto, type SkillItemDto, type SkillPlatformDto, type SkillProjectDto, type SkillRootDto, type SkillScope,
} from "@/lib/skillsApi"
import { SKILL_PLATFORMS, globalChanges, globalRowView, projectLabel, scopeLabel, staleParkedNote, targetDirHint, type GlobalDraft } from "@/lib/skillsManager"
import { demoImportScan } from "@/mock/skills"
import { Tip } from "@/components/ui/Tip"
import { useToast } from "@/components/ui/Toast"
import { providerName } from "@/lib/providerCatalog"
import { DialogShell, Field, FieldSelect, INPUT_CLASS, Tag } from "./skillParts"
import { Icon } from "@/icons"
import { Banner, Button, Checkbox, OptionTile } from "@/ui"

function errorText(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason)
}

function scopeOptions(projects: string[]) {
  return [{ value: "global", label: "全局" }, ...projects.map((p) => ({ value: `project:${p}`, label: `项目 · ${projectLabel(p)}` }))]
}

function parseScope(key: string): { scope: SkillScope; project: string | null } {
  return key.startsWith("project:") ? { scope: "project", project: key.slice("project:".length) } : { scope: "global", project: null }
}

/* ───────────── 导入 ───────────── */

export function ImportDialog({ platform, platforms, projects, onClose, onDone }: {
  platform: string
  platforms: SkillPlatformDto[]
  projects: string[]
  onClose: () => void
  onDone: () => void
}) {
  const toast = useToast()
  const [target, setTarget] = useState(platform === "all" ? "claude" : platform)
  const [scopeKey, setScopeKey] = useState("global")
  const [source, setSource] = useState<string | null>(isTauri ? null : "D:/Downloads/jolu-skills/")
  const [scan, setScan] = useState<SkillImportScanDto | null>(null)
  const [scanning, setScanning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [actions, setActions] = useState<Record<string, SkillImportAction>>({})
  const [enable, setEnable] = useState(false)
  const [importing, setImporting] = useState(false)
  const [mode, setMode] = useState<"local" | "git">("local")
  const [gitUrl, setGitUrl] = useState("")
  const [gitBranch, setGitBranch] = useState("")
  const [fetching, setFetching] = useState(false)
  const checkout = useRef<string | null>(null)
  const closed = useRef(false)
  const { scope, project } = parseScope(scopeKey)

  const dropCheckout = () => {
    const dir = checkout.current
    checkout.current = null
    if (dir && isTauri) void skillsApi.importGitCleanup(dir).catch(() => undefined)
  }
  useEffect(() => {
    closed.current = false
    return () => {
      closed.current = true
      dropCheckout()
    }
  }, [])

  useEffect(() => {
    if (!source) return
    let stale = false
    setScanning(true)
    setError(null)
    const load = isTauri ? skillsApi.importScan(source, target, scope, project) : Promise.resolve(demoImportScan(targetDirHint(target, scope, project)))
    load.then((result) => {
      if (stale) return
      setScan(result)
      setPicked(new Set(result.items.filter((i) => i.status === "ok").map((i) => i.name)))
      setActions(Object.fromEntries(result.items.filter((i) => i.status === "conflict").map((i) => [i.name, "skip" as SkillImportAction])))
    }, (reason: unknown) => !stale && (setScan(null), setError(errorText(reason)))).finally(() => !stale && setScanning(false))
    return () => { stale = true }
  }, [source, target, scope, project])

  const pick = async () => {
    if (!isTauri) return
    try {
      const folder = await skillsApi.pickFolder("选择要导入的 Skills 目录")
      if (folder) setSource(folder)
    } catch (reason) {
      setError(errorText(reason))
    }
  }

  const switchMode = (next: "local" | "git") => {
    if (next === mode || fetching) return
    dropCheckout()
    setMode(next)
    setScan(null)
    setError(null)
    setSource(next === "local" && !isTauri ? "D:/Downloads/jolu-skills/" : null)
  }

  const gitUrlOk = gitUrl.trim().startsWith("https://")
  const fetchGit = async () => {
    if (!gitUrlOk || fetching) return
    dropCheckout()
    setSource(null)
    setScan(null)
    setError(null)
    if (!isTauri) return setSource(gitUrl.trim())
    setFetching(true)
    try {
      const result = await skillsApi.importGit(gitUrl.trim(), gitBranch.trim() || null, target, scope, project)
      if (closed.current) return void skillsApi.importGitCleanup(result.checkout_dir).catch(() => undefined)
      checkout.current = result.checkout_dir
      setSource(result.scan.source)
    } catch (reason) {
      if (!closed.current) setError(errorText(reason))
    } finally {
      if (!closed.current) setFetching(false)
    }
  }

  const choices = (scan?.items ?? []).flatMap((item): { name: string; action: SkillImportAction }[] => {
    if (item.status === "ok") return picked.has(item.name) ? [{ name: item.name, action: "import" }] : []
    if (item.status === "conflict" && actions[item.name] === "overwrite") return [{ name: item.name, action: "overwrite" }]
    return []
  })
  const counts = {
    ok: scan?.items.filter((i) => i.status === "ok").length ?? 0,
    conflict: scan?.items.filter((i) => i.status === "conflict").length ?? 0,
    invalid: scan?.items.filter((i) => i.status === "invalid").length ?? 0,
  }
  const step = !source ? 1 : importing ? 3 : 2

  const submit = async () => {
    if (!source || !choices.length) return
    if (!isTauri) {
      toast.success(`已导入 ${choices.length} 项（示例）`, "浏览器预览不会复制文件")
      return onDone()
    }
    setImporting(true)
    try {
      const result = await skillsApi.importSkills(source, target, scope, project, choices, enable)
      if (result.failed.length) toast.warn(`已导入 ${result.imported.length} 项，${result.failed.length} 项失败`, result.failed.map((f) => `${f.path}：${f.reason}`).join("；"))
      else toast.success(`已导入 ${result.imported.length} 项`, enable ? undefined : "导入项默认停用，可在列表中启用")
      onDone()
    } catch (reason) {
      setError(errorText(reason))
    } finally {
      setImporting(false)
    }
  }

  return (
    <DialogShell
      width={680}
      title="导入 Skills"
      description="从本地目录或 Git 仓库扫描并导入 SKILL.md。"
      onClose={onClose}
      busy={importing}
      actions={
        <>
          <Button kind="ghost" size="md" disabled={importing} onClick={onClose}>取消</Button>
          <Button kind="primary" size="md" icon="download" loading={importing} disabled={!choices.length || importing || scanning} onClick={() => void submit()}>
            {importing ? "导入中…" : `导入 ${choices.length} 项`}
          </Button>
        </>
      }
    >
      <div className="flex h-7 items-center gap-3 text-xs">
        {["选择来源", "检查内容", "确认导入"].map((label, i) => (
          <span key={label} className="flex items-center gap-2.5">
            {i > 0 && <Icon name="next" size="xs" className="text-jn-text-3" />}
            <span className={cn(i + 1 <= step ? "text-jn-primary-ink" : "text-jn-text-3", i + 1 === step && "font-semibold")}>{i + 1} · {label}</span>
          </span>
        ))}
      </div>

      <div role="radiogroup" aria-label="导入来源" className="grid grid-cols-2 gap-2.5">
        <OptionTile active={mode === "local"} label="本地目录" icon="folder" disabled={fetching} onClick={() => switchMode("local")} />
        <OptionTile active={mode === "git"} label="Git 仓库" icon="gitBranch" disabled={fetching} onClick={() => switchMode("git")} />
      </div>

      {mode === "local" ? (
        <div className="flex min-h-[58px] items-center gap-2.5 rounded-[14px] bg-jn-well p-3.5">
          <Icon name="openFolder" size="md" className="shrink-0 text-jn-primary-ink" />
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="truncate font-mono text-xs text-jn-text">{source ?? "尚未选择目录"}</span>
            <span className="text-[11px] text-jn-text-3">
              {scanning ? "正在检查…" : scan ? `已扫描 ${scan.items.length} 个 Skill · 包含引用文件与资源` : "选择包含 SKILL.md 的目录，或其上一级目录"}
            </span>
          </div>
          <Button kind="ghost" size="sm" onClick={() => void pick()}>{source ? "更换目录" : "选择目录"}</Button>
        </div>
      ) : (
        <div className="flex flex-col gap-2 rounded-[14px] bg-jn-well p-3.5">
          <form className="flex items-end gap-2.5" onSubmit={(e) => { e.preventDefault(); void fetchGit() }}>
            <label className="flex min-w-0 flex-1 flex-col gap-1.5">
              <span className="text-xs text-jn-text-2">仓库地址</span>
              <input value={gitUrl} onChange={(e) => setGitUrl(e.target.value)} disabled={fetching} placeholder="https://github.com/owner/repo" spellCheck={false} aria-label="仓库地址" className={cn(INPUT_CLASS, "bg-jn-surface font-mono")} />
            </label>
            <label className="flex w-[140px] shrink-0 flex-col gap-1.5">
              <span className="text-xs text-jn-text-2">分支（可选）</span>
              <input value={gitBranch} onChange={(e) => setGitBranch(e.target.value)} disabled={fetching} placeholder="默认分支" spellCheck={false} aria-label="分支（可选）" className={cn(INPUT_CLASS, "bg-jn-surface font-mono")} />
            </label>
            <Button type="submit" kind="soft" size="md" icon="download" loading={fetching} disabled={!gitUrlOk || fetching}>
              {fetching ? "获取中…" : "获取"}
            </Button>
          </form>
          <span className="truncate text-[11px] text-jn-text-3" title={scan?.source}>
            {fetching ? "正在用本机 git 拉取仓库（最长 60 秒）…"
              : scanning ? "正在检查…"
                : scan ? `已扫描 ${scan.items.length} 个 Skill · 包含引用文件与资源`
                  : gitUrl.trim() && !gitUrlOk ? "仓库地址只支持 https:// 开头" : "只接受 https:// 地址；用本机 git 拉取到临时目录，导入后自动删除"}
          </span>
        </div>
      )}

      <div className="grid grid-cols-2 gap-3.5">
        <Field label="目标供应商">
          <FieldSelect label="目标供应商" value={target} onChange={setTarget} options={SKILL_PLATFORMS.map((id) => ({ value: id, label: providerName(id) }))} />
        </Field>
        <Field label="安装范围">
          <FieldSelect label="安装范围" value={scopeKey} onChange={setScopeKey} options={scopeOptions(projects)} />
        </Field>
      </div>
      <span className="truncate font-mono text-[11.5px] text-jn-text-3">目标：{scan?.target ?? targetDirHint(target, scope, project, platforms)}</span>

      {error && <Banner tone="danger"><span role="alert">{error}</span></Banner>}

      {scan && (
        <div className="flex flex-col overflow-hidden rounded-[16px] border border-jn-line">
          <div className="flex h-9 items-center justify-between bg-jn-well px-3.5">
            <span className="text-xs font-semibold text-jn-text">待导入项目</span>
            <span className="text-[11px] text-jn-text-3">{counts.ok} 可导入 · {counts.conflict} 冲突 · {counts.invalid} 异常</span>
          </div>
          {!scan.items.length && <p className="px-3.5 py-4 text-xs text-jn-text-3">这个目录里没有找到带 SKILL.md 的 Skill</p>}
          <div className="max-h-[220px] overflow-y-auto">
            {scan.items.map((item) => {
              const checked = picked.has(item.name)
              return (
                <div key={item.name} className="flex h-[58px] items-center gap-2.5 border-t border-jn-line px-3.5">
                  {item.status === "ok" ? (
                    <Checkbox
                      checked={checked}
                      label={`导入 ${item.name}`}
                      onChange={(next) => setPicked((set) => {
                        const copy = new Set(set)
                        if (next) copy.add(item.name)
                        else copy.delete(item.name)
                        return copy
                      })}
                    />
                  ) : (
                    <Checkbox checked={false} disabled label={item.name} onChange={() => {}} />
                  )}
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className="truncate text-xs font-semibold text-jn-text">{item.name}</span>
                    <span className="truncate text-[11px] text-jn-text-3">{item.issue ?? "目录与元信息有效"}</span>
                  </div>
                  {item.status === "ok" && <Tag tone="on">可导入</Tag>}
                  {item.status === "invalid" && <Tag tone="danger">不可导入</Tag>}
                  {item.status === "conflict" && (
                    <Menu.Root modal={false}>
                      <Menu.Trigger className="jn-interactive inline-flex h-7 items-center gap-1 rounded-full bg-jn-well px-3 text-xs font-medium text-jn-text outline-none hover:bg-jn-track">
                        {actions[item.name] === "overwrite" ? "覆盖" : "跳过"}<Icon name="expand" size="xs" />
                      </Menu.Trigger>
                      <Menu.Portal>
                        <Menu.Content align="end" sideOffset={6} className="jn-popover z-[160] w-[220px] rounded-[16px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface">
                          <Menu.Item className="flex h-8 cursor-pointer items-center rounded-[10px] px-2.5 text-xs text-jn-text outline-none data-[highlighted]:bg-jn-well" onSelect={() => setActions((a) => ({ ...a, [item.name]: "skip" }))}>跳过</Menu.Item>
                          <Menu.Item className="flex min-h-8 cursor-pointer flex-col items-start justify-center rounded-[10px] px-2.5 py-1 text-xs text-jn-text outline-none data-[highlighted]:bg-jn-well" onSelect={() => setActions((a) => ({ ...a, [item.name]: "overwrite" }))}>
                            覆盖
                            <span className="text-[11px] text-jn-text-3">先备份目标目录，再替换</span>
                          </Menu.Item>
                        </Menu.Content>
                      </Menu.Portal>
                    </Menu.Root>
                  )}
                </div>
              )
            })}
          </div>
        </div>
      )}

      <div className="flex items-center gap-2.5 self-start">
        <Checkbox checked={enable} label="导入后启用" onChange={setEnable} />
        <button type="button" onClick={() => setEnable(!enable)} className="text-left text-xs text-jn-text outline-none">
          导入后启用
          <span className="ml-1.5 text-[11px] text-jn-text-3">默认关闭；关闭时 Claude Code 放入 Jolu 停用区，Codex 按路径停用</span>
        </button>
      </div>
      <Banner tone="info">
        导入仅复制文件，不跟随符号链接；冲突项默认跳过，选择覆盖时先备份目标目录；格式异常项需先修复。
      </Banner>
    </DialogShell>
  )
}

/* ───────────── 复制到其他供应商 ───────────── */

export function CopyDialog({ items, projects, onClose, onDone }: {
  items: SkillItemDto[]
  projects: string[]
  onClose: () => void
  onDone: () => void
}) {
  const toast = useToast()
  const first = items[0]
  const [target, setTarget] = useState(first.platform === "claude" ? "codex" : "claude")
  const [scopeKey, setScopeKey] = useState("global")
  const [checks, setChecks] = useState<Record<string, SkillCopyResultDto | string> | null>(null)
  const [copying, setCopying] = useState(false)
  const { scope, project } = parseScope(scopeKey)

  useEffect(() => {
    let stale = false
    setChecks(null)
    if (!isTauri) {
      setChecks(Object.fromEntries(items.map((i) => [i.path, { target: `${targetDirHint(target, scope, project)}${i.dir_name}`, copied: false, conflict: false, issue: null }])))
      return
    }
    Promise.all(items.map((i) => skillsApi.copy(i.path, target, scope, project, true).then((r) => [i.path, r] as const, (reason: unknown) => [i.path, errorText(reason)] as const)))
      .then((list) => !stale && setChecks(Object.fromEntries(list)))
    return () => { stale = true }
  }, [items, target, scope, project])

  const ready = checks ? items.filter((i) => {
    const c = checks[i.path]
    return typeof c === "object" && !c.conflict && !c.issue
  }) : []

  const submit = async () => {
    if (!isTauri) {
      toast.success(`已复制 ${ready.length} 项（示例）`)
      return onDone()
    }
    setCopying(true)
    const failed: string[] = []
    for (const item of ready) {
      try {
        await skillsApi.copy(item.path, target, scope, project, false)
      } catch (reason) {
        failed.push(`${item.name}：${errorText(reason)}`)
      }
    }
    setCopying(false)
    if (failed.length) toast.warn(`已复制 ${ready.length - failed.length} 项，${failed.length} 项失败`, failed.join("；"))
    else toast.success(`已复制 ${ready.length} 项到 ${providerName(target)}`, "源文件保留，创建了独立副本")
    onDone()
  }

  const single = items.length === 1
  return (
    <DialogShell
      width={520}
      title="复制到其他供应商"
      description={`${single ? first.name : `已选择 ${items.length} 个 Skill`} · ${providerName(first.platform)} → ${providerName(target)}`}
      onClose={onClose}
      busy={copying}
      actions={
        <>
          <Button kind="ghost" size="md" disabled={copying} onClick={onClose}>取消</Button>
          <Button kind="primary" size="md" icon="copy" loading={copying} disabled={!ready.length || copying} onClick={() => void submit()}>
            {copying ? "复制中…" : single ? "复制" : `复制 ${ready.length} 项`}
          </Button>
        </>
      }
    >
      <div className="grid grid-cols-2 gap-3">
        <Field label="目标供应商">
          <FieldSelect label="目标供应商" value={target} onChange={setTarget} options={SKILL_PLATFORMS.map((id) => ({ value: id, label: providerName(id) }))} />
        </Field>
        <Field label="安装范围">
          <FieldSelect label="安装范围" value={scopeKey} onChange={setScopeKey} options={scopeOptions(projects)} />
        </Field>
      </div>
      <p className="text-xs text-jn-text-3">确认目标支持当前 Skill 格式后才允许复制。默认保留源文件，创建独立副本。</p>
      <div className="flex max-h-[220px] flex-col gap-1.5 overflow-y-auto rounded-[14px] bg-jn-well p-3">
        {!checks && <Icon name="loading" size="sm" className="animate-spin text-jn-text-3" />}
        {checks && items.map((item) => {
          const c = checks[item.path]
          const text = typeof c === "string" ? c : c.conflict ? "目标已有同名 Skill，不会覆盖" : c.issue ?? "兼容性校验通过"
          const good = typeof c === "object" && !c.conflict && !c.issue
          return (
            <div key={item.path} className="flex items-center gap-2 text-xs">
              {!single && <span className="w-32 shrink-0 truncate font-medium text-jn-text">{item.name}</span>}
              <Tag tone={good ? "on" : "danger"}>{text}</Tag>
              {typeof c === "object" && <span className="min-w-0 truncate font-mono text-[11px] text-jn-text-3" title={c.target}>{c.target}</span>}
            </div>
          )
        })}
      </div>
    </DialogShell>
  )
}

/* ───────────── 移除确认 ───────────── */

export function RemoveDialog({ item, onClose, onDone }: { item: SkillItemDto; onClose: () => void; onDone: () => void }) {
  const toast = useToast()
  const [removing, setRemoving] = useState(false)
  const removeLock = useRef(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async () => {
    if (removeLock.current) return
    removeLock.current = true
    if (!isTauri) {
      toast.success("已移除（示例）")
      return onDone()
    }
    setRemoving(true)
    try {
      const backup = await skillsApi.remove(item.path)
      toast.success(`已移除 ${item.name}`, `备份保存在 ${backup}`)
      onDone()
    } catch (reason) {
      setError(errorText(reason))
      removeLock.current = false
    } finally {
      setRemoving(false)
    }
  }

  return (
    <DialogShell
      width={440}
      title="移除此安装项？"
      description={`${item.name} · ${providerName(item.platform)}${item.scope === "project" ? ` / ${scopeLabel(item)}` : ""}`}
      onClose={onClose}
      busy={removing}
      actions={
        <>
          <Button kind="ghost" size="md" disabled={removing} onClick={onClose}>取消</Button>
          <Button kind="danger" size="md" icon="delete" loading={removing} disabled={removing} onClick={() => void submit()}>
            {removing ? "移除中…" : "移除 Skill"}
          </Button>
        </>
      }
    >
      <p className="break-all rounded-[12px] bg-jn-well px-3 py-2 font-mono text-[11.5px] text-jn-text-2">{item.path}</p>
      <p className="text-xs leading-relaxed text-jn-text-3">只影响此目录中的安装项，其他供应商副本保留。运行中的任务不立即中断；后续加载将不再使用。</p>
      <Banner tone="info">移除前自动备份到应用数据目录。</Banner>
      {error && <Banner tone="danger"><span role="alert">{error}</span></Banner>}
    </DialogShell>
  )
}

/* ───────────── 扫描目录设置 ───────────── */

const GLOBAL_DIR_NOTE = "工具是否读取该目录取决于其配置（如 CLAUDE_CONFIG_DIR / CODEX_HOME）"

export function DirectoryDialog({ platform, roots, projects, onClose, onSaved }: {
  platform: string
  roots: SkillRootDto[]
  projects: SkillProjectDto[]
  onClose: () => void
  onSaved: () => void
}) {
  const [custom, setCustom] = useState(() => roots.filter((r) => r.platform === "custom").map((r) => r.path))
  const [manual, setManual] = useState(() => projects.filter((p) => p.source === "manual").map((p) => p.path))
  const [ignored, setIgnored] = useState(() => new Set(projects.filter((p) => p.ignored).map((p) => p.path)))
  const [drafts, setDrafts] = useState<Record<string, GlobalDraft>>({})
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const globals = roots.filter((r) => r.scope === "global" && r.platform !== "custom")
  const sessions = projects.filter((p) => p.source === "session")
  const known = (path: string) => projects.find((p) => p.path === path)

  const add = async () => {
    if (!isTauri) return
    try {
      const folder = await skillsApi.pickFolder("添加项目目录")
      if (folder && !manual.includes(folder)) setManual([...manual, folder])
    } catch (reason) {
      setError(errorText(reason))
    }
  }

  const pickGlobal = async (root: SkillRootDto) => {
    if (!isTauri) return
    try {
      const folder = await skillsApi.pickFolder(`更换 ${providerName(root.platform)} 全局目录`)
      if (folder) setDrafts((d) => ({ ...d, [root.platform]: folder }))
    } catch (reason) {
      setError(errorText(reason))
    }
  }

  const save = async () => {
    if (!isTauri) return onSaved()
    setSaving(true)
    setError(null)
    try {
      await skillsApi.saveDirSettings({
        projects: manual,
        custom_dirs: custom,
        ignore: sessions.filter((p) => p.ignored !== ignored.has(p.path)).map((p) => ({ path: p.path, ignored: ignored.has(p.path) })),
        globals: globalChanges(roots, drafts),
      })
      onSaved()
    } catch (reason) {
      setError(errorText(reason))
    } finally {
      setSaving(false)
    }
  }

  const skillDirs = (path: string) => {
    const p = known(path)
    if (!p) return "保存后扫描"
    return p.tools.length ? `含 ${p.tools.map(providerName).join(" / ")} 的项目 Skills 目录` : "没有 Skills 目录 · 可作为新建或复制的目标"
  }

  const row = (key: string, label: string, path: string, note: ReactNode, action: ReactNode, dim = false) => (
    <div key={key} className={cn("flex items-center gap-2.5 rounded-[14px] bg-jn-well p-3", dim && "opacity-55")}>
      <Icon name="openFolder" size="sm" className="shrink-0 text-jn-text-3" />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-[11px] text-jn-text-3">{label}</span>
        <span className="truncate font-mono text-xs text-jn-text" title={path}>{path}</span>
        {note}
      </div>
      {action}
    </div>
  )

  const globalRow = (r: SkillRootDto) => {
    if (!r.overridable) {
      const note = r.exists ? null : <span className="text-[11px] text-jn-text-3">未检测到这个目录</span>
      return row(`g-${r.platform}-${r.path}`, `${providerName(r.platform)} · 全局目录`, r.path, note,
        <Tip label="共享目录，不受工具环境变量影响，固定扫描"><span tabIndex={0}><Button kind="ghost" size="sm" disabled>更换</Button></span></Tip>)
    }
    const view = globalRowView(r, drafts[r.platform])
    const notes = [
      !view.changed && !r.exists ? "未检测到这个目录" : null,
      view.defaultPath ? `默认位置：${view.defaultPath}` : null,
    ].filter(Boolean).join(" · ")
    const note = (
      <>
        {notes && <span className="text-[11px] text-jn-text-3">{notes}</span>}
        {view.staleParked > 0 && (
          <span className="flex items-start gap-1 text-[11px] text-jn-text">
            <Icon name="info" size="xs" className="mt-0.5 shrink-0" />{staleParkedNote(view.staleParked)}
          </span>
        )}
      </>
    )
    const action = (
      <div className="flex shrink-0 gap-1.5">
        {view.overridden && <Button kind="ghost" size="sm" disabled={!isTauri || saving} onClick={() => setDrafts((d) => ({ ...d, [r.platform]: null }))}>恢复默认</Button>}
        <Button kind="ghost" size="sm" disabled={!isTauri || saving} onClick={() => void pickGlobal(r)}>更换</Button>
      </div>
    )
    return row(`g-${r.platform}-${r.path}`, `${providerName(r.platform)} · 全局目录${view.overridden ? " · 已更换" : ""}`, view.path, note, action)
  }

  return (
    <DialogShell
      width={600}
      title="扫描目录设置"
      description={`供应商：${platform === "all" ? "全部" : providerName(platform)} · 全局目录可更换，只影响 Jolu 扫描和写入的位置，${GLOBAL_DIR_NOTE}`}
      onClose={onClose}
      busy={saving}
      actions={
        <>
          <Button kind="ghost" size="sm" icon="add" disabled={!isTauri || saving} onClick={() => void add()}>添加项目目录</Button>
          <Button kind="primary" size="md" icon="refresh" loading={saving} disabled={saving} onClick={() => void save()}>保存并扫描</Button>
        </>
      }
    >
      <div className="flex max-h-[380px] flex-col gap-2 overflow-y-auto">
        {globals.map(globalRow)}
        {custom.map((p) => row(`c-${p}`, "自定义 · 只读扫描", p, null, <Button kind="ghost" size="sm" disabled={saving} onClick={() => setCustom(custom.filter((x) => x !== p))}>移除</Button>))}
        <Button kind="ghost" size="sm" icon="add" className="self-start" disabled={!isTauri || saving} onClick={() => void skillsApi.pickFolder("添加自定义扫描目录").then((p) => { if (p && !custom.includes(p)) setCustom([...custom, p]) }).catch((e) => setError(errorText(e)))}>
          添加自定义目录
        </Button>
        {manual.map((p) => row(`m-${p}`, "项目目录 · 手动添加", p, <span className="text-[11px] text-jn-text-3">{skillDirs(p)}</span>,
          <Button kind="ghost" size="sm" icon="close" onClick={() => setManual(manual.filter((x) => x !== p))}>移除</Button>,
        ))}
        {sessions.map((p) => {
          const off = ignored.has(p.path)
          return row(`s-${p.path}`, off ? "项目目录 · 来自会话记录 · 已忽略" : "项目目录 · 来自会话记录", p.path, <span className="text-[11px] text-jn-text-3">{skillDirs(p.path)}</span>,
            <Button kind="ghost" size="sm" onClick={() => setIgnored((set) => {
              const next = new Set(set)
              if (off) next.delete(p.path)
              else next.add(p.path)
              return next
            })}>{off ? "恢复" : "忽略"}</Button>,
            off,
          )
        })}
        {!manual.length && !sessions.length && <p className="px-1 text-xs text-jn-text-3">会话记录里还没有项目目录；可以手动添加。</p>}
      </div>
      {error && <Banner tone="danger"><span role="alert">{error}</span></Banner>}
      <p className="text-[11px] text-jn-text-3">只扫描含有 Skills 目录的项目；已不存在的目录自动跳过。更换全局目录、忽略或移除项目不会移动或删除任何文件或会话数据。</p>
    </DialogShell>
  )
}
