use axum::body::Body;
use axum::response::Response;
use super::*;

/* ───────────────────────────── 网关接入点 ───────────────────────────── */

/// 响应侧缓冲上限（与 translate.rs 的上游缓冲同量级）
const RESPONSE_BUFFER_LIMIT: usize = 64 * 1024 * 1024;

/// 请求侧入口（server.rs 在鉴权后、选路前对三类协议端点调用）。
/// - 关闭：原样；
/// - 开启且引擎可用：命中才改写，未命中原样；
/// - 开启但引擎不可用：`Err`（503 拒发）——绝不原样放行。
pub(crate) fn prepare_request(body: axum::body::Bytes, entry: &str) -> Result<axum::body::Bytes, Response> {
    let Some(masker) = request_masker() else { return Ok(body) };
    let policy = policy();
    let masked = if masker.available() { masker.mask_body_with(&body, &policy) } else { Err(()) };
    match masked {
        Ok((masked, report)) => {
            remember(entry, &report);
            if !report.blocked.is_empty() {
                let kinds: Vec<&str> = report.blocked.iter().map(|k| kind_label(k)).collect();
                return Err(crate::gateway::wire::error_response(
                    axum::http::StatusCode::BAD_REQUEST,
                    &format!("请求里带有设为「拦下」的{}，Jolu 没有把它发出；可在设置 › 隐私脱敏里改成「遮住」", kinds.join("、")),
                ));
            }
            match masked {
                Some(masked) => {
                    eprintln!("[网关] 隐私脱敏：请求体遮蔽 {} 处", report.masked);
                    Ok(masked)
                }
                None => Ok(body),
            }
        }
        Err(()) => Err(crate::gateway::wire::error_response(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "隐私脱敏已开启但引擎暂不可用，为避免原文离开本机已拒绝本次请求",
        )),
    }
}

/// 响应侧入口（server.rs 在 wire::handle 返回后调用）：把响应里的占位符换回真值。
/// - SSE：逐块回填，尾部保留缓冲处理被分块切割的占位符；
/// - JSON / 文本：整段缓冲后回填；
/// - 其余二进制：不动。
///
/// 回填是尽力而为：占位符本身不是机密，恢复不了最多原样露出占位符，不构成泄漏；
/// 这与请求侧「失败即拒发」不同——响应已经从上游回来了，拦下它没有安全收益。
/// 响应侧中间件的装饰层（think-tags）：非流式 JSON 整段处理；`text/event-stream`
/// 逐事件交给 `middleware::EventChain`（onEvent）。
pub(crate) async fn restore_middleware(response: Response, middlewares: &[crate::gateway::middleware::Middleware]) -> Response {
    if !middlewares.iter().any(|m| m.enabled && m.kind == "think-tags") {
        return response;
    }
    let is_stream = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("event-stream"));
    if is_stream {
        // 流式：逐事件处理（onEvent，3.7）
        let Some(chain) = crate::gateway::middleware::EventChain::new(middlewares) else { return response };
        let (mut parts, body) = response.into_parts();
        parts.headers.remove(axum::http::header::CONTENT_LENGTH);
        return Response::from_parts(parts, Body::from_stream(crate::gateway::middleware::transform_sse(body.into_data_stream(), chain)));
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await else {
        // 读不出来就交回原响应——响应侧中间件是装饰，不能因此让请求失败
        return Response::from_parts(parts, axum::body::Body::empty());
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Response::from_parts(parts, axum::body::Body::from(bytes));
    };
    let next = crate::gateway::middleware::restore_chain(middlewares, text);
    let next_bytes = next.into_bytes();
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    if let Ok(value) = axum::http::HeaderValue::from_str(&next_bytes.len().to_string()) {
        parts.headers.insert(axum::http::header::CONTENT_LENGTH, value);
    }
    Response::from_parts(parts, axum::body::Body::from(next_bytes))
}

pub(crate) async fn restore_response(response: Response) -> Response {
    let Some(masker) = request_masker() else { return response };
    // 本进程从没遮过任何值：响应里不可能有占位符，直接放行
    if masker.hit_count() == 0 {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let content_type = parts
        .headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if content_type.contains("text/event-stream") {
        let unmarker = StreamUnmarker::new(masker);
        use futures_util::StreamExt as _;
        let stream = futures_util::stream::unfold(
            (body.into_data_stream(), unmarker, false),
            |(mut upstream, mut unmarker, ended)| async move {
                if ended {
                    return None;
                }
                loop {
                    match upstream.next().await {
                        Some(Ok(chunk)) => {
                            let out = unmarker.feed(&chunk);
                            if !out.is_empty() {
                                return Some((
                                    Ok::<axum::body::Bytes, std::convert::Infallible>(axum::body::Bytes::from(out)),
                                    (upstream, unmarker, false),
                                ));
                            }
                        }
                        // 上游结束（或出错）：把扣住的尾部回填后放行
                        _ => {
                            let out = unmarker.finish();
                            return Some((Ok(axum::body::Bytes::from(out)), (upstream, unmarker, true)));
                        }
                    }
                }
            },
        );
        // 分块边界变了：原 Content-Length 不再成立
        parts.headers.remove(axum::http::header::CONTENT_LENGTH);
        return Response::from_parts(parts, Body::from_stream(stream));
    }
    if !content_type.contains("json") && !content_type.contains("text") {
        // 二进制（图片 / 音频等）：没有占位符可言，原样透传
        return Response::from_parts(parts, body);
    }
    let bytes = match axum::body::to_bytes(body, RESPONSE_BUFFER_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return crate::gateway::wire::error_response(
                axum::http::StatusCode::BAD_GATEWAY,
                &format!("读取上游回复失败：{error}"),
            );
        }
    };
    let restored = if find_from(&bytes, 0, PLACEHOLDER_PREFIX.as_bytes()).is_some() {
        match std::str::from_utf8(&bytes) {
            Ok(text) => masker.unmark_text(text).into_bytes(),
            Err(_) => bytes.to_vec(),
        }
    } else {
        bytes.to_vec()
    };
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(restored))
}
