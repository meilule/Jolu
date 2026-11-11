//! JS 异步钩子（E1 / 3.8，对齐 magpie `gateway-request-hooks.md`）。
//!
//! 中间件表里 `kind = "js-hook"` 的项：`{"path": "<.mjs/.js 文件>", "timeoutMs": 1000, "options": {…}}`。
//! - `export async function onRequest(body, ctx)`：选路前（隐私脱敏之前）拿到调用方协议的请求体，
//!   返回对象作为新请求体，返回 undefined 保持不变。抛错、超时、返回非对象，或改了 `model` /
//!   `stream` / 工具声明 / `tool_choice` 时整次改动回滚，该钩子本次算失败、不再收到 onResponse。
//!   钩子不能拒发请求。
//! - `export async function onResponse(reply, ctx)`：回复交给调用方之后，拿到整理成该协议完整
//!   回复形状的副本（流式回复边转发边留副本，最多 8 MiB），`ctx.success` 说明是否完整收到；
//!   发完即走，不等它。
//!
//! 钩子跑在一个常驻的 Node / Bun 进程里（宿主脚本 `js_hooks_host.mjs`，一行一个 JSON），
//! 网关不内嵌 JS 引擎。`ctx` 只有协议、模型、是否流式、路径、agent、会话、`kind`（子代理）、
//! `origin`（local / gateway-key / remote）、`options` 与每个钩子自己的 `state`；不给请求头和凭据。
//! 日志只记固定结果码，不记钩子内容。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::response::Response;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::gateway::middleware::Middleware;

pub(crate) const KIND: &str = "js-hook";
const DEFAULT_TIMEOUT_MS: u64 = 1000;
const CAPTURE_MAX: usize = 8 * 1024 * 1024;
/// 改了就回滚的字段：网关按它们查过 Key 的模型权限，调用方按它们读回复。
const PROTECTED: [&str; 5] = ["model", "stream", "tools", "tool_choice", "functions"];

#[derive(Clone, Debug)]
pub(crate) struct Hook {
    name: String,
    path: PathBuf,
    timeout_ms: u64,
    options: Value,
}

/// 超出 100–30000 的夹到边上，不是整数的用默认值。
fn timeout_of(config: &Value) -> u64 {
    match config.get("timeoutMs") {
        Some(v) => v.as_u64().map(|n| n.clamp(100, 30_000)).or_else(|| v.as_i64().map(|_| 100)).unwrap_or(DEFAULT_TIMEOUT_MS),
        None => DEFAULT_TIMEOUT_MS,
    }
}

/// 启用的 JS 钩子（按中间件顺序）。
pub(crate) fn hooks_of(middlewares: &[Middleware]) -> Vec<Hook> {
    middlewares
        .iter()
        .filter(|m| m.enabled && m.kind == KIND)
        .filter_map(|m| {
            let path = m.config.get("path")?.as_str()?.trim();
            (!path.is_empty()).then(|| Hook {
                name: m.name.clone(),
                path: PathBuf::from(path),
                timeout_ms: timeout_of(&m.config),
                options: m.config.get("options").cloned().unwrap_or_else(|| json!({})),
            })
        })
        .collect()
}

/// 保存前的校验：路径是存在的 .js / .mjs 文件，超时是数字。
pub(crate) fn validate(config: &Value) -> Result<(), String> {
    let path = config.get("path").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if path.is_empty() {
        return Err("js-hook 需要 path（钩子脚本的完整路径）".into());
    }
    let file = std::path::Path::new(path);
    if !file.is_absolute() {
        return Err("js-hook 的 path 必须是完整路径".into());
    }
    if !matches!(file.extension().and_then(|e| e.to_str()), Some("js" | "mjs")) {
        return Err("js-hook 只接受 .js 或 .mjs 文件".into());
    }
    if !file.is_file() {
        return Err(format!("找不到钩子脚本：{path}"));
    }
    if config.get("timeoutMs").is_some_and(|v| !v.is_number()) {
        return Err("timeoutMs 必须是毫秒数".into());
    }
    if config.get("options").is_some_and(|v| !v.is_object()) {
        return Err("options 必须是 JSON 对象".into());
    }
    Ok(())
}

