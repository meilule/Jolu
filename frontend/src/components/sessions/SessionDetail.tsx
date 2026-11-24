import * as Menu from "@radix-ui/react-dropdown-menu"
import { useEffect, useRef, useState, type ReactNode } from "react"
import { useContentMotion } from "@/lib/motion"
import { LoadingSurface } from "@/components/ui/LoadingSurface"
import { cn } from "@/lib/utils"
import { api, isTauri, type MessageDto, type MessagePageDto, type SessionItemDto } from "@/lib/api"
import { canCompress, canContinue, consumption, middleEllipsis, projectName, sessionTitle, spanLabel, statusLabel } from "@/lib/sessionThreads"
import { formatTokens, formatUsd, modelDisplayName } from "@/lib/usageStats"
import { accountLabel } from "@/lib/usageFilters"
import { demoMessages } from "@/mock/sessionThreads"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { MessageMarkdown } from "./MessageMarkdown"
import { Tip } from "@/components/ui/Tip"
import { useToast } from "@/components/ui/Toast"
import { providerName } from "@/lib/providerCatalog"
import type { Connection, PlatformId } from "@/types"
import { Icon, type IconName } from "@/icons"
import { Collapse } from "@/motion"
import { Button, IconButton, StatusPill, Tabs } from "@/ui"

type DetailTab = "messages" | "info"

