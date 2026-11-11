//! 命名客户端 Key 的每日 Token 预算（对齐 magpie `internal/budget`：预扣 + 回冲）。
//!
//! 为什么要预扣：用量要等响应结束才知道，只在请求前查「今天已花多少」会让并发请求
//! 一起越过上限。所以请求进来先按估算值预扣（在途），响应结束再把预扣换成实际用量。
//!
//! - 估算：请求体字节数 ÷ 4（输入的粗略上界）+ `max_tokens`（缺省 4096）。
//! - 实际：边转发边从响应里读 `usage`（Anthropic / OpenAI Chat / Responses / Gemini，
//!   流式与非流式）；读不到就按预扣额记账——宁可多算，不放超。
//! - 已花用量按「Key × 本地日期」存 SQLite；在途预扣只在内存（重启后没有在途请求）。

use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

/// 没写 `max_tokens` 时按这个输出上界估算。
const DEFAULT_OUTPUT_ESTIMATE: i64 = 4096;

pub(crate) fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS gateway_key_tokens (
            name  TEXT NOT NULL,
            day   TEXT NOT NULL,
            spent INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (name, day)
        );",
    )
}

/// 请求的 Token 估算值（预扣额）。
pub(crate) fn estimate(body: &[u8]) -> i64 {
    let output = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            ["max_tokens", "max_output_tokens", "max_completion_tokens"]
                .iter()
                .find_map(|key| value.get(key).and_then(Value::as_i64))
                .or_else(|| value.pointer("/generationConfig/maxOutputTokens").and_then(Value::as_i64))
        })
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_OUTPUT_ESTIMATE);
    (body.len() as i64 / 4) + output
}

fn day_of(now: chrono::DateTime<chrono::Local>) -> String {
    now.format("%Y-%m-%d").to_string()
}

/// 在途预扣：(Key, 日期) → 预扣合计。
static INFLIGHT: std::sync::LazyLock<Mutex<HashMap<(String, String), i64>>> = std::sync::LazyLock::new(Default::default);

/// 一次预扣；回冲时按它把在途额度换成实际用量。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reservation {
    pub name: String,
    pub day: String,
    pub estimate: i64,
}

pub(crate) fn spent(conn: &Connection, name: &str, day: &str) -> Result<i64, String> {
    init(conn).map_err(|_| "无法读取 Token 预算".to_string())?;
    conn.query_row("SELECT spent FROM gateway_key_tokens WHERE name = ?1 AND day = ?2", [name, day], |r| r.get(0))
        .optional()
        .map(|value| value.unwrap_or(0))
        .map_err(|_| "无法读取 Token 预算".to_string())
}

fn inflight(name: &str, day: &str) -> i64 {
    INFLIGHT.lock().map(|map| map.get(&(name.to_string(), day.to_string())).copied().unwrap_or(0)).unwrap_or(0)
}

/// 预扣：Key 没设 `tokens_per_day` 时返回 `Ok(None)`；今日已花 + 在途 + 本次估算超过上限时
/// 返回 429 拒绝（带距重置的秒数）。
pub(crate) fn reserve(
    conn: &Connection,
    name: &str,
    body: &[u8],
    now: chrono::DateTime<chrono::Local>,
) -> Result<Option<Reservation>, crate::gateway::access::RequestLimitRefusal> {
    let store_error = |_| crate::gateway::access::RequestLimitRefusal::unavailable("无法读取 Token 预算，请稍后重试");
    let limit: Option<i64> = conn
        .query_row("SELECT tokens_per_day FROM gateway_client_keys WHERE name = ?1", [name], |r| r.get(0))
        .optional()
        .map_err(store_error)?
        .flatten();
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return Ok(None);
    };
    let day = day_of(now);
    let estimate = estimate(body);
    let used = spent(conn, name, &day).map_err(|_| crate::gateway::access::RequestLimitRefusal::unavailable("无法读取 Token 预算，请稍后重试"))?;
    let mut map = INFLIGHT.lock().map_err(|_| crate::gateway::access::RequestLimitRefusal::unavailable("Token 预算暂不可用"))?;
    let pending = map.entry((name.to_string(), day.clone())).or_insert(0);
    if used + *pending + estimate > limit {
        let tomorrow = (now + chrono::Duration::days(1)).date_naive().and_hms_opt(0, 0, 0).unwrap_or_default();
        let reset = tomorrow.and_local_timezone(chrono::Local).earliest().unwrap_or(now);
        return Err(crate::gateway::access::RequestLimitRefusal::limited(
            format!(
                "客户端 Key「{name}」今日 Token 已用 {used}，在途 {pending}，本次约 {estimate}，超过上限 {limit}；将于 {} 重置",
                reset.format("%Y-%m-%d %H:%M")
            ),
            (reset - now).num_seconds().max(1),
        ));
    }
    *pending += estimate;
    Ok(Some(Reservation { name: name.to_string(), day, estimate }))
}

