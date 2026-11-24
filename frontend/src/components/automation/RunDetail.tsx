import { cn } from "@/lib/utils"
import {
  dayTime, durationText, hhmm, hhmmss, projectOf, runCode, runStatusLabel, runTokens, runningMs, toolName, waitingMs,
} from "@/lib/automation"
import type { RunDetailDto } from "@/lib/automationApi"
import { formatTokens } from "@/lib/usageStats"
import { Card } from "@/components/usage/usageControls"
import { KeyValue, SectionTitle } from "./automationParts"
import type { AccountRef } from "./TaskList"
import { MetricGrid } from "../ui/MetricGrid"
import { Icon, type IconName } from "@/icons"
import { Button, DataValue, MetricCard, StatusPill } from "@/ui"

const EVENT_ICONS: Record<string, IconName> = {
  start: "play",
  pause: "pause",
  wait: "clock",
  queue: "clock",
  check: "refresh",
  resume: "play",
  done: "success",
  fail: "fail",
  stop: "stop",
  attention: "danger",
}

/** 结果正文：以「- 」开头的行作为完成事项 */
function splitResult(text: string): { intro: string[]; items: string[] } {
  const intro: string[] = []
  const items: string[] = []
  for (const line of text.split(/\r?\n/)) {
    const t = line.trim()
    if (!t) continue
    const m = /^[-*•]\s+(.*)$/.exec(t)
    if (m) items.push(m[1])
    else intro.push(t)
  }
  return { intro: intro.slice(0, 8), items: items.slice(0, 12) }
}

