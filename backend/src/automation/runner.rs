use std::collections::{HashMap, HashSet};

use chrono::{Local, NaiveTime, TimeZone};
use rusqlite::Connection;
use serde_json::Value as Json;

use super::{
    clip, now_ms,
    schedule::parse_reset,
    store::set_attention,
    RESUME_PROMPT,
};

/* ─────────────────────────── 执行器输出 ─────────────────────────── */

#[derive(Debug, Default, Clone)]
pub struct Progress {
    pub session_id: Option<String>,
    pub completed: bool,
    pub failed: bool,
    pub result: Option<String>,
    pub error: Option<String>,
    pub input: Option<i64>,
    pub cache_read: Option<i64>,
    pub output: Option<i64>,
    pub files: Vec<String>,
    /// Claude `rate_limit_event` 报告已到限额（`status: "rejected"`）
    pub rate_limited: bool,
    /// 同一事件给出的限额重置时间（毫秒）
    pub limit_resets_at: Option<i64>,
}

pub(crate) fn add(slot: &mut Option<i64>, v: Option<i64>) {
    if let Some(v) = v { *slot = Some(slot.unwrap_or(0) + v); }
}

pub(crate) fn tool_summary(input: &Json) -> String {
    for key in ["file_path", "command", "pattern", "path", "url", "description"] {
        if let Some(s) = input.get(key).and_then(Json::as_str) { return clip(s, 80); }
    }
    String::new()
}

impl Progress {
    pub(crate) fn file(&mut self, path: &str) {
        let path = path.trim();
        if !path.is_empty() && !self.files.iter().any(|f| f == path) && self.files.len() < 50 { self.files.push(path.to_string()); }
    }

