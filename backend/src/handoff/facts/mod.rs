//! 会话交接 · 事实区（计划 3.3 第 1 条）：程序从解析后的会话消息里原样抽取，不调用模型、不经模型改写。
//!
//! - Facts 里的 `idx` 是内部下标（0 起，与会话页、交接输入一致）；渲染成 `#n` 时一律经 `handoff_input::msg_ref` 换算为从 1 起的编号；
//! - 失败与退出码只取来源记录（`ToolCall.failed` / `exit_code`），取不到写「未知」，不推断；
//! - 进入事实区的命令与输出先遮蔽常见的密钥形态，交接文档不带 API Key 与认证信息。

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::handoff_input::{estimate_tokens, msg_ref};
use crate::session_threads::{Message, ToolCall};

/// 用户原话时间线保留的条数与每条字数
const TIMELINE_LIMIT: usize = 30;
const TIMELINE_CHARS: usize = 300;
const COMMAND_LIMIT: usize = 20;
const FAILURE_LIMIT: usize = 10;
const VERIFY_TAIL_LINES: usize = 15;
/// 命令原文、验证末尾每行、报错行的长度上限
const LINE_CHARS: usize = 300;
/// 事实区总量上限（约 5k token），超出时按优先级裁剪
pub const FACTS_TOKEN_BUDGET: usize = 5_000;
const GIT_DIRTY_LIMIT: usize = 50;
const GIT_TIMEOUT: Duration = Duration::from_secs(5);
/// 渲染时文件清单最多列出的条数，其余写「另有 N 个」
const RENDER_FILE_LIMIT: usize = 40;

