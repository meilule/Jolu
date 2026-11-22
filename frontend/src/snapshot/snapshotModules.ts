/**
 * 一键签到、Skills、自动化的示例数据（静态快照）。
 * 取自 `mock/checkin.ts`、`mock/skills.ts`、`mock/automation.ts`（画板 58–67 / 74–85 / 149–160）。
 */
import type { AutomationAccountDto, AutomationOverviewDto, QuotaVerdictDto, RunDetailDto, RunPageDto, RunDto, SkillOptionDto, TaskDetailDto, TaskItemDto } from "@/lib/automationApi"
import type { SkillDetailDto, SkillItemDto, SkillsOverviewDto } from "@/lib/skillsApi"
import type { CheckinOverview, CheckinUpdate } from "@/lib/checkinApi"
import type { CheckinState } from "@/lib/checkinApi"
import { NOW } from "./snapshotData"

const MIN = 60_000
const DAY = 86_400_000

/* ─────────────────────────── 一键签到 ─────────────────────────── */

const CHECKIN_ACCOUNTS = [
  { id: "checkin-workbuddy-cn", platform: "workbuddy", edition: "cn", name: "WorkBuddy", account: "user@example.com", state: "claimed", reward: 100, unit: "积分", week: 3, month: 12, all: 26 },
  { id: "checkin-qoder-cn", platform: "qoder", edition: "cn", name: "Qoder CN", account: "momo@example.com", state: "ready", reward: 100, unit: "Credits", week: 2, month: 9, all: 18 },
  { id: "checkin-trae-cn", platform: "trae", edition: "cn", name: "Trae CN", account: "个人账号", state: "ready", reward: 50, unit: "积分", week: 4, month: 15, all: 41 },
  { id: "checkin-qoder-global", platform: "qoder", edition: "global", name: "Qoder", account: "alex@example.com", state: "claimed", reward: 100, unit: "Credits", week: 5, month: 20, all: 33 },
] as const

type CheckinSeed = (typeof CHECKIN_ACCOUNTS)[number]

const claimedCheckins = new Set<string>(CHECKIN_ACCOUNTS.filter((a) => a.state === "claimed").map((a) => a.id))

function checkinOutcome(a: CheckinSeed, claimed: boolean) {
  const state: CheckinState = claimed ? "claimed" : "ready"
  return {
    state,
    message: claimed ? "今日奖励已领取" : "今日奖励可领取",
    reward: a.reward, unit: a.unit, period_key: null as string | null, reset_at: null as number | null,
  }
}

function checkinUpdate(a: CheckinSeed): CheckinUpdate {
  const claimed = claimedCheckins.has(a.id)
  const days = (n: number) => ({ days: n, reward: n * a.reward })
  return {
    id: a.id, account_key: `snap:${a.id}`, account: a.account,
    outcome: checkinOutcome(a, claimed), checked_at: NOW - 4 * MIN,
    statistics: { has_records: true, unit: a.unit, week: days(a.week), month: days(a.month), all: days(a.all) },
  }
}

export function demoCheckinOverview(): CheckinOverview {
  return {
    accounts: CHECKIN_ACCOUNTS.map((a) => ({
      id: a.id, platform: a.platform, edition: a.edition, name: a.name, account: a.account, connection_status: "active",
    })),
    records: CHECKIN_ACCOUNTS.filter((a) => claimedCheckins.has(a.id)).map((a, index) => ({
      id: index + 1, connection_id: a.id, name: a.account, platform: a.platform, edition: a.edition,
      outcome: checkinOutcome(a, true), created_ms: NOW - (index + 1) * 47 * MIN,
    })),
  }
}

export function demoCheckinStatus(id: string): CheckinUpdate {
  const account = CHECKIN_ACCOUNTS.find((a) => a.id === id)
  if (!account) throw new Error("示例账号不存在")
  return checkinUpdate(account)
}

export function demoCheckinClaim(id: string): CheckinUpdate {
  const account = CHECKIN_ACCOUNTS.find((a) => a.id === id)
  if (!account) throw new Error("示例账号不存在")
  claimedCheckins.add(account.id)
  return checkinUpdate(account)
}