/** 会话详情（R1-05）：消息记录与会话信息两个页签，正文只读，底部明确标注继续能力。 */
export function SessionDetail({ session, connections, onDrillUsage, onContinue, onCreateAutomation, onCompress }: {
  session: SessionItemDto
  connections: readonly Connection[]
  onDrillUsage: () => void
  onContinue: () => void
  onCreateAutomation: () => void
  onCompress: () => void
}) {
  const toast = useToast()
  const [tab, setTab] = useState<DetailTab>("messages")
  const [page, setPage] = useState<MessagePageDto | null>(null)
  const [messages, setMessages] = useState<MessageDto[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const bodyMotion = useContentMotion(`${session.session_id}-${tab}-${!!page}`)
  const revision = useRef(0)
  const platform = session.platform as PlatformId
  const name = providerName(session.platform)
  const account = session.connection_id ? connections.find((c) => c.id === session.connection_id) : null

  const load = () => {
    const current = ++revision.current
    setError(null)
    setLoadingMore(false)
    if (!isTauri) {
      const demo = demoMessages(session.platform, session.session_id)
      setPage(demo)
      setMessages(demo.messages)
      return
    }
    setPage(null)
    api.sessionMessages(session.platform, session.session_id, null)
      .then((result) => {
        if (current !== revision.current) return
        setPage(result)
        setMessages(result.messages)
      })
      .catch((reason: unknown) => current === revision.current && setError(reason instanceof Error ? reason.message : String(reason)))
  }

  useEffect(() => {
    load()
    return () => { revision.current++ }
  }, [session.platform, session.session_id])

  const loadEarlier = () => {
    if (!page || !page.offset || !isTauri || loadingMore) return
    setLoadingMore(true)
    const current = revision.current
    api.sessionMessages(session.platform, session.session_id, page.offset)
      .then((result) => {
        if (current !== revision.current) return
        setPage({ ...result, total: page.total })
        setMessages((value) => [...result.messages, ...value])
      })
      .catch((reason: unknown) => {
        if (current === revision.current) toast.danger("读取更早消息失败", reason instanceof Error ? reason.message : String(reason))
      })
      .finally(() => { if (current === revision.current) setLoadingMore(false) })
  }

  const copy = (text: string, what: string) => {
    void navigator.clipboard.writeText(text).then(() => toast.success("已复制", what), () => toast.danger("复制失败", "请手动选择文字复制"))
  }

  const resumable = canContinue(session.platform) && session.has_source
  const compressButton = (
    <Button kind="soft" size="sm" icon="compress" disabled={!canCompress(session.platform)} onClick={onCompress}>
      压缩会话
    </Button>
  )
  const continueButton = (
    <Button kind="primary" size="sm" icon="external" disabled={!resumable} onClick={onContinue}>
      在 {name} 中继续
    </Button>
  )

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 rounded-jn-card border border-jn-line bg-jn-surface p-5 shadow-jn-surface">
      <div className="flex items-start gap-2.5">
        <div className="flex min-w-0 flex-1 flex-col gap-1.5">
          <h3 className="truncate text-lg font-bold text-jn-text" title={sessionTitle(session)}>{sessionTitle(session)}</h3>
          <span className="truncate whitespace-pre text-xs text-jn-text-3">{name}  /  {projectName(session.cwd)}  /  {spanLabel(session.first_ts, session.last_ts)}</span>
        </div>
        <Menu.Root modal={false}>
          <Menu.Trigger asChild>
            <IconButton icon="more" label="更多会话操作" size="sm" kind="ghost" />
          </Menu.Trigger>
          <Menu.Portal>
            <Menu.Content align="end" sideOffset={6} className="jn-popover z-[140] w-[200px] rounded-[16px] border border-jn-line bg-jn-surface p-1.5 shadow-jn-surface">
              <MenuItem icon="copy" onSelect={() => copy(session.session_id, "会话 ID")}>复制会话 ID</MenuItem>
              <MenuItem icon="folder" disabled={!session.cwd} onSelect={() => session.cwd && copy(session.cwd, "项目路径")}>复制项目路径</MenuItem>
              <MenuItem icon="navUsage" onSelect={onDrillUsage}>查看用量明细</MenuItem>
              <MenuItem icon="external" disabled={!resumable} onSelect={onContinue}>在原工具中继续</MenuItem>
            </Menu.Content>
          </Menu.Portal>
        </Menu.Root>
      </div>

      <div className="flex min-h-7 flex-wrap items-center gap-x-2 gap-y-1.5">
        <StatusPill tone={session.status === "done" || session.status === "running" ? "success" : session.status === "failed" ? "danger" : "neutral"}>
          {statusLabel(session.status)}
        </StatusPill>
        <Chip>{account ? accountLabel(account) : "未归属账号"}</Chip>
        <Chip>{page ? (page.available ? `${page.total} 条消息` : "无消息正文") : "— 条消息"}</Chip>
        <Chip>{formatTokens(consumption(session))} Token</Chip>
        <span className="min-w-0 flex-1" />
        <Tabs
          style="segment"
          value={tab}
          onChange={setTab}
          items={[
            { key: "messages", label: "消息记录" },
            { key: "info", label: "会话信息" },
          ]}
        />
      </div>

      <div ref={bodyMotion} className="-mr-2 min-h-0 flex-1 overflow-y-auto border-t border-jn-line pt-3 pr-2">
        {tab === "messages" ? (
          error ? (
            <StateBox title="会话源暂时不可读" note="读取失败不代表会话结束；旧记录保留。" action={<Button kind="soft" size="sm" icon="retry" onClick={load}>重新读取</Button>} detail={error} />
          ) : !page ? (
            <LoadingSurface layout="messages" label="正在读取消息" />
          ) : !page.available ? (
            <StateBox
              title="这个来源未保留消息正文"
              note="会话标题和 Token 仍可查看。不会将统计记录拼成对话内容。"
              action={<Button kind="soft" size="sm" icon="next" onClick={() => setTab("info")}>查看会话信息</Button>}
            />
          ) : (
            <div className="flex flex-col gap-3">
              {page.note && <p className="rounded-[12px] bg-jn-well px-3 py-2 text-[11.5px] leading-relaxed text-jn-text-3">{page.note}</p>}
              {page.offset > 0 && (
                <button type="button" onClick={loadEarlier} disabled={loadingMore} className="jn-interactive flex h-7 items-center justify-center gap-1.5 self-center rounded-full bg-jn-well px-3 text-[11.5px] font-medium text-jn-text-2 outline-none hover:bg-jn-track hover:text-jn-text">
                  <Icon name="chevronsUp" size="xs" />
                  {loadingMore ? "正在读取…" : `加载更早的 ${page.offset} 条消息`}
                </button>
              )}
              {messages.map((m, index) => <MessageView key={index} message={m} platform={platform} name={name} />)}
              {!messages.length && <span className="text-xs text-jn-text-3">源文件里没有可显示的消息</span>}
            </div>
          )
        ) : (
          <InfoPanel session={session} page={page} accountText={account ? accountLabel(account) : "未归属 · 本机记录未提供账号身份"} onCopy={copy} onDrillUsage={onDrillUsage} />
        )}
      </div>

      <div className="flex min-h-9 shrink-0 flex-wrap items-center gap-x-2.5 gap-y-2 border-t border-jn-line pt-3">
        <StatusPill tone={resumable ? "success" : "neutral"}>
          {resumable ? "可在原工具继续" : !session.has_source ? "仅用量记录" : "仅本机只读"}
        </StatusPill>
        <span className="inline-flex items-center gap-1.5 whitespace-nowrap text-xs text-jn-text-3">
          <Icon name="eye" size="sm" />
          本机会话 · 只读
        </span>
        <span className="flex-1" />
        <div className="flex min-w-0 max-w-full flex-wrap items-center justify-end gap-2">
          <Button kind="soft" size="sm" icon="bot" onClick={onCreateAutomation}>
            创建自动化
          </Button>
          {canCompress(session.platform)
            ? compressButton
            : <Tip label={`${name} 暂不支持压缩（需要 Claude Code / Codex 会话）`}><span className="flex shrink-0">{compressButton}</span></Tip>}
          {resumable
            ? continueButton
            : <Tip label={!canContinue(session.platform) ? `${name} 暂无可靠的继续入口` : "没有可读的源文件，无法定位会话"}><span className="flex shrink-0">{continueButton}</span></Tip>}
        </div>
      </div>
    </section>
  )
}

