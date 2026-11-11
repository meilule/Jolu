//! 调用方协议封装：解析模型 → 路由选连接 → 协议匹配 → 适配器转发 → 按协议回封。
//! 跨协议组合返回明确的 400（不做静默翻译，见执行文档 §5.4）。

use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;
use axum::response::{IntoResponse, Response};

use crate::gateway::activity::RouteMeta;
use crate::gateway::route::{self, RouteDecision};
use crate::proxy::ProxyDeps;

/// 调用方入口协议，由请求路径判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallerProtocol {
    /// Anthropic Messages（`/v1/messages`）。
    Anthropic,
    /// OpenAI Chat Completions（`/v1/chat/completions`）。
    Chat,
    /// OpenAI Responses（`/v1/responses`）。
    Responses,
}

impl CallerProtocol {
    /// 连接 protocol 字段的对应值（直通匹配用）。
    fn wire_name(self) -> &'static str {
        match self {
            CallerProtocol::Anthropic => "anthropic",
            CallerProtocol::Chat | CallerProtocol::Responses => "openai",
        }
    }
}

/// 错误响应（JSON，保持依赖面不变的手工拼装）。
pub(crate) fn error_response(status: StatusCode, message: &str) -> Response {
    let body = format!(
        "{{\"error\":{{\"type\":\"jolu_gateway\",\"message\":\"{}\"}}}}",
        message.replace('"', "\\\"")
    );
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

/// 带 Retry-After 的错误响应：限流时告诉 agent 该等多久，而不是立刻重试。
pub(crate) fn error_response_with_retry(status: StatusCode, message: &str, retry_after: u64) -> Response {
    let body = format!(
        "{{\"error\":{{\"type\":\"jolu_gateway\",\"message\":\"{}\"}}}}",
        message.replace('"', "\\\"")
    );
    let mut response = axum::http::Response::new(axum::body::Body::from(body));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        axum::http::header::RETRY_AFTER,
        axum::http::HeaderValue::from_str(&retry_after.to_string())
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("1")),
    );
    response
}

/// 成功与失败计一次熔断计数：连续失败达阈值（默认 5）且开启自动逃生时切回直连。
/// 与观测模式的 `note_upstream_unreachable` 同义，但阈值独立（`gateway_circuit_threshold`）。
pub(crate) fn note_outcome(deps: &Arc<ProxyDeps>, failed: bool, reason: &str) {
    use std::sync::atomic::Ordering;
    if !failed {
        deps.consecutive_failures.store(0, Ordering::Relaxed);
        return;
    }
    if !deps.fallback_enabled {
        return;
    }
    let count = deps.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
    if count >= deps.circuit_threshold {
        deps.consecutive_failures.store(0, Ordering::Relaxed);
        (deps.on_circuit)(reason);
    }
}

/// 从请求体提取 model 字段（任何协议都在顶层）。
fn extract_model(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()?
        .get("model")?
        .as_str()
        .map(str::to_string)
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 200)
        .map(str::to_string)
}

/// Anthropic 的 `metadata.user_id` 可能是整段 JSON（Claude Code 把 device_id 和
/// session_id 塞在一起），优先取出其中的 session_id。
fn anthropic_user_session(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let user = value.pointer("/metadata/user_id")?.as_str()?.trim();
    if user.is_empty() {
        return None;
    }
    if let Ok(inner) = serde_json::from_str::<serde_json::Value>(user) {
        if let Some(session) = inner
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(session.to_string());
        }
    }
    Some(user.to_string())
}

fn prompt_cache_key(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    value
        .get("prompt_cache_key")?
        .as_str()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

/// 提取会话亲和键。优先级：
/// `X-Jolu-Session` → Claude 的 `metadata.user_id` → Codex `session-id` / `thread-id` /
/// `x-client-request-id` → 通用会话头 → 请求体 `prompt_cache_key` → 调用方令牌 + slug。
///
/// token 只做最后兜底：Codex 不写 token 时，旧逻辑会把所有会话合并成 `tok::codex`，
/// 换来源等于每次都让 prompt cache 失效。
pub(crate) fn session_key(protocol: CallerProtocol, headers: &HeaderMap, body: &[u8], model: &str) -> String {
    if let Some(session) = header_value(headers, "x-jolu-session") {
        return format!("hdr:x-jolu-session:{session}");
    }
    if protocol == CallerProtocol::Anthropic {
        if let Some(session) = anthropic_user_session(body) {
            return format!("user:{session}");
        }
    }
    for (name, prefix) in [
        ("session-id", "codex-session"),
        ("thread-id", "codex-thread"),
        ("x-client-request-id", "codex-request"),
        ("x-session-id", "session"),
        ("x-session-affinity", "affinity"),
        ("x-claude-code-session-id", "claude-session"),
        ("x-conversation-id", "conversation"),
        ("x-opencode-session", "opencode-session"),
    ] {
        if let Some(session) = header_value(headers, name) {
            return format!("hdr:{prefix}:{session}");
        }
    }
    if let Some(key) = prompt_cache_key(body) {
        return format!("body:prompt-cache:{key}");
    }
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.strip_prefix("Bearer ").unwrap_or(s).trim().to_string())
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()).map(str::to_string))
        .unwrap_or_default();
    let slug = crate::gateway::route::split_model_id(model).map(|(s, _)| s).unwrap_or("");
    format!("tok:{token}:{slug}")
}

/// 会话 ID 只接受常见的标识字符，避免把整段 JSON 或异常长值当成会话。
fn plausible_session_id(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 128
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then(|| value.to_string())
}

/// Claude Code 的 `metadata.user_id`：新版是 JSON（含 session_id），旧版是
/// `user_<hash>_account_<uuid>_session_<uuid>`。两种都取不出会话时返回 None。
fn claude_session_from_user_id(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let user = value.pointer("/metadata/user_id")?.as_str()?.trim();
    if let Ok(inner) = serde_json::from_str::<serde_json::Value>(user) {
        return inner.get("session_id").and_then(|v| v.as_str()).and_then(plausible_session_id);
    }
    user.rsplit_once("_session_").and_then(|(_, session)| plausible_session_id(session))
}

/// 已验证的调用方会话（统一网关开发说明 §6.1）。只认工具原生携带、可与会话线程对应的会话 ID：
/// - Claude Code：`X-Claude-Code-Session-Id` 头，或 `metadata.user_id` 里的 session_id；
/// - Codex：`session-id`（根会话；子线程请求也归到用户看到的根会话）；
/// - OpenCode：`x-opencode-session-id`（子会话带 `x-opencode-parent-session-id`，归到父会话），
///   统计里的会话 ID 带 `opencode:` 前缀（见 `opencode_usage.rs`），这里按同一格式拼接。
///
/// 请求级 ID（`x-client-request-id`）、`prompt_cache_key`、通用亲和头和令牌兜底都只用于亲和，
/// 不能当成会话身份；识别不出 Agent 时也不猜。
pub(crate) fn session_identity(agent: Option<&str>, headers: &HeaderMap, body: &[u8]) -> Option<crate::gateway::activity::SessionRef> {
    let (platform, session) = match agent? {
        "claude-code" => (
            "claude",
            header_value(headers, "x-claude-code-session-id")
                .and_then(|v| plausible_session_id(&v))
                .or_else(|| claude_session_from_user_id(body))?,
        ),
        "codex" => (
            "codex",
            header_value(headers, "session-id")
                .or_else(|| header_value(headers, "session_id"))
                .and_then(|v| plausible_session_id(&v))?,
        ),
        "opencode" => (
            "opencode",
            header_value(headers, "x-opencode-parent-session-id")
                .or_else(|| header_value(headers, "x-opencode-session-id"))
                .and_then(|v| plausible_session_id(&v))
                .map(|id| format!("opencode:{id}"))?,
        ),
        _ => return None,
    };
    Some(crate::gateway::activity::SessionRef { platform: platform.into(), session_id: session })
}

