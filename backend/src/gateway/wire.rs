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
pub(crate) fn agent_from_headers(headers: &HeaderMap) -> Option<String> {
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
        return Some(agent.to_string());
    }
    if let Some(originator) = headers.get("originator").and_then(|v| v.to_str().ok()) {
        let originator = originator.to_ascii_lowercase();
        if originator.contains("codex") {
            return Some("codex".into());
        }
        if originator.contains("claude") {
            return Some("claude-code".into());
        }
        if originator.contains("gemini") {
            return Some("gemini".into());
        }
    }
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or_default();
    // Gemini CLI 旧版 UA 以 GeminiCLI 开头；0.65 起改为 CloudCodeVSCode/…（…proxy_client=geminicli）
    if ua.starts_with("GeminiCLI") || ua.to_ascii_lowercase().contains("proxy_client=geminicli") {
        return Some("gemini".into());
    }
    if ua.starts_with("codex_cli_rs") {
        return Some("codex".into());
    }
    if ua.starts_with("claude-cli/") {
        return Some("claude-code".into());
    }
    // OpenCode 每个模型请求都带 x-opencode-client / x-opencode-session-id
    if ua.to_ascii_lowercase().starts_with("opencode/")
        || headers.contains_key("x-opencode-client")
        || headers.contains_key("x-opencode-session-id")
    {
        return Some("opencode".into());
    }
    None
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
    let request_id = crate::gateway::activity::next_request_id();
    let agent = agent_from_headers(&headers);
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
                Ok(Some(refusal)) => {
                    return error_response_with_retry(StatusCode::TOO_MANY_REQUESTS, &refusal.message, refusal.retry_after_seconds.max(1) as u64);
                }
                Err(reason) => return error_response(StatusCode::SERVICE_UNAVAILABLE, &reason),
                Ok(None) => {}
            }
        }
    }
    let meta = RouteMeta {
        session: session_identity(agent.as_deref(), &headers, &body),
        agent: agent.clone(),
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
        // 裸模型名 → 目录里第一个暴露它的来源（对齐 magpie 的裸名解析）。
        // 只在模型 id 不带前缀时被调用，因此正常带前缀的请求不多这一次查询。
        let bare = |name: &str| -> Option<String> {
            let conn = deps.db.lock().ok()?;
            crate::gateway::catalog::list(&conn)
                .ok()?
                .into_iter()
                .find(|entry| entry.model == name)
                .map(|entry| entry.slug)
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
            // Cap hold：额度快照显示已用满（重置时间在未来且额度 100%）的成员降到末尾。
            // 目前额度快照只记重置时间，没有已用百分比——保守处理：不判定为用满，
            // 传 false（等额度快照带上用量后再接，注释说明边界）。
            let exhausted_of = |_slug: &str| -> bool { false };
            crate::gateway::routing_groups::ordered_members(
                &group,
                &mut rotate,
                &usage_of,
                &resets_of,
                &exhausted_of,
                chrono::Utc::now().timestamp_millis(),
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
            RouteDecision::Direct(entry, upstream_model) => vec![(entry.clone(), upstream_model)],
            RouteDecision::Subscription(entry) => vec![(entry.clone(), String::new())],
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
    let total = candidates.len();
    for (index, (entry, upstream_model)) in candidates.into_iter().enumerate() {
        let last = index + 1 == total;
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
        let response = forward_one(
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
        .await;
        if !is_retryable_status(response.status()) || last {
            return response;
        }
        eprintln!("[路由] 成员「{}」返回 {}，转移下一个候选", entry.slug, response.status());
    }
    error_response(StatusCode::SERVICE_UNAVAILABLE, "没有可用的候选成员")
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
            let guard = begin(entry, &meta);
            let upstream_for_log = upstream_model.to_string();
            let response = crate::gateway::adapter::direct::forward(
                deps.clone(),
                entry,
                upstream_model.to_string(),
                axum::http::Method::POST,
                path_and_query,
                headers,
                body.clone(),
            )
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
                        "claude" => crate::gateway::adapter::claude::forward(deps.clone(), entry, session, body.clone()).await,
                        "grok" => crate::gateway::adapter::grok::forward(deps.clone(), entry, protocol, body.clone()).await,
                        "zcode" => crate::gateway::adapter::zcode::forward(deps.clone(), entry, protocol, body.clone(), session).await,
                        _ => {
                            // 只说 Chat 的来源（WorkBuddy / Qoder / Trae）：Anthropic 调用方（Claude Code）经协议转换后转发，回复再转回
                            let anthropic = protocol == CallerProtocol::Anthropic;
                            let converted = if anthropic {
                                crate::gateway::translate::anthropic_request(body).map(|b| (CallerProtocol::Chat, b))
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
                    response
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
        assert_eq!(agent_from_headers(&headers).as_deref(), Some("claude-code"));
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer jolu-codex.abcd1234".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).as_deref(), Some("codex"));
        let mut headers = HeaderMap::new();
        headers.insert("originator", "codex_cli_rs".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).as_deref(), Some("codex"));
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "GeminiCLI/0.34.0/... (win32; x64)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).as_deref(), Some("gemini"));
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "CloudCodeVSCode/0.65.0 (aidev_client; os_type=Windows; proxy_client=geminicli)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).as_deref(), Some("gemini"));
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "claude-cli/2.1.0 (external, cli)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).as_deref(), Some("claude-code"));
        assert_eq!(agent_from_headers(&HeaderMap::new()), None);
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
        assert_eq!(agent_from_headers(&headers).as_deref(), Some("opencode"));

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