/* ─────────────────────────── Skills ─────────────────────────── */

const SKILL_HOME = "C:/Users/demo/.claude/skills"
const SKILL_PROJECT = "D:/develop/jolu"

function skill(name: string, description: string | null, extra: Partial<SkillItemDto> = {}): SkillItemDto {
  return {
    platform: "claude", name, dir_name: name, description, scope: "global", project: null, source: "local",
    enabled: true, status: "enabled", issue: null, path: `${SKILL_HOME}/${name}`, original_path: null,
    disabled_by: null, readonly: false, symlink: false, modified_ms: NOW - 90 * MIN, ...extra,
  }
}

const inProject = (name: string): Partial<SkillItemDto> => ({
  scope: "project", project: SKILL_PROJECT, source: "project", path: `${SKILL_PROJECT}/.claude/skills/${name}`,
})

const CLAUDE_SKILLS: SkillItemDto[] = [
  skill("frontend-design", "设计界面布局与视觉规范"),
  skill("code-review", "审查变更与潜在回归", { source: "git" }),
  skill("project-conventions", "项目编码与目录约定", inProject("project-conventions")),
  skill("test-runner", "选择测试并整理验证结果"),
  skill("git-commit-zh", "生成中文提交信息", { source: "git" }),
  skill("docs-writer", "编写与维护项目文档", inProject("docs-writer")),
  skill("release-notes", "汇总版本更新内容", { enabled: false, status: "disabled", disabled_by: "tool", issue: "由工具配置停用：这条配置按名称生效，会影响所有同名副本" }),
  skill("ui-audit", "检查界面一致性与可访问性", {
    ...inProject("ui-audit"), source: "local", enabled: false, status: "disabled", disabled_by: "parked",
    original_path: `${SKILL_PROJECT}/.claude/skills/ui-audit`,
    path: "C:/Users/demo/AppData/Roaming/jolu/skills-disabled/20261022-claude/ui-audit",
  }),
  skill("frontend-design", "设计界面布局与视觉规范（项目版）", { ...inProject("frontend-design"), status: "conflict", issue: "同名条目：项目与全局同时存在" }),
  skill("api-contract", null, { source: "imported", enabled: false, status: "invalid", issue: "SKILL.md 缺少 description" }),
]

const CODEX_SKILLS: SkillItemDto[] = [
  skill("code-review", "审查变更与潜在回归", { platform: "codex", path: "C:/Users/demo/.agents/skills/code-review" }),
  skill("test-runner", "选择测试并整理验证结果", { platform: "codex", path: "C:/Users/demo/.agents/skills/test-runner" }),
  skill("skill-creator", "创建与更新 Skill", { platform: "codex", scope: "system", source: "system", readonly: true, path: "C:/Users/demo/.codex/skills/.system/skill-creator" }),
]