/// 回冲：把这次的预扣从在途里扣掉，记入实际用量（读不到用量按预扣额记）。
pub(crate) fn settle(conn: &Connection, reservation: &Reservation, actual: Option<i64>) {
    if let Ok(mut map) = INFLIGHT.lock() {
        let key = (reservation.name.clone(), reservation.day.clone());
        if let Some(pending) = map.get_mut(&key) {
            *pending = (*pending - reservation.estimate).max(0);
            if *pending == 0 {
                map.remove(&key);
            }
        }
    }
    let charged = actual.unwrap_or(reservation.estimate).max(0);
    if init(conn).is_err() {
        return;
    }
    let _ = conn.execute(
        "INSERT INTO gateway_key_tokens(name, day, spent) VALUES(?1, ?2, ?3)
         ON CONFLICT(name, day) DO UPDATE SET spent = spent + excluded.spent",
        rusqlite::params![reservation.name, reservation.day, charged],
    );
}

/* ───────────────────────── 从响应里读用量 ───────────────────────── */

/// 逐块喂响应字节，读出输入 / 输出 Token。流式事件里的用量是累计值，取各字段的最大值。
#[derive(Debug, Default)]
pub(crate) struct UsageMeter {
    line: Vec<u8>,
    /// 非 SSE 的整段 JSON（上限 4MB，超出放弃整段解析，只认已读到的 SSE 行）
    whole: Vec<u8>,
    input: Option<i64>,
    output: Option<i64>,
}

const WHOLE_LIMIT: usize = 4 * 1024 * 1024;

impl UsageMeter {
    pub(crate) fn feed(&mut self, chunk: &[u8]) {
        if self.whole.len() + chunk.len() <= WHOLE_LIMIT {
            self.whole.extend_from_slice(chunk);
        } else {
            self.whole.clear();
            self.whole.shrink_to_fit();
        }
        for byte in chunk {
            if *byte == b'\n' {
                let line = std::mem::take(&mut self.line);
                self.scan_line(&line);
            } else {
                self.line.push(*byte);
            }
        }
    }

    fn scan_line(&mut self, line: &[u8]) {
        let text = String::from_utf8_lossy(line);
        let Some(data) = text.trim().strip_prefix("data:") else { return };
        if let Ok(value) = serde_json::from_str::<Value>(data.trim()) {
            self.observe(&value);
        }
    }

    fn observe(&mut self, value: &Value) {
        let (input, output) = usage_of(value);
        if let Some(input) = input {
            self.input = Some(self.input.map_or(input, |seen| seen.max(input)));
        }
        if let Some(output) = output {
            self.output = Some(self.output.map_or(output, |seen| seen.max(output)));
        }
    }

    /// 流结束：补扫最后一行与整段 JSON，返回输入 + 输出合计；一项都没读到为 None。
    pub(crate) fn finish(&mut self) -> Option<i64> {
        let line = std::mem::take(&mut self.line);
        self.scan_line(&line);
        if self.input.is_none() && self.output.is_none() {
            if let Ok(value) = serde_json::from_slice::<Value>(&self.whole) {
                self.observe(&value);
            }
        }
        match (self.input, self.output) {
            (None, None) => None,
            (input, output) => Some(input.unwrap_or(0) + output.unwrap_or(0)),
        }
    }
}

impl UsageMeter {
    /// `finish` 之后读输入（含缓存读写）与输出；没读到的为 None。
    pub(crate) fn parts(&self) -> (Option<i64>, Option<i64>) {
        (self.input, self.output)
    }
}

fn int(value: &Value, pointer: &str) -> Option<i64> {
    value.pointer(pointer).and_then(Value::as_i64)
}

