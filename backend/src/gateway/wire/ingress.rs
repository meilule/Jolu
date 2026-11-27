//! 请求预处理：身份提取、中间件链、命名客户端 Key 限额/并发槽/预算预扣与上下文采集。

use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::Value;
use tokio::sync::OwnedSemaphorePermit;

use super::{error_response, error_response_with_retry, missing_model_reason, CallerProtocol};
use crate::gateway::activity::RouteMeta;
use crate::gateway::route;
use crate::proxy::ProxyDeps;

pub(super) struct PreparedRequest {
    pub started: std::time::Instant,
    pub protocol: CallerProtocol,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub model: String,
    pub session: String,
    pub request_id: u64,
    pub meta: RouteMeta,
    pub allowed_accounts: Option<Vec<String>>,
    pub routed: crate::gateway::budget::Routed,
    pub capture: Option<crate::gateway::context::capture::Capture>,
    pub reservation: Option<crate::gateway::budget::Reservation>,
    pub budget_db: Arc<Mutex<rusqlite::Connection>>,
    pub slot: Option<OwnedSemaphorePermit>,
}

/// 从请求体提取 model 字段（任何协议都在顶层）。
fn extract_model(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
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
    let value: Value = serde_json::from_slice(body).ok()?;
    let user = value.pointer("/metadata/user_id")?.as_str()?.trim();
    if user.is_empty() {
        return None;
    }
    if let Ok(inner) = serde_json::from_str::<Value>(user) {
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
    let value: Value = serde_json::from_slice(body).ok()?;
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
    let slug = route::split_model_id(model).map(|(s, _)| s).unwrap_or("");
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
    let value: Value = serde_json::from_slice(body).ok()?;
    let user = value.pointer("/metadata/user_id")?.as_str()?.trim();
    if let Ok(inner) = serde_json::from_str::<Value>(user) {
        return inner.get("session_id").and_then(|v| v.as_str()).and_then(plausible_session_id);
    }
    user.rsplit_once("_session_").and_then(|(_, session)| plausible_session_id(session))
}

/// 已验证的调用方会话（统一网关开发说明 §6.1）。只认工具原生携带、可与会话线程对应的会话 ID。
pub(crate) fn session_identity(
    agent: Option<&str>,
    headers: &HeaderMap,
    body: &[u8],
) -> Option<crate::gateway::activity::SessionRef> {
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

/// Codex 伪装成 Claude Code 的纠正（magpie `codexForClaudeCode` 同款）。
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

/// VS Code 的 Agents 窗口用 `vscode_agent_host` 名字跑 Codex。
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
        let first = ua.split(['/', ' ']).next().unwrap_or(ua);
        let truncated: String = first.chars().take(64).collect();
        Self { agent: None, identified_by: None, unidentified_ua: (!truncated.is_empty()).then_some(truncated) }
    }
}

/// 从调用方令牌 / 客户端头识别是哪个 agent（只用于路由流水与灵动岛，不做鉴权）。
pub(crate) fn agent_from_headers(headers: &HeaderMap) -> AgentIdentity {
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
        .filter(|agent| crate::gateway::wiring::agents().contains(agent))
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
    if ua.to_ascii_lowercase().contains("proxy_client=geminicli") {
        return AgentIdentity::identified("gemini", "ua");
    }
    if headers.contains_key("x-opencode-client") || headers.contains_key("x-opencode-session-id") {
        return AgentIdentity::identified("opencode", "ua");
    }
    let product = ua.split(['/', ' ']).next().unwrap_or("").trim().to_ascii_lowercase();
    if !product.is_empty() {
        for spec in crate::gateway::wiring_specs::SPECS {
            if spec.ua.iter().any(|prefix| product == *prefix) {
                return AgentIdentity::identified(spec.id, "ua");
            }
        }
    }
    AgentIdentity::unknown(ua)
}

/// 命名 Key 的拒绝 → 响应：状态按拒绝种类，只有限额拒绝带 Retry-After。
fn refusal_response(refusal: &crate::gateway::access::RequestLimitRefusal) -> Response {
    let status = StatusCode::from_u16(refusal.status).unwrap_or(StatusCode::TOO_MANY_REQUESTS);
    match refusal.retry_after() {
        Some(seconds) => error_response_with_retry(status, &refusal.message, seconds as u64),
        None => error_response(status, &refusal.message),
    }
}

