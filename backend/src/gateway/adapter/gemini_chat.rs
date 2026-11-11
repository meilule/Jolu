//! Gemini 订阅（Code Assist）与 OpenAI Chat 之间的翻译（3.2）。
//!
//! 方向与 `gemini.rs` 相反：那边是 Gemini CLI 调用、Chat 上游；这里是 Chat 调用方
//! （Responses / Anthropic 调用方先在 wire 里转成 Chat）、Gemini 上游。
//! - 请求：messages → contents（assistant → model，tool → functionResponse）、system →
//!   systemInstruction、function 工具 → functionDeclarations（`parametersJsonSchema` 原样带
//!   JSON Schema）、tool_choice → functionCallingConfig、采样参数 → generationConfig；
//! - 回复：parts 的文字 / 思考 / functionCall → content / reasoning_content / tool_calls；
//! - 流式：每个 Gemini 片段转成 Chat 片段，结束时补 finish_reason 与（要求时的）用量片段。
//!
//! Gemini 3 要求历史里的 functionCall 带回它当时给的 thoughtSignature。Chat 协议没地方放，
//! 这里按工具调用 id 记在内存里（有上限）；查不到时用 Google 文档给的占位签名，让模型照常
//! 接着用历史。

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};

use serde_json::{json, Map, Value};

/// 查不到原签名时的占位（Google「迁移历史」文档里的跳过校验写法）。
const SKIP_SIGNATURE: &str = "skip_thought_signature_validator";
const SIGNATURE_CAP: usize = 4096;

static SIGNATURES: LazyLock<Mutex<(HashMap<String, String>, VecDeque<String>)>> =
    LazyLock::new(|| Mutex::new((HashMap::new(), VecDeque::new())));

fn remember_signature(id: &str, signature: &str) {
    if let Ok(mut guard) = SIGNATURES.lock() {
        let (map, order) = &mut *guard;
        if map.insert(id.to_string(), signature.to_string()).is_none() {
            order.push_back(id.to_string());
        }
        while order.len() > SIGNATURE_CAP {
            if let Some(old) = order.pop_front() {
                map.remove(&old);
            }
        }
    }
}

fn signature_of(id: &str) -> Option<String> {
    SIGNATURES.lock().ok()?.0.get(id).cloned()
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str().or_else(|| p.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// `data:<mime>;base64,<数据>` → inlineData。Code Assist 不替调用方下载网络图片。
fn inline_image(url: &str) -> Result<Value, String> {
    let rest = url.strip_prefix("data:").ok_or("Gemini 订阅只接受内嵌（data:）图片，请把图片转成 base64 再发")?;
    let (head, data) = rest.split_once(',').ok_or("图片 data URL 无效")?;
    let mime = head.strip_suffix(";base64").ok_or("图片 data URL 必须是 base64")?;
    Ok(json!({"inlineData": {"mimeType": mime, "data": data}}))
}

fn user_parts(content: &Value) -> Result<Vec<Value>, String> {
    match content {
        Value::String(s) => Ok(vec![json!({"text": s})]),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                match item["type"].as_str() {
                    Some("text") | Some("input_text") => parts.push(json!({"text": item["text"].as_str().unwrap_or_default()})),
                    Some("image_url") => {
                        let url = item["image_url"]["url"].as_str().or_else(|| item["image_url"].as_str()).unwrap_or_default();
                        parts.push(inline_image(url)?);
                    }
                    other => return Err(format!("Gemini 订阅暂不支持「{}」内容块", other.unwrap_or("未知"))),
                }
            }
            Ok(parts)
        }
        Value::Null => Ok(Vec::new()),
        _ => Err("messages 里存在无法解析的内容".into()),
    }
}

