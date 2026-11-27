#![allow(unused_imports)]
//! 会话交接（计划 3.3「分段纪要 + 合成」、3.4 压缩对话弹窗）。

pub(crate) mod baseline;
pub(crate) mod digest;
#[cfg(all(debug_assertions, feature = "handoff-eval"))]
pub mod eval;
pub(crate) mod executor;
pub(crate) mod facts;
pub(crate) mod input;
pub(crate) mod pipeline;
pub(crate) mod prompts;
pub(crate) mod runner;
pub(crate) mod store;
#[cfg(test)]
mod tests;

pub use executor::{executor_args, fork_args, ExecOutput, Executor, StopReason};
pub(crate) use executor::{codex_mcp_audit, codex_mcp_disable_args, run_args, CliExecutor, Totals};
#[cfg(all(debug_assertions, feature = "handoff-eval"))]
pub(crate) use executor::eval_executor;
pub use pipeline::{
    check_file_name, check_output, covered_range, generate, generate_forked, refine,
    render_local_export, render_markdown, suggested_file_name, unknown_paths, ChatEvent, Draft,
    GenerateRequest, PreviousHandoff, RenderMeta,
};
pub(crate) use pipeline::{check_digest, check_refs, estimate_generate, snapshot_hash, step};
pub use prompts::{
    DIGEST_PROMPT, FORK_PROMPT, NO_TOOLS_PREAMBLE, NO_TOOLS_REMINDER, REFINE_PROMPT,
    ROLLUP_PROMPT, SYNTH_PROMPT,
};
pub(crate) use prompts::{digest_prompt, fork_prompt, refine_prompt, rollup_prompt, synth_prompt};
pub use runner::{available_name, shutdown, FileNameCheck, HandoffResult, HandoffState};
pub(crate) use runner::{
    cmd_cancel, cmd_estimate_refine, cmd_finish, cmd_local_export, cmd_refine, cmd_render,
    cmd_save, cmd_start, cmd_state, file_name_check,
};
pub use store::{ensure_schema, latest_handoff, load_digests, save_digest, HandoffMeta, HandoffRecord};
pub(crate) use store::{insert_handoff, NewHandoff};

#[cfg(test)]
pub(crate) use executor::{is_context_exceeded, map_stop, register_pid, scan_stream, CLAUDE_ISOLATION_SETTINGS};
#[cfg(test)]
pub(crate) use pipeline::{
    covered_text, first_next_step, format_ms, has_ref, has_tool_call, native_row, next_step_refs,
    normalize_path, path_candidates, platform_name, quote_block, ranges_text, split_sections,
    summary_body, unknown_in, user_numbers, Attempt, Usage, LOCAL_CLIP, LOCAL_RECENT, TOOL_MARKERS,
};
#[cfg(test)]
pub(crate) use prompts::{digest_label, digests_block, fill, focus_text, target_text};
#[cfg(test)]
pub(crate) use runner::{finish_job, new_range, with_conn, FinishData, Job, JOB};

pub const INPUT_BUDGET: usize = 167_000;
pub const RECENT_RAW_TOKENS: usize = 20_000;
pub const DIGEST_SEGMENT_TOKENS: usize = 40_000;
pub const DIGEST_ROLLUP_TOKENS: usize = 60_000;
pub const MAX_REFINE_ROUNDS: u32 = 10;
pub const PROMPT_VERSION: &str = "handoff-v2";
pub const CHAT_EVENT: &str = crate::events::HANDOFF_CHAT;
pub(crate) const NATIVE_NOTE: &str = "来自原工具压缩摘要，未经 Jolu 复核";
pub(crate) const DIGEST_OUTPUT_ESTIMATE: usize = 500;

pub const SECTIONS: [&str; 8] = [
    "交接目标",
    "当前状态与卡点",
    "下一步",
    "目标与约束",
    "已完成",
    "关键决策与原因",
    "失败的尝试",
    "待确认",
];