export function demoSkillsOverview(platform: string): SkillsOverviewDto {
  const items = platform === "claude" ? CLAUDE_SKILLS : platform === "codex" ? CODEX_SKILLS : [...CLAUDE_SKILLS, ...CODEX_SKILLS]
  return {
    platforms: [
      { id: "claude", detected: true, count: CLAUDE_SKILLS.length, verified: true, global_dir: SKILL_HOME },
      { id: "codex", detected: true, count: CODEX_SKILLS.length, verified: true, global_dir: "C:/Users/demo/.agents/skills" },
      { id: "gemini", detected: true, count: 0, verified: true, global_dir: "C:/Users/demo/.gemini/skills" },
      { id: "opencode", detected: false, count: 0, verified: true, global_dir: null },
      { id: "qoder", detected: false, count: 0, verified: true, global_dir: null },
      { id: "trae", detected: false, count: 0, verified: true, global_dir: null },
      { id: "zcode", detected: true, count: 0, verified: true, global_dir: "C:/Users/demo/.zcode/skills" },
      { id: "workbuddy", detected: false, count: 0, verified: false, global_dir: null },
    ],
    items,
    roots: [
      { platform: "claude", scope: "global", project: null, path: SKILL_HOME, exists: true, readonly: false, overridable: true, overridden: false, default_path: null, parked_here: 0 },
      { platform: "claude", scope: "project", project: SKILL_PROJECT, path: `${SKILL_PROJECT}/.claude/skills`, exists: true, readonly: false, overridable: false, overridden: false, default_path: null, parked_here: 0 },
      { platform: "codex", scope: "global", project: null, path: "C:/Users/demo/.agents/skills", exists: true, readonly: false, overridable: false, overridden: false, default_path: null, parked_here: 0 },
      { platform: "codex", scope: "global", project: null, path: "C:/Users/demo/.codex/skills", exists: true, readonly: false, overridable: true, overridden: false, default_path: null, parked_here: 0 },
    ].filter((r) => platform === "all" || r.platform === platform) as SkillsOverviewDto["roots"],
    projects: [
      { path: SKILL_PROJECT, source: "session", ignored: false, tools: ["claude"] },
      { path: "D:/develop/jolu-docs", source: "session", ignored: false, tools: [] },
      { path: "D:/work/api-proxy", source: "manual", ignored: false, tools: [] },
    ],
    errors: [],
    scanned_at: NOW - 2 * MIN,
  }
}

const DEMO_SKILL_CONTENT = `---
name: frontend-design
description: 设计界面布局、视觉层级与交互细节。
---

# Frontend Design

为现有项目建立清晰、统一且可落地的界面设计。

## 适用场景
新建页面、调整布局、优化表格或审查交互一致性时使用。

## 执行步骤
1. 阅读当前界面、主题与用户约束。
2. 确定页面信息层级和主要操作。
3. 复用已有组件、颜色和图标。
4. 检查空状态、加载、错误与键盘操作。

## 输出要求
说明具体改动，给出可检查的设计结果；未确定的能力明确标注。

## 附带资源
references/design-checklist.md
assets/preview.png
`

export function demoSkillDetail(item: SkillItemDto): SkillDetailDto {
  return {
    item,
    files: [
      { rel: "SKILL.md", dir: false, depth: 0, size: 812 },
      { rel: "references", dir: true, depth: 0, size: null },
      { rel: "references/design-checklist.md", dir: false, depth: 1, size: 1260 },
      { rel: "assets", dir: true, depth: 0, size: null },
      { rel: "assets/preview.png", dir: false, depth: 1, size: 48_213 },
    ],
    files_truncated: false,
    content: item.name === "frontend-design" ? DEMO_SKILL_CONTENT : `---\nname: ${item.name}\ndescription: ${item.description ?? ""}\n---\n\n# ${item.name}\n\n${item.description ?? "（示例内容）"}\n`,
    truncated: false,
  }
}

export function demoSkillFile(item: SkillItemDto, rel: string) {
  if (rel.endsWith(".png")) return { text: null, binary: true, size: 48_213, truncated: false }
  return { text: `# ${rel}\n\n${item.description ?? "示例文件内容"}\n`, binary: false, size: 812, truncated: false }
}

export function demoSkillImportScan(target: string) {
  return {
    source: "D:/Downloads/jolu-skills/",
    target,
    items: [
      { name: "api-review", path: "D:/Downloads/jolu-skills/api-review", description: "在提交前核对 API 契约和错误处理", status: "ok" as const, issue: null },
      { name: "frontend-design", path: "D:/Downloads/jolu-skills/frontend-design", description: "设计界面", status: "conflict" as const, issue: "目标已有同名 Skill" },
      { name: "quick-test", path: "D:/Downloads/jolu-skills/quick-test", description: null, status: "invalid" as const, issue: "SKILL.md 缺少 description，需先修复" },
    ],
  }
}

/* ─────────────────────────── 自动化 ─────────────────────────── */

const automationAt = (h: number, m: number, dayOffset = 0): number => {
  const d = new Date(NOW)
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + dayOffset, h, m).getTime()
}