/// Chat 请求 → Code Assist 的 `request` 字段。
pub(super) fn chat_to_gemini(chat: &Value) -> Result<Value, String> {
    let messages = chat["messages"].as_array().ok_or("缺少 messages 字段")?;
    if chat["n"].as_u64().is_some_and(|n| n > 1) {
        return Err("暂不支持要求多个候选的请求".into());
    }
    let mut system: Vec<String> = Vec::new();
    let mut contents: Vec<Value> = Vec::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut push = |role: &str, parts: Vec<Value>, contents: &mut Vec<Value>| {
        if parts.is_empty() {
            return;
        }
        // 相邻同角色合成一条：Gemini 要求一轮的函数结果放在同一条里
        if let Some(last) = contents.last_mut().filter(|c| c["role"] == role) {
            if let Some(existing) = last["parts"].as_array_mut() {
                existing.extend(parts);
                return;
            }
        }
        contents.push(json!({"role": role, "parts": parts}));
    };
    for message in messages {
        match message["role"].as_str().unwrap_or_default() {
            "system" | "developer" => system.push(text_of(&message["content"])),
            "user" => push("user", user_parts(&message["content"])?, &mut contents),
            "assistant" => {
                let mut parts = Vec::new();
                let text = text_of(&message["content"]);
                if !text.is_empty() {
                    parts.push(json!({"text": text}));
                }
                let mut first_call = true;
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    let id = call["id"].as_str().unwrap_or_default();
                    let name = call["function"]["name"].as_str().unwrap_or_default();
                    names.insert(id.to_string(), name.to_string());
                    let raw = call["function"]["arguments"].as_str().unwrap_or("{}");
                    let args: Value = serde_json::from_str(if raw.trim().is_empty() { "{}" } else { raw }).unwrap_or(json!({}));
                    let mut part = json!({"functionCall": {"name": name, "args": args}});
                    // 一轮里只有第一个调用带签名（Gemini 3 的规则）
                    if first_call {
                        part["thoughtSignature"] = json!(signature_of(id).unwrap_or_else(|| SKIP_SIGNATURE.into()));
                        first_call = false;
                    }
                    parts.push(part);
                }
                push("model", parts, &mut contents);
            }
            "tool" => {
                let id = message["tool_call_id"].as_str().unwrap_or_default();
                let name = names
                    .get(id)
                    .cloned()
                    .or_else(|| message["name"].as_str().map(str::to_owned))
                    .ok_or("工具结果找不到对应的调用（tool_call_id 不匹配）")?;
                let text = text_of(&message["content"]);
                let response = serde_json::from_str::<Value>(&text)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({"content": text}));
                push("user", vec![json!({"functionResponse": {"name": name, "response": response}})], &mut contents);
            }
            other => return Err(format!("不支持的消息角色「{other}」")),
        }
    }
    if contents.is_empty() {
        return Err("对话内容为空".into());
    }
    let mut request = json!({"contents": contents});
    let system = system.into_iter().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>();
    if !system.is_empty() {
        request["systemInstruction"] = json!({"parts": [{"text": system.join("\n\n")}]});
    }
    let declarations: Vec<Value> = chat["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["type"] == "function")
        .map(|t| {
            let f = &t["function"];
            let mut d = json!({"name": f["name"], "description": f["description"].as_str().unwrap_or_default()});
            if f["parameters"].is_object() {
                d["parametersJsonSchema"] = f["parameters"].clone();
            }
            d
        })
        .collect();
    if !declarations.is_empty() {
        request["tools"] = json!([{"functionDeclarations": declarations}]);
        let config = match &chat["tool_choice"] {
            Value::String(s) if s == "none" => Some(json!({"mode": "NONE"})),
            Value::String(s) if s == "required" => Some(json!({"mode": "ANY"})),
            Value::Object(o) => o
                .get("function")
                .and_then(|f| f["name"].as_str())
                .map(|name| json!({"mode": "ANY", "allowedFunctionNames": [name]})),
            _ => None,
        };
        if let Some(config) = config {
            request["toolConfig"] = json!({"functionCallingConfig": config});
        }
    }
    let mut generation = Map::new();
    if let Some(n) = chat["max_completion_tokens"].as_u64().or_else(|| chat["max_tokens"].as_u64()) {
        generation.insert("maxOutputTokens".into(), json!(n));
    }
    for (from, to) in [("temperature", "temperature"), ("top_p", "topP"), ("seed", "seed")] {
        if let Some(v) = chat.get(from).filter(|v| v.is_number()) {
            generation.insert(to.into(), v.clone());
        }
    }
    match &chat["stop"] {
        Value::String(s) => {
            generation.insert("stopSequences".into(), json!([s]));
        }
        Value::Array(list) => {
            generation.insert("stopSequences".into(), Value::Array(list.clone()));
        }
        _ => {}
    }
    if matches!(chat["response_format"]["type"].as_str(), Some("json_object" | "json_schema")) {
        generation.insert("responseMimeType".into(), json!("application/json"));
        if let Some(schema) = chat["response_format"]["json_schema"]["schema"].as_object() {
            generation.insert("responseJsonSchema".into(), Value::Object(schema.clone()));
        }
    }
    if !generation.is_empty() {
        request["generationConfig"] = Value::Object(generation);
    }
    Ok(request)
}

