/**
 * 5.6 WSL 接线（后端 `gateway/wiring_wsl.rs`；无画板，界面待确认）的类型与纯逻辑。
 */

export interface DistroNetDto {
  distro: string
  home: string
  /** mirrored / nat / virtioproxy / unknown */
  mode: string
  host_ip: string | null
}

export interface WslWiringRowDto {
  distro: string
  agent: string
  path: string
  base: string
  client_key: string | null
  /** 文件里还是 Jolu 写的地址 */
  intact: boolean
  wired_ms: number
}

export interface WslWiringStateDto {
  net: DistroNetDto | null
  net_error: string | null
  rows: WslWiringRowDto[]
  agents: string[]
  lan_shared: boolean
}

export const WSL_AGENT_LABELS: Readonly<Record<string, string>> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  opencode: "OpenCode",
}

/** 网络模式说明：决定能不能接、怎么接 */
export function netSummary(state: WslWiringStateDto): { text: string; canWire: boolean; tone: "ok" | "warn" } {
  if (!state.net) return { text: `读不到发行版的网络情况：${state.net_error ?? "原因未知"}`, canWire: false, tone: "warn" }
  const { mode, host_ip } = state.net
  if (mode === "mirrored" || mode === "virtioproxy") {
    return { text: `${mode} 网络：发行版里的 127.0.0.1 就是 Windows，和本机一样接入`, canWire: true, tone: "ok" }
  }
  if (state.lan_shared && host_ip) {
    return { text: `${mode} 网络：经 Windows 地址 ${host_ip} 接入，每个工具签一把专用 Key（需 Windows 防火墙放行 WSL）`, canWire: true, tone: "ok" }
  }
  return {
    text: `${mode} 网络：发行版里的 127.0.0.1 到不了 Windows。可在 %UserProfile%\\.wslconfig 的 [wsl2] 下写 networkingMode=mirrored 后执行 wsl --shutdown，或在「模型路由 › 对外访问」打开局域网共享`,
    canWire: false,
    tone: "warn",
  }
}

/** 一个工具在该发行版里的状态 */
export function rowState(row: WslWiringRowDto | undefined): { text: string; wired: boolean; tone: "ok" | "warn" | "muted" } {
  if (!row) return { text: "未接入", wired: false, tone: "muted" }
  if (!row.intact) return { text: "已被改走（断开只清记录）", wired: true, tone: "warn" }
  return { text: row.client_key ? `已接入 · 专用 Key ${row.client_key}` : "已接入", wired: true, tone: "ok" }
}
