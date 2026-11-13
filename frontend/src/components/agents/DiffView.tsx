import { FileCode } from "lucide-react"
import { cn } from "@/lib/utils"
import type { WiringFileDiffDto } from "@/lib/gatewayApi"

/**
 * 写入预览 / 写入的改动（F6，画板 V5-2「写入预览」与 V5-1「查看差异」）。
 * 行高 24、等宽 11px；新增行绿底、删除行红底、折叠段灰字。密钥已在后端遮住。
 */
export function DiffView({ file, caption }: { file: WiringFileDiffDto; caption: string }) {
  return (
    <section className="flex flex-col overflow-hidden rounded-xl border border-jolu-border" aria-label={`${file.path} ${caption}`}>
      <header className="flex items-center gap-2 border-b border-jolu-table-divider bg-jolu-side px-3.5 py-2.5">
        <FileCode aria-hidden className="size-3.5 shrink-0 text-jolu-muted" />
        <span className="min-w-0 truncate font-mono text-meta text-jolu-text">{file.path}</span>
        <span className="shrink-0 text-meta text-jolu-muted">{caption}</span>
      </header>
      <div className="max-h-[360px] overflow-auto py-1">
        {file.lines.length === 0 && <p className="px-3.5 py-2 text-note text-jolu-muted">没有改动</p>}
        {file.lines.map((line, index) => (
          <div
            key={index}
            className={cn(
              "flex h-6 min-w-max items-center gap-2.5 px-3.5 font-mono text-note",
              line.op === "+" && "bg-jolu-status-success-bg",
              line.op === "-" && "bg-jolu-status-error-bg",
            )}
          >
            <span aria-hidden className={cn("w-2 shrink-0 font-bold", line.op === "+" ? "text-jolu-on" : line.op === "-" ? "text-jolu-danger" : "text-jolu-muted")}>
              {line.op === "…" ? "" : line.op.trim()}
            </span>
            <span className={cn("whitespace-pre", line.op === " " ? "text-jolu-muted" : line.op === "…" ? "italic text-jolu-muted" : "text-jolu-text")}>
              {line.op === "…" ? `… ${line.text} …` : line.text}
            </span>
          </div>
        ))}
      </div>
    </section>
  )
}