/// 从调用方令牌 / 客户端头识别是哪个 agent（只用于路由流水，不做鉴权）。
/// Codex 伪装成 Claude Code 的纠正（magpie `codexForClaudeCode` 同款）。
///
/// 为什么需要：OpenAI 的 `/codex:review` 插件在 Claude Code 里跑 `codex app-server`，
/// 它的 originator 和 UA 都写成 "Claude Code/0.162.1 ..."——按 originator 或 UA
/// 判断会把 Codex 的请求记成 Claude Code，用量归错 agent（现场记录 wyztony on X：
/// Codex 的调用量从来没记上）。
///
/// 判定信号：Codex 每轮都带 `x-codex-window-id` 或 `x-codex-turn-metadata`，
/// 而 Claude Code 自己的请求（`claude-cli/…`）从不带这两个头。两者同时成立
/// 才判定为 Codex——单靠 UA 写 "Claude Code" 不够，防止未来 Claude Code
/// 也发类似头时误判。
///
/// 参照 magpie `internal/gateway/gateway.go::codexForClaudeCode`（commit `8016124`）。
fn codex_masquerading_as_claude(headers: &HeaderMap) -> bool {
    let has_codex_marker = headers.contains_key("x-codex-window-id")
        || headers.contains_key("x-codex-turn-metadata");
    if !has_codex_marker {
        return false;
    }
    ["user-agent", "originator"].iter().any(|name| {
        headers
            .get(*name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_ascii_lowercase().starts_with("claude code"))
            .unwrap_or(false)
    })
}

/// VS Code 的 Agents 窗口用 `vscode_agent_host` 名字跑 Codex（Codex 自己的
/// `codexAgent.ts` clientInfo），UA 和 originator 都写这个名字。
/// 不认的话 Codex 自己的模型在那里被问 → 404 而不是走 Codex 的登录。
///
/// 参照 magpie `internal/gateway/gateway.go::hostedCodex`。
fn hosted_codex(headers: &HeaderMap) -> bool {
    ["user-agent", "originator"].iter().any(|name| {
        headers
            .get(*name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_ascii_lowercase().starts_with("vscode_agent_host"))
            .unwrap_or(false)
    })
}

/// agent 识别结果：agent id + 识别依据 + 识别不出时的 UA 首段。
///
/// 识别依据让「显示不对」可以定位是识别规则的问题还是工具本身的问题——
/// `token`（接线 token 带 agent 名）/ `originator` / `ua` / `codex_marker`。
/// `unidentified_ua` 对齐 magpie `AgentOf` 最后一行 `return name`：
/// 识别不出时不返回 None 了事，而是把 UA 首段给出来（截 64 字符），
/// 用户报问题时能直接看出是哪个工具漏了。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AgentIdentity {
    pub agent: Option<String>,
    pub identified_by: Option<&'static str>,
    pub unidentified_ua: Option<String>,
}

impl AgentIdentity {
    fn identified(agent: &str, by: &'static str) -> Self {
        Self { agent: Some(agent.into()), identified_by: Some(by), unidentified_ua: None }
    }
    fn unknown(ua: &str) -> Self {
        // UA 首段（到 / 或空格），截 64 字符——足够看出是哪个工具，不存完整 UA
        let first = ua.split(['/', ' ']).next().unwrap_or(ua);
        let truncated: String = first.chars().take(64).collect();
        Self { agent: None, identified_by: None, unidentified_ua: (!truncated.is_empty()).then_some(truncated) }
    }
}

/// 从调用方令牌 / 客户端头识别是哪个 agent（只用于路由流水与灵动岛，不做鉴权）。
pub(crate) fn agent_from_headers(headers: &HeaderMap) -> AgentIdentity {
    // 反伪装先于一切：token / originator / UA 都可能被 Codex 插件写成 "Claude Code"，
    // 但 Codex 特有的头藏不住——先纠正再走正常分支
    if codex_masquerading_as_claude(headers) {
        return AgentIdentity::identified("codex", "codex_marker");
    }
    if hosted_codex(headers) {
        return AgentIdentity::identified("codex", "vscode_agent_host");
    }
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.strip_prefix("Bearer ").unwrap_or(s).trim())
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()).map(str::trim));
    if let Some(agent) = token
        .and_then(|token| token.strip_prefix("jolu-"))
        .and_then(|rest| rest.split('.').next())
        .filter(|agent| crate::gateway::wiring::AGENTS.contains(agent))
    {
        return AgentIdentity::identified(agent, "token");
    }
    if let Some(originator) = headers.get("originator").and_then(|v| v.to_str().ok()) {
        let originator = originator.to_ascii_lowercase();
        if originator.contains("codex") {
            return AgentIdentity::identified("codex", "originator");
        }
        if originator.contains("claude") {
            return AgentIdentity::identified("claude-code", "originator");
        }
        if originator.contains("gemini") {
            return AgentIdentity::identified("gemini", "originator");
        }
    }
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or_default();
    // Gemini CLI 旧版 UA 以 GeminiCLI 开头；0.65 起改为 CloudCodeVSCode/…（…proxy_client=geminicli）
    if ua.starts_with("GeminiCLI") || ua.to_ascii_lowercase().contains("proxy_client=geminicli") {
        return AgentIdentity::identified("gemini", "ua");
    }
    if ua.starts_with("codex_cli_rs") {
        return AgentIdentity::identified("codex", "ua");
    }
    if ua.starts_with("claude-cli/") {
        return AgentIdentity::identified("claude-code", "ua");
    }
    // OpenCode 每个模型请求都带 x-opencode-client / x-opencode-session-id
    if ua.to_ascii_lowercase().starts_with("opencode/")
        || headers.contains_key("x-opencode-client")
        || headers.contains_key("x-opencode-session-id")
    {
        return AgentIdentity::identified("opencode", "ua");
    }
    // 识别不出：给 UA 首段，让用户报问题时能直接看出是哪个工具漏了
    AgentIdentity::unknown(ua)
}

/// 写一条路由流水并发结束事件。事件先于写库发出：数据库忙、这一条没写进库时界面仍能收到。
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_route(
    deps: &Arc<ProxyDeps>,
    request_id: u64,
    meta: &RouteMeta,
    source: Option<&str>,
    upstream: Option<&str>,
    status: Option<u16>,
    result: &str,
    started: std::time::Instant,
) {
    use crate::gateway::activity::{emit_end, RouteEnd};
    let duration_ms = started.elapsed().as_millis() as i64;
    emit_end(
        &deps.on_activity,
        &RouteEnd {
            request_id,
            ts: chrono::Local::now().timestamp_millis(),
            agent: meta.agent.as_deref(),
            identified_by: meta.identified_by,
            unidentified_ua: meta.unidentified_ua.as_deref(),
            client_key: meta.client_key.as_deref(),
            requested_model: &meta.requested_model,
            source,
            upstream_model: upstream,
            status_code: status,
            result,
            duration_ms,
            connection_id: meta.connection_id.as_deref(),
            session_platform: meta.session_platform(),
            session_id: meta.session_id(),
        },
    );
    let Ok(conn) = deps.db.try_lock() else { return };
    crate::gateway::route_log::record(
        &conn,
        crate::gateway::route_log::NewEntry {
            agent: meta.agent.as_deref(),
            requested_model: &meta.requested_model,
            source,
            upstream_model: upstream,
            status_code: status.map(i64::from),
            result,
            duration_ms: Some(duration_ms),
            connection_id: meta.connection_id.as_deref(),
            session_platform: meta.session_platform(),
            session_id: meta.session_id(),
            client_key: meta.client_key.as_deref(),
            request_id: Some(request_id as i64),
            // 不带来源前缀、也不是分组的模型 id 走的是裸名解析（目录里第一个暴露它的来源 / 默认来源）
            resolved_from: (!meta.requested_model.contains('/')).then_some("bare"),
        },
    );
}

/// 选路成功、交给上游之前：发开始事件；返回的 guard 保证请求中途被取消时也有结束事件。
pub(crate) fn begin_route(
    deps: &Arc<ProxyDeps>,
    request_id: u64,
    meta: &RouteMeta,
    entry: &route::PoolEntry,
    started: std::time::Instant,
) -> crate::gateway::activity::EndGuard {
    crate::gateway::activity::emit_start(
        &deps.on_activity,
        &crate::gateway::activity::RouteStart {
            request_id,
            ts: chrono::Local::now().timestamp_millis(),
            agent: meta.agent.as_deref(),
            identified_by: meta.identified_by,
            unidentified_ua: meta.unidentified_ua.as_deref(),
            client_key: meta.client_key.as_deref(),
            requested_model: &meta.requested_model,
            source: &entry.slug,
            connection_id: meta.connection_id.as_deref(),
            session_platform: meta.session_platform(),
            session_id: meta.session_id(),
        },
    );
    crate::gateway::activity::EndGuard::new(deps.on_activity.clone(), request_id, meta, &entry.slug, started)
}

/// 选中连接后的元数据（连接 id 只在命中来源时才有）。
pub(crate) fn linked(meta: &RouteMeta, entry: &route::PoolEntry) -> RouteMeta {
    RouteMeta { connection_id: Some(entry.connection_id.clone()), ..meta.clone() }
}

