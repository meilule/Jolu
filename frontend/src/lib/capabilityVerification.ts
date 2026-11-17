/**
 * 平台能力验证三态（总清单 4.7；无画板，界面标「待确认」；测试见 `capabilityVerification.test.mjs`）。
 *
 * 口径：只认真实账号 / 真机跑通的记录；合成测试、本地模拟上游、只跑到初始化成功都不算真实跑通。
 * - verified 已验证：有真实条件跑通的记录；
 * - partial 部分验证：部分档位 / 版本 / 流程真实跑通，其余缺样本；
 * - unverified 未验证：只有实现与合成 / 模拟测试，或记录写明缺真实条件；
 * - none 无记录：没找到任何验证记录（不推断）；
 * - na 不适用：Jolu 没给这个平台做这项能力。
 *
 * 数据由 2026-11-17 逐份核对 `docs/verification/*.md`、`docs/todo/2026-10-22-unverified-integrations.md` 与 AGENTS.md
 * 得出；每格的 `record` 是依据文件，`date` 取记录里真实条件跑通的日期（未验证时为最近一次合成验证日期或空）。
 * 新增真实验证后改这里的对应格，并写明新的依据文件。记录之间的矛盾见 `docs/verification/2026-11-17-capability-verification.md`。
 */

export type VerifyState = "verified" | "partial" | "unverified" | "none" | "na"
export type Capability = "login" | "quota" | "sessions" | "skills" | "automation" | "gateway" | "checkin"

export interface VerifyCell {
  platform: string
  capability: Capability
  state: VerifyState
  /** 依据日期（YYYY-MM-DD）；没有为 null */
  date: string | null
  /** 依据文件（仓库相对路径）；没有记录时为 null */
  record: string | null
  /** 一句话依据（多引原文） */
  evidence: string
  /** 还缺什么样本；没有为 null */
  missing: string | null
}

export const PLATFORMS: readonly { id: string; name: string }[] = [
  { id: "claude", name: "Claude Code" },
  { id: "codex", name: "Codex" },
  { id: "gemini", name: "Gemini" },
  { id: "grok", name: "Grok" },
  { id: "opencode", name: "OpenCode" },
  { id: "zcode", name: "ZCode" },
  { id: "qoder", name: "Qoder" },
  { id: "workbuddy", name: "WorkBuddy" },
  { id: "trae", name: "Trae" },
]

export const CAPABILITIES: readonly { id: Capability; name: string }[] = [
  { id: "login", name: "登录与账号" },
  { id: "quota", name: "额度 / 余额" },
  { id: "sessions", name: "会话与用量" },
  { id: "skills", name: "Skills" },
  { id: "automation", name: "自动化" },
  { id: "gateway", name: "订阅转发" },
  { id: "checkin", name: "签到" },
]

export const STATE_LABEL: Record<VerifyState, string> = {
  verified: "已验证",
  partial: "部分验证",
  unverified: "未验证",
  none: "无记录",
  na: "不适用",
}