    /// 读一行执行器输出，返回需要写入执行日志的行
    pub fn feed(&mut self, platform: &str, line: &str) -> Vec<String> {
        let Ok(v) = serde_json::from_str::<Json>(line.trim()) else { return vec![] };
        let mut logs = Vec::new();
        let kind = v.get("type").and_then(Json::as_str).unwrap_or("");
        if platform == "claude" {
            match kind {
                "system" => {
                    if let Some(id) = v.get("session_id").and_then(Json::as_str) { self.session_id = Some(id.to_string()); }
                }
                // 限额状态变化；`resetsAt` 为 Unix 秒（官方文档 Agent SDK 的 RateLimitInfo）。
                // 主额度 rejected 但超额用量仍可用（`overageStatus` 为 allowed / allowed_warning）时会话继续，不记为耗尽
                "rate_limit_event" => {
                    let info = v.get("rate_limit_info");
                    let field = |k: &str| info.and_then(|i| i.get(k)).and_then(Json::as_str);
                    let overage_ok = matches!(field("overageStatus").or_else(|| field("overage_status")), Some("allowed" | "allowed_warning"));
                    if field("status") == Some("rejected") && !overage_ok {
                        self.rate_limited = true;
                        let at = info.and_then(|i| i.get("resetsAt").or_else(|| i.get("resets_at"))).and_then(Json::as_i64);
                        if let Some(at) = at.and_then(|n| parse_reset(&n.to_string())) { self.limit_resets_at = Some(at); }
                    }
                }
                "assistant" => {
                    let blocks = v.pointer("/message/content").and_then(Json::as_array).cloned().unwrap_or_default();
                    for b in blocks {
                        match b.get("type").and_then(Json::as_str) {
                            Some("text") => {
                                if let Some(t) = b.get("text").and_then(Json::as_str).and_then(|t| t.lines().find(|l| !l.trim().is_empty())) { logs.push(clip(t, 160)); }
                            }
                            Some("tool_use") => {
                                let name = b.get("name").and_then(Json::as_str).unwrap_or("工具");
                                let input = b.get("input").cloned().unwrap_or(Json::Null);
                                if matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
                                    if let Some(p) = input.get("file_path").or_else(|| input.get("notebook_path")).and_then(Json::as_str) { self.file(p); }
                                }
                                let summary = tool_summary(&input);
                                logs.push(if summary.is_empty() { format!("调用 {name}") } else { format!("调用 {name} · {summary}") });
                            }
                            _ => {}
                        }
                    }
                }
                "result" => {
                    if let Some(id) = v.get("session_id").and_then(Json::as_str) { self.session_id = Some(id.to_string()); }
                    let is_error = v.get("is_error").and_then(Json::as_bool).unwrap_or(false);
                    let subtype = v.get("subtype").and_then(Json::as_str).unwrap_or("");
                    let text = v.get("result").and_then(Json::as_str).map(str::to_string);
                    if let Some(u) = v.get("usage") {
                        let n = |k: &str| u.get(k).and_then(Json::as_i64);
                        let input = [n("input_tokens"), n("cache_creation_input_tokens"), n("cache_read_input_tokens")];
                        if input.iter().any(Option::is_some) { add(&mut self.input, Some(input.iter().flatten().sum())); }
                        add(&mut self.cache_read, n("cache_read_input_tokens"));
                        add(&mut self.output, n("output_tokens"));
                    }
                    if !is_error && subtype == "success" {
                        self.completed = true;
                        self.result = text;
                    } else {
                        self.failed = true;
                        self.error = text.filter(|t| !t.trim().is_empty()).or_else(|| Some(format!("执行器返回 {subtype}")));
                    }
                }
                _ => {}
            }
        } else {
            match kind {
                "thread.started" => {
                    if let Some(id) = v.get("thread_id").and_then(Json::as_str) { self.session_id = Some(id.to_string()); }
                }
                "item.completed" => {
                    let item = v.get("item").cloned().unwrap_or(Json::Null);
                    match item.get("type").and_then(Json::as_str) {
                        Some("agent_message") => {
                            if let Some(t) = item.get("text").and_then(Json::as_str) {
                                if let Some(first) = t.lines().find(|l| !l.trim().is_empty()) { logs.push(clip(first, 160)); }
                                self.result = Some(t.to_string());
                            }
                        }
                        Some("command_execution") => {
                            if let Some(c) = item.get("command").and_then(Json::as_str) { logs.push(format!("执行命令 · {}", clip(c, 120))); }
                        }
                        Some("file_change") => {
                            let changes = item.get("changes").and_then(Json::as_array).cloned().unwrap_or_default();
                            for c in &changes {
                                if let Some(p) = c.get("path").and_then(Json::as_str) { self.file(p); }
                            }
                            logs.push(format!("修改文件 · {} 个", changes.len()));
                        }
                        Some("error") => {
                            if let Some(m) = item.get("message").and_then(Json::as_str) { logs.push(format!("提示 · {}", clip(m, 120))); }
                        }
                        _ => {}
                    }
                }
                "turn.completed" => {
                    if let Some(u) = v.get("usage") {
                        let n = |k: &str| u.get(k).and_then(Json::as_i64);
                        add(&mut self.input, n("input_tokens"));
                        add(&mut self.cache_read, n("cached_input_tokens"));
                        add(&mut self.output, n("output_tokens"));
                    }
                    self.completed = true;
                }
                "turn.failed" => {
                    self.failed = true;
                    self.error = v.pointer("/error/message").and_then(Json::as_str).map(str::to_string).or_else(|| Some("turn failed".into()));
                }
                "error" => {
                    if let Some(m) = v.get("message").and_then(Json::as_str) { self.error = Some(m.to_string()); }
                }
                _ => {}
            }
        }
        logs
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Completed,
    Cancelled,
    /// 额度耗尽（5 小时 / 周额度用完）：等额度恢复后续跑
    Limit { resets_at: Option<i64>, message: String },
    /// 短时限流（429 / too many requests，额度并没用完）：一两分钟后重试，不等额度重置（4.7）
    RateLimited { retry_at: Option<i64>, message: String },
    /// 余额不足（按量计费的 Key 欠费 / 402）：等多久都不会好，转「需要处理」（4.7）
    Balance(String),
    Reauth(String),
    /// Codex 拒绝在非 Git、未信任的目录执行；保留目录保护，不加 `--skip-git-repo-check`
    UntrustedDir(String),
    Failed(String),
}

pub(crate) const UNTRUSTED_DIR_REASON: &str = "Codex 只能在 Git 仓库或已信任的目录中执行。请把工作目录改为 Git 仓库，或先在 Codex 中信任该目录。";

/// codex-cli 0.162.0 原文：`Not inside a trusted directory and --skip-git-repo-check was not specified.`
pub fn is_untrusted_dir(text: &str) -> bool {
    has_any(text, &["not inside a trusted directory", "--skip-git-repo-check was not specified"])
}

pub(crate) fn has_any(text: &str, words: &[&str]) -> bool {
    let lower = text.to_lowercase();
    words.iter().any(|w| lower.contains(w))
}

/// `ve hit your`：Claude Code 2.1.308 的 `You've hit your session limit / weekly limit / Opus limit …`
/// 与 Codex 的 `You’ve hit your usage limit`（两种撇号）
pub fn is_limit(text: &str) -> bool {
    has_any(text, &["usage limit", "usage_limit", "rate limit", "rate_limit", "ratelimit", "limit reached", "hit your limit", "ve hit your", "quota", "too many requests", "429", "额度"])
}

