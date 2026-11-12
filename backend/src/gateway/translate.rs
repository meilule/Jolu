//! Anthropic Messages ↔ OpenAI Chat Completions，供只说 Chat 的订阅来源服务 Claude Code 等调用方。
//! 按 magpie（github.com/yetone/magpie，MIT）internal/gateway 的 readAnthropic / buildChat /
//! anthropicEncoder / renderAnthropic 移植核心路径；搜索、引用与思考签名等 magpie 专有能力不移植。
use axum::{
    body::{Body, Bytes},
    http::StatusCode,
    response::Response,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures_util::StreamExt;
use serde_json::{json, Value};

/// 调用 ID 中非 `[A-Za-z0-9_-]` 的字符：Anthropic 不接受，按 magpie 加 `mp_` 前缀后 base64url 编码。
const ID_MARK: &str = "mp_";

fn safe_chars(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn out_id(id: &str) -> String {
    if safe_chars(id) && !id.starts_with(ID_MARK) {
        return id.into();
    }
    format!("{ID_MARK}{}", URL_SAFE_NO_PAD.encode(id.as_bytes()))
}

fn in_id(id: &str) -> String {
    let Some(raw) = id.strip_prefix(ID_MARK).filter(|_| safe_chars(id)) else { return id.into() };
    match URL_SAFE_NO_PAD.decode(raw).ok().and_then(|b| String::from_utf8(b).ok()) {
        Some(raw) if !raw.is_empty() && out_id(&raw) == id => raw,
        _ => id.into(),
    }
}

fn new_id() -> String {
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seed = format!("{}:{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(), SEQ.fetch_add(1, Ordering::Relaxed));
    Sha256::digest(seed.as_bytes()).iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// system 字段：字符串或文本块数组。
fn text_of(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.into();
    }
    v.as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn image_url(source: &Value) -> Option<String> {
    if let Some(url) = source["url"].as_str().filter(|u| !u.is_empty()) {
        return Some(url.into());
    }
    let data = source["data"].as_str().filter(|d| !d.is_empty())?;
    Some(format!("data:{};base64,{data}", source["media_type"].as_str().unwrap_or("image/png")))
}

/// 工具结果：文本与其中的图片（magpie toolOutput）。
fn tool_output(content: &Value) -> (String, Vec<Value>) {
    let text = text_of(content);
    let images = content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "image")
        .filter_map(|b| image_url(&b["source"]))
        .map(|url| json!({"type":"image_url","image_url":{"url":url}}))
        .collect();
    (text, images)
}

/// magpie effortOfBudget：思考预算换成档位。
fn effort_of_budget(n: i64) -> &'static str {
    match n {
        n if n <= 0 => "",
        n if n <= 4096 => "low",
        n if n <= 12000 => "medium",
        n if n <= 24000 => "high",
        _ => "xhigh",
    }
}

fn effort_of(s: &str) -> &'static str {
    match s.trim().to_ascii_lowercase().as_str() {
        "minimal" | "none" | "low" => "low",
        "medium" => "medium",
        "high" => "high",
        "xhigh" => "xhigh",
        "max" | "ultra" => "max",
        _ => "",
    }
}

/// Anthropic Messages 请求体 → OpenAI Chat Completions 请求体。
pub(crate) fn anthropic_request(body: &[u8]) -> Result<Bytes, String> {
    let a: Value = serde_json::from_slice(body).map_err(|_| "请求 JSON 无效")?;
    let mut messages: Vec<Value> = Vec::new();
    let system = text_of(&a["system"]);
    if !system.is_empty() {
        messages.push(json!({"role":"system","content":system}));
    }
    for m in a["messages"].as_array().ok_or("请求缺少 messages")? {
        let role = m["role"].as_str().unwrap_or("user");
        if let Some(text) = m["content"].as_str() {
            if !text.is_empty() {
                messages.push(json!({"role":role,"content":text}));
            }
            continue;
        }
        let blocks = m["content"].as_array().ok_or("消息内容格式无效")?;
        let mut parts: Vec<Value> = Vec::new();
        let mut calls: Vec<Value> = Vec::new();
        let mut thinking = String::new();
        let mut results: Vec<Value> = Vec::new();
        let mut result_images: Vec<Value> = Vec::new();
        for b in blocks {
            match b["type"].as_str().unwrap_or_default() {
                "text" => {
                    if let Some(t) = b["text"].as_str() {
                        parts.push(json!({"type":"text","text":t}));
                    }
                }
                "image" => {
                    if let Some(url) = image_url(&b["source"]) {
                        parts.push(json!({"type":"image_url","image_url":{"url":url}}));
                    }
                }
                "tool_use" => calls.push(json!({
                    "id": in_id(b["id"].as_str().unwrap_or_default()),
                    "type": "function",
                    "function": {"name": b["name"], "arguments": if b["input"].is_null() { "{}".into() } else { b["input"].to_string() }},
                })),
                "tool_result" => {
                    let (mut text, images) = tool_output(&b["content"]);
                    if b["is_error"] == true && !text.is_empty() {
                        text = format!("Error: {text}");
                    }
                    results.push(json!({"role":"tool","tool_call_id":in_id(b["tool_use_id"].as_str().unwrap_or_default()),"content":text}));
                    result_images.extend(images);
                }
                "thinking" => thinking.push_str(b["thinking"].as_str().unwrap_or_default()),
                _ => {}
            }
        }
        if role == "assistant" {
            let text: String = parts.iter().filter_map(|p| p["text"].as_str()).collect();
            if text.is_empty() && calls.is_empty() {
                continue;
            }
            let mut msg = json!({"role":"assistant","content":text});
            if !calls.is_empty() {
                msg["tool_calls"] = Value::Array(calls);
            }
            if !thinking.is_empty() {
                msg["reasoning_content"] = json!(thinking);
            }
            messages.push(msg);
            continue;
        }
        // 工具结果必须紧跟在助手的 tool_calls 之后；结果里的图片与用户的话放在随后的用户消息里
        messages.extend(results);
        result_images.extend(parts);
        if !result_images.is_empty() {
            messages.push(json!({"role":"user","content":result_images}));
        }
    }
    let mut chat = json!({"model":a["model"],"messages":messages});
    for (from, to) in [("max_tokens", "max_tokens"), ("temperature", "temperature"), ("top_p", "top_p"), ("stop_sequences", "stop")] {
        if !a[from].is_null() {
            chat[to] = a[from].clone();
        }
    }
    if a["stream"] == true {
        chat["stream"] = json!(true);
        chat["stream_options"] = json!({"include_usage":true});
    }
    // 服务端工具（web_search 等）与延迟加载占位工具在别处没有意义
    let tools: Vec<Value> = a["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["input_schema"].is_object() || t["type"].as_str().is_none_or(|ty| ty.is_empty() || ty == "custom"))
        .filter(|t| t["name"] != "DeferredToolPlaceholder")
        .map(|t| json!({"type":"function","function":{"name":t["name"],"description":t["description"].as_str().unwrap_or_default(),
            "parameters":if t["input_schema"].is_object() { t["input_schema"].clone() } else { json!({"type":"object","properties":{}}) }}}))
        .collect();
    if !tools.is_empty() {
        chat["tools"] = Value::Array(tools);
    }
    match a["tool_choice"]["type"].as_str() {
        Some("auto") => chat["tool_choice"] = json!("auto"),
        Some("none") => chat["tool_choice"] = json!("none"),
        Some("any") => chat["tool_choice"] = json!("required"),
        Some("tool") => chat["tool_choice"] = json!({"type":"function","function":{"name":a["tool_choice"]["name"]}}),
        _ => {}
    }
    if a["tool_choice"]["disable_parallel_tool_use"] == true {
        chat["parallel_tool_calls"] = json!(false);
    }
    // 只有明确要求思考时才带档位：Claude Code 的标题请求带 effort 却不思考
    match a["thinking"]["type"].as_str() {
        Some("disabled" | "between_tools") => chat["reasoning_effort"] = json!("none"),
        Some("enabled" | "adaptive") => {
            let effort = Some(effort_of(a["output_config"]["effort"].as_str().unwrap_or_default()))
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| effort_of_budget(a["thinking"]["budget_tokens"].as_i64().unwrap_or(0)));
            if !effort.is_empty() {
                chat["reasoning_effort"] = json!(effort);
            }
        }
        _ => {}
    }
    Ok(Bytes::from(chat.to_string()))
}

fn stop_to_anthropic(reason: &str) -> &'static str {
    match reason {
        "length" => "max_tokens",
        "tool_calls" | "function_call" => "tool_use",
        "content_filter" => "refusal",
        _ => "end_turn",
    }
}

