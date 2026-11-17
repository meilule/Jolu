//! 路由流水：每次网关请求一行，给「统一网关」页展示最近去向。
//!
//! 只记元数据：时间、agent、请求模型、路由到的来源与连接、已验证的调用方会话、状态码、等待上游响应的耗时。
//! 不记提示词、不记凭证。流式响应的耗时是「到响应头」的时间，不是首字时间，界面按「耗时」展示。
//!
//! 只保留最近约 1000 行（分批清理，可能暂时超过），分页统计只代表当前保留范围。

use rusqlite::{params, Connection};

/// 保留行数；清理每 50 次写执行一次。
const KEEP_ROWS: i64 = 1000;

#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteLogEntry {
    /// 自增流水 id：分页的稳定行标识与唯一次级排序键
    pub id: i64,
    pub ts: i64,
    pub agent: Option<String>,
    pub requested_model: String,
    pub source: Option<String>,
    pub upstream_model: Option<String>,
    pub status_code: Option<i64>,
    /// ok / rejected / failed
    pub result: String,
    pub duration_ms: Option<i64>,
    pub connection_id: Option<String>,
    pub session_platform: Option<String>,
    pub session_id: Option<String>,
    /// 命中的命名客户端 Key 名（局域网共享，P8.3）；旧行与本机工具的请求为 None
    pub client_key: Option<String>,
    /// 同一次网关请求的编号：分组成员失败转移时，各次尝试的行共享它（进程内递增，重启后从头计）
    pub request_id: Option<i64>,
    /// 模型 id 怎么解析到来源的：`bare` = 请求用的是不带来源前缀的裸名（目录或默认来源兜底）
    pub resolved_from: Option<String>,
    /// 请求发生时生效的 Profile（应用 Profile 后、手动改接线前）
    pub profile: Option<String>,
}

pub struct NewEntry<'a> {
    pub agent: Option<&'a str>,
    pub requested_model: &'a str,
    pub source: Option<&'a str>,
    pub upstream_model: Option<&'a str>,
    pub status_code: Option<i64>,
    pub result: &'a str,
    pub duration_ms: Option<i64>,
    pub connection_id: Option<&'a str>,
    pub session_platform: Option<&'a str>,
    pub session_id: Option<&'a str>,
    /// 命中的命名客户端 Key 名（局域网共享时由鉴权中间件识别）；None 表示本机调用方
    pub client_key: Option<&'a str>,
    /// 同一次网关请求的编号（见 [`RouteLogEntry::request_id`]）
    pub request_id: Option<i64>,
    /// 见 [`RouteLogEntry::resolved_from`]
    pub resolved_from: Option<&'a str>,
    /// 见 [`RouteLogEntry::profile`]
    pub profile: Option<&'a str>,
}

pub fn record(conn: &Connection, entry: NewEntry<'_>) {
    let _ = conn.execute(
        "INSERT INTO gateway_route_log(ts, agent, requested_model, source, upstream_model, status_code, result, duration_ms,
                                       connection_id, session_platform, session_id, client_key, request_id, resolved_from, profile)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            chrono::Local::now().timestamp_millis(),
            entry.agent,
            entry.requested_model,
            entry.source,
            entry.upstream_model,
            entry.status_code,
            entry.result,
            entry.duration_ms,
            entry.connection_id,
            entry.session_platform,
            entry.session_id,
            entry.client_key,
            entry.request_id,
            entry.resolved_from,
            entry.profile,
        ],
    );
    // 只保留最近 1000 行；每 50 次写清理一次，避免每条请求都做一次 NOT IN 子查询。
    use std::sync::atomic::{AtomicU32, Ordering};
    static WRITES: AtomicU32 = AtomicU32::new(0);
    if WRITES.fetch_add(1, Ordering::Relaxed) % 50 == 49 {
        let _ = conn.execute(
            "DELETE FROM gateway_route_log WHERE id NOT IN (SELECT id FROM gateway_route_log ORDER BY id DESC LIMIT ?1)",
            [KEEP_ROWS],
        );
        // 上下文快照与流水在同一处清理（执行文档 §6）
        crate::gateway::context::store::prune(conn);
    }
}