/// 各协议的用量位置。输入含缓存读写（它们同样占上游额度）。
fn usage_of(value: &Value) -> (Option<i64>, Option<i64>) {
    // Anthropic：message_start 的 message.usage，或非流式 / message_delta 的 usage
    for base in ["/message/usage", "/usage", "/response/usage"] {
        let Some(usage) = value.pointer(base) else { continue };
        let anthropic_in = int(usage, "/input_tokens").map(|input| {
            input + int(usage, "/cache_read_input_tokens").unwrap_or(0) + int(usage, "/cache_creation_input_tokens").unwrap_or(0)
        });
        let input = anthropic_in.or_else(|| int(usage, "/prompt_tokens"));
        let output = int(usage, "/output_tokens").or_else(|| int(usage, "/completion_tokens"));
        if input.is_some() || output.is_some() {
            return (input, output);
        }
    }
    // Gemini：usageMetadata
    if let Some(meta) = value.get("usageMetadata") {
        return (int(meta, "/promptTokenCount"), int(meta, "/candidatesTokenCount").map(|n| n + int(meta, "/thoughtsTokenCount").unwrap_or(0)));
    }
    (None, None)
}

/* ───────────────────────── 计量响应流：结束（或中断）时回冲 ───────────────────────── */

type Db = std::sync::Arc<Mutex<Connection>>;

struct MeteredStream {
    inner: axum::body::BodyDataStream,
    meter: UsageMeter,
    pending: Option<(Reservation, Db)>,
    /// 上游失败（非 2xx）且没读到用量时按 0 记：失败的请求不该吃掉预算
    success: bool,
}

impl MeteredStream {
    fn settle_once(&mut self) {
        let Some((reservation, db)) = self.pending.take() else { return };
        let actual = self.meter.finish().or(if self.success { None } else { Some(0) });
        let guard = db.lock();
        if let Ok(conn) = guard {
            settle(&conn, &reservation, actual);
        }
    }
}

impl futures_util::Stream for MeteredStream {
    type Item = Result<axum::body::Bytes, axum::Error>;

    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        let this = &mut *self;
        let polled = std::pin::Pin::new(&mut this.inner).poll_next(cx);
        match &polled {
            std::task::Poll::Ready(Some(Ok(bytes))) => this.meter.feed(bytes),
            std::task::Poll::Ready(None) | std::task::Poll::Ready(Some(Err(_))) => this.settle_once(),
            std::task::Poll::Pending => {}
        }
        polled
    }
}

impl Drop for MeteredStream {
    /// 客户端中途断开：按已读到的用量（或预扣额）回冲，不让在途额度永久挂着。
    fn drop(&mut self) {
        self.settle_once();
    }
}

