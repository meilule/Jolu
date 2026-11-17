//! Anthropic Messages（移植 magpie `anthropic` / `anthropicBlock`）。
//!
//! Jolu 的不同：认不出的内容块类型与消息角色记到 unknown（magpie 记到对话）；
//! `mcp_tool_use` 与各种 `*_tool_result` 按调用与结果认。

use serde_json::Value;

use super::calls::call_of;
use super::estimate::{raw_tokens, tokens_of, IMAGE_TOKENS};
use super::parts::{Builder, Kind};
use super::text::user_says;

/// 内容：字符串，或内容块数组。
pub(super) enum Block<'a> {
    Text(&'a str),
    Obj(&'a Value),
}

impl<'a> Block<'a> {
    fn ty(&self) -> &'a str {
        match self {
            Block::Text(_) => "text",
            Block::Obj(v) => v.get("type").and_then(Value::as_str).unwrap_or(""),
        }
    }

    fn str_field(&self, key: &str) -> &'a str {
        match self {
            Block::Text(s) if key == "text" => s,
            Block::Text(_) => "",
            Block::Obj(v) => v.get(key).and_then(Value::as_str).unwrap_or(""),
        }
    }

    fn field(&self, key: &str) -> Option<&'a Value> {
        match self {
            Block::Text(_) => None,
            Block::Obj(v) => v.get(key),
        }
    }
}

pub(super) fn blocks_of(c: Option<&Value>) -> Vec<Block<'_>> {
    match c {
        Some(Value::String(s)) => vec![Block::Text(s)],
        Some(Value::Array(a)) => a.iter().map(Block::Obj).collect(),
        _ => vec![],
    }
}

pub(super) fn parse(b: &mut Builder, req: &Value) -> bool {
    let Some(messages) = req.get("messages").and_then(Value::as_array) else { return false };
    for s in blocks_of(req.get("system")) {
        let text = s.str_field("text");
        if text.starts_with("x-anthropic-billing-header") {
            b.add(Kind::System, "billing", "", tokens_of(text));
            continue;
        }
        b.system_text(text, "prompt");
    }
    if let Some(tools) = req.get("tools").and_then(Value::as_array) {
        for t in tools {
            let name = t.get("name").and_then(Value::as_str).unwrap_or("");
            b.tool(name, raw_tokens(t));
        }
    }
    for m in messages {
        let role = m.get("role").and_then(Value::as_str).unwrap_or("");
        let bs = blocks_of(m.get("content"));
        match role {
            "system" | "developer" => {
                for x in &bs {
                    b.system_text(x.str_field("text"), "harness");
                }
                continue;
            }
            "user" => {
                // 回答工具调用的消息接着当前这一轮；用户说了话的另起一轮
                let results = bs.iter().any(|x| x.ty() == "tool_result");
                if !results && bs.iter().any(|x| x.ty() == "text" && user_says(x.str_field("text"))) {
                    b.new_turn();
                }
            }
            "assistant" => {}
            _ => {
                b.add(Kind::Unknown, role_name(role), "role", raw_tokens(m));
                continue;
            }
        }
        for x in &bs {
            block(b, role, x);
        }
    }
    true
}

fn role_name(role: &str) -> &str {
    if role.is_empty() {
        "(no role)"
    } else {
        role
    }
}

fn block(b: &mut Builder, role: &str, x: &Block) {
    let ty = x.ty();
    match ty {
        "text" => {
            let text = x.str_field("text");
            if role == "user" {
                b.user_text(text);
            } else {
                b.chat(tokens_of(text));
            }
        }
        "thinking" => b.chat(tokens_of(x.str_field("thinking"))),
        "redacted_thinking" => b.chat(tokens_of(x.str_field("data")) / 4),
        "tool_use" | "server_tool_use" | "mcp_tool_use" => {
            let name = x.str_field("name");
            let input = x.field("input");
            b.calls.insert(x.str_field("id").to_string(), call_of(name, input.and_then(Value::as_object), None));
            b.chat(tokens_of(name) + input.map_or(0, raw_tokens));
        }
        "image" | "document" => {
            let source = x.field("source");
            // 不是内嵌 base64 的文档（纯文字、链接）按原文算
            if ty == "document" && source.is_some_and(|s| s.get("type").and_then(Value::as_str) != Some("base64")) {
                b.add(Kind::Files, "document", "image", source.map_or(0, raw_tokens));
                return;
            }
            b.add(Kind::Files, ty, "image", IMAGE_TOKENS);
        }
        _ if ty == "tool_result" || ty.ends_with("_tool_result") => {
            let content = x.field("content");
            let mut n = 0;
            match content {
                Some(Value::String(s)) => n = tokens_of(s),
                Some(Value::Array(_)) => {
                    for c in blocks_of(content) {
                        match c.ty() {
                            "image" | "document" => b.add(Kind::Files, c.ty(), "image", IMAGE_TOKENS),
                            _ => n += tokens_of(c.str_field("text")),
                        }
                    }
                }
                Some(other) => n = raw_tokens(other),
                None => {}
            }
            b.result(x.str_field("tool_use_id"), n);
        }
        _ => {
            if let Block::Obj(v) = x {
                b.add(Kind::Unknown, if ty.is_empty() { "(no type)" } else { ty }, "type", raw_tokens(v));
            }
        }
    }
}
