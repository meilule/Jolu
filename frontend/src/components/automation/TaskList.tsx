import { useEffect, useMemo, useState } from "react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { observeActiveRequest } from "@/lib/activeRequest"
import { accountText, countByFilter, dayTime, filterTasks, launchLabel, projectOf, taskStatus, toolName, windowLabel, type TaskFilter } from "@/lib/automation"
import { automationApi, type QuotaVerdictDto, type TaskAction, type TaskItemDto } from "@/lib/automationApi"
import { zoneName, localZone } from "@/lib/usageStats"
import { demoQuota } from "@/mock/automation"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { Card, MenuFilter } from "@/components/usage/usageControls"
import { TaskMenu } from "./automationParts"
import type { PlatformId } from "@/types"
import { MetricGrid } from "../ui/MetricGrid"
import { Icon } from "@/icons"
import { Button, DataValue, EmptyState, Input, MetricCard, QuotaRing, StatusPill, Tabs, type StatusTone } from "@/ui"

const FILTERS: { value: TaskFilter; label: string }[] = [
  { value: "running", label: "执行中" },
  { value: "waiting", label: "等待额度" },
  { value: "paused", label: "已暂停" },
  { value: "done", label: "已完成" },
]

export interface AccountRef {
  name: string
  kind: string
}

function pillTone(tone: "on" | "text" | "danger" | "muted"): StatusTone {
  if (tone === "on") return "success"
  if (tone === "danger") return "danger"
  return "neutral"
}