/// 修改时间当版本号：改了钩子文件，宿主下次重新加载。
fn version_of(path: &std::path::Path) -> String {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default()
}

/* ───────────────────────────── 宿主进程 ───────────────────────────── */

struct Host {
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    alive: AtomicBool,
    _child: Mutex<tokio::process::Child>,
}

static HOST: LazyLock<tokio::sync::Mutex<Option<Arc<Host>>>> = LazyLock::new(|| tokio::sync::Mutex::new(None));
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
const HOST_SCRIPT: &str = include_str!("js_hooks_host.mjs");

/// 优先 Bun（magpie 同款），没有就用 Node。
fn runtime() -> Option<PathBuf> {
    crate::session_threads::find_executable("bun").or_else(|| crate::session_threads::find_executable("node"))
}

async fn host() -> Result<Arc<Host>, &'static str> {
    let mut guard = HOST.lock().await;
    if let Some(host) = guard.as_ref().filter(|h| h.alive.load(Ordering::Relaxed)) {
        return Ok(host.clone());
    }
    let runtime = runtime().ok_or("no-runtime")?;
    let script = std::env::temp_dir().join(format!("jolu-js-hooks-host-{}.mjs", env!("CARGO_PKG_VERSION")));
    if std::fs::read_to_string(&script).ok().as_deref() != Some(HOST_SCRIPT) {
        std::fs::write(&script, HOST_SCRIPT).map_err(|_| "host-write")?;
    }
    let mut command = tokio::process::Command::new(runtime);
    command
        .arg(&script)
        .current_dir(std::env::temp_dir())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command.spawn().map_err(|_| "host-spawn")?;
    let stdin = child.stdin.take().ok_or("host-spawn")?;
    let stdout = child.stdout.take().ok_or("host-spawn")?;
    let host = Arc::new(Host {
        stdin: tokio::sync::Mutex::new(stdin),
        pending: Mutex::new(HashMap::new()),
        alive: AtomicBool::new(true),
        _child: Mutex::new(child),
    });
    let reader = host.clone();
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
            let Some(id) = value["id"].as_u64() else { continue };
            if let Some(sender) = reader.pending.lock().ok().and_then(|mut p| p.remove(&id)) {
                let _ = sender.send(value);
            }
        }
        // 宿主退出：等着的调用全部按失败处理，下一次请求重新拉起
        reader.alive.store(false, Ordering::Relaxed);
        if let Ok(mut pending) = reader.pending.lock() {
            pending.clear();
        }
    });
    *guard = Some(host.clone());
    Ok(host)
}

async fn send(host: &Host, message: &Value) -> Result<(), &'static str> {
    use tokio::io::AsyncWriteExt;
    let line = format!("{message}\n");
    let mut stdin = host.stdin.lock().await;
    stdin.write_all(line.as_bytes()).await.map_err(|_| "host-gone")?;
    stdin.flush().await.map_err(|_| "host-gone")
}

/// 发一条消息并等回复（`wait` 内）。
async fn call(mut message: Value, wait: Duration) -> Result<Value, &'static str> {
    let host = host().await?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    message["id"] = json!(id);
    let (tx, rx) = oneshot::channel();
    host.pending.lock().map_err(|_| "host-gone")?.insert(id, tx);
    if let Err(code) = send(&host, &message).await {
        host.alive.store(false, Ordering::Relaxed);
        return Err(code);
    }
    let reply = tokio::time::timeout(wait, rx).await;
    if let Ok(mut pending) = host.pending.lock() {
        pending.remove(&id);
    }
    match reply {
        Ok(Ok(value)) if value["ok"] == true => Ok(value),
        Ok(Ok(value)) => Err(match value["code"].as_str() {
            Some("timeout") => "timeout",
            Some("load") => "load",
            _ => "threw",
        }),
        Ok(Err(_)) => Err("host-gone"),
        Err(_) => Err("timeout"),
    }
}

