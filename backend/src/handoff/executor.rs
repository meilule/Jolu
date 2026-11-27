use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value as Json;

use super::pipeline::{platform_name, Usage};

/* ─────────────────────────── 执行器 ─────────────────────────── */

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// 来源没有给出（Codex `exec --json` 不报告停止原因）
    #[default]
    Unknown,
    EndTurn,
    MaxTokens,
    Refusal,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExecOutput {
    pub text: String,
    pub stop: StopReason,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub context_exceeded: bool,
    /// 执行过程中模型调用了工具（Claude `tool_use`、Codex 命令 / 文件 / MCP 事件）；视同质量检查不通过
    pub used_tools: bool,
}

pub trait Executor {
    fn run(&self, prompt: &str) -> Result<ExecOutput, String>;
}

/// D-01 的命令行参数：非交互、不落会话、不续接原会话；绝不带 `--resume`。提示词一律走标准输入。
/// Claude 另加 `--tools ""` 禁用全部工具、`--strict-mcp-config` 不加载 MCP（关闭 hooks 的 `--settings` 在 [`run_args`] 里按次加）；
/// Codex 保持 `--sandbox read-only`，已配置的 MCP 在 [`run_args`] 里逐个关闭
pub fn executor_args(platform: &str, model: Option<&str>) -> Result<Vec<String>, String> {
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    if let Some(m) = model {
        let safe = m.len() <= 128 && !m.starts_with('-') && m.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/' | '[' | ']' | '@'));
        if !safe {
            return Err("模型名称不合法".into());
        }
    }
    let mut args: Vec<String> = match platform {
        // `--tools ""`：Claude Code 2.1.309 `--help` 原文「Use "" to disable all tools」，彻底不提供任何内置工具
        // `--strict-mcp-config`（不带 `--mcp-config`）：「Only use MCP servers from --mcp-config, ignoring all other MCP configurations」，即不加载任何 MCP
        "claude" => ["-p", "--no-session-persistence", "--tools", "", "--strict-mcp-config", "--output-format", "stream-json", "--verbose"].map(String::from).to_vec(),
        "codex" => ["exec", "--json", "--ephemeral", "--sandbox", "read-only", "--skip-git-repo-check"].map(String::from).to_vec(),
        _ => return Err("压缩只能使用 Claude Code 或 Codex 命令行当前登录的账号".into()),
    };
    if let Some(m) = model {
        args.push(if platform == "codex" { "-m".into() } else { "--model".into() });
        args.push(m.to_string());
    }
    if platform == "codex" {
        args.push("-".into());
    }
    Ok(args)
}

/// D-34 的命令行参数：分叉续接原会话、不落盘。原会话文件一字不改，也不会多出新会话：
/// - Claude Code：`--resume <id> --fork-session`（官方文档：分叉把历史复制到新会话 id，原会话不变）+
///   `--no-session-persistence`（分叉出的会话也不写盘）；同样禁用全部工具与 MCP；
/// - Codex：`exec fork <id> -`（0.169 `codex exec fork --help`）+ `--ephemeral`（不落盘）；
///   `fork` 没有 `--sandbox` 参数，用 `-c sandbox_mode="read-only"`。
/// 提示词一律走标准输入（`-` / Claude 的 `-p` 读标准输入）
pub fn fork_args(platform: &str, session_id: &str, model: Option<&str>) -> Result<Vec<String>, String> {
    let id = session_id.trim();
    let safe_id = !id.is_empty() && id.len() <= 128 && !id.starts_with('-') && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    if !safe_id {
        return Err("会话标识不合法，无法在原工具里分叉".into());
    }
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    // 模型名的合法性检查与普通压缩一致
    executor_args(platform, model)?;
    let mut args: Vec<String> = match platform {
        "claude" => ["-p", "--resume", id, "--fork-session", "--no-session-persistence", "--tools", "", "--strict-mcp-config", "--output-format", "stream-json", "--verbose"]
            .map(String::from)
            .to_vec(),
        "codex" => ["exec", "fork", "--json", "--ephemeral", "--skip-git-repo-check", "-c", "sandbox_mode=\"read-only\""].map(String::from).to_vec(),
        _ => return Err("只有 Claude Code 与 Codex 的会话能在原工具里分叉压缩".into()),
    };
    if let Some(m) = model {
        args.push(if platform == "codex" { "-m".into() } else { "--model".into() });
        args.push(m.to_string());
    }
    if platform == "codex" {
        args.push(id.to_string());
        args.push("-".into());
    }
    Ok(args)
}


