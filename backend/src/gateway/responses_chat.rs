//! OpenAI Responses ⇄ Chat Completions（3.3）：让 Codex（只说 Responses）用只提供 Chat 接口的第三方来源。
//!
//! 为什么：Codex CLI 只发 `/v1/responses`，而 DeepSeek、Kimi、GLM、OpenRouter 等绝大多数第三方只有
//! `/v1/chat/completions`。不翻译时这些来源对 Codex 一律 404，只能用 OpenAI 自己。
//!
//! 管什么：
//! - 请求：instructions、消息（文本与图片）、`function_call` / `function_call_output` 历史、Codex 的
//!   自由格式工具（`custom`，如 apply_patch：包成只有一个 `input` 字符串参数的函数，回来再拆回去）、
//!   tools、tool_choice、max_output_tokens、reasoning.effort、temperature / top_p；
//! - 回复：非流式完整对象，以及流式事件（`response.created` → 文本 / 参数增量 →
//!   `response.output_item.done` → `response.completed` 带用量）；推理内容（`reasoning_content`）作为
//!   reasoning 项的摘要给出。
//!
//! 不管什么：Responses 的内置工具（web_search、file_search、local_shell 等）Chat 没有对应物，丢弃并在
//! 日志里说明；`previous_response_id` 这类服务端会话状态 Chat 没有，Codex 默认不用。

use serde_json::{json, Value};

/// 包装 Codex 自由格式工具时用的参数名。
const CUSTOM_INPUT: &str = "input";

/// 请求里的自由格式工具名（回复时要把同名的函数调用拆回 `custom_tool_call`）。
pub(crate) fn custom_tool_names(body: &Value) -> Vec<String> {
    body.get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter(|tool| tool.get("type").and_then(Value::as_str) == Some("custom"))
                .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn content_parts(content: &Value) -> Value {
    match content {
        Value::String(text) => Value::String(text.clone()),
        Value::Array(items) => {
            let parts: Vec<Value> = items
                .iter()
                .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("input_text" | "output_text" | "text") => Some(json!({"type": "text", "text": part.get("text").cloned().unwrap_or(json!(""))})),
                    Some("input_image") => {
                        let url = part.get("image_url").and_then(Value::as_str).map(str::to_string)
                            .or_else(|| part.pointer("/image_url/url").and_then(Value::as_str).map(str::to_string))?;
                        Some(json!({"type": "image_url", "image_url": {"url": url}}))
                    }
                    _ => None,
                })
                .collect();
            // 只有文字时合成一个字符串：部分第三方不认数组形式的 assistant 内容
            if parts.iter().all(|p| p["type"] == "text") {
                Value::String(parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join(""))
            } else {
                Value::Array(parts)
            }
        }
        _ => Value::String(String::new()),
    }
}

