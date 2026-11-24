import { useEffect, useState } from "react"
import { LoadingSurface } from "@/components/ui/LoadingSurface"
import { useContentMotion } from "@/lib/motion"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { observeActiveRequest } from "@/lib/activeRequest"
import { skillsApi, type SkillDetailDto, type SkillFileContentDto, type SkillItemDto } from "@/lib/skillsApi"
import { formatBytes, modifiedLabel, scopeLabel, sourceLabel, statusLabel } from "@/lib/skillsManager"
import { demoSkillDetail } from "@/mock/skills"
import { Tip } from "@/components/ui/Tip"
import { useToast } from "@/components/ui/Toast"
import { providerName } from "@/lib/providerCatalog"
import { MarkdownPreview, SkillIcon, Tag } from "./skillParts"
import { Icon, type IconName } from "@/icons"
import { Banner, Button, StatusPill, Tabs } from "@/ui"

const SCOPE_DETAIL: Record<SkillItemDto["scope"], string> = {
  global: "全局 · 当前系统用户",
  project: "项目",
  system: "系统内置 · 工具托管",
  synced: "claude.ai 同步 · 只读",
}

function fileIconName(rel: string, dir: boolean): IconName {
  if (dir) return "folder"
  if (/\.(png|jpe?g|gif|webp|svg)$/i.test(rel)) return "image"
  if (/\.(md|txt)$/i.test(rel)) return "fileText"
  return "file"
}

