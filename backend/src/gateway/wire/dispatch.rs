//! 候选失败转移、替身联网工具循环与单次尝试分发。

use std::sync::Arc;

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::Value;

use super::codec::{self, Conversion};
use super::egress::{begin_route, linked, observe_caller_timing, record_route, watch_stream};
use super::ingress::PreparedRequest;
use super::select::CandidatePlan;
use super::{error_response, note_outcome, now_ms, CallerProtocol};
use crate::gateway::activity::RouteMeta;
use crate::gateway::adapter::registry::{self, with_thinking_repair};
use crate::gateway::route;
use crate::proxy::ProxyDeps;

/// 一轮回复最多读这么多（非流式的完整消息，含长文本与工具调用）。
const ROUND_BODY_LIMIT: usize = 32 * 1024 * 1024;

/// 可转移的上游错误：账号级失败（401/402/403）、限流（429/408）与服务端错误（5xx）。
fn is_retryable_status(status: StatusCode) -> bool {
    status.is_server_error() || matches!(status.as_u16(), 401 | 402 | 403 | 408 | 429)
}

/// 把请求体 model 字段改写为候选成员的模型（分组成员各有各的模型）。
fn rewrite_body_model(body: &[u8], model: &str) -> Result<Bytes, String> {
    let mut value: Value = serde_json::from_slice(body).map_err(|error| format!("请求体不是有效 JSON：{error}"))?;
    value["model"] = Value::String(model.to_string());
    Ok(Bytes::from(value.to_string()))
}

