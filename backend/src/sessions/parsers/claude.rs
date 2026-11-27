use std::collections::{HashMap, HashSet};
use serde_json::Value as Json;
use crate::sessions::types::{Message, ToolCall};
use super::{
    attach_output, claude_result, clip, is_injected, parse_ts, push_assistant,
    tool_paths, tool_summary, Limits, VIEW,
};

pub fn parse_claude<I: IntoIterator<Item = String>>(lines: I) -> Vec<Message> {
    parse_claude_with(lines, VIEW)
}

pub(crate) fn parse_claude_with<I: IntoIterator<Item = String>>(lines: I, limits: Limits) -> Vec<Message> {
    let mut out = Vec::new();
    let mut pending: HashMap<String, (usize, usize)> = HashMap::new();
    // 压缩后 Claude Code 会把保留段按原 uuid 再写一遍；按 uuid 去重，避免消息重复、序号错位
    let mut seen: HashSet<String> = HashSet::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<Json>(&line) else { continue };
        let kind = v.get("type").and_then(Json::as_str).unwrap_or("");
        if matches!(kind, "user" | "assistant") {
            if let Some(uuid) = v.get("uuid").and_then(Json::as_str) {
                if !seen.insert(uuid.to_string()) {
                    continue;
                }
            }
        }
        if !matches!(kind, "user" | "assistant") || v.get("isSidechain").and_then(Json::as_bool) == Some(true) || v.get("isMeta").and_then(Json::as_bool) == Some(true) {
            continue;
        }
        if v.get("isCompactSummary").and_then(Json::as_bool) == Some(true) {
            continue;
        }
        let ts = parse_ts(&v);
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else { continue };
        if kind == "user" {
            match content {
                Json::String(s) if !is_injected(s) => out.push(Message { role: "user".into(), ts, text: clip(s.trim(), limits.text), tools: Vec::new(), ..Message::default() }),
                Json::Array(blocks) => {
                    let mut texts = Vec::new();
                    for b in blocks {
                        match b.get("type").and_then(Json::as_str) {
                            Some("text") => {
                                if let Some(t) = b.get("text").and_then(Json::as_str).filter(|t| !is_injected(t)) {
                                    texts.push(t.trim().to_string());
                                }
                            }
                            Some("tool_result") => {
                                if let Some(id) = b.get("tool_use_id").and_then(Json::as_str) {
                                    attach_output(&mut out, &mut pending, id, claude_result(b, limits));
                                }
                            }
                            _ => {}
                        }
                    }
                    if !texts.is_empty() {
                        out.push(Message { role: "user".into(), ts, text: clip(&texts.join("\n\n"), limits.text), tools: Vec::new(), ..Message::default() });
                    }
                }
                _ => {}
            }
        } else if let Json::Array(blocks) = content {
            for b in blocks {
                match b.get("type").and_then(Json::as_str) {
                    Some("text") => push_assistant(&mut out, ts, b.get("text").and_then(Json::as_str).map(String::from), None, limits.text),
                    Some("tool_use") => {
                        let input = b.get("input").cloned().unwrap_or(Json::Null);
                        let tool = ToolCall {
                            name: b.get("name").and_then(Json::as_str).unwrap_or("工具").to_string(),
                            summary: tool_summary(&input, limits),
                            paths: tool_paths(&input),
                            ..ToolCall::default()
                        };
                        push_assistant(&mut out, ts, None, Some(tool), limits.text);
                        if let Some(id) = b.get("id").and_then(Json::as_str) {
                            let m = out.len() - 1;
                            pending.insert(id.to_string(), (m, out[m].tools.len() - 1));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out.retain(|m| !(m.text.is_empty() && m.tools.is_empty()));
    out
}