/// 仅转换已知数字；平铺的 Anthropic 缓存字段保持其原口径。
/// Chat 嵌套 cached_tokens 属于 prompt 的子集，只有此种明确关系才扣除。
pub(super) fn usage_of(u: &Value) -> Value {
    let count=|v:&Value|v.as_i64().filter(|n|*n>=0);
    let flat_read=count(&u["cache_read_input_tokens"]);
    let nested_read=count(&u["prompt_tokens_details"]["cached_tokens"]);
    let mut out=serde_json::Map::new();
    if let Some(prompt)=count(&u["prompt_tokens"]) {
        let prompt=if flat_read.is_none(){nested_read.map_or(prompt,|cached|prompt.saturating_sub(cached).max(0))}else{prompt};
        out.insert("input_tokens".into(),json!(prompt));
    }
    if let Some(output)=count(&u["completion_tokens"]){out.insert("output_tokens".into(),json!(output));}
    if let Some(read)=flat_read.or(nested_read){out.insert("cache_read_input_tokens".into(),json!(read));}
    if let Some(write)=count(&u["cache_creation_input_tokens"]){out.insert("cache_creation_input_tokens".into(),json!(write));}
    Value::Object(out)
}

fn message_id(id: &str) -> String {
    if id.is_empty() {
        return format!("msg_{}", new_id());
    }
    if id.starts_with("msg_") {
        return id.into();
    }
    format!("msg_{}", id.trim_start_matches("chatcmpl-"))
}

/// 非流式 chat.completion → Anthropic message。
fn render(chat: &Value, model: &str) -> Value {
    let msg = &chat["choices"][0]["message"];
    let mut content: Vec<Value> = Vec::new();
    if let Some(r) = msg["reasoning_content"].as_str().filter(|r| !r.is_empty()) {
        content.push(json!({"type":"thinking","thinking":r,"signature":""}));
    }
    if let Some(t) = msg["content"].as_str().filter(|t| !t.is_empty()) {
        content.push(json!({"type":"text","text":t}));
    }
    for c in msg["tool_calls"].as_array().into_iter().flatten() {
        let args = c["function"]["arguments"].as_str().unwrap_or("{}");
        let input: Value = serde_json::from_str(args).ok().filter(Value::is_object).unwrap_or_else(|| json!({}));
        let id = c["id"].as_str().filter(|s| !s.is_empty()).map(out_id).unwrap_or_else(|| format!("toolu_{}", new_id()));
        content.push(json!({"type":"tool_use","id":id,"name":c["function"]["name"],"input":input}));
    }
    json!({"id":message_id(chat["id"].as_str().unwrap_or_default()),"type":"message","role":"assistant","model":model,"content":content,
        "stop_reason":stop_to_anthropic(chat["choices"][0]["finish_reason"].as_str().unwrap_or("stop")),"stop_sequence":null,
        "usage":usage_of(&chat["usage"])})
}

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Text,
    Thinking,
    Tool,
}

/// Chat SSE → Anthropic 事件流（magpie anthropicEncoder 的核心路径）。
struct Encoder {
    model: String,
    started: bool,
    index: usize,
    open: Option<Kind>,
    args: bool,
    tool: Option<i64>,
    stop: String,
    usage: Value,
    done: bool,
    buf: Vec<u8>,
}

fn event(out: &mut Vec<u8>, name: &str, data: Value) {
    out.extend_from_slice(format!("event: {name}\ndata: {data}\n\n").as_bytes());
}

impl Encoder {
    fn new(model: &str) -> Self {
        Self { model: model.into(), started: false, index: 0, open: None, args: false, tool: None, stop: "stop".into(), usage: json!({}), done: false, buf: Vec::new() }
    }