/// 记为「改过」的工具（Codex FileChange 事件在解析时也记为 apply_patch）
const MODIFY_TOOLS: [&str; 5] = ["Edit", "Write", "MultiEdit", "NotebookEdit", "apply_patch"];
const READ_TOOLS: [&str; 3] = ["Read", "Glob", "Grep"];
/// Bash 类调用。Codex 的 `exec` 是脚本工具，真实命令在 `command_execution` 事件里，不重复计入
const COMMAND_TOOLS: [&str; 9] = ["Bash", "PowerShell", "shell", "shell_command", "exec_command", "local_shell_call", "local_shell", "container.exec", "command_execution"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandFact {
    pub idx: usize,
    pub command: String,
    pub exit: Option<i32>,
    /// 最后一次验证：输出末尾 15 行；失败的尝试：报错的第一行；命令清单里为空
    pub tail: String,
    /// 来源记录的失败标记（Claude `is_error` 等）；成功的 Claude Bash 没有退出码，只能靠它区分「成功」与「未知」
    #[serde(default)]
    pub failed: Option<bool>,
}

impl CommandFact {
    /// 退出状态：Some(true) 失败、Some(false) 成功、None 未知
    pub fn is_failure(&self) -> Option<bool> {
        match self.exit {
            Some(code) => Some(code != 0 || self.failed == Some(true)),
            None => self.failed,
        }
    }

    fn status_text(&self) -> String {
        match (self.exit, self.is_failure()) {
            (Some(code), _) => format!("退出码 {code}"),
            (None, Some(true)) => "失败（退出码未记录）".into(),
            (None, Some(false)) => "成功（退出码未记录）".into(),
            (None, None) => "退出状态未知".into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GitSnapshot {
    /// 分离 HEAD 时为 None
    pub branch: Option<String>,
    /// 短 SHA；还没有提交时为 None
    pub head: Option<String>,
    /// 未提交的文件（`git status --porcelain` 原样，最多 50 项）
    pub dirty: Vec<String>,
    pub dirty_total: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Facts {
    pub user_timeline: Vec<(usize, String)>,
    pub earlier_user_count: usize,
    pub files_modified: Vec<(String, usize)>,
    pub files_read: Vec<(String, usize)>,
    pub commands: Vec<CommandFact>,
    pub last_verify: Option<CommandFact>,
    pub failures: Vec<CommandFact>,
    pub git: Option<GitSnapshot>,
    pub transcript_path: Option<String>,
    /// 全部 Bash 类调用条数（`commands` 只保留 20 条）
    #[serde(default)]
    pub commands_total: usize,
    /// 会话消息总条数（校验「下一步」引用的 `#n` 不超出范围）
    #[serde(default)]
    pub total: usize,
    /// 全部用户消息的内部下标（`user_timeline` 只留最近 30 条；校验「下一步」至少引用一条用户消息时用全部）
    #[serde(default)]
    pub user_indices: Vec<usize>,
}

fn clip(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    format!("{}…", text.chars().take(limit).collect::<String>())
}


mod redact;
pub use redact::redact;
pub(crate) use redact::{find_value, token_spans, PEM_MAX, PREFIX_KEYS, SECRET_MIN, VALUE_MAX};
#[cfg(test)]
use redact::*;

/// 行首形如 `#12 用户：` / `#12 助手：` 的文字会冒充消息标记，前面加 `\`（精简原文时用于正文与工具输出）
pub(crate) fn escape_role_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (k, line) in text.split('\n').enumerate() {
        if k > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        let rest = trimmed.strip_prefix('#').map(|r| r.trim_start_matches(|c: char| c.is_ascii_digit()));
        let marker = rest.is_some_and(|r| r.len() < trimmed.len() - 1 && {
            let r = r.trim_start();
            r.starts_with("用户") || r.starts_with("助手")
        });
        if marker {
            out.push_str(&line[..line.len() - trimmed.len()]);
            out.push('\\');
            out.push_str(trimmed);
        } else {
            out.push_str(line);
        }
    }
    out
}

/* ------------------------------ 抽取 ------------------------------ */

/// 测试、构建、检查类命令：`test|build|check|lint|typecheck|tsc|pytest|jest|vitest|cargo (test|build|check|clippy)`。
/// 按词匹配（`--test`、`test:unit` 算，`test.txt`、`checkout` 不算）
pub fn is_verify_command(cmd: &str) -> bool {
    const WORDS: [&str; 10] = ["test", "build", "check", "lint", "typecheck", "tsc", "pytest", "jest", "vitest", "clippy"];
    let tokens: Vec<String> = cmd
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '"' | '\''))
        .filter(|t| !t.is_empty())
        .map(|t| {
            let t = t.trim_start_matches('-');
            let t = t.split(':').next().unwrap_or(t);
            t.rsplit(['/', '\\']).next().unwrap_or(t).trim_end_matches(".exe").to_ascii_lowercase()
        })
        .collect();
    tokens.iter().enumerate().any(|(i, t)| {
        if t == "clippy" {
            return i > 0 && tokens[i - 1] == "cargo";
        }
        WORDS.contains(&t.as_str())
    })
}

/// 路径比较键：`\` 换成 `/`、去掉末尾 `/`、不区分大小写（Windows）
pub(crate) fn path_key(path: &str) -> String {
    path.trim().replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// 工作目录下的文件显示为相对路径，其余保持原样
pub(crate) fn display_path(path: &str, cwd: Option<&Path>) -> String {
    let path = path.trim();
    if let Some(cwd) = cwd.and_then(|c| c.to_str()) {
        let base = path_key(cwd);
        let key = path_key(path);
        if !base.is_empty() && key.starts_with(&format!("{base}/")) {
            let rest_len = key.len() - base.len() - 1;
            let normalized = path.replace('\\', "/");
            if let Some(rest) = normalized.get(normalized.len().saturating_sub(rest_len)..) {
                return rest.to_string();
            }
        }
    }
    path.to_string()
}

/// 工具涉及的文件：解析时取到的路径；手写的消息（测试、旧数据）退回到读写类工具的摘要
pub(crate) fn tool_paths(tool: &ToolCall) -> Vec<String> {
    if !tool.paths.is_empty() {
        return tool.paths.clone();
    }
    let by_summary = matches!(tool.name.as_str(), "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit");
    if by_summary && !tool.summary.trim().is_empty() { vec![tool.summary.trim().to_string()] } else { Vec::new() }
}

pub(crate) fn tool_failed(tool: &ToolCall) -> bool {
    tool.failed == Some(true) || tool.exit_code.is_some_and(|c| c != 0)
}

/// 报错的第一行：跳过退出码、耗时等元信息行
pub(crate) fn first_error_line(output: Option<&str>) -> String {
    const META: [&str; 8] = ["Exit code", "Process exited with code", "Script failed", "Script completed", "Wall time", "Chunk ID", "Original token count", "Output:"];
    output
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !META.iter().any(|m| l.starts_with(m)))
        .map(|l| clip(&redact(l), LINE_CHARS))
        .unwrap_or_default()
}

/// 末尾 n 行，每行先遮蔽再截到 300 字（先截断会留下绕过遮蔽的残片）
pub(crate) fn last_lines(output: Option<&str>, n: usize) -> String {
    let lines: Vec<&str> = output.unwrap_or("").trim_end().lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| clip(&redact(l), LINE_CHARS)).collect::<Vec<_>>().join("\n")
}

pub fn extract_facts(messages: &[Message], cwd: Option<&Path>, transcript: Option<&Path>) -> Facts {
    let users: Vec<(usize, String)> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "user" && !m.text.trim().is_empty())
        .map(|(i, m)| (i, clip(&redact(&m.text), TIMELINE_CHARS)))
        .collect();
    let earlier = users.len().saturating_sub(TIMELINE_LIMIT);

    // 文件：键 → (显示名, 最后涉及的 #n)；改过的从读过里去掉
    let mut last: HashMap<String, (String, usize)> = HashMap::new();
    let mut modified: Vec<String> = Vec::new();
    let mut read: Vec<String> = Vec::new();
    let mut all_commands: Vec<CommandFact> = Vec::new();
    let mut failures: Vec<CommandFact> = Vec::new();
    let mut last_verify: Option<(CommandFact, Option<String>)> = None;
    for (i, m) in messages.iter().enumerate() {
        for tool in &m.tools {
            let name = tool.name.as_str();
            let is_modify = MODIFY_TOOLS.contains(&name);
            if is_modify || READ_TOOLS.contains(&name) {
                for path in tool_paths(tool) {
                    let key = path_key(&path);
                    last.insert(key.clone(), (display_path(&path, cwd), i));
                    // 失败的编辑没有改动文件，只算涉及过
                    let list = if is_modify && !tool_failed(tool) { &mut modified } else { &mut read };
                    if !list.contains(&key) {
                        list.push(key);
                    }
                }
            }
            let is_command = COMMAND_TOOLS.contains(&name);
            if is_command {
                let fact = CommandFact { idx: i, command: clip(&redact(tool.summary.trim()), LINE_CHARS), exit: tool.exit_code, tail: String::new(), failed: tool.failed };
                if is_verify_command(&tool.summary) {
                    last_verify = Some((fact.clone(), tool.output.clone()));
                }
                all_commands.push(fact);
            }
            if tool_failed(tool) {
                let command = if is_command || tool.summary.trim().is_empty() { tool.summary.trim().to_string() } else { format!("{name} {}", tool.summary.trim()) };
                let command = if command.is_empty() { name.to_string() } else { command };
                failures.push(CommandFact { idx: i, command: clip(&redact(&command), LINE_CHARS), exit: tool.exit_code, tail: first_error_line(tool.output.as_deref()), failed: tool.failed });
            }
        }
    }
    let files = |keys: &[String], skip: &[String]| -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = keys.iter().filter(|k| !skip.contains(k)).filter_map(|k| last.get(k).cloned()).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    };
    let files_modified = files(&modified, &[]);
    let files_read = files(&read, &modified);

    // 命令：最多 20 条，测试 / 构建 / 检查类排在前面；各自保留最近的，按时间顺序列出
    let (verify, other): (Vec<CommandFact>, Vec<CommandFact>) = all_commands.iter().cloned().partition(|c| is_verify_command(&c.command));
    let verify: Vec<CommandFact> = verify[verify.len().saturating_sub(COMMAND_LIMIT)..].to_vec();
    let room = COMMAND_LIMIT - verify.len();
    let other: Vec<CommandFact> = other[other.len().saturating_sub(room)..].to_vec();
    let commands = verify.into_iter().chain(other).collect();

    Facts {
        user_timeline: users[earlier..].to_vec(),
        earlier_user_count: earlier,
        files_modified,
        files_read,
        commands_total: all_commands.len(),
        commands,
        last_verify: last_verify.map(|(mut fact, output)| {
            fact.tail = last_lines(output.as_deref(), VERIFY_TAIL_LINES);
            fact
        }),
        failures: failures[failures.len().saturating_sub(FAILURE_LIMIT)..].to_vec(),
        git: cwd.filter(|c| c.is_dir()).and_then(git_snapshot),
        transcript_path: transcript.map(|p| p.display().to_string()),
        total: messages.len(),
        user_indices: users.iter().map(|(i, _)| *i).collect(),
    }
}