/// Responses 请求 → Chat 请求。
pub(crate) fn request(body: &Value) -> Result<Value, String> {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(instructions) = body.get("instructions").and_then(Value::as_str).filter(|t| !t.is_empty()) {
        messages.push(json!({"role": "system", "content": instructions}));
    }
    match body.get("input") {
        Some(Value::String(text)) => messages.push(json!({"role": "user", "content": text})),
        Some(Value::Array(items)) => {
            for item in items {
                let kind = item.get("type").and_then(Value::as_str).unwrap_or("message");
                match kind {
                    "message" => {
                        let role = match item.get("role").and_then(Value::as_str).unwrap_or("user") {
                            "developer" => "system",
                            other => other,
                        };
                        messages.push(json!({"role": role, "content": content_parts(item.get("content").unwrap_or(&Value::Null))}));
                    }
                    "function_call" | "custom_tool_call" => {
                        let arguments = if kind == "custom_tool_call" {
                            json!({CUSTOM_INPUT: item.get("input").cloned().unwrap_or(json!(""))}).to_string()
                        } else {
                            item.get("arguments").and_then(Value::as_str).unwrap_or("{}").to_string()
                        };
                        let call = json!({
                            "id": item.get("call_id").cloned().unwrap_or(json!("")),
                            "type": "function",
                            "function": {"name": item.get("name").cloned().unwrap_or(json!("")), "arguments": arguments},
                        });
                        // 连续的调用并进同一条 assistant 消息（Chat 的形状）
                        match messages.last_mut() {
                            Some(last) if last["role"] == "assistant" && last.get("tool_calls").is_some() => {
                                last["tool_calls"].as_array_mut().map(|calls| calls.push(call));
                            }
                            _ => messages.push(json!({"role": "assistant", "content": Value::Null, "tool_calls": [call]})),
                        }
                    }
                    "function_call_output" | "custom_tool_call_output" => {
                        let output = match item.get("output") {
                            Some(Value::String(text)) => text.clone(),
                            Some(other) => other.to_string(),
                            None => String::new(),
                        };
                        messages.push(json!({"role": "tool", "tool_call_id": item.get("call_id").cloned().unwrap_or(json!("")), "content": output}));
                    }
                    // 推理项（加密内容）与其他服务端状态 Chat 接不住，跳过
                    _ => {}
                }
            }
        }
        _ => return Err("Responses 请求缺少 input".into()),
    }
    let mut chat = json!({"model": body.get("model").cloned().unwrap_or(json!("")), "messages": messages});
    let streaming = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    if streaming {
        chat["stream"] = json!(true);
        chat["stream_options"] = json!({"include_usage": true});
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let mut converted = Vec::new();
        for tool in tools {
            match tool.get("type").and_then(Value::as_str) {
                Some("function") => converted.push(json!({"type": "function", "function": {
                    "name": tool.get("name").cloned().unwrap_or(json!("")),
                    "description": tool.get("description").cloned().unwrap_or(json!("")),
                    "parameters": tool.get("parameters").cloned().unwrap_or(json!({"type": "object", "properties": {}})),
                }})),
                Some("custom") => converted.push(json!({"type": "function", "function": {
                    "name": tool.get("name").cloned().unwrap_or(json!("")),
                    "description": tool.get("description").cloned().unwrap_or(json!("")),
                    "parameters": {"type": "object", "properties": {CUSTOM_INPUT: {"type": "string", "description": "工具的完整输入原文"}}, "required": [CUSTOM_INPUT]},
                }})),
                other => eprintln!("[协议转换] Responses 内置工具 {other:?} 在 Chat 接口没有对应物，已省略"),
            }
        }
        if !converted.is_empty() {
            chat["tools"] = Value::Array(converted);
        }
    }
    match body.get("tool_choice") {
        Some(Value::String(choice)) => chat["tool_choice"] = json!(choice),
        Some(choice) if choice.get("name").is_some() => chat["tool_choice"] = json!({"type": "function", "function": {"name": choice["name"]}}),
        _ => {}
    }
    for (from, to) in [("max_output_tokens", "max_tokens"), ("temperature", "temperature"), ("top_p", "top_p"), ("parallel_tool_calls", "parallel_tool_calls")] {
        if let Some(value) = body.get(from).filter(|v| !v.is_null()) {
            chat[to] = value.clone();
        }
    }
    if let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str).filter(|e| !e.is_empty() && *e != "none") {
        chat["reasoning_effort"] = json!(effort);
    }
    Ok(chat)
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn usage_of(chat_usage: &Value) -> Value {
    let input = chat_usage.get("prompt_tokens").and_then(Value::as_i64).unwrap_or(0);
    let output = chat_usage.get("completion_tokens").and_then(Value::as_i64).unwrap_or(0);
    let mut usage = json!({"input_tokens": input, "output_tokens": output, "total_tokens": chat_usage.get("total_tokens").and_then(Value::as_i64).unwrap_or(input + output)});
    if let Some(cached) = chat_usage.pointer("/prompt_tokens_details/cached_tokens") {
        usage["input_tokens_details"] = json!({"cached_tokens": cached});
    }
    if let Some(reasoning) = chat_usage.pointer("/completion_tokens_details/reasoning_tokens") {
        usage["output_tokens_details"] = json!({"reasoning_tokens": reasoning});
    }
    usage
}

fn call_item(id: &str, call_id: &str, name: &str, arguments: &str, custom: &[String]) -> Value {
    if custom.iter().any(|c| c == name) {
        let input = serde_json::from_str::<Value>(arguments)
            .ok()
            .and_then(|v| v.get(CUSTOM_INPUT).and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| arguments.to_string());
        json!({"type": "custom_tool_call", "id": id, "call_id": call_id, "name": name, "input": input, "status": "completed"})
    } else {
        json!({"type": "function_call", "id": id, "call_id": call_id, "name": name, "arguments": arguments, "status": "completed"})
    }
}

