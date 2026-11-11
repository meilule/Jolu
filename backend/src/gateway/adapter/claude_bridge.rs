//! Claude 订阅的富请求桥（3.1）：流式、图片与调用方工具。
//!
//! 仍然驱动官方 `claude` 客户端（不在 HTTP 上重放 OAuth，见 `claude.rs` 模块头）：
//! - 输入走 `--input-format stream-json`：一条 user 消息，正文是对话记录，图片 / 文档块原样带上；
//! - 输出加 `--include-partial-messages`：客户端转出的 `stream_event` 就是 Anthropic 的流事件，
//!   原样（改回工具名、模型名）交给调用方；非流式调用方由同一串事件拼成完整消息；
//! - 调用方的工具由网关自己当 MCP 服务器（`/__jolu/claude-mcp/<令牌>`，HTTP 传输）提供给
//!   客户端。模型调工具时客户端的 `tools/call` 挂在网关里，本段回复以 `tool_use` 结束交给
//!   调用方；调用方下一次请求带着 `tool_result` 来，网关把结果交回挂着的调用，同一个客户端
//!   进程接着生成（magpie `claude_subscription.go` 的做法，见其 `claude-subscription-bridge.md`）。
//!
//! 令牌每个进程一个（32 字节随机），写在只有本进程可读的临时配置文件里，不上命令行。
//! 账号锁只在准备授权与保存刷新结果时持有：挂起等待工具结果期间不占锁，否则同一账号的
//! 并行子代理会互相等到超时。

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, oneshot};

use crate::gateway::route::PoolEntry;
use crate::gateway::wire::error_response;
use crate::proxy::ProxyDeps;

/// MCP 服务器名：客户端把工具叫成 `mcp__jolu__<名>`。
const SERVER: &str = "jolu";
const PREFIX: &str = "mcp__jolu__";
/// 挂起的进程最多等调用方这么久；过了就结束进程，之后的结果按新对话重来。
const PARK_IDLE: Duration = Duration::from_secs(15 * 60);
/// 调用方送回结果后，等客户端的 `tools/call` 到达的上限（它在收到整段回复后才发出）。
const MATCH_WAIT: Duration = Duration::from_secs(30);
/// 两条输出之间的最长间隔：超过就当客户端卡死，明确报错。
const EVENT_IDLE: Duration = Duration::from_secs(600);

/// 需要走本桥的请求：流式、带工具、或消息里有图片 / 文档 / 工具块。纯文本非流式仍走原路径。
pub(super) fn wanted(body: &Value) -> bool {
    body["stream"].as_bool() == Some(true)
        || body["tools"].as_array().is_some_and(|t| !t.is_empty())
        || body["messages"].as_array().is_some_and(|ms| {
            ms.iter().any(|m| {
                m["content"].as_array().is_some_and(|bs| {
                    bs.iter().any(|b| matches!(b["type"].as_str(), Some("image" | "document" | "tool_use" | "tool_result")))
                })
            })
        })
}

/// 官方客户端自己决定采样参数：只接受等于默认的 temperature；`tool_choice` 只认 auto / none。
/// `thinking`、`output_config`、`metadata`、`context_management` 由客户端按自己的设置处理，不报错。
fn check(body: &Value) -> Result<(), String> {
    for field in ["top_p", "top_k", "stop_sequences"] {
        if body.get(field).is_some_and(|v| !v.is_null()) {
            return Err(format!("Claude 订阅由官方客户端生成，不支持 {field}，请移除该选项"));
        }
    }
    if body.get("temperature").is_some_and(|t| !t.is_null() && t.as_f64() != Some(1.0)) {
        return Err("Claude 订阅由官方客户端生成，temperature 只接受默认值 1".into());
    }
    match body["tool_choice"]["type"].as_str() {
        None | Some("auto" | "none") => {}
        Some(other) => return Err(format!("Claude 订阅暂不支持 tool_choice={other}，只支持 auto / none")),
    }
    let messages = body["messages"].as_array().ok_or("缺少 messages 字段")?;
    if messages.is_empty() {
        return Err("对话内容为空".into());
    }
    if messages.iter().any(|m| !matches!(m["role"].as_str(), Some("user" | "assistant"))) {
        return Err("Messages 只支持 user / assistant 角色，系统提示词请放入 system".into());
    }
    Ok(())
}