fn bind_session_affinity(deps: &Arc<ProxyDeps>, session: &str, entry: &route::PoolEntry) {
    if let Some(affinity) = deps.affinity.as_ref() {
        if let Ok(mut guard) = affinity.lock() {
            guard.bind(session, &entry.connection_id, now_ms());
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn forward_api(
    deps: &Arc<ProxyDeps>,
    entry: &route::PoolEntry,
    protocol: CallerProtocol,
    upstream_model: &str,
    body: &Bytes,
    path_and_query: &str,
    headers: &HeaderMap,
    session: &str,
    request_id: u64,
    meta: &RouteMeta,
    started: std::time::Instant,
    requested_model: &str,
) -> Response {
    let conversion = match codec::decide_api_conversion(protocol, entry) {
        Ok(conv) => conv,
        Err(err_resp) => return err_resp,
    };
    let original_body = body.clone();
    let (wire_body, path) = match codec::encode_api_request(conversion, protocol, body, path_and_query) {
        Ok(pair) => pair,
        Err(err_resp) => return err_resp,
    };
    let guard = begin_route(deps, request_id, meta, entry, started);
    let upstream_for_log = upstream_model.to_string();
    let path_ref = path.as_str();
    let response = with_thinking_repair(
        &entry.connection_id,
        entry.base_url.as_deref(),
        upstream_model,
        &wire_body,
        |b| {
            crate::gateway::adapter::direct::forward(
                deps.clone(),
                entry,
                upstream_model.to_string(),
                axum::http::Method::POST,
                path_ref,
                headers,
                b,
            )
        },
    )
    .await;
    let response = codec::decode_api_response(conversion, protocol, response, requested_model, &original_body).await;
    let response = if conversion != Conversion::None {
        observe_caller_timing(deps.clone(), entry, &upstream_for_log, protocol, started, response)
    } else {
        response
    };
    guard.finish();
    let result = if response.status().is_success() { "ok" } else { "failed" };
    let row_id = record_route(
        deps,
        request_id,
        meta,
        Some(&entry.slug),
        Some(&upstream_for_log),
        Some(response.status().as_u16()),
        result,
        started,
    );
    note_outcome(deps, !response.status().is_success(), &format!("连接「{}」返回 {}", entry.slug, response.status()));
    if response.status().is_success() {
        bind_session_affinity(deps, session, entry);
    }
    watch_stream(deps, response, row_id, request_id, meta, &entry.slug, started)
}

#[allow(clippy::too_many_arguments)]
async fn forward_subscription_attempt(
    deps: &Arc<ProxyDeps>,
    entry: &route::PoolEntry,
    protocol: CallerProtocol,
    upstream_model: &str,
    body: &Bytes,
    session: &str,
    request_id: u64,
    meta: &RouteMeta,
    started: std::time::Instant,
    requested_model: &str,
) -> Response {
    let response = match registry::check_protocol_support(&entry.platform, protocol) {
        Err(unsupported) => unsupported,
        Ok(()) => {
            let guard = begin_route(deps, request_id, meta, entry, started);
            let resp = registry::forward_subscription(
                deps,
                entry,
                protocol,
                upstream_model,
                requested_model,
                body,
                session,
                started,
            )
            .await;
            guard.finish();
            resp
        }
    };
    let result = if response.status().is_success() { "ok" } else { "failed" };
    let row_id = record_route(
        deps,
        request_id,
        meta,
        Some(&entry.slug),
        None,
        Some(response.status().as_u16()),
        result,
        started,
    );
    if response.status().is_success() {
        bind_session_affinity(deps, session, entry);
    }
    watch_stream(deps, response, row_id, request_id, meta, &entry.slug, started)
}

#[allow(clippy::too_many_arguments)]
async fn forward_one(
    deps: &Arc<ProxyDeps>,
    entry: &route::PoolEntry,
    protocol: CallerProtocol,
    upstream_model: &str,
    body: &Bytes,
    path_and_query: &str,
    headers: &HeaderMap,
    session: &str,
    request_id: u64,
    mut meta: RouteMeta,
    started: std::time::Instant,
    requested_model: &str,
) -> Response {
    if let Some(note) = codec::dropped_tools_note(protocol, entry, body) {
        meta.decision = Some(match meta.decision.take() {
            Some(decision) => format!("{decision} · {note}"),
            None => note,
        });
    }
    if entry.kind == "api" {
        forward_api(
            deps,
            entry,
            protocol,
            upstream_model,
            body,
            path_and_query,
            headers,
            session,
            request_id,
            &meta,
            started,
            requested_model,
        )
        .await
    } else {
        forward_subscription_attempt(
            deps,
            entry,
            protocol,
            upstream_model,
            body,
            session,
            request_id,
            &meta,
            started,
            requested_model,
        )
        .await
    }
}

pub(super) async fn execute_with_failover(
    deps: &Arc<ProxyDeps>,
    prepared: &PreparedRequest,
    plan: CandidatePlan,
    path_and_query: &str,
) -> Response {
    let total = plan.candidates.len();
    for (index, (entry, upstream_model)) in plan.candidates.into_iter().enumerate() {
        let last = index + 1 == total;
        let (upstream_model, member_effort) = crate::gateway::member_effort::split(&upstream_model);
        if let Ok(mut cell) = prepared.routed.lock() {
            let priced_model = if upstream_model.is_empty() {
                route::split_model_id(&prepared.model)
                    .map(|(_, m)| m.to_string())
                    .unwrap_or_else(|| prepared.model.clone())
            } else {
                upstream_model.clone()
            };
            if let Some(capture) = prepared.capture.as_ref() {
                capture.set_route(&entry.connection_id, &entry.platform, &priced_model);
            }
            *cell = Some((entry.platform.clone(), priced_model));
        }
        if !route::source_is_current(deps, &entry) {
            if last {
                record_route(deps, prepared.request_id, &plan.meta, None, None, Some(503), "rejected", prepared.started);
                return error_response(StatusCode::SERVICE_UNAVAILABLE, "来源已关闭或配置已变化，请刷新后重试");
            }
            eprintln!("[路由] 成员「{}」配置已变化，跳过换下一个候选", entry.slug);
            continue;
        }
        let budget_block = deps.db.try_lock().ok().and_then(|conn| {
            crate::pricing::budget::check_gateway_connection_budget(
                &conn,
                &entry.connection_id,
                chrono::Utc::now().timestamp_millis(),
            )
        });
        if let Some(reason) = budget_block {
            if last {
                record_route(
                    deps,
                    prepared.request_id,
                    &plan.meta,
                    Some(&entry.slug),
                    None,
                    Some(429),
                    "rejected",
                    prepared.started,
                );
                return error_response(StatusCode::TOO_MANY_REQUESTS, &reason);
            }
            eprintln!("[路由] 成员「{}」{reason}，尝试下一个候选", entry.slug);
            continue;
        }
        let meta = linked(&plan.meta, &entry);
        let try_body = if upstream_model.is_empty() || upstream_model == prepared.model {
            prepared.body.clone()
        } else {
            rewrite_body_model(&prepared.body, &upstream_model).unwrap_or_else(|_| prepared.body.clone())
        };
        let try_body = match member_effort.as_deref() {
            Some(effort) => crate::gateway::member_effort::apply(prepared.protocol, &try_body, effort),
            None => try_body,
        };
        let target_model = if upstream_model.is_empty() {
            prepared.model.as_str()
        } else {
            upstream_model.as_str()
        };
        let native_tools = entry.platform == "claude"
            || crate::gateway::thinking_guard::serves_claude(entry.base_url.as_deref(), target_model);
        let response = if prepared.protocol == CallerProtocol::Anthropic && !native_tools {
            let (deps_ref, entry_ref, upstream_ref, headers_ref, session_ref, model_ref, meta_ref) = (
                deps,
                &entry,
                &upstream_model,
                &prepared.headers,
                &prepared.session,
                &prepared.model,
                &meta,
            );
            let (request_id, started, protocol) = (prepared.request_id, prepared.started, prepared.protocol);
            with_tool_loop(&try_body, |round_body| {
                let meta = meta_ref.clone();
                async move {
                    forward_one(
                        deps_ref,
                        entry_ref,
                        protocol,
                        upstream_ref,
                        &round_body,
                        path_and_query,
                        headers_ref,
                        session_ref,
                        request_id,
                        meta,
                        started,
                        model_ref,
                    )
                    .await
                }
            })
            .await
        } else {
            forward_one(
                deps,
                &entry,
                prepared.protocol,
                &upstream_model,
                &try_body,
                path_and_query,
                &prepared.headers,
                &prepared.session,
                prepared.request_id,
                meta,
                prepared.started,
                &prepared.model,
            )
            .await
        };
        if !is_retryable_status(response.status()) || last {
            return response;
        }
        if let Some(group_slug) = prepared.model.strip_prefix("group/") {
            let member_model = match member_effort.as_deref() {
                Some(effort) => format!("{upstream_model}:{effort}"),
                None => upstream_model.clone(),
            };
            crate::gateway::routing_groups::rest_member(
                group_slug,
                &entry.slug,
                &member_model,
                chrono::Utc::now().timestamp_millis(),
            );
        }
        eprintln!("[路由] 成员「{}」返回 {}，转移下一个候选", entry.slug, response.status());
    }
    error_response(StatusCode::SERVICE_UNAVAILABLE, "没有可用的候选成员")
}

/// 替身工具循环：请求声明了联网服务端工具时，网关执行模型的调用并续问。
pub(super) async fn with_tool_loop<F, Fut>(body: &Bytes, send: F) -> Response
where
    F: Fn(Bytes) -> Fut,
    Fut: std::future::Future<Output = Response>,
{
    with_tool_loop_within(body, crate::gateway::tool_loop::LOOP_BUDGET, send).await
}

/// 同 [`with_tool_loop`]，整体时限可注入（测试用短时限）。
pub(super) async fn with_tool_loop_within<F, Fut>(
    body: &Bytes,
    budget: std::time::Duration,
    send: F,
) -> Response
where
    F: Fn(Bytes) -> Fut,
    Fut: std::future::Future<Output = Response>,
{
    use crate::gateway::tool_loop;
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return send(body.clone()).await;
    };
    let managed = tool_loop::managed_tools(&value);
    if managed.is_empty() {
        return send(body.clone()).await;
    }
    let streaming = tool_loop::rewrite_request(&mut value, &managed);
    let mut usage = tool_loop::Usage::default();
    let mut notes: Vec<String> = Vec::new();
    let loop_started = std::time::Instant::now();
    for round in 0..tool_loop::MAX_ROUNDS {
        let request = serde_json::to_vec(&value).map(Bytes::from).unwrap_or_else(|_| body.clone());
        let response = send(request).await;
        if !response.status().is_success() {
            return response;
        }
        let (parts, response_body) = response.into_parts();
        let Ok(bytes) = axum::body::to_bytes(response_body, ROUND_BODY_LIMIT).await else {
            return error_response(StatusCode::BAD_GATEWAY, "上游回复读取失败");
        };
        let Ok(mut message) = serde_json::from_slice::<Value>(&bytes) else {
            return Response::from_parts(parts, axum::body::Body::from(bytes));
        };
        tool_loop::add_usage(&mut usage, &message);
        let (calls, foreign) = tool_loop::calls_in(&message, &managed);
        if calls.is_empty() {
            return deliver_message(tool_loop::finish(message, &notes, usage, &managed, None), streaming);
        }
        if foreign {
            tool_loop::strip_managed(&mut message, &managed);
            notes.push("模型同时调用了工具自己的功能，联网调用本轮未执行".into());
            return deliver_message(tool_loop::finish(message, &notes, usage, &managed, None), streaming);
        }
        if round + 1 == tool_loop::MAX_ROUNDS {
            return deliver_message(
                tool_loop::finish(message, &notes, usage, &managed, Some(tool_loop::rounds_exhausted_note())),
                streaming,
            );
        }
        if loop_started.elapsed() >= budget {
            let stop = tool_loop::budget_exhausted_note(budget);
            return deliver_message(tool_loop::finish(message, &notes, usage, &managed, Some(stop)), streaming);
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