/// 非流式 Chat 回复 → Responses 完整对象。
pub(crate) fn response(chat: &Value, custom: &[String]) -> Value {
    let message = chat.pointer("/choices/0/message").cloned().unwrap_or(json!({}));
    let mut output: Vec<Value> = Vec::new();
    if let Some(reasoning) = message.get("reasoning_content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
        output.push(json!({"type": "reasoning", "id": "rs_0", "summary": [{"type": "summary_text", "text": reasoning}]}));
    }
    if let Some(text) = message.get("content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
        output.push(json!({"type": "message", "id": "msg_0", "role": "assistant", "status": "completed",
            "content": [{"type": "output_text", "text": text, "annotations": []}]}));
    }
    for (index, call) in message.get("tool_calls").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let call_id = call.get("id").and_then(Value::as_str).unwrap_or_default();
        output.push(call_item(&format!("fc_{index}"), call_id, call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default(),
            call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("{}"), custom));
    }
    json!({
        "id": chat.get("id").and_then(Value::as_str).map(|id| format!("resp_{id}")).unwrap_or_else(|| "resp_jolu".into()),
        "object": "response",
        "created_at": chat.get("created").cloned().unwrap_or(json!(now_secs())),
        "status": "completed",
        "model": chat.get("model").cloned().unwrap_or(json!("")),
        "output": output,
        "usage": usage_of(chat.get("usage").unwrap_or(&Value::Null)),
    })
}

/// 流式翻译：逐个喂 Chat 的 SSE 数据块，吐出 Responses 的 SSE 事件文本。
#[derive(Debug, Default)]
pub(crate) struct StreamTranslator {
    custom: Vec<String>,
    started: bool,
    id: String,
    model: String,
    reasoning: String,
    reasoning_index: Option<usize>,
    text: String,
    text_index: Option<usize>,
    /// Chat 的工具调用序号 → (输出序号, call_id, name, arguments)
    calls: Vec<(usize, usize, String, String, String)>,
    next_index: usize,
    usage: Option<Value>,
    sequence: u64,
}

impl StreamTranslator {
    pub fn new(custom: Vec<String>) -> Self {
        Self { custom, ..Default::default() }
    }

    fn event(&mut self, out: &mut String, name: &str, mut data: Value) {
        data["type"] = json!(name);
        data["sequence_number"] = json!(self.sequence);
        self.sequence += 1;
        out.push_str(&format!("event: {name}\ndata: {data}\n\n"));
    }

    fn base(&self, status: &str, output: Vec<Value>) -> Value {
        let mut value = json!({"id": self.id, "object": "response", "created_at": now_secs(), "status": status, "model": self.model, "output": output});
        if let Some(usage) = &self.usage {
            value["usage"] = usage.clone();
        }
        value
    }

