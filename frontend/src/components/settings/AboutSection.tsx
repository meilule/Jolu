import { useEffect, useState } from "react"
import { ChevronRight, CircleHelp, ExternalLink, RefreshCw } from "lucide-react"
import { openUrl } from "@/lib/api"
import { Button, Input, Select, StatusPill, Tag } from "@/ui"
import { settingsApi } from "@/lib/settingsApi"
import { UPDATES_ENABLED, progressPercent } from "@/lib/updateState"
import { REPO_URL } from "@/components/panel/HeaderActions"
import { AppIcon } from "@/components/brand/AppIcon"
import { useSettings } from "@/hooks/useSettings"
import { useUpdate } from "@/components/panel/UpdateContext"
import { useToast } from "@/components/ui/Toast"
import { ActionButton, SettingsGroup, SettingsRow, Switch } from "./settingsUi"
import { CapabilitySection } from "./CapabilitySection"

/** 关于与更新（画板 96）：检查更新默认启用；构建时停用（UPDATES_ENABLED = false）则如实显示停用状态 */
export function AboutSection() {
  const cfg = useSettings()
  const [version, setVersion] = useState<string | null>(null)
  const interval = cfg.settings.update_interval_minutes ?? 30
  const intervalHint = cfg.settings.auto_check_updates
    ? `每 ${formatInterval(interval)} 检查一次；可在下拉中选择常用值或自定义`
    : "关闭自动检查时不生效"
  useEffect(() => {
    let stopped = false
    settingsApi.appVersion().then((v) => { if (!stopped) setVersion(v) }).catch(() => {})
    return () => { stopped = true }
  }, [])
  const versionText = version ? `v${version}` : "—"

  return (
    <>
      <BrandHeroCard versionText={versionText} />

      <SettingsGroup icon={RefreshCw} title="应用更新">
        {UPDATES_ENABLED ? <UpdateRow versionText={versionText} /> : (
          <SettingsRow
            className="min-h-[72px]"
            title={`当前版本 ${versionText}`}
            desc="检查更新暂时停用，新版本请到 GitHub Releases 下载。"
            control={<ActionButton icon={<RefreshCw aria-hidden className="size-3.5" />} disabled>暂未开放</ActionButton>}
          />
        )}
        <SettingsRow
          className="min-h-[72px]"
          title="自动检查更新"
          desc={UPDATES_ENABLED ? "启动时检查一次，之后按下面的间隔复查；新版本自动在后台下载，安装由你确认。" : "当前发布通道尚未开放，偏好会保留；启用更新后生效。"}
          control={<Switch liquid label="自动检查更新" on={cfg.settings.auto_check_updates} disabled={!cfg.ready || cfg.isSaving("auto_check_updates")} onChange={(on) => void cfg.setAutoCheckUpdates(on)} />}
        />
        <SettingsRow
          className="min-h-[72px]"
          title="检查间隔"
          desc={intervalHint}
          control={
            <UpdateIntervalPicker
              value={cfg.settings.update_interval_minutes ?? 30}
              disabled={!cfg.ready || !cfg.settings.auto_check_updates || cfg.isSaving("update_interval")}
              onCommit={(minutes) => void cfg.setUpdateInterval(minutes)}
            />
          }
        />
        <SettingsRow
          className="min-h-[72px]"
          title="更新日志"
          desc="版本变更与修复记录"
          control={<ActionButton tone="soft" onClick={() => void openUrl(`${REPO_URL}/releases`)}>查看日志<ChevronRight aria-hidden className="size-3.5" /></ActionButton>}
        />
      </SettingsGroup>

      {cfg.error && <p role="alert" className="text-xs text-jn-danger">{cfg.error}</p>}

      <SettingsGroup icon={CircleHelp} title="帮助与项目">
        <SettingsRow
          className="min-h-[72px]"
          title="GitHub"
          desc="源代码与问题反馈"
          control={<ActionButton tone="soft" onClick={() => void openUrl(REPO_URL)}>访问项目<ChevronRight aria-hidden className="size-3.5" /></ActionButton>}
        />
        <SettingsRow
          className="min-h-[72px]"
          title="问题反馈"
          desc="提交 Issue"
          control={<ActionButton tone="soft" onClick={() => void openUrl(`${REPO_URL}/issues`)}>提交 Issues<ExternalLink aria-hidden className="size-3.5" /></ActionButton>}
        />
      </SettingsGroup>

      <CapabilitySection />

    </>
  )
}