/* ------------------------------ Git 快照 ------------------------------ */

/// 只读执行 git：不写索引锁（`GIT_OPTIONAL_LOCKS=0`）、不触发 fsmonitor 钩子、不弹窗、超时即结束
pub(crate) fn run_git(cwd: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(cwd).args(["-c", "core.quotepath=false", "-c", "core.fsmonitor=false"]).args(args).env("GIT_OPTIONAL_LOCKS", "0").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut text);
        text
    });
    // 超时结束整棵进程树（git 会起 git-remote-https 等子进程；只杀 git 的话子进程还拿着
    // stdout 管道，读线程就一直挂着，见 `crate::proc`）
    let status = crate::proc::wait_or_kill_tree(&mut child, GIT_TIMEOUT, Duration::from_millis(20))?;
    let text = reader.join().ok()?;
    status.success().then_some(text)
}

/// 只读获取分支、HEAD 和未提交的文件（最多 50 项）；不是 Git 仓库或 git 不可用时为 None
pub fn git_snapshot(cwd: &Path) -> Option<GitSnapshot> {
    let status = run_git(cwd, &["status", "--porcelain=v1", "--branch", "--untracked-files=normal"])?;
    let mut lines = status.lines();
    let branch = lines.next().and_then(|l| l.strip_prefix("## ")).and_then(|l| {
        let l = l.strip_prefix("No commits yet on ").unwrap_or(l);
        let name = l.split("...").next().unwrap_or(l).split(' ').next().unwrap_or(l).trim();
        (!name.is_empty() && name != "HEAD").then(|| name.to_string())
    });
    let dirty: Vec<String> = lines.filter(|l| !l.trim().is_empty()).map(str::to_string).collect();
    let head = run_git(cwd, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    Some(GitSnapshot { branch, head, dirty_total: dirty.len(), dirty: dirty.into_iter().take(GIT_DIRTY_LIMIT).collect() })
}

/* ------------------------------ 渲染 ------------------------------ */

/// 行内代码：内容带反引号时加长分隔符
pub(crate) fn inline_code(text: &str) -> String {
    let text = text.replace(['\r', '\n'], " ");
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    if longest == 0 {
        return format!("`{text}`");
    }
    let fence = "`".repeat(longest + 1);
    format!("{fence} {text} {fence}")
}

/// 代码块：围栏比内容里最长的反引号串更长
pub(crate) fn code_block(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}text\n{}\n{fence}", text.trim_end())
}