/* ─────────────────────────── 命令行执行器 ─────────────────────────── */

/// 本次对话累计的调用次数与 token（任意一次没有报告时记为未知）
#[derive(Debug, Default)]
pub(crate) struct Totals {
    pub(crate) calls: AtomicU64,
    pub(crate) usage: Mutex<Usage>,
}

pub(crate) struct CliExecutor {
    pub(crate) platform: String,
    pub(crate) exe: PathBuf,
    pub(crate) args: Vec<String>,
    /// 工作目录：缺省为空的临时目录；在原工具里分叉时用原会话的项目目录（Claude 按项目目录找会话）
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) pid: Arc<Mutex<Option<u32>>>,
    pub(crate) totals: Arc<Totals>,
}

#[cfg(all(debug_assertions, feature = "handoff-eval"))]
pub(crate) fn eval_executor(platform: &str) -> Result<impl Executor, String> {
    let exe = crate::session_threads::find_executable(platform).ok_or("未找到所选 CLI")?;
    Ok(CliExecutor { platform: platform.into(), exe, args: executor_args(platform, None)?, cwd: None, cancel: Arc::new(AtomicBool::new(false)), pid: Arc::new(Mutex::new(None)), totals: Arc::new(Totals::default()) })
}

pub(crate) fn map_stop(s: &str) -> StopReason {
    match s {
        "end_turn" | "stop_sequence" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "refusal" => StopReason::Refusal,
        _ => StopReason::Unknown,
    }
}

/// 停止原因与工具调用：Claude `assistant.message.stop_reason`、`result.stop_reason`、`tool_use` 块；
/// Codex `exec --json` 的命令 / 文件 / MCP / 搜索事件
pub(crate) fn scan_stream(platform: &str, line: &str, stop: &mut StopReason, used_tools: &mut bool) {
    let Ok(v) = serde_json::from_str::<Json>(line.trim()) else { return };
    let kind = v.get("type").and_then(Json::as_str).unwrap_or("");
    if platform == "claude" {
        match kind {
            "assistant" => {
                let blocks = v.pointer("/message/content").and_then(Json::as_array);
                if blocks.is_some_and(|b| b.iter().any(|b| b.get("type").and_then(Json::as_str) == Some("tool_use"))) {
                    *used_tools = true;
                }
                if let Some(s) = v.pointer("/message/stop_reason").and_then(Json::as_str) {
                    *stop = map_stop(s);
                }
            }
            "result" => {
                if let Some(s) = v.get("stop_reason").and_then(Json::as_str) {
                    *stop = map_stop(s);
                }
            }
            _ => {}
        }
    } else if matches!(kind, "item.started" | "item.completed") {
        let item = v.get("item").and_then(|i| i.get("type")).and_then(Json::as_str).unwrap_or("");
        if matches!(item, "command_execution" | "file_change" | "mcp_tool_call" | "web_search") {
            *used_tools = true;
        }
    }
}

/// 模型上下文超限的报错（Claude `Prompt is too long`；Codex `context_length_exceeded` 等）。
/// 不匹配 `too many tokens` 之类的宽泛说法，避免把限流（tokens per minute）误判为上下文超限
pub(crate) fn is_context_exceeded(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["prompt is too long", "context_length_exceeded", "maximum context length", "exceeds the context window", "exceeds the context", "input is too long for"].iter().any(|w| lower.contains(w))
}

