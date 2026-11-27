//! 订阅源协议支持判定与统一分发入口。

use std::sync::Arc;

use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::Response;

use crate::gateway::route;
use crate::gateway::wire::{codec, error_response, CallerProtocol};
use crate::proxy::ProxyDeps;

/// 400 错误体最多读这么多来判断是否 thinking 签名拒绝；错误体通常几百字节。
const REFUSAL_BODY_LIMIT: usize = 64 * 1024;

/// 发往 Claude 的一次转发，带 thinking 签名的预防剔除与「被拒后修一次」。
pub(crate) async fn with_thinking_repair<F, Fut>(
    owner: &str,
    base_url: Option<&str>,
    upstream_model: &str,
    body: &Bytes,
    send: F,
) -> Response
where
    F: Fn(Bytes) -> Fut,
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
    let first = if stripped == 0 {
        body.clone()
    } else {
        serde_json::to_vec(&value).map(Into::into).unwrap_or_else(|_| body.clone())
    };
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

/// 订阅平台在发开始事件之前的协议兼容判定。
pub(crate) fn check_protocol_support(platform: &str, protocol: CallerProtocol) -> Result<(), Response> {
    match platform {
        "codex" | "opencode" if protocol == CallerProtocol::Anthropic => {
            Err(error_response(StatusCode::BAD_REQUEST, "Codex 订阅只服务 OpenAI 协议请求"))
        }
        "claude" if protocol != CallerProtocol::Anthropic => {
            Err(error_response(StatusCode::BAD_REQUEST, "Claude 订阅当前只服务 Anthropic Messages 协议请求"))
        }
        "codex" | "opencode" | "claude" | "grok" | "workbuddy" | "qoder" | "trae" | "zcode" | "gemini" => Ok(()),
        _ => Err(error_response(StatusCode::NOT_IMPLEMENTED, "该平台的订阅适配器尚未接入")),
    }
}

/// 订阅适配器统一分发入口（含 Chat-only 订阅双向转换与 Grok / WorkBuddy / ZCode 流尾记账）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn forward_subscription(
    deps: &Arc<ProxyDeps>,
    entry: &route::PoolEntry,
    protocol: CallerProtocol,
    upstream_model: &str,
    requested_model: &str,
    body: &Bytes,
    session: &str,
    started: std::time::Instant,
) -> Response {
    let platform = entry.platform.as_str();
    let response = match platform {
        "codex" | "opencode" => super::codex::forward(deps.clone(), entry, protocol, body.clone()).await,
        "claude" => {
            with_thinking_repair(&entry.connection_id, Some("https://api.anthropic.com"), upstream_model, body, |b| {
                super::claude::forward(deps.clone(), entry, session, b)
            })
            .await
        }
        "grok" => super::grok::forward(deps.clone(), entry, protocol, body.clone()).await,
        "zcode" => super::zcode::forward(deps.clone(), entry, protocol, body.clone(), session).await,
        "gemini" => {
            let model = if upstream_model.is_empty() {
                route::split_model_id(requested_model)
                    .map(|(_, m)| m.to_string())
                    .unwrap_or_else(|| requested_model.to_string())
            } else {
                upstream_model.to_string()
            };
            match codec::to_chat_body(protocol, body, false) {
                Err(err_resp) => err_resp,
                Ok(chat) => {
                    let resp = super::gemini_subscription::forward_chat(deps.clone(), entry, &model, chat).await;
                    codec::from_chat_response(protocol, resp, requested_model, body).await
                }
            }
        }
        _ => match codec::to_chat_body(protocol, body, false) {
            Err(err_resp) => err_resp,
            Ok(chat_body) => {
                let chat_protocol = match protocol {
                    CallerProtocol::Anthropic | CallerProtocol::Responses => CallerProtocol::Chat,
                    other => other,
                };
                let resp = match platform {
                    "workbuddy" => super::workbuddy::forward(deps.clone(), entry, chat_protocol, chat_body).await,
                    "qoder" => super::qoder::forward(deps.clone(), entry, chat_protocol, chat_body).await,
                    _ => super::trae::forward(deps.clone(), entry, chat_protocol, chat_body, session).await,
                };
                codec::from_chat_response(protocol, resp, requested_model, body).await
            }
        },
    };
    if matches!(platform, "grok" | "workbuddy" | "zcode") {
        let model = Some(if upstream_model.is_empty() {
            requested_model.to_string()
        } else {
            upstream_model.to_string()
        });
        crate::gateway::subscription::record_usage_on_end(deps.clone(), entry.clone(), model, started, response)
    } else {
        response
    }
}
