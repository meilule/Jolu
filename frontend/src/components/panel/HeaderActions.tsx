import { useRef } from "react"
import { isTauri, openUrl } from "@/lib/api"
import { resolveDarkTheme } from "@/lib/displayPreferences"
import { UPDATES_ENABLED, progressPercent } from "@/lib/updateState"
import { useSettings } from "@/hooks/useSettings"
import { useSystemDark } from "@/hooks/useSystemDark"
import { useToast } from "@/components/ui/Toast"
import { Tip } from "@/components/ui/Tip"
import { useUpdate } from "@/components/panel/UpdateContext"
import { IconButton, Button } from "@/ui"

export const REPO_URL = "https://github.com/meilule/Jolu"

/**
 * 页头右端的全局操作组（GitHub / 灵动岛显隐 / 主题 / 检查更新 / 版本号）。
 *
 * 设计稿 R1-C 把这一组放在页头，图标按钮 36×36（`IconButton`）；
 * 发现新版本后换成浅绿底的版本按钮（鼠鼠 2026-10-27 确认，画板无此状态）。
 * 行为与旧 `components/panel/PageHeader` 完全一致，只换外观与令牌。
 */
export function HeaderActions() {
  const cfg = useSettings()
  const systemDark = useSystemDark()
  const toast = useToast()
  const update = useUpdate()
  const dark = resolveDarkTheme(cfg.settings.theme, systemDark)
  const visible = cfg.settings.island_visible
  const desktopOnly = isTauri ? undefined : "（仅桌面端可用）"

  // 连点保护：检查在途时忽略后续点击，避免同一结果反复播报（Toast 会合并成 ×5）。
  const checkingRef = useRef(false)
  const onUpdate = () => {
    if (checkingRef.current) return
    checkingRef.current = true
    void update.checkNow()
      .then((available) => toast.success(available ? "发现新版本，正在后台下载" : "已是最新版本"))
      .catch((reason) => toast.danger("检查更新失败", String(reason)))
      .finally(() => { checkingRef.current = false })
  }

  return (
    <div className="flex shrink-0 items-center gap-1.5">
      <Tip label="GitHub" side="bottom">
        <IconButton icon="github" label="GitHub" kind="ghost" onClick={() => void openUrl(REPO_URL)} />
      </Tip>
      <Tip label={(visible ? "隐藏灵动岛" : "显示灵动岛") + (desktopOnly ?? "")} side="bottom">
        <span className="inline-flex">
          <IconButton icon="navIsland" label={visible ? "隐藏灵动岛" : "显示灵动岛"} kind={visible ? "surface" : "ghost"}
            disabled={!isTauri || !cfg.ready} onClick={() => void cfg.setIslandVisible(!visible)} />
        </span>
      </Tip>
      <Tip label={(dark ? "切换到浅色" : "切换到深色") + (desktopOnly ?? "")} side="bottom">
        <span className="inline-flex">
          <IconButton icon={dark ? "light" : "dark"} label={dark ? "切换到浅色" : "切换到深色"}
            kind="ghost" disabled={!isTauri || !cfg.ready} onClick={() => void cfg.setTheme(dark ? "light" : "dark")} />
        </span>
      </Tip>
      {UPDATES_ENABLED && update.info && update.phase !== "ready" ? <UpdateVersionButton /> : (
        <Tip label={!UPDATES_ENABLED ? "检查更新暂时停用" : update.phase === "checking" ? "正在检查更新" : "检查更新"} side="bottom">
          <span className="inline-flex">
            <IconButton icon={update.phase === "checking" ? "loading" : "update"} label="检查更新" kind="ghost"
              className={update.phase === "checking" ? "animate-spin" : undefined}
              aria-busy={update.phase === "checking" || undefined}
              disabled={!UPDATES_ENABLED} onClick={onUpdate} />
          </span>
        </Tip>
      )}
    </div>
  )
}

/** 有新版本时的版本按钮：下载完成后点击直接进入安装界面 */
function UpdateVersionButton() {
  const { phase, info, progress, openUpdate } = useUpdate()
  if (!info) return null
  const version = `v${info.availableVersion}`
  const percent = progressPercent(progress.downloaded, progress.total)
  const label = phase === "downloaded" ? `安装 ${version}`
    : phase === "installing" ? `正在打开 ${version} 安装界面`
      : phase === "downloading" ? `正在下载 ${version}${percent === null ? "" : `（${percent}%）`}`
        : phase === "downloadError" ? `${version} 下载失败，点击重试`
          : `下载 ${version}`
  const busy = phase === "downloading" || phase === "installing"
  return (
    <Tip label={label} side="bottom">
      <span className="inline-flex">
        <Button kind="soft" size="sm" icon="update" loading={busy}
          disabled={phase === "installing"} onClick={openUpdate} aria-label={label} aria-busy={busy || undefined}>
          <span className="tnum">{version}</span>
        </Button>
      </span>
    </Tip>
  )
}