const COLUMNS: &str = "id, ts, agent, requested_model, source, upstream_model, status_code, result, duration_ms,
                       connection_id, session_platform, session_id, client_key, request_id, resolved_from, profile";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RouteLogEntry> {
    Ok(RouteLogEntry {
        id: r.get(0)?,
        ts: r.get(1)?,
        agent: r.get(2)?,
        requested_model: r.get(3)?,
        source: r.get(4)?,
        upstream_model: r.get(5)?,
        status_code: r.get(6)?,
        result: r.get(7)?,
        duration_ms: r.get(8)?,
        connection_id: r.get(9)?,
        session_platform: r.get(10)?,
        session_id: r.get(11)?,
        client_key: r.get(12)?,
        request_id: r.get(13)?,
        resolved_from: r.get(14)?,
        profile: r.get(15)?,
    })
}

pub fn recent(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<RouteLogEntry>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM gateway_route_log ORDER BY id DESC LIMIT ?1"))?;
    let rows = stmt.query_map([limit as i64], row)?;
    rows.collect()
}

/// 一页流水与这次浏览的查询范围。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteLogPage {
    pub rows: Vec<RouteLogEntry>,
    /// 实际返回的页码（请求页超出范围时收到最后一页；没有数据时为 1）
    pub page: i64,
    pub page_size: i64,
    /// 查询上界内、当前保留范围内的总行数
    pub total: i64,
    /// 没有数据时为 0
    pub page_count: i64,
    /// 本次浏览的查询上界（含）：翻页时带回，新写入的流水不会挤动旧页；没有数据时为 None
    pub upper_id: Option<i64>,
}

/// 分页：按 id 从大到小。`upper_id` 为 None 时以当前最大 id 建立新的查询范围。
/// 计数与列表都限定在同一上界内；保留范围清理掉的旧行会让总数变小，页码随之收拢到最后一页。
pub fn page(conn: &Connection, page: i64, page_size: i64, upper_id: Option<i64>) -> rusqlite::Result<RouteLogPage> {
    let page_size = page_size.clamp(1, 100);
    let upper = match upper_id {
        Some(id) => Some(id),
        None => conn.query_row("SELECT MAX(id) FROM gateway_route_log", [], |r| r.get::<_, Option<i64>>(0))?,
    };
    let Some(upper) = upper else {
        return Ok(RouteLogPage { rows: Vec::new(), page: 1, page_size, total: 0, page_count: 0, upper_id: None });
    };
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM gateway_route_log WHERE id <= ?1", [upper], |r| r.get(0))?;
    let page_count = (total + page_size - 1) / page_size;
    let page = page.clamp(1, page_count.max(1));
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM gateway_route_log WHERE id <= ?1 ORDER BY id DESC LIMIT ?2 OFFSET ?3"
    ))?;
    let rows = stmt
        .query_map(params![upper, page_size, (page - 1) * page_size], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(RouteLogPage { rows, page, page_size, total, page_count, upper_id: Some(upper) })
}

/// 某个会话经网关到某个连接的历史关联（保留范围内）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionLink {
    pub session_platform: String,
    pub session_id: String,
    /// 被拒或旧流水没有连接 id 时为 None
    pub connection_id: Option<String>,
    pub source: Option<String>,
    /// 最近一次的请求模型与上游模型
    pub requested_model: String,
    pub upstream_model: Option<String>,
    pub agent: Option<String>,
    pub last_ts: i64,
    pub requests: i64,
}

