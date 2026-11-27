//! 观测模式零缓冲透传与首字/全程计时（零 `gateway` 依赖）。

use std::sync::Arc;
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;

use super::deps::{
    detect_platform, elapsed_ms, extract_served_model, now_ms, ProxyDeps, HEAD_CAP,
    REQUEST_BODY_LIMIT,
};
use crate::gateway::activity::{
    emit_end, emit_start, next_request_id, EndGuard, RouteEnd, RouteMeta, RouteStart,
};
use crate::proxy_config::ProxyPlatform;

pub(crate) fn build_upstream_url(base: &str, uri: &axum::http::Uri) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    match uri.query() {
        Some(query) => format!("{trimmed}{}?{query}", uri.path()),
        None => format!("{trimmed}{}", uri.path()),
    }
}

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn strip_request_headers(headers: &HeaderMap) -> HeaderMap {
    headers
        .iter()
        .filter(|(name, _)| {
            let name = name.as_str();
            !is_hop_by_hop(name)
                && !name.starts_with("proxy-")
                && name != "host"
                && name != "content-length"
                && name != "accept-encoding"
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn strip_response_headers(headers: &HeaderMap) -> HeaderMap {
    headers
        .iter()
        .filter(|(name, _)| {
            let name = name.as_str();
            !is_hop_by_hop(name) && name != "content-length" && name != "content-encoding"
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (status, message.to_string()).into_response()
}

struct ObserveActivity {
    request_id: u64,
    agent: &'static str,
    requested_model: String,
    source: String,
    guard: EndGuard,
}

struct TimingCore {
    deps: Arc<ProxyDeps>,
    platform: ProxyPlatform,
    connection_id: Option<String>,
    t0: Instant,
    status: StatusCode,
    first_ms: Option<i64>,
    head: Vec<u8>,
    head_full: bool,
    settled: bool,
    activity: Option<ObserveActivity>,
}

impl TimingCore {
    fn settle(&mut self, complete: bool) {
        if self.settled {
            return;
        }
        self.settled = true;
        let duration = elapsed_ms(self.t0);
        if let Some(act) = self.activity.take() {
            act.guard.finish();
            let served = extract_served_model(&self.head);
            let ok = complete && self.status.is_success();
            emit_end(
                &self.deps.on_activity,
                &RouteEnd {
                    request_id: act.request_id,
                    ts: now_ms(),
                    agent: Some(act.agent),
                    identified_by: Some("observe_proxy"),
                    unidentified_ua: None,
                    client_key: None,
                    requested_model: &act.requested_model,
                    source: Some(&act.source),
                    upstream_model: served.as_deref(),
                    status_code: Some(self.status.as_u16()),
                    result: if ok { "ok" } else { "failed" },
                    duration_ms: duration,
                    connection_id: self.connection_id.as_deref(),
                    session_platform: None,
                    session_id: None,
                },
            );
        }
        if !complete {
            return;
        }
        let first = self.first_ms.unwrap_or(duration);
        self.deps.settle_timing(
            self.platform,
            &self.head,
            first,
            duration,
            self.status.as_u16() as i64,
            self.connection_id.as_deref(),
        );
    }
}

fn is_inference_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains("/messages") || p.contains("/responses") || p.contains("/completions")
}

fn extract_request_model(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(str::trim).map(str::to_string))
        .filter(|m| !m.is_empty())
        .map(|m| m.chars().take(80).collect())
        .unwrap_or_else(|| "default".into())
}

fn observe_source_label(deps: &ProxyDeps, connection_id: Option<&str>) -> String {
    let Some(id) = connection_id else { return "direct".into() };
    let Ok(conn) = deps.db.try_lock() else { return "direct".into() };
    conn.query_row(
        "SELECT COALESCE(slug, name) FROM connections WHERE id = ?1",
        [id],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|_| "direct".into())
}

fn timed_body(response: reqwest::Response, core: TimingCore) -> Body {
    let stream = response.bytes_stream();
    let timed = futures_util::stream::unfold((stream, Some(core)), |(mut stream, core)| async move {
        let Some(mut core) = core else {
            return None;
        };
        match stream.next().await {
            Some(Ok(chunk)) => {
                if core.first_ms.is_none() {
                    core.first_ms = Some(elapsed_ms(core.t0));
                }
                if !core.head_full {
                    let take = (HEAD_CAP - core.head.len()).min(chunk.len());
                    core.head.extend_from_slice(&chunk[..take]);
                    core.head_full = core.head.len() >= HEAD_CAP;
                }
                Some((Ok::<Bytes, reqwest::Error>(chunk), (stream, Some(core))))
            }
            Some(Err(_)) => {
                core.settle(false);
                None
            }
            None => {
                core.settle(true);
                None
            }
        }
    });
    Body::from_stream(timed)
}

pub(crate) async fn forward(State(deps): State<Arc<ProxyDeps>>, request: Request) -> Response {
    let t0 = Instant::now();
    let (parts, body) = request.into_parts();
    let platform = detect_platform(&parts.headers, parts.uri.path());
    let Some((platform, base)) = deps.upstream_for(platform) else {
        return error_response(
            StatusCode::BAD_GATEWAY,
            "本地代理：无法判定该请求属于哪个平台的上游（多平台接管时不猜测）",
        );
    };
    let connection_id = deps.connection_for(platform, &parts.headers);

    let body_bytes = match axum::body::to_bytes(body, REQUEST_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                &format!("本地代理：读取请求体失败：{error}"),
            );
        }
    };

    let activity = if is_inference_path(parts.uri.path()) {
        let request_id = next_request_id();
        let agent = match platform {
            ProxyPlatform::Claude => "claude-code",
            ProxyPlatform::Codex => "codex",
        };
        let requested_model = extract_request_model(&body_bytes);
        let source = observe_source_label(&deps, connection_id.as_deref());
        let meta = RouteMeta {
            request_uid: None,
            agent: Some(agent.into()),
            identified_by: Some("observe_proxy"),
            unidentified_ua: None,
            requested_model: requested_model.clone(),
            connection_id: connection_id.clone(),
            session: None,
            client_key: None,
            decision: None,
        };
        emit_start(
            &deps.on_activity,
            &RouteStart {
                request_id,
                ts: now_ms(),
                agent: Some(agent),
                identified_by: Some("observe_proxy"),
                unidentified_ua: None,
                client_key: None,
                requested_model: &requested_model,
                source: &source,
                connection_id: connection_id.as_deref(),
                session_platform: None,
                session_id: None,
            },
        );
        let guard = EndGuard::new(deps.on_activity.clone(), request_id, &meta, &source, t0);
        Some(ObserveActivity { request_id, agent, requested_model, source, guard })
    } else {
        None
    };

    let mut upstream_request = deps
        .client
        .request(parts.method.clone(), build_upstream_url(base, &parts.uri))
        .headers(strip_request_headers(&parts.headers));
    if !body_bytes.is_empty() {
        upstream_request = upstream_request.body(body_bytes);
    }

    let response = match upstream_request.send().await {
        Ok(response) => response,
        Err(error) => {
            drop(activity);
            deps.note_upstream_unreachable(format!("上游不可达：{error}"));
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("本地代理：上游不可达，{error}"),
            );
        }
    };
    deps.note_success();

    let status = response.status();
    let headers = strip_response_headers(response.headers());
    let core = TimingCore {
        deps: deps.clone(),
        platform,
        connection_id,
        t0,
        status,
        first_ms: None,
        head: Vec::new(),
        head_full: false,
        settled: false,
        activity,
    };
    let mut builder = Response::builder().status(status);
    match builder.headers_mut() {
        Some(slot) => {
            *slot = headers;
            slot.insert("x-jolu-proxy", axum::http::HeaderValue::from_static("1"));
        }
        None => return error_response(StatusCode::BAD_GATEWAY, "本地代理：构造响应失败"),
    }
    builder.body(timed_body(response, core)).unwrap_or_else(|error| {
        error_response(
            StatusCode::BAD_GATEWAY,
            &format!("本地代理：构造响应失败：{error}"),
        )
    })
}
