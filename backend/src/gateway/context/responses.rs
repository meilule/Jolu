//! OpenAI Responses（移植 magpie `responses` / `responsesTool` / `responsesItem` / `responsesContent`）。
//!
//! Jolu 的不同：认不出的条目类型与消息角色记到 unknown（magpie 记到对话）；
//! 其他 `*_call` / `*_call_output` 条目（web_search_call、mcp_call……）按调用与结果认。

use serde_json::Value;

use super::calls::{call_of, call_of_text};
use super::estimate::{len_tokens, raw_tokens, tokens_of, IMAGE_TOKENS};
use super::parts::{Builder, Kind};
use super::text::user_says;

/// 消息内容的一段：文字，或图片 / 文件。
pub(super) struct RPart<'a> {
    pub text: &'a str,
    pub image: bool,
}

pub(super) fn content_of(c: Option<&Value>) -> Vec<RPart<'_>> {
    match c {
        Some(Value::String(s)) => vec![RPart { text: s, image: false }],
        Some(Value::Array(a)) => a
            .iter()
            .map(|p| {
                let ty = p.get("type").and_then(Value::as_str).unwrap_or("");
                RPart {
                    text: p.get("text").and_then(Value::as_str).unwrap_or(""),
                    image: matches!(ty, "input_image" | "input_file" | "image_url"),
                }
            })
            .collect(),
        _ => vec![],
    }
}

pub(super) fn parse(b: &mut Builder, req: &Value) -> bool {
    let input = req.get("input");
    if input.is_none() && req.get("instructions").is_none() {
        return false;
    }
    b.held = req.get("previous_response_id").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    if let Some(ins) = req.get("instructions").and_then(Value::as_str) {
        if !ins.is_empty() {
            b.system_text(ins, "prompt");
        }
    }
    if let Some(tools) = req.get("tools").and_then(Value::as_array) {
        for t in tools {
            tool(b, t);
        }
    }
    match input {
        Some(Value::String(s)) => {
            b.new_turn();
            b.user_text(s);
        }
        Some(Value::Array(items)) => {
            for it in items {
                item(b, it);
            }
        }
        _ => {}
    }
    true
}

/// 一个工具定义（Chat Completions 的也走这里）：命名空间里 Codex 自己的函数逐个记，其他命名空间整体记一项。
pub(super) fn tool(b: &mut Builder, t: &Value) {
    let ty = t.get("type").and_then(Value::as_str).unwrap_or("");
    let mut name = t.get("name").and_then(Value::as_str).unwrap_or("");
    if name.is_empty() {
        name = t.get("function").and_then(|f| f.get("name")).and_then(Value::as_str).unwrap_or("");
    }
    if name.is_empty() {
        name = ty; // web_search、image_generation……
    }
    let sub = t.get("tools").and_then(Value::as_array).filter(|a| !a.is_empty());
    if ty == "namespace" || sub.is_some() {
        let subs = sub.map_or(&[][..], |a| a.as_slice());
        if name == "functions" {
            for x in subs {
                tool(b, x);
            }
            return;
        }
        // MCP 服务器的命名空间按工具命名规则叫 mcp__<服务器>…；Codex 自己的（clock、collaboration……）不是 MCP
        let (tag, name) = match name.strip_prefix("mcp__") {
            Some(rest) => ("mcp", rest.split("__").next().unwrap_or(rest).trim_end_matches('_')),
            None => ("namespace", name),
        };
        b.add(Kind::Tools, name, tag, raw_tokens(t));
        b.add_n(Kind::Tools, name, tag, subs.len() as i64 - 1);
        return;
    }
    b.tool(name, raw_tokens(t));
}

fn user_parts(b: &mut Builder, parts: &[RPart]) {
    if parts.iter().any(|p| p.image || user_says(p.text)) {
        b.new_turn();
    }
    for p in parts {
        if p.image {
            b.add(Kind::Files, "image", "image", IMAGE_TOKENS);
            continue;
        }
        b.user_text(p.text);
    }
}

