import { invoke, isTauri, type AppSettings } from "./api"
import type { DefaultPage, MainPage } from "./mainNavigation"
import type { Density } from "./appearance"
import type { NetworkDiagnosisDto, NetworkMode } from "./networkSettings"

/** 设置 · 数据与存储（画板 92）：请求记录与会话索引的条数 */
export interface StorageCountsDto {
  requests: number
  sessions: number
}

export const settingsApi = {
  storageCounts: () => invoke<StorageCountsDto>("storage_counts"),
  /** 当前应用版本；浏览器预览没有桌面运行时，返回 null 由界面显示「—」 */
  appVersion: async (): Promise<string | null> => {
    if (!isTauri) return null
    const { getVersion } = await import("@tauri-apps/api/app")
    return getVersion()
  },
  /** 设置 · 常规（画板 86）：默认打开页面 */
  setDefaultPage: (page: DefaultPage) => invoke<AppSettings>("set_default_page", { page }),
  /** 只写入上次访问的页面，不广播设置变更 */
  rememberPage: (page: MainPage) => invoke<void>("remember_page", { page }),
  /** 设置 · 外观（画板 88）：界面缩放、列表密度、减少动效；缩放由后端应用到主窗口 */
  setAppearancePreferences: (zoom: number, density: Density, reduceMotion: boolean) =>
    invoke<AppSettings>("set_appearance_preferences", { zoom, density, reduceMotion }),
  /** 设置 · 网络与代理（画板 90 / 102）：保存应用联网方式；非手动代理传 null 保留上次的代理地址 */
  setNetwork: (mode: NetworkMode, proxyUrl: string | null) => invoke<AppSettings>("set_network", { mode, proxyUrl }),
  /** 用草稿参数做连接诊断，不保存设置；不发模型请求、不带凭证 */
  diagnoseNetwork: (mode: NetworkMode, proxyUrl: string | null) => invoke<NetworkDiagnosisDto>("diagnose_network", { mode, proxyUrl }),
  /** 设置 · 通知（画板 94）：任务提醒的两个开关，传 null 的保持不变 */
  setTaskNotify: (resumed: boolean | null, finished: boolean | null) => invoke<AppSettings>("set_task_notify", { resumed, finished }),
  /** 设置 · 隐私脱敏（P5，画板待确认）：开关立即生效并广播 settings-changed；返回最新设置 */
  privacyMaskingSet: (on: boolean) => invoke<AppSettings>("privacy_masking_set", { on }),
  /** ZCode 通道顺序（画板待确认）：开 = 先试免费 Start Plan；立即生效，不用重启网关 */
  zcodeFreeFirstSet: (on: boolean) => invoke<AppSettings>("zcode_free_first_set", { on }),
  /** 批次 G：保持系统唤醒；不支持的系统上打开会报错，不假装成功 */
  setKeepAwake: (on: boolean) => invoke<AppSettings>("set_keep_awake", { on }),
  /** 批次 G：每日 warm-up 开关与触发小时（后端按 0–23 截断） */
  setWarmup: (enabled: boolean, hour: number) => invoke<AppSettings>("set_warmup", { enabled, hour }),
  /** 批次 G：OTLP 导出端点；传 null 关闭导出，保存前后端会校验协议并补 /v1/metrics */
  setOtlpEndpoint: (endpoint: string | null) => invoke<AppSettings>("set_otlp_endpoint", { endpoint }),
  /** 批次 G：按天导出聚合用量到 OTLP 端点（不含会话与凭据） */
  otlpExportUsage: (endpoint: string, days: number, authorization: string | null) =>
    invoke<OtlpExportResultDto>("otlp_export_usage", { endpoint, days, authorization }),
  /** 批次 G：各 agent CLI 的版本与建议更新命令（只读，不自动升级） */
  agentVersions: () => invoke<AgentVersionRow[]>("agent_versions"),
  /** 批次 G：探测 WSL 与其中 agent 的安装情况（只读） */
  wslProbeAgents: () => invoke<WslProbeDto>("wsl_probe_agents"),
}

/** OTLP 导出结果：成功时带上实际请求的地址与数据点数 */
export interface OtlpExportResultDto {
  endpoint: string
  points: number
  status: number | null
  response: string | null
}

/** 一个 agent 的版本检测结果（与后端 `agent_version::AgentVersion` 一致） */
export interface AgentVersionRow {
  agent: string
  name: string
  found: boolean
  version: string | null
  raw: string | null
  update_command: string | null
  update_note: string
}

export interface WslAgentRow {
  agent: string
  found: boolean
  path: string | null
  config_path: string
  config_exists: boolean
}

export interface WslProbeDto {
  available: boolean
  reason: string | null
  distros: string[]
  default_distro: string | null
  agents: WslAgentRow[]
}