/// 给响应套上计量：透传字节不缓冲，结束时把预扣换成实际用量。
pub(crate) fn meter(response: axum::response::Response, reservation: Reservation, db: Db) -> axum::response::Response {
    let success = response.status().is_success();
    let (parts, body) = response.into_parts();
    let stream = MeteredStream { inner: body.into_data_stream(), meter: UsageMeter::default(), pending: Some((reservation, db)), success };
    axum::response::Response::from_parts(parts, axum::body::Body::from_stream(stream))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE gateway_client_keys (name TEXT PRIMARY KEY, tokens_per_day INTEGER);").unwrap();
        init(&conn).unwrap();
        conn
    }

    fn now() -> chrono::DateTime<chrono::Local> {
        chrono::Local::now()
    }

    #[test]
    fn estimate_uses_body_size_and_declared_output_cap() {
        let body = br#"{"model":"m","max_tokens":100,"messages":[]}"#;
        assert_eq!(estimate(body), body.len() as i64 / 4 + 100);
        let gemini = br#"{"generationConfig":{"maxOutputTokens":50}}"#;
        assert_eq!(estimate(gemini), gemini.len() as i64 / 4 + 50);
        assert_eq!(estimate(b"{}"), DEFAULT_OUTPUT_ESTIMATE);
    }

    #[test]
    fn keys_without_token_limit_are_not_reserved() {
        let conn = conn();
        conn.execute("INSERT INTO gateway_client_keys VALUES('free', NULL)", []).unwrap();
        assert_eq!(reserve(&conn, "free", b"{}", now()).unwrap(), None);
    }

    #[test]
    fn inflight_reservations_count_against_the_limit_until_settled() {
        let conn = conn();
        conn.execute("INSERT INTO gateway_client_keys VALUES('k-inflight', 10000)", []).unwrap();
        let body = br#"{"max_tokens":4000}"#;
        let first = reserve(&conn, "k-inflight", body, now()).unwrap().unwrap();
        let second = reserve(&conn, "k-inflight", body, now()).unwrap().unwrap();
        // 两个在途共约 8000，第三个会越过 10000：并发请求不能一起越过上限
        let refused = reserve(&conn, "k-inflight", body, now()).unwrap_err();
        assert_eq!(refused.status, 429);
        assert!(refused.retry_after().is_some_and(|s| s >= 1));
        // 回冲后按实际用量计：第一条只花了 100，额度回到可用
        settle(&conn, &first, Some(100));
        settle(&conn, &second, Some(200));
        assert_eq!(spent(&conn, "k-inflight", &first.day).unwrap(), 300);
        assert!(reserve(&conn, "k-inflight", body, now()).unwrap().is_some());
    }

    #[test]
    fn unknown_usage_is_charged_at_the_estimate() {
        let conn = conn();
        conn.execute("INSERT INTO gateway_client_keys VALUES('k-unknown', 100000)", []).unwrap();
        let reservation = reserve(&conn, "k-unknown", br#"{"max_tokens":500}"#, now()).unwrap().unwrap();
        settle(&conn, &reservation, None);
        assert_eq!(spent(&conn, "k-unknown", &reservation.day).unwrap(), reservation.estimate);
    }

    #[tokio::test]
    async fn metered_response_settles_actual_usage_when_the_body_is_consumed() {
        let db = std::sync::Arc::new(Mutex::new(conn()));
        db.lock().unwrap().execute("INSERT INTO gateway_client_keys VALUES('k-stream', 100000)", []).unwrap();
        let reservation = reserve(&db.lock().unwrap(), "k-stream", br#"{"max_tokens":9000}"#, now()).unwrap().unwrap();
        let day = reservation.day.clone();
        let upstream = axum::response::Response::new(axum::body::Body::from("data: {\"usage\":{\"prompt_tokens\":30,\"completion_tokens\":12}}

"));
        let response = meter(upstream, reservation, db.clone());
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(!bytes.is_empty(), "字节原样透传");
        assert_eq!(spent(&db.lock().unwrap(), "k-stream", &day).unwrap(), 42);
        assert_eq!(inflight("k-stream", &day), 0, "回冲后不再占用在途额度");
    }

    #[tokio::test]
    async fn failed_upstream_without_usage_does_not_eat_the_budget() {
        let db = std::sync::Arc::new(Mutex::new(conn()));
        db.lock().unwrap().execute("INSERT INTO gateway_client_keys VALUES('k-failed', 100000)", []).unwrap();
        let reservation = reserve(&db.lock().unwrap(), "k-failed", b"{}", now()).unwrap().unwrap();
        let day = reservation.day.clone();
        let mut upstream = axum::response::Response::new(axum::body::Body::from("{\"error\":\"overloaded\"}"));
        *upstream.status_mut() = axum::http::StatusCode::SERVICE_UNAVAILABLE;
        drop(axum::body::to_bytes(meter(upstream, reservation, db.clone()).into_body(), usize::MAX).await.unwrap());
        assert_eq!(spent(&db.lock().unwrap(), "k-failed", &day).unwrap(), 0);
    }

    fn metered(chunks: &[&str]) -> Option<i64> {
        let mut meter = UsageMeter::default();
        for chunk in chunks {
            meter.feed(chunk.as_bytes());
        }
        meter.finish()
    }

    #[test]
    fn meter_reads_anthropic_stream_usage_across_chunk_boundaries() {
        let total = metered(&[
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":5,\"output_tokens\":1}}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_",
            "tokens\":42}}\n\n",
        ]);
        assert_eq!(total, Some(15 + 42));
    }

    #[test]
    fn meter_reads_openai_gemini_and_plain_json_bodies() {
        assert_eq!(metered(&["data: {\"choices\":[]}\n\ndata: {\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\ndata: [DONE]\n\n"]), Some(10));
        assert_eq!(metered(&["data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":4,\"output_tokens\":6}}}\n"]), Some(10));
        assert_eq!(metered(&["{\"usageMetadata\":{\"promptTokenCount\":8,", "\"candidatesTokenCount\":2}}"]), Some(10));
        assert_eq!(metered(&["{\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}"]), Some(3));
        assert_eq!(metered(&["{\"error\":\"x\"}"]), None);
    }
}
