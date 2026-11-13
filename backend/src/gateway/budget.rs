//! 命名客户端 Key 的每日 Token 预算（对齐 magpie `internal/budget`：预扣 + 回冲）。
//!
//! 为什么要预扣：用量要等响应结束才知道，只在请求前查「今天已花多少」会让并发请求
//! 一起越过上限。所以请求进来先按估算值预扣（在途），响应结束再把预扣换成实际用量。
//!
//! - 估算：请求体字节数 ÷ 4（输入的粗略上界）+ `max_tokens`（缺省 4096）。
//! - 实际：边转发边从响应里读 `usage`（Anthropic / OpenAI Chat / Responses / Gemini，
//!   流式与非流式）；读不到就按预扣额记账——宁可多算，不放超。
//! - 已花用量按「Key × 本地日期」存 SQLite；在途预扣只在内存（重启后没有在途请求）。
//! - 周 / 月上限是对日行求和：周从周一算起，月从 1 号算起（本地时间）。
//! - 费用：响应结束时按实际路由到的平台与模型、用价目表估算（缓存读写按输入价计，偏高）；
//!   价目表里没有的模型不计费用、也不参与费用上限——未知不当零，也不按估计拦人。
//!   费用只能在响应后知道，所以费用上限是「已花达到上限后拒绝」，不做预扣。

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
    )?;
    // 费用（美元的百万分之一，整数存避免浮点累加误差）；旧表补列
    let _ = conn.execute("ALTER TABLE gateway_key_tokens ADD COLUMN cost_micros INTEGER NOT NULL DEFAULT 0", []);
    Ok(())
}

/// 统计窗口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Window {
    Day,
    Week,
    Month,
}

impl Window {
    fn label(self) -> &'static str {
        match self {
            Window::Day => "今日",
            Window::Week => "本周",
            Window::Month => "本月",
        }
    }

    /// 窗口起始日（含）与下一次重置的时刻。
    fn bounds(self, now: chrono::DateTime<chrono::Local>) -> (String, chrono::DateTime<chrono::Local>) {
        use chrono::Datelike;
        let today = now.date_naive();
        let (start, next) = match self {
            Window::Day => (today, today + chrono::Duration::days(1)),
            Window::Week => {
                let monday = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
                (monday, monday + chrono::Duration::days(7))
            }
            Window::Month => {
                let first = today.with_day(1).unwrap_or(today);
                let next = if first.month() == 12 {
                    chrono::NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
                } else {
                    chrono::NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
                }
                .unwrap_or(first + chrono::Duration::days(31));
                (first, next)
            }
        };
        let reset = next.and_hms_opt(0, 0, 0).unwrap_or_default().and_local_timezone(chrono::Local).earliest().unwrap_or(now);
        (start.format("%Y-%m-%d").to_string(), reset)
    }
}

/// 窗口内已花（Token，美元）。
pub(crate) fn spent_in(conn: &Connection, name: &str, window: Window, now: chrono::DateTime<chrono::Local>) -> Result<(i64, f64), String> {
    init(conn).map_err(|_| "无法读取 Token 预算".to_string())?;
    let (from, _) = window.bounds(now);
    let to = day_of(now);
    conn.query_row(
        "SELECT COALESCE(SUM(spent), 0), COALESCE(SUM(cost_micros), 0) FROM gateway_key_tokens WHERE name = ?1 AND day >= ?2 AND day <= ?3",
        [name, from.as_str(), to.as_str()],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? as f64 / 1_000_000.0)),
    )
    .map_err(|_| "无法读取 Token 预算".to_string())
}

/// 一把 Key 的 Token 与费用上限（都可空 = 不限）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeyBudget {
    #[serde(default)]
    pub tokens_per_day: Option<i64>,
    #[serde(default)]
    pub tokens_per_week: Option<i64>,
    #[serde(default)]
    pub tokens_per_month: Option<i64>,
    /// 美元
    #[serde(default)]
    pub cost_per_day: Option<f64>,
    #[serde(default)]
    pub cost_per_week: Option<f64>,
    #[serde(default)]
    pub cost_per_month: Option<f64>,
}

