//! 每日 warm-up 的执行侧（批次 G §3.12）。
//!
//! 判断层在 `warmup`，这里负责：找一个已接线的、当前登录账号匹配的 agent，
//! 用非交互方式发一条极短提示词，并把运行结果如实回报。
//!
//! 为什么要挑 agent 而不是直接打网关：账号必须是原工具当前登录的账号（与自动化同一条
//! 约束），走 CLI 就不会绕过这个前提。挑不到可用 agent 时如实跳过，不偷偷换凭据。

use serde::Serialize;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// warm-up 单次执行超时：只是发一条「回 ok」，超过 90 秒没有结果就不再等。
const RUN_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Serialize)]
pub struct WarmupOutcome {
    /// `ok` / `skipped` / `failed`
    pub status: String,
    /// 实际执行用的 agent（`claude-code` / `codex`），跳过时为 None
    pub platform: Option<String>,
    /// 结果说明，直接给用户看
    pub detail: String,
}

/// 在已接线的 agent 里挑一个当前账号匹配的。
///
/// 顺序固定（claude-code → codex），保证行为可预测；用户想换就断开前一个的接线。
pub(crate) fn pick_agent(db: &crate::automation::Shared) -> Option<String> {
    let current = crate::automation::current_connection_ids(db);
    if current.is_empty() { return None; }
    crate::warmup::WARMUP_PLATFORMS
        .into_iter()
        .find(|platform| {
            let tool = match *platform { "claude-code" => "claude", _ => "codex" };
            crate::session_threads::find_executable(tool).is_some()
        })
        .map(str::to_string)
}

/// 真正拉起一次非交互执行。stdout 不落任何统计（warm-up 不该算进用量）。
pub(crate) fn run_once(platform: &str) -> WarmupOutcome {
    let (tool, display) = match platform {
        "claude-code" => ("claude", "Claude Code"),
        "codex" => ("codex", "Codex"),
        other => {
            return WarmupOutcome {
                status: "skipped".into(),
                platform: None,
                detail: format!("{other} 没有非交互执行器，不能做 warm-up"),
            }
        }
    };
    let Some(exe) = crate::session_threads::find_executable(tool) else {
        return WarmupOutcome {
            status: "skipped".into(),
            platform: Some(platform.to_string()),
            detail: format!("未检测到 {tool} 命令，请安装或配置后重试"),
        };
    };
    // 用当前工作目录（用户主目录）作为 cwd：warm-up 不属于任何项目
    let cwd = crate::creds::home().unwrap_or_else(|| std::path::PathBuf::from("."));
    let args = crate::automation::build_args(platform, None, None);
    let mut command = Command::new(&exe);
    command
        .args(&args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return WarmupOutcome {
                status: "failed".into(),
                platform: Some(platform.to_string()),
                detail: format!("启动 {display} 失败：{error}"),
            }
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(crate::warmup::WARMUP_PROMPT.as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    match wait_with_timeout(&mut child, RUN_TIMEOUT) {
        Some(output) => {
            if output.status_success {
                WarmupOutcome {
                    status: "ok".into(),
                    platform: Some(platform.to_string()),
                    detail: format!("已通过 {display} 完成今日 warm-up"),
                }
            } else {
                WarmupOutcome {
                    status: "failed".into(),
                    platform: Some(platform.to_string()),
                    detail: format!("{display} 返回非零退出码，今日 warm-up 未完成"),
                }
            }
        }
        None => WarmupOutcome {
            status: "failed".into(),
            platform: Some(platform.to_string()),
            detail: format!("{display} 超时未结束，已终止本次 warm-up"),
        },
    }
}

struct ChildOutput {
    status_success: bool,
}

/// 等子进程结束；超时杀掉并返回 None。
fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> Option<ChildOutput> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(ChildOutput { status_success: status.success() }),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_platform_is_reported_not_silently_ok() {
        let outcome = run_once("gemini");
        assert_eq!(outcome.status, "skipped");
        assert_eq!(outcome.platform, None);
        assert!(outcome.detail.contains("没有非交互执行器"));
    }

    #[test]
    fn pick_agent_prefers_claude_then_codex_and_never_others() {
        // 无数据库时拿不到「当前账号」，直接返回 None，不硬挑一个
        let db: crate::automation::Shared = std::sync::Arc::new(std::sync::Mutex::new(
            rusqlite::Connection::open_in_memory().unwrap(),
        ));
        let picked = pick_agent(&db);
        assert!(picked.is_none() || picked.as_deref() == Some("claude-code") || picked.as_deref() == Some("codex"),
            "只会挑 claude-code 或 codex，实际 {picked:?}");
    }
}