/** Skill 详情与文件（R1-30）：文件列表、来源信息与 SKILL.md 预览 / 源码 */
export function SkillDetail({ item, onBack, onEdit, onCopy, onOpenDir }: {
  item: SkillItemDto
  onBack: () => void
  onEdit: (item: SkillItemDto, detail: SkillDetailDto) => void
  onCopy: (item: SkillItemDto) => void
  onOpenDir: (item: SkillItemDto) => void
}) {
  const toast = useToast()
  const [detail, setDetail] = useState<SkillDetailDto | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [file, setFile] = useState("SKILL.md")
  const [fileContent, setFileContent] = useState<SkillFileContentDto | null>(null)
  const [tab, setTab] = useState<"preview" | "source">("preview")

  useEffect(() => {
    setDetail(null)
    setError(null)
    setFile("SKILL.md")
    if (!isTauri) return setDetail(demoSkillDetail(item))
    return observeActiveRequest(skillsApi.detail(item.path), setDetail, (reason) => setError(String(reason)))
  }, [item])

  useEffect(() => {
    setFileContent(null)
    if (file === "SKILL.md" || !detail) return
    if (!isTauri) return setFileContent({ text: file.endsWith(".png") ? null : `# ${file}\n\n（示例文件内容）`, binary: file.endsWith(".png"), size: 0, truncated: false })
    return observeActiveRequest(skillsApi.file(item.path, file), setFileContent, (reason) => setFileContent({ text: `读取失败：${String(reason)}`, binary: false, size: 0, truncated: false }))
  }, [file, detail, item.path])

  const current = detail?.item ?? item
  const isSkillFile = file === "SKILL.md"
  const text = isSkillFile ? detail?.content ?? "" : fileContent?.text ?? null
  const markdown = isSkillFile || /\.md$/i.test(file)

  const bodyMotion = useContentMotion(`${file}-${tab}-${!!detail}-${!!fileContent}`)

  const copyPath = async () => {
    try {
      await navigator.clipboard.writeText(current.path)
      toast.success("已复制路径")
    } catch {
      toast.danger("复制失败", "请检查剪贴板权限")
    }
  }

  return (
    <div className="flex w-full flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <Button kind="ghost" size="sm" icon="arrowLeft" onClick={onBack}>返回 Skills</Button>
        <span className="flex-1" />
        <Button kind="ghost" size="sm" icon="openFolder" onClick={() => onOpenDir(current)}>打开目录</Button>
        <Button kind="ghost" size="sm" icon="copy" onClick={() => onCopy(current)}>复制到供应商</Button>
        {current.readonly ? (
          <Tip label={current.symlink ? "符号链接安装项只读，请到原位置修改" : "工具托管的安装项只读，可复制为自定义"}>
            <span tabIndex={0}><Button kind="primary" size="sm" icon="edit" disabled>编辑 Skill</Button></span>
          </Tip>
        ) : (
          <Button kind="primary" size="sm" icon="edit" disabled={!detail} onClick={() => detail && onEdit(current, detail)}>编辑 Skill</Button>
        )}
      </div>

      <section className="flex min-h-[92px] flex-wrap items-center gap-4 rounded-jn-card border border-jn-line bg-jn-surface p-5 shadow-jn-surface">
        <SkillIcon item={current} size={48} />
        <div className="flex min-w-0 flex-1 flex-col gap-1.5">
          <h2 className="truncate text-xl font-bold text-jn-text">{current.name}</h2>
          <p className="truncate text-xs text-jn-text-3">{current.issue ?? current.description ?? "没有描述"}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Tag>{providerName(current.platform)}</Tag>
          <Tag>{scopeLabel(current)}</Tag>
          <StatusPill tone={current.status === "enabled" ? "success" : (current.status === "disabled" || current.status === "unknown") ? "neutral" : "danger"}>
            {statusLabel(current.status)}
          </StatusPill>
        </div>
      </section>

      {error ? (
        <Banner tone="danger">读取失败：{error}</Banner>
      ) : (
        <div className="grid min-h-[620px] grid-cols-1 gap-4 lg:grid-cols-[280px_minmax(0,1fr)]">
          <aside className="flex flex-col gap-4 rounded-jn-card border border-jn-line bg-jn-surface p-4 shadow-jn-surface">
            <h3 className="text-sm font-semibold text-jn-text">文件与来源</h3>
            <div className="flex max-h-[260px] flex-col gap-0.5 overflow-y-auto">
              {!detail && <Icon name="loading" size="sm" className="animate-spin text-jn-text-3" />}
              {detail?.files.map((entry) => {
                const icon = fileIconName(entry.rel, entry.dir)
                const active = !entry.dir && entry.rel === file
                const name = entry.rel.split("/").pop() ?? entry.rel
                return (
                  <button
                    key={entry.rel}
                    type="button"
                    disabled={entry.dir}
                    onClick={() => setFile(entry.rel)}
                    title={entry.rel}
                    style={{ paddingLeft: 10 + entry.depth * 14 }}
                    className={cn(
                      "jn-interactive flex h-8 shrink-0 items-center gap-2 rounded-[10px] pr-2.5 text-left text-xs outline-none disabled:cursor-default",
                      active ? "bg-jn-primary-wash font-semibold text-jn-primary-ink" : "text-jn-text enabled:hover:bg-jn-well",
                    )}
                  >
                    <Icon name={icon} size="sm" className={cn("shrink-0", active ? "text-jn-primary-ink" : "text-jn-text-3")} />
                    <span className="min-w-0 flex-1 truncate">{entry.dir ? `${name} /` : name}</span>
                    {!entry.dir && <span className="tnum shrink-0 text-[11px] text-jn-text-3">{formatBytes(entry.size)}</span>}
                  </button>
                )
              })}
              {detail?.files_truncated && <span className="px-2 text-[11px] text-jn-text-3">文件较多，只显示前 200 项</span>}
            </div>
            <span className="h-px bg-jn-line" />
            <Meta label="来源" value={sourceLabel(current.source)} />
            <Meta label="作用范围" value={current.scope === "project" ? `项目 · ${scopeLabel(current)}` : SCOPE_DETAIL[current.scope]} />
            <Meta label="最近修改" value={modifiedLabel(current.modified_ms)} />
            <Meta label="校验结果" value={current.issue ?? "名称、描述与目录结构有效"} danger={!!current.issue} />
            <div className="flex flex-col gap-1.5 rounded-[12px] bg-jn-well p-3">
              <span className="text-[11px] text-jn-text-3">{current.original_path ? "原所在目录" : "所在目录"}</span>
              <span className="break-all font-mono text-[11.5px] text-jn-text-2">{current.original_path ?? current.path}</span>
              {current.original_path && (
                <span className="text-[11px] text-jn-text-3">已移到 Jolu 停用区（工具不会加载）：<span className="break-all font-mono">{current.path}</span></span>
              )}
            </div>
            <Button kind="ghost" size="sm" icon="copy" className="self-start" onClick={() => void copyPath()}>复制路径</Button>
          </aside>

          <section className="flex min-w-0 flex-col gap-4 rounded-jn-card border border-jn-line bg-jn-surface p-6 shadow-jn-surface">
            <div className="flex flex-wrap items-center gap-3 border-b border-jn-line pb-3">
              <Tabs
                style="segment"
                value={tab}
                onChange={setTab}
                items={[
                  { key: "preview", label: "预览" },
                  { key: "source", label: isSkillFile ? "Markdown 源码" : "源码" },
                ]}
              />
              <span className="flex-1" />
              {!isSkillFile && <span className="truncate font-mono text-[11.5px] text-jn-text-3">{file}</span>}
            </div>
            <div ref={bodyMotion} className="min-h-0 flex-1 overflow-y-auto">
              {!detail || (!isSkillFile && !fileContent) ? (
                <LoadingSurface layout="messages" label="正在读取文件" />
              ) : text === null ? (
                <p className="text-xs text-jn-text-3">二进制文件不预览{fileContent?.size ? `（${formatBytes(fileContent.size)}）` : ""}</p>
              ) : tab === "preview" && markdown ? (
                <MarkdownPreview source={text} stripFront={isSkillFile} />
              ) : (
                <pre className="whitespace-pre-wrap break-words rounded-[14px] bg-jn-well p-4 font-mono text-[12px] leading-5 text-jn-text">{text}</pre>
              )}
              {(isSkillFile ? detail?.truncated : fileContent?.truncated) && <p className="mt-3 text-[11px] text-jn-text-3">文件较大，只显示前 512 KB</p>}
            </div>
          </section>
        </div>
      )}

      <Banner tone="info">
        启用不代表立即执行。保存或启停后的生效时机以工具自身的加载规则为准。
      </Banner>
    </div>
  )
}

function Meta({ label, value, danger }: { label: string; value: string; danger?: boolean }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-[11px] text-jn-text-3">{label}</span>
      <span className={cn("text-xs", danger ? "text-jn-danger" : "text-jn-text")}>{value}</span>
    </div>
  )
}