/// 客户端只保留 `[A-Za-z0-9_-]`、最长 64（含前缀）的 MCP 工具名。合规的名字原样；
/// 否则截短、替换字符并接上整名的哈希，保证不同名字不撞、同一名字每次一样。
pub(super) fn bridge_name(name: &str) -> String {
    let tail_hashed = name.len() > 13
        && name.as_bytes()[name.len() - 13] == b'_'
        && name[name.len() - 12..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let clean = !name.is_empty()
        && name.len() <= 51
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && !tail_hashed;
    if clean {
        return name.into();
    }
    use sha2::Digest;
    let cut: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .take(38)
        .collect();
    let digest = sha2::Sha256::digest(name.as_bytes());
    let hex: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("{cut}_{hex}")
}

/// 工具的 inputSchema 必须以 object 为根：根上是 oneOf / anyOf / allOf 的，客户端会把整个
/// 工具丢掉。把各分支的字段合起来，每个分支都要求的字段仍然必填。
fn object_schema(schema: &Value) -> Value {
    let variants = ["oneOf", "anyOf", "allOf"].iter().find_map(|k| schema.get(*k).and_then(Value::as_array));
    let Some(variants) = variants else {
        if schema["type"] == "object" || schema.get("properties").is_some() {
            let mut schema = schema.clone();
            schema["type"] = json!("object");
            return schema;
        }
        return json!({"type": "object"});
    };
    let mut properties = schema["properties"].as_object().cloned().unwrap_or_default();
    let mut required: Option<Vec<String>> = None;
    for variant in variants {
        if let Some(props) = variant["properties"].as_object() {
            for (k, v) in props {
                properties.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        let own: Vec<String> = variant["required"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        required = Some(match required {
            None => own,
            Some(prev) => prev.into_iter().filter(|k| own.contains(k)).collect(),
        });
    }
    let mut out = json!({"type": "object", "properties": properties});
    if let Some(required) = required.filter(|r| !r.is_empty()) {
        out["required"] = json!(required);
    }
    out
}

/// 调用方工具 → MCP 工具表与「桥上的名字 → 调用方的名字」。
fn mcp_tools(tools: &[Value]) -> (Vec<Value>, HashMap<String, String>) {
    let mut list = Vec::new();
    let mut names = HashMap::new();
    for tool in tools {
        let Some(name) = tool["name"].as_str().filter(|n| !n.is_empty()) else { continue };
        // 服务器端工具（web_search 等带 type 的）不由调用方执行，不桥接
        if tool.get("type").and_then(Value::as_str).is_some_and(|t| t != "custom") {
            continue;
        }
        let bridged = bridge_name(name);
        list.push(json!({
            "name": bridged,
            "description": tool["description"].as_str().unwrap_or_default(),
            "inputSchema": object_schema(&tool["input_schema"]),
        }));
        names.insert(bridged, name.to_string());
    }
    (list, names)
}

/// 把对话变成给客户端的一条 user 消息的内容块。只有一条消息时原样；多条时前面的逐条渲染成
/// 「角色: 内容」文字记录，图片 / 文档块原样插在原位置，工具调用与结果写成文字。
fn render(messages: &[Value]) -> Result<Vec<Value>, String> {
    let mut out: Vec<Value> = Vec::new();
    let mut text = String::new();
    let single = messages.len() == 1;
    if !single {
        text.push_str("（以下是此前的对话记录，请接着回应最后一条 user 消息）\n\n");
    }
    fn flush(text: &mut String, out: &mut Vec<Value>) {
        if !text.trim().is_empty() {
            out.push(json!({"type": "text", "text": text.trim_end()}));
        }
        text.clear();
    }
    for message in messages {
        let role = message["role"].as_str().unwrap_or("user");
        let lead = if single { String::new() } else { format!("{role}: ") };
        match &message["content"] {
            Value::String(s) => {
                text.push_str(&lead);
                text.push_str(s);
                text.push_str("\n\n");
            }
            Value::Array(blocks) => {
                for block in blocks {
                    match block["type"].as_str().unwrap_or_default() {
                        "text" => {
                            text.push_str(&lead);
                            text.push_str(block["text"].as_str().unwrap_or_default());
                            text.push_str("\n\n");
                        }
                        "image" | "document" => {
                            if !block["source"].is_object() {
                                return Err("图片 / 文档块缺少 source".into());
                            }
                            flush(&mut text, &mut out);
                            let mut block = block.clone();
                            if let Some(obj) = block.as_object_mut() {
                                obj.remove("cache_control");
                            }
                            out.push(block);
                        }
                        "tool_use" => {
                            text.push_str(&format!(
                                "{lead}[调用工具 {}，参数 {}]\n\n",
                                block["name"].as_str().unwrap_or_default(),
                                block["input"]
                            ));
                        }
                        "tool_result" => {
                            let failed = if block["is_error"].as_bool() == Some(true) { "（出错）" } else { "" };
                            text.push_str(&format!("{lead}[工具结果{failed}] "));
                            match &block["content"] {
                                Value::String(s) => text.push_str(s),
                                Value::Array(parts) => {
                                    for part in parts {
                                        if part["type"] == "image" {
                                            flush(&mut text, &mut out);
                                            out.push(json!({"type": "image", "source": part["source"]}));
                                        } else if let Some(s) = part["text"].as_str() {
                                            text.push_str(s);
                                        }
                                    }
                                }
                                _ => {}
                            }
                            text.push_str("\n\n");
                        }
                        // 思考块属于上一轮生成，客户端会自己重新思考
                        "thinking" | "redacted_thinking" => {}
                        other => return Err(format!("不支持的消息块类型「{other}」")),
                    }
                }
            }
            _ => return Err("messages 里存在无法解析的内容".into()),
        }
    }
    flush(&mut text, &mut out);
    if out.is_empty() {
        return Err("对话内容为空".into());
    }
    Ok(out)
}

/// 调用方送回的一个工具结果 → MCP `tools/call` 的结果。
fn mcp_result(block: &Value, extra: &[String]) -> Value {
    let mut content = Vec::new();
    match &block["content"] {
        Value::String(s) => content.push(json!({"type": "text", "text": s})),
        Value::Array(parts) => {
            for part in parts {
                match part["type"].as_str() {
                    Some("text") => content.push(json!({"type": "text", "text": part["text"]})),
                    Some("image") if part["source"]["type"] == "base64" => content.push(json!({
                        "type": "image",
                        "data": part["source"]["data"],
                        "mimeType": part["source"]["media_type"],
                    })),
                    _ => content.push(json!({"type": "text", "text": part.to_string()})),
                }
            }
        }
        Value::Null => {}
        other => content.push(json!({"type": "text", "text": other.to_string()})),
    }
    // 调用方把提示文字和结果放在同一条消息里时（Claude Code 的 system-reminder），一并带上
    for s in extra {
        content.push(json!({"type": "text", "text": s}));
    }
    if content.is_empty() {
        content.push(json!({"type": "text", "text": ""}));
    }
    json!({"content": content, "isError": block["is_error"].as_bool() == Some(true)})
}

fn tool_error(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

/// 客户端发来、还没拿到结果的一次工具调用。
struct Parked {
    seq: u64,
    name: String,
    args: Value,
    reply: oneshot::Sender<Value>,
}

/// 本段回复里交给调用方的工具调用（等它的结果）。
#[derive(Clone, Debug, PartialEq)]
struct Issued {
    id: String,
    /// 桥上的名字（不含前缀）
    name: String,
    input: Value,
}

/// 一个活着的客户端进程。
struct Run {
    token: String,
    entry: PoolEntry,
    tools: Vec<Value>,
    names: HashMap<String, String>,
    events: tokio::sync::Mutex<mpsc::Receiver<Value>>,
    stdin: tokio::sync::Mutex<Option<tokio::process::ChildStdin>>,
    child: tokio::sync::Mutex<Option<tokio::process::Child>>,
    calls: Mutex<Vec<Parked>>,
    issued: Mutex<Vec<Issued>>,
    touched: Mutex<Instant>,
    /// 隔离授权（生产）：结束或挂起时把客户端刷新过的授权存回
    bound: Option<crate::gateway::subscription::BoundAuth>,
    profile: Mutex<Option<crate::cli_browser_auth::Profile>>,
    /// 本进程的临时文件（MCP 配置、系统提示），随进程结束删除
    scratch: Scratch,
}

struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

static RUNS: LazyLock<Mutex<HashMap<String, Arc<Run>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Run {
    fn touch(&self) {
        if let Ok(mut t) = self.touched.lock() {
            *t = Instant::now();
        }
    }

    /// 客户端调工具：挂起，等调用方的结果（或进程结束）。
    async fn park(&self, name: String, args: Value) -> Value {
        let (reply, rx) = oneshot::channel();
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(Parked { seq, name, args, reply });
        }
        let outcome = tokio::time::timeout(PARK_IDLE, rx).await;
        if let Ok(mut calls) = self.calls.lock() {
            calls.retain(|c| c.seq != seq);
        }
        match outcome {
            Ok(Ok(value)) => value,
            _ => tool_error("调用方没有送回这次工具调用的结果"),
        }
    }

    /// 调用方送回 `id` 的结果：找到对应的挂起调用（名字与参数一致，其次同名最早的）交过去。
    async fn deliver(&self, id: &str, result: Value) -> bool {
        let Some(issued) = self.issued.lock().ok().and_then(|i| i.iter().find(|x| x.id == id).cloned()) else {
            return false;
        };
        let deadline = Instant::now() + MATCH_WAIT;
        loop {
            let parked = self.calls.lock().ok().and_then(|mut calls| {
                let at = calls
                    .iter()
                    .position(|c| c.name == issued.name && c.args == issued.input)
                    .or_else(|| calls.iter().position(|c| c.name == issued.name))?;
                Some(calls.remove(at))
            });
            if let Some(parked) = parked {
                if let Ok(mut list) = self.issued.lock() {
                    list.retain(|x| x.id != id);
                }
                return parked.reply.send(result).is_ok();
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }

    /// 结束进程：关 stdin 让客户端收尾，等一会儿再强制结束；存回刷新过的授权。
    async fn finish(self: &Arc<Self>, deps: &Arc<ProxyDeps>) {
        if let Ok(mut runs) = RUNS.lock() {
            runs.remove(&self.token);
        }
        self.stdin.lock().await.take();
        if let Some(mut child) = self.child.lock().await.take() {
            if tokio::time::timeout(Duration::from_secs(20), child.wait()).await.is_err() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
        self.save_refresh(deps).await;
        self.profile.lock().ok().and_then(|mut p| p.take());
    }

    async fn save_refresh(&self, deps: &Arc<ProxyDeps>) {
        let Some(bound) = &self.bound else { return };
        let _guard = crate::gateway::subscription::lock(&self.entry.connection_id).await;
        let profile = self.profile.lock().ok();
        if let Some(profile) = profile.as_ref().and_then(|p| p.as_ref()) {
            if let Err(reason) = super::claude::save_profile_refresh(deps, &self.entry, bound, profile) {
                eprintln!("[网关] Claude 订阅刷新后的授权未能保存：{reason}");
            }
        }
    }
}

/// 网关的 MCP 端点（`POST /__jolu/claude-mcp/<令牌>`）。令牌不对一律 404。
pub(crate) async fn mcp(token: &str, body: &[u8]) -> Response {
    let run = RUNS.lock().ok().and_then(|runs| {
        runs.iter().find(|(k, _)| crate::gateway::access::equal(k, token)).map(|(_, run)| run.clone())
    });
    let Some(run) = run else {
        return error_response(StatusCode::NOT_FOUND, "没有这个工具桥");
    };
    let Ok(request) = serde_json::from_slice::<Value>(body) else {
        return json_reply(StatusCode::BAD_REQUEST, json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}}));
    };
    let answers = match &request {
        Value::Array(batch) => {
            let mut out = Vec::new();
            for item in batch {
                if let Some(answer) = answer(&run, item).await {
                    out.push(answer);
                }
            }
            (!out.is_empty()).then(|| Value::Array(out))
        }
        single => answer(&run, single).await,
    };
    match answers {
        Some(value) => json_reply(StatusCode::OK, value),
        // 只有通知：202 无内容（Streamable HTTP）
        None => Response::builder().status(StatusCode::ACCEPTED).body(Body::empty()).unwrap_or_default(),
    }
}

async fn answer(run: &Arc<Run>, request: &Value) -> Option<Value> {
    let id = request.get("id").filter(|v| !v.is_null())?.clone();
    run.touch();
    let result = match request["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": request["params"]["protocolVersion"].as_str().unwrap_or("2025-03-26"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": SERVER, "version": env!("CARGO_PKG_VERSION")},
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": run.tools}),
        "tools/call" => {
            let name = request["params"]["name"].as_str().unwrap_or_default().to_string();
            let args = request["params"]["arguments"].clone();
            let args = if args.is_null() { json!({}) } else { args };
            if !run.names.contains_key(&name) {
                tool_error(&format!("没有这个工具：{name}"))
            } else {
                run.park(name, args).await
            }
        }
        _ => {
            return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn json_reply(status: StatusCode, value: Value) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .unwrap_or_default()
}

/// 清掉挂太久没人管的进程。
async fn sweep(deps: &Arc<ProxyDeps>) {
    let stale: Vec<Arc<Run>> = RUNS
        .lock()
        .map(|runs| {
            runs.values()
                .filter(|r| r.touched.lock().is_ok_and(|t| t.elapsed() > PARK_IDLE))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for run in stale {
        run.finish(deps).await;
    }
}

/// 调用方的最后一条 user 消息里的工具结果，以及同一条消息里的其余文字。
fn trailing_results(messages: &[Value]) -> (Vec<Value>, Vec<String>) {
    let Some(last) = messages.last().filter(|m| m["role"] == "user") else { return (Vec::new(), Vec::new()) };
    let Some(blocks) = last["content"].as_array() else { return (Vec::new(), Vec::new()) };
    let results: Vec<Value> = blocks.iter().filter(|b| b["type"] == "tool_result").cloned().collect();
    let extra = blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str().map(str::to_owned))
        .collect();
    (results, extra)
}

/// 本段回复怎么结束的。
enum Ended {
    /// 停在调用方的工具上，进程挂起等结果
    Parked(Vec<Issued>),
    /// 这轮说完了
    Done,
    /// 出错（消息已经发给调用方）
    Failed,
}

/// 从客户端的输出里读出一段回复（到 message_stop），改好名字后发给 `tx`。
async fn pump(run: &Run, model: &str, tx: &mpsc::Sender<Result<Value, String>>, usage: &mut Map<String, Value>) -> Ended {
    let mut events = run.events.lock().await;
    let mut tracked: HashMap<u64, (Issued, String)> = HashMap::new();
    let mut issued: Vec<Issued> = Vec::new();
    let mut stop_reason: Option<String> = None;
    let fail = |message: String| async move {
        let _ = tx.send(Err(message)).await;
        Ended::Failed
    };
    loop {
        let value = match tokio::time::timeout(EVENT_IDLE, events.recv()).await {
            Err(_) => return fail("Claude 订阅超过 10 分钟没有新输出".into()).await,
            Ok(None) => return fail("Claude 客户端提前退出，没有完成本段回复".into()).await,
            Ok(Some(value)) => value,
        };
        run.touch();
        match value["type"].as_str() {
            Some("stream_event") => {
                let mut event = value["event"].clone();
                match event["type"].as_str() {
                    Some("message_start") => {
                        event["message"]["model"] = json!(model);
                        if let Some(u) = event["message"]["usage"].as_object() {
                            usage.extend(u.clone());
                        }
                    }
                    Some("content_block_start") if event["content_block"]["type"] == "tool_use" => {
                        let name = event["content_block"]["name"].as_str().unwrap_or_default().to_string();
                        if let Some(bridged) = name.strip_prefix(PREFIX) {
                            let caller = run.names.get(bridged).cloned().unwrap_or_else(|| bridged.to_string());
                            event["content_block"]["name"] = json!(caller);
                            let index = event["index"].as_u64().unwrap_or(0);
                            let id = event["content_block"]["id"].as_str().unwrap_or_default().to_string();
                            tracked.insert(index, (Issued { id, name: bridged.to_string(), input: json!({}) }, String::new()));
                        }
                    }
                    Some("content_block_delta") if event["delta"]["type"] == "input_json_delta" => {
                        if let Some((_, buf)) = tracked.get_mut(&event["index"].as_u64().unwrap_or(0)) {
                            buf.push_str(event["delta"]["partial_json"].as_str().unwrap_or_default());
                        }
                    }
                    Some("content_block_stop") => {
                        if let Some((mut call, buf)) = tracked.remove(&event["index"].as_u64().unwrap_or(0)) {
                            call.input = if buf.trim().is_empty() { json!({}) } else { serde_json::from_str(&buf).unwrap_or(json!({})) };
                            issued.push(call);
                        }
                    }
                    Some("message_delta") => {
                        stop_reason = event["delta"]["stop_reason"].as_str().map(str::to_owned);
                        if let Some(u) = event["usage"].as_object() {
                            usage.extend(u.clone());
                        }
                    }
                    _ => {}
                }
                let stop = event["type"] == "message_stop";
                if tx.send(Ok(event)).await.is_err() {
                    // 调用方断开：本段作废
                    return Ended::Failed;
                }
                if stop {
                    break;
                }
            }
            // 接口错误由客户端合成一条 `<synthetic>` 的 assistant 消息，没有流事件
            Some("assistant") if value["message"]["model"] == "<synthetic>" => {
                let text = value["message"]["content"][0]["text"].as_str().unwrap_or("Claude 接口返回错误").to_string();
                return fail(format!("Claude 订阅生成失败：{text}")).await;
            }
            Some("result") => {
                let failed = value["is_error"].as_bool() == Some(true)
                    || value["subtype"].as_str().is_some_and(|s| s.starts_with("error"));
                let message = value["result"].as_str().unwrap_or("Claude 未能完成本次生成，请检查所选账号授权与额度");
                return fail(if failed { format!("Claude 订阅生成失败：{message}") } else { "Claude 客户端没有输出流事件".into() }).await;
            }
            _ => {}
        }
    }
    if stop_reason.as_deref() == Some("tool_use") && !issued.is_empty() {
        Ended::Parked(issued)
    } else {
        Ended::Done
    }
}

/// 一串流事件 → 完整的 Anthropic 消息（非流式调用方）。
fn assemble(events: &[Value]) -> Value {
    let mut message = json!({"type": "message", "role": "assistant", "content": [], "stop_reason": null, "stop_sequence": null});
    let mut blocks: Vec<(Value, String)> = Vec::new();
    for event in events {
        match event["type"].as_str() {
            Some("message_start") => {
                if let Some(start) = event["message"].as_object() {
                    for (k, v) in start {
                        if k != "content" {
                            message[k] = v.clone();
                        }
                    }
                }
            }
            Some("content_block_start") => blocks.push((event["content_block"].clone(), String::new())),
            Some("content_block_delta") => {
                let Some((block, buf)) = blocks.get_mut(event["index"].as_u64().unwrap_or(0) as usize) else { continue };
                let delta = &event["delta"];
                match delta["type"].as_str() {
                    Some("text_delta") => append(block, "text", delta["text"].as_str()),
                    Some("thinking_delta") => append(block, "thinking", delta["thinking"].as_str()),
                    Some("signature_delta") => block["signature"] = delta["signature"].clone(),
                    Some("input_json_delta") => buf.push_str(delta["partial_json"].as_str().unwrap_or_default()),
                    _ => {}
                }
            }
            Some("message_delta") => {
                for (k, v) in event["delta"].as_object().into_iter().flatten() {
                    message[k] = v.clone();
                }
                if let Some(u) = event["usage"].as_object() {
                    if !message["usage"].is_object() {
                        message["usage"] = json!({});
                    }
                    for (k, v) in u {
                        message["usage"][k] = v.clone();
                    }
                }
            }
            _ => {}
        }
    }
    message["content"] = Value::Array(
        blocks
            .into_iter()
            .map(|(mut block, buf)| {
                if block["type"] == "tool_use" {
                    block["input"] = if buf.trim().is_empty() { json!({}) } else { serde_json::from_str(&buf).unwrap_or(json!({})) };
                }
                block
            })
            .collect(),
    );
    message
}

fn append(block: &mut Value, key: &str, piece: Option<&str>) {
    let mut s = block[key].as_str().unwrap_or_default().to_string();
    s.push_str(piece.unwrap_or_default());
    block[key] = json!(s);
}

fn sse(event: &Value) -> Bytes {
    Bytes::from(format!("event: {}\ndata: {}\n\n", event["type"].as_str().unwrap_or("message"), event))
}

fn sse_error(message: &str) -> Bytes {
    let event = json!({"type": "error", "error": {"type": "api_error", "message": message}});
    Bytes::from(format!("event: error\ndata: {event}\n\n"))
}

/// 启动参数：可执行文件（测试可注入）后面跟富请求需要的选项。
fn args(base: Vec<String>, model: &str, mcp_config: &std::path::Path, system: Option<&std::path::Path>, with_tools: bool) -> Vec<String> {
    let mut args = base;
    args.extend([
        "-p", "--verbose", "--input-format", "stream-json", "--output-format", "stream-json", "--include-partial-messages",
        "--tools", "", "--strict-mcp-config", "--setting-sources", "user",
    ].map(String::from));
    args.push("--mcp-config".into());
    args.push(mcp_config.to_string_lossy().into_owned());
    if with_tools {
        args.push("--allowedTools".into());
        args.push(format!("mcp__{SERVER}"));
    }
    if let Some(system) = system {
        args.push("--system-prompt-file".into());
        args.push(system.to_string_lossy().into_owned());
    }
    args.push("--model".into());
    args.push(model.into());
    args
}

/// 网关自己的地址（MCP 回调用）。测试可以指定。
fn gateway_base(injected: Option<&str>) -> Option<String> {
    injected.map(str::to_owned).or_else(|| crate::proxy::running_port().map(|port| format!("http://127.0.0.1:{port}")))
}

/// 富请求入口。`command` / `base` 仅测试注入（假客户端与临时网关地址）。
pub(super) async fn forward(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    body: &Value,
    command: Option<Vec<String>>,
    isolated: bool,
    base: Option<&str>,
) -> Response {
    sweep(&deps).await;
    if let Err(reason) = check(body) {
        return error_response(StatusCode::BAD_REQUEST, &reason);
    }
    let requested = body["model"].as_str().unwrap_or_default();
    let model = crate::gateway::route::split_model_id(requested).map(|(_, rest)| rest.to_string()).unwrap_or_else(|| requested.to_string());
    let streaming = body["stream"].as_bool() == Some(true);
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let started = Instant::now();

    // 带着工具结果来：找到停在这些调用上的进程，把结果交回去接着读
    let (results, extra) = trailing_results(&messages);
    let live = results.first().and_then(|first| {
        let id = first["tool_use_id"].as_str()?;
        RUNS.lock().ok()?.values().find(|r| r.issued.lock().is_ok_and(|i| i.iter().any(|x| x.id == id))).cloned()
    });
    let run = match live {
        Some(run) => {
            let mut delivered = true;
            let count = results.len();
            for (i, block) in results.iter().enumerate() {
                let extra = if i + 1 == count { extra.as_slice() } else { &[] };
                let id = block["tool_use_id"].as_str().unwrap_or_default();
                delivered &= run.deliver(id, mcp_result(block, extra)).await;
            }
            if delivered {
                run.touch();
                run
            } else {
                // 对不上（进程已经不在等这些调用）：结束它，按新对话重来
                run.finish(&deps).await;
                match start(&deps, entry, body, &messages, &model, command, isolated, base).await {
                    Ok(run) => run,
                    Err(response) => return response,
                }
            }
        }
        None => match start(&deps, entry, body, &messages, &model, command, isolated, base).await {
            Ok(run) => run,
            Err(response) => return response,
        },
    };

    let (tx, mut rx) = mpsc::channel::<Result<Value, String>>(256);
    let pump_run = run.clone();
    let pump_deps = deps.clone();
    let pump_model = model.clone();
    let pump_entry = entry.clone();
    tokio::spawn(async move {
        let mut usage = Map::new();
        let ended = pump(&pump_run, &pump_model, &tx, &mut usage).await;
        drop(tx);
        if isolated && !usage.is_empty() {
            let record = json!({"model": pump_model, "usage": usage});
            crate::gateway::subscription::record_usage(&pump_deps, &pump_entry, &record, started.elapsed().as_millis() as i64);
        }
        match ended {
            Ended::Parked(issued) => {
                if let Ok(mut list) = pump_run.issued.lock() {
                    *list = issued;
                }
                pump_run.touch();
                pump_run.save_refresh(&pump_deps).await;
            }
            Ended::Done | Ended::Failed => pump_run.finish(&pump_deps).await,
        }
    });

    // 第一条输出到之前出错：给调用方一个真正的错误状态，而不是 200 的空流
    let first = match rx.recv().await {
        Some(Ok(event)) => event,
        Some(Err(message)) => return error_response(StatusCode::BAD_GATEWAY, &message),
        None => return error_response(StatusCode::BAD_GATEWAY, "Claude 客户端没有输出"),
    };
    if streaming {
        let stream = futures_util::stream::unfold((Some(first), rx), |(pending, mut rx)| async move {
            if let Some(event) = pending {
                return Some((Ok::<Bytes, std::io::Error>(sse(&event)), (None, rx)));
            }
            match rx.recv().await? {
                Ok(event) => Some((Ok(sse(&event)), (None, rx))),
                Err(message) => Some((Ok(sse_error(&message)), (None, rx))),
            }
        });
        return Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .header("x-jolu-gateway", entry.slug.as_str())
            .body(Body::from_stream(stream))
            .unwrap_or_else(|_| error_response(StatusCode::BAD_GATEWAY, "构造回复失败"));
    }
    let mut events = vec![first];
    while let Some(item) = rx.recv().await {
        match item {
            Ok(event) => events.push(event),
            Err(message) => return error_response(StatusCode::BAD_GATEWAY, &message),
        }
    }
    let message = assemble(&events);
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("x-jolu-gateway", entry.slug.as_str())
        .body(Body::from(message.to_string()))
        .unwrap_or_else(|_| error_response(StatusCode::BAD_GATEWAY, "构造回复失败"))
}

/// 新起一个客户端进程：写临时配置、启动、送进第一条 user 消息，登记到 [`RUNS`]。
#[allow(clippy::too_many_arguments)]
async fn start(
    deps: &Arc<ProxyDeps>,
    entry: &PoolEntry,
    body: &Value,
    messages: &[Value],
    model: &str,
    command: Option<Vec<String>>,
    isolated: bool,
    base: Option<&str>,
) -> Result<Arc<Run>, Response> {
    let content = render(messages).map_err(|reason| error_response(StatusCode::BAD_REQUEST, &reason))?;
    let offered = body["tool_choice"]["type"] != "none";
    let (tools, names) = if offered {
        mcp_tools(body["tools"].as_array().map(Vec::as_slice).unwrap_or_default())
    } else {
        (Vec::new(), HashMap::new())
    };
    let system = match &body["system"] {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => Some(blocks.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")),
        _ => None,
    };

    let (bound, profile) = if isolated {
        let _guard = crate::gateway::subscription::lock(&entry.connection_id).await;
        let bound = deps
            .db
            .lock()
            .map_err(|_| "网关状态暂不可用".to_string())
            .and_then(|conn| crate::gateway::subscription::load(&conn, &entry.connection_id, "claude"))
            .map_err(|reason| error_response(StatusCode::UNAUTHORIZED, &reason))?;
        let profile = super::claude::prepare_profile(&bound).map_err(|reason| error_response(StatusCode::SERVICE_UNAVAILABLE, &reason))?;
        (Some(bound), Some(profile))
    } else {
        (None, None)
    };

    let token: String = {
        use aes_gcm::aead::{rand_core::RngCore, OsRng};
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    };
    let dir = std::env::temp_dir().join(format!("jolu-claude-bridge-{}", &token[..16]));
    std::fs::create_dir_all(&dir).map_err(|_| error_response(StatusCode::SERVICE_UNAVAILABLE, "无法准备工具桥临时目录"))?;
    let scratch = Scratch(dir.clone());
    let servers = if tools.is_empty() {
        json!({})
    } else {
        let Some(base) = gateway_base(base) else {
            return Err(error_response(StatusCode::SERVICE_UNAVAILABLE, "网关没在监听，无法为 Claude 订阅提供工具"));
        };
        json!({SERVER: {"type": "http", "url": format!("{base}/__jolu/claude-mcp/{token}")}})
    };
    let mcp_path = dir.join("mcp.json");
    write_private(&mcp_path, &json!({"mcpServers": servers}).to_string())
        .map_err(|_| error_response(StatusCode::SERVICE_UNAVAILABLE, "无法写入工具桥配置"))?;
    let system_path = match &system {
        Some(text) => {
            let path = dir.join("system.txt");
            write_private(&path, text).map_err(|_| error_response(StatusCode::SERVICE_UNAVAILABLE, "无法写入系统提示"))?;
            Some(path)
        }
        None => None,
    };

    let program = match command {
        Some(injected) if !injected.is_empty() => injected,
        _ => vec![super::claude::executable()],
    };
    let full = args(program, model, &mcp_path, system_path.as_deref(), !tools.is_empty());
    let (program, rest) = full.split_first().map(|(p, r)| (p.clone(), r.to_vec())).unwrap_or_default();
    let mut std_command = std::process::Command::new(&program);
    if let Some(profile) = &profile {
        crate::cli_browser_auth::configure_environment(&mut std_command, profile);
    } else {
        std_command.current_dir(std::env::temp_dir());
    }
    // 回环地址（MCP 端点）不能走代理
    let prior = std::env::var("NO_PROXY").unwrap_or_default();
    let no_proxy = if prior.trim() == "*" { prior } else { format!("{prior},localhost,127.0.0.1,::1").trim_start_matches(',').to_string() };
    std_command.env("NO_PROXY", &no_proxy).env("no_proxy", &no_proxy);
    if let Some(max) = body["max_tokens"].as_u64().filter(|n| *n > 0) {
        std_command.env("CLAUDE_CODE_MAX_OUTPUT_TOKENS", max.to_string());
    }
    let mut child = tokio::process::Command::from(std_command)
        .args(&rest)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| error_response(StatusCode::SERVICE_UNAVAILABLE, &format!("无法启动 Claude Code（{error}），请先安装官方客户端")))?;
    let stdout = child.stdout.take().ok_or_else(|| error_response(StatusCode::BAD_GATEWAY, "claude 进程未提供 stdout"))?;
    let mut stdin = child.stdin.take().ok_or_else(|| error_response(StatusCode::BAD_GATEWAY, "claude 进程未提供 stdin"))?;
    let (event_tx, event_rx) = mpsc::channel::<Value>(1024);
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if event_tx.send(value).await.is_err() {
                    break;
                }
            }
        }
    });
    let first = json!({"type": "user", "message": {"role": "user", "content": content}});
    {
        use tokio::io::AsyncWriteExt;
        let line = format!("{first}\n");
        if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
            return Err(error_response(StatusCode::BAD_GATEWAY, "向 claude 写入输入失败"));
        }
    }
    let run = Arc::new(Run {
        token: token.clone(),
        entry: entry.clone(),
        tools,
        names,
        events: tokio::sync::Mutex::new(event_rx),
        stdin: tokio::sync::Mutex::new(Some(stdin)),
        child: tokio::sync::Mutex::new(Some(child)),
        calls: Mutex::new(Vec::new()),
        issued: Mutex::new(Vec::new()),
        touched: Mutex::new(Instant::now()),
        bound,
        profile: Mutex::new(profile),
        scratch,
    });
    let _ = &run.scratch;
    if let Ok(mut runs) = RUNS.lock() {
        runs.insert(token, run.clone());
    }
    Ok(run)
}

/// 只给当前用户读写的文件（令牌在里面）。Windows 的临时目录本就是用户私有。
fn write_private(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        file.write_all(content.as_bytes())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_that_need_the_bridge_are_recognised() {
        assert!(!wanted(&json!({"messages": [{"role": "user", "content": "hi"}]})));
        assert!(wanted(&json!({"stream": true, "messages": []})));
        assert!(wanted(&json!({"tools": [{"name": "a"}], "messages": []})));
        assert!(wanted(&json!({"messages": [{"role": "user", "content": [{"type": "image", "source": {}}]}]})));
    }

    #[test]
    fn tool_names_are_kept_when_valid_and_hashed_otherwise() {
        assert_eq!(bridge_name("read_file"), "read_file");
        let dotted = bridge_name("fs.read");
        let underscored = bridge_name("fs_read");
        assert_ne!(dotted, underscored, "a.b 与 a_b 不能撞");
        assert!(dotted.starts_with("fs_read_") && dotted.len() == "fs_read_".len() + 12, "{dotted}");
        assert_eq!(bridge_name("fs.read"), dotted, "同一名字每次一样");
        let long = "x".repeat(80);
        assert!(bridge_name(&long).len() <= 51);
        // 看起来像已经哈希过的名字也要重新哈希，避免和别的工具撞
        assert_ne!(bridge_name(&dotted), dotted);
    }

    #[test]
    fn union_schemas_are_folded_to_an_object_root() {
        let schema = json!({"oneOf": [
            {"type": "object", "properties": {"a": {"type": "string"}, "k": {"type": "string"}}, "required": ["a", "k"]},
            {"type": "object", "properties": {"b": {"type": "number"}, "k": {"type": "string"}}, "required": ["b", "k"]},
        ]});
        let folded = object_schema(&schema);
        assert_eq!(folded["type"], "object");
        assert!(folded["properties"]["a"].is_object() && folded["properties"]["b"].is_object());
        assert_eq!(folded["required"], json!(["k"]), "每个分支都要求的才必填");
        assert_eq!(object_schema(&json!({"type": "string"})), json!({"type": "object"}));
    }

    #[test]
    fn conversations_render_to_text_with_images_in_place() {
        let one = render(&[json!({"role": "user", "content": [{"type": "text", "text": "看图"}, {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAA"}}]})]).unwrap();
        assert_eq!(one.len(), 2);
        assert_eq!(one[1]["type"], "image");
        let many = render(&[
            json!({"role": "user", "content": "读一下 a.rs"}),
            json!({"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "read", "input": {"path": "a.rs"}}]}),
            json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "fn main() {}"}]}),
        ])
        .unwrap();
        let text = many[0]["text"].as_str().unwrap();
        assert!(text.contains("user: 读一下 a.rs"));
        assert!(text.contains("assistant: [调用工具 read，参数 {\"path\":\"a.rs\"}]"));
        assert!(text.contains("[工具结果] fn main() {}"));
        assert!(render(&[json!({"role": "user", "content": [{"type": "audio"}]})]).is_err());
    }

    #[test]
    fn sampling_options_the_client_cannot_honour_are_refused() {
        let ok = json!({"messages": [{"role": "user", "content": "hi"}], "thinking": {"type": "enabled"}, "temperature": 1});
        assert!(check(&ok).is_ok(), "thinking 交给客户端，默认温度放行");
        assert!(check(&json!({"messages": [{"role": "user", "content": "hi"}], "temperature": 0.2})).is_err());
        assert!(check(&json!({"messages": [{"role": "user", "content": "hi"}], "tool_choice": {"type": "any"}})).is_err());
    }

    #[test]
    fn stream_events_assemble_into_a_whole_message() {
        let events = [
            json!({"type": "message_start", "message": {"id": "msg_1", "model": "m", "usage": {"input_tokens": 5}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "好"}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "的"}}),
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "t1", "name": "read", "input": {}}}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"path\":"}}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "\"a\"}"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 7}}),
            json!({"type": "message_stop"}),
        ];
        let message = assemble(&events);
        assert_eq!(message["id"], "msg_1");
        assert_eq!(message["content"][0]["text"], "好的");
        assert_eq!(message["content"][1]["input"], json!({"path": "a"}));
        assert_eq!(message["stop_reason"], "tool_use");
        assert_eq!(message["usage"], json!({"input_tokens": 5, "output_tokens": 7}));
    }

    #[test]
    fn tool_results_become_mcp_results_with_trailing_notes() {
        let block = json!({"type": "tool_result", "tool_use_id": "t", "is_error": true, "content": [
            {"type": "text", "text": "失败了"},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAA"}},
        ]});
        let result = mcp_result(&block, &["<system-reminder>注意</system-reminder>".into()]);
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][1], json!({"type": "image", "data": "AAA", "mimeType": "image/png"}));
        assert_eq!(result["content"][2]["text"], "<system-reminder>注意</system-reminder>");
    }

    /// 假客户端（node）：读 MCP 配置、流出一次工具调用、经网关 MCP 端点挂起拿结果，
    /// 再流出第二段文字。两次网关请求：第一次停在 tool_use，第二次带结果接着读同一个进程。
    #[tokio::test]
    async fn tool_calls_park_in_the_gateway_and_resume_the_same_client() {
        let deps = crate::proxy::test_support::bare_deps();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = crate::gateway::server::router(deps.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await;
        });
        let base = format!("http://{addr}");
        let script = r#"