fn item(b: &mut Builder, raw: &Value) {
    let s = |k: &str| raw.get(k).and_then(Value::as_str).unwrap_or("");
    let role = s("role");
    let mut ty = s("type");
    if ty.is_empty() && !role.is_empty() {
        ty = "message";
    }
    match ty {
        "additional_tools" => {
            for t in raw.get("tools").and_then(Value::as_array).into_iter().flatten() {
                tool(b, t);
            }
        }
        "message" => {
            let parts = content_of(raw.get("content"));
            match role {
                "system" | "developer" => {
                    for p in &parts {
                        b.system_text(p.text, "prompt");
                    }
                }
                "user" => user_parts(b, &parts),
                "assistant" => {
                    for p in &parts {
                        b.chat(tokens_of(p.text));
                    }
                }
                _ => b.add(Kind::Unknown, if role.is_empty() { "(no role)" } else { role }, "role", raw_tokens(raw)),
            }
        }
        "reasoning" => {
            let mut n = 0;
            for x in raw.get("summary").and_then(Value::as_array).into_iter().flatten() {
                n += tokens_of(x.get("text").and_then(Value::as_str).unwrap_or(""));
            }
            for x in raw.get("content").and_then(Value::as_array).into_iter().flatten() {
                n += tokens_of(x.get("text").and_then(Value::as_str).unwrap_or(""));
            }
            // 封存的推理：base64 的约四分之三是原文
            n += (s("encrypted_content").len() * 3 / 4 / 4) as i64;
            b.chat(n);
        }
        "function_call" | "custom_tool_call" | "local_shell_call" | "tool_search_call" => {
            let name = s("name");
            let (call, len) = match ty {
                "custom_tool_call" => (call_of_text(name, s("input")), s("input").len()),
                "local_shell_call" => {
                    let action = raw.get("action");
                    let len = action.map_or(0, super::estimate::json_len);
                    (call_of(name, action.and_then(Value::as_object), None), len)
                }
                _ => match raw.get("arguments") {
                    Some(Value::String(a)) => (call_of_text(name, a), a.len()),
                    Some(v) => (call_of(name, v.as_object(), None), super::estimate::json_len(v)),
                    None => (call_of(name, None, None), 0),
                },
            };
            b.calls.insert(s("call_id").to_string(), call);
            b.chat(tokens_of(name) + len_tokens(len));
        }
        "function_call_output" | "custom_tool_call_output" | "local_shell_call_output" | "tool_search_output" => {
            let mut n = 0;
            match raw.get("output") {
                Some(Value::String(o)) => n = tokens_of(o),
                out => {
                    for p in content_of(out) {
                        if p.image {
                            b.add(Kind::Files, "image", "image", IMAGE_TOKENS);
                            continue;
                        }
                        n += tokens_of(p.text);
                    }
                }
            }
            if ty == "tool_search_output" {
                for t in raw.get("tools").and_then(Value::as_array).into_iter().flatten() {
                    tool(b, t);
                }
            }
            b.result(s("call_id"), n);
        }
        "compaction" | "compaction_summary" => b.add(Kind::Chat, "compacted", "compacted", raw_tokens(raw)),
        "item_reference" => b.chat(raw_tokens(raw)),
        _ if ty.ends_with("_call_output") => {
            let tokens = raw.get("output").map_or(0, |o| o.as_str().map_or_else(|| raw_tokens(o), tokens_of));
            b.result(s("call_id"), tokens);
        }
        _ if ty.ends_with("_call") => {
            // 上游托管的工具（web_search_call、mcp_call……）：调用连同上游带回的内容一起算对话
            b.chat(raw_tokens(raw));
        }
        _ => b.add(Kind::Unknown, if ty.is_empty() { "(no type)" } else { ty }, "type", raw_tokens(raw)),
    }
}