    fn start(&mut self, out: &mut Vec<u8>, id: &str) {
        if self.started {
            return;
        }
        self.started = true;
        event(out, "message_start", json!({"type":"message_start","message":{"id":message_id(id),"type":"message","role":"assistant","model":self.model,
            "content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0}}}));
    }

    fn close(&mut self, out: &mut Vec<u8>) {
        let Some(kind) = self.open.take() else { return };
        if kind == Kind::Tool && !self.args {
            event(out, "content_block_delta", json!({"type":"content_block_delta","index":self.index,"delta":{"type":"input_json_delta","partial_json":"{}"}}));
        }
        event(out, "content_block_stop", json!({"type":"content_block_stop","index":self.index}));
        self.index += 1;
        self.tool = None;
    }

    fn open_block(&mut self, out: &mut Vec<u8>, kind: Kind, block: Value) {
        if self.open == Some(kind) && kind != Kind::Tool {
            return;
        }
        self.close(out);
        self.args = false;
        event(out, "content_block_start", json!({"type":"content_block_start","index":self.index,"content_block":block}));
        self.open = Some(kind);
    }

    fn delta(&self, out: &mut Vec<u8>, delta: Value) {
        event(out, "content_block_delta", json!({"type":"content_block_delta","index":self.index,"delta":delta}));
    }

    fn chunk(&mut self, out: &mut Vec<u8>, c: &Value) {
        if let Some(err) = c.get("error").filter(|e| !e.is_null()) {
            self.start(out, "");
            self.close(out);
            let message = err["message"].as_str().map(str::to_owned).unwrap_or_else(|| err.to_string());
            event(out, "error", json!({"type":"error","error":{"type":"api_error","message":message}}));
            self.done = true;
            return;
        }
        self.start(out, c["id"].as_str().unwrap_or_default());
        let delta = &c["choices"][0]["delta"];
        if let Some(r) = delta["reasoning_content"].as_str().filter(|r| !r.is_empty()) {
            self.open_block(out, Kind::Thinking, json!({"type":"thinking","thinking":"","signature":""}));
            self.delta(out, json!({"type":"thinking_delta","thinking":r}));
        }
        if let Some(t) = delta["content"].as_str().filter(|t| !t.is_empty()) {
            self.open_block(out, Kind::Text, json!({"type":"text","text":""}));
            self.delta(out, json!({"type":"text_delta","text":t}));
        }
        for tc in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = tc["index"].as_i64().unwrap_or(0);
            let name = tc["function"]["name"].as_str().unwrap_or_default();
            if !name.is_empty() && self.tool != Some(index) {
                let id = tc["id"].as_str().filter(|s| !s.is_empty()).map(out_id).unwrap_or_else(|| format!("toolu_{}", new_id()));
                self.open_block(out, Kind::Tool, json!({"type":"tool_use","id":id,"name":name,"input":{}}));
                self.tool = Some(index);
            }
            if let Some(args) = tc["function"]["arguments"].as_str().filter(|a| !a.is_empty()) {
                if self.open == Some(Kind::Tool) && self.tool == Some(index) {
                    self.args = true;
                    self.delta(out, json!({"type":"input_json_delta","partial_json":args}));
                }
            }
        }
        if let Some(f) = c["choices"][0]["finish_reason"].as_str().filter(|f| !f.is_empty()) {
            self.stop = f.into();
        }
        if c["usage"].is_object() {
            self.usage = usage_of(&c["usage"]);
        }
    }

    fn finish(&mut self, out: &mut Vec<u8>) {
        if self.done {
            return;
        }
        self.done = true;
        self.start(out, "");
        self.close(out);
        let usage = if self.usage.as_object().is_some_and(|u| !u.is_empty()) { self.usage.clone() } else { json!({"output_tokens":0}) };
        event(out, "message_delta", json!({"type":"message_delta","delta":{"stop_reason":stop_to_anthropic(&self.stop),"stop_sequence":null},"usage":usage}));
        event(out, "message_stop", json!({"type":"message_stop"}));
    }

    fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        self.buf.extend_from_slice(bytes);
        while let Some(n) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=n).collect();
            if self.done {
                continue;
            }
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else { continue };
            if data == "[DONE]" {
                self.finish(&mut out);
                continue;
            }
            if let Ok(chunk) = serde_json::from_str::<Value>(data) {
                self.chunk(&mut out, &chunk);
            }
        }
        out
    }
}

fn error_body(status: StatusCode, message: &str) -> Response {
    let kind = match status.as_u16() {
        400 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        429 => "rate_limit_error",
        503 | 529 => "overloaded_error",
        _ => "api_error",
    };
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(json!({"type":"error","error":{"type":kind,"message":message}}).to_string()))
        .unwrap()
}

/// 把只说 Chat 的来源的回复转回 Anthropic Messages：错误、JSON 与流式三种形态。
pub(crate) async fn anthropic_response(response: Response, model: &str) -> Response {    let (parts, body) = response.into_parts();
    let is_stream = parts.headers.get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/event-stream"));
    if !parts.status.is_success() {
        let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap_or_default();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let message = value["error"]["message"].as_str().map(str::to_owned).unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
        let mut out = error_body(parts.status, &message);
        for name in ["retry-after", "x-jolu-gateway"] {
            if let Some(v) = parts.headers.get(name) {
                out.headers_mut().insert(name, v.clone());
            }
        }
        return out;
    }
    if !is_stream {
        let bytes = match axum::body::to_bytes(body, 64 << 20).await {
            Ok(b) => b,
            Err(_) => return error_body(StatusCode::BAD_GATEWAY, "上游回复读取失败"),
        };
        let Ok(chat) = serde_json::from_slice::<Value>(&bytes) else {
            return error_body(StatusCode::BAD_GATEWAY, "上游回复不是有效的 Chat Completions JSON");
        };
        let mut out = Response::builder().status(StatusCode::OK).header("content-type", "application/json");
        if let Some(v) = parts.headers.get("x-jolu-gateway") {
            out = out.header("x-jolu-gateway", v.clone());
        }
        return out.body(Body::from(render(&chat, model).to_string())).unwrap();
    }
    let state = (body.into_data_stream(), Encoder::new(model), false);
    let stream = futures_util::stream::unfold(state, |(mut upstream, mut encoder, ended)| async move {
        if ended {
            return None;
        }
        loop {
            match upstream.next().await {
                Some(Ok(bytes)) => {
                    let out = encoder.feed(&bytes);
                    if !out.is_empty() {
                        let done = encoder.done;
                        return Some((Ok::<Bytes, std::convert::Infallible>(Bytes::from(out)), (upstream, encoder, done)));
                    }
                }
                _ => {
                    let mut out = encoder.feed(b"\n");
                    if !encoder.done {
                        encoder.start(&mut out, "");
                        encoder.close(&mut out);
                        event(&mut out, "error", json!({"type":"error","error":{"type":"api_error","message":"上游回复在完成标记前中断，未作为完整回答"}}));
                        encoder.done = true;
                    }
                    return Some((Ok(Bytes::from(out)), (upstream, encoder, true)));
                }
            }
        }
    });
    let mut out = Response::builder().status(StatusCode::OK).header("content-type", "text/event-stream").header("cache-control", "no-cache");
    if let Some(v) = parts.headers.get("x-jolu-gateway") {
        out = out.header("x-jolu-gateway", v.clone());
    }
    out.body(Body::from_stream(stream)).unwrap()
}