/** R1-17 顶部品牌卡片：Logo、版本号、定位文案与更新主动作 */
function BrandHeroCard({ versionText }: { versionText: string }) {
  const toast = useToast()
  const { phase, info, progress, checkNow, openUpdate } = useUpdate()
  const checking = phase === "checking"
  const hasUpdate = UPDATES_ENABLED && !!info && phase !== "ready"
  const percent = progressPercent(progress.downloaded, progress.total)
  const available = info ? `v${info.availableVersion}` : ""

  const check = () => {
    void checkNow()
      .then((found) => toast.success(found ? "发现新版本，正在后台下载" : "已是最新版本"))
      .catch((reason) => toast.danger("检查更新失败", String(reason)))
  }

  return (
    <section className="flex flex-wrap items-center gap-4 rounded-jn-card border border-jn-line bg-jn-surface px-[22px] py-[18px] shadow-jn-surface">
      <span className="grid size-12 shrink-0 place-items-center rounded-[14px] bg-jn-well">
        <AppIcon size={34} />
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-[18px] font-bold text-jn-text">Jolu</span>
          <Tag>{versionText}</Tag>
          {hasUpdate && (
            <StatusPill tone={phase === "downloadError" ? "danger" : "success"}>
              {phase === "downloaded"
                ? `${available} 已下载`
                : phase === "downloading"
                  ? `下载中${percent === null ? "" : ` ${percent}%`}`
                  : phase === "downloadError"
                    ? `${available} 下载失败`
                    : `发现 ${available}`}
            </StatusPill>
          )}
        </div>
        <span className="text-xs text-jn-text-3">Windows 桌面 AI 工具管理器</span>
      </div>
      {UPDATES_ENABLED && (
        hasUpdate ? (
          <Button kind="primary" size="sm" icon="update" loading={phase === "installing"} onClick={openUpdate}>
            {phase === "downloaded" || phase === "installing" ? "重启并安装" : phase === "downloading" ? "查看下载" : phase === "downloadError" ? "重试下载" : `下载 ${available}`}
          </Button>
        ) : (
          <Button kind="soft" size="sm" icon="refresh" loading={checking} disabled={checking} onClick={check}>
            {checking ? "检查中…" : "检查更新"}
          </Button>
        )
      )}
    </section>
  )
}

/** 检查更新启用后的行：沿用 UpdateContext 的检查、下载与安装流程 */
function UpdateRow({ versionText }: { versionText: string }) {
  const toast = useToast()
  const { phase, info, progress, checkNow, openUpdate } = useUpdate()
  const checking = phase === "checking"
  const hasUpdate = !!info && phase !== "ready"
  const percent = progressPercent(progress.downloaded, progress.total)
  const check = () => {
    void checkNow()
      .then((available) => toast.success(available ? "发现新版本，正在后台下载" : "已是最新版本"))
      .catch((reason) => toast.danger("检查更新失败", String(reason)))
  }
  const version = info ? `v${info.availableVersion}` : ""
  const desc = !hasUpdate ? undefined
    : phase === "downloaded" ? `${version} 安装包已下载并校验，点击安装即进入安装界面，装完自动重启`
      : phase === "downloading" ? `正在后台下载 ${version}${percent === null ? "" : `（${percent}%）`}`
        : phase === "installing" ? `正在打开 ${version} 安装界面`
          : phase === "downloadError" ? `${version} 下载失败，可重试`
            : `发现新版本 ${version}`
  return (
    <SettingsRow
      className="min-h-[72px]"
      title={`当前版本 ${versionText}`}
      desc={desc}
      control={hasUpdate
        ? <ActionButton tone="accent" busy={phase === "installing"} onClick={openUpdate}>
          {phase === "downloaded" || phase === "installing" ? "安装" : phase === "downloading" ? "查看下载" : phase === "downloadError" ? "重试下载" : "下载"} {version}
        </ActionButton>
        : <ActionButton icon={<RefreshCw aria-hidden className="size-3.5" />} busy={checking} onClick={check}>{checking ? "检查中…" : "检查更新"}</ActionButton>}
    />
  )
}

/** 常用间隔；不在列表中的值走「自定义」输入。 */
const PRESET_INTERVALS = [5, 10, 15, 30, 60, 120, 240] as const

export function formatInterval(minutes: number): string {
  if (minutes < 60) return `${minutes} 分钟`
  if (minutes % 60 === 0) return `${minutes / 60} 小时`
  return `${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分钟`
}

/**
 * 检查间隔选择：预设下拉 + 自定义输入。
 * 自定义值只在离开输入框或按回车时提交，避免每敲一个数字就写一次设置。
 */
function UpdateIntervalPicker({
  value, disabled, onCommit,
}: {
  value: number
  disabled: boolean
  onCommit: (minutes: number) => void
}) {
  const [custom, setCustom] = useState(false)
  const [draft, setDraft] = useState(String(value))
  useEffect(() => { setDraft(String(value)) }, [value])
  const preset = PRESET_INTERVALS.includes(value as (typeof PRESET_INTERVALS)[number])

  const commit = () => {
    const minutes = Number(draft)
    if (!Number.isFinite(minutes) || minutes < 5 || minutes > 1440) {
      setDraft(String(value))
      setCustom(false)
      return
    }
    onCommit(minutes)
    setCustom(false)
  }

  if (custom) {
    return (
      <div className="flex items-center gap-2">
        <Input
          type="number"
          min={5}
          max={1440}
          value={draft}
          disabled={disabled}
          aria-label="自定义检查间隔（分钟）"
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => { if (event.key === "Enter") commit() }}
          className="h-8 w-24"
        />
        <span className="text-[12px] text-jn-text-3">分钟</span>
      </div>
    )
  }

  return (
    <div className="w-[150px]">
      <Select variant="filter" width="full" label="检查间隔" value={preset ? String(value) : "custom"} disabled={disabled}
        onChange={(next) => {
          if (next === "custom") {
            setCustom(true)
            return
          }
          onCommit(Number(next))
        }}
        options={[
          ...PRESET_INTERVALS.map((minutes) => ({ value: String(minutes), label: formatInterval(minutes) })),
          { value: "custom", label: "自定义…" },
        ]} />
    </div>
  )
}