/// Codex 已配置的 MCP 服务逐个关闭：`-c mcp_servers.<名称>.enabled=false`。
/// Codex 的配置层对表逐键深合并，`-c mcp_servers={}` 清不掉已有服务；`enabled = false` 会让该服务跳过初始化
/// （openai/codex `config/src/mcp_types.rs`）。只读 `config.toml` 的键名，不读取服务的命令、环境变量等内容；
/// 名称含点号或特殊字符时无法安全写成点分路径，跳过
pub(crate) fn codex_mcp_disable_args(config_toml: &str) -> Vec<String> {
    let Ok(table) = config_toml.parse::<toml::Table>() else { return Vec::new() };
    let Some(servers) = table.get("mcp_servers").and_then(toml::Value::as_table) else { return Vec::new() };
    servers
        .keys()
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')))
        .flat_map(|name| ["-c".to_string(), format!("mcp_servers.{name}.enabled=false")])
        .collect()
}

/// Codex 的 MCP 隔离是否完全：config.toml 解析失败、名称无法安全写成点分路径而被跳过、配置了插件（插件自带的 MCP 关不掉）时，
/// 返回给对话弹窗的提示；没有配置文件时视为没有 MCP
pub(crate) fn codex_mcp_audit(config_toml: Option<&str>) -> Option<String> {
    let text = config_toml?;
    let Ok(table) = text.parse::<toml::Table>() else {
        return Some("未能确认关闭全部 MCP 服务：config.toml 无法解析，已照常继续".into());
    };
    let mut reasons = Vec::new();
    if let Some(servers) = table.get("mcp_servers").and_then(toml::Value::as_table) {
        let skipped: Vec<&str> = servers.keys().map(String::as_str).filter(|name| name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))).collect();
        if !skipped.is_empty() {
            reasons.push(format!("名称含特殊字符的服务（{}）无法关闭", skipped.join("、")));
        }
    }
    if table.contains_key("plugins") || table.contains_key("plugin") {
        reasons.push("插件自带的 MCP 服务无法关闭".to_string());
    }
    (!reasons.is_empty()).then(|| format!("未能确认关闭全部 MCP 服务：{}，已照常继续", reasons.join("；")))
}

pub(crate) fn codex_config_text() -> Option<String> {
    let dir = std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| crate::creds::home().map(|h| h.join(".codex")))?;
    std::fs::read_to_string(dir.join("config.toml")).ok()
}

/// 每次运行的隔离参数：Claude 用单独的设置文件关闭全部 hooks（`--settings <文件>`，内容 `{"disableAllHooks": true}`）；
/// Codex 关闭已配置的 MCP 服务。Codex 的附加参数插在末尾的 `-`（从标准输入读提示词）之前
pub(crate) fn run_args(platform: &str, base: &[String], settings_file: Option<&Path>, codex_config: Option<&str>) -> Vec<String> {
    let mut args = base.to_vec();
    match platform {
        "claude" => {
            if let Some(file) = settings_file {
                args.push("--settings".into());
                args.push(file.display().to_string());
            }
        }
        "codex" => {
            let extra = codex_config.map(codex_mcp_disable_args).unwrap_or_default();
            let stdin_last = args.last().map(String::as_str) == Some("-");
            // `exec fork … <会话 id> -`：额外参数要放在会话 id 之前
            let forked = args.get(1).map(String::as_str) == Some("fork");
            let at = match (stdin_last, forked) {
                (true, true) => args.len().saturating_sub(2),
                (true, false) => args.len() - 1,
                _ => args.len(),
            };
            args.splice(at..at, extra);
        }
        _ => {}
    }
    args
}

pub(crate) const CLAUDE_ISOLATION_SETTINGS: &str = r#"{"disableAllHooks": true}"#;

/// 记录执行器进程；写入后再看一次取消标志：`stop()` 若在写入前已置位，它读不到 pid，这里返回 true 由调用方立即结束进程
pub(crate) fn register_pid(slot: &Mutex<Option<u32>>, cancel: &AtomicBool, pid: u32) -> bool {
    *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(pid);
    cancel.load(Ordering::Acquire)
}

/// 结束整个进程树（执行器会再拉起子进程），见 `crate::proc`
pub(crate) fn kill_tree(pid: u32) {
    crate::proc::kill_tree(pid);
}

