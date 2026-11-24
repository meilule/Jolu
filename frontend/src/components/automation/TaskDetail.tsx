import { useEffect, useState } from "react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { observeActiveRequest } from "@/lib/activeRequest"
import {
  accountText, blockBanner, clock, dayTime, launchLabel, projectOf, runningMs, shortId, taskCode, taskStatus, toolName, untilText, windowLabel,
} from "@/lib/automation"
import { automationApi, type QuotaVerdictDto, type TaskAction, type TaskDetailDto } from "@/lib/automationApi"
import { localZone, zoneName } from "@/lib/usageStats"
import { demoQuota } from "@/mock/automation"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { Card } from "@/components/usage/usageControls"
import { KeyValue, QuotaBar, SectionTitle, TONE, TaskMenu } from "./automationParts"
import type { AccountRef } from "./TaskList"
import type { PlatformId } from "@/types"
import { Icon, type IconName } from "@/icons"
import { Banner, Button, IconButton, StatusPill } from "@/ui"

/** 任务详情：恢复时间与检查状态、继续执行的位置、执行内容，右栏为账号额度、同账号队列与等待策略。 */
export function TaskDetail({ detail, now, account, onBack, onAction, onEdit, onCancel, onDelete, onOpenRun, onOpenSession, onOpenProviders }: {
  detail: TaskDetailDto
  now: number
  account: AccountRef | undefined
  onBack: () => void
  onAction: (action: TaskAction) => Promise<void>
  onEdit: () => void
  onCancel: () => void
  onDelete: () => void
  onOpenRun: (runId: number) => void
  onOpenSession: (platform: string, sessionId: string) => void
  onOpenProviders: () => void
}) {
  const { item, execution, queue, events } = detail
  const run = item.active
  const last = item.last
  const status = taskStatus(item, now, zoneName(localZone()))
  const [busy, setBusy] = useState<TaskAction | null>(null)
  const [quotaResult, setQuotaResult] = useState<{ connectionId: string; quota: QuotaVerdictDto | null; error: string | null } | null>(null)
  const connectionId = execution?.connection_id ?? null
  const quota = quotaResult?.connectionId === connectionId ? quotaResult.quota : null
  const quotaError = quotaResult?.connectionId === connectionId ? quotaResult.error : null
  const accountName = (run ?? last)?.account ?? account?.name ?? "未知账号"
  const selectedRun = run ?? last
  const sessionId = selectedRun ? selectedRun.session_id : execution?.session_id
  const sessionPlatform = selectedRun ? selectedRun.platform : execution?.platform
  const canOpenSession = execution && sessionId && (sessionPlatform === "claude" || sessionPlatform === "codex")

  useEffect(() => {
    if (!connectionId) return
    if (!isTauri) {
      setQuotaResult({ connectionId, quota: demoQuota(Date.now()), error: null })
      return
    }
    return observeActiveRequest(
      automationApi.quota(connectionId, false),
      (value) => setQuotaResult({ connectionId, quota: value, error: null }),
      (reason) => setQuotaResult({ connectionId, quota: null, error: String(reason) }),
    )
  }, [connectionId, run?.last_check_at])

  const act = async (action: TaskAction) => {
    setBusy(action)
    try {
      await onAction(action)
    } finally {
      setBusy(null)
    }
  }
  const banner = run && (run.status === "waiting" || run.status === "attention") ? blockBanner(run, now) : null
  const pauseEvent = [...events].reverse().find((e) => e.kind === "pause")

  return (
    <>
      <div className="flex flex-wrap items-center gap-2.5">
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <h2 className="truncate text-[20px] font-bold text-jn-text">{execution?.name ?? selectedRun?.task_name ?? item.name}</h2>
          <span className="truncate text-xs text-jn-text-3">任务 {taskCode(item.id)} · {projectOf(execution?.cwd)} · 创建于{dayTime(item.created_at, now)}</span>
        </div>
        <Button kind="ghost" size="sm" icon="arrowLeft" onClick={onBack}>返回任务</Button>
        <TaskMenu
          item={item}
          now={now}
          onAction={(a) => void act(a)}
          onEdit={onEdit}
          onQueue={() => document.getElementById("automation-queue")?.scrollIntoView({ behavior: "smooth", block: "center" })}
          onCancel={onCancel}
          onDelete={onDelete}
          trigger={<IconButton icon="more" label="任务更多操作" size="sm" kind="ghost" />}
        />
      </div>

      {banner && (
        <Banner tone={banner.danger ? "danger" : "info"}>
          <span className="font-semibold">{banner.title}</span> — {banner.body}
        </Banner>
      )}

      <div className="grid grid-cols-1 items-start gap-4 lg:grid-cols-[minmax(0,640fr)_minmax(0,340fr)]">
        <div className="flex min-w-0 flex-col gap-3.5">
          <Card className="flex flex-col gap-3.5 p-5">
            <StatusCard item={detail.item} now={now} busy={busy} onAction={act} onCancel={onCancel} onOpenRun={onOpenRun} onOpenProviders={onOpenProviders} onEdit={onEdit} statusLabel={status.label} statusTone={status.tone} statusNext={status.next} />
          </Card>

          <Card className="flex flex-col gap-3 p-5">
            <SectionTitle title="继续执行的位置" note={sessionId ? "已关联原会话" : "尚未开始"} />
            <KeyValue label="执行工具 / 账号" value={`${toolName(execution?.platform ?? "unknown")} / ${accountName}`} />
            <KeyValue label="项目" value={execution?.cwd ?? "—"} title={execution?.cwd} />
            {execution && (selectedRun?.started_at || execution.session_id) ? (
              <>
                <KeyValue label="原会话" value={sessionId ? shortId(sessionId) : "新建会话（开始执行后关联）"} title={sessionId ?? undefined} />
                {pauseEvent && <KeyValue label="中断原因" value={pauseEvent.detail ?? pauseEvent.title} title={pauseEvent.detail ?? undefined} />}
                <Banner tone="info">
                  <span className="font-semibold">{execution?.auto_resume ? "恢复后继续当前会话" : "未开启自动续跑"}</span>：
                  {execution?.auto_resume
                    ? "保留已经完成的文件修改。执行器确认可以续跑后，从原会话继续；不支持时提示手动处理。"
                    : "执行中额度耗尽时结束本次运行，保留记录；可在编辑任务中开启「等待恢复后自动继续」。"}
                </Banner>
              </>
            ) : (
              <span className="text-xs text-jn-text-3">{execution ? "尚未开始：首次执行后显示原会话与中断位置。" : "—"}</span>
            )}
          </Card>

          <Card className="flex flex-col gap-2.5 p-5">
            <SectionTitle title="执行内容" />
            <p className="whitespace-pre-wrap rounded-[12px] bg-jn-well p-3.5 text-xs leading-[1.6] text-jn-text">{execution?.prompt ?? "—"}</p>
            {canOpenSession && (
              <Button kind="soft" size="sm" icon="next" className="self-start" onClick={() => onOpenSession(sessionPlatform!, sessionId!)}>查看关联会话</Button>
            )}
          </Card>
        </div>

        <div className="flex min-w-0 flex-col gap-3.5">
          <Card className="flex flex-col gap-4 p-5">
            <div className="flex items-center gap-2.5">
              {execution && <PlatformLogo platform={execution.platform as PlatformId} size={22} />}
              <div className="flex min-w-0 flex-col gap-0.5">
                <span className="truncate text-sm font-semibold text-jn-text">{accountName}</span>
                <span className="truncate text-[11px] text-jn-text-3">{account ? accountText(account.name, account.kind) : execution ? "账号已移除" : "—"}</span>
              </div>
            </div>
            {quota === null ? (
              <span className="flex items-center gap-1.5 text-xs text-jn-text-3">
                {!connectionId ? "—" : quotaError ? <>额度读取失败：{quotaError}</> : <><Icon name="loading" size="xs" className="animate-spin" />正在读取额度…</>}
              </span>
            ) : quota.windows.length ? quota.windows.slice(0, 3).map((w) => (
              <QuotaBar
                key={w.key}
                label={`${windowLabel(w)}已用`}
                value={`${w.used_percent === null ? "—" : `${Math.round(w.used_percent)}%`} · ${w.blocking ? "当前阻塞" : "可用"}`}
                note={w.resets_at ? `预计${dayTime(w.resets_at, now)} ${w.blocking ? "恢复" : "重置"}` : undefined}
                percent={w.used_percent}
                blocking={w.blocking}
              />
            )) : (
              <span className="text-xs text-jn-text-3">{quota.reason ?? "该账号没有可显示的额度窗口"}</span>
            )}
            <Button kind="soft" size="sm" icon="next" className="self-start" onClick={onOpenProviders}>查看账号额度</Button>
          </Card>

          <Card className="flex flex-col gap-3 p-5">
            <div id="automation-queue" className="contents" />
            <SectionTitle title={`${accountName} 的队列`} note={`${queue.length} 个任务`} />
            {queue.length ? queue.map((q, index) => (
              <span key={`${q.task_id}-${index}`} className={cn("truncate text-xs", q.task_id === item.id ? "font-semibold text-jn-primary-ink" : "text-jn-text")}>
                {String(index + 1).padStart(2, "0")}  {q.name} · {q.status === "running" ? "执行中" : q.status === "waiting" ? "等待额度" : "排队中"}
              </span>
            )) : <span className="text-xs text-jn-text-3">{connectionId ? "当前账号没有排队的任务" : "—"}</span>}
            <span className="text-[11px] text-jn-text-3">同一账号依次执行。额度恢复后，先继续当前任务。</span>
          </Card>

          <Card className="flex flex-col gap-3 p-5">
            <SectionTitle title="等待策略" />
            <KeyValue label="启动方式" value={execution ? launchLabel(execution) : "—"} />
            <KeyValue label="等待截止" value={execution ? execution.wait_deadline ? dayTime(execution.wait_deadline, now) : "不限制" : "—"} />
            <KeyValue label="恢复方式" value={execution ? execution.auto_resume ? "自动检查后续跑" : "不自动续跑" : "—"} />
            <KeyValue label="重复运行" value="禁止重叠执行" />
            <span className="text-[11px] text-jn-text-3">电脑休眠期间不检查；唤醒或重启 Jolu 后重新检查可用额度。</span>
          </Card>
        </div>
      </div>
      <p className="text-[11px] text-jn-text-3">新建且从未执行的任务显示「尚未开始」，不展示中断步骤与原会话；实际续跑能力按执行工具开放。</p>
    </>
  )
}