/// 按（会话, 连接）聚合流水里的会话关联，最新的在前。只统计带已验证会话标识的行。
/// 模型、来源与 Agent 取每组 id 最大（最近一次）的那一行。
pub fn session_links(conn: &Connection) -> rusqlite::Result<Vec<SessionLink>> {
    let mut stmt = conn.prepare(
        "SELECT l.session_platform, l.session_id, l.connection_id, l.source, l.requested_model, l.upstream_model, l.agent,
                g.last_id, g.last_ts, g.requests
         FROM (SELECT MAX(id) AS last_id, MAX(ts) AS last_ts, COUNT(*) AS requests
               FROM gateway_route_log
               WHERE session_platform IS NOT NULL AND session_id IS NOT NULL AND session_id <> ''
               GROUP BY session_platform, session_id, connection_id) g
         JOIN gateway_route_log l ON l.id = g.last_id
         ORDER BY g.last_id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SessionLink {
            session_platform: r.get(0)?,
            session_id: r.get(1)?,
            connection_id: r.get(2)?,
            source: r.get(3)?,
            requested_model: r.get(4)?,
            upstream_model: r.get(5)?,
            agent: r.get(6)?,
            last_ts: r.get(8)?,
            requests: r.get(9)?,
        })
    })?;
    rows.collect()
}

/// 某个来源（连接）经网关的使用情况（保留范围内）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceUsage {
    pub connection_id: String,
    pub last_ts: i64,
    pub requests: i64,
    /// 其中失败的次数（被拒的请求没有连接 id，不计入）
    pub failed: i64,
}

/// 按连接汇总保留范围内的流水；旧流水没有连接 id 的不计入。
pub fn source_usage(conn: &Connection) -> rusqlite::Result<Vec<SourceUsage>> {
    let mut stmt = conn.prepare(
        "SELECT connection_id, MAX(ts), COUNT(*), COALESCE(SUM(result NOT IN ('ok', 'rejected')), 0)
         FROM gateway_route_log WHERE connection_id IS NOT NULL
         GROUP BY connection_id ORDER BY MAX(ts) DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SourceUsage { connection_id: r.get(0)?, last_ts: r.get(1)?, requests: r.get(2)?, failed: r.get(3)? })
    })?;
    rows.collect()
}

/// 模型路由概览（画板 V5-3「四项指标」、V10-01 范围下拉、Agent 页「今日路由 N」）。
///
/// 一次网关请求在分组改道时会写多行（共享 `request_id`），这里按请求而不是按行计：
/// - `today` / `yesterday`：请求数；
/// - `reroutes`：本期写了不止一行的请求（失败后换下一个成员）；
/// - `errors`：本期最后一次尝试仍不是 ok 的请求（Agent 真正收到的错误）；
/// - `by_agent`：本期各 agent 的请求数（agent 未识别的不计）。
///
/// **字段名沿用 `today` / `yesterday`，含义是「本期 / 上期」**（概览页的范围下拉）：
/// 今天 → 本期今天 / 上期昨天；昨天 → 本期昨天 / 上期前天；近 7 天 → 本期 7 天 / 上期前 7 天；
/// 近 30 天同理。前端文案按范围换称呼即可，计算不分叉。
///
/// 流水只保留最近 1000 行：最早一行晚于上期起点时，`partial = true`，两期数字都是下限。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DayOverview {
    pub today: i64,
    pub yesterday: i64,
    pub reroutes: i64,
    pub errors: i64,
    pub by_agent: std::collections::BTreeMap<String, i64>,
    pub partial: bool,
    /// 本期最近一次 Agent 收到的错误（最后一次尝试那行）
    pub last_error: Option<RouteLogEntry>,
}

/// 概览页的时间范围（V10-01 右上「今天」下拉）。
/// 每个范围给出「本期起点 + 上期起点」两个时间戳：上期用于「比上期」的对比与 `partial` 判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Today,
    Yesterday,
    Last7Days,
    Last30Days,
}