function runRow(patch: Partial<RunDto> & Pick<RunDto, "id" | "task_id" | "task_name" | "platform" | "connection_id" | "status" | "created_at">): RunDto {
  return {
    source: "now", block_kind: null, reason: null, resets_at: null, last_check_at: null, next_check_at: null,
    check_failures: 0, resume_pending: false, session_id: null, account: null, model: null, started_at: null,
    ended_at: null, exec_ms: 0, wait_ms: 0, exec_started_at: null, wait_started_at: null, waits: 0,
    input_tokens: null, cache_read_tokens: null, output_tokens: null, result: null, files: [], error: null,
    ...patch,
  }
}

function automationTasks(): TaskItemDto[] {
  const common = {
    model: null, session_id: null, skills: [] as string[], schedule: null, auto_resume: true, wait_deadline: null,
    state: "active" as const, next_run_at: null, missed_at: null, last: null, queue_ahead: null, serial_queue: true,
  }
  return [
    {
      ...common, id: 24, name: "完善登录页面", platform: "claude", connection_id: "conn-claude-momo", cwd: "D:/develop/jolu",
      model: "claude-sonnet-4-5", session_id: "snap-claude-1",
      prompt: "完善登录页面的表单校验和错误提示，沿用当前绿色主题。完成后运行相关测试，并汇总改动与验证结果。",
      skills: ["项目约定", "测试检查"], launch: "quota", created_at: automationAt(16, 2), updated_at: automationAt(16, 48),
      active: runRow({
        id: 93, task_id: 24, task_name: "完善登录页面", platform: "claude", connection_id: "conn-claude-momo", status: "waiting", source: "quota",
        block_kind: "quota", reason: "5 小时额度已用完", resets_at: automationAt(18, 30), last_check_at: automationAt(16, 48),
        next_check_at: automationAt(18, 30), resume_pending: true, session_id: "snap-claude-1", account: "Momo",
        created_at: automationAt(16, 2), started_at: automationAt(16, 2), exec_ms: 14 * MIN, wait_started_at: automationAt(16, 16), waits: 1,
      }),
    },
    {
      ...common, id: 25, name: "补充接口测试", platform: "codex", connection_id: "conn-codex-alex", cwd: "D:/work/api-proxy",
      prompt: "为代理接口补充单元测试。", launch: "now", created_at: automationAt(16, 30), updated_at: automationAt(16, 44),
      active: runRow({ id: 92, task_id: 25, task_name: "补充接口测试", platform: "codex", connection_id: "conn-codex-alex", status: "running", account: "Alex", created_at: NOW - 5 * MIN, started_at: NOW - 272_000, exec_started_at: NOW - 272_000 }),
    },
    {
      ...common, id: 26, name: "整理项目文档", platform: "claude", connection_id: "conn-claude-momo", cwd: "D:/develop/jolu-docs",
      prompt: "整理项目文档目录。", launch: "quota", created_at: automationAt(16, 20), updated_at: automationAt(16, 40), queue_ahead: 1,
      active: runRow({ id: 94, task_id: 26, task_name: "整理项目文档", platform: "claude", connection_id: "conn-claude-momo", status: "queued", source: "quota", account: "Momo", created_at: automationAt(16, 20) }),
    },
    {
      ...common, id: 27, name: "生成项目日报", platform: "codex", connection_id: "conn-codex-alex", cwd: "D:/develop/jolu",
      prompt: "汇总今天的提交并生成日报。", launch: "schedule",
      schedule: { freq: "daily" as const, time: "09:00", weekday: null, date: null, timezone: "Asia/Shanghai" },
      next_run_at: automationAt(9, 0, 1), created_at: automationAt(9, 0, -7), updated_at: automationAt(9, 3), active: null,
    },
    {
      ...common, id: 28, name: "检查缓存统计", platform: "claude", connection_id: "conn-claude-team", cwd: "D:/develop/jolu",
      prompt: "核对缓存命中统计。", launch: "now", created_at: automationAt(8, 42), updated_at: automationAt(8, 43),
      active: runRow({ id: 95, task_id: 28, task_name: "检查缓存统计", platform: "claude", connection_id: "conn-claude-team", status: "waiting", block_kind: "quota", reason: "周额度已用完", resets_at: automationAt(9, 0, 2), resume_pending: false, account: "Team", created_at: automationAt(8, 42), wait_started_at: automationAt(8, 42) }),
    },
    {
      ...common, id: 29, name: "整理发布说明", platform: "codex", connection_id: "conn-codex-alex", cwd: "D:/develop/jolu-docs",
      prompt: "整理本周发布说明。", launch: "now", state: "paused", created_at: automationAt(8, 0), updated_at: automationAt(8, 20), active: null,
    },
  ]
}

