//! 每日 warm-up（批次 G §3.12）。
//!
//! 目的：订阅型来源（Claude Code / Codex 等）长时间不用会掉额度窗口或需要重新预热，
//! 每天固定时间发一条**最小请求**保活。
//!
//! 设计取舍：
//! - **默认关闭**：会真实消耗额度，必须用户显式打开，界面上写清「会消耗额度」。
//! - **走已接线的 agent**，不发独立的网关请求：复用自动化那套 `build_args` 与 CLI 路径，
//!   这样账号必须是原工具当前登录的账号这一约束天然成立，不会偷偷用别的凭据。
//! - 每天只发一次，用设置里的 `warmup_last_date` 记日期；跨天判断按**本地日期**。
//! - 只发一条极短提示词，不落任何会话文件正文（CLI 自己会记它的历史，Jolu 不额外存）。
//!
//! 本模块只负责「该不该发」的纯判断与进程拉起；调度循环在 `lib.rs` 的定时器里。

use serde::Serialize;

/// warm-up 用的提示词：极短、无副作用、不触发工具调用。
pub(crate) const WARMUP_PROMPT: &str = "Reply with just: ok";

/// 支持的平台：只做能非交互执行的（与自动化的执行器一致）。
pub(crate) const WARMUP_PLATFORMS: [&str; 2] = ["claude-code", "codex"];

#[derive(Debug, Clone, Serialize)]
pub struct WarmupSkip {
    /// 为什么这次不发：`disabled` / `already_today` / `not_due` / `unsupported_platform` / `no_account`
    pub reason: String,
    /// 给用户看的中文说明
    pub detail: String,
}

/// 判断此刻是否该发 warm-up。
///
/// 输入是设置里的四个字段与当前本地时间，输出是 `Ok(())`（该发）或带原因的跳过。
/// 抽成纯函数是为了能在测试里把「今天已发过」「还没到点」这些边界跑一遍。
pub(crate) fn should_fire(
    enabled: bool, hour: u32, last_date: Option<&str>, today: &str, now_hour: u32,
) -> Result<(), WarmupSkip> {
    if !enabled {
        return Err(WarmupSkip { reason: "disabled".into(), detail: "每日 warm-up 未开启".into() });
    }
    if last_date == Some(today) {
        return Err(WarmupSkip { reason: "already_today".into(), detail: "今天已经 warm-up 过".into() });
    }
    if now_hour < hour.min(23) {
        return Err(WarmupSkip {
            reason: "not_due".into(),
            detail: format!("还没到设定时间（{} 点）", hour.min(23)),
        });
    }
    Ok(())
}

/// 本地日期字符串 `YYYY-MM-DD`。
pub(crate) fn local_today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub(crate) fn local_hour() -> u32 {
    use chrono::Timelike;
    chrono::Local::now().hour()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_never_fires() {
        let skip = should_fire(false, 9, None, "2026-11-09", 23).unwrap_err();
        assert_eq!(skip.reason, "disabled");
    }

    #[test]
    fn fires_once_a_day_after_the_hour() {
        assert!(should_fire(true, 9, None, "2026-11-09", 9).is_ok());
        assert!(should_fire(true, 9, Some("2026-11-08"), "2026-11-09", 12).is_ok(), "昨天发过，今天可以再发");
        let skip = should_fire(true, 9, Some("2026-11-09"), "2026-11-09", 23).unwrap_err();
        assert_eq!(skip.reason, "already_today");
    }

    #[test]
    fn waits_until_the_configured_hour() {
        let skip = should_fire(true, 9, None, "2026-11-09", 8).unwrap_err();
        assert_eq!(skip.reason, "not_due");
        assert!(should_fire(true, 0, None, "2026-11-09", 0).is_ok(), "0 点整就该发");
    }

    #[test]
    fn hour_is_clamped_so_a_bad_value_still_fires_today() {
        // 用户把小时设成 99 时按 23 处理，不会永远等不到
        assert!(should_fire(true, 99, None, "2026-11-09", 23).is_ok());
        assert!(should_fire(true, 99, None, "2026-11-09", 22).is_err());
    }

    #[test]
    fn only_non_interactive_agents_are_supported() {
        assert!(WARMUP_PLATFORMS.contains(&"claude-code"));
        assert!(WARMUP_PLATFORMS.contains(&"codex"));
        for other in ["gemini", "opencode", "zcode", "qoder", "workbuddy", "trae"] {
            assert!(!WARMUP_PLATFORMS.contains(&other), "{other} 没有非交互执行器，不能做 warm-up");
        }
    }

    #[test]
    fn prompt_is_minimal_and_side_effect_free() {
        assert!(WARMUP_PROMPT.len() < 40, "warm-up 提示词要极短，别烧额度");
        assert!(!WARMUP_PROMPT.contains("edit"), "不能触发文件操作");
    }

    #[test]
    fn local_today_is_iso_date() {
        let today = local_today();
        assert_eq!(today.len(), 10, "YYYY-MM-DD");
        assert_eq!(today.matches('-').count(), 2);
    }
}