impl Executor for CliExecutor {
    fn run(&self, prompt: &str) -> Result<ExecOutput, String> {
        if self.cancel.load(Ordering::Acquire) {
            return Err("已取消压缩".into());
        }
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("jolu-handoff-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败：{e}"))?;
        // 设置文件放在工作目录旁边，工作目录保持为空
        let settings = (self.platform == "claude").then(|| dir.with_extension("settings.json"));
        if let Some(file) = &settings {
            if let Err(e) = std::fs::write(file, CLAUDE_ISOLATION_SETTINGS) {
                // 写了一半的设置文件也删掉
                let _ = std::fs::remove_file(file);
                let _ = std::fs::remove_dir_all(&dir);
                return Err(format!("写入临时设置失败：{e}"));
            }
        }
        let cleanup = || {
            let _ = std::fs::remove_dir_all(&dir);
            if let Some(file) = &settings {
                let _ = std::fs::remove_file(file);
            }
        };
        let codex_config = (self.platform == "codex").then(codex_config_text).flatten();
        let args = run_args(&self.platform, &self.args, settings.as_deref(), codex_config.as_deref());
        let mut command = Command::new(&self.exe);
        command.args(&args).current_dir(self.cwd.as_deref().unwrap_or(&dir)).env_remove("CLAUDECODE").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                cleanup();
                return Err(format!("启动 {} 失败：{e}", platform_name(&self.platform)));
            }
        };
        if register_pid(&self.pid, &self.cancel, child.id()) {
            kill_tree(child.id());
        }
        let writer = child.stdin.take().map(|mut stdin| {
            let prompt = prompt.to_string();
            std::thread::spawn(move || {
                let _ = stdin.write_all(prompt.as_bytes());
            })
        });
        let stderr = child.stderr.take().map(|mut err| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = err.read_to_string(&mut text);
                let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
                lines[lines.len().saturating_sub(8)..].join("\n")
            })
        });
        let mut progress = crate::automation::Progress::default();
        let mut stop = StopReason::Unknown;
        let mut used_tools = false;
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = progress.feed(&self.platform, &line);
                scan_stream(&self.platform, &line, &mut stop, &mut used_tools);
            }
        }
        let exit_ok = child.wait().map(|s| s.success()).unwrap_or(false);
        if let Some(w) = writer {
            let _ = w.join();
        }
        let stderr_tail = stderr.and_then(|h| h.join().ok()).unwrap_or_default();
        *self.pid.lock().unwrap_or_else(|p| p.into_inner()) = None;
        cleanup();
        let stopped = self.cancel.load(Ordering::Acquire);

        self.totals.calls.fetch_add(1, Ordering::AcqRel);
        let output = ExecOutput {
            text: progress.result.clone().unwrap_or_default(),
            stop,
            input_tokens: progress.input.map(|n| n.max(0) as u64),
            output_tokens: progress.output.map(|n| n.max(0) as u64),
            context_exceeded: false,
            used_tools,
        };
        self.totals.usage.lock().unwrap_or_else(|p| p.into_inner()).add(&output);
        use crate::automation::Outcome;
        match crate::automation::classify(&progress, exit_ok, &stderr_tail, stopped) {
            Outcome::Completed => Ok(output),
            Outcome::Cancelled => Err("已取消压缩".into()),
            Outcome::Limit { message, .. } | Outcome::RateLimited { message, .. } | Outcome::Balance(message) | Outcome::Failed(message) | Outcome::Reauth(message) | Outcome::UntrustedDir(message) if is_context_exceeded(&message) => {
                Ok(ExecOutput { context_exceeded: true, ..output })
            }
            Outcome::Limit { message, .. } => Err(format!("额度不足：{}", clip(&message, 300))),
            Outcome::RateLimited { message, .. } => Err(format!("短时限流，请稍后再试：{}", clip(&message, 300))),
            Outcome::Balance(message) => Err(format!("余额不足：{}", clip(&message, 300))),
            Outcome::Reauth(message) => Err(format!("需要重新登录：{}", clip(&message, 300))),
            Outcome::UntrustedDir(message) | Outcome::Failed(message) => Err(clip(&message, 500)),
        }
    }
}

pub(crate) fn clip(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    format!("{}…", text.chars().take(limit).collect::<String>())
}