/* ───────────────────────────── onRequest ───────────────────────────── */

/// 请求的上下文（不含请求头与凭据）。
pub(crate) struct RequestCtx {
    pub protocol: &'static str,
    pub path: String,
    pub agent: Option<String>,
    pub session: Option<String>,
    pub subagent: bool,
    pub origin: &'static str,
}

/// onRequest 之后留给 onResponse 的东西。
pub(crate) struct Accepted {
    run: String,
    protocol: &'static str,
    hooks: Vec<Hook>,
    ctx: Value,
    request: Value,
}

/// 钩子链的总时限：各钩子超时之和夹在 2–10 秒，单个钩子更长时取它。
fn budget(hooks: &[Hook]) -> Duration {
    let sum: u64 = hooks.iter().map(|h| h.timeout_ms).sum();
    let longest = hooks.iter().map(|h| h.timeout_ms).max().unwrap_or(0);
    Duration::from_millis(sum.clamp(2_000, 10_000).max(longest))
}

fn protected_changed(before: &Value, after: &Value) -> bool {
    PROTECTED.iter().any(|k| before.get(*k) != after.get(*k))
}

/// 依次跑 onRequest。返回（交给后续处理的请求体，留给 onResponse 的状态）。
pub(crate) async fn on_request(hooks: &[Hook], body: Bytes, ctx: RequestCtx) -> (Bytes, Option<Accepted>) {
    if hooks.is_empty() {
        return (body, None);
    }
    let Ok(original) = serde_json::from_slice::<Value>(&body) else { return (body, None) };
    if !original.is_object() {
        return (body, None);
    }
    let run = format!("run-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
    let ctx_value = json!({
        "protocol": ctx.protocol,
        "model": original["model"],
        "stream": original["stream"].as_bool() == Some(true),
        "path": ctx.path,
        "origin": ctx.origin,
        "agent": ctx.agent.unwrap_or_default(),
        "session": ctx.session.unwrap_or_default(),
        "kind": if ctx.subagent { "subagent" } else { "" },
    });
    let deadline = tokio::time::Instant::now() + budget(hooks);
    let mut current = original.clone();
    let mut changed = false;
    let mut passed = Vec::new();
    for hook in hooks {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            eprintln!("[JS 钩子] 「{}」未执行：钩子链超出总时限", hook.name);
            break;
        }
        let message = json!({
            "op": "request",
            "run": run,
            "hook": {"path": hook.path, "version": version_of(&hook.path), "timeoutMs": hook.timeout_ms, "options": hook.options},
            "body": current,
            "ctx": ctx_value,
        });
        let wait = left.min(Duration::from_millis(hook.timeout_ms + 500));
        match call(message, wait).await {
            Ok(reply) => match &reply["body"] {
                Value::Null => passed.push(hook.clone()),
                next @ Value::Object(_) if !protected_changed(&current, next) => {
                    current = next.clone();
                    changed = true;
                    passed.push(hook.clone());
                }
                Value::Object(_) => eprintln!("[JS 钩子] 「{}」改了模型 / 流式 / 工具声明，已回滚", hook.name),
                _ => eprintln!("[JS 钩子] 「{}」返回的不是对象，已回滚", hook.name),
            },
            Err(code) => eprintln!("[JS 钩子] 「{}」失败（{code}），已回滚", hook.name),
        }
    }
    let body = if changed { Bytes::from(current.to_string()) } else { body };
    let accepted = Accepted { run, protocol: ctx.protocol, hooks: passed, ctx: ctx_value, request: current };
    (body, Some(accepted))
}

/* ───────────────────────────── onResponse ───────────────────────────── */