impl Range {
    /// 从命令参数解析；未知值退回「今天」（不让前端一个错字把页面变空）。
    pub fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or("today").trim().to_ascii_lowercase().as_str() {
            "yesterday" => Self::Yesterday,
            "7d" | "last7" | "last7d" => Self::Last7Days,
            "30d" | "last30" | "last30d" => Self::Last30Days,
            _ => Self::Today,
        }
    }

    /// 覆盖的天数（本期）。用于算上期起点。
    fn days(self) -> i64 {
        match self {
            Self::Today | Self::Yesterday => 1,
            Self::Last7Days => 7,
            Self::Last30Days => 30,
        }
    }

    /// 本期是否截止到今天；`false` 表示本期是「昨天 / 昨天往前 N 天」这类已结束的区间。
    fn ends_today(self) -> bool {
        matches!(self, Self::Today | Self::Last7Days | Self::Last30Days)
    }

    /// 本期起点与上期起点（本地时间自然日边界，毫秒）。
    /// `today` 由调用方传入，便于测试固定日期。
    pub fn bounds(self, today: chrono::NaiveDate) -> (i64, i64) {
        let days = self.days();
        // 本期覆盖的最后一个自然日
        let end_day = if self.ends_today() { today } else { today.pred_opt().unwrap_or(today) };
        let current_start = end_day - chrono::Duration::days(days - 1);
        let prior_start = current_start - chrono::Duration::days(days);
        (midnight_ms(current_start), midnight_ms(prior_start))
    }
}

/// 本地时区某天 00:00 的毫秒时间戳；时区异常时退回 0（与既有实现一致）。
fn midnight_ms(day: chrono::NaiveDate) -> i64 {
    day.and_hms_opt(0, 0, 0)
        .and_then(|midnight| midnight.and_local_timezone(chrono::Local).earliest())
        .map(|midnight| midnight.timestamp_millis())
        .unwrap_or(0)
}

pub fn day_overview(conn: &Connection, today_start: i64, yesterday_start: i64) -> rusqlite::Result<DayOverview> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM gateway_route_log WHERE ts >= ?1 ORDER BY id ASC"))?;
    let rows = stmt.query_map([yesterday_start], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    let oldest: Option<i64> = conn.query_row("SELECT MIN(ts) FROM gateway_route_log", [], |r| r.get(0))?;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM gateway_route_log", [], |r| r.get(0))?;
    // 按请求分组。request_id 是进程内计数、重启后从头开始：编号明显倒退时视为新一轮，避免把两次运行的请求并在一起
    let mut groups: Vec<(i64, Vec<RouteLogEntry>)> = Vec::new();
    let mut index: std::collections::HashMap<(u32, i64), usize> = std::collections::HashMap::new();
    let (mut epoch, mut max_seen) = (0u32, 0i64);
    for entry in rows {
        let key = match entry.request_id {
            Some(id) => {
                if id + 100 < max_seen {
                    epoch += 1;
                    max_seen = 0;
                }
                max_seen = max_seen.max(id);
                (epoch, id)
            }
            None => (u32::MAX, -entry.id),
        };
        match index.get(&key) {
            Some(&at) => groups[at].1.push(entry),
            None => {
                index.insert(key, groups.len());
                groups.push((entry.ts, vec![entry]));
            }
        }
    }
    let mut out = DayOverview {
        partial: total >= 1000 && oldest.is_some_and(|ts| ts > yesterday_start),
        ..Default::default()
    };
    for (first_ts, attempts) in groups {
        if first_ts < today_start {
            out.yesterday += 1;
            continue;
        }
        out.today += 1;
        if attempts.len() > 1 {
            out.reroutes += 1;
        }
        if let Some(agent) = attempts[0].agent.clone().filter(|a| !a.is_empty()) {
            *out.by_agent.entry(agent).or_default() += 1;
        }
        let last = attempts.last().cloned();
        if let Some(last) = last.filter(|l| l.result != "ok") {
            out.errors += 1;
            out.last_error = Some(last);
        }
    }
    Ok(out)
}