/** 任务页签（R1-10）：四枚指标卡 + 主次两栏（左侧筛选与任务表，右侧 300px 等待额度环与「接下来」排期）。 */
export function TaskList({ data, now, accounts, onOpen, onCreate, onEdit, onAction, onCancel, onDelete }: {
  data: { tasks: TaskItemDto[]; enabled: number; running: number; running_note: string | null; waiting: number; next_reset: number | null; done_today: number; failed_today: number }
  now: number
  accounts: Map<string, AccountRef>
  onOpen: (item: TaskItemDto, target: "task" | "run") => void
  onCreate: () => void
  onEdit: (item: TaskItemDto) => void
  onAction: (item: TaskItemDto, action: TaskAction) => void
  onCancel: (item: TaskItemDto) => void
  onDelete: (item: TaskItemDto) => void
}) {
  const [filter, setFilter] = useState<TaskFilter>("all")
  const [account, setAccount] = useState<string>("__all__")
  const [search, setSearch] = useState("")
  const zone = zoneName(localZone())
  const counts = countByFilter(data.tasks, now)
  const rows = filterTasks(data.tasks, filter, account === "__all__" ? null : account, search, now)
  const accountIds = [...new Set(data.tasks.map((t) => t.connection_id))]
  const filtered = filter !== "all" || account !== "__all__" || !!search.trim()
  const clear = () => {
    setFilter("all")
    setAccount("__all__")
    setSearch("")
  }

  const waitingTask = useMemo(
    () => data.tasks.find((t) => t.active?.status === "waiting") ?? null,
    [data.tasks],
  )
  const upcoming = useMemo(() => {
    return data.tasks
      .filter((t) => t.state === "active" && (t.active?.status === "waiting" || t.active?.status === "queued" || t.next_run_at != null))
      .sort((a, b) => {
        const at = a.active?.resets_at ?? a.next_run_at ?? Number.MAX_SAFE_INTEGER
        const bt = b.active?.resets_at ?? b.next_run_at ?? Number.MAX_SAFE_INTEGER
        return at - bt
      })
      .slice(0, 4)
  }, [data.tasks])

  return (
    <div className="@container flex min-w-0 flex-col gap-3.5">
      <MetricGrid>
        <MetricCard
          title={<span className="inline-flex items-center gap-1.5"><Icon name="listChecks" size="sm" className="text-jn-text-3" />已启用任务</span>}
          value={<DataValue state={data.enabled === 0 ? "zero" : "value"} value={data.enabled} unit="个" className="[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none" />}
          compare="含定时与额度恢复任务"
        />
        <MetricCard
          title={<span className="inline-flex items-center gap-1.5"><Icon name="play" size="sm" className="text-jn-primary" />执行中</span>}
          value={<DataValue state={data.running === 0 ? "zero" : "value"} value={data.running} unit="个" className="[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none" />}
          compare={data.running_note ?? "当前没有执行中的任务"}
        />
        <MetricCard
          title={<span className="inline-flex items-center gap-1.5"><Icon name="hourglass" size="sm" className="text-jn-warning" />等待额度</span>}
          value={<DataValue state={data.waiting === 0 ? "zero" : "value"} value={data.waiting} unit="个" className="[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none" />}
          compare={data.next_reset ? `最近预计 ${dayTime(data.next_reset, now).replace("今天 ", "")} 恢复` : data.waiting ? "恢复时间未知" : "没有等待中的任务"}
        />
        <MetricCard
          title={<span className="inline-flex items-center gap-1.5"><Icon name="success" size="sm" className="text-jn-primary" />今日完成</span>}
          value={<DataValue state={data.done_today === 0 ? "zero" : "value"} value={data.done_today} unit="次" className="[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none" />}
          compare={data.failed_today ? `另有 ${data.failed_today} 次运行失败` : "今天没有失败的运行"}
        />
      </MetricGrid>

      {data.tasks.length === 0 ? (
        <Card className="p-5">
          <EmptyState
            icon="bot"
            title="还没有自动化任务"
            reason="创建一个任务，让 AI 在指定时间或额度恢复后开始工作。"
            action={<Button kind="primary" size="sm" icon="add" onClick={onCreate}>新建任务</Button>}
          />
        </Card>
      ) : (
        <div className="grid grid-cols-1 items-start gap-3.5 @[1000px]:grid-cols-[minmax(0,1fr)_284px] @[1120px]:grid-cols-[minmax(0,1fr)_300px]">
          <div className="flex min-w-0 flex-col gap-3">
            <div className="flex flex-wrap items-center gap-2">
              <Tabs
                style="segment"
                value={filter}
                onChange={setFilter}
                className="border border-jn-line/80"
                items={[
                  { key: "all" as const, label: `全部 ${counts.all}` },
                  ...FILTERS.map((f) => ({
                    key: f.value,
                    label: counts[f.value] > 0 && f.value !== "done" ? `${f.label} ${counts[f.value]}` : f.label,
                  })),
                ]}
              />
              <span className="min-w-0 flex-1" />
              <MenuFilter
                label="账号筛选"
                trailingChevron
                value={account}
                onChange={setAccount}
                options={[{ value: "__all__", label: "全部账号" }, ...accountIds.map((id) => ({ value: id, label: accounts.get(id)?.name ?? "未知账号" }))]}
              />
              <Input
                size="md"
                icon="search"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Escape") setSearch("") }}
                placeholder="搜索任务或项目"
                aria-label="搜索任务"
                className="w-[184px] shrink-0 bg-jn-surface shadow-jn-surface"
              />
            </div>

            <Card className="flex flex-col gap-1 p-3">
              <div className="overflow-x-auto">
                <div className="flex min-w-[580px] flex-col gap-1">
                  <div className="flex h-8 items-center gap-2 px-2.5 text-[11.5px] font-medium text-jn-text-3">
                    <span className="min-w-[128px] flex-1">任务 / 项目</span>
                    <span className="w-[124px] shrink-0">执行工具 / 账号</span>
                    <span className="w-[80px] shrink-0">启动方式</span>
                    <span className="w-[116px] shrink-0">状态 / 下一步</span>
                    <span className="w-[126px] shrink-0 text-right">操作</span>
                  </div>
                  {rows.length ? rows.map((item, index) => {
                    const status = taskStatus(item, now, zone)
                    const acc = accounts.get(item.connection_id)
                    return (
                      <div key={item.id} className={cn("jn-interactive flex min-h-[60px] items-center gap-2 rounded-[14px] px-2.5 py-2 hover:bg-jn-primary-wash/50", index % 2 === 0 && "bg-jn-well")}>
                        <div className="flex min-w-[128px] flex-1 flex-col gap-0.5 pr-1">
                          <span className="truncate text-[13px] font-semibold text-jn-text">{item.name}</span>
                          <span className="truncate text-[11px] text-jn-text-3" title={item.cwd}>{projectOf(item.cwd)}</span>
                        </div>
                        <div className="flex w-[124px] shrink-0 flex-col gap-0.5 pr-1">
                          <span className="flex items-center gap-1.5 text-xs text-jn-text"><PlatformLogo platform={item.platform as PlatformId} size={14} /><span className="truncate">{toolName(item.platform)}</span></span>
                          <span className="truncate text-[11px] text-jn-text-3">{acc ? accountText(acc.name, acc.kind) : "账号已移除"}</span>
                        </div>
                        <span className="w-[80px] shrink-0 truncate text-xs text-jn-text-2">{launchLabel(item)}</span>
                        <div className="flex w-[116px] shrink-0 flex-col items-start gap-1">
                          <StatusPill tone={pillTone(status.tone)}>{status.label}</StatusPill>
                          <span className="w-full truncate text-[11px] text-jn-text-3" title={status.next}>{status.next}</span>
                        </div>
                        <div className="flex w-[126px] shrink-0 items-center justify-end gap-1">
                          <Button kind="soft" size="sm" onClick={() => onOpen(item, status.action === "查看运行" ? "run" : "task")}>
                            {status.action}
                          </Button>
                          <TaskMenu
                            item={item}
                            now={now}
                            onAction={(action) => onAction(item, action)}
                            onEdit={() => onEdit(item)}
                            onQueue={() => onOpen(item, "task")}
                            onCancel={() => onCancel(item)}
                            onDelete={() => onDelete(item)}
                          />
                        </div>
                      </div>
                    )
                  }) : (
                    <div className="flex items-center gap-3 rounded-[12px] bg-jn-well px-4 py-6 text-xs text-jn-text-3">
                      <span>筛选无结果：没有匹配的任务</span>
                      {filtered && <Button kind="ghost" size="sm" icon="retry" onClick={clear}>清除筛选</Button>}
                    </div>
                  )}
                </div>
              </div>
              <div className="flex h-10 items-center gap-2.5 border-t border-jn-line px-3 text-xs text-jn-text-3">
                <span>{filtered ? `匹配 ${rows.length} / 共 ${data.tasks.length} 个任务` : `共 ${data.tasks.length} 个任务`}</span>
                <span className="flex-1" />
                <span>按最近更新排序</span>
              </div>
            </Card>
          </div>

          {/* R1-10 右侧 300px 侧栏：等待额度环卡 + 接下来排期卡 */}
          <aside className="flex min-w-0 flex-col gap-3.5">
            {waitingTask ? (
              <WaitingQuotaCard
                task={waitingTask}
                account={accounts.get(waitingTask.connection_id)}
                now={now}
                onRunNow={() => onAction(waitingTask, "run")}
                onCancelWait={() => onCancel(waitingTask)}
                onOpen={() => onOpen(waitingTask, "task")}
              />
            ) : (
              <Card className="flex flex-col gap-2.5 p-4">
                <div className="flex items-center gap-2">
                  <Icon name="hourglass" size="sm" className="text-jn-text-3" />
                  <span className="text-sm font-semibold text-jn-text">额度等待</span>
                  <span className="flex-1" />
                  <StatusPill tone="neutral">无等待</StatusPill>
                </div>
                <p className="text-xs leading-relaxed text-jn-text-3">
                  当前没有等待额度恢复的任务。创建「额度恢复后执行」任务或开启自动续跑后，等待状态与重置倒计时会展示在这里。
                </p>
              </Card>
            )}

            <Card className="flex flex-col gap-2.5 p-4">
              <div className="flex items-center gap-2">
                <Icon name="clock" size="sm" className="text-jn-text-3" />
                <span className="text-sm font-semibold text-jn-text">接下来</span>
                <span className="flex-1" />
                <span className="rounded-full bg-jn-well px-2 py-0.5 text-[11px] font-semibold text-jn-text-2">{upcoming.length}</span>
              </div>
              {upcoming.length ? (
                <div className="flex flex-col gap-1.5">
                  {upcoming.map((task) => {
                    const acc = accounts.get(task.connection_id)
                    const timeText = task.active?.status === "waiting"
                      ? (task.active.resets_at ? dayTime(task.active.resets_at, now).replace("今天 ", "") : "等待重置")
                      : task.active?.status === "queued"
                        ? (task.queue_ahead ? `前方 ${task.queue_ahead} 个` : "排在上一个之后")
                        : task.next_run_at
                          ? dayTime(task.next_run_at, now).replace("今天 ", "")
                          : "—"
                    return (
                      <button
                        key={task.id}
                        type="button"
                        onClick={() => onOpen(task, "task")}
                        className="jn-interactive flex items-center gap-2.5 rounded-[12px] bg-jn-well px-3 py-2.5 text-left hover:bg-jn-primary-wash/60"
                      >
                        <PlatformLogo platform={task.platform as PlatformId} size={18} />
                        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                          <span className="truncate text-xs font-semibold text-jn-text">{task.name}</span>
                          <span className="truncate text-[11px] text-jn-text-3">
                            {launchLabel(task)}{acc ? ` · ${acc.name}` : ""}
                          </span>
                        </span>
                        <span className="tnum shrink-0 font-mono text-[11px] font-semibold text-jn-primary-ink">{timeText}</span>
                      </button>
                    )
                  })}
                </div>
              ) : (
                <p className="py-2 text-xs text-jn-text-3">暂无排期或排队中的任务。</p>
              )}
            </Card>
          </aside>
        </div>
      )}
    </div>
  )
}