/// 回复照常转给调用方，同时留一份副本；结束后整理成完整回复交给通过的钩子（发完即走）。
pub(crate) fn on_response(accepted: Option<Accepted>, response: Response) -> Response {
    let Some(accepted) = accepted else { return response };
    if accepted.hooks.is_empty() {
        tokio::spawn(async move {
            let _ = end(&accepted.run).await;
        });
        return response;
    }
    let ok_status = response.status().is_success();
    let streamed = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("event-stream"));
    let (parts, body) = response.into_parts();
    use futures_util::StreamExt;
    struct Tap {
        upstream: std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, axum::Error>> + Send>>,
        copy: Vec<Bytes>,
        size: usize,
        overflow: bool,
        accepted: Option<Accepted>,
    }
    let state = Tap { upstream: Box::pin(body.into_data_stream()), copy: Vec::new(), size: 0, overflow: false, accepted: Some(accepted) };
    let stream = futures_util::stream::unfold(state, move |mut tap| async move {
        match tap.upstream.next().await {
            Some(Ok(chunk)) => {
                tap.size += chunk.len();
                if tap.size > CAPTURE_MAX {
                    tap.overflow = true;
                    tap.copy.clear();
                } else if !tap.overflow {
                    tap.copy.push(chunk.clone());
                }
                Some((Ok::<Bytes, axum::Error>(chunk), tap))
            }
            other => {
                let failed = matches!(other, Some(Err(_)));
                if let Some(accepted) = tap.accepted.take() {
                    let success = ok_status && !failed && !tap.overflow;
                    let bytes: Vec<u8> = tap.copy.concat();
                    tokio::spawn(notify(accepted, bytes, streamed, success));
                }
                match other {
                    Some(Err(error)) => Some((Err(error), tap)),
                    _ => None,
                }
            }
        }
    });
    Response::from_parts(parts, Body::from_stream(stream))
}

async fn notify(accepted: Accepted, bytes: Vec<u8>, streamed: bool, success: bool) {
    let reply = if success { consolidate(accepted.protocol, &bytes, streamed) } else { None };
    let mut ctx = accepted.ctx.clone();
    ctx["request"] = accepted.request.clone();
    ctx["success"] = json!(success && reply.is_some());
    for hook in &accepted.hooks {
        let message = json!({
            "op": "response",
            "run": accepted.run,
            "hook": {"path": hook.path, "version": version_of(&hook.path), "timeoutMs": hook.timeout_ms, "options": hook.options},
            "reply": reply.clone().unwrap_or(Value::Null),
            "ctx": ctx,
        });
        if let Err(code) = call(message, Duration::from_millis(hook.timeout_ms + 500)).await {
            eprintln!("[JS 钩子] 「{}」的 onResponse 失败（{code}）", hook.name);
        }
    }
    let _ = end(&accepted.run).await;
}

async fn end(run: &str) -> Result<(), &'static str> {
    let guard = HOST.lock().await;
    let Some(host) = guard.as_ref().filter(|h| h.alive.load(Ordering::Relaxed)).cloned() else { return Ok(()) };
    drop(guard);
    send(&host, &json!({"op": "end", "run": run})).await
}

/// 关掉宿主（关 stdin 让它自己退出）。Windows 上子进程输出在阻塞线程池里读，
/// 宿主不退出的话测试运行时收尾会一直等这次读。
#[cfg(test)]
async fn shutdown() {
    if let Some(host) = HOST.lock().await.take() {
        host.alive.store(false, Ordering::Relaxed);
        use tokio::io::AsyncWriteExt;
        let _ = host.stdin.lock().await.shutdown().await;
        if let Ok(mut child) = host._child.lock() {
            let _ = child.start_kill();
        }
    }
}

/// SSE 文本里每个事件的 JSON。
fn sse_events(text: &str) -> Vec<Value> {
    text.replace("\r\n", "\n")
        .split("\n\n")
        .filter_map(|block| {
            let data: Vec<&str> = block.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim_start).collect();
            serde_json::from_str(&data.join("\n")).ok()
        })
        .collect()
}