/// Anthropic 上游的消息端点：Chat / Responses 调用方的原路径（/v1/chat/completions、
/// /v1/responses）对 anthropic 上游没有意义，统一改写到 Messages API。
pub(crate) fn messages_path() -> &'static str {
    "/v1/messages"
}

/// Chat 消息体（字符串或数组）的文本聚合。
fn chat_text(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.into();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Chat 的 `image_url` → Anthropic image 块。Anthropic 不接受 http 引用，只收 base64：
/// data: URL 解回 base64；http(s) 的跳过（Anthropic API 无法直接取远端图）。
fn chat_image(source: &Value) -> Option<Value> {
    let url = source["image_url"]["url"].as_str().or_else(|| source["url"].as_str())?;
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let media = meta.strip_prefix("image/").unwrap_or("png");
    if data.is_empty() {
        return None;
    }
    Some(json!({"type":"image","source":{"type":"base64","media_type":format!("image/{media}"),"data":data}}))
}

fn effort_budget(effort: &str) -> i64 {
    // effort_of_budget 的近似反查：这些只是提示值，Anthropic 只要求 budget ≥ 1024 且 < max_tokens
    match effort {
        "low" => 2048,
        "medium" => 8192,
        "high" => 16384,
        _ => 32000,
    }
}

/// Chat Completions 请求体 → Anthropic Messages 请求体。
///
/// 给说 Chat 的调用方（第三方工具、局域网共享的客户端）服务只提供 Anthropic 端点的
/// API 来源。要点：
/// - system 消息合并进 Anthropic 的顶层 `system`；
/// - `role:"tool"` 聚成一条 user 消息里的多个 tool_result（Anthropic 要求 tool_result
///   紧跟对应的 tool_use，且必须住在 user 消息里）；
/// - `max_tokens` 是 Anthropic 必填项：Chat 请求没带时按 8192 兜底；
/// - `reasoning_effort` 映射成 thinking + budget，并保证 max_tokens 大于 budget；
/// - 图片只接受 data: URL（base64），http 引用跳过——Anthropic API 不取远端图；
/// - 助手消息里的 `reasoning_content`（DeepSeek 风格）丢弃：Anthropic 的 thinking 块
///   需要官方签名，伪造的空签名块会被拒绝。
pub(crate) fn chat_to_anthropic_request(body: &[u8]) -> Result<Bytes, String> {
    let c: Value = serde_json::from_slice(body).map_err(|_| "请求 JSON 无效")?;
    let mut system_parts: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    // Chat 的 tool 消息聚合：Anthropic 的 tool_result 必须在一条 user 消息里
    let mut pending: Vec<Value> = Vec::new();
    let flush = |messages: &mut Vec<Value>, pending: &mut Vec<Value>| {
        if !pending.is_empty() {
            messages.push(json!({"role":"user","content":std::mem::take(pending)}));
        }
    };
    for m in c["messages"].as_array().ok_or("请求缺少 messages")? {
        let role = m["role"].as_str().unwrap_or("user").to_ascii_lowercase();
        match role.as_str() {
            "system" | "developer" => system_parts.push(chat_text(&m["content"])),
            "tool" => {
                let content = m["content"].as_str().map(Value::from).unwrap_or_else(|| m["content"].clone());
                pending.push(json!({"type":"tool_result","tool_use_id":out_id(m["tool_call_id"].as_str().unwrap_or_default()),"content":content}));
            }
            "assistant" => {
                flush(&mut messages, &mut pending);
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(text) = m["content"].as_str().filter(|t| !t.is_empty()) {
                    blocks.push(json!({"type":"text","text":text}));
                } else if let Some(parts) = m["content"].as_array() {
                    for p in parts {
                        if p["type"].as_str() == Some("text") {
                            if let Some(t) = p["text"].as_str().filter(|t| !t.is_empty()) {
                                blocks.push(json!({"type":"text","text":t}));
                            }
                        }
                    }
                }
                for call in m["tool_calls"].as_array().into_iter().flatten() {
                    let args = call["function"]["arguments"].as_str().unwrap_or("{}");
                    let input: Value = serde_json::from_str(args).ok().filter(Value::is_object).unwrap_or_else(|| json!({}));
                    blocks.push(json!({"type":"tool_use","id":out_id(call["id"].as_str().unwrap_or_default()),"name":call["function"]["name"],"input":input}));
                }
                if blocks.is_empty() {
                    continue;
                }
                messages.push(json!({"role":"assistant","content":blocks}));
            }
            _ => {
                flush(&mut messages, &mut pending);
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(text) = m["content"].as_str().filter(|t| !t.is_empty()) {
                    blocks.push(json!({"type":"text","text":text}));
                } else if let Some(parts) = m["content"].as_array() {
                    for p in parts {
                        match p["type"].as_str().unwrap_or_default() {
                            "text" => {
                                if let Some(t) = p["text"].as_str().filter(|t| !t.is_empty()) {
                                    blocks.push(json!({"type":"text","text":t}));
                                }
                            }
                            "image_url" => {
                                if let Some(image) = chat_image(p) {
                                    blocks.push(image);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if blocks.is_empty() {
                    continue;
                }
                messages.push(json!({"role":"user","content":blocks}));
            }
        }
    }
    flush(&mut messages, &mut pending);
    let mut out = json!({"model":c["model"],"messages":messages});
    if !system_parts.is_empty() {
        out["system"] = json!(system_parts.join("\n\n"));
    }
    // max_tokens 是 Anthropic 必填：没带就按 8192 兜底（大窗口模型可以更高，但保守值最普适）
    let max_tokens = c["max_tokens"].as_i64().filter(|n| *n > 0).unwrap_or(8192);
    out["max_tokens"] = json!(max_tokens);
    for (from, to) in [("temperature", "temperature"), ("top_p", "top_p")] {
        if !c[from].is_null() {
            out[to] = c[from].clone();
        }
    }
    if let Some(stop) = c["stop"].as_str().filter(|s| !s.is_empty()) {
        out["stop_sequences"] = json!([stop]);
    } else if let Some(stops) = c["stop"].as_array().filter(|s| !s.is_empty()) {
        out["stop_sequences"] = json!(stops.iter().filter_map(|s| s.as_str()).collect::<Vec<_>>());
    }
    let tools: Vec<Value> = c["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["type"].as_str() == Some("function") || t["function"].is_object())
        .filter_map(|t| {
            let function = &t["function"];
            let name = function["name"].as_str()?;
            Some(json!({"name":name,"description":function["description"].as_str().unwrap_or_default(),
                "input_schema":if function["parameters"].is_object() { function["parameters"].clone() } else { json!({"type":"object","properties":{}}) }}))
        })
        .collect();
    if !tools.is_empty() {
        out["tools"] = Value::Array(tools);
    }
    match c["tool_choice"].as_str() {
        Some("auto") => out["tool_choice"] = json!({"type":"auto"}),
        Some("none") => out["tool_choice"] = json!({"type":"none"}),
        Some("required") => out["tool_choice"] = json!({"type":"any"}),
        _ => {}
    }
    if c["tool_choice"]["type"].as_str() == Some("function") {
        if let Some(name) = c["tool_choice"]["function"]["name"].as_str() {
            out["tool_choice"] = json!({"type":"tool","name":name});
        }
    }
    if c["stream"] == true {
        out["stream"] = json!(true);
    }
    // reasoning_effort → thinking（budget 是近似值：Anthropic 只要求 ≥1024 且 < max_tokens）
    let effort = c["reasoning_effort"].as_str().map(effort_of).filter(|e| !e.is_empty());
    if let Some(effort) = effort {
        let budget = effort_budget(effort);
        out["thinking"] = json!({"type":"enabled","budget_tokens":budget});
        if max_tokens <= budget {
            out["max_tokens"] = json!(budget + 1024);
        }
    }
    Ok(Bytes::from(out.to_string()))
}

fn chat_finish_reason(stop: &str) -> &'static str {
    match stop {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        _ => "stop",
    }
}

/// Anthropic 用量 → Chat 用量（`usage_of` 的反方向）。
fn chat_usage(u: &Value) -> Value {
    let count = |v: &Value| v.as_i64().filter(|n| *n >= 0);
    let mut out = serde_json::Map::new();
    if let Some(prompt) = count(&u["input_tokens"]) {
        out.insert("prompt_tokens".into(), json!(prompt));
    }
    if let Some(output) = count(&u["output_tokens"]) {
        out.insert("completion_tokens".into(), json!(output));
    }
    if let Some(read) = count(&u["cache_read_input_tokens"]) {
        out.insert("prompt_tokens_details".into(), json!({"cached_tokens":read}));
    }
    Value::Object(out)
}

/// 非流式 Anthropic message → chat.completion。
fn render_chat(message: &Value, model: &str) -> Value {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut calls: Vec<Value> = Vec::new();
    for block in message["content"].as_array().into_iter().flatten() {
        match block["type"].as_str().unwrap_or_default() {
            "text" => content.push_str(block["text"].as_str().unwrap_or_default()),
            "thinking" => reasoning.push_str(block["thinking"].as_str().unwrap_or_default()),
            "tool_use" => {
                let id = block["id"].as_str().filter(|s| !s.is_empty()).map(out_id).unwrap_or_else(|| format!("call_{}", new_id()));
                calls.push(json!({"id":id,"type":"function","function":{"name":block["name"],"arguments":block["input"].to_string()}}));
            }
            _ => {}
        }
    }
    let mut msg = json!({"role":"assistant","content":content});
    if !reasoning.is_empty() {
        msg["reasoning_content"] = json!(reasoning);
    }
    if !calls.is_empty() {
        msg["tool_calls"] = Value::Array(calls);
    }
    let usage = chat_usage(&message["usage"]);
    json!({"id":format!("chatcmpl-{}", message["id"].as_str().unwrap_or_default().trim_start_matches("msg_")),"object":"chat.completion",
        "created":chrono::Utc::now().timestamp(),"model":model,
        "choices":[{"index":0,"message":msg,"finish_reason":chat_finish_reason(message["stop_reason"].as_str().unwrap_or("end_turn"))}],
        "usage":usage})
}

/// Anthropic 事件流 → Chat chunk 流（`Encoder` 的反方向）。
/// 事件按 magpie 官方客户端的序列：message_start → (content_block_start / _delta / _stop)* →
/// message_delta（stop_reason + usage）→ message_stop。
struct ChatDecoder {
    /// 回给调用方的 model（message_start 里的为准，缺省用请求里的）
    model: String,
    tool_seq: i64,
    /// 当前打开的内容块类型：text / thinking / tool_use
    open: Option<&'static str>,
    done: bool,
    buf: Vec<u8>,
}

impl ChatDecoder {
    fn new(model: &str) -> Self {
        Self { model: model.into(), tool_seq: 0, open: None, done: false, buf: Vec::new() }
    }

    fn chunk(out: &mut Vec<u8>, delta: Value, finish: Option<&str>, usage: Option<&Value>) {
        let mut choice = json!({"index":0,"delta":delta});
        if let Some(finish) = finish {
            choice["finish_reason"] = json!(finish);
        }
        let mut body = json!({"object":"chat.completion.chunk","choices":[choice]});
        if let Some(usage) = usage {
            body["usage"] = usage.clone();
        }
        out.extend_from_slice(format!("data: {body}\n\n").as_bytes());
    }

    /// 处理一条 Anthropic SSE 事件；产出 0..n 条 Chat chunk 行。
    fn event(&mut self, out: &mut Vec<u8>, name: &str, data: &Value) {
        if self.done {
            return;
        }
        match name {
            "message_start" => {
                if let Some(m) = data["message"].as_object() {
                    // Map 的 [] 索引在键缺失时会 panic，get 才是「缺了就 None」
                    if let Some(id) = m.get("id").and_then(Value::as_str) {
                        self.model = format!("chatcmpl-{}", id.trim_start_matches("msg_"));
                    }
                    if let Some(model) = m.get("model").and_then(Value::as_str) {
                        self.model = model.into();
                    }
                }
                Self::chunk(out, json!({"role":"assistant","content":""}), None, None);
            }
            "content_block_start" => {
                let index = data["index"].as_i64().unwrap_or(0);
                let block_type = data["content_block"]["type"].as_str().unwrap_or("text").to_owned();
                if block_type == "tool_use" {
                    let id = data["content_block"]["id"].as_str().filter(|s| !s.is_empty()).map(out_id).unwrap_or_else(|| format!("call_{}", new_id()));
                    Self::chunk(out, json!({"tool_calls":[{"index":self.tool_seq,"id":id,"type":"function",
                        "function":{"name":data["content_block"]["name"],"arguments":""}}]}), None, None);
                    self.tool_seq += 1;
                }
                self.open = Some(match block_type.as_str() { "thinking" => "thinking", "tool_use" => "tool_use", _ => "text" });
                let _ = index;
            }
            "content_block_delta" => match data["delta"]["type"].as_str().unwrap_or_default() {
                "text_delta" => {
                    if let Some(t) = data["delta"]["text"].as_str() {
                        Self::chunk(out, json!({"content":t}), None, None);
                    }
                }
                "thinking_delta" => {
                    if let Some(t) = data["delta"]["thinking"].as_str() {
                        Self::chunk(out, json!({"reasoning_content":t}), None, None);
                    }
                }
                "input_json_delta" if self.open == Some("tool_use") => {
                    if let Some(args) = data["delta"]["partial_json"].as_str() {
                        Self::chunk(out, json!({"tool_calls":[{"index":self.tool_seq.saturating_sub(1),"function":{"arguments":args}}]}), None, None);
                    }
                }
                _ => {}
            },
            "content_block_stop" => self.open = None,
            "message_delta" => {
                let stop = data["delta"]["stop_reason"].as_str().unwrap_or("end_turn");
                let usage = data["usage"].as_object().filter(|u| !u.is_empty()).map(|u| chat_usage(&Value::Object(u.clone())));
                Self::chunk(out, json!({}), Some(chat_finish_reason(stop)), usage.as_ref());
            }
            "message_stop" => {
                out.extend_from_slice(b"data: [DONE]\n\n");
                self.done = true;
            }
            "error" => {
                let message = data["error"]["message"].as_str().map(str::to_owned).unwrap_or_else(|| data.to_string());
                out.extend_from_slice(format!("data: {}\n\n", json!({"error":{"message":message,"type":"api_error"}})).as_bytes());
                out.extend_from_slice(b"data: [DONE]\n\n");
                self.done = true;
            }
            _ => {}
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        self.buf.extend_from_slice(bytes);
        while let Some(n) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=n).collect();
            let line = String::from_utf8_lossy(&line);
            let trimmed = line.trim_end_matches(['\r', '\n']).trim().to_string();
            if trimmed.is_empty() {
                continue;
            }
            if self.done {
                continue;
            }
            if let Some(data) = trimmed.strip_prefix("data:").map(str::trim) {
                if let Ok(value) = serde_json::from_str::<Value>(data) {
                    let name = value["type"].as_str().unwrap_or_default().to_owned();
                    self.event(&mut out, &name, &value);
                }
                continue;
            }
            // event: 行只是名字的另一种来源；data.type 为准，这里不再记录
            let _ = trimmed.strip_prefix("event:");
        }
        out
    }
}

/// 把 Anthropic 上游的回复转回 Chat 形态：错误、JSON 与流式三种形态。
/// 「Chat / Responses 调用方 → anthropic 协议的 API 来源」的回程。
pub(crate) async fn chat_response(response: Response, model: &str) -> Response {
    let (parts, body) = response.into_parts();
    let is_stream = parts.headers.get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/event-stream"));
    let mut keep = Response::builder().status(parts.status);
    for name in ["retry-after", "x-jolu-gateway"] {
        if let Some(v) = parts.headers.get(name) {
            keep = keep.header(name, v.clone());
        }
    }
    if !parts.status.is_success() {
        let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap_or_default();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let message = value["error"]["message"].as_str().map(str::to_owned).unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
        return keep
            .header("content-type", "application/json")
            .body(Body::from(json!({"error":{"message":message,"type":"api_error"}}).to_string()))
            .unwrap();
    }
    if !is_stream {
        let bytes = match axum::body::to_bytes(body, 64 << 20).await {
            Ok(b) => b,
            Err(_) => return error_body(StatusCode::BAD_GATEWAY, "上游回复读取失败"),
        };
        let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
            return error_body(StatusCode::BAD_GATEWAY, "上游回复不是有效的 Anthropic Messages JSON");
        };
        return keep
            .header("content-type", "application/json")
            .body(Body::from(render_chat(&message, model).to_string()))
            .unwrap();
    }
    let state = (body.into_data_stream(), ChatDecoder::new(model), false, Vec::new());
    let stream = futures_util::stream::unfold(state, |(mut upstream, mut decoder, ended, mut carry)| async move {
        if ended {
            return None;
        }
        loop {
            // 先冲掉上轮遗留的编码输出（feed 可能积攒多事件）
            if !carry.is_empty() {
                let done = decoder.done;
                let out = std::mem::take(&mut carry);
                return Some((Ok::<Bytes, std::convert::Infallible>(Bytes::from(out)), (upstream, decoder, done, carry)));
            }
            match upstream.next().await {
                Some(Ok(bytes)) => {
                    carry = decoder.feed(&bytes);
                    if !carry.is_empty() {
                        continue;
                    }
                }
                _ => {
                    // 上游在完成标记前中断：如实收尾，不发 [DONE]（未完成的回答）
                    let mut out = decoder.feed(b"\n");
                    if !decoder.done {
                        out.extend_from_slice(format!("data: {}\n\n", json!({"error":{"message":"上游回复在完成标记前中断，未作为完整回答","type":"api_error"}})).as_bytes());
                        decoder.done = true;
                    }
                    return Some((Ok(Bytes::from(out)), (upstream, decoder, true, carry)));
                }
            }
        }
    });
    keep.header("content-type", "text/event-stream").header("cache-control", "no-cache").body(Body::from_stream(stream)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_request_becomes_chat() {
        let body = json!({
            "model":"src/gmodel","max_tokens":2048,"stream":true,
            "system":[{"type":"text","text":"be brief"}],
            "thinking":{"type":"enabled","budget_tokens":10000},
            "tools":[{"name":"read","description":"r","input_schema":{"type":"object"}},{"type":"web_search_20250305","name":"web_search"}],
            "tool_choice":{"type":"any"},
            "messages":[
                {"role":"user","content":"hi"},
                {"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":"s"},{"type":"text","text":"reading"},{"type":"tool_use","id":"toolu_1","name":"read","input":{"path":"a"}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"ok"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AA"}}]},{"type":"text","text":"go on"}]}
            ]});
        let chat: Value = serde_json::from_slice(&anthropic_request(body.to_string().as_bytes()).unwrap()).unwrap();
        let msgs = chat["messages"].as_array().unwrap();
        assert_eq!(msgs[0], json!({"role":"system","content":"be brief"}));
        assert_eq!(msgs[2]["tool_calls"][0]["function"]["arguments"], r#"{"path":"a"}"#);
        assert_eq!(msgs[2]["reasoning_content"], "t");
        assert_eq!(msgs[3], json!({"role":"tool","tool_call_id":"toolu_1","content":"ok"}));
        assert_eq!(msgs[4]["content"][0]["image_url"]["url"], "data:image/png;base64,AA");
        assert_eq!(msgs[4]["content"][1]["text"], "go on");
        assert_eq!(chat["tools"].as_array().unwrap().len(), 1, "服务端工具不转发");
        assert_eq!(chat["tool_choice"], "required");
        assert_eq!(chat["reasoning_effort"], "medium");
        assert_eq!(chat["stream_options"]["include_usage"], true);
    }

    #[test]
    fn flat_cache_counts_survive_messages_json_and_stream_without_inventing_unknowns() {
        let usage=json!({"prompt_tokens":100,"completion_tokens":1,"cache_read_input_tokens":80,"cache_creation_input_tokens":5});
        let chat=json!({"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":usage});
        let output=render(&chat,"source/model");
        assert_eq!(output["usage"]["cache_read_input_tokens"],80);
        assert_eq!(output["usage"]["cache_creation_input_tokens"],5);
        let mut encoder=Encoder::new("source/model");
        let chunk=json!({"choices":[{"delta":{},"finish_reason":"stop"}],"usage":usage});
        let mut output=encoder.feed(format!("data: {chunk}\n\n").as_bytes());output.extend(encoder.feed(b"data: [DONE]\n\n"));
        let text=String::from_utf8(output).unwrap();
        assert!(text.contains("\"cache_read_input_tokens\":80"));assert!(text.contains("\"cache_creation_input_tokens\":5"));
        let unknown=usage_of(&json!({"completion_tokens":0}));
        assert_eq!(unknown,json!({"output_tokens":0}));
    }
    #[tokio::test]
    async fn truncated_chat_stream_never_becomes_successful_messages_stop() {
        let response = Response::builder().header("content-type", "text/event-stream")
            .body(Body::from("data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n")).unwrap();
        let response = anthropic_response(response, "source/model").await;
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("event: error"));
        assert!(!text.contains("event: message_stop"));
        assert!(!text.contains("\"stop_reason\":\"end_turn\""));
    }

    /// 3.3 补全：Chat 调用方 → anthropic 上游的请求转换
    #[test]
    fn chat_request_becomes_anthropic_messages() {
        let body = json!({
            "model":"src/gmodel","max_tokens":4096,"stream":true,"temperature":0.7,
            "messages":[
                {"role":"system","content":"be brief"},
                {"role":"user","content":"hi"},
                {"role":"assistant","content":"reading","tool_calls":[{"id":"call:1/2","type":"function","function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]},
                {"role":"tool","tool_call_id":"call:1/2","content":"ok"},
                {"role":"user","content":[{"type":"text","text":"go on"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AA"}}]}
            ],
            "tools":[{"type":"function","function":{"name":"read","description":"r","parameters":{"type":"object"}}}],
            "tool_choice":"required"});
        let a: Value = serde_json::from_slice(&chat_to_anthropic_request(body.to_string().as_bytes()).unwrap()).unwrap();
        assert_eq!(a["system"], "be brief");
        assert_eq!(a["max_tokens"], 4096);
        let msgs = a["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        let tool_use = &msgs[1]["content"][1];
        assert_eq!(tool_use["type"], "tool_use");
        assert!(safe_chars(tool_use["id"].as_str().unwrap()), "非法 id 要重编码");
        // tool 消息聚成一条 user 消息的 tool_result，紧跟 assistant
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(msgs[2]["content"][0]["tool_use_id"], tool_use["id"]);
        // 随后的用户消息（文本 + data: 图片）
        assert_eq!(msgs[3]["content"][0]["text"], "go on");
        assert_eq!(msgs[3]["content"][1]["source"]["type"], "base64");
        assert_eq!(a["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(a["tool_choice"]["type"], "any");
        assert_eq!(a["stream"], true);
    }

    #[test]
    fn chat_request_without_max_tokens_gets_anthropic_required_default() {
        let body = json!({"model":"m","messages":[{"role":"user","content":"hi"}]});
        let a: Value = serde_json::from_slice(&chat_to_anthropic_request(body.to_string().as_bytes()).unwrap()).unwrap();
        assert!(a["max_tokens"].as_i64().unwrap() > 0, "Anthropic 必填 max_tokens，不能缺");
    }

    #[test]
    fn reasoning_effort_becomes_thinking_with_larger_max_tokens() {
        let body = json!({"model":"m","messages":[{"role":"user","content":"hi"}],"reasoning_effort":"high"});
        let a: Value = serde_json::from_slice(&chat_to_anthropic_request(body.to_string().as_bytes()).unwrap()).unwrap();
        assert_eq!(a["thinking"]["type"], "enabled");
        assert!(a["max_tokens"].as_i64().unwrap() > a["thinking"]["budget_tokens"].as_i64().unwrap(), "max_tokens 必须大于 budget");
    }

    #[test]
    fn anthropic_message_renders_chat_completion() {
        let message = json!({"id":"msg_x","type":"message","role":"assistant","model":"src/gmodel",
            "content":[{"type":"thinking","thinking":"r"},{"type":"text","text":"done"},
                {"type":"tool_use","id":"toolu_1","name":"read","input":{"a":1}}],
            "stop_reason":"tool_use","usage":{"input_tokens":10,"output_tokens":3,"cache_read_input_tokens":4}});
        let chat = render_chat(&message, "src/gmodel");
        assert_eq!(chat["id"], "chatcmpl-x");
        assert_eq!(chat["choices"][0]["message"]["content"], "done");
        assert_eq!(chat["choices"][0]["message"]["reasoning_content"], "r");
        assert_eq!(chat["choices"][0]["message"]["tool_calls"][0]["function"]["name"], "read");
        assert_eq!(chat["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(chat["usage"]["prompt_tokens"], 10);
        assert_eq!(chat["usage"]["prompt_tokens_details"]["cached_tokens"], 4);
    }

    #[tokio::test]
    async fn anthropic_message_json_becomes_chat_completion_response() {
        let message = json!({"id":"msg_1","type":"message","role":"assistant","model":"m","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}});
        let response = Response::builder().header("content-type", "application/json").body(Body::from(message.to_string())).unwrap();
        let response = chat_response(response, "m").await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
        let chat: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(chat["choices"][0]["message"]["content"], "ok");
        assert_eq!(chat["usage"]["completion_tokens"], 1);
    }

    /// 3.3 补全：Anthropic 事件流 → Chat chunk 流（含工具调用与 [DONE]）
    #[tokio::test]
    async fn anthropic_stream_decodes_chat_chunks() {
        let events = [
            ("message_start", json!({"type":"message_start","message":{"id":"msg_7","role":"assistant","model":"m","content":[]}})),
            ("content_block_start", json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}})),
            ("content_block_delta", json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"好"}})),
            ("content_block_stop", json!({"type":"content_block_stop","index":0})),
            ("content_block_start", json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"read"}})),
            ("content_block_delta", json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"a\""}})),
            ("content_block_delta", json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":":1}"}})),
            ("content_block_stop", json!({"type":"content_block_stop","index":1})),
            ("message_delta", json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}})),
            ("message_stop", json!({"type":"message_stop"})),
        ];
        let mut sse = String::new();
        for (name, data) in events {
            sse.push_str(&format!("event: {name}\ndata: {data}\n\n"));
        }
        let response = Response::builder().header("content-type", "text/event-stream").body(Body::from(sse)).unwrap();
        let response = chat_response(response, "m").await;
        assert!(response.headers().get("content-type").unwrap().to_str().unwrap().starts_with("text/event-stream"));
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let datas: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("data: ")).collect();
        assert_eq!(datas.last().copied(), Some("[DONE]"), "以 [DONE] 收尾");
        let chunks: Vec<Value> = datas[..datas.len() - 1].iter().map(|d| serde_json::from_str(d).unwrap()).collect();
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
        assert_eq!(chunks[1]["choices"][0]["delta"]["content"], "好");
        let tool_start = chunks.iter().find(|c| c["choices"][0]["delta"]["tool_calls"].as_array().is_some_and(|t| !t[0]["id"].as_str().unwrap_or_default().is_empty())).unwrap();
        assert_eq!(tool_start["choices"][0]["delta"]["tool_calls"][0]["function"]["name"], "read");
        let args: String = chunks.iter().filter_map(|c| c["choices"][0]["delta"]["tool_calls"].as_array()).flatten()
            .filter_map(|t| t["function"]["arguments"].as_str()).collect();
        assert_eq!(args, "{\"a\":1}", "分片的参数要拼回完整 JSON");
        let finish = chunks.iter().find(|c| c["choices"][0]["finish_reason"].is_string()).unwrap();
        assert_eq!(finish["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(finish["usage"]["completion_tokens"], 5);
    }

    #[tokio::test]
    async fn truncated_anthropic_stream_never_fakes_chat_completion() {
        let response = Response::builder().header("content-type", "text/event-stream")
            .body(Body::from("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"role\":\"assistant\",\"content\":[]}}\n\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"part\"}}\n\n")).unwrap();
        let response = chat_response(response, "m").await;
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("上游回复在完成标记前中断"), "{text}");
        assert!(!text.contains("[DONE]"), "未完成的回答不发 [DONE]");
    }

    #[test]
    fn anthropic_error_becomes_chat_error() {
        // 同步测 error 形态：Decoder 的 error 事件也要走
        let mut d = ChatDecoder::new("m");
        let out = d.feed(b"data: {\"type\":\"error\",\"error\":{\"type\":\"api_error\",\"message\":\"boom\"}}\n\n");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\"message\":\"boom\""));
        assert!(text.contains("[DONE]"));
    }

    #[test]
    fn call_ids_round_trip() {
        assert_eq!(out_id("call_abc-1"), "call_abc-1");
        let odd = out_id("call:1/2");
        assert!(safe_chars(&odd) && odd.starts_with(ID_MARK));
        assert_eq!(in_id(&odd), "call:1/2");
        assert_eq!(in_id("toolu_1"), "toolu_1");
    }

    #[test]
    fn chat_json_renders_anthropic_message() {
        let chat = json!({"id":"chatcmpl-x","choices":[{"message":{"content":"done","reasoning_content":"r","tool_calls":[{"id":"call_1","function":{"name":"read","arguments":"{\"a\":1}"}}]},"finish_reason":"tool_calls"}],
            "usage":{"prompt_tokens":10,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":4}}});
        let m = render(&chat, "src/gmodel");
        assert_eq!(m["id"], "msg_x");
        assert_eq!(m["content"][0]["type"], "thinking");
        assert_eq!(m["content"][2]["input"]["a"], 1);
        assert_eq!(m["stop_reason"], "tool_use");
        assert_eq!(m["usage"]["input_tokens"], 6);
        assert_eq!(m["usage"]["cache_read_input_tokens"], 4);
    }

    #[test]
    fn chat_stream_encodes_anthropic_events() {
        let mut e = Encoder::new("src/gmodel");
        let mut out = Vec::new();
        for c in [
            json!({"id":"chatcmpl-1","choices":[{"delta":{"role":"assistant","content":""}}]}),
            json!({"choices":[{"delta":{"reasoning_content":"想"}}]}),
            json!({"choices":[{"delta":{"content":"好"}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read","arguments":"{\"a\""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":1}"}}]}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2}}),
        ] {
            out.extend(e.feed(format!("data: {c}\n\n").as_bytes()));
        }
        out.extend(e.feed(b"data: [DONE]\n\n"));
        let text = String::from_utf8(out).unwrap();
        let names: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("event: ")).collect();
        assert_eq!(names, ["message_start", "content_block_start", "content_block_delta", "content_block_stop", "content_block_start", "content_block_delta", "content_block_stop",
            "content_block_start", "content_block_delta", "content_block_delta", "content_block_stop", "message_delta", "message_stop"]);
        assert!(text.contains("\"stop_reason\":\"tool_use\"") && text.contains("\"output_tokens\":2") && text.contains("\"id\":\"msg_1\""));
        let mut e = Encoder::new("m");
        let failed = String::from_utf8(e.feed(b"data: {\"error\":{\"message\":\"boom\"}}\n\n")).unwrap();
        assert!(failed.contains("event: error") && failed.contains("boom"));
    }
}