/// 网关运行态（亲和 + 冷却）的借用引用，避免在 async 里长期持锁。
struct GatewayState<'a> {
    affinity: &'a Mutex<crate::gateway::route::Affinity>,
    cooldowns: &'a Mutex<crate::gateway::route::Cooldowns>,
}

/// 统一入口。`path_and_query` 原样拼到上游 base 后（路径参数与查询都保真）。
/// `client_key` 是鉴权中间件命中的命名客户端 Key（局域网共享时）；只进路由流水。
pub(crate) async fn handle(
    protocol: CallerProtocol,
    deps: Arc<ProxyDeps>,
    headers: HeaderMap,
    mut body: axum::body::Bytes,
    path_and_query: &str,
    client_key: Option<String>,
) -> Response {
    let started = std::time::Instant::now();
    // 内联字节统一成标准 base64（base64_input.rs）：在任何人读请求体之前做
    body = crate::gateway::base64_input::standardize_body(body);
    let request_id = crate::gateway::activity::next_request_id();
    let identity = agent_from_headers(&headers);
    let agent = identity.agent.clone();
    // 中间件链（P6.1）：model-map / param-override 在选路之前执行，失败隔离——
    // 单个中间件出错只跳过自己；映射后的 id 是路由、流水与用量记账的 id。
    let middleware_list = deps
        .db
        .lock()
        .ok()
        .and_then(|conn| crate::gateway::middleware::list(&conn).ok())
        .unwrap_or_default();
    let mut body_value = serde_json::from_slice::<Value>(&body).ok();
    let middleware_outcome = body_value.as_mut().map(|value| crate::gateway::middleware::apply_chain(&middleware_list, value));
    if let Some(value) = body_value.as_ref() {
        if let Ok(bytes) = serde_json::to_vec(value) {
            body = bytes.into();
        }
    }
    if let Some(outcome) = &middleware_outcome {
        if let Some(from) = &outcome.mapped_from {
            let to = extract_model(&body).unwrap_or_default();
            eprintln!("[中间件] model-map：{from} → {to}");
        }
        for reason in &outcome.skipped {
            eprintln!("[中间件] 跳过：{reason}");
        }
        // word-guard 命中阻断词：进 provider 之前拒发（与 magpie 的中间件语义一致）
        if let Some(reason) = &outcome.blocked {
            return error_response(StatusCode::BAD_REQUEST, reason);
        }
    }
    let Some(raw_model) = extract_model(&body) else {
        return error_response(StatusCode::BAD_REQUEST, "请求体缺少 model 字段或不是有效 JSON");
    };
    // 规范名 = 剥掉 Claude Code 的 [1m]/[1M] 窗口标记。路由、流水、用量、上游
    // 全部用规范名，避免同一模型在统计里出现两行、上游收到不认识的后缀。
    let model = route::canonical_model(&raw_model).to_string();
    // 命名 Key 的模型白名单：中间件在 body 之前跑，拿不到模型名，所以在读到模型后补这道检查
    if let Some(name) = client_key.as_deref() {
        if let Ok(conn) = deps.db.lock() {
            match crate::gateway::access::check_client_key_limit_for(
                &conn,
                name,
                chrono::Local::now(),
                Some(&model),
                true,
            ) {
                Ok(Some(refusal)) => return refusal_response(&refusal),
                Err(reason) => return error_response(StatusCode::SERVICE_UNAVAILABLE, &reason),
                Ok(None) => {}
            }
        }
    }
    // 命名 Key 的同时请求数（key_slots.rs）：满了按到达顺序排队；在预扣之前取，
    // 排队中客户端断开时不会留下预扣
    let cap = client_key.as_deref().and_then(|name| {
        deps.db.lock().ok().and_then(|conn| crate::gateway::access::client_key_concurrency(&conn, name))
    });
    let slot = match client_key.as_deref() {
        Some(name) => crate::gateway::key_slots::acquire(name, cap).await,
        None => None,
    };
    // 实际路由到的（平台，模型）：候选循环填，响应结束时计费用（budget.rs）
    let routed: crate::gateway::budget::Routed = Default::default();
    // 命名 Key 的账号范围：只能用这些连接
    let allowed_accounts = client_key.as_deref().and_then(|name| {
        deps.db.lock().ok().and_then(|conn| crate::gateway::access::client_key_accounts(&conn, name))
    });
    // 命名 Key 的 Token / 费用预算（budget.rs）：先按估算预扣，响应结束按实际用量回冲
    let reservation = match client_key.as_deref() {
        Some(name) => match deps.db.lock() {
            Ok(conn) => match crate::gateway::budget::reserve(&conn, name, &body, chrono::Local::now(), routed.clone()) {
                Ok(reservation) => reservation,
                Err(refusal) => return refusal_response(&refusal),
            },
            Err(_) => return error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试"),
        },
        None => None,
    };
    let budget_db = deps.db.clone();
    // 以下选路与转发包在一个 async 块里：块内的 `return` 只结束这个块，
    // 返回的响应统一在块外套上 Token 计量。
    let response = async {
    let meta = RouteMeta {
        session: session_identity(agent.as_deref(), &headers, &body),
        agent: agent.clone(),
        identified_by: identity.identified_by,
        unidentified_ua: identity.unidentified_ua.clone(),
        requested_model: model.clone(),
        connection_id: None,
        client_key,
    };

    // 模型池快照：拿不到数据库锁时沿用旧索引（与观测模式 KeyIndex 同策略）
    let pool = {
        let guard = match deps.pool_index() {
            Ok(g) => g,
            Err(_) => return error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试"),
        };
        route::pool_snapshot(&deps.db, guard)
    };
    let settings_default = deps.gateway_default_connection();

    // 亲和参与选路：会话绑定连接被 429 冷却时不切换，直接报错（保 prompt cache）
    let state = match (deps.affinity.as_ref(), deps.cooldowns.as_ref()) {
        (Some(a), Some(c)) => GatewayState { affinity: a, cooldowns: c },
        _ => return error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试"),
    };
    let session = session_key(protocol, &headers, &body, &model);
    let decision = {
        // 旧 slug 只在未命中时查历史表；数据库锁忙时按未命中处理，不影响主路径。
        let history = |slug: &str| -> Option<String> {
            deps.db
                .try_lock()
                .ok()
                .and_then(|conn| route::historical_slug_message(&conn, slug))
        };
        // 裸模型名 → 发请求的 agent 自己的订阅优先，否则目录里第一个暴露它的来源（route::pick_bare）。
        // 只在模型 id 不带前缀时被调用，因此正常带前缀的请求不多这一次查询。
        let bare = |name: &str| -> Option<String> {
            let conn = deps.db.lock().ok()?;
            let catalog = crate::gateway::catalog::list(&conn).ok()?;
            route::pick_bare(
                catalog.iter().map(|entry| (entry.slug.as_str(), entry.platform.as_deref(), entry.kind.as_str(), entry.model.as_str())),
                name,
                agent.as_deref(),
            )
        };
        let mut affinity = match state.affinity.lock() {
            Ok(g) => g,
            Err(_) => return error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试"),
        };
        let cooldowns = match state.cooldowns.lock() {
            Ok(g) => g,
            Err(_) => return error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试"),
        };
        route::decide_with_affinity(
            &pool,
            &model,
            settings_default.as_deref(),
            Some(&session),
            &mut affinity,
            &cooldowns,
            &history,
            &bare,
        )
    };

    // ── 候选列表 ──
    // 普通模型：decide 产出一个候选。`group/<slug>`：路由分组按策略展开为有序候选
    // （主选 + 兜底），成员失败时按可转移错误换下一个——对齐 magpie 的 fallback 语义。
    let mut group_note: Option<String> = None;
    let candidates: Vec<(route::PoolEntry, String)> = if let Some(group_slug) = model.strip_prefix("group/") {
        let group = deps
            .db
            .lock()
            .ok()
            .and_then(|conn| crate::gateway::routing_groups::get(&conn, group_slug).ok().flatten());
        let Some(group) = group else {
            return error_response(StatusCode::NOT_FOUND, &format!("没有名为「{group_slug}」的路由分组"));
        };
        // 会话已绑定的成员提到最前（prompt cache 值得保留，与单模型亲和同语义）
        let bound = state.affinity.lock().ok().and_then(|affinity| {
            let connection_id = affinity.get(&session)?;
            pool.iter().find(|e| e.connection_id == connection_id).map(|e| e.slug.clone())
        });
        let usage_of = |slug: &str| -> i64 {
            deps.db
                .try_lock()
                .ok()
                .and_then(|conn| route::member_usage(&conn, slug).ok())
                .unwrap_or(0)
        };
        let (ordered, note) = {
            // smart / pace 的额度快照：成员 slug → 连接 id → 下一次重置时间
            let resets_of = |slug: &str| -> Option<i64> {
                let connection_id = pool.iter().find(|e| e.slug == slug)?.connection_id.clone();
                crate::gateway::quota_snapshot::resets_of(&connection_id)
            };
            let mut rotate = crate::gateway::routing_groups::rotate_state();
            // Cap hold：额度快照显示已用满（有窗口到 100% 且重置时间还没到）的成员降到末尾。
            // 快照缺用量百分比或重置时间时判定为「不知道」，不降权。
            let now_ms = chrono::Utc::now().timestamp_millis();
            let exhausted_of = |slug: &str| -> bool {
                pool.iter()
                    .find(|e| e.slug == slug)
                    .map(|e| e.connection_id.as_str())
                    .is_some_and(|id| crate::gateway::quota_snapshot::is_exhausted(id, now_ms))
            };
            crate::gateway::routing_groups::ordered_members(
                &group,
                &mut rotate,
                &usage_of,
                &resets_of,
                &exhausted_of,
                now_ms,
                bound.as_deref(),
            )
        };
        group_note = note;
        let (expanded, skipped) = crate::gateway::routing_groups::expand(&ordered, 0, &|slug| {
            deps.db
                .lock()
                .ok()
                .and_then(|conn| crate::gateway::routing_groups::get(&conn, slug).ok().flatten())
                .map(|group| group.members)
        });
        for reason in &skipped {
            eprintln!("[路由分组] 跳过成员：{reason}");
        }
        expanded
            .into_iter()
            .filter_map(|member| {
                let entry = pool.iter().find(|e| e.slug == member.slug)?.clone();
                Some((entry, member.model))
            })
            .collect()
    } else {
        match decision {
            RouteDecision::Direct(entry, upstream_model) => {
                let bare = upstream_model.clone();
                let mut list = vec![(entry.clone(), upstream_model)];
                list.extend(alternates_for(&deps, &pool, entry, &bare).into_iter().map(|alt| (alt, bare.clone())));
                list
            }
            RouteDecision::Subscription(entry) => {
                // 订阅适配器从请求体读模型：替补来源要把 body 的 model 改成裸名（与分组成员同一做法）
                let bare = route::split_model_id(&model).map(|(_, name)| name.to_string()).unwrap_or_else(|| model.clone());
                let mut list = vec![(entry.clone(), String::new())];
                list.extend(alternates_for(&deps, &pool, entry, &bare).into_iter().map(|alt| (alt, bare.clone())));
                list
            }
            // 指定来源已标为额度用尽 / 登录失效：同平台还有可用的其他账号时直接用它们，
            // 不必等这个来源恢复（route::same_model_alternates）
            RouteDecision::Reject { kind: route::RejectKind::PaymentRequired | route::RejectKind::Unauthorized, .. }
                if fallback_for_rejected(&deps, &pool, &model).is_some() =>
            {
                fallback_for_rejected(&deps, &pool, &model).unwrap_or_default()
            }
            RouteDecision::Reject { kind, message, retry_after } => {
                let status = match kind {
                    route::RejectKind::BadRequest => StatusCode::BAD_REQUEST,
                    route::RejectKind::NotFound => StatusCode::NOT_FOUND,
                    route::RejectKind::Unauthorized => StatusCode::UNAUTHORIZED,
                    route::RejectKind::PaymentRequired => StatusCode::PAYMENT_REQUIRED,
                    route::RejectKind::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
                    route::RejectKind::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
                };
                record_route(&deps, request_id, &meta, None, None, Some(status.as_u16()), "rejected", started);
                return match retry_after {
                    Some(seconds) => error_response_with_retry(status, &message, seconds),
                    None => error_response(status, &message),
                };
            }
        }
    };
    if candidates.is_empty() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            group_note.as_deref().unwrap_or("路由分组的成员都不在模型池中：请检查分组配置"),
        );
    }

    // ── 候选循环 ──
    // 主选失败且属可转移错误（401/402/403/408/429/5xx）时换下一个成员；
    // 请求本身的 4xx（400/404/413/422…）是确定性的，转移只会重复，直接回给调用方。
    // 同一账号（account_fp 相同的不同来源，如同一把 Key 建了两个连接）一次请求只试一次：
    // 第一次失败的原因在第二次一样会出现，再试只是白等（magpie R3）
    let mut tried_accounts: Vec<String> = Vec::new();
    let candidates: Vec<(route::PoolEntry, String)> = candidates
        .into_iter()
        .filter(|(entry, _)| match entry.account_fp.as_deref() {
            Some(fp) if tried_accounts.iter().any(|seen| seen == fp) => {
                eprintln!("[路由] 成员「{}」与前面的候选是同一账号，本次不再重复尝试", entry.slug);
                false
            }
            Some(fp) => {
                tried_accounts.push(fp.to_string());
                true
            }
            None => true,
        })
        .collect();
    // 命名 Key 的账号范围（2.6）：范围外的候选去掉；一个都不剩时如实拒绝，不悄悄换别的来源
    let candidates: Vec<(route::PoolEntry, String)> = match allowed_accounts.as_ref() {
        Some(allowed) => {
            let before: Vec<String> = candidates.iter().map(|(entry, _)| entry.slug.clone()).collect();
            let kept: Vec<_> = candidates.into_iter().filter(|(entry, _)| allowed.contains(&entry.connection_id)).collect();
            if kept.is_empty() {
                return error_response(
                    StatusCode::FORBIDDEN,
                    &format!("这把客户端 Key 只能使用指定的账号，本次要用的来源（{}）不在范围内", before.join("、")),
                );
            }
            kept
        }
        None => candidates,
    };
    let total = candidates.len();
    for (index, (entry, upstream_model)) in candidates.into_iter().enumerate() {
        let last = index + 1 == total;
        if let Ok(mut cell) = routed.lock() {
            let priced_model = if upstream_model.is_empty() { route::split_model_id(&model).map(|(_, m)| m.to_string()).unwrap_or_else(|| model.clone()) } else { upstream_model.clone() };
            *cell = Some((entry.platform.clone(), priced_model));
        }
        if !route::source_is_current(&deps, &entry) {
            if last {
                return error_response(StatusCode::SERVICE_UNAVAILABLE, "来源已关闭或配置已变化，请刷新后重试");
            }
            eprintln!("[路由] 成员「{}」配置已变化，跳过换下一个候选", entry.slug);
            continue;
        }
        let meta = linked(&meta, &entry);
        // 订阅适配器从请求体读模型：分组成员的模型与请求不同时要重写 body 的 model 字段
        let try_body = if upstream_model.is_empty() || upstream_model == model {
            body.clone()
        } else {
            match rewrite_body_model(&body, &upstream_model) {
                Ok(bytes) => bytes,
                Err(_) => body.clone(),
            }
        };
        // Claude Code 的联网服务端工具：上游不是真 Claude 时由网关代为执行（tool_loop.rs）
        let target_model = if upstream_model.is_empty() { model.as_str() } else { upstream_model.as_str() };
        let native_tools = entry.platform == "claude" || crate::gateway::thinking_guard::serves_claude(entry.base_url.as_deref(), target_model);
        let response = if protocol == CallerProtocol::Anthropic && !native_tools {
            let (deps_ref, entry_ref, upstream_ref, headers_ref, session_ref, model_ref, meta_ref) =
                (&deps, &entry, &upstream_model, &headers, &session, &model, &meta);
            with_tool_loop(&try_body, |round_body| {
                let meta = meta_ref.clone();
                async move {
                    forward_one(deps_ref, entry_ref, protocol, upstream_ref, &round_body, path_and_query, headers_ref, session_ref, request_id, meta, started, model_ref).await
                }
            })
            .await
        } else {
            forward_one(
                &deps,
                &entry,
                protocol,
                &upstream_model,
                &try_body,
                path_and_query,
                &headers,
                &session,
                request_id,
                meta,
                started,
                &model,
            )
            .await
        };
        if !is_retryable_status(response.status()) || last {
            return response;
        }
        eprintln!("[路由] 成员「{}」返回 {}，转移下一个候选", entry.slug, response.status());
    }
    error_response(StatusCode::SERVICE_UNAVAILABLE, "没有可用的候选成员")
    }
    .await;
    let response = match reservation {
        Some(reservation) => crate::gateway::budget::meter(response, reservation, budget_db),
        None => response,
    };
    crate::gateway::key_slots::hold(response, slot)
}

