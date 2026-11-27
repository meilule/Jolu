#![allow(unused_imports)]
//! 自动化（Jolu.pen 模块 E，画板 58–73）：任务、运行记录与本机调度。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Local;
use rusqlite::Connection;

pub(crate) mod queries;
pub(crate) mod runner;
pub(crate) mod schedule;
pub(crate) mod scheduler;
pub(crate) mod schema;
pub(crate) mod store;
#[cfg(test)]
mod tests;

pub use queries::{Account, Overview, QueueItem, RunDetail, RunFilter, RunPage, TaskDetail, TaskItem, ToolInfo};
pub(crate) use queries::{accounts_for, models_for, overview, run_detail, runs, task_detail, tools_list};
pub use runner::{
    build_args, classify, codex_retry_at, is_auth, is_balance, is_limit, is_rate_limit,
    is_untrusted_dir, limit_reset, missing_skills, run_prompt, skill_prefix, skills_ready, Outcome,
    Progress, RATE_LIMIT_RETRY_MS,
};
pub use schedule::{apply_check, next_fire, parse_reset, verdict, QuotaVerdict, WindowInfo};
pub use scheduler::start;
pub(crate) use scheduler::{
    cmd_action, cmd_create, cmd_delete, cmd_overview, cmd_pick_dir, cmd_quota, cmd_run, cmd_runs,
    cmd_skills, cmd_task, cmd_update, cmd_waiting_connections, current_connection_ids,
    register_quota_querier, register_tick_hook, shared,
};
pub use schema::ensure_schema;
pub use store::{
    act, apply_account_check, claim_run, create_task, delete_task, due_checks, finish,
    fire_schedules, notice_for, notices_of, recover, startable, take_resume_confirmed, update_task,
    validate, waiting_connections, Event, PendingNotice, Run, Schedule, Task, TaskInput,
};
#[cfg(test)]
pub(crate) use queries::queue_ahead;
#[cfg(test)]
pub(crate) use runner::{kill_tree, missing_reason, Procs};
#[cfg(test)]
pub(crate) use schedule::backoff;
#[cfg(test)]
pub(crate) use scheduler::{emit, kick, kill_run, with_conn};
#[cfg(test)]
pub(crate) use store::{
    active_run, add_event, set_attention, events_of, insert_run, last_run, load_run, load_task, safe_model,
    waiting_runs,
};

pub(crate) type Shared = Arc<Mutex<Connection>>;

/// 前端订阅的变化事件：任务或运行状态变化后发出
pub const CHANGED: &str = crate::events::AUTOMATION_CHANGED;
pub(crate) const TICK: Duration = Duration::from_secs(30);
pub(crate) const MISSED_GRACE_MS: i64 = 10 * 60_000;
pub(crate) const PAGE: i64 = 6;
pub(crate) const LOG_LIMIT: i64 = 200;
pub(crate) const RESUME_PROMPT: &str =
    "额度已恢复。请从中断处继续完成上一条任务，不要重复已经完成的修改；完成后汇总改动与验证结果。";
pub(crate) const ACTIVE: &str = "('queued','waiting','running','attention')";
pub(crate) const MISSING_SNAPSHOT: &str =
    "此运行没有原始执行配置快照，无法确认工具、账号、目录与指令；已保留历史和会话，请取消旧运行并核对任务配置后重新创建任务，不会按当前配置自动续跑。";

pub(crate) fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

pub(crate) fn clip(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    format!("{}…", text.chars().take(limit).collect::<String>())
}

pub fn tool_name(platform: &str) -> Option<&'static str> {
    match platform {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        _ => None,
    }
}

pub(crate) fn display_name(platform: &str) -> &'static str {
    match platform {
        "codex" => "Codex",
        "claude" => "Claude Code",
        _ => "未知工具",
    }
}
