use std::collections::HashMap;
use serde_json::Value as Json;
use crate::sessions::types::{Message, ToolCall};
use super::{
    attach_output, clip, codex_result, is_injected, output_text, parse_ts, push_assistant,
    tool_paths, tool_summary, Limits, VIEW,
};

pub fn parse_codex<I: IntoIterator<Item = String>>(lines: I) -> Vec<Message> {
    parse_codex_with(lines, VIEW)
}

/// Codex 新版把真实命令记在 `event_msg` → `item_completed`（`CommandExecution`：`command` 数组、`exit_code`、`status`），
/// 文件修改记在 `FileChange.changes`（以路径为键）。只有交接的详细解析读取，会话页显示保持不变
pub(crate) fn codex_event_tool(item: &Json, limits: Limits) -> Option<ToolCall> {
    match item.get("type").and_then(Json::as_str)? {
        "CommandExecution" | "command_execution" => {
            let command = match item.get("command")? {
                Json::Array(parts) => parts.last().and_then(Json::as_str).unwrap_or("").to_string(),
                Json::String(s) => s.clone(),
                _ => String::new(),
            };
            let exit_code = item.get("exit_code").and_then(Json::as_i64).map(|n| n as i32);
            let status_failed = item.get("status").and_then(Json::as_str).map(|s| s == "failed");
            Some(ToolCall {
                name: "command_execution".into(),
                summary: limits.summary(&command),
                output: item.get("aggregated_output").and_then(output_text).map(|t| limits.output(&t)),
                failed: match exit_code {
                    Some(code) => Some(code != 0 || status_failed == Some(true)),
                    None => status_failed,
                },
                exit_code,
                paths: Vec::new(),
            })
        }
        "FileChange" | "file_change" => {
            let paths: Vec<String> = match item.get("changes")? {
                Json::Object(map) => map.keys().cloned().collect(),
                Json::Array(items) => items.iter().filter_map(|c| c.get("path").and_then(Json::as_str).map(String::from)).collect(),
                _ => Vec::new(),
            };
            (!paths.is_empty()).then(|| ToolCall {
                name: "apply_patch".into(),
                summary: limits.summary(&paths.join(", ")),
                failed: item.get("status").and_then(Json::as_str).map(|s| s == "failed"),
                paths,
                ..ToolCall::default()
            })
        }
        _ => None,
    }
}

pub(crate) fn parse_codex_with<I: IntoIterator<Item = String>>(lines: I, limits: Limits) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    let mut pending: HashMap<String, (usize, usize)> = HashMap::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<Json>(&line) else { continue };
        if limits.detail && v.get("type").and_then(Json::as_str) == Some("event_msg") {
            let payload = v.get("payload");
            if payload.and_then(|p| p.get("type")).and_then(Json::as_str) == Some("item_completed") {
                // 只附加到已有的最后一条消息上，不新建消息，保证 `#n` 与会话页一致
                if let (Some(tool), Some(last)) = (payload.and_then(|p| p.get("item")).and_then(|item| codex_event_tool(item, limits)), out.last_mut()) {
                    last.tools.push(tool);
                }
            }
            continue;
        }
        if v.get("type").and_then(Json::as_str) != Some("response_item") {
            continue;
        }
        let ts = parse_ts(&v);
        let Some(p) = v.get("payload") else { continue };
        match p.get("type").and_then(Json::as_str) {
            Some("message") => {
                let role = p.get("role").and_then(Json::as_str).unwrap_or("");
                let text = p
                    .get("content")
                    .and_then(Json::as_array)
                    .map(|blocks| blocks.iter().filter_map(|b| b.get("text").and_then(Json::as_str)).collect::<Vec<_>>().join("\n"))
                    .unwrap_or_default();
                match role {
                    "user" if !is_injected(&text) => out.push(Message { role: "user".into(), ts, text: clip(text.trim(), limits.text), tools: Vec::new(), ..Message::default() }),
                    "assistant" => push_assistant(&mut out, ts, Some(text), None, limits.text),
                    _ => {}
                }
            }
            Some("function_call") | Some("custom_tool_call") | Some("local_shell_call") => {
                let input = p.get("arguments").or_else(|| p.get("input")).or_else(|| p.get("action")).cloned().unwrap_or(Json::Null);
                let tool = ToolCall {
                    name: p.get("name").and_then(Json::as_str).unwrap_or("shell").to_string(),
                    summary: tool_summary(&input, limits),
                    paths: tool_paths(&input),
                    ..ToolCall::default()
                };
                push_assistant(&mut out, ts, None, Some(tool), limits.text);
                if let Some(id) = p.get("call_id").and_then(Json::as_str) {
                    let m = out.len() - 1;
                    pending.insert(id.to_string(), (m, out[m].tools.len() - 1));
                }
            }
            Some("function_call_output") | Some("custom_tool_call_output") => {
                if let Some(id) = p.get("call_id").and_then(Json::as_str) {
                    attach_output(&mut out, &mut pending, id, codex_result(p.get("output"), limits));
                }
            }
            _ => {}
        }
    }
    out.retain(|m| !(m.text.is_empty() && m.tools.is_empty()));
    out
}