/// 命名 Key 的拒绝 → 响应：状态按拒绝种类，只有限额拒绝带 Retry-After。
fn refusal_response(refusal: &crate::gateway::access::RequestLimitRefusal) -> Response {
    let status = StatusCode::from_u16(refusal.status).unwrap_or(StatusCode::TOO_MANY_REQUESTS);
    match refusal.retry_after() {
        Some(seconds) => error_response_with_retry(status, &refusal.message, seconds as u64),
        None => error_response(status, &refusal.message),
    }
}

/// 可转移的上游错误：账号级失败（401/402/403）、限流（429/408）与服务端错误（5xx）
/// 都值得换下一个候选；请求本身的 4xx（格式、模型不存在等）是确定性的，转移只会重复。
fn is_retryable_status(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(status.as_u16(), 401 | 402 | 403 | 408 | 429)
}

/// 把请求体 model 字段改写为候选成员的模型（分组成员各有各的模型）。
fn rewrite_body_model(body: &[u8], model: &str) -> Result<axum::body::Bytes, String> {
    let mut value: serde_json::Value =
        serde_json::from_slice(body).map_err(|error| format!("请求体不是有效 JSON：{error}"))?;
    value["model"] = serde_json::Value::String(model.to_string());
    Ok(axum::body::Bytes::from(value.to_string()))
}