const fs = require('fs');
const argv = process.argv.slice(2);
const cfg = JSON.parse(fs.readFileSync(argv[argv.indexOf('--mcp-config') + 1], 'utf8'));
const url = cfg.mcpServers.jolu.url;
const out = (o) => process.stdout.write(JSON.stringify(o) + '\n');
const ev = (e) => out({type: 'stream_event', event: e});
let buf = '';
process.stdin.on('data', async (chunk) => {
  buf += chunk;
  if (!buf.includes('\n')) return;
  const first = JSON.parse(buf.split('\n')[0]);
  buf = '';
  const said = first.message.content.map((b) => b.text || '').join('');
  const call = async (method, params, id) => (await (await fetch(url, {method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify({jsonrpc: '2.0', id, method, params})})).json());
  const list = await call('tools/list', {}, 1);
  const name = list.result.tools[0].name;
  out({type: 'system', subtype: 'init', session_id: 's'});
  ev({type: 'message_start', message: {id: 'm1', type: 'message', role: 'assistant', content: [], model: 'x', usage: {input_tokens: 3}}});
  ev({type: 'content_block_start', index: 0, content_block: {type: 'tool_use', id: 'toolu_1', name: 'mcp__jolu__' + name, input: {}}});
  ev({type: 'content_block_delta', index: 0, delta: {type: 'input_json_delta', partial_json: '{"path":"a.rs"}'}});
  ev({type: 'content_block_stop', index: 0});
  ev({type: 'message_delta', delta: {stop_reason: 'tool_use'}, usage: {output_tokens: 2}});
  ev({type: 'message_stop'});
  const result = await call('tools/call', {name, arguments: {path: 'a.rs'}}, 2);
  const got = result.result.content.map((c) => c.text).join('|');
  ev({type: 'message_start', message: {id: 'm2', type: 'message', role: 'assistant', content: [], model: 'x', usage: {input_tokens: 4}}});
  ev({type: 'content_block_start', index: 0, content_block: {type: 'text', text: ''}});
  ev({type: 'content_block_delta', index: 0, delta: {type: 'text_delta', text: 'said=' + said + ';got=' + got}});
  ev({type: 'content_block_stop', index: 0});
  ev({type: 'message_delta', delta: {stop_reason: 'end_turn'}, usage: {output_tokens: 5}});
  ev({type: 'message_stop'});
  out({type: 'result', subtype: 'success', result: 'done'});
});
process.stdin.on('end', () => process.exit(0));
"#;
        let command = vec!["node".to_string(), "-e".to_string(), script.to_string(), "--".to_string()];
        let first = json!({
            "model": "claude/claude-sonnet-5",
            "stream": true,
            "tools": [{"name": "fs.read", "description": "读文件", "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}}}],
            "messages": [{"role": "user", "content": "读 a.rs"}],
        });
        let response = forward(deps.clone(), &entry(), &first, Some(command.clone()), false, Some(&base)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let text = String::from_utf8(axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
        assert!(text.contains("\"name\":\"fs.read\""), "工具名要改回调用方的名字：{text}");
        assert!(text.contains("\"model\":\"claude-sonnet-5\""), "{text}");
        assert!(text.contains("\"stop_reason\":\"tool_use\"") && text.contains("event: message_stop"), "{text}");
        assert!(!text.contains("mcp__jolu__"), "{text}");

        let second = json!({
            "model": "claude/claude-sonnet-5",
            "messages": [
                {"role": "user", "content": "读 a.rs"},
                {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_1", "name": "fs.read", "input": {"path": "a.rs"}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "fn main() {}"}, {"type": "text", "text": "提醒"}]},
            ],
            "tools": first["tools"],
        });
        let response = forward(deps.clone(), &entry(), &second, Some(command), false, Some(&base)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let message: Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(message["content"][0]["text"], "said=读 a.rs;got=fn main() {}|提醒", "同一个进程拿到结果接着说：{message}");
        assert_eq!(message["stop_reason"], "end_turn");
        server.abort();
    }

    #[tokio::test]
    async fn unknown_bridge_tokens_are_not_found() {
        let response = mcp("nope", br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_early_failure_is_a_real_error_status() {
        let deps = crate::proxy::test_support::bare_deps();
        let script = "process.stdin.once('data', () => { console.log(JSON.stringify({type:'result',subtype:'error_during_execution',is_error:true,result:'额度用完'})); process.exit(0); });";
        let body = json!({"model": "claude/m", "stream": true, "messages": [{"role": "user", "content": "hi"}]});
        let response = forward(deps, &entry(), &body, Some(vec!["node".into(), "-e".into(), script.into(), "--".into()]), false, None).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let text = String::from_utf8(axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
        assert!(text.contains("额度用完"), "{text}");
    }

    fn entry() -> PoolEntry {
        PoolEntry {
            connection_id: "c-claude".into(),
            slug: "claude".into(),
            name: "Claude 订阅".into(),
            platform: "claude".into(),
            kind: "auth".into(),
            protocol: "anthropic".into(),
            base_url: None,
            secret: None,
            account_fp: None,
            model: None,
            status: "connected".into(),
        }
    }
}