function MenuItem({ icon, children, onSelect, disabled }: { icon: IconName; children: ReactNode; onSelect: () => void; disabled?: boolean }) {
  return (
    <Menu.Item disabled={disabled} onSelect={onSelect} className="flex h-8 cursor-pointer select-none items-center gap-2 rounded-[10px] px-2.5 text-xs text-jn-text outline-none data-[highlighted]:bg-jn-primary-wash data-[highlighted]:text-jn-primary-ink data-[disabled]:opacity-40">
      <Icon name={icon} size="sm" className="shrink-0 opacity-75" />
      {children}
    </Menu.Item>
  )
}

function Chip({ children }: { children: ReactNode }) {
  return <span className="inline-flex h-6 max-w-full items-center truncate whitespace-nowrap rounded-full bg-jn-well px-2.5 text-[11px] font-medium text-jn-text-2">{children}</span>
}

function StateBox({ title, note, action, detail }: { title: string; note: string; action: ReactNode; detail?: string }) {
  return (
    <div className="flex flex-col items-start gap-2.5 rounded-[16px] bg-jn-well p-4">
      <span className="text-sm font-semibold text-jn-text">{title}</span>
      <span className="text-xs text-jn-text-3">{note}</span>
      {detail && <span className="text-xs text-jn-danger">{detail}</span>}
      {action}
    </div>
  )
}