impl KeyBudget {
    fn tokens(&self, window: Window) -> Option<i64> {
        match window {
            Window::Day => self.tokens_per_day,
            Window::Week => self.tokens_per_week,
            Window::Month => self.tokens_per_month,
        }
        .filter(|n| *n > 0)
    }
    fn cost(&self, window: Window) -> Option<f64> {
        match window {
            Window::Day => self.cost_per_day,
            Window::Week => self.cost_per_week,
            Window::Month => self.cost_per_month,
        }
        .filter(|n| *n > 0.0)
    }
}

pub(crate) fn budget_of(conn: &Connection, name: &str) -> rusqlite::Result<Option<KeyBudget>> {
    // 新列可能还没迁移（旧库、这次启动还没碰过 Key 表）：先补列，否则每个带 Key 的请求都会 503
    let _ = crate::gateway::access::client_store_init(conn);
    conn.query_row(
        "SELECT tokens_per_day, tokens_per_week, tokens_per_month, cost_per_day, cost_per_week, cost_per_month
           FROM gateway_client_keys WHERE name = ?1",
        [name],
        |r| {
            Ok(KeyBudget {
                tokens_per_day: r.get(0)?,
                tokens_per_week: r.get(1)?,
                tokens_per_month: r.get(2)?,
                cost_per_day: r.get(3)?,
                cost_per_week: r.get(4)?,
                cost_per_month: r.get(5)?,
            })
        },
    )
    .optional()
}

/// 本次请求实际路由到的（平台，模型）：候选循环每试一个来源就更新，响应结束时按它计价。
pub(crate) type Routed = std::sync::Arc<Mutex<Option<(String, String)>>>;

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
#[derive(Debug, Clone)]
pub(crate) struct Reservation {
    pub name: String,
    pub day: String,
    pub estimate: i64,
    /// 计价用：实际路由到的平台与模型（没路由成功则为空，不计费用）
    pub routed: Routed,
}

#[cfg(test)]
#[cfg(test)]
#[cfg(test)]
pub(crate) fn spent(conn: &Connection, name: &str, day: &str) -> Result<i64, String> {
    init(conn).map_err(|_| "无法读取 Token 预算".to_string())?;
    conn.query_row("SELECT spent FROM gateway_key_tokens WHERE name = ?1 AND day = ?2", [name, day], |r| r.get(0))
        .optional()
        .map(|value| value.unwrap_or(0))
        .map_err(|_| "无法读取 Token 预算".to_string())
}

#[cfg(test)]
#[cfg(test)]
#[cfg(test)]
fn inflight(name: &str, day: &str) -> i64 {
    INFLIGHT.lock().map(|map| map.get(&(name.to_string(), day.to_string())).copied().unwrap_or(0)).unwrap_or(0)
}

/// 预扣：命名 Key 一律记账（便于按 Key 出账）；任一窗口的 Token 上限会被「已花 + 在途 + 本次估算」
/// 超过、或任一窗口的费用已达上限时，返回 429 拒绝（带距该窗口重置的秒数）。Key 不存在时不记账。
pub(crate) fn reserve(
    conn: &Connection,
    name: &str,
    body: &[u8],
    now: chrono::DateTime<chrono::Local>,
    routed: Routed,
) -> Result<Option<Reservation>, crate::gateway::access::RequestLimitRefusal> {
    let unavailable = || crate::gateway::access::RequestLimitRefusal::unavailable("无法读取 Token 预算，请稍后重试");
    let Some(budget) = budget_of(conn, name).map_err(|_| unavailable())? else {
        return Ok(None);
    };
    let day = day_of(now);
    let estimate = estimate(body);
    let mut map = INFLIGHT.lock().map_err(|_| crate::gateway::access::RequestLimitRefusal::unavailable("Token 预算暂不可用"))?;
    // 在途只会是今天的请求，各窗口共用同一份在途
    let pending = map.get(&(name.to_string(), day.clone())).copied().unwrap_or(0);
    for window in [Window::Day, Window::Week, Window::Month] {
        let token_limit = budget.tokens(window);
        let cost_limit = budget.cost(window);
        if token_limit.is_none() && cost_limit.is_none() {
            continue;
        }
        let (used, cost) = spent_in(conn, name, window, now).map_err(|_| unavailable())?;
        let (_, reset) = window.bounds(now);
        let refuse = |message: String| {
            crate::gateway::access::RequestLimitRefusal::limited(
                format!("{message}；将于 {} 重置", reset.format("%Y-%m-%d %H:%M")),
                (reset - now).num_seconds().max(1),
            )
        };
        if let Some(limit) = token_limit {
            if used + pending + estimate > limit {
                return Err(refuse(format!(
                    "客户端 Key「{name}」{}Token 已用 {used}，在途 {pending}，本次约 {estimate}，超过上限 {limit}",
                    window.label()
                )));
            }
        }
        if let Some(limit) = cost_limit {
            if cost >= limit {
                return Err(refuse(format!("客户端 Key「{name}」{}费用已达 ${cost:.4}，上限 ${limit:.2}", window.label())));
            }
        }
    }
    *map.entry((name.to_string(), day.clone())).or_insert(0) += estimate;
    Ok(Some(Reservation { name: name.to_string(), day, estimate, routed }))
}

