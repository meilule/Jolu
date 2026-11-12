import { useEffect, useState } from "react"
import { Coffee, Moon, Send, Terminal } from "lucide-react"
import { isTauri } from "@/lib/api"
import { settingsApi, type AgentVersionRow } from "@/lib/settingsApi"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import {
  OTLP_SCOPE_NOTICE,
  hasRunnableUpdate,
  normalizeWarmupHour,
  otlpEndpointHint,
  versionLabel,
  warmupStateText,
} from "@/lib/opsSettings"
import { ActionButton, SettingsGroup, SettingsRow, Switch } from "./settingsUi"

/**
 * 运维项（批次 G §3.12）：保持唤醒、每日 warm-up、OTLP 用量导出、Agent 版本。
 *
 * 待确认：`Jolu.pen` 模块 G（画板 86–103）没有覆盖这四项，按既有 `SettingsGroup` 布局
 * 挂在设置页末尾，画板补齐后再定稿。
 *
 * 三个开关的默认值都是「关」，理由各不相同但都在界面上写明：
 * - 保持唤醒改的是系统电源策略；
 * - warm-up 会真实消耗额度；
 * - OTLP 导出会把用量数据送出本机（只送聚合值）。
 */
export function OpsSection() {
  const cfg = useSettings()
  const toast = useToast()
  const settings = cfg.settings
  const [busy, setBusy] = useState<string | null>(null)
  const [versions, setVersions] = useState<AgentVersionRow[] | null>(null)
  const [versionsError, setVersionsError] = useState<string | null>(null)
  const [hourDraft, setHourDraft] = useState<string>(String(settings.warmup_hour ?? 9))
  const [endpointDraft, setEndpointDraft] = useState<string>(settings.otlp_endpoint ?? "")
  const [exporting, setExporting] = useState(false)
  const disabled = !cfg.ready || !isTauri

  // 后端广播新设置后，把草稿对齐（用户没在编辑时直接覆盖）
  useEffect(() => {
    setHourDraft(String(settings.warmup_hour ?? 9))
    setEndpointDraft(settings.otlp_endpoint ?? "")
  }, [settings.warmup_hour, settings.otlp_endpoint])

  const loadVersions = () => {
    if (!isTauri) return
    setVersionsError(null)
    settingsApi.agentVersions()
      .then(setVersions)
      .catch((reason) => {
        setVersions(null)
        setVersionsError(String(reason))
      })
  }

  const toggleKeepAwake = async (next: boolean) => {
    setBusy("keep-awake")
    try {
      await settingsApi.setKeepAwake(next)
      toast.success(next ? "已开启保持唤醒" : "已关闭保持唤醒", next ? "运行期间系统不会进入睡眠" : "恢复系统默认电源策略")
    } catch (reason) {
      toast.danger("保持唤醒设置未保存", String(reason))
    } finally {
      setBusy(null)
    }
  }

  const toggleWarmup = async (next: boolean) => {
    setBusy("warmup")
    try {
      await settingsApi.setWarmup(next, normalizeWarmupHour(Number(hourDraft)))
      toast.success(next ? "已开启每日 warm-up" : "已关闭每日 warm-up", next ? "每天会在设定时间发一条最小请求" : "不再自动发请求")
    } catch (reason) {
      toast.danger("warm-up 设置未保存", String(reason))
    } finally {
      setBusy(null)
    }
  }

  const saveHour = async () => {
    const hour = normalizeWarmupHour(Number(hourDraft))
    setHourDraft(String(hour))
    setBusy("hour")
    try {
      await settingsApi.setWarmup(settings.warmup_enabled === true, hour)
      toast.success("已更新触发时间", `每天 ${hour}:00 之后触发一次`)
    } catch (reason) {
      toast.danger("触发时间未保存", String(reason))
    } finally {
      setBusy(null)
    }
  }

  const saveEndpoint = async () => {
    const value = endpointDraft.trim()
    setBusy("endpoint")
    try {
      const next = await settingsApi.setOtlpEndpoint(value === "" ? null : value)
      setEndpointDraft(next.otlp_endpoint ?? "")
      toast.success(value === "" ? "已关闭用量导出" : "已保存导出端点", value === "" ? "用量不再离开本机" : otlpEndpointHint(next.otlp_endpoint))
    } catch (reason) {
      toast.danger("导出端点未保存", String(reason))
    } finally {
      setBusy(null)
    }
  }

  const exportNow = async () => {
    const endpoint = (settings.otlp_endpoint ?? "").trim()
    if (endpoint === "") {
      toast.danger("还没有导出端点", "先在上面填写并保存一个 OTLP 端点")
      return
    }
    setExporting(true)
    try {
      const result = await settingsApi.otlpExportUsage(endpoint, 1, null)
      toast.success("已导出近 1 天用量", `${result.points} 组平台/模型聚合值 → ${result.endpoint}`)
    } catch (reason) {
      toast.danger("导出失败", String(reason))
    } finally {
      setExporting(false)
    }
  }

  const today = new Date().toLocaleDateString("sv-SE")

  return (
    <>
      <SettingsGroup
        icon={Moon}
        title="保持唤醒"
        desc="运行期间不让系统进入睡眠，避免自动化任务被挂起。"
      >
        <SettingsRow
          className="min-h-[76px]"
          title="保持系统唤醒"
          desc="开启后 Jolu 在运行期间阻止系统睡眠（屏幕仍可关闭）。只影响本机电源策略，退出应用立刻恢复。非 Windows 系统不支持。"
          control={
            <Switch
              label="保持系统唤醒"
              on={settings.keep_awake === true}
              disabled={disabled || busy === "keep-awake"}
              onChange={(next) => void toggleKeepAwake(next)}
            />
          }
        />
      </SettingsGroup>

      <SettingsGroup
        icon={Coffee}
        title="每日 warm-up"
        desc="每天定时发一条最小请求保活订阅通道。默认关闭。"
      >
        <SettingsRow
          className="min-h-[76px]"
          title="启用每日 warm-up"
          desc={`会真实消耗额度：每天到点通过已接线的 Claude Code 或 Codex 发一句「Reply with just: ok」，跑成功才记为已完成，失败会留到下一次检查重试。当前状态：${warmupStateText(settings.warmup_enabled === true, settings.warmup_last_date, today)}。`}
          control={
            <Switch
              label="启用每日 warm-up"
              on={settings.warmup_enabled === true}
              disabled={disabled || busy === "warmup"}
              onChange={(next) => void toggleWarmup(next)}
            />
          }
        />
        <SettingsRow
          className="min-h-[64px]"
          title="触发时间"
          desc="本地时间整点，0–23。到点后第一次调度检查就会发出（检查间隔 30 秒）。"
          control={
            <div className="flex items-center gap-1.5">
              <input
                value={hourDraft}
                onChange={(event) => setHourDraft(event.target.value)}
                inputMode="numeric"
                aria-label="warm-up 触发小时"
                className="h-8 w-[64px] rounded-lg border border-jolu-border bg-jolu-bg px-2 text-center text-[12px] text-jolu-text outline-none focus:border-jolu-accent"
              />
              <span className="text-[11px] text-jolu-muted">点</span>
              <ActionButton disabled={disabled || busy === "hour"} onClick={() => void saveHour()}>保存</ActionButton>
            </div>
          }
        />
      </SettingsGroup>

      <SettingsGroup
        icon={Send}
        title="用量导出（OTLP）"
        desc="把本机用量按 OTLP 指标推给自建的可观测端点。默认不导出。"
      >
        <SettingsRow
          className="min-h-[76px]"
          title="OTLP 端点"
          desc={`${OTLP_SCOPE_NOTICE} 留空即关闭。`}
          control={
            <div className="flex items-center gap-1.5">
              <input
                value={endpointDraft}
                onChange={(event) => setEndpointDraft(event.target.value)}
                placeholder="http://localhost:4318"
                aria-label="OTLP 端点"
                className="h-8 w-[220px] rounded-lg border border-jolu-border bg-jolu-bg px-2 text-[12px] text-jolu-text outline-none focus:border-jolu-accent"
              />
              <ActionButton disabled={disabled || busy === "endpoint"} onClick={() => void saveEndpoint()}>保存</ActionButton>
            </div>
          }
        />
        <SettingsRow
          className="min-h-[64px]"
          title="立即导出近 1 天"
          desc={otlpEndpointHint(settings.otlp_endpoint)}
          control={
            <ActionButton
              busy={exporting}
              disabled={disabled || !settings.otlp_endpoint}
              onClick={() => void exportNow()}
            >立即导出</ActionButton>
          }
        />
      </SettingsGroup>

      <SettingsGroup
        icon={Terminal}
        title="Agent 版本"
        desc="只读检测各 CLI 的版本，给出对应的更新命令，不会自动升级。"
      >
        <div className="flex flex-col gap-2 border-t border-jolu-border py-3.5">
          <div className="flex items-center justify-between">
            <span className="text-[11px] text-jolu-muted">
              {versions === null ? "尚未检测" : `已检测 ${versions.length} 个命令`}
            </span>
            <ActionButton disabled={disabled} onClick={loadVersions}>{versions === null ? "检测版本" : "重新检测"}</ActionButton>
          </div>
          {versionsError && (
            <div className="rounded-lg bg-jolu-status-error-bg px-2.5 py-2 text-[9px] text-jolu-danger">
              检测失败：{versionsError}
            </div>
          )}
          {versions !== null && (
            <div className="overflow-hidden rounded-lg border border-jolu-border">
              <div className="grid grid-cols-[minmax(0,1fr)_110px_minmax(0,1.6fr)] items-center gap-2 bg-jolu-side px-2.5 py-1.5 text-[9px] font-medium text-jolu-muted">
                <span>Agent</span><span>版本</span><span>升级方式</span>
              </div>
              {versions.map((row) => (
                <div
                  key={row.agent}
                  className="grid grid-cols-[minmax(0,1fr)_110px_minmax(0,1.6fr)] items-center gap-2 border-t border-jolu-border px-2.5 py-1.5 text-[11px]"
                >
                  <span className="truncate text-jolu-text" title={row.name}>{row.name}</span>
                  <span className={row.found ? "text-jolu-text" : "text-jolu-muted"}>{versionLabel(row)}</span>
                  {hasRunnableUpdate(row) ? (
                    <code className="truncate font-mono text-[9px] text-jolu-text" title={row.update_command ?? ""}>
                      {row.update_command}
                    </code>
                  ) : (
                    <span className="truncate text-[9px] text-jolu-muted" title={row.update_note}>{row.update_note}</span>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      </SettingsGroup>
    </>
  )
}