fn finish_reason(reason: &str) -> &'static str {
    match reason {
        "MAX_TOKENS" => "length",
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY" => "content_filter",
        _ => "stop",
    }
}

fn chat_usage(usage: &Value) -> Option<Value> {
    let prompt = usage["promptTokenCount"].as_i64()?;
    let output = usage["candidatesTokenCount"].as_i64().unwrap_or(0) + usage["thoughtsTokenCount"].as_i64().unwrap_or(0);
    let mut out = json!({
        "prompt_tokens": prompt,
        "completion_tokens": output,
        "total_tokens": usage["totalTokenCount"].as_i64().unwrap_or(prompt + output),
    });
    if let Some(cached) = usage["cachedContentTokenCount"].as_i64() {
        out["prompt_tokens_details"] = json!({"cached_tokens": cached});
    }
    if let Some(thoughts) = usage["thoughtsTokenCount"].as_i64() {
        out["completion_tokens_details"] = json!({"reasoning_tokens": thoughts});
    }
    Some(out)
}

/// 一个 functionCall part → Chat 的 tool_call（记下签名）。
fn tool_call(part: &Value, ordinal: usize, response_id: &str) -> Value {
    let call = &part["functionCall"];
    let id = call["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("call_{}_{ordinal}", response_id.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect::<String>()));
    if let Some(signature) = part["thoughtSignature"].as_str() {
        remember_signature(&id, signature);
    }
    json!({
        "id": id,
        "type": "function",
        "function": {"name": call["name"], "arguments": call["args"].as_object().map(|a| Value::Object(a.clone()).to_string()).unwrap_or_else(|| "{}".into())},
    })
}

/// 完整的 Gemini 回复 → Chat 回复。
pub(super) fn gemini_to_chat(response: &Value, model: &str, id: &str) -> Value {
    let candidate = &response["candidates"][0];
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut calls = Vec::new();
    for part in candidate["content"]["parts"].as_array().into_iter().flatten() {
        if part.get("functionCall").is_some() {
            calls.push(tool_call(part, calls.len(), id));
        } else if let Some(t) = part["text"].as_str() {
            if part["thought"].as_bool() == Some(true) {
                reasoning.push_str(t);
            } else {
                text.push_str(t);
            }
        }
    }
    let finish = if !calls.is_empty() { "tool_calls" } else { finish_reason(candidate["finishReason"].as_str().unwrap_or("STOP")) };
    let mut message = json!({"role": "assistant", "content": if text.is_empty() && !calls.is_empty() { Value::Null } else { json!(text) }});
    if !calls.is_empty() {
        message["tool_calls"] = json!(calls);
    }
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    let mut out = json!({
        "id": id,
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model,
        "choices": [{"index": 0, "message": message, "finish_reason": finish}],
    });
    if let Some(usage) = chat_usage(&response["usageMetadata"]) {
        out["usage"] = usage;
    }
    out
}

/// 流式：Gemini 片段 → Chat 片段。
pub(super) struct ChatChunker {
    id: String,
    model: String,
    started: bool,
    calls: usize,
    finish: Option<String>,
    usage: Option<Value>,
    include_usage: bool,
}

impl ChatChunker {
    pub(super) fn new(id: String, model: String, include_usage: bool) -> Self {
        Self { id, model, started: false, calls: 0, finish: None, usage: None, include_usage }
    }

    fn chunk(&self, delta: Value, finish: Option<&str>) -> Value {
        json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": chrono::Utc::now().timestamp(),
            "model": self.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        })
    }

    pub(super) fn feed(&mut self, response: &Value) -> Vec<Value> {
        let mut out = Vec::new();
        let candidate = &response["candidates"][0];
        for part in candidate["content"]["parts"].as_array().into_iter().flatten() {
            let mut delta = if self.started { json!({}) } else { json!({"role": "assistant"}) };
            if part.get("functionCall").is_some() {
                let mut call = tool_call(part, self.calls, &self.id);
                call["index"] = json!(self.calls);
                self.calls += 1;
                delta["tool_calls"] = json!([call]);
            } else if let Some(t) = part["text"].as_str().filter(|t| !t.is_empty()) {
                let key = if part["thought"].as_bool() == Some(true) { "reasoning_content" } else { "content" };
                delta[key] = json!(t);
            } else {
                continue;
            }
            self.started = true;
            out.push(self.chunk(delta, None));
        }
        if let Some(reason) = candidate["finishReason"].as_str() {
            self.finish = Some(finish_reason(reason).into());
        }
        if response["usageMetadata"].is_object() {
            self.usage = chat_usage(&response["usageMetadata"]);
        }
        out
    }

    /// 结束：finish_reason 片段，以及调用方要了用量时的用量片段。返回（片段，Gemini 用量供记账）。
    pub(super) fn finish(&mut self) -> Vec<Value> {
        let reason = if self.calls > 0 { "tool_calls".to_string() } else { self.finish.clone().unwrap_or_else(|| "stop".into()) };
        let delta = if self.started { json!({}) } else { json!({"role": "assistant", "content": ""}) };
        let mut out = vec![self.chunk(delta, Some(&reason))];
        if self.include_usage {
            if let Some(usage) = &self.usage {
                out.push(json!({
                    "id": self.id, "object": "chat.completion.chunk", "created": chrono::Utc::now().timestamp(),
                    "model": self.model, "choices": [], "usage": usage,
                }));
            }
        }
        out
    }
}

