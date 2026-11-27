#![allow(unused_imports)]
//! 会话线程领域模块：本机会话列表、各平台消息流只读解析与「在原工具继续」。

pub(crate) mod list;
pub(crate) mod parsers;
pub(crate) mod resume;
pub mod source;
#[cfg(test)]
mod tests;
pub mod types;

pub use list::list;
pub use parsers::{
    antigravity::parse_antigravity, claude::parse_claude, codex::parse_codex, messages,
};
pub(crate) use parsers::{
    parse_detail, read_lines, session_platform_allowed, source_path, EXTERNAL_SESSION_PLATFORMS,
};
pub use resume::{continue_check, continue_session, ContinueCheck};
pub(crate) use resume::{find_executable, safe_id};
pub(crate) use source::{read_cwd, source_files};
pub use types::{
    Message, MessagePage, SessionFilter, SessionItem, SessionList, TokenUsage, ToolCall,
    UNATTRIBUTED,
};

#[cfg(test)]
pub(crate) use list::{cached_antigravity_title, file_contains};
#[cfg(test)]
pub(crate) use parsers::{
    antigravity::{add_antigravity_tokens, append_clipped, parse_antigravity_with},
    attach_output,
    claude::parse_claude_with,
    claude_result, clip, clip_middle,
    codex::{codex_event_tool, parse_codex_with},
    codex_result, exit_line, is_injected, output_text, parse_file, parse_input, parse_ts,
    push_assistant, session_cost, tool_paths, tool_summary,
    zcode::{
        keep_recent_parts, open_zcode_db, zcode_add_tokens, zcode_child_session,
        zcode_child_sessions, zcode_db_path, zcode_load, zcode_load_with, zcode_message,
        zcode_page, zcode_page_with, zcode_raw_session_id, zcode_schema_ok, zcode_tokens,
        zcode_tool, ZcodeStream, ZCODE_BUSY_MS, ZCODE_PART_BYTES, ZCODE_PART_LIMIT,
    },
    LineSource, Limits, ToolResult, DETAIL, VIEW,
};