export const CELLS: readonly VerifyCell[] = [
  { platform: "claude", capability: "login", state: "partial", date: "2026-10-26", record: "docs/verification/2026-10-26-claude-local-config-precedence.md", evidence: "API Key 本机导入真实跑通（状态 connected）；订阅 OAuth 登录、真实身份接口与跨账号切换仍未验证（2026-10-23-account-backend.md）", missing: "完整订阅 OAuth 登录样本、真实身份接口与跨账号切换" },
  { platform: "claude", capability: "quota", state: "partial", date: "2026-10-26", record: "docs/verification/2026-10-26-claude-local-config-precedence.md", evidence: "「真实只读余额烟测通过，返回 ok、有效 USD 余额字段」；编程套餐真实返回不支持（当前凭证未关联 Coding Plan）", missing: "订阅 OAuth 额度样本、已关联 Coding Plan 的凭证" },
  { platform: "claude", capability: "sessions", state: "verified", date: "2026-10-24", record: "docs/verification/2026-10-24-real-acceptance-preflight.md", evidence: "真实只读统计 Claude Code 149 个会话，未发现读取失败或损坏行；真实短会话压缩、取消与导出通过", missing: "50 轮以上长会话的压缩质量评测样本" },
  { platform: "claude", capability: "skills", state: "partial", date: "2026-10-23", record: "docs/verification/2026-10-22-desktop-acceptance.md", evidence: "隔离目录真机：新建、编辑、停用、启用、导入及冲突处理 15 项通过；未接触真实 ~/.claude/skills", missing: "真实 ~/.claude/skills 下原工具的发现与热加载核对" },
  { platform: "claude", capability: "automation", state: "partial", date: "2026-10-22", record: "docs/verification/2026-10-22-automation-real-run.md", evidence: "真实 Claude Code 2.1.308：立即运行、定时、运行中停止、续接原会话均通过；真实耗尽额度后续跑未测", missing: "真实额度耗尽 → 等待 → 续跑" },
  { platform: "claude", capability: "gateway", state: "unverified", date: "2026-11-04", record: "docs/verification/2026-11-04-subscription-api.md", evidence: "只用假 Claude CLI 和合成账号；尚未进行真实账号生成与额度归账验收", missing: "真实订阅账号经网关的生成请求" },
  { platform: "claude", capability: "checkin", state: "na", date: null, record: "AGENTS.md", evidence: "签到只覆盖 WorkBuddy 国内、Qoder 双版、Trae 国内", missing: null },

  { platform: "codex", capability: "login", state: "partial", date: "2026-10-22", record: "docs/verification/2026-10-22-automation-real-run.md", evidence: "本机订阅连接真实可用（当前登录账号返回 current: true）；浏览器独立登录只有合成验证", missing: "浏览器独立登录真实样本；「设置为使用」的真实切换" },
  { platform: "codex", capability: "quota", state: "verified", date: "2026-10-23", record: "docs/verification/2026-10-23-account-backend.md", evidence: "当前已登录 Codex 的 wham/usage 真实查询成功，真实零保留", missing: "5 小时窗口样本（服务未返回）；实时订阅接口对当前授权返回 403" },
  { platform: "codex", capability: "sessions", state: "verified", date: "2026-10-24", record: "docs/verification/2026-10-24-real-acceptance-preflight.md", evidence: "真实只读统计 Codex 39 个会话、1–120 轮，未发现读取失败", missing: null },
  { platform: "codex", capability: "skills", state: "partial", date: "2026-10-23", record: "docs/verification/2026-10-22-desktop-acceptance.md", evidence: "隔离目录真机：新建 → 停用 → 启用并核对 config.toml 通过；没有接触真实 ~/.codex", missing: "真实 ~/.codex 与 .agents/skills 下原工具的发现与热加载" },
  { platform: "codex", capability: "automation", state: "partial", date: "2026-10-22", record: "docs/verification/2026-10-22-automation-real-run.md", evidence: "codex-cli 0.162.0：立即运行、定时、停止、续接均通过；真实额度耗尽后续跑未测", missing: "真实额度耗尽后续跑；非 Git 目录「需要处理」的真机复测" },
  { platform: "codex", capability: "gateway", state: "unverified", date: "2026-11-04", record: "docs/verification/2026-11-04-subscription-api.md", evidence: "只有本地模拟 Codex（并发刷新、SSE 分块），没有真实账号验收", missing: "真实订阅账号经网关的 Chat / Responses 请求" },
  { platform: "codex", capability: "checkin", state: "na", date: null, record: "AGENTS.md", evidence: "签到范围不含 Codex", missing: null },

  { platform: "gemini", capability: "login", state: "unverified", date: "2026-10-26", record: "docs/verification/2026-10-26-all-provider-queries.md", evidence: "「Gemini 真实来源 unauthorized，本机没有可用登录」", missing: "有效 Google 登录样本（含 Code Assist 初始化）" },
  { platform: "gemini", capability: "quota", state: "unverified", date: "2026-10-26", record: "docs/verification/2026-10-26-all-provider-queries.md", evidence: "真实查询为 unauthorized，没有取得额度；解析只有离线测试", missing: "有效登录下的真实额度响应" },
  { platform: "gemini", capability: "sessions", state: "none", date: null, record: null, evidence: "没有找到 Gemini 本机会话或用量采集的验证记录", missing: "任何真实采集记录" },
  { platform: "gemini", capability: "skills", state: "unverified", date: "2026-10-23", record: "docs/todo/2026-10-22-unverified-integrations.md", evidence: "扫描、写入边界、启停只有合成测试；Gemini 下共享 .agents 只读", missing: "原工具安装后的发现与热加载核对" },
  { platform: "gemini", capability: "automation", state: "na", date: null, record: "AGENTS.md", evidence: "自动化只支持 Claude Code / Codex", missing: null },
  { platform: "gemini", capability: "gateway", state: "unverified", date: "2026-11-04", record: "docs/verification/2026-11-04-subscription-api.md", evidence: "本地模拟 Gemini 项目、生成与归账，不调用真实端点", missing: "真实订阅账号经 Code Assist 的生成请求" },
  { platform: "gemini", capability: "checkin", state: "na", date: null, record: "AGENTS.md", evidence: "签到范围不含 Gemini", missing: null },

  { platform: "grok", capability: "login", state: "unverified", date: "2026-10-26", record: "docs/verification/2026-10-26-all-provider-browser-login.md", evidence: "只有「真实 Grok 设备授权初始化成功」，完整登录不能用初始化成功替代", missing: "完整登录 → 保存 → 真实查询" },
  { platform: "grok", capability: "quota", state: "unverified", date: "2026-10-26", record: "docs/verification/2026-10-26-all-provider-queries.md", evidence: "Grok 额度缺真实样本，自动测试只覆盖解析", missing: "真实授权额度样本" },
  { platform: "grok", capability: "sessions", state: "none", date: null, record: null, evidence: "没有找到 Grok 会话或用量采集的记录（本机没有 Grok 本地数据）", missing: "任何真实采集记录" },
  { platform: "grok", capability: "skills", state: "na", date: null, record: "AGENTS.md", evidence: "Skills 管理未接入 Grok", missing: null },
  { platform: "grok", capability: "automation", state: "na", date: null, record: "AGENTS.md", evidence: "自动化只支持 Claude Code / Codex", missing: null },
  { platform: "grok", capability: "gateway", state: "unverified", date: "2026-11-04", record: "docs/verification/2026-11-04-subscription-api.md", evidence: "本地模拟 Grok token / userinfo，没有调用真实端点", missing: "真实订阅账号经 cli-chat-proxy 的请求" },
  { platform: "grok", capability: "checkin", state: "na", date: null, record: "AGENTS.md", evidence: "签到范围不含 Grok", missing: null },

  { platform: "opencode", capability: "login", state: "unverified", date: "2026-10-23", record: "docs/verification/2026-10-23-real-validation.md", evidence: "「providers list 为 0 个凭证；订阅登录和付费模型请求仍不能声称通过」", missing: "OpenAI 订阅与 Go 账号的真实登录" },
  { platform: "opencode", capability: "quota", state: "unverified", date: "2026-10-26", record: "docs/verification/2026-10-26-all-provider-queries.md", evidence: "OpenCode 订阅缺真实样本", missing: "OpenAI 订阅与 Go 额度的真实样本" },
  { platform: "opencode", capability: "sessions", state: "partial", date: "2026-10-23", record: "docs/verification/2026-10-23-real-validation.md", evidence: "旧表真实读取 15 条统计、总 Token 196,367，与来源总量一致；新表 V2 有数据时缺样本", missing: "SQLite V2 新表有数据时的真实样本" },
  { platform: "opencode", capability: "skills", state: "unverified", date: "2026-10-23", record: "docs/verification/2026-10-23-real-validation.md", evidence: "只用真实 debug skill 确认了共享 .agents/skills 目录；Jolu 的读写与启停只有合成测试", missing: "在 Jolu 中对真实 OpenCode Skills 目录的读写、启停与热加载" },
  { platform: "opencode", capability: "automation", state: "na", date: null, record: "AGENTS.md", evidence: "自动化只支持 Claude Code / Codex", missing: null },
  { platform: "opencode", capability: "gateway", state: "unverified", date: "2026-11-04", record: "docs/verification/2026-11-04-subscription-api.md", evidence: "OpenAI 登录账号复用 Codex 转发路径，只做了合成与回环验证", missing: "真实 OpenAI 订阅账号的转发" },
  { platform: "opencode", capability: "checkin", state: "na", date: null, record: "AGENTS.md", evidence: "签到范围不含 OpenCode", missing: null },

  { platform: "zcode", capability: "login", state: "partial", date: "2026-11-13", record: "docs/verification/2026-11-13-zcode-v2-credential.md", evidence: "本机登录凭据真实读取（10 条全部解开）；浏览器设备流只到初始化 HTTP 200", missing: "浏览器设备流完整登录 → 保存；ZCode API Key 的真实填写与验证" },
  { platform: "zcode", capability: "quota", state: "partial", date: "2026-11-13", record: "docs/verification/2026-11-13-zcode-v2-credential.md", evidence: "真实查询 state=ok（ZCODE WEEKEND BUILD，0 个窗口）；2026-11-05 Start Plan 真实返回 2 个日额度窗口", missing: "付费个人 Coding Plan、团队套餐、API Key 额度样本" },
  { platform: "zcode", capability: "sessions", state: "partial", date: "2026-11-14", record: "docs/verification/2026-11-14-zcode-session-running-state.md", evidence: "真机胶囊显示「工作中」与本轮实时 Token，收尾后判完成正确", missing: "会话正文 sqlite 解析与根会话列表的真机验收" },
  { platform: "zcode", capability: "skills", state: "unverified", date: "2026-10-23", record: "docs/todo/2026-10-22-unverified-integrations.md", evidence: "Skills 只有合成测试", missing: "真实 ZCode Skills 目录核对" },
  { platform: "zcode", capability: "automation", state: "na", date: null, record: "AGENTS.md", evidence: "自动化只支持 Claude Code / Codex", missing: null },
  { platform: "zcode", capability: "gateway", state: "unverified", date: "2026-11-13", record: "docs/verification/2026-11-13-zcode-v2-credential.md", evidence: "真机只读到转发凭据（Coding Plan Key、Start Plan JWT），端到端转发等额度恢复后再测", missing: "真实转发请求（Coding Plan / Start Plan 双通道）" },
  { platform: "zcode", capability: "checkin", state: "na", date: null, record: "AGENTS.md", evidence: "签到范围不含 ZCode", missing: null },

  { platform: "qoder", capability: "login", state: "verified", date: "2026-10-27", record: "docs/verification/2026-10-27-account-connect-dialogs.md", evidence: "「Qoder 国内 / 国际的官方实时身份校验全部通过」；国内浏览器授权 → 保存 → 真实查询通过（2026-10-26）", missing: "整应用重启后的查询；企业与特殊套餐样本" },
  { platform: "qoder", capability: "quota", state: "partial", date: "2026-10-26", record: "docs/verification/2026-10-26-provider-browser-authorization.md", evidence: "国内真实查询 state=ok、3 个窗口；国际版真实 CLI 来源 unauthorized", missing: "国际版真实额度成功样本" },
  { platform: "qoder", capability: "sessions", state: "none", date: null, record: null, evidence: "没有找到 Qoder 会话或用量采集的记录", missing: "任何真实采集记录" },
  { platform: "qoder", capability: "skills", state: "unverified", date: "2026-10-23", record: "docs/todo/2026-10-22-unverified-integrations.md", evidence: "Skills 只有合成测试", missing: "真实 Qoder Skills 目录核对" },
  { platform: "qoder", capability: "automation", state: "na", date: null, record: "AGENTS.md", evidence: "自动化只支持 Claude Code / Codex", missing: null },
  { platform: "qoder", capability: "gateway", state: "partial", date: "2026-11-14", record: "docs/verification/2026-11-14-qoder-model-list.md", evidence: "国内真实模型列表实时拉取成功（12 条）；真实对话尚未验收", missing: "真实对话（流式、非流式、工具调用）；国际版" },
  { platform: "qoder", capability: "checkin", state: "partial", date: "2026-10-27", record: "docs/verification/2026-10-27-daily-checkin.md", evidence: "国内真实只读确认本轮已领取 100 Credits，未重复领取；国际版真实只读为 Inactive", missing: "真实领取请求；国际版可领取活动样本" },

  { platform: "workbuddy", capability: "login", state: "verified", date: "2026-10-27", record: "docs/verification/2026-10-27-account-connect-dialogs.md", evidence: "WorkBuddy 国内 / AI 浏览器账号「官方实时身份校验全部通过」", missing: "整应用重启后的查询；企业样本" },
  { platform: "workbuddy", capability: "quota", state: "partial", date: "2026-10-26", record: "docs/verification/2026-10-26-provider-browser-authorization.md", evidence: "国内重新登录后真实查询 state=ok、1 个窗口（同日另一份记录曾是 401，重新登录后成功）", missing: "WorkBuddy AI 国际版的真实额度记录" },
  { platform: "workbuddy", capability: "sessions", state: "none", date: null, record: null, evidence: "没有找到 WorkBuddy 会话或用量采集的记录", missing: "任何真实采集记录" },
  { platform: "workbuddy", capability: "skills", state: "unverified", date: "2026-10-23", record: "docs/verification/2026-10-23-remaining-tasks.md", evidence: "按鼠鼠决定只读扫描 .codebuddy/skills，只有合成测试", missing: "真实 .codebuddy/skills 扫描核对" },
  { platform: "workbuddy", capability: "automation", state: "na", date: null, record: "AGENTS.md", evidence: "自动化只支持 Claude Code / Codex", missing: null },
  { platform: "workbuddy", capability: "gateway", state: "partial", date: "2026-11-04", record: "docs/verification/2026-11-04-workbuddy-channel-validation.md", evidence: "国内 hy3 真实请求经生产适配器 HTTP 200、收到 10 个 SSE 块；不代表国际版与完整工具任务", missing: "国际版、其他模型、完整工具任务、非流式" },
  { platform: "workbuddy", capability: "checkin", state: "verified", date: "2026-10-27", record: "docs/verification/2026-10-27-workbuddy-checkin-protocol.md", evidence: "国内真实领取：Ready → Claimed，奖励 100", missing: "浏览器账号的真实领取、跨日边界" },

  { platform: "trae", capability: "login", state: "partial", date: "2026-10-27", record: "docs/verification/2026-10-27-account-connect-dialogs.md", evidence: "国内真实网页授权 → 保存，已保存 CN 与 Global Session 通过实时身份核验；多账号缺第二个真实账号", missing: "第二个国内真实账号的登录样本" },
  { platform: "trae", capability: "quota", state: "verified", date: "2026-10-27", record: "docs/verification/2026-10-27-trae-official-statistics.md", evidence: "国内积分汇总真实返回（总量 4700、已用 0）；两版官方额度查询 HTTP 200（2026-10-25）", missing: "付费国际套餐、企业私有域样本" },
  { platform: "trae", capability: "sessions", state: "none", date: null, record: null, evidence: "没有找到 Trae 会话或用量采集的记录（智能体库格式无法核对）", missing: "任何真实采集记录" },
  { platform: "trae", capability: "skills", state: "unverified", date: "2026-10-23", record: "docs/todo/2026-10-22-unverified-integrations.md", evidence: "Skills 只有合成测试", missing: "真实 Trae Skills 目录核对" },
  { platform: "trae", capability: "automation", state: "na", date: null, record: "AGENTS.md", evidence: "自动化只支持 Claude Code / Codex", missing: null },
  { platform: "trae", capability: "gateway", state: "unverified", date: "2026-11-04", record: "docs/verification/2026-11-04-trae-subscription-adapter.md", evidence: "只用合成凭据和本地模拟上游，不代表真实权限与套餐模型", missing: "真实账号的目录与对话（流式与工具）" },
  { platform: "trae", capability: "checkin", state: "verified", date: "2026-10-27", record: "docs/verification/2026-10-27-trae-checkin-protocol.md", evidence: "国内真实领取：Ready → Claimed，奖励 100；国际版不支持", missing: "浏览器账号的真实领取、第二个账号" },
]

/** 某平台某能力的那一格；表里没有时当作「无记录」，不推断 */
export function cellOf(platform: string, capability: Capability): VerifyCell {
  return CELLS.find((cell) => cell.platform === platform && cell.capability === capability)
    ?? { platform, capability, state: "none", date: null, record: null, evidence: "没有找到验证记录", missing: null }
}

/** 各状态的格数（不适用的不算进分母） */
export function stateCounts(cells: readonly VerifyCell[] = CELLS): Record<VerifyState, number> {
  const counts: Record<VerifyState, number> = { verified: 0, partial: 0, unverified: 0, none: 0, na: 0 }
  for (const cell of cells) counts[cell.state] += 1
  return counts
}

/** 悬停 / 详情里的一句话：「部分验证 · 2026-10-26 · 依据 …」 */
export function cellSummary(cell: VerifyCell): string {
  return [STATE_LABEL[cell.state], cell.date, cell.record ? `依据 ${cell.record}` : null].filter(Boolean).join(" · ")
}