/** 运行详情（R1-29）：本次运行 4 指标卡、结果、变更与日志，右栏竖向时间线与执行上下文。 */
export function RunDetail({ detail, now, account, onBack, onOpenSession, onStop, stopping }: {
  detail: RunDetailDto
  now: number
  account: AccountRef | undefined
  onBack: () => void
  onOpenSession: (platform: string, sessionId: string) => void
  onStop: () => void
  stopping: boolean
}) {
  const { run, task, events, logs } = detail
  const status = runStatusLabel(run.status)
  const exec = runningMs(run, now)
  const wait = waitingMs(run, now)
  const tokens = runTokens(run)
  const accountName = account?.name ?? run.account ?? "未知账号"
  const span = run.started_at ? `${dayTime(run.started_at, now)}${run.ended_at ? `–${hhmm(run.ended_at)}` : " 起"}` : `创建于${dayTime(run.created_at, now)}`
  const result = run.result ? splitResult(run.result) : null
  const resultIcon: IconName = run.status === "completed" ? "success" : run.status === "running" ? "loading" : run.status === "failed" ? "fail" : "danger"
  const resultTitle = { completed: "任务完成", failed: "运行失败", cancelled: "已停止运行", running: "正在执行", waiting: "等待额度", queued: "排队中", attention: "需要处理" }[run.status]
  const resultNote = run.status === "completed"
    ? (run.waits ? `经历 ${run.waits} 次额度等待` : "未经历额度等待")
    : run.status === "failed" ? "查看错误信息" : run.reason ?? (run.status === "running" ? "可查看日志并停止" : "—")

  return (
    <>
      <div className="flex flex-wrap items-center gap-2.5">
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="flex items-center gap-2.5">
            <h2 className="truncate text-[20px] font-bold text-jn-text">{run.task_name}</h2>
            <StatusPill tone={status.tone === "on" ? "success" : status.tone === "danger" ? "danger" : "neutral"}>
              {status.label}
            </StatusPill>
          </div>
          <span className="truncate text-xs text-jn-text-3">运行 {runCode(run.id)} · {span} · {toolName(run.platform)} / {accountName}</span>
        </div>
        <Button kind="ghost" size="sm" icon="arrowLeft" onClick={onBack}>返回记录</Button>
        {run.status === "running" && (
          <Button kind="danger" size="sm" icon="stop" loading={stopping} disabled={stopping} onClick={onStop}>
            {stopping ? "正在停止…" : "停止运行"}
          </Button>
        )}
        {run.session_id && run.platform !== "unknown" && (
          <Button kind="primary" size="sm" icon="external" onClick={() => onOpenSession(run.platform, run.session_id!)}>查看会话</Button>
        )}
      </div>

      <MetricGrid>
        <MetricCard
          title="运行结果"
          value={<span className={cn("text-[24px] font-bold leading-none", status.tone === "danger" ? "text-jn-danger" : status.tone === "on" ? "text-jn-primary-ink" : "text-jn-text")}>{status.label}</span>}
          compare={resultNote}
        />
        <MetricCard
          title="实际执行"
          value={<span className="tnum font-mono text-[24px] font-bold leading-none text-jn-text">{durationText(exec)}</span>}
          compare="不含等待时长"
        />
        <MetricCard
          title="等待额度"
          value={wait ? <span className="tnum font-mono text-[24px] font-bold leading-none text-jn-text">{durationText(wait)}</span> : <DataValue state="unknown" note="无等待" />}
          compare={run.waits ? (run.status === "completed" ? "已从原会话继续" : `${run.waits} 次等待`) : "未经历额度等待"}
        />
        <MetricCard
          title="消耗 Token"
          value={
            tokens === null ? (
              <DataValue state="unknown" note="未返回" />
            ) : (
              <DataValue state={tokens === 0 ? "zero" : "value"} value={tokens} format={(v) => formatTokens(v).replace(" ", "")} className="[&>span:first-child]:text-[24px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none" />
            )
          }
          compare={tokens === null ? "执行器未返回用量" : `输入 ${formatTokens(run.input_tokens).replace(" ", "")} / 输出 ${formatTokens(run.output_tokens).replace(" ", "")}`}
        />
      </MetricGrid>

      <div className="grid grid-cols-1 items-start gap-4 lg:grid-cols-[minmax(0,640fr)_minmax(0,340fr)]">
        <div className="flex min-w-0 flex-col gap-3.5">
          <Card className="flex flex-col gap-3 p-5">
            <div className="flex items-center gap-2.5">
              <span className={cn(
                "grid size-8 shrink-0 place-items-center rounded-[10px]",
                run.status === "failed" || run.status === "attention" ? "bg-jn-danger-soft text-jn-danger" : "bg-jn-primary-wash text-jn-primary-ink",
              )}>
                <Icon name={resultIcon} size="md" className={cn(run.status === "running" && "animate-spin")} />
              </span>
              <span className="text-base font-bold text-jn-text">{resultTitle}</span>
            </div>
            {result ? (
              <>
                {result.intro.map((line, i) => <p key={i} className="text-[13px] leading-[1.6] text-jn-text">{line}</p>)}
                {result.items.map((line, i) => (
                  <span key={i} className="flex items-start gap-2 text-xs text-jn-text">
                    <Icon name="check" size="xs" className="mt-0.5 shrink-0 text-jn-primary-ink" />
                    {line}
                  </span>
                ))}
              </>
            ) : run.error ? (
              <p className="whitespace-pre-wrap rounded-[12px] bg-jn-danger-soft p-3 text-xs leading-[1.6] text-jn-danger">{run.error}</p>
            ) : (
              <p className="text-xs text-jn-text-3">{run.reason ?? (run.status === "running" ? "执行器尚未返回结果。" : "执行器没有返回结果。")}</p>
            )}
          </Card>

          <Card className="flex flex-col gap-2.5 p-5">
            <SectionTitle title="变更与验证" note="执行器返回" />
            {run.files.length ? run.files.slice(0, 12).map((file) => (
              <div key={file} className="flex items-center gap-2.5 rounded-[10px] bg-jn-well px-3 py-2">
                <Icon name="file" size="sm" className="shrink-0 text-jn-text-3" />
                <span className="min-w-0 flex-1 truncate font-mono text-xs text-jn-text" title={file}>{file.split(/[\\/]/).pop()}</span>
                <span className="truncate font-mono text-[11px] text-jn-text-3" title={file}>{file.includes("/") || file.includes("\\") ? file.replace(/[\\/][^\\/]*$/, "") : "已修改"}</span>
              </div>
            )) : <span className="text-xs text-jn-text-3">执行器没有返回文件修改</span>}
            <div className="flex items-center gap-2.5 rounded-[10px] bg-jn-well px-3 py-2">
              <Icon name="listChecks" size="sm" className="shrink-0 text-jn-text-3" />
              <span className="flex-1 text-xs text-jn-text">相关测试</span>
              <StatusPill tone="neutral">未验证</StatusPill>
            </div>
            <span className="text-[11px] text-jn-text-3">仅展示实际执行器返回的结果；未返回验证信息时标记「未验证」。</span>
          </Card>

          <Card className="flex flex-col gap-2.5 p-5">
            <SectionTitle title="执行日志" note="完整内容见关联会话" />
            {logs.length ? (
              <pre className="max-h-[240px] overflow-auto whitespace-pre-wrap rounded-[12px] bg-jn-well p-3 font-mono text-[12px] leading-[1.7] text-jn-text-2">
                {logs.map((l) => `${hhmmss(l.ts)}  ${l.title}`).join("\n")}
              </pre>
            ) : <span className="text-xs text-jn-text-3">暂无执行日志</span>}
            {run.session_id && run.platform !== "unknown" && (
              <Button kind="soft" size="sm" icon="next" className="self-start" onClick={() => onOpenSession(run.platform, run.session_id!)}>打开完整会话</Button>
            )}
          </Card>
        </div>

        <div className="flex min-w-0 flex-col gap-3.5">
          <Card className="flex flex-col gap-3.5 p-5">
            <SectionTitle title="运行过程" note="同一次运行" />
            {events.length ? (
              <div className="flex flex-col">
                {events.map((e, i) => {
                  const iconName = EVENT_ICONS[e.kind] ?? "clock"
                  const isDanger = e.kind === "fail" || e.kind === "attention"
                  const isLast = i === events.length - 1
                  return (
                    <div key={i} className="relative flex gap-3 pb-4 last:pb-0">
                      {!isLast && <span aria-hidden className="absolute top-6 bottom-0 left-[11px] w-[1.5px] bg-jn-line" />}
                      <span className={cn(
                        "relative z-10 grid size-6 shrink-0 place-items-center rounded-full",
                        isDanger ? "bg-jn-danger-soft text-jn-danger" : "bg-jn-primary-wash text-jn-primary-ink",
                      )}>
                        <Icon name={iconName} size="xs" />
                      </span>
                      <div className="flex min-w-0 flex-1 flex-col gap-0.5 pt-0.5">
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-xs font-semibold text-jn-text">{e.title}</span>
                          <span className="tnum shrink-0 font-mono text-[11px] text-jn-text-3">{hhmmss(e.ts)}</span>
                        </div>
                        {e.detail && <span className="break-words text-[11.5px] text-jn-text-3">{e.detail}</span>}
                      </div>
                    </div>
                  )
                })}
              </div>
            ) : <span className="text-xs text-jn-text-3">暂无记录</span>}
          </Card>

          <Card className="flex flex-col gap-3 p-5">
            <SectionTitle title="执行上下文" />
            <div className="flex flex-col gap-2.5 rounded-[14px] bg-jn-well p-3.5">
              <KeyValue label="项目" value={projectOf(task?.cwd)} title={task?.cwd} />
              <KeyValue label="账号" value={accountName} />
              <KeyValue label="模型" value={run.model ?? "跟随工具默认"} />
              <KeyValue label="Skills" value={task ? task.skills.length ? task.skills.join(" / ") : "未选择" : "—"} />
            </div>
            <span className="text-[11px] text-jn-text-3">执行器、模型和账号以本次运行快照为准。</span>
          </Card>
        </div>
      </div>
      <p className="text-[11px] text-jn-text-3">自动化负责安排与记录运行；文件修改权限沿用任务执行器的明确授权。</p>
    </>
  )
}
