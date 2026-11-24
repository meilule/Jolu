/**
 * 会话线程弹窗 —— 设计稿 `docs/ui/jolu-new.pen` R1-46（在原工具继续）与 R1-47（创建自动化确认）。
 */
import { useEffect, useState } from "react"
import { api, isTauri, type ContinueCheckDto, type SessionItemDto } from "@/lib/api"
import { middleEllipsis, projectName, sessionTitle } from "@/lib/sessionThreads"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { providerName } from "@/lib/providerCatalog"
import type { PlatformId } from "@/types"
import { Icon } from "@/icons"
import { Banner, Button, Dialog } from "@/ui"

/** 在原工具继续（R1-46）：检查原工具与项目目录，确认后在新的控制台窗口里续聊。 */
export function ContinueDialog({ session, accountKnown, onClose, onDone }: {
  session: SessionItemDto
  accountKnown: boolean
  onClose: () => void
  onDone: () => void
}) {
  const name = providerName(session.platform)
  const [check, setCheck] = useState<ContinueCheckDto | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [opening, setOpening] = useState(false)

  useEffect(() => {
    if (!isTauri) {
      setCheck({ supported: true, tool_found: true, cwd: session.cwd, cwd_accessible: true })
      return
    }
    api.sessionContinueCheck(session.platform, session.session_id).then(setCheck, (reason: unknown) => setError(String(reason)))
  }, [session.platform, session.session_id, session.cwd])

  const ready = !!check && check.supported && check.tool_found && check.cwd_accessible
  const open = async () => {
    if (!isTauri) {
      onDone()
      return
    }
    setOpening(true)
    setError(null)
    try {
      await api.sessionContinue(session.platform, session.session_id)
      onDone()
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setOpening(false)
    }
  }

  const checkRow = (ok: boolean | null, label: string) => (
    <span className="flex items-center gap-2 text-xs text-jn-text-2">
      {ok === null ? (
        <Icon name="loading" size="sm" className="animate-spin text-jn-text-3" />
      ) : ok ? (
        <Icon name="success" size="sm" className="text-jn-primary" />
      ) : (
        <Icon name="danger" size="sm" className="text-jn-danger" />
      )}
      {label}
    </span>
  )

  return (
    <Dialog
      open
      onOpenChange={(next) => { if (!next && !opening) onClose() }}
      preventClose={opening}
      width="md"
      title={
        <span className="flex items-center gap-2.5">
          <PlatformLogo platform={session.platform as PlatformId} size={22} />
          <span>在 {name} 中继续</span>
        </span>
      }
      description="检查原工具与项目目录，确认后在新的控制台窗口里续聊。"
      actions={
        <>
          <Button kind="ghost" size="md" disabled={opening} onClick={onClose}>取消</Button>
          <Button kind="primary" size="md" icon="terminal" loading={opening} disabled={!ready || opening} onClick={() => void open()}>
            {opening ? `正在打开 ${name}…` : `打开 ${name}`}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-2 rounded-[14px] bg-jn-well p-3.5">
        <span className="text-[13px] font-semibold text-jn-text">{sessionTitle(session)}</span>
        {([
          ["会话", middleEllipsis(session.session_id)],
          ["目录", check?.cwd ?? session.cwd ?? "未识别"],
          ["来源", `本机 ${name} 记录`],
        ] as const).map(([label, value]) => (
          <span key={label} className="flex gap-3 text-xs">
            <span className="w-8 shrink-0 text-jn-text-3">{label}</span>
            <span className="min-w-0 truncate font-mono text-jn-text" title={value}>{value}</span>
          </span>
        ))}
      </div>

      <div className="flex flex-col gap-2">
        {checkRow(check ? check.tool_found : null, check && !check.tool_found ? `未检测到 ${name}` : `已检测到 ${name}`)}
        {checkRow(check ? check.cwd_accessible : null, check && !check.cwd_accessible ? "项目目录不可访问" : "项目目录可访问")}
      </div>

      <Banner tone="info">
        沿用原工具当前的登录状态。{accountKnown ? "Jolu 不会据此自动切换原工具账号。" : "这条历史会话未归属账号，Jolu 不会据此自动切换原工具账号。"}
      </Banner>

      {(error || (check && !ready)) && (
        <p role="alert" className="text-xs text-jn-danger">
          {error ?? (check && !check.tool_found ? `请安装或配置 ${name} 后重试。当前仍可查看记录与复制会话 ID。` : "项目目录不可访问，无法在原目录打开。")}
        </p>
      )}
    </Dialog>
  )
}

/** 从会话创建自动化（R1-47）：确认后前往「自动化 → 新建任务」，不会立即执行 */
export function AutomationConfirmDialog({ session, onClose, onConfirm }: {
  session: SessionItemDto
  onClose: () => void
  onConfirm: () => void
}) {
  return (
    <Dialog
      open
      onOpenChange={(next) => { if (!next) onClose() }}
      width="md"
      title="为此会话创建自动化任务？"
      description="将前往「自动化 → 新建任务」，并带入当前会话和项目。"
      actions={
        <>
          <Button kind="ghost" size="md" onClick={onClose}>取消</Button>
          <Button kind="primary" size="md" icon="arrowRight" onClick={onConfirm}>继续新建</Button>
        </>
      }
    >
      <div className="flex items-center gap-3 rounded-[14px] bg-jn-well p-3.5">
        <span className="grid size-9 shrink-0 place-items-center rounded-[10px] bg-jn-surface">
          <PlatformLogo platform={session.platform as PlatformId} size={20} />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-[13px] font-semibold text-jn-text">{sessionTitle(session)}</span>
          <span className="truncate text-xs text-jn-text-3">{providerName(session.platform)} · {projectName(session.cwd)}</span>
          <span className="truncate font-mono text-[11px] text-jn-text-3" title={session.session_id}>{middleEllipsis(session.session_id)}</span>
        </div>
      </div>

      <p className="text-xs leading-relaxed text-jn-text-3">
        确认后仅打开任务配置，任务创建后才会执行。请在新建页面选择执行账号、填写任务内容与启动方式；已有会话内容不会被清空。
      </p>
    </Dialog>
  )
}
