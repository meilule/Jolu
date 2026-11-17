//! OpenAI Chat Completions（移植 magpie `chatCompletions`）。
//!
//! Jolu 的不同：认不出的消息角色记到 unknown（magpie 记到对话）；推理字段也认 `reasoning`。

use serde_json::Value;

use super::calls::call_of_text;
use super::estimate::{raw_tokens, tokens_of, IMAGE_TOKENS};
use super::parts::{Builder, Kind};
use super::responses::{content_of, tool};
use super::text::user_says;

pub(super) fn parse(b: &mut Builder, req: &Value) -> bool {
    let Some(messages) = req.get("messages").and_then(Value::as_array) else { return false };
    if let Some(tools) = req.get("tools").and_then(Value::as_array) {
        for t in tools {
            tool(b, t);
        }
    }
    for m in messages {
        let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("");
        let parts = content_of(m.get("content"));
        match s("role") {
            "system" | "developer" => {
                for p in &parts {
                    b.system_text(p.text, "prompt");
                }
            }
            "user" => {
                if parts.iter().any(|p| p.image || user_says(p.text)) {
                    b.new_turn();
                }
                for p in &parts {
                    if p.image {
                        b.add(Kind::Files, "image", "image", IMAGE_TOKENS);
                        continue;
                    }
                    b.user_text(p.text);
                }
            }
            "tool" | "function" => {
                let mut n = 0;
                for p in &parts {
                    if p.image {
                        b.add(Kind::Files, "image", "image", IMAGE_TOKENS);
                        continue;
                    }
                    n += tokens_of(p.text);
                }
                // 旧式 function 角色没有调用 id，按函数名认
                let id = match s("tool_call_id") {
                    "" => format!("name:{}", s("name")),
                    id => id.to_string(),
                };
                b.result(&id, n);
            }
            "assistant" => {
                // 推理、说的话、调用各算一段
                let reasoning = if s("reasoning_content").is_empty() { s("reasoning") } else { s("reasoning_content") };
                b.chat(tokens_of(reasoning));
                b.chat(parts.iter().map(|p| tokens_of(p.text)).sum());
                for c in m.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
                    let f = c.get("function");
                    let name = f.and_then(|f| f.get("name")).and_then(Value::as_str).unwrap_or("");
                    let args = f.and_then(|f| f.get("arguments")).and_then(Value::as_str).unwrap_or("");
                    let id = c.get("id").and_then(Value::as_str).unwrap_or("");
                    b.calls.insert(id.to_string(), call_of_text(name, args));
                    b.chat(tokens_of(name) + tokens_of(args));
                }
                if let Some(fc) = m.get("function_call") {
                    let name = fc.get("name").and_then(Value::as_str).unwrap_or("");
                    let args = fc.get("arguments").and_then(Value::as_str).unwrap_or("");
                    b.calls.insert(format!("name:{name}"), call_of_text(name, args));
                    b.chat(tokens_of(name) + tokens_of(args));
                }
            }
            role => b.add(Kind::Unknown, if role.is_empty() { "(no role)" } else { role }, "role", raw_tokens(m)),
        }
    }
    true
}
