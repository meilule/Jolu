import { Icon } from "@/icons"
import { cn } from "@/lib/utils"
import type { WiringFileDiffDto } from "@/lib/gatewayApi"

/**
 * 写入预览 / 写入的改动 —— 设计稿 R1-33「差异」卡。
 *
 * 结构：文件头（44 高、`jn-well` 底）+ 行列表（24 高、行号列 + 标记列 + 正文）。
 * 新增行 `jn-primary-wash`、删除行 `jn-danger-soft`、折叠段斜体灰字。
 * 密钥已在后端遮住，这里只负责显示。
 *
 * 画板文件头右侧是 `+6` / `−1` 计数；`WiringFileDiffDto` 没有行号字段，
 * 行号列因此不显示数字（画板有，数据没有，标「待确认」）。
 */
export function DiffView({ file, caption }: { file: WiringFileDiffDto; caption: string }) {
  const added = file.lines.filter((line) => line.op === "+").length
  const removed = file.lines.filter((line) => line.op === "-").length
  return (
    <section className="flex min-h-0 flex-col overflow-hidden rounded-jn-card border border-jn-line bg-jn-surface" aria-label={`${file.path} ${caption}`}>
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-jn-line bg-jn-well px-4">
        <Icon name="file" size="sm" className="shrink-0 text-jn-text-3" />
        <span className="min-w-0 truncate font-mono text-[12.5px] font-semibold text-jn-text">{file.path}</span>
        <span className="min-w-0 flex-1" />
        <span className="shrink-0 text-[11px] text-jn-text-3">{caption}</span>
        {added > 0 && <span className="shrink-0 text-[12px] font-semibold text-jn-primary">+{added}</span>}
        {removed > 0 && <span className="shrink-0 text-[12px] font-semibold text-jn-danger">−{removed}</span>}
      </header>
      <div className="max-h-[420px] overflow-auto py-2">
        {file.lines.length === 0 && <p className="px-4 py-2 text-[11px] text-jn-text-3">没有改动</p>}
        {file.lines.map((line, index) => (
          <div
            key={index}
            className={cn(
              "flex h-6 min-w-max items-center gap-2.5 px-4 font-mono text-[11.5px]",
              line.op === "+" && "bg-jn-primary-wash",
              line.op === "-" && "bg-jn-danger-soft",
            )}
          >
            <span className={cn("w-2 shrink-0 text-[11px] font-bold", line.op === "+" ? "text-jn-primary" : line.op === "-" ? "text-jn-danger" : "text-jn-text-3")}>
              {line.op === "…" ? "" : line.op.trim()}
            </span>
            <span className={cn("whitespace-pre", line.op === " " ? "text-jn-text-3" : line.op === "…" ? "text-jn-text-3 italic" : "text-jn-text")}>
              {line.op === "…" ? `… ${line.text} …` : line.text}
            </span>
          </div>
        ))}
      </div>
    </section>
  )
}