/// 余额不足：按量计费的账户没钱了。只认明确的说法，`insufficient_quota` 之类可能指订阅额度的词不算。
pub fn is_balance(text: &str) -> bool {
    has_any(text, &["insufficient balance", "insufficient_balance", "credit balance is too low", "payment required", "402", "余额不足", "欠费", "billing"])
}

/// 短时限流：服务端按频率拒绝，额度本身没用完。用量 / 会话 / 周额度用完的说法优先算额度耗尽。
pub fn is_rate_limit(text: &str) -> bool {
    let exhausted = has_any(text, &["usage limit", "usage_limit", "limit reached", "hit your limit", "ve hit your", "session limit", "weekly limit", "quota", "额度"]);
    !exhausted && has_any(text, &["rate limit", "rate_limit", "ratelimit", "too many requests", "429", "overloaded", "请求过于频繁"])
}

/// 短时限流没给重试时间时等多久再试
pub const RATE_LIMIT_RETRY_MS: i64 = 60_000;

pub fn is_auth(text: &str) -> bool {
    has_any(text, &["401", "unauthorized", "authentication", "invalid api key", "/login", "not logged in", "log in again", "token expired", "expired token", "invalid_grant", "重新登录"])
}

/// 执行器在限额提示里附带的恢复时间，如 `Claude AI usage limit reached|1761300000`
pub fn limit_reset(text: &str) -> Option<i64> {
    text.split('|').skip(1).find_map(|part| {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        (digits.len() >= 9).then(|| parse_reset(&digits)).flatten()
    })
}

/// Codex 限额提示末尾的 `try again at <时间>`，按本机时区：当天为 `3:05 PM`，跨天为 `Oct 23rd, 2026 3:05 PM`
/// （codex-cli 0.162.0 内置格式 `%-I:%M %p` 与 `%b %-d{st|nd|rd|th}, %Y %-I:%M %p`）
pub fn codex_retry_at<Tz: TimeZone>(tz: &Tz, text: &str, now_ms: i64) -> Option<i64> {
    const KEY: &str = "try again at ";
    let start = text.to_ascii_lowercase().find(KEY)? + KEY.len();
    let rest = text[start..].split(['.', '\n']).next()?.trim();
    let parts: Vec<&str> = rest.split_whitespace().collect();
    let local = match parts.as_slice() {
        [time, ampm] => {
            let time = NaiveTime::parse_from_str(&format!("{time} {ampm}"), "%I:%M %p").ok()?;
            let today = tz.timestamp_millis_opt(now_ms).single()?.date_naive();
            let at = tz.from_local_datetime(&today.and_time(time)).earliest()?;
            // 提示只精确到分钟：比现在早 5 分钟以内仍按当天算，更早才是次日
            if at.timestamp_millis() > now_ms - 5 * 60_000 { at } else { tz.from_local_datetime(&today.succ_opt()?.and_time(time)).earliest()? }
        }
        [month, day, year, time, ampm] => {
            let day: String = day.chars().take_while(char::is_ascii_digit).collect();
            let naive = chrono::NaiveDateTime::parse_from_str(&format!("{month} {day} {year} {time} {ampm}"), "%b %d %Y %I:%M %p").ok()?;
            tz.from_local_datetime(&naive).earliest()?
        }
        _ => return None,
    };
    Some(local.timestamp_millis())
}

pub fn classify(p: &Progress, exit_ok: bool, stderr_tail: &str, stopped: bool) -> Outcome {
    if stopped { return Outcome::Cancelled; }
    if p.completed && !p.failed { return Outcome::Completed; }
    let stderr = stderr_tail.trim();
    let message = p.error.clone()
        .or_else(|| (!stderr.is_empty()).then(|| stderr.to_string()))
        .unwrap_or_else(|| if exit_ok { "执行器没有返回结果".into() } else { "执行器异常退出".into() });
    // 先按文本判断；`rate_limit_event` 的结构化标记只在文本没有给出其他判断时兜底，避免吞掉认证、网络等真实原因
    let limit = |message: String| {
        let resets_at = limit_reset(&message).or(p.limit_resets_at).or_else(|| codex_retry_at(&Local, &message, now_ms()));
        Outcome::Limit { resets_at, message }
    };
    if is_untrusted_dir(&message) {
        Outcome::UntrustedDir(message)
    } else if is_balance(&message) {
        Outcome::Balance(message)
    } else if is_rate_limit(&message) {
        let retry_at = limit_reset(&message).or_else(|| codex_retry_at(&Local, &message, now_ms()));
        Outcome::RateLimited { retry_at, message }
    } else if is_limit(&message) {
        limit(message)
    } else if is_auth(&message) {
        Outcome::Reauth(message)
    } else if p.rate_limited {
        limit(message)
    } else {
        Outcome::Failed(message)
    }
}