export function demoAutomationOverview(): AutomationOverviewDto {
  return {
    tasks: automationTasks(), enabled: 5, running: 1, running_note: "Codex · Alex", waiting: 2,
    next_reset: automationAt(18, 30), done_today: 8, failed_today: 1,
  }
}

export function demoAutomationTask(id: number): TaskDetailDto {
  const item = automationTasks().find((x) => x.id === id) ?? automationTasks()[0]!
  return {
    item, execution: item,
    queue: [{ task_id: 24, name: "完善登录页面", status: "waiting" }, { task_id: 26, name: "整理项目文档", status: "queued" }].filter((q) => q.task_id === item.id || item.connection_id === "conn-claude-momo"),
    events: [
      { ts: automationAt(16, 2), kind: "start", title: "开始执行", detail: "在所选会话中追加任务指令" },
      { ts: automationAt(16, 16), kind: "pause", title: "额度耗尽，保留进度", detail: "检查表单校验与错误提示" },
      { ts: automationAt(16, 48), kind: "check", title: "检查额度", detail: "5 小时额度已用完" },
    ],
  }
}

function automationRunRows(): RunDto[] {
  return [
    runRow({ id: 92, task_id: 25, task_name: "补充接口测试", platform: "codex", connection_id: "conn-codex-alex", status: "running", account: "Alex", created_at: automationAt(18, 42), started_at: automationAt(18, 42), exec_started_at: NOW - 272_000, input_tokens: 14_200, output_tokens: 4_400 }),
    runRow({
      id: 91, task_id: 24, task_name: "完善登录页面", platform: "claude", connection_id: "conn-claude-momo", status: "completed", source: "quota",
      account: "Momo", model: "claude-sonnet-4-5", session_id: "snap-claude-1", created_at: automationAt(16, 2),
      started_at: automationAt(16, 2), ended_at: automationAt(18, 34), exec_ms: 1_122_000, wait_ms: 8_043_000, waits: 1,
      input_tokens: 46_200, cache_read_tokens: 38_000, output_tokens: 8_600,
      result: "已完善登录表单校验，并统一错误提示样式。额度恢复后延续原会话，保留此前完成的修改。\n- 补充邮箱、密码必填与格式校验\n- 统一输入框错误态及绿色聚焦样式\n- 完成相关测试并汇总验证结果",
      files: ["src/LoginForm.tsx", "src/LoginForm.test.tsx"],
    }),
    runRow({ id: 90, task_id: 26, task_name: "整理项目文档", platform: "claude", connection_id: "conn-claude-momo", status: "completed", source: "quota", account: "Momo", created_at: automationAt(18, 37), started_at: automationAt(18, 37), ended_at: automationAt(18, 42), exec_ms: 316_000, input_tokens: 18_000, output_tokens: 4_400 }),
    runRow({ id: 89, task_id: 27, task_name: "生成项目日报", platform: "codex", connection_id: "conn-codex-alex", status: "completed", source: "schedule", account: "Alex", created_at: automationAt(9, 0), started_at: automationAt(9, 0), ended_at: automationAt(9, 3), exec_ms: 192_000, input_tokens: 10_000, output_tokens: 2_200 }),
    runRow({ id: 88, task_id: 28, task_name: "检查缓存统计", platform: "claude", connection_id: "conn-claude-team", status: "failed", account: "Team", created_at: automationAt(8, 42), started_at: automationAt(8, 42), ended_at: automationAt(8, 42), exec_ms: 8_000, error: "登录失效，需要处理" }),
    runRow({ id: 87, task_id: 30, task_name: "整理测试清单", platform: "codex", connection_id: "conn-codex-alex", status: "cancelled", source: "manual", account: "Alex", created_at: automationAt(8, 20), started_at: automationAt(8, 20), ended_at: automationAt(8, 22), exec_ms: 166_000, input_tokens: 8_000, output_tokens: 1_600 }),
  ]
}