/// 某时刻之后的请求数，按结果分组：(ok, rejected, failed)。供灵动岛「今日经网关」使用。
/// 流水只保留最近 1000 行，当天超过 1000 次时这是下限，调用方据此标注。
pub fn counts_since(conn: &Connection, since_ms: i64) -> rusqlite::Result<(i64, i64, i64)> {
    conn.query_row(
        "SELECT COALESCE(SUM(result = 'ok'), 0), COALESCE(SUM(result = 'rejected'), 0),
                COALESCE(SUM(result NOT IN ('ok', 'rejected')), 0)
         FROM gateway_route_log WHERE ts >= ?1",
        [since_ms],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(result: &str) -> NewEntry<'_> {
        NewEntry {
            agent: None, requested_model: "a/b", source: None, upstream_model: None, status_code: None, result,
            duration_ms: None, connection_id: None, session_platform: None, session_id: None, client_key: None, request_id: None, resolved_from: None, profile: None }
    }

    fn db() -> Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    /// 范围下拉的各档时间窗：本期起点与上期起点必须连续、等长，且落在自然日零点。
    #[test]
    fn range_bounds_are_contiguous_equal_length_windows() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
        let day = 86_400_000i64;
        for range in [Range::Today, Range::Yesterday, Range::Last7Days, Range::Last30Days] {
            let (current, prior) = range.bounds(today);
            let span = current - prior;
            assert_eq!(span, range.days() * day, "{range:?} 本期与上期长度不等");
            assert!(prior < current, "{range:?} 上期应在本期之前");
        }
        // 具体边界：今天 → 本期今天 00:00，上期昨天 00:00
        let (cur, prior) = Range::Today.bounds(today);
        assert_eq!(cur, midnight_ms(today));
        assert_eq!(prior, midnight_ms(today.pred_opt().unwrap()));
        // 昨天 → 本期昨天 00:00，上期前天 00:00
        let (cur, prior) = Range::Yesterday.bounds(today);
        assert_eq!(cur, midnight_ms(today.pred_opt().unwrap()));
        assert_eq!(prior, midnight_ms(today - chrono::Duration::days(2)));
        // 近 7 天 → 含今天共 7 天，上期为之前 7 天
        let (cur, prior) = Range::Last7Days.bounds(today);
        assert_eq!(cur, midnight_ms(today - chrono::Duration::days(6)));
        assert_eq!(prior, midnight_ms(today - chrono::Duration::days(13)));
        // 近 30 天同理
        let (cur, prior) = Range::Last30Days.bounds(today);
        assert_eq!(cur, midnight_ms(today - chrono::Duration::days(29)));
        assert_eq!(prior, midnight_ms(today - chrono::Duration::days(59)));
    }

    #[test]
    fn range_parse_defaults_to_today_for_unknown_values() {
        assert_eq!(Range::parse(None), Range::Today);
        assert_eq!(Range::parse(Some("")), Range::Today);
        assert_eq!(Range::parse(Some("today")), Range::Today);
        assert_eq!(Range::parse(Some("YESTERDAY")), Range::Yesterday);
        assert_eq!(Range::parse(Some("7d")), Range::Last7Days);
        assert_eq!(Range::parse(Some("30d")), Range::Last30Days);
        // 拼错不能把页面变空：退回今天
        assert_eq!(Range::parse(Some("7days")), Range::Today);
        assert_eq!(Range::parse(Some("week")), Range::Today);
    }

    #[test]
    fn day_overview_counts_requests_not_rows() {
        let conn = db();
        let insert = |ts: i64, agent: &str, result: &str, request_id: Option<i64>| {
            conn.execute(
                "INSERT INTO gateway_route_log(ts, agent, requested_model, result, request_id) VALUES(?1, ?2, 'group/x', ?3, ?4)",
                params![ts, agent, result, request_id],
            )
            .unwrap();
        };
        // 昨天 1 次；今天：一次改道后成功、一次两次尝试都失败、一次直接成功（无 request_id）
        insert(50, "claude-code", "ok", Some(1));
        insert(150, "claude-code", "failed", Some(2));
        insert(151, "claude-code", "ok", Some(2));
        insert(160, "codex", "failed", Some(3));
        insert(161, "codex", "failed", Some(3));
        insert(170, "claude-code", "ok", None);
        let day = day_overview(&conn, 100, 0).unwrap();
        assert_eq!((day.today, day.yesterday, day.reroutes, day.errors), (3, 1, 2, 1));
        assert_eq!(day.by_agent.get("claude-code"), Some(&2));
        assert_eq!(day.by_agent.get("codex"), Some(&1));
        assert_eq!(day.last_error.as_ref().map(|e| e.ts), Some(161), "取最后一次尝试那行");
        assert!(!day.partial);
        // 重启后 request_id 从头计：不能并进上一轮的同号请求
        insert(300, "codex", "ok", Some(500));
        insert(301, "codex", "ok", Some(2));
        assert_eq!(day_overview(&conn, 100, 0).unwrap().today, 5);
    }

    #[test]
    fn recent_is_newest_first_and_keeps_metadata() {
        let conn = db();
        record(&conn, NewEntry { agent: Some("codex"), requested_model: "kimi/kimi-k2", source: Some("kimi"), upstream_model: Some("kimi-k2"), status_code: Some(200), result: "ok", duration_ms: Some(42), connection_id: Some("c1"), session_platform: Some("codex"), session_id: Some("s1"), client_key: None, request_id: None, resolved_from: None, profile: None });
        record(&conn, NewEntry { agent: None, requested_model: "nope/m", source: None, upstream_model: None, status_code: Some(404), result: "rejected", duration_ms: None, connection_id: None, session_platform: None, session_id: None, client_key: None, request_id: None, resolved_from: None, profile: None });
        let rows = recent(&conn, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].result, "rejected");
        assert!(rows[0].id > rows[1].id);
        assert_eq!(rows[1].agent.as_deref(), Some("codex"));
        assert_eq!(rows[1].duration_ms, Some(42));
        assert_eq!(rows[1].session_id.as_deref(), Some("s1"));
        assert_eq!(rows[1].connection_id.as_deref(), Some("c1"));
    }

    #[test]
    fn counts_since_groups_by_result() {
        let conn = db();
        for result in ["ok", "ok", "rejected", "failed"] {
            record(&conn, entry(result));
        }
        assert_eq!(counts_since(&conn, 0).unwrap(), (2, 1, 1));
        assert_eq!(counts_since(&conn, i64::MAX).unwrap(), (0, 0, 0));
    }

    /// 局域网共享（P8.3）：命中的命名客户端 Key 随流水落库，供限额按 Key 名统计。
    #[test]
    fn client_key_is_recorded_and_readable_back() {
        let conn = db();
        record(&conn, NewEntry { client_key: Some("lab-agent"), ..entry("ok") });
        record(&conn, NewEntry { client_key: None, ..entry("ok") });
        let rows = recent(&conn, 10).unwrap();
        assert_eq!(rows[0].client_key, None, "本机请求不带 Key 名");
        assert_eq!(rows[1].client_key.as_deref(), Some("lab-agent"));
        let named = conn
            .query_row(
                "SELECT COUNT(*) FROM gateway_route_log WHERE client_key = 'lab-agent' AND result = 'ok'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(named, 1);
    }

    #[test]
    fn page_handles_empty_full_and_partial_pages() {
        let conn = db();
        let empty = page(&conn, 1, 10, None).unwrap();
        assert_eq!((empty.total, empty.page_count, empty.page, empty.rows.len()), (0, 0, 1, 0));
        assert_eq!(empty.upper_id, None);
        for count in [1usize, 9, 1] {
            for _ in 0..count {
                record(&conn, entry("ok"));
            }
            let total = conn.query_row("SELECT COUNT(*) FROM gateway_route_log", [], |r| r.get::<_, i64>(0)).unwrap();
            let first = page(&conn, 1, 10, None).unwrap();
            assert_eq!(first.total, total);
            assert_eq!(first.page_count, (total + 9) / 10);
            assert_eq!(first.rows.len() as i64, total.min(10));
        }
        // 11 条：第 2 页只有 1 条，超出范围的页收拢到最后一页
        let last = page(&conn, 2, 10, None).unwrap();
        assert_eq!((last.page, last.rows.len()), (2, 1));
        let over = page(&conn, 99, 10, None).unwrap();
        assert_eq!(over.page, 2);
    }

    #[test]
    fn upper_bound_keeps_pages_stable_while_new_rows_arrive() {
        let conn = db();
        for _ in 0..25 {
            record(&conn, entry("ok"));
        }
        let first = page(&conn, 1, 10, None).unwrap();
        let upper = first.upper_id;
        for _ in 0..7 {
            record(&conn, entry("failed"));
        }
        let second = page(&conn, 2, 10, upper).unwrap();
        let third = page(&conn, 3, 10, upper).unwrap();
        assert_eq!(second.total, 25, "新写入不进入本次浏览范围");
        let mut ids: Vec<i64> = first.rows.iter().chain(&second.rows).chain(&third.rows).map(|r| r.id).collect();
        assert_eq!(ids.len(), 25);
        ids.dedup();
        assert_eq!(ids.len(), 25, "连续翻页没有重复");
        assert!(ids.windows(2).all(|w| w[0] > w[1]), "按 id 从大到小且无遗漏");
        let refreshed = page(&conn, 1, 10, None).unwrap();
        assert_eq!(refreshed.total, 32, "显式刷新重新建立范围");
    }

    #[test]
    fn source_usage_counts_per_connection_and_skips_rows_without_one() {
        let conn = db();
        let with = |conn_id: &'static str, result: &'static str| NewEntry { connection_id: Some(conn_id), ..entry(result) };
        record(&conn, with("c1", "ok"));
        record(&conn, with("c1", "failed"));
        record(&conn, with("c2", "ok"));
        record(&conn, entry("rejected"));
        let usage = source_usage(&conn).unwrap();
        assert_eq!(usage.len(), 2);
        let c1 = usage.iter().find(|u| u.connection_id == "c1").unwrap();
        assert_eq!((c1.requests, c1.failed), (2, 1));
    }

    #[test]
    fn session_links_group_by_session_and_connection_with_latest_model() {
        let conn = db();
        let link = |model: &'static str, conn_id: &'static str, session: &'static str| NewEntry {
            agent: Some("codex"), requested_model: model, source: Some("kimi"), upstream_model: None, status_code: Some(200),
            result: "ok", duration_ms: None, connection_id: Some(conn_id), session_platform: Some("codex"), session_id: Some(session),
            client_key: None, request_id: None, resolved_from: None, profile: None };
        record(&conn, link("kimi/a", "c1", "s1"));
        record(&conn, link("kimi/b", "c1", "s1"));
        record(&conn, link("kimi/a", "c2", "s1"));
        record(&conn, link("kimi/a", "c1", "s2"));
        record(&conn, entry("ok"));
        let links = session_links(&conn).unwrap();
        assert_eq!(links.len(), 3, "没有会话标识的流水不参与关联");
        assert_eq!((links[0].session_id.as_str(), links[0].connection_id.as_deref()), ("s2", Some("c1")));
        let s1c1 = links.iter().find(|l| l.session_id == "s1" && l.connection_id.as_deref() == Some("c1")).unwrap();
        assert_eq!((s1c1.requests, s1c1.requested_model.as_str()), (2, "kimi/b"));
    }
}