function StatusCard({ item, now, busy, onAction, onCancel, onOpenRun, onOpenProviders, onEdit, statusLabel, statusTone, statusNext }: {
  item: TaskDetailDto["item"]
  now: number
  busy: TaskAction | null
  onAction: (action: TaskAction) => void
  onCancel: () => void
  onOpenRun: (runId: number) => void
  onOpenProviders: () => void
  onEdit: () => void
  statusLabel: string
  statusTone: "on" | "text" | "danger" | "muted"
  statusNext: string
}) {
  const run = item.active
  const spin = (action: TaskAction) => busy === action
  const header = (icon: IconName, title: string, spinning = false, danger = false) => (
    <div className="flex items-center gap-2.5">
      <Icon name={icon} size="lg" className={cn(danger ? "text-jn-danger" : "text-jn-primary-ink", spinning && "animate-spin")} />
      <span className="text-[13px] font-medium text-jn-text-3">{title}</span>
      <span className="flex-1" />
      <StatusPill tone={statusTone === "danger" ? "danger" : statusTone === "on" ? "success" : "neutral"}>{statusLabel}</StatusPill>
    </div>
  )
  const cancelButton = item.state !== "cancelled" && (
    <Button kind="danger" size="sm" icon="close" disabled={!!busy} onClick={onCancel}>取消任务</Button>
  )
  const pauseButton = item.state === "paused"
    ? <Button kind="ghost" size="sm" icon="play" disabled={!!busy} onClick={() => onAction("resume")}>恢复任务</Button>
    : item.state === "active" && <Button kind="ghost" size="sm" icon="pause" disabled={!!busy} onClick={() => onAction("pause")}>{run?.status === "waiting" ? "暂停等待" : "暂停任务"}</Button>

  if (run?.status === "waiting") {
    const checking = spin("check")
    return (
      <>
        {header("clock", run.resets_at ? "预计恢复" : "恢复时间")}
        <span className="tnum font-mono text-[30px] font-bold leading-none text-jn-text">{run.resets_at ? dayTime(run.resets_at, now) : "未知"}</span>
        <span className="text-xs text-jn-text-3">{run.resets_at ? `${untilText(run.resets_at, now)} · 预计时间不代表已可执行` : "暂未获取重置时间，任务保持等待"}</span>
        <div className="flex gap-5 text-xs text-jn-text-3">
          <span>上次检查 {run.last_check_at ? dayTime(run.last_check_at, now).replace("今天 ", "") : "—"}</span>
          <span>下次检查 {run.next_check_at ? dayTime(run.next_check_at, now).replace("今天 ", "") : "—"}</span>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button kind="primary" size="sm" icon="refresh" loading={checking} disabled={!!busy || item.state === "paused"} onClick={() => onAction("check")}>
            {checking ? "检查中…" : "立即检查"}
          </Button>
          {pauseButton}
          {cancelButton}
        </div>
      </>
    )
  }
  if (run?.status === "running") {
    return (
      <>
        {header("loading", "正在执行", true)}
        <span className="tnum font-mono text-[30px] font-bold leading-none text-jn-text">{clock(runningMs(run, now), false)}</span>
        <span className="text-xs text-jn-text-3">实际执行时长，不含等待额度的时间</span>
        <div className="flex flex-wrap gap-2">
          <Button kind="soft" size="sm" icon="next" onClick={() => onOpenRun(run.id)}>查看日志</Button>
          <Button kind="danger" size="sm" icon="stop" disabled={!!busy} onClick={() => onAction("stop")}>停止运行</Button>
        </div>
      </>
    )
  }
  if (run?.status === "queued") {
    return (
      <>
        {header("clock", "排队中")}
        <span className="text-[30px] font-bold leading-none text-jn-text">{item.queue_ahead ? `前方 ${item.queue_ahead} 个任务` : "即将开始"}</span>
        <span className="text-xs text-jn-text-3">同一账号按队列顺序执行，前一个任务结束后自动开始。</span>
        <div className="flex flex-wrap gap-2">{pauseButton}{cancelButton}</div>
      </>
    )
  }
  if (run?.status === "attention") {
    const reauth = run.block_kind === "reauth"
    const accountIssue = run.block_kind === "account_mismatch"
    return (
      <>
        {header("danger", "需要处理", false, true)}
        <span className="text-xs leading-[1.6] text-jn-text">{run.reason ?? "查看详情后处理"}</span>
        <div className="flex flex-wrap gap-2">
          {reauth || accountIssue ? (
            <Button kind="soft" size="sm" icon={reauth ? "connect" : "next"} onClick={onOpenProviders}>{reauth ? "重新登录" : "前往供应商"}</Button>
          ) : null}
          <Button kind="primary" size="sm" icon="retry" disabled={!!busy || item.state !== "active" || run.block_kind === "snapshot_missing"} onClick={() => onAction("retry")}>重新排队</Button>
          {cancelButton}
        </div>
      </>
    )
  }
  if (item.launch === "schedule" && item.state === "active" && item.next_run_at) {
    return (
      <>
        {header("clock", "下次运行")}
        <span className="tnum font-mono text-[30px] font-bold leading-none text-jn-text">{dayTime(item.next_run_at, now)}</span>
        <span className="text-xs text-jn-text-3">{launchLabel(item)} · {zoneName(localZone())}时间 · 仍在执行时跳过本次触发</span>
        <div className="flex flex-wrap gap-2">
          <Button kind="primary" size="sm" icon="play" disabled={!!busy} onClick={() => onAction("run")}>立即运行</Button>
          {pauseButton}
          {cancelButton}
        </div>
      </>
    )
  }
  const last = item.last
  return (
    <>
      {header("clock", last ? "最近运行" : "尚未开始")}
      <span className={cn("text-[30px] font-bold leading-none", TONE[statusTone])}>{statusLabel}</span>
      <span className="text-xs text-jn-text-3">{statusNext}</span>
      <div className="flex flex-wrap gap-2">
        {last && <Button kind="soft" size="sm" icon="next" onClick={() => onOpenRun(last.id)}>查看运行记录</Button>}
        {item.state === "active" || item.state === "finished" ? (
          <Button kind="primary" size="sm" icon="play" disabled={!!busy} onClick={() => onAction(item.missed_at ? "run" : last ? "retry" : "run")}>
            {item.missed_at ? "补跑" : last ? "再次运行" : "立即运行"}
          </Button>
        ) : null}
        {item.state === "paused" && pauseButton}
        {item.state !== "cancelled" && <Button kind="ghost" size="sm" icon="edit" onClick={onEdit}>编辑任务</Button>}
        {cancelButton}
      </div>
    </>
  )
}