function WaitingQuotaCard({ task, account, now, onRunNow, onCancelWait, onOpen }: {
  task: TaskItemDto
  account: AccountRef | undefined
  now: number
  onRunNow: () => void
  onCancelWait: () => void
  onOpen: () => void
}) {
  const [quota, setQuota] = useState<QuotaVerdictDto | null>(null)
  useEffect(() => {
    if (!task.connection_id) return
    if (!isTauri) {
      setQuota(demoQuota(Date.now()))
      return
    }
    return observeActiveRequest(
      automationApi.quota(task.connection_id, false),
      setQuota,
      () => setQuota(null),
    )
  }, [task.connection_id, task.active?.last_check_at])

  const blocking = quota?.windows.find((w) => w.blocking) ?? quota?.windows[0] ?? null
  const usedPercent = blocking && typeof blocking.used_percent === "number" ? Math.round(blocking.used_percent) : null
  const resetAt = task.active?.resets_at ?? blocking?.resets_at ?? null
  const accountLabel = account?.name ?? task.active?.account ?? "所选账号"
  const windowName = blocking ? windowLabel(blocking) : "额度窗口"

  return (
    <Card className="flex flex-col gap-3 p-4">
      <div className="flex items-start gap-3">
        <QuotaRing size="md" usedPercent={usedPercent} />
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <button type="button" onClick={onOpen} className="truncate text-left text-sm font-bold text-jn-text hover:text-jn-primary-ink">
            {task.name}
          </button>
          <span className="truncate text-xs text-jn-text-3">{toolName(task.platform)} · {accountLabel}</span>
          <StatusPill tone="warning">{windowName}</StatusPill>
        </div>
      </div>
      <p className="text-xs leading-relaxed text-jn-text-2">
        {usedPercent !== null
          ? `${accountLabel} 的${windowName}已用 ${usedPercent}%，${resetAt ? `预计${dayTime(resetAt, now)}重置` : "重置时间待更新"}。重置后用同一账号自动续跑，执行前会再检查一次 Skills 和登录账号。`
          : `${resetAt ? `预计${dayTime(resetAt, now)}恢复` : "正在等待额度恢复"}，恢复后用同一账号继续执行。`}
      </p>
      <div className="flex items-center gap-2 pt-0.5">
        <Button kind="primary" size="sm" icon="play" className="flex-1" onClick={onRunNow}>现在就运行</Button>
        <Button kind="ghost" size="sm" onClick={onCancelWait}>取消等待</Button>
      </div>
    </Card>
  )
}
