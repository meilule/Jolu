import { useEffect, useState } from "react"
import { isTauri, listenEvent } from "@/lib/api"
import { formatBytes, progressPercent } from "@/lib/updateState"
import { useUpdate } from "./UpdateContext"
import { AUTOMATION_CHANGED, automationApi } from "@/lib/automationApi"
import { cn } from "@/lib/utils"
import { Icon } from "@/icons"
import { Banner, Button, Dialog, Progress } from "@/ui"

/** 更新进度对话框：下载、校验与安装重启。 */
export function UpdateDialog({ onViewTasks }: { onViewTasks: () => void }) {
  const { dialogOpen, closeDialog, cancelDownload, phase, info, progress, error, startDownload, installDownloaded } = useUpdate()
  const [running, setRunning] = useState<number | null>(null)
  const [taskCheckError, setTaskCheckError] = useState(false)

  useEffect(() => {
    if (!dialogOpen || phase !== "downloaded") return
    let stopped = false
    let off: (() => void) | undefined
    const check = () => {
      if (!isTauri) { setRunning(0); return }
      void automationApi.overview().then((value) => {
        if (!stopped) { setRunning(value.running); setTaskCheckError(false) }
      }).catch(() => { if (!stopped) { setRunning(null); setTaskCheckError(true) } })
    }
    setRunning(null)
    setTaskCheckError(false)
    check()
    void listenEvent(AUTOMATION_CHANGED, check).then((unsubscribe) => {
      if (stopped) unsubscribe()
      else off = unsubscribe
    })
    return () => { stopped = true; off?.() }
  }, [dialogOpen, phase])

  if (!info || !dialogOpen) return null
  if (phase !== "downloading" && phase !== "downloadError" && phase !== "downloaded" && phase !== "installing" && phase !== "ready") return null

  const percent = progressPercent(progress.downloaded, progress.total)
  const installing = phase === "installing"
  const downloaded = phase === "downloaded"
  const downloadError = phase === "downloadError"
  const ready = phase === "ready"
  const tasksRunning = downloaded && ((running ?? 0) > 0 || (running === null && !!error?.includes("自动化任务")))
  const bytes = progress.total === null
    ? formatBytes(progress.downloaded)
    : `${formatBytes(progress.downloaded)} / ${formatBytes(progress.total)}`

  const title = ready
    ? "安装完成"
    : installing
      ? "正在安装"
      : downloaded
        ? `准备安装 v${info.availableVersion}`
        : downloadError
          ? "下载失败"
          : `正在下载 v${info.availableVersion}`

  const subtitle = ready
    ? isTauri ? `v${info.availableVersion} 已安装，应用即将自动重启` : "预览模式：安装流程已模拟完成"
    : installing
      ? "安装器正在接管，完成后应用自动重启"
      : downloaded
        ? "安装包已下载并通过签名校验，点击安装即可完成更新"
        : downloadError
          ? "请重试下载更新包"
          : "正在下载并校验更新包"

  return (
    <Dialog
      open
      onOpenChange={(next) => { if (!next && !installing) closeDialog() }}
      preventClose={installing}
      width="sm"
      title={
        <span className="flex items-center gap-3">
          <span className={cn("grid size-9 shrink-0 place-items-center rounded-full", downloadError ? "bg-jn-danger-soft text-jn-danger" : "bg-jn-primary-wash text-jn-primary-ink", installing && "animate-pulse")}>
            <Icon name={ready || downloaded ? "check" : downloadError ? "download" : "loading"} size="md" className={cn(!ready && !downloaded && !downloadError && "animate-spin")} />
          </span>
          <span>{title}</span>
        </span>
      }
      description={subtitle}
      actions={
        phase === "downloading" ? (
          <Button kind="ghost" size="md" onClick={cancelDownload}>取消下载</Button>
        ) : (downloaded || downloadError || ready) ? (
          <>
            <Button kind="ghost" size="md" onClick={closeDialog}>
              {downloaded ? "稍后安装" : "关闭"}
            </Button>
            {!ready && (
              <Button
                kind="primary"
                size="md"
                onClick={downloadError ? startDownload : tasksRunning ? () => { closeDialog(); onViewTasks() } : installDownloaded}
                disabled={downloaded && running === null && !taskCheckError && !error}
              >
                {downloadError ? "重试下载" : tasksRunning ? "查看任务" : "重启并安装"}
              </Button>
            )}
          </>
        ) : undefined
      }
    >
      <div className="tnum flex items-center gap-2 rounded-[14px] bg-jn-well px-3.5 py-2.5 font-mono text-xs">
        <span className="text-jn-text-3">v{info.currentVersion}</span>
        <span aria-hidden className="text-jn-text-3">→</span>
        <span className="font-bold text-jn-primary-ink">v{info.availableVersion}</span>
        <span className="flex-1" />
        {info.pubDate && <span className="font-sans text-[11px] text-jn-text-3">{info.pubDate.slice(0, 10)} 发布</span>}
      </div>

      {info.notes && (
        <div className="flex flex-col gap-1.5 rounded-[14px] bg-jn-well p-3.5">
          <span className="text-xs font-semibold text-jn-text">更新说明</span>
          <p className="text-xs leading-relaxed text-jn-text-2">{info.notes}</p>
        </div>
      )}

      {phase === "downloading" && (
        <div className="flex flex-col gap-2">
          <Progress percent={percent === null ? 40 : percent} label="下载进度" />
          <div className="tnum flex justify-between font-mono text-xs">
            <span className="text-jn-text-2">{bytes}</span>
            <span className="font-bold text-jn-primary-ink">{percent === null ? "…" : `${percent}%`}</span>
          </div>
        </div>
      )}

      {downloaded && !error && (
        <Banner tone={tasksRunning ? "warning" : "info"}>
          {tasksRunning
            ? `还有 ${running} 个自动化任务正在运行，请先处理任务。`
            : taskCheckError
              ? "任务状态暂时无法读取，安装时将再次检查。"
              : running === null ? "正在检查运行中的任务…" : "没有运行中的自动化任务，可以安装更新。"}
        </Banner>
      )}

      {error && (
        <Banner tone="danger">
          <span role="alert">{error}</span>
        </Banner>
      )}

      <p className="text-center text-xs text-jn-text-3">
        {ready
          ? "灵动岛与后台采集会一并恢复"
          : installing ? "请等待安装器完成，应用将自动重启"
            : downloaded ? "点击下方安装按钮后无需再次确认"
              : downloadError ? "网络恢复后可直接重试"
                : "下载期间不会中断用量采集"}
      </p>
    </Dialog>
  )
}