function clock(ts: number | null): string {
  if (ts === null) return ""
  const d = new Date(ts)
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`
}

function MessageView({ message, platform, name }: { message: MessageDto; platform: PlatformId; name: string }) {
  if (message.role === "user") {
    return (
      <div className="flex flex-col gap-2 rounded-[14px] bg-jn-well p-3.5">
        <div className="flex items-center justify-between">
          <span className="text-xs font-semibold text-jn-text">你</span>
          <span className="tnum text-[11px] text-jn-text-3">{clock(message.ts)}</span>
        </div>
        <MessageMarkdown text={message.text} />
      </div>
    )
  }
  return (
    <div className="flex flex-col gap-2.5 py-1">
      <div className="flex items-center gap-2">
        <PlatformLogo platform={platform} size={17} />
        <span className="text-xs font-semibold text-jn-text">{name}</span>
        {message.model && <span className="min-w-0 truncate font-mono text-[11px] text-jn-text-3" title={message.model}>{message.model}</span>}
        <span className="tnum text-[11px] text-jn-text-3">{clock(message.ts)}</span>
      </div>
      {message.reasoning && (
        <details className="rounded-[12px] bg-jn-well px-3 py-2">
          <summary className="cursor-pointer text-[11.5px] text-jn-text-3">推理过程 · {message.reasoning.length} 字</summary>
          <pre className="mt-1.5 max-h-48 overflow-auto whitespace-pre-wrap break-words font-mono text-[12px] text-jn-text-3">{message.reasoning}</pre>
        </details>
      )}
      {message.text && <MessageMarkdown text={message.text} />}
      {message.tools.map((tool, index) => <ToolRow key={index} tool={tool} />)}
    </div>
  )
}

function ToolRow({ tool }: { tool: MessageDto["tools"][number] }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="flex flex-col gap-1.5">
      <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className="jn-interactive flex h-[34px] items-center gap-2 rounded-[10px] bg-jn-well px-3 text-left outline-none hover:bg-jn-track/60">
        <Icon name={open ? "expand" : "next"} size="xs" className="shrink-0 text-jn-text-3" />
        <Icon name="file" size="sm" className="shrink-0 text-jn-text-3" />
        <span className="min-w-0 flex-1 truncate text-xs text-jn-text">{tool.summary ? `${tool.name} · ${tool.summary}` : tool.name}</span>
        <span className={cn("text-[11px]", tool.output ? "text-jn-text-3" : "text-jn-text-3/70")}>{tool.output ? "已完成" : "无输出记录"}</span>
      </button>
      <Collapse open={open}>
        <div className="rounded-[10px] bg-jn-well p-3">
          <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words font-mono text-[12px] text-jn-text-3">{tool.output ?? "来源没有记录这次调用的输出"}</pre>
        </div>
      </Collapse>
    </div>
  )
}

function InfoPanel({ session, page, accountText, onCopy, onDrillUsage }: {
  session: SessionItemDto
  page: MessagePageDto | null
  accountText: string
  onCopy: (text: string, what: string) => void
  onDrillUsage: () => void
}) {
  const row = (label: string, value: ReactNode, copyText?: string, what?: string) => (
    <div className="flex min-h-[30px] items-center gap-3">
      <span className="w-[60px] shrink-0 text-xs text-jn-text-3">{label}</span>
      <span className="min-w-0 flex-1 truncate text-xs text-jn-text">{value}</span>
      {copyText && <IconButton icon="copy" label={`复制${what}`} size="sm" kind="ghost" onClick={() => onCopy(copyText, what!)} />}
    </div>
  )
  const models = page?.models ?? []
  const split: [string, number | null][] = [["普通输入", session.fresh_input], ["输出", session.output], ["缓存读取", session.cache_read], ["缓存写入", session.cache_write]]
  return (
    <div className="flex flex-col gap-3.5">
      <div className="flex flex-col gap-2 rounded-[16px] bg-jn-well p-3.5">
        {row("会话 ID", <span className="font-mono" title={session.session_id}>{middleEllipsis(session.session_id)}</span>, session.session_id, "会话 ID")}
        {row("项目目录", session.cwd ?? "未识别", session.cwd ?? undefined, "项目目录")}
        {row("数据来源", session.has_source || page?.available ? `本机 ${providerName(session.platform)} 记录` : `本机 ${providerName(session.platform)} 用量记录（无正文）`)}
        {row("模型", models.length ? (
          <span className="flex items-center gap-1.5"><PlatformLogo platform={session.platform as PlatformId} size={16} />{models.map(modelDisplayName).join("、")}</span>
        ) : "—")}
        {row("账号归属", accountText)}
      </div>
      <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
        {split.map(([label, value]) => (
          <div key={label} className="flex min-w-0 flex-col gap-1 rounded-[14px] bg-jn-well p-3">
            <span className="text-[11px] text-jn-text-3">{label}</span>
            <span className={cn("tnum font-mono text-[17px] font-bold", value === null ? "text-jn-text-3" : "text-jn-text")}>{formatTokens(value)}</span>
          </div>
        ))}
      </div>
      <div className="flex items-center gap-2.5">
        <span className="text-xs text-jn-text-3">
          估算费用 {page && page.priced_requests ? formatUsd(page.cost) : "—"} · {session.requests} 次调用
          {page && page.priced_requests < page.requests ? `（${page.requests - page.priced_requests} 次未计价）` : ""}
        </span>
        <span className="flex-1" />
        <Button kind="soft" size="sm" icon="next" onClick={onDrillUsage}>查看用量明细</Button>
      </div>
      <p className="text-[11px] text-jn-text-3">缺少模型价格或计量字段时显示 —；账号归属不使用当前登录账号推断。</p>
    </div>
  )
}