fn apply_middlewares(deps: &Arc<ProxyDeps>, mut body: Bytes) -> Result<Bytes, Response> {
    let middleware_list = deps
        .db
        .lock()
        .ok()
        .and_then(|conn| crate::gateway::middleware::list(&conn).ok())
        .unwrap_or_default();
    let mut body_value = serde_json::from_slice::<Value>(&body).ok();
    let outcome = body_value
        .as_mut()
        .map(|value| crate::gateway::middleware::apply_chain(&middleware_list, value));
    if let Some(value) = body_value.as_ref() {
        if let Ok(bytes) = serde_json::to_vec(value) {
            body = bytes.into();
        }
    }
    if let Some(outcome) = &outcome {
        if let Some(from) = &outcome.mapped_from {
            let to = extract_model(&body).unwrap_or_default();
            eprintln!("[中间件] model-map：{from} → {to}");
        }
        for reason in &outcome.skipped {
            eprintln!("[中间件] 跳过：{reason}");
        }
        if let Some(reason) = &outcome.blocked {
            return Err(error_response(StatusCode::BAD_REQUEST, reason));
        }
    }
    Ok(body)
}

pub(super) async fn prepare(
    protocol: CallerProtocol,
    deps: &Arc<ProxyDeps>,
    headers: HeaderMap,
    body: Bytes,
    client_key: Option<String>,
) -> Result<PreparedRequest, Response> {
    let started = std::time::Instant::now();
    let body = crate::gateway::base64_input::standardize_body(body);
    let request_id = crate::gateway::activity::next_request_id();
    let request_uid = deps.request_uids.take(&deps.db);
    let identity = agent_from_headers(&headers);
    let agent = identity.agent.clone();

    let body = apply_middlewares(deps, body)?;
    let Some(raw_model) = extract_model(&body) else {
        return Err(error_response(StatusCode::BAD_REQUEST, missing_model_reason(&body)));
    };
    let model = route::canonical_model(&raw_model).to_string();

    if let Some(name) = client_key.as_deref() {
        if let Ok(conn) = deps.db.lock() {
            match crate::gateway::access::check_client_key_limit_for(
                &conn,
                name,
                chrono::Local::now(),
                Some(&model),
                true,
            ) {
                Ok(Some(refusal)) => return Err(refusal_response(&refusal)),
                Err(reason) => return Err(error_response(StatusCode::SERVICE_UNAVAILABLE, &reason)),
                Ok(None) => {}
            }
        }
    }

    let cap = client_key.as_deref().and_then(|name| {
        deps.db.lock().ok().and_then(|conn| crate::gateway::access::client_key_concurrency(&conn, name))
    });
    let slot = match client_key.as_deref() {
        Some(name) => crate::gateway::key_slots::acquire(name, cap).await,
        None => None,
    };

    let routed: crate::gateway::budget::Routed = Default::default();
    let allowed_accounts = client_key.as_deref().and_then(|name| {
        deps.db.lock().ok().and_then(|conn| crate::gateway::access::client_key_accounts(&conn, name))
    });
    let reservation = match client_key.as_deref() {
        Some(name) => match deps.db.lock() {
            Ok(conn) => match crate::gateway::budget::reserve(&conn, name, &body, chrono::Local::now(), routed.clone()) {
                Ok(reservation) => reservation,
                Err(refusal) => return Err(refusal_response(&refusal)),
            },
            Err(_) => return Err(error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试")),
        },
        None => None,
    };

    let session_ref = session_identity(agent.as_deref(), &headers, &body);
    crate::gateway::header_probe::record(
        agent.as_deref(),
        &headers,
        &body,
        session_ref.as_ref().map(|s| (s.platform.as_str(), s.session_id.as_str())),
    );
    let capture = session_ref.as_ref().map(|s| {
        crate::gateway::context::capture::Capture::start(
            deps.db.clone(),
            crate::gateway::context::Protocol::of_caller(protocol),
            body.clone(),
            &s.platform,
            &s.session_id,
            request_id,
            request_uid,
        )
    });

    let session = session_key(protocol, &headers, &body, &model);
    let meta = RouteMeta {
        request_uid,
        session: session_ref,
        agent,
        identified_by: identity.identified_by,
        unidentified_ua: identity.unidentified_ua,
        requested_model: model.clone(),
        connection_id: None,
        client_key,
        decision: None,
    };

    Ok(PreparedRequest {
        started,
        protocol,
        headers,
        body,
        model,
        session,
        request_id,
        meta,
        allowed_accounts,
        routed,
        capture,
        reservation,
        budget_db: deps.db.clone(),
        slot,
    })
}