/// 单个候选的转发：Direct / Subscription 两类分发（原主路径逻辑原样搬入），
/// 每次尝试独立记流水、发开始/结束事件，成功时会话绑定到该来源（亲和）。
#[allow(clippy::too_many_arguments)]
async fn forward_one(
    deps: &Arc<ProxyDeps>,
    entry: &route::PoolEntry,
    protocol: CallerProtocol,
    upstream_model: &str,
    body: &axum::body::Bytes,
    path_and_query: &str,
    headers: &HeaderMap,
    session: &str,
    request_id: u64,
    meta: RouteMeta,
    started: std::time::Instant,
    requested_model: &str,
) -> Response {
    let begin = |entry: &route::PoolEntry, meta: &RouteMeta| begin_route(deps, request_id, meta, entry, started);
    let outcome = |response: &Response| if response.status().is_success() { "ok" } else { "failed" };
    let bind = |entry: &route::PoolEntry| {
        if let Some(affinity) = deps.affinity.as_ref() {
            if let Ok(mut guard) = affinity.lock() {
                guard.bind(session, &entry.connection_id, now_ms());
            }
        }
    };
    match entry.kind.as_str() {
        "api" => {
            if entry.protocol != protocol.wire_name() {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    &format!(
                        "连接「{}」只提供 {} 协议端点，不能服务 {} 请求；请为该连接选择 {} 兼容地址",
                        entry.slug,
                        entry.protocol,
                        wire_label(protocol),
                        wire_label(protocol),
                    ),
                );
            }
            // 发往真 Claude 的请求先剔除外来 thinking 块（thinking_guard 模块头说明因果）；
            // 其余上游（zcode 的 GLM、DeepSeek 的兼容端点等）原样发。
            let guard = begin(entry, &meta);
            let upstream_for_log = upstream_model.to_string();
            let response = with_thinking_repair(&entry.connection_id, entry.base_url.as_deref(), upstream_model, body, |body| {
                crate::gateway::adapter::direct::forward(
                    deps.clone(),
                    entry,
                    upstream_model.to_string(),
                    axum::http::Method::POST,
                    path_and_query,
                    headers,
                    body,
                )
            })
            .await;
            guard.finish();
            record_route(
                deps,
                request_id,
                &meta,
                Some(&entry.slug),
                Some(&upstream_for_log),
                Some(response.status().as_u16()),
                outcome(&response),
                started,
            );
            note_outcome(deps, !response.status().is_success(), &format!("连接「{}」返回 {}", entry.slug, response.status()));
            if response.status().is_success() {
                bind(entry);
            }
            response
        }
        // 订阅来源：按平台分发到各自适配器；协议不匹配本地直接返回，不发开始事件
        _ => {
            let platform = entry.platform.as_str();
            let unsupported = match platform {
                "codex" | "opencode" if protocol == CallerProtocol::Anthropic => {
                    Some(error_response(StatusCode::BAD_REQUEST, "Codex 订阅只服务 OpenAI 协议请求"))
                }
                "claude" if protocol != CallerProtocol::Anthropic => {
                    Some(error_response(StatusCode::BAD_REQUEST, "Claude 订阅当前只服务 Anthropic Messages 协议请求"))
                }
                "codex" | "opencode" | "claude" | "grok" | "workbuddy" | "qoder" | "trae" | "zcode" => None,
                _ => Some(error_response(StatusCode::NOT_IMPLEMENTED, "该平台的订阅适配器尚未接入")),
            };
            let response = match unsupported {
                Some(response) => response,
                None => {
                    let guard = begin(entry, &meta);
                    let response = match platform {
                        "codex" | "opencode" => crate::gateway::adapter::codex::forward(deps.clone(), entry, protocol, body.clone()).await,
                        // claude 订阅的上游就是 api.anthropic.com：外来的 thinking 块先剔，被拒再修一次（同上）
                        "claude" => {
                            with_thinking_repair(&entry.connection_id, Some("https://api.anthropic.com"), upstream_model, body, |body| {
                                crate::gateway::adapter::claude::forward(deps.clone(), entry, session, body)
                            })
                            .await
                        }
                        "grok" => crate::gateway::adapter::grok::forward(deps.clone(), entry, protocol, body.clone()).await,
                        "zcode" => crate::gateway::adapter::zcode::forward(deps.clone(), entry, protocol, body.clone(), session).await,
                        _ => {
                            // 只说 Chat 的来源（WorkBuddy / Qoder / Trae）：Anthropic 调用方（Claude Code）经协议转换后转发，回复再转回
                            let anthropic = protocol == CallerProtocol::Anthropic;
                            let converted = if anthropic {
                                crate::gateway::translate::anthropic_request(&body).map(|b| (CallerProtocol::Chat, b))
                            } else {
                                Ok((protocol, body.clone()))
                            };
                            match converted {
                                Err(e) => error_response(StatusCode::BAD_REQUEST, &e),
                                Ok((protocol, body)) => {
                                    let response = match platform {
                                        "workbuddy" => crate::gateway::adapter::workbuddy::forward(deps.clone(), entry, protocol, body).await,
                                        "qoder" => crate::gateway::adapter::qoder::forward(deps.clone(), entry, protocol, body).await,
                                        _ => crate::gateway::adapter::trae::forward(deps.clone(), entry, protocol, body, session).await,
                                    };
                                    if anthropic { crate::gateway::translate::anthropic_response(response, requested_model).await } else { response }
                                }
                            }
                        }
                    };
                    guard.finish();
                    // Grok / WorkBuddy / ZCode 的适配器不自己记账：流结束时从流里读用量入库
                    if matches!(platform, "grok" | "workbuddy" | "zcode") {
                        let model = Some(if upstream_model.is_empty() { requested_model.to_string() } else { upstream_model.to_string() });
                        crate::gateway::subscription::record_usage_on_end(deps.clone(), entry.clone(), model, started, response)
                    } else {
                        response
                    }
                }
            };
            record_route(
                deps,
                request_id,
                &meta,
                Some(&entry.slug),
                None,
                Some(response.status().as_u16()),
                outcome(&response),
                started,
            );
            if response.status().is_success() {
                bind(entry);
            }
            response
        }
    }
}
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn wire_label(protocol: CallerProtocol) -> &'static str {
    match protocol {
        CallerProtocol::Anthropic => "Anthropic Messages",
        CallerProtocol::Chat => "OpenAI Chat",
        CallerProtocol::Responses => "OpenAI Responses",
    }
}

/// 带来源前缀的请求被「额度用尽 / 登录失效」拒绝时的替补：找到那个来源，取它的同平台替补。
fn fallback_for_rejected(deps: &Arc<ProxyDeps>, pool: &[route::PoolEntry], model: &str) -> Option<Vec<(route::PoolEntry, String)>> {
    let (slug, name) = route::split_model_id(model)?;
    let primary = pool.iter().find(|entry| entry.slug == slug)?;
    let alternates = alternates_for(deps, pool, primary, name);
    (!alternates.is_empty()).then(|| alternates.into_iter().map(|alt| (alt, name.to_string())).collect())
}

/// 普通来源失败时的替补候选（最多 2 个，加主选共 3 个；见 `route::same_model_alternates`）。
fn alternates_for(deps: &Arc<ProxyDeps>, pool: &[route::PoolEntry], primary: &route::PoolEntry, model_name: &str) -> Vec<route::PoolEntry> {
    let catalog = deps.db.try_lock().ok().and_then(|conn| crate::gateway::catalog::list(&conn).ok()).unwrap_or_default();
    let serves = |slug: &str| catalog.iter().any(|entry| entry.slug == slug && entry.model == model_name);
    route::same_model_alternates(pool, primary, &serves, 2)
}

