import { cn } from "@/lib/utils"
import { RUN_RANGES, RUN_STATUSES, clock, hhmmss, runCode, runStatusLabel, runTokens, runningMs, shortTool, sourceLabel, waitingMs } from "@/lib/automation"
import type { RunFilterDto, RunPageDto, RunRange, RunStatus } from "@/lib/automationApi"
import { formatTokens, localZone } from "@/lib/usageStats"
import { Card, MenuFilter } from "@/components/usage/usageControls"
import type { AccountRef } from "./TaskList"
import { MetricGrid } from "../ui/MetricGrid"
import { Icon } from "@/icons"
import { Banner, Button, DataValue, IconButton, MetricCard, Pagination, StatusPill, type StatusTone } from "@/ui"

const ALL = "__all__"

function statusTone(tone: "on" | "danger" | "muted" | "text"): StatusTone {
  if (tone === "on") return "success"
  if (tone === "danger") return "danger"
  return "neutral"
}

/** 运行记录页签（R1-25）：4 枚 MetricCard 概览、筛选栏、圆角斑马纹记录表、Pagination 与提示横幅。 */
export function RunList({ data, filter, page, now, loading, accounts, onFilter, onPage, onRefresh, onOpen }: {
  data: RunPageDto
  filter: RunFilterDto
  page: number
  now: number
  loading: boolean
  accounts: Map<string, AccountRef>
  onFilter: (next: RunFilterDto) => void
  onPage: (page: number) => void
  onRefresh: () => void
  onOpen: (runId: number) => void
}) {
  const today = filter.range === "today"
  const rangeLabel = RUN_RANGES.find((r) => r.value === filter.range)?.label ?? "今天"
  const accountIds = [...new Set(data.rows.map((r) => r.connection_id).concat(filter.connection ? [filter.connection] : []))]

  return (
    <>
      <MetricGrid>
        <MetricCard
          title={<span className="inline-flex items-center gap-1.5"><Icon name="success" size="sm" className="text-jn-primary" />{today ? "今日完成" : "完成"}</span>}
          value={<DataValue state={data.completed === 0 ? "zero" : "value"} value={data.completed} format={(v) => String(v)} className="[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none" />}
          compare={`其中 ${data.completed_waited} 次经历额度等待`}
        />
        <MetricCard
          title={<span className="inline-flex items-center gap-1.5"><Icon name="play" size="sm" className="text-jn-info" />执行中</span>}
          value={<DataValue state={data.running === 0 ? "zero" : "value"} value={data.running} format={(v) => String(v)} className="[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none" />}
          compare={data.running ? "可查看步骤并停止" : "当前没有执行中的运行"}
        />
        <MetricCard
          title={<span className="inline-flex items-center gap-1.5"><Icon name="fail" size="sm" className={data.failed > 0 ? "text-jn-danger" : "text-jn-text-3"} />失败</span>}
          value={<DataValue state={data.failed === 0 ? "zero" : "value"} value={data.failed} format={(v) => String(v)} className={cn("[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none", data.failed > 0 && "[&>span:first-child]:text-jn-danger")} />}
          compare={data.failed_note ?? "没有失败的运行"}
        />
        <MetricCard
          title={<span className="inline-flex items-center gap-1.5"><Icon name="coins" size="sm" className="text-jn-text-3" />{today ? "今日消耗 Token" : "消耗 Token"}</span>}
          value={
            data.tokens === null ? (
              <DataValue state="unknown" note="未提供" />
            ) : (
              <DataValue state={data.tokens === 0 ? "zero" : "value"} value={data.tokens} format={(v) => formatTokens(v).replace(" ", "")} className="[&>span:first-child]:text-[28px] [&>span:first-child]:font-bold [&>span:first-child]:leading-none" />
            )
          }
          compare="不含等待期间；以执行器返回为准"
        />
      </MetricGrid>

      <div className="flex flex-wrap items-center gap-2">
        <MenuFilter trailingChevron label="时间范围" value={filter.range} onChange={(v) => onFilter({ ...filter, range: v as RunRange })} options={RUN_RANGES.map((r) => ({ value: r.value, label: r.label }))} />
        <MenuFilter trailingChevron label="状态筛选" value={filter.status ?? ALL} onChange={(v) => onFilter({ ...filter, status: v === ALL ? null : (v as RunStatus) })} options={[{ value: ALL, label: "全部状态" }, ...RUN_STATUSES]} />
        <MenuFilter
          trailingChevron
          label="账号筛选"
          value={filter.connection ?? ALL}
          onChange={(v) => onFilter({ ...filter, connection: v === ALL ? null : v })}
          options={[{ value: ALL, label: "全部账号" }, ...accountIds.map((id) => ({ value: id, label: accounts.get(id)?.name ?? "未知账号" }))]}
        />
        <MenuFilter
          trailingChevron
          label="任务筛选"
          value={filter.task === null ? ALL : String(filter.task)}
          onChange={(v) => onFilter({ ...filter, task: v === ALL ? null : Number(v) })}
          options={[{ value: ALL, label: "全部任务" }, ...data.tasks.map((t) => ({ value: String(t.task_id), label: t.name }))]}
        />
        <span className="flex-1" />
        <IconButton icon="refresh" label="刷新运行记录" size="sm" kind="ghost" disabled={loading} onClick={onRefresh} className={cn(loading && "[&_svg]:animate-spin")} />
      </div>

      <Card className="flex flex-col gap-1 p-3">
        <div className="overflow-x-auto">
          <div className="flex min-w-[820px] flex-col gap-1">
            <div className="flex h-8 items-center gap-3 px-3 text-[11.5px] font-medium text-jn-text-3">
              <span className="w-[220px] shrink-0">任务 / 运行编号</span>
              <span className="w-[176px] shrink-0">账号 / 启动方式</span>
              <span className="min-w-[80px] flex-1">开始时间</span>
              <span className="w-[132px] shrink-0">执行 / 等待</span>
              <span className="w-[84px] shrink-0">Token</span>
              <span className="w-[168px] shrink-0 text-right">状态 / 操作</span>
            </div>
            {data.rows.length ? data.rows.map((run, index) => {
              const status = runStatusLabel(run.status)
              const account = accounts.get(run.connection_id)?.name ?? run.account ?? "未知账号"
              return (
                <div
                  key={run.id}
                  className={cn(
                    "jn-interactive flex h-[64px] items-center gap-3 rounded-[12px] px-3 hover:bg-jn-primary-wash/50",
                    index % 2 === 0 && "bg-jn-well",
                  )}
                >
                  <div className="flex w-[220px] shrink-0 flex-col gap-0.5">
                    <span className="truncate text-xs font-semibold text-jn-text">{run.task_name}</span>
                    <span className="font-mono text-[11px] text-jn-text-3">{runCode(run.id)}</span>
                  </div>
                  <div className="flex w-[176px] shrink-0 flex-col gap-0.5">
                    <span className="truncate text-xs text-jn-text">{shortTool(run.platform)} · {account}</span>
                    <span className="text-[11px] text-jn-text-3">{sourceLabel(run.source)}</span>
                  </div>
                  <span className="tnum min-w-[80px] flex-1 font-mono text-xs text-jn-text">{hhmmss(run.started_at ?? run.created_at)}</span>
                  <span className="tnum w-[132px] shrink-0 font-mono text-xs text-jn-text">{clock(runningMs(run, now))} / {clock(waitingMs(run, now))}</span>
                  <span className="tnum w-[84px] shrink-0 font-mono text-xs text-jn-text">{formatTokens(runTokens(run)).replace(" ", "")}</span>
                  <div className="flex w-[168px] shrink-0 items-center justify-end gap-2">
                    <StatusPill tone={statusTone(status.tone)}>{status.label}</StatusPill>
                    <Button kind="soft" size="sm" icon="next" onClick={() => onOpen(run.id)}>详情</Button>
                  </div>
                </div>
              )
            }) : (
              <div className="rounded-[12px] bg-jn-well px-4 py-8 text-center text-xs text-jn-text-3">{rangeLabel}没有符合条件的运行记录</div>
            )}
          </div>
        </div>
        <div className="pt-2">
          <Pagination
            page={page}
            pageCount={Math.max(1, data.pages)}
            onChange={(next) => onPage(next)}
            summary={`${rangeLabel}共 ${data.total} 次运行 · 当前显示 ${data.rows.length} 条`}
          />
        </div>
      </Card>

      <Banner tone="info">
        <span className="font-semibold">执行耗时与等待时长分开记录</span>：额度不足期间不计入执行耗时；同一次运行可多次等待和恢复，重试会生成新的运行记录。
      </Banner>
      <p className="text-[11px] text-jn-text-3">时间按 {localZone()} 展示；Token 缺失显示 —，不以 0 代替。</p>
    </>
  )
}
