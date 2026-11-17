//! Gemini 原生 `generateContent`（移植 magpie `gemini`）。
//!
//! 字段同时认 camelCase 与 snake_case（REST 两种都收）。`model` 角色里带 `thought` 的段落按思考计入对话（与 magpie 一致）。
//! Jolu 的不同：`executableCode` 算对话、`codeExecutionResult` 算结果；认不出的段落记到 unknown（magpie 当作用户文字）。

use serde_json::Value;

use super::calls::call_of;
use super::estimate::{raw_tokens, tokens_of, IMAGE_TOKENS};
use super::parts::{Builder, Kind, ToolCall};
use super::text::user_says;

fn get<'a>(v: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    v.get(camel).or_else(|| v.get(snake))
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

pub(super) fn parse(b: &mut Builder, req: &Value) -> bool {
    let Some(contents) = req.get("contents").and_then(Value::as_array) else { return false };
    if let Some(sys) = get(req, "systemInstruction", "system_instruction") {
        match sys {
            Value::String(s) => b.system_text(s, "prompt"),
            _ => {
                for p in sys.get("parts").and_then(Value::as_array).into_iter().flatten() {
                    b.system_text(str_of(p, "text"), "prompt");
                }
            }
        }
    }
    for t in req.get("tools").and_then(Value::as_array).into_iter().flatten() {
        for d in get(t, "functionDeclarations", "function_declarations").and_then(Value::as_array).into_iter().flatten() {
            b.tool(str_of(d, "name"), raw_tokens(d));
        }
    }
    for c in contents {
        let role = str_of(c, "role");
        let parts = c.get("parts").and_then(Value::as_array).map_or(&[][..], |a| a.as_slice());
        let is_response = |p: &Value| get(p, "functionResponse", "function_response").is_some();
        if role != "model" && parts.iter().any(|p| !is_response(p) && user_says(str_of(p, "text"))) {
            b.new_turn();
        }
        for p in parts {
            if let Some(fc) = get(p, "functionCall", "function_call") {
                // Gemini 按 id 或函数名把结果对上调用
                let name = str_of(fc, "name");
                let id = match str_of(fc, "id") {
                    "" => format!("name:{name}"),
                    id => id.to_string(),
                };
                let args = fc.get("args");
                b.calls.insert(id, call_of(name, args.and_then(Value::as_object), None));
                b.chat(tokens_of(name) + args.map_or(0, raw_tokens));
            } else if let Some(fr) = get(p, "functionResponse", "function_response") {
                let name = str_of(fr, "name");
                let id = match str_of(fr, "id") {
                    "" => format!("name:{name}"),
                    id => id.to_string(),
                };
                b.calls.entry(id.clone()).or_insert_with(|| ToolCall { name: name.to_string(), ..Default::default() });
                let response = fr.get("response");
                // Gemini CLI 把输出放在 response.output，出错时在 response.error
                let output = response
                    .and_then(|r| r.get("output").or_else(|| r.get("error")).or_else(|| r.get("content")))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                b.result(&id, response.map_or(0, raw_tokens), output);
            } else if get(p, "inlineData", "inline_data").is_some() || get(p, "fileData", "file_data").is_some() {
                b.add(Kind::Files, "image", "image", IMAGE_TOKENS);
            } else if let Some(code) = get(p, "executableCode", "executable_code") {
                b.chat(raw_tokens(code));
            } else if let Some(out) = get(p, "codeExecutionResult", "code_execution_result") {
                b.add(Kind::Results, "code_execution", "result", raw_tokens(out));
            } else if let Some(text) = p.get("text").and_then(Value::as_str) {
                if role == "model" {
                    b.chat(tokens_of(text));
                } else {
                    b.user_text(text);
                }
            } else {
                let key = p.as_object().and_then(|o| o.keys().find(|k| !matches!(k.as_str(), "thought" | "thoughtSignature" | "thought_signature")));
                match key {
                    Some(k) => b.add(Kind::Unknown, k, "type", raw_tokens(p)),
                    // 只有思考签名的段落：属于模型这一轮
                    None => b.chat(raw_tokens(p)),
                }
            }
        }
    }
    true
}
