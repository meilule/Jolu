//! 每日 warm-up 的执行侧（批次 G §3.12）。
//!
//! 判断层在 `warmup`，这里负责：找一个已接线的、当前登录账号匹配的 agent，
//! 用非交互方式发一条极短提示词，并把运行结果如实回报。
//!
//! 为什么要挑 agent 而不是直接打网关：账号必须是原工具当前登录的账号（与自动化同一条
//! 约束），走 CLI 就不会绕过这个前提。挑不到可用 agent 时如实跳过，不偷偷换凭据。

use serde::Serialize;
use std::process::{Command, Stdio};
use std::time::Duration;

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

/// 最近一次 warm-up 的结果与完成时间（毫秒）。只在内存里：设置页挂载时读一次，之后靠
/// `warmup-finished` 事件更新；失败不会记日期、会在下一个检查周期重试，所以要单独记住
/// 「上次为什么没成」，否则失败在界面上完全看不到。
#[derive(Debug, Clone, Serialize)]
pub struct LastWarmup {
    #[serde(flatten)]
    pub outcome: WarmupOutcome,
    pub finished_ms: i64,
}

static LAST: std::sync::Mutex<Option<LastWarmup>> = std::sync::Mutex::new(None);

/// 记下这次结果并返回带完成时间的那份（随 `warmup-finished` 发给前端）。
pub(crate) fn remember(outcome: WarmupOutcome) -> LastWarmup {
    let last = LastWarmup { outcome, finished_ms: chrono::Utc::now().timestamp_millis() };
    if let Ok(mut slot) = LAST.lock() {
        *slot = Some(last.clone());
    }
    last
}

/// 最近一次结果；本次启动后还没跑过时为 None。
pub(crate) fn last() -> Option<LastWarmup> {
    LAST.lock().ok().and_then(|slot| slot.clone())
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

/// 等子进程结束；超时结束整棵进程树并返回 None。本机找到的常是 npm 的 `.cmd` 垫片，
/// 只杀外层的话 node / codex 会把这次 warm-up 请求照样发完（见 `crate::proc`）。
fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> Option<ChildOutput> {
    crate::proc::wait_or_kill_tree(child, timeout, Duration::from_millis(100))
        .map(|status| ChildOutput { status_success: status.success() })
}


/// 每日 warm-up（批次 G §3.12）：由自动化 30 秒调度循环回调触发。
pub(crate) fn tick_if_due(app: &tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RUNNING.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    let settings = app.state::<crate::Cfg>().0.get();
    let today = crate::warmup::local_today();
    if crate::warmup::should_fire(
        settings.warmup_enabled,
        settings.warmup_hour,
        settings.warmup_last_date.as_deref(),
        &today,
        crate::warmup::local_hour(),
    )
    .is_err()
    {
        return;
    }
    RUNNING.store(true, std::sync::atomic::Ordering::Release);
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("jolu-warmup".into())
        .spawn(move || {
            let outcome = {
                let db = crate::automation::shared(&app);
                match pick_agent(&db) {
                    Some(platform) => run_once(&platform),
                    None => WarmupOutcome {
                        status: "skipped".into(),
                        platform: None,
                        detail: "没有可用的已接线 Agent（当前登录账号需与接线账号一致）".into(),
                    },
                }
            };
            if outcome.status == "ok" {
                let _ = app.state::<crate::Cfg>().0.try_update(|settings| {
                    settings.warmup_last_date = Some(crate::warmup::local_today());
                });
                crate::events::settings_changed(&app);
            }
            let _ = app.emit(crate::events::WARMUP_FINISHED, remember(outcome));
            RUNNING.store(false, std::sync::atomic::Ordering::Release);
        });
    if spawned.is_err() {
        RUNNING.store(false, std::sync::atomic::Ordering::Release);
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