    /// 喂一个 Chat 流式数据块（`data:` 后面的 JSON）。
    pub fn push(&mut self, chunk: &Value) -> String {
        let mut out = String::new();
        if !self.started {
            self.started = true;
            self.id = chunk.get("id").and_then(Value::as_str).map(|id| format!("resp_{id}")).unwrap_or_else(|| "resp_jolu".into());
            self.model = chunk.get("model").and_then(Value::as_str).unwrap_or_default().to_string();
            let created = self.base("in_progress", Vec::new());
            self.event(&mut out, "response.created", json!({"response": created.clone()}));
            self.event(&mut out, "response.in_progress", json!({"response": created}));
        }
        if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
            self.usage = Some(usage_of(usage));
        }
        let Some(delta) = chunk.pointer("/choices/0/delta") else { return out };
        if let Some(reasoning) = delta.get("reasoning_content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            if self.reasoning_index.is_none() {
                let index = self.next_index;
                self.next_index += 1;
                self.reasoning_index = Some(index);
                self.event(&mut out, "response.output_item.added", json!({"output_index": index, "item": {"type": "reasoning", "id": "rs_0", "summary": []}}));
            }
            self.reasoning.push_str(reasoning);
            let index = self.reasoning_index.unwrap_or(0);
            self.event(&mut out, "response.reasoning_summary_text.delta", json!({"item_id": "rs_0", "output_index": index, "summary_index": 0, "delta": reasoning}));
        }
        if let Some(text) = delta.get("content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            if self.text_index.is_none() {
                let index = self.next_index;
                self.next_index += 1;
                self.text_index = Some(index);
                self.event(&mut out, "response.output_item.added", json!({"output_index": index, "item": {"type": "message", "id": "msg_0", "role": "assistant", "status": "in_progress", "content": []}}));
                self.event(&mut out, "response.content_part.added", json!({"item_id": "msg_0", "output_index": index, "content_index": 0, "part": {"type": "output_text", "text": "", "annotations": []}}));
            }
            self.text.push_str(text);
            let index = self.text_index.unwrap_or(0);
            self.event(&mut out, "response.output_text.delta", json!({"item_id": "msg_0", "output_index": index, "content_index": 0, "delta": text}));
        }
        for call in delta.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
            let chat_index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let position = match self.calls.iter().position(|c| c.0 == chat_index) {
                Some(position) => position,
                None => {
                    let index = self.next_index;
                    self.next_index += 1;
                    let call_id = call.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                    let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default().to_string();
                    self.calls.push((chat_index, index, call_id.clone(), name.clone(), String::new()));
                    let item = call_item(&format!("fc_{chat_index}"), &call_id, &name, "", &self.custom);
                    self.event(&mut out, "response.output_item.added", json!({"output_index": index, "item": item}));
                    self.calls.len() - 1
                }
            };
            if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str).filter(|a| !a.is_empty()) {
                self.calls[position].4.push_str(arguments);
                let (chat_index, index, ..) = self.calls[position].clone();
                // 自由格式工具的参数是包装后的 JSON，增量没法逐字拆回原文：等结束时一次给出
                if !self.custom.iter().any(|c| *c == self.calls[position].3) {
                    self.event(&mut out, "response.function_call_arguments.delta", json!({"item_id": format!("fc_{chat_index}"), "output_index": index, "delta": arguments}));
                }
            }
        }
        out
    }

    /// 流结束：补齐各项的 done 事件与 `response.completed`。
    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        if !self.started {
            return out;
        }
        let mut items: Vec<(usize, Value)> = Vec::new();
        if let Some(index) = self.reasoning_index {
            let item = json!({"type": "reasoning", "id": "rs_0", "summary": [{"type": "summary_text", "text": self.reasoning}]});
            self.event(&mut out, "response.output_item.done", json!({"output_index": index, "item": item.clone()}));
            items.push((index, item));
        }
        if let Some(index) = self.text_index {
            let text = self.text.clone();
            self.event(&mut out, "response.output_text.done", json!({"item_id": "msg_0", "output_index": index, "content_index": 0, "text": text}));
            let part = json!({"type": "output_text", "text": text, "annotations": []});
            self.event(&mut out, "response.content_part.done", json!({"item_id": "msg_0", "output_index": index, "content_index": 0, "part": part.clone()}));
            let item = json!({"type": "message", "id": "msg_0", "role": "assistant", "status": "completed", "content": [part]});
            self.event(&mut out, "response.output_item.done", json!({"output_index": index, "item": item.clone()}));
            items.push((index, item));
        }
        for (chat_index, index, call_id, name, arguments) in self.calls.clone() {
            let arguments = if arguments.is_empty() { "{}".to_string() } else { arguments };
            if !self.custom.iter().any(|c| *c == name) {
                self.event(&mut out, "response.function_call_arguments.done", json!({"item_id": format!("fc_{chat_index}"), "output_index": index, "arguments": arguments}));
            }
            let item = call_item(&format!("fc_{chat_index}"), &call_id, &name, &arguments, &self.custom);
            self.event(&mut out, "response.output_item.done", json!({"output_index": index, "item": item.clone()}));
            items.push((index, item));
        }
        items.sort_by_key(|(index, _)| *index);
        let completed = self.base("completed", items.into_iter().map(|(_, item)| item).collect());
        self.event(&mut out, "response.completed", json!({"response": completed}));
        self.started = false;
        out
    }
}

/// 把上游的 Chat 回复（流式或非流式）转成 Responses 回复。
pub(crate) async fn response_body(upstream: axum::response::Response, custom: Vec<String>) -> axum::response::Response {
    use axum::http::StatusCode;
    let status = upstream.status();
    if !status.is_success() {
        return upstream;
    }
    let streaming = upstream.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|ct| ct.contains("event-stream"));
    if !streaming {
        let Ok(bytes) = axum::body::to_bytes(upstream.into_body(), 32 * 1024 * 1024).await else {
            return crate::gateway::wire::error_response(StatusCode::BAD_GATEWAY, "上游回复读取失败");
        };
        let Ok(chat) = serde_json::from_slice::<Value>(&bytes) else {
            return crate::gateway::wire::error_response(StatusCode::BAD_GATEWAY, "上游回复不是 JSON，无法转成 Responses");
        };
        return axum::response::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(response(&chat, &custom).to_string()))
            .unwrap_or_else(|_| crate::gateway::wire::error_response(StatusCode::BAD_GATEWAY, "构造回复失败"));
    }
    let stream = SseTranslate { inner: upstream.into_body().into_data_stream(), line: Vec::new(), translator: StreamTranslator::new(custom), done: false };
    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(axum::body::Body::from_stream(stream))
        .unwrap_or_else(|_| crate::gateway::wire::error_response(StatusCode::BAD_GATEWAY, "构造回复失败"))
}