/// 一轮回复最多读这么多（非流式的完整消息，含长文本与工具调用）。
const ROUND_BODY_LIMIT: usize = 32 * 1024 * 1024;

/// 替身工具循环：请求声明了联网服务端工具时，网关执行模型的调用并续问，直到模型不再调用
/// 或到达轮次上限（见 `tool_loop` 模块头）。没有声明这类工具时等同于直接发一次。
async fn with_tool_loop<F, Fut>(body: &axum::body::Bytes, send: F) -> Response
where
    F: Fn(axum::body::Bytes) -> Fut,
    Fut: std::future::Future<Output = Response>,
{
    use crate::gateway::tool_loop;
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else { return send(body.clone()).await };
    let managed = tool_loop::managed_tools(&value);
    if managed.is_empty() {
        return send(body.clone()).await;
    }
    let streaming = tool_loop::rewrite_request(&mut value, &managed);
    let mut usage = (0i64, 0i64);
    let mut notes: Vec<String> = Vec::new();
    for round in 0..tool_loop::MAX_ROUNDS {
        let request = serde_json::to_vec(&value).map(axum::body::Bytes::from).unwrap_or_else(|_| body.clone());
        let response = send(request).await;
        if !response.status().is_success() {
            return response;
        }
        let (parts, response_body) = response.into_parts();
        let Ok(bytes) = axum::body::to_bytes(response_body, ROUND_BODY_LIMIT).await else {
            return error_response(StatusCode::BAD_GATEWAY, "上游回复读取失败");
        };
        // 读不懂（比如上游仍回了流）就原样交回，不猜
        let Ok(mut message) = serde_json::from_slice::<Value>(&bytes) else {
            return Response::from_parts(parts, axum::body::Body::from(bytes));
        };
        tool_loop::add_usage(&mut usage, &message);
        let (calls, foreign) = tool_loop::calls_in(&message, &managed);
        if calls.is_empty() {
            return deliver_message(tool_loop::finish(message, &notes, usage, &managed, false), streaming);
        }
        if foreign {
            tool_loop::strip_managed(&mut message, &managed);
            notes.push("模型同时调用了工具自己的功能，联网调用本轮未执行".into());
            return deliver_message(tool_loop::finish(message, &notes, usage, &managed, false), streaming);
        }
        if round + 1 == tool_loop::MAX_ROUNDS {
            return deliver_message(tool_loop::finish(message, &notes, usage, &managed, true), streaming);
        }
        let mut results = Vec::new();
        for call in &calls {
            let (text, is_error, note) = tool_loop::execute(call).await;
            eprintln!("[替身工具] {note}");
            notes.push(note);
            results.push((call.id.clone(), text, is_error));
        }
        tool_loop::append_round(&mut value, &message, &results);
    }
    error_response(StatusCode::BAD_GATEWAY, "联网工具循环异常结束")
}

/// 把最终消息按客户端要的形式交回：流式写成 SSE，否则 JSON。
fn deliver_message(message: Value, streaming: bool) -> Response {
    let (content_type, body) = if streaming {
        ("text/event-stream", crate::gateway::tool_loop::message_to_sse(&message))
    } else {
        ("application/json", message.to_string())
    };
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", content_type)
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|_| error_response(StatusCode::BAD_GATEWAY, "构造回复失败"))
}

/// 400 错误体最多读这么多来判断是否 thinking 签名拒绝；错误体通常几百字节。
const REFUSAL_BODY_LIMIT: usize = 64 * 1024;

