//! 调用方协议封装：解析模型 → 路由选连接 → 协议匹配 → 适配器转发 → 按协议回封。
//! 跨协议组合返回明确的 400（不做静默翻译，见执行文档 §5.4）。

pub(crate) mod codec;
pub(crate) mod dispatch;
pub(crate) mod egress;
pub(crate) mod ingress;
pub(crate) mod select;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::proxy::ProxyDeps;

pub(crate) use codec::original_custom_tools;
pub(crate) use egress::{begin_route, linked, record_route};
pub(crate) use ingress::{agent_from_headers, session_identity};
#[cfg(test)]
pub(crate) use ingress::session_key;

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
    if count >= deps.circuit_threshold() {
        deps.consecutive_failures.store(0, Ordering::Relaxed);
        (deps.on_circuit)(reason);
    }
}

/// 读不到 model 时的说明。请求体不是 UTF-8 单独点名：Windows PowerShell 5 的
/// `Invoke-RestMethod -Body "<中文>"` 按本地代码页发送，中文会变成非法字节——
/// 这不是网关或脱敏改坏了 JSON，而是调用方的编码问题。
pub(crate) fn missing_model_reason(body: &[u8]) -> &'static str {
    if std::str::from_utf8(body).is_err() {
        "请求体不是 UTF-8 编码（常见于 Windows PowerShell 5 直接发送含中文的字符串），请以 UTF-8 字节发送"
    } else {
        "请求体缺少 model 字段或不是有效 JSON"
    }
}

pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 统一入口。`path_and_query` 原样拼到上游 base 后（路径参数与查询都保真）。
/// `client_key` 是鉴权中间件命中的命名客户端 Key（局域网共享时）；只进路由流水。
pub(crate) async fn handle(
    protocol: CallerProtocol,
    deps: Arc<ProxyDeps>,
    headers: HeaderMap,
    body: axum::body::Bytes,
    path_and_query: &str,
    client_key: Option<String>,
) -> Response {
    let prepared = match ingress::prepare(protocol, &deps, headers, body, client_key).await {
        Ok(ctx) => ctx,
        Err(early_response) => return early_response,
    };
    let response = match select::resolve_candidates(&deps, &prepared).await {
        Ok(plan) => dispatch::execute_with_failover(&deps, &prepared, plan, path_and_query).await,
        Err(reject_response) => reject_response,
    };
    egress::finalize(response, prepared)
}