pub(crate) fn render_files(out: &mut String, title: &str, files: &[(String, usize)]) {
    out.push_str(&format!("### {title}\n"));
    if files.is_empty() {
        out.push_str("（无）\n\n");
        return;
    }
    for (path, idx) in files.iter().take(RENDER_FILE_LIMIT) {
        out.push_str(&format!("- {}（最后涉及 {}）\n", inline_code(path), msg_ref(*idx)));
    }
    if files.len() > RENDER_FILE_LIMIT {
        out.push_str(&format!("- …另有 {} 个\n", files.len() - RENDER_FILE_LIMIT));
    }
    out.push('\n');
}

/// 为了放进预算被省略的条目数
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Trimmed {
    pub(crate) read: usize,
    pub(crate) commands: usize,
    pub(crate) failures: usize,
    pub(crate) verify_lines: usize,
    pub(crate) modified: usize,
    pub(crate) timeline: usize,
}

impl Trimmed {
    pub(crate) fn note(&self) -> Option<String> {
        let parts: Vec<String> = [
            (self.timeline, "条用户原话"),
            (self.modified, "个改过的文件"),
            (self.verify_lines, "行验证输出"),
            (self.failures, "条失败记录"),
            (self.commands, "条命令"),
            (self.read, "个读过的文件"),
        ]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect();
        (!parts.is_empty()).then(|| format!("> 事实区超出约 {}k token，已省略较早的 {}。\n\n", FACTS_TOKEN_BUDGET / 1000, parts.join("、")))
    }
}

