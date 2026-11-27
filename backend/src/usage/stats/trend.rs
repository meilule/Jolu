use chrono::{Duration, Local, NaiveDate, TimeZone, Timelike};
use serde::Serialize;
use super::*;

#[derive(Debug, Serialize)]
pub struct TrendBucket {
    pub start: i64,
    pub end: i64,
    pub label: String,
    pub fresh_input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub total: Option<i64>,
    pub requests: i64,
    /// 分桶内未提供完整 Token 分项或总量的请求数；大于 0 时分项按已知小计汇总，不抹零整根柱子。
    pub unknown_requests: i64,
    /// 分桶内可计算请求的输入缓存命中率（0.0..=1.0）；无可计算请求时为 None。
    pub cache_hit_rate: Option<f64>,
    /// 已计价部分的估算费用
    pub cost: f64,
}

#[derive(Debug, Serialize)]
pub struct Trend {
    /// "1h" | "2h" | "day" | "week"，界面据此写「每小时 / 每 2 小时 / 每天 / 每周」
    pub bucket: &'static str,
    pub points: Vec<TrendBucket>,
}


pub fn local_ms(date: NaiveDate) -> i64 {
    let naive = date.and_hms_opt(0, 0, 0).expect("零点总是有效");
    Local.from_local_datetime(&naive).earliest().map(|d| d.timestamp_millis()).unwrap_or_else(|| naive.and_utc().timestamp_millis())
}

pub fn local_date(ms: i64) -> NaiveDate {
    Local.timestamp_millis_opt(ms).single().map(|d| d.date_naive()).unwrap_or_default()
}

/// 趋势分桶：单日内按 1 小时，两天内按 2 小时，62 天内按天，更长按周；边界跟随本地时区
pub fn buckets(start: i64, end: i64) -> (&'static str, Vec<(i64, i64, String)>) {
    let span = end - start;
    let mut out = Vec::new();
    if span <= 86_400_000 {
        let first = Local.timestamp_millis_opt(start).single();
        // 对齐到整点，保证单日刻度落在 00:00 / 01:00 …
        let mut at = first
            .and_then(|d| d.with_minute(0)?.with_second(0)?.with_nanosecond(0))
            .map(|d| d.timestamp_millis())
            .unwrap_or(start);
        while at < end {
            let next = at + 3_600_000;
            let label = Local.timestamp_millis_opt(at).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default();
            out.push((at.max(start), next.min(end), label));
            at = next;
        }
        return ("1h", out);
    }
    if span <= 2 * 86_400_000 {
        let first = Local.timestamp_millis_opt(start).single();
        // 对齐到偶数整点，保证刻度落在 00:00 / 02:00 …
        let mut at = first
            .and_then(|d| d.with_minute(0)?.with_second(0)?.with_nanosecond(0))
            .map(|d| d - Duration::hours(i64::from(d.hour() % 2)))
            .map(|d| d.timestamp_millis())
            .unwrap_or(start);
        while at < end {
            let next = at + 2 * 3_600_000;
            let label = Local.timestamp_millis_opt(at).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default();
            out.push((at.max(start), next.min(end), label));
            at = next;
        }
        return ("2h", out);
    }
    let step = if span <= 62 * 86_400_000 { 1 } else { 7 };
    let mut day = local_date(start);
    loop {
        let at = local_ms(day);
        if at >= end {
            break;
        }
        let next_day = day + Duration::days(step);
        out.push((at.max(start), local_ms(next_day).min(end), day.format("%m/%d").to_string()));
        day = next_day;
    }
    (if step == 1 { "day" } else { "week" }, out)
}

pub fn trend(reqs: &[Req], start: i64, end: i64) -> Trend {
    let (bucket, ranges) = buckets(start, end);
    let mut groups: Vec<Vec<&Req>> = vec![Vec::new(); ranges.len()];
    for r in reqs {
        if let Some(i) = ranges.iter().position(|(s, e, _)| r.ts >= *s && r.ts < *e) {
            groups[i].push(r);
        }
    }
    let known_or_none = |group: &[&Req], pick: &dyn Fn(&Req) -> Option<i64>| -> Option<i64> {
        if group.is_empty() {
            return Some(0);
        }
        let (known, unknown) = known_subtotal(group.iter().copied(), pick);
        (unknown < group.len() as i64).then_some(known)
    };
    let points = ranges
        .into_iter()
        .zip(groups)
        .map(|((start, end, label), group)| {
            // 按已知小计汇总分桶柱图，避免单条未知语义/缺字段记录把整根柱子抹成 None/0；
            // 同时通过 unknown_requests 告诉界面有多少条未提供完整分项。
            let unknown_requests = group
                .iter()
                .filter(|r| r.total.is_none() || r.fresh_input.is_none())
                .count() as i64;
            let cache_known: Vec<&Req> = group
                .iter()
                .copied()
                .filter(|r| r.cache_read.is_some() && r.hit_denominator.is_some())
                .collect();
            TrendBucket {
                start,
                end,
                label,
                fresh_input: known_or_none(&group, &|r| r.fresh_input),
                output: known_or_none(&group, &|r| r.output),
                cache_read: known_or_none(&group, &|r| r.cache_read),
                cache_write: known_or_none(&group, &|r| r.cache_write),
                total: known_or_none(&group, &|r| r.total),
                requests: group.len() as i64,
                unknown_requests,
                cache_hit_rate: hit_rate(cache_known.iter().copied()),
                cost: cost(group.iter().copied()).amount,
            }
        })
        .collect();
    Trend { bucket, points }
}
