import { useEffect, useState } from "react"
import { Power, RefreshCw } from "lucide-react"
import { api, isTauri } from "@/lib/api"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import { DEFAULT_PAGE_OPTIONS, resolveOpenPage, type DefaultPage } from "@/lib/mainNavigation"
import { ActionButton, SelectButton, SettingsGroup, SettingsRow, Switch } from "./settingsUi"

type Cfg = ReturnType<typeof useSettings>

/**
 * 常规（画板 86）：启动偏好、数据刷新。
 * 全局快捷键已按鼠鼠要求删除，Jolu.pen 画板 86 / 100 已同步删除对应分组与录入状态。
 */
export function GeneralSection({ cfg }: { cfg: Cfg }) {
  const toast = useToast()
  const disabled = !cfg.ready
  const [autostart, setAutostart] = useState<boolean | null>(null)
  const [autostartBusy, setAutostartBusy] = useState(isTauri)
  /** 读不到系统注册项是持续状态，就地说明；切换失败是一次性结果，走 Toast */
  const [autostartError, setAutostartError] = useState<string | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [refreshedAt, setRefreshedAt] = useState<Date | null>(null)

  useEffect(() => {
    if (!isTauri) return
    let stopped = false
    api.autostartStatus()
      .then((on) => { if (!stopped) { setAutostart(on); setAutostartError(null) } })
      .catch((reason) => { if (!stopped) setAutostartError(`开机启动状态读取失败：${String(reason)}`) })
      .finally(() => { if (!stopped) setAutostartBusy(false) })
    return () => { stopped = true }
  }, [])

  const toggleAutostart = (on: boolean) => {
    setAutostartBusy(true)
    api.setAutostart(on)
      .then((enabled) => {
        // 以后端回读的实际值为准，注册项没写进去时不报喜
        setAutostart(enabled)
        setAutostartError(null)
        toast.success(enabled ? "已开启开机启动" : "已关闭开机启动")
      })
      .catch((reason) => toast.danger("开机启动设置失败", String(reason)))
      .finally(() => setAutostartBusy(false))
  }

  const refreshNow = () => {
    setRefreshing(true)
    return api.islandAction("refresh")
      .then(() => setRefreshedAt(new Date()))
      .catch((reason) => toast.show({ tone: "danger", title: "刷新失败", detail: String(reason), action: { label: "重新刷新", onClick: refreshNow } }))
      .finally(() => setRefreshing(false))
  }

  return (
    <>
      <SettingsGroup icon={Power} title="启动偏好">
        <SettingsRow
          title="开机启动"
          desc={autostartError ?? undefined}
          descClassName={autostartError ? "text-jolu-danger" : undefined}
          control={<Switch liquid label="开机启动" on={autostart ?? false} disabled={autostartBusy || autostart === null} onChange={toggleAutostart} />}
        />
        <SettingsRow
          title="静默启动"
          desc={autostart === false ? "开启开机启动后生效" : undefined}
          control={<Switch liquid label="静默启动" on={cfg.settings.silent_startup} disabled={disabled || cfg.isSaving("silent_startup") || autostart !== true} onChange={(on) => void cfg.setSilentStartup(on)} />}
        />
        <SettingsRow
          title="默认打开页面"
          control={
            <SelectButton
              label="默认打开页面"
              value={cfg.settings.default_page === "last" ? "last" : resolveOpenPage(cfg.settings.default_page, null)}
              options={[...DEFAULT_PAGE_OPTIONS]}
              disabled={disabled || cfg.isSaving("default_page")}
              onChange={(v) => void cfg.setDefaultPage(v as DefaultPage)}
            />
          }
        />
      </SettingsGroup>

      <SettingsGroup icon={RefreshCw} title="数据刷新">
        <SettingsRow
          title="自动刷新间隔"
          desc="不影响本地 Token 采集"
          control={
            <SelectButton
              label="自动刷新间隔"
              value={String(cfg.settings.refresh_minutes)}
              disabled={disabled || cfg.isSaving("refresh_minutes")}
              options={[1, 5, 15, 30].map((n) => ({ value: String(n), label: `${n} 分钟` }))}
              onChange={(v) => void cfg.setDisplayPreferences({ refresh_minutes: Number(v) })}
            />
          }
        />
        <SettingsRow
          title="立即刷新"
          desc={refreshedAt
            ? `已开始刷新：${refreshedAt.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })} · 重新扫描本机会话并读取额度与余额`
            : undefined}
          control={<ActionButton icon={<RefreshCw aria-hidden className="size-3.5" />} busy={refreshing} disabled={!isTauri} onClick={refreshNow}>{refreshing ? "刷新中…" : "刷新数据"}</ActionButton>}
        />
      </SettingsGroup>
    </>
  )
}