/// 按优先级裁剪到预算（用户原话 > 改过的文件 > 最后验证 > 失败 > 其余命令 > 读过的文件），先删优先级低的、较早的
pub(crate) fn fit_facts(f: &Facts) -> (Facts, Trimmed) {
    let mut f = f.clone();
    let mut t = Trimmed::default();
    while estimate_tokens(&render_body(&f, &t)) > FACTS_TOKEN_BUDGET {
        if !f.files_read.is_empty() {
            f.files_read.pop();
            t.read += 1;
        } else if !f.commands.is_empty() {
            // 先删非验证类里最早的，再删验证类里最早的
            let at = f.commands.iter().position(|c| !is_verify_command(&c.command)).unwrap_or(0);
            f.commands.remove(at);
            t.commands += 1;
        } else if !f.failures.is_empty() {
            f.failures.remove(0);
            t.failures += 1;
        } else if let Some(v) = f.last_verify.as_mut().filter(|v| v.tail.lines().count() > 1) {
            v.tail = v.tail.lines().skip(1).collect::<Vec<_>>().join("\n");
            t.verify_lines += 1;
        } else if !f.files_modified.is_empty() {
            f.files_modified.pop();
            t.modified += 1;
        } else if f.user_timeline.len() > 1 {
            f.user_timeline.remove(0);
            f.earlier_user_count += 1;
            t.timeline += 1;
        } else {
            break;
        }
    }
    (f, t)
}

/// 事实区 Markdown（交接文档与合成输入共用）；总量超过约 5k token 时按优先级裁剪
pub fn render_facts(f: &Facts) -> String {
    let (f, t) = fit_facts(f);
    render_body(&f, &t)
}