/// 发往 Claude 的一次转发，带 thinking 签名的预防剔除与「被拒后修一次」。
///
/// 为什么：预防剔除只认得空签名与 UUID；另一个 Claude 账号签的块、过期签名格式上
/// 看不出来，只能等 Claude 回 400 再按点名位置剔掉重发（`thinking_guard` 400 节）。
/// 只重发一次：剔过仍拒、或没有新块可剔，就把原 400 交给调用方，绝不循环。
/// 非 Claude 上游原样发、原样回，不读响应体。
async fn with_thinking_repair<F, Fut>(
    owner: &str, base_url: Option<&str>, upstream_model: &str, body: &axum::body::Bytes, send: F,
) -> Response
where
    F: Fn(axum::body::Bytes) -> Fut,
    Fut: std::future::Future<Output = Response>,
{
    use crate::gateway::thinking_guard;
    if !thinking_guard::serves_claude(base_url, upstream_model) {
        return send(body.clone()).await;
    }
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return send(body.clone()).await;
    };
    let stripped = thinking_guard::strip_foreign_thinking(&mut value) + thinking_guard::strip_refused(owner, &mut value);
    let first = if stripped == 0 { body.clone() } else { serde_json::to_vec(&value).map(Into::into).unwrap_or_else(|_| body.clone()) };
    let response = send(first).await;
    if response.status() != StatusCode::BAD_REQUEST {
        return response;
    }
    let (parts, error_body) = response.into_parts();
    let Ok(error_bytes) = axum::body::to_bytes(error_body, REFUSAL_BODY_LIMIT).await else {
        return error_response(StatusCode::BAD_GATEWAY, "上游 400 的错误体读取失败");
    };
    if !thinking_guard::remember_refusal(owner, &error_bytes, &value) {
        return Response::from_parts(parts, axum::body::Body::from(error_bytes));
    }
    thinking_guard::strip_refused(owner, &mut value);
    match serde_json::to_vec(&value) {
        Ok(retry) => send(retry.into()).await,
        Err(_) => Response::from_parts(parts, axum::body::Body::from(error_bytes)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closing_or_pausing_a_source_rejects_warmed_pool_before_upstream() {
        let deps=crate::proxy::test_support::bare_deps();
        let listener=tokio::net::TcpListener::bind(("127.0.0.1",0)).await.unwrap();
        let base=format!("http://{}",listener.local_addr().unwrap());
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,base_url,protocol,status,serve_enabled,created_ms,slug) VALUES('isolation-source','custom','api','isolated','fixture-token',?1,'openai','connected',1,1,'isolated')",[base]).unwrap();
        let pool=route::pool_snapshot(&deps.db,deps.pool_index().unwrap());
        assert_eq!(pool.len(),1);
        for (enabled,status) in [(false,"connected"),(true,"paused")] {
            deps.db.lock().unwrap().execute("UPDATE connections SET serve_enabled=?1,status=?2 WHERE id='isolation-source'",rusqlite::params![enabled,status]).unwrap();
            let response=tokio::time::timeout(std::time::Duration::from_millis(500),handle(CallerProtocol::Chat,deps.clone(),HeaderMap::new(),axum::body::Bytes::from_static(br#"{"model":"isolated/fixture","messages":[{"role":"user","content":"test"}]}"#),"/v1/chat/completions",None)).await;
            assert!(response.is_ok(),"不应等待上游；缓存中的来源已经关闭或暂停");
            assert_eq!(response.unwrap().status(),StatusCode::SERVICE_UNAVAILABLE);
            assert!(tokio::time::timeout(std::time::Duration::from_millis(20),listener.accept()).await.is_err(),"关闭后的来源不能收到新请求");
        }
    }
    use std::sync::atomic::{AtomicU32, Ordering};
    fn json_response(value: serde_json::Value) -> Response {
        Response::builder().status(StatusCode::OK).header("content-type", "application/json").body(axum::body::Body::from(value.to_string())).unwrap()
    }

    #[tokio::test]
    async fn without_server_tools_the_request_goes_once_unchanged() {
        let body = axum::body::Bytes::from(r#"{"model":"m","stream":true,"messages":[],"tools":[{"name":"Read"}]}"#);
        let seen = std::sync::Mutex::new(Vec::<axum::body::Bytes>::new());
        let response = with_tool_loop(&body, |round| {
            seen.lock().unwrap().push(round);
            async { json_response(serde_json::json!({"content": []})) }
        })
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(seen.lock().unwrap().as_slice(), &[body], "原样只发一次");
    }

    #[tokio::test]
    async fn a_round_ending_in_foreign_tools_drops_managed_calls_and_streams_back() {
        let body = axum::body::Bytes::from(serde_json::json!({
            "model": "m", "stream": true, "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "Read", "input_schema": {}}, {"type": "web_search_20250305", "name": "web_search"}]
        }).to_string());
        let sent = std::sync::Mutex::new(Vec::<serde_json::Value>::new());
        let response = with_tool_loop(&body, |round| {
            sent.lock().unwrap().push(serde_json::from_slice(&round).unwrap());
            async {
                json_response(serde_json::json!({"id": "msg", "type": "message", "role": "assistant", "model": "m", "stop_reason": "tool_use",
                    "content": [{"type": "tool_use", "id": "a", "name": "Read", "input": {}}, {"type": "tool_use", "id": "b", "name": "web_search", "input": {"query": "q"}}],
                    "usage": {"input_tokens": 3, "output_tokens": 4}}))
            }
        })
        .await;
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["stream"], false, "每轮按非流式问上游");
        assert!(sent[0]["tools"][1].get("type").is_none(), "服务端工具换成了普通工具");
        assert_eq!(response.headers().get("content-type").unwrap(), "text/event-stream", "客户端要流式就回 SSE");
        let text = String::from_utf8(axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
        assert!(text.contains(r#""name":"Read""#) && !text.contains(r#""name":"web_search""#), "接管的调用已去掉");
    }

    #[tokio::test]
    async fn managed_calls_are_executed_and_the_next_round_answers() {
        // 空数据目录：没有搜索后端，执行结果是「未配置」的错误，照样续问下一轮
        let dir = std::env::temp_dir().join(format!("jolu-tool-loop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let previous = std::env::var_os("JOLU_DATA_DIR");
        std::env::set_var("JOLU_DATA_DIR", &dir);
        let body = axum::body::Bytes::from(serde_json::json!({
            "model": "m", "messages": [{"role": "user", "content": "搜一下"}],
            "tools": [{"type": "web_search_20250305", "name": "web_search"}]
        }).to_string());
        let sent = std::sync::Mutex::new(Vec::<serde_json::Value>::new());
        let response = with_tool_loop(&body, |round| {
            let value: serde_json::Value = serde_json::from_slice(&round).unwrap();
            let first = value["messages"].as_array().unwrap().len() == 1;
            sent.lock().unwrap().push(value);
            async move {
                if first {
                    json_response(serde_json::json!({"content": [{"type": "tool_use", "id": "s1", "name": "web_search", "input": {"query": "rust"}}],
                        "stop_reason": "tool_use", "usage": {"input_tokens": 10, "output_tokens": 2}}))
                } else {
                    json_response(serde_json::json!({"content": [{"type": "text", "text": "没法搜，但我知道"}], "stop_reason": "end_turn",
                        "usage": {"input_tokens": 20, "output_tokens": 5}}))
                }
            }
        })
        .await;
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 2, "执行后续问一轮");
        let second = sent[1]["messages"].as_array().unwrap();
        assert_eq!(second[2]["content"][0]["tool_use_id"], "s1");
        assert_eq!(second[2]["content"][0]["is_error"], true, "没配置搜索后端如实告诉模型");
        assert_eq!(response.headers().get("content-type").unwrap(), "application/json", "客户端没要流式就回 JSON");
        let message: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert!(message["content"][0]["text"].as_str().unwrap().contains("搜索「rust」失败"));
        assert_eq!(message["content"][1]["text"], "没法搜，但我知道");
        assert_eq!((message["usage"]["input_tokens"].as_i64(), message["usage"]["output_tokens"].as_i64()), (Some(30), Some(7)), "用量取两轮之和");
        match previous {
            Some(value) => std::env::set_var("JOLU_DATA_DIR", value),
            None => std::env::remove_var("JOLU_DATA_DIR"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn upstream_errors_inside_the_loop_are_returned_as_is() {
        let body = axum::body::Bytes::from(serde_json::json!({"model": "m", "messages": [], "tools": [{"type": "web_fetch_20250910", "name": "web_fetch"}]}).to_string());
        let response = with_tool_loop(&body, |_| async { refusal_400() }).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    fn refusal_400() -> Response {
        Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(axum::body::Body::from(r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages.1.content.0: Invalid `signature` in `thinking` block"}}"#))
            .unwrap()
    }

    fn signed_by_another_account() -> axum::body::Bytes {
        axum::body::Bytes::from(serde_json::json!({
            "model": "claude-opus-5-5",
            "messages": [
                {"role": "user", "content": "你好"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "另一个账号签的", "signature": "c2lnbmVkLWJ5LWFub3RoZXItYWNjb3VudA"},
                    {"type": "text", "text": "回答"}
                ]},
                {"role": "user", "content": "继续"}
            ]
        }).to_string())
    }

    #[tokio::test]
    async fn claude_refusing_a_thinking_signature_is_mended_once_and_remembered() {
        let sent = std::sync::Mutex::new(Vec::<serde_json::Value>::new());
        let send = |body: axum::body::Bytes| {
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let thinking = value.to_string().contains("\"thinking\"");
            sent.lock().unwrap().push(value);
            async move { if thinking { refusal_400() } else { Response::new(axum::body::Body::from("ok")) } }
        };
        let body = signed_by_another_account();
        let response = with_thinking_repair("repair-owner", Some("https://api.anthropic.com"), "claude-opus-5-5", &body, send).await;
        assert_eq!(response.status(), StatusCode::OK, "剔掉被拒的块后重发成功");
        {
            let sent = sent.lock().unwrap();
            assert_eq!(sent.len(), 2, "只重发一次");
            assert_eq!(sent[1]["messages"][1]["content"].as_array().unwrap().len(), 1);
            assert_eq!(sent[1]["messages"][1]["content"][0]["type"], "text", "text 保留");
        }
        // 同一来源的下一轮：首发就不带被拒过的块
        let response = with_thinking_repair("repair-owner", Some("https://api.anthropic.com"), "claude-opus-5-5", &body, send).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(sent.lock().unwrap().len(), 3, "记住之后不再先挨一次 400");
    }

    #[tokio::test]
    async fn repeated_refusal_and_other_errors_are_returned_as_is() {
        let calls = AtomicU32::new(0);
        let always_refuse = |_body: axum::body::Bytes| {
            calls.fetch_add(1, Ordering::Relaxed);
            async { refusal_400() }
        };
        let body = signed_by_another_account();
        let response = with_thinking_repair("repair-owner-loop", Some("https://api.anthropic.com"), "claude-opus-5-5", &body, always_refuse).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "剔过仍拒：原样交回 400");
        assert_eq!(calls.load(Ordering::Relaxed), 2, "最多两次，不循环");
        let text = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(String::from_utf8_lossy(&text).contains("Invalid `signature`"), "错误体原样保留");
        // 非 Claude 上游：不剔、不重试
        let calls = AtomicU32::new(0);
        let refuse = |_body: axum::body::Bytes| {
            calls.fetch_add(1, Ordering::Relaxed);
            async { refusal_400() }
        };
        let response = with_thinking_repair("repair-owner-glm", Some("https://open.bigmodel.cn/api/anthropic"), "glm-5.3", &body, refuse).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }


    fn deps_with_circuit(threshold: u32, hits: Arc<AtomicU32>, enabled: bool) -> Arc<ProxyDeps> {
        let mut deps = Arc::try_unwrap(crate::proxy::test_support::bare_deps()).unwrap_or_else(|_| {
            // bare_deps 是 Arc，拿不到所有权时重建一份最小 deps 供熔断计数测试
            crate::proxy::test_support::bare_deps();
            unreachable!()
        });
        deps.circuit_threshold = threshold;
        deps.fallback_enabled = enabled;
        deps.on_circuit = Arc::new(move |_| {
            hits.fetch_add(1, Ordering::Relaxed);
        });
        Arc::new(deps)
    }

    #[test]
    fn circuit_counts_consecutive_failures_and_resets_on_success() {
        let hits = Arc::new(AtomicU32::new(0));
        let deps = deps_with_circuit(3, hits.clone(), true);
        note_outcome(&deps, true, "a");
        note_outcome(&deps, true, "b");
        assert_eq!(hits.load(Ordering::Relaxed), 0, "未达阈值不熔断");
        note_outcome(&deps, true, "c");
        assert_eq!(hits.load(Ordering::Relaxed), 1, "达阈值熔断一次");
        // 熔断后计数归零，不会连续触发
        note_outcome(&deps, true, "d");
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        // 成功清零
        note_outcome(&deps, false, "");
        note_outcome(&deps, true, "e");
        assert_eq!(hits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn circuit_is_off_when_auto_escape_disabled() {
        let hits = Arc::new(AtomicU32::new(0));
        let deps = deps_with_circuit(1, hits.clone(), false);
        for _ in 0..5 {
            note_outcome(&deps, true, "x");
        }
        assert_eq!(hits.load(Ordering::Relaxed), 0, "关闭自动逃生时绝不自动切断");
    }

    #[test]
    fn agent_from_headers_reads_token_originator_and_user_agent() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "jolu-claude-code.abcd1234".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("claude-code"));
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer jolu-codex.abcd1234".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("codex"));
        let mut headers = HeaderMap::new();
        headers.insert("originator", "codex_cli_rs".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("codex"));
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "GeminiCLI/0.34.0/... (win32; x64)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("gemini"));
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "CloudCodeVSCode/0.65.0 (aidev_client; os_type=Windows; proxy_client=geminicli)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("gemini"));
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "claude-cli/2.1.0 (external, cli)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("claude-code"));
        assert_eq!(agent_from_headers(&HeaderMap::new()).agent, None);
    }

    #[test]
    fn codex_masquerading_as_claude_code_is_corrected() {
        // OpenAI /codex:review 插件在 Claude Code 里跑 codex app-server：
        // originator 和 UA 都写 "Claude Code/..."，但 Codex 每轮都带这两个头
        // （现场记录 wyztony on X：Codex 的调用量从来没记上）
        let mut headers = HeaderMap::new();
        headers.insert("originator", "Claude Code/0.162.1".parse().unwrap());
        headers.insert("user-agent", "Claude Code/0.162.1 (external, cli)".parse().unwrap());
        headers.insert("x-codex-window-id", "win-1".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("codex"), "有 Codex 特有头 + UA 写 Claude Code → 是 Codex");
        // turn-metadata 也能单独触发
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "Claude Code/0.162.1".parse().unwrap());
        headers.insert("x-codex-turn-metadata", "{}".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("codex"));
        // 没有 Codex 特有头：还是 Claude Code（不误判）
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "claude-cli/2.1.114 (external, cli)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("claude-code"));
    }

    #[test]
    fn vscode_agent_host_resolves_to_codex() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "vscode_agent_host/1.2.3".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("codex"));
    }

    #[test]
    fn identified_by_carries_the_matching_rule() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "jolu-claude-code.abcd".parse().unwrap());
        let identity = agent_from_headers(&headers);
        assert_eq!(identity.identified_by, Some("token"), "token 识别的标注 token");
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "codex_cli_rs/0.1".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).identified_by, Some("ua"));
    }

    #[test]
    fn unidentified_ua_carries_first_segment_for_diagnostics() {
        // 识别不出：给 UA 首段（到 / 或空格），让用户报问题时直接看出是哪个工具
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "some-new-tool/1.0 (darwin; arm64)".parse().unwrap());
        let identity = agent_from_headers(&headers);
        assert_eq!(identity.agent, None);
        assert_eq!(identity.unidentified_ua.as_deref(), Some("some-new-tool"), "取 UA 首段");
        // 空 UA：不填
        assert_eq!(agent_from_headers(&HeaderMap::new()).unidentified_ua, None);
    }

    #[test]
    fn session_identity_only_trusts_native_session_ids() {
        let none = HeaderMap::new();
        // Claude Code：JSON user_id 里的 session_id；旧格式 _session_ 后缀；专用头优先
        let body = br#"{"metadata":{"user_id":"{\"device_id\":\"d\",\"session_id\":\"ses-1\"}"}}"#;
        let got = session_identity(Some("claude-code"), &none, body).unwrap();
        assert_eq!((got.platform.as_str(), got.session_id.as_str()), ("claude", "ses-1"));
        let legacy = br#"{"metadata":{"user_id":"user_abc_account_11-22_session_33-44"}}"#;
        assert_eq!(session_identity(Some("claude-code"), &none, legacy).unwrap().session_id, "33-44");
        let mut headers = HeaderMap::new();
        headers.insert("x-claude-code-session-id", "hdr-ses".parse().unwrap());
        assert_eq!(session_identity(Some("claude-code"), &headers, body).unwrap().session_id, "hdr-ses");
        // 普通 user_id 不是会话
        assert!(session_identity(Some("claude-code"), &none, br#"{"metadata":{"user_id":"someone"}}"#).is_none());

        // Codex：只认根会话头；请求级 ID 与缓存键不算
        let mut headers = HeaderMap::new();
        headers.insert("session-id", "root-1".parse().unwrap());
        headers.insert("thread-id", "child-1".parse().unwrap());
        let got = session_identity(Some("codex"), &headers, b"{}").unwrap();
        assert_eq!((got.platform.as_str(), got.session_id.as_str()), ("codex", "root-1"));
        let mut headers = HeaderMap::new();
        headers.insert("x-client-request-id", "req-1".parse().unwrap());
        assert!(session_identity(Some("codex"), &headers, br#"{"prompt_cache_key":"cache-1"}"#).is_none());

        // OpenCode：本会话 ID；子会话归到父会话；只有亲和头不算
        let mut headers = HeaderMap::new();
        headers.insert("x-opencode-session-id", "ses_child".parse().unwrap());
        assert_eq!(session_identity(Some("opencode"), &headers, b"{}").unwrap().session_id, "opencode:ses_child");
        headers.insert("x-opencode-parent-session-id", "ses_root".parse().unwrap());
        let got = session_identity(Some("opencode"), &headers, b"{}").unwrap();
        assert_eq!((got.platform.as_str(), got.session_id.as_str()), ("opencode", "opencode:ses_root"));
        let mut headers = HeaderMap::new();
        headers.insert("x-opencode-session", "affinity-root".parse().unwrap());
        assert!(session_identity(Some("opencode"), &headers, b"{}").is_none());
        let mut headers = HeaderMap::new();
        headers.insert("x-opencode-client", "cli".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("opencode"));

        // 识别不出 Agent，或别的 Agent 只有通用头：不猜
        let mut headers = HeaderMap::new();
        headers.insert("session-id", "root-1".parse().unwrap());
        assert!(session_identity(None, &headers, body).is_none());
        assert!(session_identity(Some("gemini"), &headers, body).is_none());
    }

    #[test]
    fn session_key_prefers_anthropic_metadata_user_id() {
        let body = br#"{"model":"kimi/kimi-k2","metadata":{"user_id":"user-abc"}}"#;
        let key = session_key(CallerProtocol::Anthropic, &HeaderMap::new(), body, "kimi/kimi-k2");
        assert_eq!(key, "user:user-abc");
        // Claude Code 把 user_id 写成 JSON：取里面的 session_id
        let body = br#"{"model":"kimi/kimi-k2","metadata":{"user_id":"{\"device_id\":\"d\",\"session_id\":\"ses-1\"}"}}"#;
        let key = session_key(CallerProtocol::Anthropic, &HeaderMap::new(), body, "kimi/kimi-k2");
        assert_eq!(key, "user:ses-1");
        // 完全没有会话字段 → 退化为令牌 + slug
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer jolu-codex".parse().unwrap());
        let body = br#"{"model":"codex/gpt-5.5"}"#;
        let key = session_key(CallerProtocol::Chat, &headers, body, "codex/gpt-5.5");
        assert_eq!(key, "tok:jolu-codex:codex");
    }

    #[test]
    fn session_key_prefers_codex_and_generic_session_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("session-id", "root-session".parse().unwrap());
        headers.insert("thread-id", "child-thread".parse().unwrap());
        headers.insert("x-client-request-id", "child-thread".parse().unwrap());
        let body = br#"{"model":"codex/gpt-5.5","prompt_cache_key":"cache-1"}"#;
        assert_eq!(
            session_key(CallerProtocol::Responses, &headers, body, "codex/gpt-5.5"),
            "hdr:codex-session:root-session",
            "session-id 是根会话，优先于子线程"
        );

        let mut headers = HeaderMap::new();
        headers.insert("session-id", "root-session".parse().unwrap());
        assert_eq!(
            session_key(CallerProtocol::Chat, &headers, br#"{"prompt_cache_key":"cache-1"}"#, "codex/m"),
            "hdr:codex-session:root-session"
        );

        let mut headers = HeaderMap::new();
        headers.insert("x-session-affinity", "affinity-1".parse().unwrap());
        assert_eq!(
            session_key(CallerProtocol::Chat, &headers, b"{}", "codex/m"),
            "hdr:affinity:affinity-1"
        );

        // 只有请求体里的 prompt_cache_key（Codex 的线程 id）也能区分会话
        assert_eq!(
            session_key(CallerProtocol::Chat, &HeaderMap::new(), br#"{"prompt_cache_key":"cache-9"}"#, "codex/m"),
            "body:prompt-cache:cache-9"
        );

        // 显式覆盖头优先于所有 CLI 字段
        let mut headers = HeaderMap::new();
        headers.insert("x-jolu-session", "explicit".parse().unwrap());
        headers.insert("session-id", "root-session".parse().unwrap());
        assert_eq!(
            session_key(CallerProtocol::Chat, &headers, b"{}", "codex/m"),
            "hdr:x-jolu-session:explicit"
        );
    }
}
