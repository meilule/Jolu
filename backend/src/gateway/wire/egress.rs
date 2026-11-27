//! 出口收尾：路由流水写入、流中断改判、转换后调用方计时回填、上下文与预算计量及并发槽释放。

use std::sync::Arc;

use axum::response::Response;

use super::ingress::PreparedRequest;
use super::CallerProtocol;
use crate::gateway::activity::RouteMeta;
use crate::gateway::route;
use crate::proxy::ProxyDeps;

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
) -> crate::gateway::route_log::RowSlot {
    use crate::gateway::activity::{emit_end, RouteEnd};
    let duration_ms = started.elapsed().as_millis() as i64;
    let ts = chrono::Local::now().timestamp_millis();
    emit_end(
        &deps.on_activity,
        &RouteEnd {
            request_id,
            ts,
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
    crate::gateway::route_log::queue_record(
        &deps.db,
        ts,
        owned_entry(meta, request_id, source, upstream, status, result, duration_ms),
    )
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
    let (db, meta_for_row, source) = (deps.db.clone(), meta.clone(), entry.slug.clone());
    crate::gateway::activity::EndGuard::new(deps.on_activity.clone(), request_id, meta, &entry.slug, started).on_drop(
        move |duration_ms| {
            crate::gateway::route_log::queue_record(
                &db,
                chrono::Local::now().timestamp_millis(),
                owned_entry(&meta_for_row, request_id, Some(&source), None, None, "cancelled", duration_ms),
            );
        },
    )
}

fn owned_entry(
    meta: &RouteMeta,
    request_id: u64,
    source: Option<&str>,
    upstream: Option<&str>,
    status: Option<u16>,
    result: &str,
    duration_ms: i64,
) -> crate::gateway::route_log::OwnedEntry {
    crate::gateway::route_log::OwnedEntry {
        agent: meta.agent.clone(),
        requested_model: meta.requested_model.clone(),
        source: source.map(str::to_string),
        upstream_model: upstream.map(str::to_string),
        status_code: status.map(i64::from),
        result: result.to_string(),
        duration_ms: Some(duration_ms),
        connection_id: meta.connection_id.clone(),
        session_platform: meta.session_platform().map(str::to_string),
        session_id: meta.session_id().map(str::to_string),
        client_key: meta.client_key.clone(),
        request_id: Some(request_id as i64),
        request_uid: meta.request_uid,
        resolved_from: (!meta.requested_model.contains('/')).then_some("bare"),
        profile: crate::gateway::profiles::active(),
        decision: meta.decision.clone(),
    }
}

/// 流式响应包一层：没读完时改判流水，改判成功后补发同 `request_id` 的结束事件（P4 审计 D2）。
pub(super) fn watch_stream(
    deps: &Arc<ProxyDeps>,
    response: Response,
    row: crate::gateway::route_log::RowSlot,
    request_id: u64,
    meta: &RouteMeta,
    source: &str,
    started: std::time::Instant,
) -> Response {
    let (emit, meta, source) = (deps.on_activity.clone(), meta.clone(), source.to_string());
    let status = response.status().as_u16();
    crate::gateway::route_log::CancelWatch::watch_notify(response, &deps.db, row, move |end| {
        crate::gateway::activity::emit_end(
            &emit,
            &crate::gateway::activity::RouteEnd {
                request_id,
                ts: chrono::Local::now().timestamp_millis(),
                agent: meta.agent.as_deref(),
                identified_by: meta.identified_by,
                unidentified_ua: meta.unidentified_ua.as_deref(),
                client_key: meta.client_key.as_deref(),
                requested_model: &meta.requested_model,
                source: Some(&source),
                upstream_model: None,
                status_code: Some(status),
                result: end.result(),
                duration_ms: started.elapsed().as_millis() as i64,
                connection_id: meta.connection_id.as_deref(),
                session_platform: meta.session_platform(),
                session_id: meta.session_id(),
            },
        );
    })
}

/// 选中连接后的元数据（连接 id 只在命中来源时才有）。
pub(crate) fn linked(meta: &RouteMeta, entry: &route::PoolEntry) -> RouteMeta {
    RouteMeta { connection_id: Some(entry.connection_id.clone()), ..meta.clone() }
}

/// 协议转换后的响应流计时观察：提取面向调用方的去重键（如 msg_ / resp_）并回填首字与总耗时。
pub(super) fn observe_caller_timing(
    deps: Arc<ProxyDeps>,
    entry: &route::PoolEntry,
    upstream_model: &str,
    protocol: CallerProtocol,
    started: std::time::Instant,
    response: Response,
) -> Response {
    let status = response.status().as_u16() as i64;
    let (parts, body) = response.into_parts();
    let platform = match protocol {
        CallerProtocol::Anthropic => "claude",
        CallerProtocol::Responses | CallerProtocol::Chat => "codex",
    };
    struct CallerTimed {
        inner: axum::body::BodyDataStream,
        deps: Arc<ProxyDeps>,
        connection_id: String,
        platform: &'static str,
        upstream_model: String,
        started: std::time::Instant,
        status: i64,
        first_byte_ms: Option<i64>,
        head: Vec<u8>,
        settled: bool,
    }
    impl CallerTimed {
        fn settle(&mut self) {
            if self.settled {
                return;
            }
            self.settled = true;
            crate::gateway::adapter::settle_gateway_timing_with_model(
                &self.deps,
                &self.connection_id,
                self.platform,
                Some(&self.upstream_model),
                &self.head,
                self.first_byte_ms,
                self.started.elapsed().as_millis() as i64,
                self.status,
            );
        }
    }
    impl futures_util::Stream for CallerTimed {
        type Item = Result<axum::body::Bytes, axum::Error>;
        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            let polled = std::pin::Pin::new(&mut self.inner).poll_next(cx);
            match &polled {
                std::task::Poll::Ready(Some(Ok(bytes))) => {
                    if !bytes.is_empty() && self.first_byte_ms.is_none() {
                        self.first_byte_ms = Some(self.started.elapsed().as_millis() as i64);
                    }
                    if self.head.len() < crate::proxy::HEAD_CAP {
                        let room = crate::proxy::HEAD_CAP - self.head.len();
                        self.head.extend_from_slice(&bytes[..bytes.len().min(room)]);
                    }
                }
                std::task::Poll::Ready(None) | std::task::Poll::Ready(Some(Err(_))) => self.settle(),
                std::task::Poll::Pending => {}
            }
            polled
        }
    }
    impl Drop for CallerTimed {
        fn drop(&mut self) {
            self.settle();
        }
    }
    let stream = CallerTimed {
        inner: body.into_data_stream(),
        deps,
        connection_id: entry.connection_id.clone(),
        platform,
        upstream_model: upstream_model.to_string(),
        started,
        status,
        first_byte_ms: None,
        head: Vec::new(),
        settled: false,
    };
    Response::from_parts(parts, axum::body::Body::from_stream(stream))
}

pub(super) fn finalize(response: Response, prepared: PreparedRequest) -> Response {
    let response = match prepared.capture.as_ref() {
        Some(capture) => capture.meter(response),
        None => response,
    };
    let response = match prepared.reservation {
        Some(reservation) => crate::gateway::budget::meter(response, reservation, prepared.budget_db),
        None => response,
    };
    crate::gateway::key_slots::hold(response, prepared.slot)
}
