use std::collections::VecDeque;
use serde_json::Value as Json;
use crate::sessions::types::{Message, TokenUsage, ToolCall};
use super::{clip, exit_line, tool_paths, tool_summary, Limits, VIEW};

pub fn parse_antigravity<I: IntoIterator<Item = String>>(lines: I) -> Vec<Message> {
    parse_antigravity_with(lines, VIEW)
}

pub(crate) fn append_clipped(slot: &mut String, piece: &str, limit: usize) {
    let trimmed = piece.trim();
    if trimmed.is_empty() {
        return;
    }
    if slot.is_empty() {
        *slot = clip(trimmed, limit);
        return;
    }
    let existing = slot.chars().count();
    if existing >= limit {
        return;
    }
    let remaining = limit.saturating_sub(existing + 2);
    if remaining == 0 {
        return;
    }
    slot.push_str("\n\n");
    slot.push_str(&clip(trimmed, remaining));
}

pub(crate) fn add_antigravity_tokens(slot: &mut Option<TokenUsage>, input: Option<i64>, output: Option<i64>, cache_read: Option<i64>) {
    if input.is_none() && output.is_none() && cache_read.is_none() {
        return;
    }
    let usage = slot.get_or_insert_with(TokenUsage::default);
    let mut step_total = 0i64;
    if let Some(v) = input {
        usage.input = Some(usage.input.unwrap_or(0) + v);
        step_total += v;
    }
    if let Some(v) = output {
        usage.output = Some(usage.output.unwrap_or(0) + v);
        step_total += v;
    }
    if let Some(v) = cache_read {
        usage.cache_read = Some(usage.cache_read.unwrap_or(0) + v);
        step_total += v;
    }
    usage.total = Some(usage.total.unwrap_or(0) + step_total);
}

pub(crate) fn parse_antigravity_with<I: IntoIterator<Item = String>>(lines: I, limits: Limits) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    // PLANNER_RESPONSE 中的 tool_calls 与紧随其后的 GENERIC 步骤按顺序一一配对
    let mut pending_tools: VecDeque<(usize, usize)> = VecDeque::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Json>(trimmed) else { continue };
        let kind = v.get("type").and_then(Json::as_str).unwrap_or("");
        let ts = v
            .get("created_at")
            .and_then(Json::as_str)
            .and_then(crate::antigravity_usage::iso_to_ms);
        match kind {
            "USER_INPUT" => {
                pending_tools.clear();
                let extracted = v
                    .get("content")
                    .and_then(Json::as_str)
                    .and_then(crate::antigravity_usage::extract_user_request_text);
                let media_count = v.get("media").and_then(Json::as_array).map(Vec::len).unwrap_or(0);
                let text = match (extracted, media_count) {
                    (Some(body), n) if n > 0 => format!("{body}\n\n[附带 {n} 个附件]"),
                    (Some(body), _) => body,
                    (None, n) if n > 0 => format!("[附带 {n} 个附件]"),
                    (None, _) => continue,
                };
                out.push(Message {
                    role: "user".into(),
                    ts,
                    text: clip(&text, limits.text),
                    tools: Vec::new(),
                    ..Message::default()
                });
            }
            "PLANNER_RESPONSE" => {
                let content = v
                    .get("content")
                    .and_then(Json::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let thinking = v
                    .get("thinking")
                    .and_then(Json::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let model = v
                    .get("model")
                    .and_then(Json::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let calls = v.get("tool_calls").and_then(Json::as_array);
                let has_tools = calls.is_some_and(|arr| !arr.is_empty());
                if content.is_none() && thinking.is_none() && !has_tools {
                    continue;
                }
                if !out.last().is_some_and(|m| m.role == "assistant") {
                    out.push(Message {
                        role: "assistant".into(),
                        ts,
                        text: String::new(),
                        tools: Vec::new(),
                        ..Message::default()
                    });
                }
                let msg_idx = out.len() - 1;
                let last = &mut out[msg_idx];
                if let Some(body) = content {
                    append_clipped(&mut last.text, body, limits.text);
                }
                if let Some(thought) = thinking {
                    let slot = last.reasoning.get_or_insert_with(String::new);
                    append_clipped(slot, thought, limits.text);
                }
                if let Some(m) = model {
                    last.model = Some(m.to_string());
                }
                let num = |k: &str| v.get(k).and_then(Json::as_i64).filter(|n| *n >= 0);
                add_antigravity_tokens(&mut last.tokens, num("input_tokens"), num("output_tokens"), num("cache_read_tokens"));
                if ts.is_some() {
                    last.ts = ts;
                }
                if let Some(arr) = calls {
                    for tc in arr {
                        let args = tc.get("args").cloned().unwrap_or(Json::Null);
                        let tool = ToolCall {
                            name: tc.get("name").and_then(Json::as_str).unwrap_or("工具").to_string(),
                            summary: tool_summary(&args, limits),
                            paths: tool_paths(&args),
                            ..ToolCall::default()
                        };
                        let tool_idx = last.tools.len();
                        last.tools.push(tool);
                        pending_tools.push_back((msg_idx, tool_idx));
                    }
                }
            }
            "GENERIC" => {
                let Some((m, t)) = pending_tools.pop_front() else { continue };
                let Some(tool) = out.get_mut(m).and_then(|msg| msg.tools.get_mut(t)) else { continue };
                let content = v
                    .get("content")
                    .and_then(Json::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let error = v
                    .get("error")
                    .and_then(Json::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let raw_out = content.or(error);
                let status = v.get("status").and_then(Json::as_str).unwrap_or("");
                let failed = if error.is_some() || status.eq_ignore_ascii_case("ERROR") || status.eq_ignore_ascii_case("FAILED") {
                    Some(true)
                } else if status.eq_ignore_ascii_case("DONE") {
                    Some(false)
                } else {
                    None
                };
                let exit_code = raw_out
                    .and_then(|txt| exit_line(txt, &["Command exited with code", "Exit code", "Process exited with code"]));
                tool.output = raw_out.map(|txt| limits.output(txt));
                tool.failed = failed;
                tool.exit_code = exit_code;
            }
            _ => {}
        }
    }
    out.retain(|m| !(m.text.is_empty() && m.tools.is_empty() && m.reasoning.is_none()));
    out
}