pub fn build_args(platform: &str, model: Option<&str>, session: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut push = |s: &str| args.push(s.to_string());
    if platform == "codex" {
        push("exec");
        push("--json");
        if let Some(m) = model { push("-m"); push(m); }
        if let Some(s) = session { push("resume"); push(s); }
        // 提示词从标准输入读取
        push("-");
    } else {
        push("-p");
        push("--output-format");
        push("stream-json");
        push("--verbose");
        if let Some(m) = model { push("--model"); push(m); }
        if let Some(s) = session { push("--resume"); push(s); }
    }
    args
}

/* ─────────────────────────── Skills（D-08） ─────────────────────────── */

/// 一个任务最多选择的 Skills 数
pub(crate) const MAX_SKILLS: usize = 10;

/// 提示词前缀：Codex 在提示词里识别 `$名称`；Claude Code 的 `-p` 模式一次只加载第一个 `/skill`，改用自然语言点名
pub fn skill_prefix(platform: &str, names: &[String]) -> String {
    if names.is_empty() { return String::new(); }
    if platform == "codex" {
        let list: Vec<String> = names.iter().map(|n| format!("${n}")).collect();
        format!("{}\n\n", list.join(" "))
    } else {
        format!("本任务请使用以下 Skills：{}\n\n", names.join("、"))
    }
}

/// 写入执行器标准输入的提示词：首次运行为「前缀 + 任务内容」，续跑为「前缀 + 续跑指令」
pub fn run_prompt(platform: &str, skills: &[String], prompt: &str, resume: bool) -> String {
    let body = if resume { RESUME_PROMPT } else { prompt };
    format!("{}{body}", skill_prefix(platform, skills))
}

/// 任务选中、但目标工具当前没有启用的 Skills，按选择顺序
pub fn missing_skills(selected: &[String], available: &[crate::skills::SkillOption]) -> Vec<String> {
    selected.iter().filter(|s| !available.iter().any(|o| &o.name == *s)).cloned().collect()
}

pub(crate) fn missing_reason(missing: &[String]) -> String {
    let names: Vec<String> = missing.iter().map(|n| format!("「{n}」")).collect();
    format!("Skill{}已停用或不存在", names.join("、"))
}

/// 执行前（首次运行与续跑都经过这里）检查选中的 Skills 仍然启用；缺失时转为「需要处理」并返回 false，不启动进程
pub fn skills_ready(conn: &Connection, run_id: i64, selected: &[String], available: &[crate::skills::SkillOption], now: i64) -> rusqlite::Result<bool> {
    let missing = missing_skills(selected, available);
    if missing.is_empty() { return Ok(true); }
    set_attention(conn, run_id, "skill_missing", &missing_reason(&missing), now)?;
    Ok(false)
}


/// 执行中的进程登记与停止请求。认领到写入进程 ID 之间有一个窗口：这期间收到的停止请求
/// 先留下标记，登记进程时发现标记就立即结束进程，按「已取消」收尾
#[derive(Default)]
pub(crate) struct Procs {
    /// 运行 ID → 进程 ID
    pub(crate) pids: HashMap<i64, u32>,
    /// 已请求停止的运行
    pub(crate) stopping: HashSet<i64>,
}

impl Procs {
    /// 认领前清掉同一运行上一轮遗留的停止标记（续跑的运行沿用原来的运行 ID）
    pub(crate) fn clear_stop(&mut self, run_id: i64) {
        self.stopping.remove(&run_id);
    }

    /// 请求停止：留下标记；进程已登记时返回进程 ID，由调用方结束进程树
    pub(crate) fn request_stop(&mut self, run_id: i64) -> Option<u32> {
        self.stopping.insert(run_id);
        self.pids.get(&run_id).copied()
    }

    /// 登记进程；返回 true 表示登记前已有停止请求，需要立即结束
    pub(crate) fn register(&mut self, run_id: i64, pid: u32) -> bool {
        self.pids.insert(run_id, pid);
        self.stopping.contains(&run_id)
    }

    /// 进程退出：注销，返回是否因停止请求结束
    pub(crate) fn exited(&mut self, run_id: i64) -> bool {
        self.pids.remove(&run_id);
        self.stopping.remove(&run_id)
    }
}


/// 结束整个进程树（执行器会再拉起子进程），见 `crate::proc`
pub(crate) fn kill_tree(pid: u32) {
    crate::proc::kill_tree(pid);
}
