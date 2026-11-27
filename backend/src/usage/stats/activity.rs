use std::collections::{BTreeMap, HashSet};
use chrono::{Datelike, Duration, NaiveDate};
use rusqlite::Connection;
use serde::Serialize;
use super::*;

#[derive(Debug, Serialize)]
pub struct ActivityDay {
    /// 本地日期 YYYY-MM-DD
    pub date: String,
    pub tokens: i64,
    pub requests: i64,
    pub cost: f64,
    pub unpriced_requests: i64,
}

#[derive(Debug, Serialize)]
pub struct Activity {
    /// 日历起点（含）：一年前那一周的周一
    pub first_date: String,
    pub last_date: String,
    /// 只列出有记录的日期
    pub days: Vec<ActivityDay>,
    /// 今日活跃时长估算（分钟）：相邻请求间隔不超过 5 分钟的时段之和
    pub today_active_minutes: i64,
    pub week_active_days: i64,
    pub month_active_days: i64,
}

/// 全年活跃日历。只按供应商和账号筛选，日期固定为结束日往前一年；同时按记忆化计价引擎算出每日估算费用（R1-48）。
pub fn activity(conn: &Connection, platform: Option<String>, connections: Vec<String>, now: i64) -> rusqlite::Result<Activity> {
    let today = local_date(now);
    let year_ago = today - Duration::days(364);
    let first = year_ago - Duration::days(i64::from(year_ago.weekday().num_days_from_monday()));
    let f = StatsFilter { start: local_ms(first), end: now.max(local_ms(first) + 1), platform, connections, ..Default::default() };
    let (clause, args) = where_clause(&f, false);
    let snapshot = crate::attribution::Snapshot::load(conn).ok();
    let mut resolver = crate::pricing::PriceResolver::load(conn);
    let mut stmt = conn.prepare(&format!(
        "SELECT r.ts, r.total_tokens, r.total_known, r.platform, r.model, r.upstream_model, r.served_model,
                r.input_tokens, r.output_tokens, r.cache_read_tokens, r.cache_write_tokens, r.cache_write_1h_tokens,
                r.input_known, r.output_known, r.cache_read_known, r.cache_write_known,
                r.input_semantics, r.connection_id, r.provider_key, m.reasoning, m.tool
         FROM requests r LEFT JOIN request_metrics m ON m.request_id = r.id
         WHERE {clause}"
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
        let opt = |v: usize, k: usize| -> rusqlite::Result<Option<i64>> {
            Ok(if r.get::<_, i64>(k)? != 0 { Some(r.get(v)?) } else { None })
        };
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)? != 0,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            opt(7, 12)?,
            opt(8, 13)?,
            opt(9, 14)?,
            opt(10, 15)?,
            r.get::<_, Option<i64>>(11)?.unwrap_or(0),
            r.get::<_, String>(16)?,
            r.get::<_, Option<String>>(17)?,
            r.get::<_, Option<String>>(18)?,
            r.get::<_, Option<i64>>(19)?.unwrap_or(0),
            r.get::<_, Option<i64>>(20)?.unwrap_or(0),
        ))
    })?;
    let mut days: BTreeMap<NaiveDate, (i64, i64, f64, i64)> = BTreeMap::new();
    let mut today_ts = Vec::new();
    for row in rows {
        let (ts, total, known, plat, model, upstream, served, input, output, cr, cw, cw1h, semantics, conn_id, provider_key, reasoning, tool) = row?;
        let date = local_date(ts);
        let entry = days.entry(date).or_default();
        if known {
            entry.0 += total;
        }
        entry.1 += 1;
        let attribution = snapshot
            .as_ref()
            .map_or_else(crate::attribution::AttributionView::platform, |snap| {
                snap.resolve(&plat, conn_id.as_deref(), provider_key.as_deref(), model.as_deref())
            });
        let effective_conn = attribution.is_connection().then(|| attribution.connection_id.as_deref()).flatten();
        let at = chrono::DateTime::from_timestamp_millis(ts).unwrap_or_else(chrono::Utc::now);
        let res = resolver.price_request(&crate::pricing::PriceRequest {
            platform: &plat,
            model: model.as_deref(),
            upstream_model: upstream.as_deref(),
            served_model: served.as_deref(),
            connection_id: effective_conn,
            input_semantics: &semantics,
            input,
            output,
            cache_read: cr,
            cache_write: cw,
            cache_write_1h: cw1h,
            reasoning,
            tool,
            at,
        });
        match res.cost {
            Some(c) => entry.2 += c,
            None => entry.3 += 1,
        }
        if date == today {
            today_ts.push(ts);
        }
    }
    today_ts.sort_unstable();
    let active_ms: i64 = today_ts.windows(2).map(|w| w[1] - w[0]).filter(|gap| *gap <= ACTIVE_GAP_MS).sum();
    let week_start = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let month_start = today.with_day(1).unwrap_or(today);
    let active: HashSet<NaiveDate> = days.keys().copied().collect();
    Ok(Activity {
        first_date: first.format("%Y-%m-%d").to_string(),
        last_date: today.format("%Y-%m-%d").to_string(),
        week_active_days: active.iter().filter(|d| **d >= week_start && **d <= today).count() as i64,
        month_active_days: active.iter().filter(|d| **d >= month_start && **d <= today).count() as i64,
        days: days
            .into_iter()
            .map(|(date, (tokens, requests, cost, unpriced_requests))| ActivityDay {
                date: date.format("%Y-%m-%d").to_string(),
                tokens,
                requests,
                cost,
                unpriced_requests,
            })
            .collect(),
        today_active_minutes: active_ms / 60_000,
    })
}