/// 把回复整理成该协议的完整回复形状（非流式就是原 JSON）。
fn consolidate(protocol: &str, bytes: &[u8], streamed: bool) -> Option<Value> {
    if !streamed {
        return serde_json::from_slice(bytes).ok();
    }
    let text = String::from_utf8_lossy(bytes);
    let events = sse_events(&text);
    match protocol {
        "anthropic" => (!events.is_empty()).then(|| crate::gateway::adapter::assemble_anthropic_events(&events)),
        "responses" => events.iter().rev().find(|e| e["type"] == "response.completed").map(|e| e["response"].clone()),
        _ => chat_from_chunks(&events),
    }
}

/// Chat 流式片段 → 完整的 chat.completion。
fn chat_from_chunks(chunks: &[Value]) -> Option<Value> {
    let first = chunks.first()?;
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut calls: Vec<Value> = Vec::new();
    let mut finish = Value::Null;
    let mut usage = Value::Null;
    for chunk in chunks {
        if chunk["usage"].is_object() {
            usage = chunk["usage"].clone();
        }
        let choice = &chunk["choices"][0];
        if !choice["finish_reason"].is_null() {
            finish = choice["finish_reason"].clone();
        }
        let delta = &choice["delta"];
        content.push_str(delta["content"].as_str().unwrap_or_default());
        reasoning.push_str(delta["reasoning_content"].as_str().unwrap_or_default());
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = call["index"].as_u64().unwrap_or(0) as usize;
            while calls.len() <= index {
                calls.push(json!({"id": "", "type": "function", "function": {"name": "", "arguments": ""}}));
            }
            let slot = &mut calls[index];
            if let Some(id) = call["id"].as_str() {
                slot["id"] = json!(id);
            }
            for key in ["name", "arguments"] {
                if let Some(piece) = call["function"][key].as_str() {
                    let joined = format!("{}{piece}", slot["function"][key].as_str().unwrap_or_default());
                    slot["function"][key] = json!(joined);
                }
            }
        }
    }
    let mut message = json!({"role": "assistant", "content": content});
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !calls.is_empty() {
        message["tool_calls"] = json!(calls);
    }
    let mut out = json!({
        "id": first["id"], "object": "chat.completion", "created": first["created"], "model": first["model"],
        "choices": [{"index": 0, "message": message, "finish_reason": finish}],
    });
    if usage.is_object() {
        out["usage"] = usage;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(name: &str, script: &str, timeout_ms: u64) -> Hook {
        let dir = std::env::temp_dir().join(format!("jolu-js-hooks-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.mjs"));
        std::fs::write(&path, script).unwrap();
        Hook { name: name.into(), path, timeout_ms, options: json!({"tag": name}) }
    }

    fn ctx() -> RequestCtx {
        RequestCtx { protocol: "chat", path: "/v1/chat/completions".into(), agent: Some("codex".into()), session: None, subagent: false, origin: "local" }
    }

    #[test]
    fn timeouts_are_clamped_and_budgets_bounded() {
        assert_eq!(timeout_of(&json!({})), 1000);
        assert_eq!(timeout_of(&json!({"timeoutMs": 50})), 100);
        assert_eq!(timeout_of(&json!({"timeoutMs": 99_999})), 30_000);
        assert_eq!(timeout_of(&json!({"timeoutMs": 1.5})), 1000);
        let short = |ms| Hook { name: "h".into(), path: PathBuf::new(), timeout_ms: ms, options: json!({}) };
        assert_eq!(budget(&[short(100)]), Duration::from_secs(2));
        assert_eq!(budget(&[short(8000), short(8000)]), Duration::from_secs(10));
        assert_eq!(budget(&[short(15_000)]), Duration::from_secs(15), "单个钩子更长时按它");
    }

    #[test]
    fn chat_chunks_and_responses_streams_consolidate() {
        let chat = "data: {\"id\":\"c\",\"model\":\"m\",\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n\ndata: {\"id\":\"c\",\"choices\":[{\"delta\":{\"content\":\"好\",\"tool_calls\":[{\"index\":0,\"id\":\"t\",\"function\":{\"name\":\"f\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";
        let reply = consolidate("chat", chat.as_bytes(), true).unwrap();
        assert_eq!(reply["choices"][0]["message"]["content"], "你好");
        assert_eq!(reply["choices"][0]["message"]["tool_calls"][0]["function"]["name"], "f");
        assert_eq!(reply["choices"][0]["finish_reason"], "tool_calls");
        let responses = "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\"}}\n\n";
        assert_eq!(consolidate("responses", responses.as_bytes(), true).unwrap()["id"], "r");
    }

    /// 真跑宿主（本仓库开发环境有 Node）：改写、回滚（抛错 / 改模型 / 超时）与 onResponse。
    #[tokio::test]
    async fn hooks_rewrite_roll_back_and_observe_replies() {
        let out = std::env::temp_dir().join(format!("jolu-js-hooks-reply-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let add = hook("add", "export async function onRequest(body, ctx) { ctx.state.seen = ctx.options.tag; return {...body, metadata: {from: ctx.protocol + ':' + ctx.origin}} }\nexport async function onResponse(reply, ctx) { const fs = await import('node:fs'); fs.writeFileSync(process.env.JOLU_TEST_OUT || '', JSON.stringify({reply, success: ctx.success, state: ctx.state, model: ctx.request.model})) }", 2000);
        let model = hook("model", "export function onRequest(body) { return {...body, model: 'other'} }", 1000);
        let threw = hook("threw", "export function onRequest() { throw new Error('secret text') }", 1000);
        let slow = hook("slow", "export function onRequest() { return new Promise(() => {}) }", 150);
        // 宿主进程继承环境：onResponse 写到哪由环境变量告诉它（只在测试里设）
        std::env::set_var("JOLU_TEST_OUT", &out);
        let body = Bytes::from(json!({"model": "a/m", "messages": [{"role": "user", "content": "hi"}]}).to_string());
        let (next, accepted) = on_request(&[add, model, threw, slow], body, ctx()).await;
        let next: Value = serde_json::from_slice(&next).unwrap();
        assert_eq!(next["metadata"]["from"], "chat:local", "第一个钩子的改写生效");
        assert_eq!(next["model"], "a/m", "改模型的钩子被回滚");
        let accepted = accepted.unwrap();
        assert_eq!(accepted.hooks.len(), 1, "失败的钩子不再收到 onResponse");
        let reply = Response::builder()
            .header("content-type", "application/json")
            .body(Body::from(r#"{"choices":[{"message":{"content":"ok"}}]}"#))
            .unwrap();
        let passed = on_response(Some(accepted), reply);
        let text = axum::body::to_bytes(passed.into_body(), 1 << 20).await.unwrap();
        assert!(String::from_utf8_lossy(&text).contains("ok"), "调用方照常拿到回复");
        let mut seen = None;
        for _ in 0..100 {
            if let Ok(raw) = std::fs::read_to_string(&out) {
                if !raw.is_empty() {
                    seen = Some(raw);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let seen: Value = serde_json::from_str(&seen.expect("onResponse 应当被调用")).unwrap();
        assert_eq!(seen["reply"]["choices"][0]["message"]["content"], "ok");
        assert_eq!(seen["success"], true);
        assert_eq!(seen["state"]["seen"], "add", "同一次请求的 state 在两个回调之间共用");
        assert_eq!(seen["model"], "a/m");
        let _ = std::fs::remove_file(&out);
        shutdown().await;
    }

    #[test]
    fn validation_needs_an_existing_absolute_script() {
        assert!(validate(&json!({})).is_err());
        assert!(validate(&json!({"path": "relative.mjs"})).is_err());
        let h = hook("valid", "export function onRequest() {}", 1000);
        assert!(validate(&json!({"path": h.path, "timeoutMs": 500})).is_ok());
        assert!(validate(&json!({"path": h.path, "options": []})).is_err());
        let txt = h.path.with_extension("txt");
        std::fs::write(&txt, "").unwrap();
        assert!(validate(&json!({"path": txt})).is_err());
    }
}