struct SseTranslate {
    inner: axum::body::BodyDataStream,
    line: Vec<u8>,
    translator: StreamTranslator,
    done: bool,
}

impl SseTranslate {
    fn feed(&mut self, chunk: &[u8]) -> String {
        let mut out = String::new();
        for byte in chunk {
            if *byte != b'\n' {
                self.line.push(*byte);
                continue;
            }
            let line = std::mem::take(&mut self.line);
            let text = String::from_utf8_lossy(&line);
            let Some(data) = text.trim().strip_prefix("data:").map(str::trim) else { continue };
            if data == "[DONE]" {
                out.push_str(&self.translator.finish());
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(data) {
                out.push_str(&self.translator.push(&value));
            }
        }
        out
    }
}

impl futures_util::Stream for SseTranslate {
    type Item = Result<axum::body::Bytes, axum::Error>;
    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;
        loop {
            if self.done {
                return Poll::Ready(None);
            }
            match std::pin::Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    let out = self.feed(&chunk);
                    if !out.is_empty() {
                        return Poll::Ready(Some(Ok(axum::body::Bytes::from(out))));
                    }
                }
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Some(Err(error))),
                Poll::Ready(None) => {
                    self.done = true;
                    // 上游没发 [DONE] 也要收尾
                    let out = self.translator.finish();
                    return if out.is_empty() { Poll::Ready(None) } else { Poll::Ready(Some(Ok(axum::body::Bytes::from(out)))) };
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// 这个 OpenAI 协议来源自己能不能接 Responses：OpenAI 官方与 Azure OpenAI 能，其余按只有 Chat 处理。
pub(crate) fn serves_responses(base_url: Option<&str>) -> bool {
    base_url
        .and_then(|base| url::Url::parse(base).ok())
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == "api.openai.com" || host.ends_with(".openai.azure.com"))
}

/// 调用方路径换成 Chat 的路径：`…/responses` → `…/chat/completions`，`…/messages` 同理；查询串保留。
pub(crate) fn chat_path(path_and_query: &str) -> String {
    let (path, query) = path_and_query.split_once('?').map(|(p, q)| (p, Some(q))).unwrap_or((path_and_query, None));
    let base = path.trim_end_matches('/').rsplit_once('/').map(|(head, _)| head).unwrap_or("");
    let path = format!("{base}/chat/completions");
    match query {
        Some(query) if !query.is_empty() && !query.starts_with("beta=") => format!("{path}?{query}"),
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_codex_request_with_history_and_tools_becomes_chat() {
        let body = json!({
            "model": "deepseek/deepseek-chat", "stream": true, "instructions": "你是编程助手",
            "reasoning": {"effort": "high"}, "max_output_tokens": 4096,
            "tools": [
                {"type": "function", "name": "shell", "description": "运行命令", "parameters": {"type": "object", "properties": {"cmd": {"type": "string"}}}},
                {"type": "custom", "name": "apply_patch", "description": "改文件"},
                {"type": "web_search"}
            ],
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "看一下目录"}, {"type": "input_image", "image_url": "data:image/png;base64,AAAA"}]},
                {"type": "reasoning", "encrypted_content": "x"},
                {"type": "function_call", "call_id": "c1", "name": "shell", "arguments": "{\"cmd\":\"ls\"}"},
                {"type": "custom_tool_call", "call_id": "c2", "name": "apply_patch", "input": "*** Begin Patch"},
                {"type": "function_call_output", "call_id": "c1", "output": "a.rs"},
                {"type": "custom_tool_call_output", "call_id": "c2", "output": "ok"},
                {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "好了"}]}
            ]
        });
        let chat = request(&body).unwrap();
        let messages = chat["messages"].as_array().unwrap();
        assert_eq!(messages[0], json!({"role": "system", "content": "你是编程助手"}));
        assert_eq!(messages[1]["content"][1]["image_url"]["url"], "data:image/png;base64,AAAA", "图片保留");
        assert_eq!(messages[2]["tool_calls"].as_array().unwrap().len(), 2, "连续调用并进一条 assistant 消息");
        assert_eq!(messages[2]["tool_calls"][1]["function"]["arguments"], r#"{"input":"*** Begin Patch"}"#, "自由格式工具包成 input 参数");
        assert_eq!(messages[3], json!({"role": "tool", "tool_call_id": "c1", "content": "a.rs"}));
        assert_eq!(messages[5], json!({"role": "assistant", "content": "好了"}), "纯文字内容合成字符串");
        assert_eq!(chat["tools"].as_array().unwrap().len(), 2, "内置 web_search 省略");
        assert_eq!(chat["tools"][1]["function"]["parameters"]["required"][0], "input");
        assert_eq!((chat["reasoning_effort"].as_str(), chat["max_tokens"].as_i64()), (Some("high"), Some(4096)));
        assert_eq!(chat["stream_options"]["include_usage"], true);
        assert_eq!(custom_tool_names(&body), vec!["apply_patch".to_string()]);
    }

    #[test]
    fn a_chat_reply_becomes_a_responses_object_with_custom_calls_unwrapped() {
        let chat = json!({"id": "x1", "model": "m", "created": 1, "choices": [{"message": {
            "content": "改好了", "reasoning_content": "想了想",
            "tool_calls": [
                {"id": "c1", "function": {"name": "shell", "arguments": "{\"cmd\":\"ls\"}"}},
                {"id": "c2", "function": {"name": "apply_patch", "arguments": "{\"input\":\"*** Begin Patch\"}"}}
            ]
        }}], "usage": {"prompt_tokens": 10, "completion_tokens": 5, "prompt_tokens_details": {"cached_tokens": 4}}});
        let out = response(&chat, &["apply_patch".to_string()]);
        let types: Vec<&str> = out["output"].as_array().unwrap().iter().map(|i| i["type"].as_str().unwrap()).collect();
        assert_eq!(types, ["reasoning", "message", "function_call", "custom_tool_call"]);
        assert_eq!(out["output"][3]["input"], "*** Begin Patch");
        assert_eq!(out["usage"], json!({"input_tokens": 10, "output_tokens": 5, "total_tokens": 15, "input_tokens_details": {"cached_tokens": 4}}));
    }

    #[test]
    fn streamed_chat_becomes_responses_events_ending_in_completed() {
        let mut t = StreamTranslator::new(vec![]);
        let mut all = String::new();
        all += &t.push(&json!({"id": "s", "model": "m", "choices": [{"delta": {"content": "你"}}]}));
        all += &t.push(&json!({"choices": [{"delta": {"content": "好"}}]}));
        all += &t.push(&json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "shell", "arguments": "{\"cmd\""}}]}}]}));
        all += &t.push(&json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": ":\"ls\"}"}}]}}]}));
        all += &t.push(&json!({"choices": [], "usage": {"prompt_tokens": 3, "completion_tokens": 2}}));
        all += &t.finish();
        let names: Vec<&str> = all.lines().filter_map(|l| l.strip_prefix("event: ")).collect();
        assert_eq!(names.first(), Some(&"response.created"));
        assert_eq!(names.last(), Some(&"response.completed"));
        assert_eq!(names.iter().filter(|n| **n == "response.output_text.delta").count(), 2);
        assert!(names.contains(&"response.function_call_arguments.done"));
        let completed: Value = serde_json::from_str(all.lines().filter_map(|l| l.strip_prefix("data: ")).last().unwrap()).unwrap();
        assert_eq!(completed["response"]["output"][0]["content"][0]["text"], "你好");
        assert_eq!(completed["response"]["output"][1]["arguments"], "{\"cmd\":\"ls\"}");
        assert_eq!(completed["response"]["usage"]["total_tokens"], 5);
        assert!(t.finish().is_empty(), "只收尾一次");
    }

    #[test]
    fn only_openai_hosts_take_responses_directly_and_paths_switch_to_chat() {
        assert!(serves_responses(Some("https://api.openai.com/v1")));
        assert!(serves_responses(Some("https://my.openai.azure.com")));
        assert!(!serves_responses(Some("https://api.deepseek.com")));
        assert_eq!(chat_path("/v1/responses"), "/v1/chat/completions");
        assert_eq!(chat_path("/v1/messages?beta=true"), "/v1/chat/completions", "Claude Code 的 beta 查询不带给 Chat");
        assert_eq!(chat_path("/responses"), "/chat/completions");
    }
}