export function demoAutomationRuns(page: number): RunPageDto {
  return {
    rows: automationRunRows(), total: 11, page, pages: 2, completed: 8, completed_waited: 2, running: 1,
    failed: 1, failed_note: "登录失效，需要处理", tokens: 428_600,
    tasks: automationTasks().map((t) => ({ task_id: t.id, name: t.name, status: t.state })),
  }
}

export function demoAutomationRunDetail(id: number): RunDetailDto {
  const row = automationRunRows().find((x) => x.id === id) ?? automationRunRows()[1]!
  return {
    run: row,
    task: automationTasks().find((x) => x.id === row.task_id) ?? null,
    events: [
      { ts: automationAt(16, 2), kind: "start", title: "开始执行", detail: "读取项目上下文与登录组件" },
      { ts: automationAt(16, 16), kind: "pause", title: "额度耗尽，保留进度", detail: "等待 5 小时额度恢复" },
      { ts: automationAt(18, 30), kind: "check", title: "检查额度恢复", detail: "5 小时与周额度均可用" },
      { ts: automationAt(18, 30), kind: "start", title: "恢复原会话", detail: "继续未完成的校验与测试" },
      { ts: automationAt(18, 34), kind: "done", title: "完成任务", detail: "结果已关联到原会话" },
    ],
    logs: [
      { ts: automationAt(18, 30), kind: "log", title: "额度检查通过，恢复原会话", detail: null },
      { ts: automationAt(18, 30), kind: "log", title: "继续检查表单错误提示", detail: null },
      { ts: automationAt(18, 32), kind: "log", title: "执行相关测试", detail: null },
      { ts: automationAt(18, 34), kind: "log", title: "返回结果，任务完成", detail: null },
    ],
  }
}

export function demoAutomationAccounts(platform: "claude" | "codex"): AutomationAccountDto[] {
  return platform === "claude"
    ? [{ id: "conn-claude-momo", name: "Momo", kind: "auth", status: "active", current: true }, { id: "conn-claude-team", name: "Team", kind: "auth", status: "active", current: false }]
    : [{ id: "conn-codex-alex", name: "Alex", kind: "auth", status: "active", current: true }]
}

export function demoAutomationQuota(): QuotaVerdictDto {
  return {
    state: "blocked", reason: "5 小时窗口已用完", resets_at: automationAt(18, 30),
    windows: [
      { key: "5h", name: "5 小时窗口", used_percent: 100, resets_at: automationAt(18, 30), blocking: true },
      { key: "7d", name: "7 天窗口", used_percent: 58, resets_at: automationAt(9, 0, 2), blocking: false },
    ],
  }
}

export const DEMO_AUTOMATION_MODELS = {
  claude: ["claude-sonnet-4-5", "claude-opus-4-1"],
  codex: ["gpt-5-codex", "gpt-5"],
}

export function demoAutomationSkills(platform: "claude" | "codex", cwd: string | null): SkillOptionDto[] {
  const global: SkillOptionDto[] = platform === "claude"
    ? [{ name: "测试检查", description: "修改后运行相关测试并汇总结果", scope: "global", project: null }, { name: "api-review", description: "核对接口契约与错误码", scope: "global", project: null }]
    : [{ name: "测试检查", description: "修改后运行相关测试并汇总结果", scope: "global", project: null }]
  return cwd ? [...global, { name: "项目约定", description: "遵守仓库的目录、命名与提交约定", scope: "project", project: cwd }] : global
}

export { DAY }