/// SSE 解码：按空行切事件，取 `data:` 行。
#[derive(Default)]
pub(super) struct SseDecoder {
    buffer: String,
    data: Vec<String>,
}

impl SseDecoder {
    pub(super) fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        let mut events = Vec::new();
        while let Some(at) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=at).collect();
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(std::mem::take(&mut self.data).join("\n"));
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data.push(data.trim_start().to_string());
            }
        }
        events
    }

    pub(super) fn finish(&mut self) -> Option<String> {
        if let Some(data) = self.buffer.trim().strip_prefix("data:") {
            self.data.push(data.trim_start().to_string());
        }
        self.buffer.clear();
        (!self.data.is_empty()).then(|| std::mem::take(&mut self.data).join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_history_with_tools_and_images_becomes_gemini_contents() {
        let chat = json!({
            "messages": [
                {"role": "system", "content": "简短回答"},
                {"role": "user", "content": [{"type": "text", "text": "看图"}, {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAA"}}]},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "read", "arguments": "{\"path\":\"a\"}"}},
                    {"id": "c2", "type": "function", "function": {"name": "read", "arguments": "{\"path\":\"b\"}"}},
                ]},
                {"role": "tool", "tool_call_id": "c1", "content": "A"},
                {"role": "tool", "tool_call_id": "c2", "content": "{\"ok\":true}"},
            ],
            "tools": [{"type": "function", "function": {"name": "read", "description": "读", "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}}}],
            "tool_choice": "required",
            "max_tokens": 100,
            "stop": "END",
        });
        let request = chat_to_gemini(&chat).unwrap();
        assert_eq!(request["systemInstruction"]["parts"][0]["text"], "简短回答");
        assert_eq!(request["contents"][0]["parts"][1]["inlineData"]["mimeType"], "image/png");
        let model = &request["contents"][1];
        assert_eq!(model["role"], "model");
        assert_eq!(model["parts"][0]["functionCall"]["args"]["path"], "a");
        assert_eq!(model["parts"][0]["thoughtSignature"], SKIP_SIGNATURE, "查不到原签名时用占位签名");
        assert!(model["parts"][1].get("thoughtSignature").is_none(), "一轮只有第一个调用带签名");
        let results = &request["contents"][2];
        assert_eq!(results["parts"].as_array().unwrap().len(), 2, "同一轮的函数结果放在一条里");
        assert_eq!(results["parts"][0]["functionResponse"]["response"]["content"], "A");
        assert_eq!(results["parts"][1]["functionResponse"]["response"]["ok"], true);
        assert_eq!(request["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"]["type"], "object");
        assert_eq!(request["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
        assert_eq!(request["generationConfig"]["maxOutputTokens"], 100);
        assert_eq!(request["generationConfig"]["stopSequences"], json!(["END"]));
        assert!(chat_to_gemini(&json!({"messages": [{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://x/y.png"}}]}]})).is_err());
    }

    #[test]
    fn gemini_reply_becomes_chat_and_signatures_are_remembered() {
        let response = json!({
            "candidates": [{"content": {"parts": [
                {"text": "想想", "thought": true},
                {"text": "好的"},
                {"functionCall": {"name": "read", "args": {"path": "a"}}, "thoughtSignature": "sig-1"},
            ]}, "finishReason": "STOP"}],
            "usageMetadata": {"promptTokenCount": 3, "candidatesTokenCount": 4, "thoughtsTokenCount": 2, "totalTokenCount": 9},
        });
        let chat = gemini_to_chat(&response, "gemini-3-pro", "resp1");
        let message = &chat["choices"][0]["message"];
        assert_eq!(message["content"], "好的");
        assert_eq!(message["reasoning_content"], "想想");
        assert_eq!(chat["choices"][0]["finish_reason"], "tool_calls");
        let id = message["tool_calls"][0]["id"].as_str().unwrap();
        assert_eq!(message["tool_calls"][0]["function"]["arguments"], "{\"path\":\"a\"}");
        assert_eq!(chat["usage"]["completion_tokens"], 6);
        // 下一轮带着这个调用回来：签名原样还给 Gemini
        let next = chat_to_gemini(&json!({"messages": [
            {"role": "user", "content": "读"},
            {"role": "assistant", "tool_calls": [{"id": id, "type": "function", "function": {"name": "read", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": id, "content": "x"},
        ]}))
        .unwrap();
        assert_eq!(next["contents"][1]["parts"][0]["thoughtSignature"], "sig-1");
    }

    #[test]
    fn stream_chunks_carry_role_once_and_end_with_the_right_reason() {
        let mut chunker = ChatChunker::new("r".into(), "m".into(), true);
        let a = chunker.feed(&json!({"candidates": [{"content": {"parts": [{"text": "你"}]}}]}));
        let b = chunker.feed(&json!({"candidates": [{"content": {"parts": [{"text": "好"}]}, "finishReason": "MAX_TOKENS"}], "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 2, "totalTokenCount": 3}}));
        assert_eq!(a[0]["choices"][0]["delta"]["role"], "assistant");
        assert!(b[0]["choices"][0]["delta"].get("role").is_none());
        let end = chunker.finish();
        assert_eq!(end[0]["choices"][0]["finish_reason"], "length");
        assert_eq!(end[1]["usage"]["total_tokens"], 3);
        let mut calls = ChatChunker::new("r".into(), "m".into(), false);
        let c = calls.feed(&json!({"candidates": [{"content": {"parts": [{"functionCall": {"name": "f", "args": {}}}]}}]}));
        assert_eq!(c[0]["choices"][0]["delta"]["tool_calls"][0]["index"], 0);
        let end = calls.finish();
        assert_eq!(end.len(), 1, "没要用量就不发用量片段");
        assert_eq!(end[0]["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn sse_events_split_on_blank_lines_across_chunks() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.feed(b"data: {\"a\":").is_empty());
        let events = decoder.feed(b"1}\r\n\r\ndata: {\"b\":2}\n\n");
        assert_eq!(events, vec!["{\"a\":1}", "{\"b\":2}"]);
        decoder.feed(b"data: tail");
        assert_eq!(decoder.finish().as_deref(), Some("tail"));
    }
}