pub(crate) fn render_body(f: &Facts, trimmed: &Trimmed) -> String {
    let mut out = String::from("## 事实区\n\n> 会话事实由 Jolu 从会话记录中抽取，未经模型改写；#n 为会话消息序号（从 1 开始）。Git 快照与回查线索为程序补充，不属于会话原文。\n\n");
    if let Some(note) = trimmed.note() {
        out.push_str(&note);
    }

    out.push_str(&format!("### 用户原话（最近 {} 条）\n", f.user_timeline.len()));
    if f.earlier_user_count > 0 {
        out.push_str(&format!("更早还有 {} 条用户消息未列出。\n\n", f.earlier_user_count));
    }
    if f.user_timeline.is_empty() {
        out.push_str("（无）\n");
    }
    for (idx, text) in &f.user_timeline {
        // 会话来源的字段渲染时再遮一遍：已存的旧版本也能用上后来加强的规则（redact 可重复处理）
        out.push_str(&format!("- {}「{}」\n", msg_ref(*idx), redact(text).replace(['\r', '\n'], " ")));
    }
    out.push('\n');

    render_files(&mut out, "改过的文件", &f.files_modified);
    render_files(&mut out, "读过的文件", &f.files_read);

    out.push_str("### 执行过的命令\n");
    if f.commands.is_empty() {
        out.push_str("（无）\n");
    } else if f.commands_total > f.commands.len() {
        out.push_str(&format!("共 {} 条，列出测试、构建、检查类与最近的 {} 条。\n\n", f.commands_total, f.commands.len()));
    }
    for c in &f.commands {
        out.push_str(&format!("- {} → {}（{}）\n", inline_code(&redact(&c.command)), c.status_text(), msg_ref(c.idx)));
    }
    out.push('\n');

    out.push_str("### 最后一次验证\n");
    match &f.last_verify {
        None => out.push_str("（无）\n\n"),
        Some(v) => {
            out.push_str(&format!("{} → {}（{}）\n\n", inline_code(&redact(&v.command)), v.status_text(), msg_ref(v.idx)));
            if v.tail.trim().is_empty() {
                out.push_str("（没有记录输出）\n\n");
            } else {
                out.push_str(&format!("输出末尾：\n\n{}\n\n", code_block(&redact(&v.tail))));
            }
        }
    }

    out.push_str("### 失败的尝试\n");
    if f.failures.is_empty() {
        out.push_str("（无）\n");
    }
    for c in &f.failures {
        let line = if c.tail.is_empty() { String::new() } else { format!("：{}", redact(&c.tail)) };
        out.push_str(&format!("- {}（{} · {}）{line}\n", inline_code(&redact(&c.command)), msg_ref(c.idx), c.status_text()));
    }
    out.push('\n');

    out.push_str("### Git 快照\n");
    out.push_str("> 生成时读取的当前工作区补充，不代表本会话改动，也不构成用户要求；不能用 #n 引用为会话事实。\n\n");
    match &f.git {
        None => out.push_str("（工作目录不是 Git 仓库，或无法读取）\n\n"),
        Some(g) => {
            out.push_str(&format!("- 分支：{}\n", g.branch.as_deref().map(inline_code).unwrap_or_else(|| "（分离 HEAD）".into())));
            out.push_str(&format!("- HEAD：{}\n", g.head.as_deref().map(inline_code).unwrap_or_else(|| "（还没有提交）".into())));
            if g.dirty.is_empty() {
                out.push_str("- 未提交的文件：（无）\n\n");
            } else {
                out.push_str(&format!("- 未提交的文件（{} 项）：\n\n{}\n", g.dirty_total, code_block(&g.dirty.join("\n"))));
                if g.dirty_total > g.dirty.len() {
                    out.push_str(&format!("…另有 {} 项\n", g.dirty_total - g.dirty.len()));
                }
                out.push('\n');
            }
        }
    }

    out.push_str("### 回查线索\n");
    match &f.transcript_path {
        Some(p) => out.push_str(&format!("- 原始会话文件：{}\n", inline_code(p))),
        None => out.push_str("- 原始会话文件：（未知）\n"),
    }
    out.push_str("- 需要原文细节时，按消息序号 #n 查阅。\n");
    out
}

/// 对话弹窗第一条步骤消息，例如「改过 6 个文件 · 12 条命令 · 最后一次验证失败」
pub fn summary_line(f: &Facts) -> String {
    let verify = match f.last_verify.as_ref().map(CommandFact::is_failure) {
        None => "没有验证记录",
        Some(Some(true)) => "最后一次验证失败",
        Some(Some(false)) => "最后一次验证通过",
        Some(None) => "最后一次验证结果未知",
    };
    format!("改过 {} 个文件 · {} 条命令 · {verify}", f.files_modified.len(), f.commands_total.max(f.commands.len()))
}

#[cfg(test)]
mod tests;