#[cfg(test)]
#[cfg(test)]
#[cfg(test)]
/// 回冲：把这次的预扣从在途里扣掉，记入实际用量（读不到用量按预扣额记）与费用（能计价才记）。
pub(crate) fn settle(conn: &Connection, reservation: &Reservation, actual: Option<i64>) {
    settle_with_cost(conn, reservation, actual, None)
}

/// 同 [`settle`]，另记这次的费用（美元）。
pub(crate) fn settle_with_cost(conn: &Connection, reservation: &Reservation, actual: Option<i64>, cost: Option<f64>) {
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
        "INSERT INTO gateway_key_tokens(name, day, spent, cost_micros) VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(name, day) DO UPDATE SET spent = spent + excluded.spent, cost_micros = cost_micros + excluded.cost_micros",
        rusqlite::params![reservation.name, reservation.day, charged, cost.map(|c| (c.max(0.0) * 1_000_000.0).round() as i64).unwrap_or(0)],
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
        // 费用：读到了用量、且知道实际路由到的平台与模型、价目表里有它，才计
        let (input, output) = self.meter.parts();
        let routed = reservation.routed.lock().ok().and_then(|cell| cell.clone());
        let cost = match (routed, input.or(output)) {
            (Some((platform, model)), Some(_)) => crate::pricing::estimate_usage(
                Some(&platform),
                Some(&model),
                crate::pricing::Usage { input: input.unwrap_or(0), output: output.unwrap_or(0), ..Default::default() },
            ),
            _ => None,
        };
        let guard = db.lock();
        if let Ok(conn) = guard {
            settle_with_cost(&conn, &reservation, actual, cost);
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

    fn at(text: &str) -> chrono::DateTime<chrono::Local> {
        chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap().and_local_timezone(chrono::Local).earliest().unwrap()
    }

    #[test]
    fn windows_start_on_monday_and_the_first_and_reset_at_the_next_one() {
        let wednesday = at("2026-11-11 15:00");
        assert_eq!(Window::Day.bounds(wednesday).0, "2026-11-11");
        assert_eq!(Window::Week.bounds(wednesday).0, "2026-11-09", "周一起算");
        assert_eq!(Window::Week.bounds(wednesday).1.format("%Y-%m-%d %H:%M").to_string(), "2026-11-16 00:00");
        assert_eq!(Window::Month.bounds(wednesday).0, "2026-11-01");
        assert_eq!(Window::Month.bounds(at("2026-12-31 23:00")).1.format("%Y-%m-%d").to_string(), "2027-01-01", "跨年");
    }

    #[test]
    fn weekly_tokens_and_daily_cost_limits_refuse_with_their_own_reset() {
        let conn = conn();
        conn.execute("INSERT INTO gateway_client_keys VALUES('wk', NULL)", []).unwrap();
        crate::gateway::access::client_store_init(&conn).unwrap();
        conn.execute("UPDATE gateway_client_keys SET tokens_per_week = 1000, cost_per_day = 0.5 WHERE name = 'wk'", []).unwrap();
        let now = at("2026-11-11 15:00");
        // 周一已经花了 900：本周再来一个约 4096 的请求超过周上限
        conn.execute("INSERT INTO gateway_key_tokens(name, day, spent, cost_micros) VALUES('wk', '2026-11-09', 900, 0)", []).unwrap();
        let refused = reserve(&conn, "wk", b"{}", now, Default::default()).unwrap_err();
        assert!(refused.message.contains("本周") && refused.message.contains("2026-11-16"), "{}", refused.message);
        // 换一把只限费用的：今天已花 $0.6 ≥ $0.5
        conn.execute("UPDATE gateway_client_keys SET tokens_per_week = NULL WHERE name = 'wk'", []).unwrap();
        conn.execute("INSERT INTO gateway_key_tokens(name, day, spent, cost_micros) VALUES('wk', '2026-11-11', 10, 600000)", []).unwrap();
        let refused = reserve(&conn, "wk", b"{}", now, Default::default()).unwrap_err();
        assert!(refused.message.contains("今日费用"), "{}", refused.message);
    }

    #[test]
    fn settle_records_cost_when_the_routed_model_is_priced() {
        let conn = conn();
        conn.execute("INSERT INTO gateway_client_keys VALUES('priced', NULL)", []).unwrap();
        let reservation = reserve(&conn, "priced", b"{}", now(), Default::default()).unwrap().unwrap();
        settle_with_cost(&conn, &reservation, Some(100), Some(0.0123));
        let (_, cost) = spent_in(&conn, "priced", Window::Day, now()).unwrap();
        assert!((cost - 0.0123).abs() < 1e-9);
        let reservation = reserve(&conn, "priced", b"{}", now(), Default::default()).unwrap().unwrap();
        settle(&conn, &reservation, Some(100));
        assert!((spent_in(&conn, "priced", Window::Day, now()).unwrap().1 - 0.0123).abs() < 1e-9, "没价目的不加费用");
    }

    #[test]
    fn keys_without_limits_are_recorded_for_billing_but_never_refused() {
        let conn = conn();
        conn.execute("INSERT INTO gateway_client_keys VALUES('free', NULL)", []).unwrap();
        for _ in 0..3 {
            let reservation = reserve(&conn, "free", &vec![b'x'; 400_000], now(), Default::default()).unwrap().expect("命名 Key 一律记账");
            settle(&conn, &reservation, Some(1_000_000));
        }
        assert_eq!(spent(&conn, "free", &day_of(now())).unwrap(), 3_000_000, "不限额照样记，按 Key 出账");
        assert!(reserve(&conn, "ghost", b"{}", now(), Default::default()).unwrap().is_none(), "不存在的 Key 不记账");
    }

    #[test]
    fn inflight_reservations_count_against_the_limit_until_settled() {
        let conn = conn();
        conn.execute("INSERT INTO gateway_client_keys VALUES('k-inflight', 10000)", []).unwrap();
        let body = br#"{"max_tokens":4000}"#;
        let first = reserve(&conn, "k-inflight", body, now(), Default::default()).unwrap().unwrap();
        let second = reserve(&conn, "k-inflight", body, now(), Default::default()).unwrap().unwrap();
        // 两个在途共约 8000，第三个会越过 10000：并发请求不能一起越过上限
        let refused = reserve(&conn, "k-inflight", body, now(), Default::default()).unwrap_err();
        assert_eq!(refused.status, 429);
        assert!(refused.retry_after().is_some_and(|s| s >= 1));
        // 回冲后按实际用量计：第一条只花了 100，额度回到可用
        settle(&conn, &first, Some(100));
        settle(&conn, &second, Some(200));
        assert_eq!(spent(&conn, "k-inflight", &first.day).unwrap(), 300);
        assert!(reserve(&conn, "k-inflight", body, now(), Default::default()).unwrap().is_some());
    }

    #[test]
    fn unknown_usage_is_charged_at_the_estimate() {
        let conn = conn();
        conn.execute("INSERT INTO gateway_client_keys VALUES('k-unknown', 100000)", []).unwrap();
        let reservation = reserve(&conn, "k-unknown", br#"{"max_tokens":500}"#, now(), Default::default()).unwrap().unwrap();
        settle(&conn, &reservation, None);
        assert_eq!(spent(&conn, "k-unknown", &reservation.day).unwrap(), reservation.estimate);
    }

    #[tokio::test]
    async fn metered_response_settles_actual_usage_when_the_body_is_consumed() {
        let db = std::sync::Arc::new(Mutex::new(conn()));
        db.lock().unwrap().execute("INSERT INTO gateway_client_keys VALUES('k-stream', 100000)", []).unwrap();
        let reservation = reserve(&db.lock().unwrap(), "k-stream", br#"{"max_tokens":9000}"#, now(), Default::default()).unwrap().unwrap();
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
        let reservation = reserve(&db.lock().unwrap(), "k-failed", b"{}", now(), Default::default()).unwrap().unwrap();
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
