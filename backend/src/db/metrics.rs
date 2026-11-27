//! 自然周期边界计算、区间过滤、费用估算与测试用聚合查询。

use chrono::{DateTime, Datelike, Duration, Local, TimeZone};
use rusqlite::{params, Connection};
use serde::Serialize;

/* ------------------------------------------------------------------------ */
/*  自然周期：使用系统时区；本周从周一 00:00 起，本月从月初起                 */
/* ------------------------------------------------------------------------ */

pub(super) fn start_of_today_at(end: i64) -> DateTime<Local> {
    let now = Local.timestamp_millis_opt(end).single().unwrap_or_else(Local::now);
    local_midnight(now.date_naive()).unwrap_or(now)
}

fn local_midnight(date: chrono::NaiveDate) -> Option<DateTime<Local>> {
    let naive = date.and_hms_opt(0, 0, 0)?;
    Local.from_local_datetime(&naive).earliest()
}

fn start_of_week_at(end: i64) -> DateTime<Local> {
    let today = start_of_today_at(end);
    // 在本地日历日期上回退到周一，再重新解析当地午夜；不能从带时区时间戳
    // 直接减 24 小时，否则跨夏令时时可能落到 23:00 或 01:00。
    let monday = today.date_naive()
        - Duration::days(today.weekday().num_days_from_monday() as i64);
    local_midnight(monday).unwrap_or(today)
}

fn start_of_month_at(end: i64) -> DateTime<Local> {
    let today = start_of_today_at(end);
    chrono::NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
        .and_then(local_midnight)
        .unwrap_or(today)
}

/// 返回 [start, end) 的 UTC 毫秒。`total` 的起点取最早记录，不能称为账号终生消耗
/// 自定义时间范围（§2.3）。两端都是本地时区的毫秒时间戳。
/// `end` 为 None 表示「结束时间跟随当前时刻」，每次查询取当时的现在。
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
pub struct CustomRange {
    pub start: i64,
    pub end: Option<i64>,
}

/// 解析区间：period == "custom" 时用显式范围，否则按预设周期。
/// 自定义范围只筛选历史 Token 查询，不影响额度窗口（§2.3）。
pub fn resolve_range_at(
    conn: &Connection,
    platform: &str,
    period: &str,
    custom: Option<CustomRange>,
    query_end_ms: i64,
) -> (i64, i64) {
    match (period, custom) {
        ("custom", Some(r)) => (r.start, r.end.unwrap_or(query_end_ms)),
        _ => period_range_at(conn, platform, period, query_end_ms),
    }
}

pub fn period_range_at(conn: &Connection, platform: &str, period: &str, end: i64) -> (i64, i64) {
    let start = match period {
        "today" => start_of_today_at(end).timestamp_millis(),
        "week" => start_of_week_at(end).timestamp_millis(),
        "month" => start_of_month_at(end).timestamp_millis(),
        _ if platform == "all" => conn
            .query_row("SELECT MIN(ts) FROM requests", [], |r| r.get::<_, Option<i64>>(0))
            .ok().flatten().unwrap_or(end),
        _ => conn
            .query_row(
                "SELECT MIN(ts) FROM requests WHERE platform = ?1",
                params![platform],
                |r| r.get::<_, Option<i64>>(0),
            )
            .ok().flatten().unwrap_or(end),
    };
    (start, end)
}

pub fn period_range(conn: &Connection, platform: &str, period: &str) -> (i64, i64) {
    period_range_at(conn, platform, period, Local::now().timestamp_millis())
}

/// `idx` 是平台参数在该 SQL 中的占位序号
pub(super) fn platform_clause_n(platform: &str, idx: usize) -> (String, Vec<String>) {
    if platform == "all" {
        (String::new(), vec![])
    } else {
        (format!(" AND platform = ?{idx}"), vec![platform.to_string()])
    }
}

/// 平台 + 模型过滤。两个维度都可缺省，占位序号由构造时给定的起点依次排开，
/// 调用方不再手工数 `?3` / `?4`——统计、趋势、费用和日志四类查询共用同一套过滤条件，
/// 序号一旦算错就是静默的错筛，而不是编译错误。
pub(super) struct Filter {
    pub(super) clause: String,
    args: Vec<rusqlite::types::Value>,
}

impl Filter {
    /// `first` 是第一个可选参数的占位序号；它前面的序号留给各查询自己的固定参数。
    /// `connection` 只筛经本地代理归账到该连接的记录；None 为全部本机记录。
    pub(super) fn new(platform: &str, model: Option<&str>, connection: Option<&str>, first: usize) -> Self {
        let mut clause = String::new();
        let mut args = Vec::new();
        if platform != "all" {
            clause.push_str(&format!(" AND platform = ?{}", first + args.len()));
            args.push(rusqlite::types::Value::Text(platform.to_string()));
        }
        if let Some(id) = connection {
            clause.push_str(&format!(" AND connection_id = ?{}", first + args.len()));
            args.push(rusqlite::types::Value::Text(id.to_string()));
        }
        // 模型名允许为空字符串（来源没给模型时记录为 NULL），"all" 作为「不筛选」的哨兵；
        // 真有模型叫 all 也不会误伤——前端传的是 None 而非字符串 "all"。
        if let Some(name) = model {
            clause.push_str(&format!(" AND model = ?{}", first + args.len()));
            args.push(rusqlite::types::Value::Text(name.to_string()));
        }
        Self { clause, args }
    }

    /// 把固定参数与过滤参数拼成一条完整的参数表。
    pub(super) fn params(&self, fixed: &[i64]) -> Vec<rusqlite::types::Value> {
        let mut out: Vec<_> = fixed.iter().map(|v| rusqlite::types::Value::Integer(*v)).collect();
        out.extend(self.args.iter().cloned());
        out
    }
}

/// 区间内出现过的模型名，供筛选下拉使用。NULL（来源未提供模型）不进列表——
/// 它不是一个可供选择的模型，而是「这条记录没说」。
/// 测试沿用的旧签名：不按连接筛选。
#[cfg(test)]
pub fn list_models_at(
    conn: &Connection,
    platform: &str,
    period: &str,
    custom: Option<CustomRange>,
    query_end_ms: i64,
) -> rusqlite::Result<Vec<String>> {
    list_models_at_scoped(conn, platform, period, custom, None, query_end_ms)
}

#[cfg(test)]
pub fn list_models_at_scoped(
    conn: &Connection,
    platform: &str,
    period: &str,
    custom: Option<CustomRange>,
    connection: Option<&str>,
    query_end_ms: i64,
) -> rusqlite::Result<Vec<String>> {
    let (start, end) = resolve_range_at(conn, platform, period, custom, query_end_ms);
    let filter = Filter::new(platform, None, connection, 3);
    let sql = format!(
        "SELECT DISTINCT model FROM requests
         WHERE ts >= ?1 AND ts < ?2 AND model IS NOT NULL AND model <> ''{}
         ORDER BY model",
        filter.clause
    );
    let mut stmt = conn.prepare(&sql)?;
    let models = stmt
        .query_map(rusqlite::params_from_iter(filter.params(&[start, end])), |r| r.get(0))?
        .collect();
    models
}

#[derive(Debug, Serialize)]
#[cfg(test)]
pub struct PeriodTotals {
    pub today: Option<i64>,
    pub week: Option<i64>,
    pub month: Option<i64>,
    pub total: Option<i64>,
    /// 自定义区间的合计；未使用自定义时间时为 None
    pub custom: Option<i64>,
    /// 累计的采集起点（本地时间字符串），用于「采集起点 2026/03/02」这类说明
    pub collected_since: Option<String>,
}

/// 测试沿用的旧签名：不按连接筛选。
#[cfg(test)]
pub fn token_totals_at(
    conn: &Connection,
    platform: &str,
    custom: Option<CustomRange>,
    model: Option<&str>,
    query_end_ms: i64,
) -> rusqlite::Result<PeriodTotals> {
    token_totals_at_scoped(conn, platform, custom, model, None, query_end_ms)
}

#[cfg(test)]
pub fn token_totals_at_scoped(
    conn: &Connection,
    platform: &str,
    custom: Option<CustomRange>,
    model: Option<&str>,
    connection: Option<&str>,
    query_end_ms: i64,
) -> rusqlite::Result<PeriodTotals> {
    let sum_range = |start: i64, end: i64| -> rusqlite::Result<Option<i64>> {
        let filter = Filter::new(platform, model, connection, 3);
        let sql = format!(
            "SELECT CASE WHEN COUNT(*) = 0 THEN {empty}
                         WHEN MIN(total_known) = 1 THEN SUM(total_tokens)
                         ELSE NULL END
             FROM requests WHERE ts >= ?1 AND ts < ?2{EXCLUDE_DEGENERATE_SQL}{}",
            filter.clause, empty=if crate::platforms::native(platform) { "NULL" } else { "0" }
        );
        conn.query_row(&sql, rusqlite::params_from_iter(filter.params(&[start, end])), |r| r.get(0))
    };

    // 四个周期共用一次扫描：累计的起点就是该平台最早记录，等价于结束时刻之前的全部记录；
    // 今日 / 本周 / 本月用条件聚合在同一批行里分别求和，口径与逐段查询一致。
    let empty = if crate::platforms::native(platform) { "NULL" } else { "0" };
    let starts = ["today", "week", "month"].map(|period| period_range_at(conn, platform, period, query_end_ms).0);
    let within = |slot: usize| format!(
        "CASE WHEN COUNT(CASE WHEN ts >= ?{slot} THEN 1 END) = 0 THEN {empty}
              WHEN MIN(CASE WHEN ts >= ?{slot} THEN total_known END) = 1
                THEN SUM(CASE WHEN ts >= ?{slot} THEN total_tokens END)
              ELSE NULL END"
    );
    let filter = Filter::new(platform, model, connection, 5);
    let sql = format!(
        "SELECT {}, {}, {},
                CASE WHEN COUNT(*) = 0 THEN {empty} WHEN MIN(total_known) = 1 THEN SUM(total_tokens) ELSE NULL END
         FROM requests WHERE ts < ?1{EXCLUDE_DEGENERATE_SQL}{}",
        within(2), within(3), within(4), filter.clause
    );
    let out: [Option<i64>; 4] = conn.query_row(
        &sql,
        rusqlite::params_from_iter(filter.params(&[query_end_ms, starts[0], starts[1], starts[2]])),
        |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]),
    )?;

    // 自定义区间与四个预设周期并列返回，界面切到「自定义时间」时用它
    let custom_total = match custom {
        Some(_) => {
            let (s, e) = resolve_range_at(conn, platform, "custom", custom, query_end_ms);
            sum_range(s, e)?
        }
        None => None,
    };
    // 采集起点跟随当前筛选：筛了模型就显示该模型最早的记录时间，
    // 否则「累计 X（采集起点 Y）」里的两个数会来自不同范围，自相矛盾。
    let since_filter = Filter::new(platform, model, connection, 1);
    let since_sql = format!(
        "SELECT MIN(ts) FROM requests WHERE 1 = 1{}",
        since_filter.clause
    );
    let collected_since = conn.query_row(
        &since_sql,
        rusqlite::params_from_iter(since_filter.params(&[])),
        |r| r.get::<_, Option<i64>>(0),
    )?
        .map(|ms| {
            Local
                .timestamp_millis_opt(ms)
                .single()
                .map(|d| d.format("%Y/%m/%d").to_string())
                .unwrap_or_default()
        });
    Ok(PeriodTotals {
        today: out[0],
        week: out[1],
        month: out[2],
        total: out[3],
        custom: custom_total,
        collected_since,
    })
}

/// 区间内的 Token 分项。每一项都可能未知——来源没提供该字段时不能当成 0
/// （§DATA-09）：把「没说」画成 0 会让用户以为真的没有缓存命中。
#[derive(Debug, Serialize)]
#[cfg(test)]
pub struct UsageBreakdown {
    /// 新增输入：已扣除缓存重读，两个平台口径一致（见 `FRESH_INPUT_SQL`）
    pub fresh_input: Option<i64>,
    pub output: Option<i64>,
    pub cache_write: Option<i64>,
    pub cache_read: Option<i64>,
    /// 真实消耗，直接取 `total_tokens` 之和，与四张周期卡同源
    pub real_total: Option<i64>,
    /// 请求条数。本地记录可精确计数，不会未知
    pub requests: i64,
    /// 缓存命中率 0–1；任一分量未知则为 None，不画进度条
    pub cache_hit_rate: Option<f64>,
    /// 平均每次请求消耗；无请求或总量未知时为 None
    pub avg_per_request: Option<f64>,
}

/// 「新增输入」的跨平台归一化。
///
/// Claude 的 `input_tokens` 本就不含缓存（缓存读写各自单列），直接用；
/// Codex 的 `input_tokens` **已含** `cached_input_tokens`（见 collector.rs 对
/// `token_usage_record` 的注释），必须减掉，否则新增输入会把缓存重读算进去而虚高。
/// 减法夹到 0：来源偶发的 cached > input 不应产生负数。
#[cfg(test)]
const FRESH_INPUT_SQL: &str =
    "CASE input_semantics WHEN 'includes_cache' THEN max(input_tokens - cache_read_tokens, 0) WHEN 'excludes_cache' THEN input_tokens ELSE NULL END";

/// 同理，新增输入的「是否已知」在 Codex 上依赖两个字段：缺任何一个都算未知。
#[cfg(test)]
const FRESH_INPUT_KNOWN_SQL: &str =
    "CASE input_semantics WHEN 'includes_cache' THEN input_known * cache_read_known WHEN 'excludes_cache' THEN input_known ELSE 0 END";

/// 命中率分母是「本可以命中的输入」：新增输入 + 缓存写入 + 缓存命中。
/// 任一分量未知则整个比率未知——用已知的部分凑一个分母会低估分母、高估命中率。
#[cfg(test)]
const HIT_DENOMINATOR_KNOWN_SQL: &str = "CASE input_semantics      WHEN 'includes_cache' THEN input_known * cache_read_known      WHEN 'excludes_cache' THEN input_known * cache_read_known ELSE 0 END";
#[cfg(test)]
const HIT_DENOMINATOR_SQL: &str = "CASE input_semantics WHEN 'includes_cache' THEN input_tokens      WHEN 'excludes_cache' THEN input_tokens + cache_read_tokens + CASE WHEN cache_write_known = 1 THEN cache_write_tokens ELSE 0 END END";

/// 退化行排除（2026-09-19 鼠鼠定版）：已知 Token 字段全为 0 却缺任一缓存字段的
/// 记录——实测来自第三方网关的空响应（usage 只有 input/output 两个 0）——本身
/// 不携带可统计信息，却会让区间的合计、命中率与费用整体变「未知」。统计聚合
/// 一律跳过它；记录保留在库中，请求日志（request_log_at）特意不加本条件，照常显示。
/// 限定 claude：Codex 协议本就不上报缓存写入，缺字段在那边是常态而非退化。
/// 一行请求的「新鲜 Token」（输入 + 输出，不含缓存重读），用于灵动岛本机今日 Token 与
/// 「今日按来源」。只在 `total_known = 1` 的行上求值，列名不带表别名。
///
/// - Codex：输入含缓存读写，扣掉；Claude / 自定义：输入本就不含缓存
/// - 原生平台按 `input_semantics`：`includes_cache` 扣缓存读，`excludes_cache` 直接相加
/// - 语义未知或分项缺失：拆不出缓存，只能沿用来源总量（上界）
pub(crate) const FRESH_TOKENS_SQL: &str = "CASE
    WHEN platform = 'codex' THEN MAX(input_tokens - cache_read_tokens - cache_write_tokens, 0) + output_tokens
    WHEN platform IN ('claude', 'custom') THEN input_tokens + output_tokens
    WHEN input_semantics = 'includes_cache' AND input_known = 1 AND output_known = 1 AND cache_read_known = 1
        THEN MAX(input_tokens - cache_read_tokens, 0) + output_tokens
    WHEN input_semantics = 'excludes_cache' AND input_known = 1 AND output_known = 1
        THEN input_tokens + output_tokens
    ELSE total_tokens
END";

pub(crate) const EXCLUDE_DEGENERATE_SQL: &str = " AND NOT (platform = 'claude' AND input_known = 1 AND input_tokens = 0 \
     AND output_known = 1 AND output_tokens = 0 \
     AND (cache_read_known = 0 OR cache_write_known = 0 OR (COALESCE(model, '') = '<synthetic>' AND COALESCE(total_tokens, 0) = 0)))";

/// 测试沿用的旧签名：不按连接筛选。
#[cfg(test)]
pub fn usage_breakdown_at(
    conn: &Connection,
    platform: &str,
    period: &str,
    custom: Option<CustomRange>,
    model: Option<&str>,
    query_end_ms: i64,
) -> rusqlite::Result<UsageBreakdown> {
    usage_breakdown_at_scoped(conn, platform, period, custom, model, None, query_end_ms)
}

#[cfg(test)]
pub fn usage_breakdown_at_scoped(
    conn: &Connection,
    platform: &str,
    period: &str,
    custom: Option<CustomRange>,
    model: Option<&str>,
    connection: Option<&str>,
    query_end_ms: i64,
) -> rusqlite::Result<UsageBreakdown> {
    let (start, end) = resolve_range_at(conn, platform, period, custom, query_end_ms);
    let filter = Filter::new(platform, model, connection, 3);
    let sql = format!(
        "SELECT COUNT(*),
                MIN({FRESH_INPUT_KNOWN_SQL}), MIN(output_known), MIN(cache_write_known),
                MIN(cache_read_known), MIN(total_known),
                SUM({FRESH_INPUT_SQL}), SUM(output_tokens), SUM(cache_write_tokens),
                SUM(cache_read_tokens), SUM(total_tokens),
                MIN({HIT_DENOMINATOR_KNOWN_SQL}), SUM({HIT_DENOMINATOR_SQL})
         FROM requests WHERE ts >= ?1 AND ts < ?2{EXCLUDE_DEGENERATE_SQL}{}",
        filter.clause
    );
    let row = conn.query_row(
        &sql,
        rusqlite::params_from_iter(filter.params(&[start, end])),
        |r| {
            let known = |index: usize| -> rusqlite::Result<bool> {
                Ok(r.get::<_, Option<i64>>(index)?.unwrap_or(0) != 0)
            };
            Ok((
                r.get::<_, i64>(0)?,
                (known(1)?, known(2)?, known(3)?, known(4)?, known(5)?),
                (
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, Option<i64>>(7)?,
                    r.get::<_, Option<i64>>(8)?,
                    r.get::<_, Option<i64>>(9)?,
                    r.get::<_, Option<i64>>(10)?,
                ),
                // 命中率分母与分项同一次扫描取出，不再单独扫一遍区间
                (r.get::<_, Option<i64>>(11)? == Some(1)).then(|| r.get::<_, Option<i64>>(12)).transpose()?.flatten(),
            ))
        },
    )?;
    let (requests, (fi_known, o_known, cw_known, cr_known, t_known), (fi, o, cw, cr, total), denominator) = row;

    // 没有任何记录时四项都是真实的 0，不是未知：区间内确实没用过。
    let at = |known: bool, value: Option<i64>| -> Option<i64> {
        if requests == 0 {
            if crate::platforms::native(platform) { None } else { Some(0) }
        } else if known {
            value
        } else {
            None
        }
    };
    let fresh_input = at(fi_known, fi);
    let output = at(o_known, o);
    let cache_write = at(cw_known, cw);
    let cache_read = at(cr_known, cr);
    let real_total = at(t_known, total);

    let cache_hit_rate = cache_read.zip(denominator).and_then(|(r,d)| (d>0 && r<=d).then_some(r as f64/d as f64));
    let avg_per_request = match real_total {
        Some(t) if requests > 0 => Some(t as f64 / requests as f64),
        _ => None,
    };

    Ok(UsageBreakdown {
        fresh_input,
        output,
        cache_write,
        cache_read,
        real_total,
        requests,
        cache_hit_rate,
        avg_per_request,
    })
}

#[derive(Debug, Serialize)]
#[cfg(test)]
pub struct TrendPoint {
    pub label: String,
    /// 总量，保持原字段名与语义；界面切「总量」视图时用它
    pub tokens: Option<i64>,
    pub fresh_input: Option<i64>,
    pub output: Option<i64>,
    pub cache_write: Option<i64>,
    pub cache_read: Option<i64>,
    /// 该桶的估算费用；桶内存在无价目模型或缺字段时为 None，界面断线而不画 0
    pub cost: Option<f64>,
}

#[derive(Debug, Serialize)]
#[cfg(test)]
pub struct Trend {
    pub points: Vec<TrendPoint>,
    /// 实际分桶粒度，界面需明示（§7.5）
    pub bucket: String,
}

/// 今日按小时、周/月按日；长时间范围降低点数而非加载全部日志
#[cfg(test)]
pub fn trend(
    conn: &Connection,
    platform: &str,
    period: &str,
    custom: Option<CustomRange>,
) -> rusqlite::Result<Trend> {
    trend_at(conn, platform, period, custom, None, Local::now().timestamp_millis())
}

/// 测试沿用的旧签名：不按连接筛选。
#[cfg(test)]
pub fn trend_at(
    conn: &Connection,
    platform: &str,
    period: &str,
    custom: Option<CustomRange>,
    model: Option<&str>,
    query_end_ms: i64,
) -> rusqlite::Result<Trend> {
    trend_at_scoped(conn, platform, period, custom, model, None, query_end_ms)
}

#[cfg(test)]
pub fn trend_at_scoped(
    conn: &Connection,
    platform: &str,
    period: &str,
    custom: Option<CustomRange>,
    model: Option<&str>,
    connection: Option<&str>,
    query_end_ms: i64,
) -> rusqlite::Result<Trend> {
    let (start, end) = resolve_range_at(conn, platform, period, custom, query_end_ms);
    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;
    const MAX_POINTS: i64 = 180;
    let hourly = period == "today" || (period == "custom" && end - start <= 2 * DAY);
    let raw_days = ((end - start).max(1) + DAY - 1) / DAY;
    let days_per_bucket = if hourly { 0 } else { (raw_days + MAX_POINTS - 1) / MAX_POINTS };
    let bucket_ms = if hourly { HOUR } else { days_per_bucket.max(1) * DAY };
    let count = if period == "today" { 24 } else {
        (((end - start).max(1) + bucket_ms - 1) / bucket_ms).clamp(1, MAX_POINTS)
    } as usize;
    // 按「桶 × 平台 × 模型」聚合：分项展示只需桶，但计价必须知道是哪个平台的哪个模型，
    // 所以一次查询同时带回两者，前端要的五个系列和费用都从这一份结果里算出来。
    let filter = Filter::new(platform, model, connection, 4);
    let sql = format!(
        "SELECT CAST((ts - ?1) / ?3 AS INTEGER) AS bucket_index, platform, model,
                SUM(total_tokens), SUM({FRESH_INPUT_SQL}), SUM(output_tokens),
                SUM(cache_write_tokens), SUM(cache_read_tokens), SUM(input_tokens),
                MIN(input_known), MIN(output_known), MIN(cache_write_known),
                MIN(cache_read_known), MIN(total_known), MIN({FRESH_INPUT_KNOWN_SQL}),
                MIN(ts), (ts / 3600000) % 168 AS hour_of_week
         FROM requests WHERE ts >= ?1 AND ts < ?2{EXCLUDE_DEGENERATE_SQL}{}
         GROUP BY bucket_index, platform, model, hour_of_week",
        filter.clause
    );
    let mut stmt = conn.prepare(&sql)?;
    #[derive(Default, Clone)]
    struct Bucket {
        tokens: i64,
        fresh_input: i64,
        output: i64,
        cache_write: i64,
        cache_read: i64,
        cost: f64,
        /// 桶内出现过无法计价的记录（缺字段或模型不在价目表），费用整桶作废
        cost_known: bool,
        known: [bool; 5],
        has_records: bool,
    }
    let mut buckets = vec![Bucket { cost_known: !crate::platforms::native(platform), known: [!crate::platforms::native(platform); 5], ..Default::default() }; count];
    let map = |row: &rusqlite::Row| -> rusqlite::Result<_> {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            [
                row.get::<_, i64>(3)?, row.get::<_, Option<i64>>(4)?.unwrap_or(0), row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?, row.get::<_, i64>(7)?, row.get::<_, i64>(8)?,
            ],
            (9..=13).all(|index| row.get::<_, i64>(index).unwrap_or(0) != 0),
            [13,14,10,11,12].map(|index| row.get::<_, i64>(index).unwrap_or(0) != 0),
            row.get::<_, i64>(15)?,
        ))
    };
    let rows: Vec<_> = stmt
        .query_map(
            rusqlite::params_from_iter(filter.params(&[start, end, bucket_ms])),
            map,
        )?
        .collect::<Result<_, _>>()?;
    // 同一桶内再按「一周中的第几个小时」拆行（峰谷价只取决于 UTC 小时与星期），
    // 每行用行内任一请求的时刻计价，费用不随查看时刻变化；Token 照常累加回桶里。
    for (index, row_platform, row_model, sums, fields_known, known, sample_ts) in rows {
        if index < 0 || (index as usize) >= buckets.len() {
            continue;
        }
        let [total, fresh_input, output, cache_write, cache_read, raw_input] = sums;
        let bucket = &mut buckets[index as usize];
        if !bucket.has_records {bucket.known=known;bucket.cost_known=true;bucket.has_records=true;}
        else {for (value, incoming) in bucket.known.iter_mut().zip(known) { *value &= incoming; }}
        bucket.tokens += total;
        bucket.fresh_input += fresh_input;
        bucket.output += output;
        bucket.cache_write += cache_write;
        bucket.cache_read += cache_read;
        // 计价拿的是**原始** input_tokens：Codex 的缓存扣除在 pricing::estimate 内部完成
        // （见 QUOTA-13），这里先减一次就会重复扣减，费用被系统性低估。
        match fields_known
            .then(|| {
                crate::pricing::estimate_usage_at(
                    Some(&row_platform), row_model.as_deref(),
                    crate::pricing::Usage { input: raw_input, output, cache_write, cache_read, ..Default::default() },
                    chrono::DateTime::from_timestamp_millis(sample_ts).unwrap_or_else(chrono::Utc::now),
                )
            })
            .flatten()
        {
            Some(value) => bucket.cost += value,
            None => bucket.cost_known = false,
        }
    }
    let bucket = if hourly { "1 小时".to_string() } else { format!("{} 天", days_per_bucket.max(1)) };
    let start_year = Local.timestamp_millis_opt(start).single().map(|date| date.year());
    let end_year = Local.timestamp_millis_opt(end.saturating_sub(1)).single().map(|date| date.year());
    let crosses_year = start_year != end_year;
    Ok(Trend {
        points: buckets
            .into_iter()
            .enumerate()
            .map(|(index, bucket)| {
                let timestamp = start + index as i64 * bucket_ms;
                let label = Local.timestamp_millis_opt(timestamp).single().map(|date| {
                    if period == "today" { date.format("%H").to_string() }
                    else if hourly && crosses_year { date.format("%Y/%m/%d %H").to_string() }
                    else if hourly { date.format("%m/%d %H").to_string() }
                    else if days_per_bucket > 1 || crosses_year { date.format("%Y/%m/%d").to_string() }
                    else { date.format("%m/%d").to_string() }
                }).unwrap_or_default();
                TrendPoint {
                    label,
                    tokens: bucket.known[0].then_some(bucket.tokens),
                    fresh_input: bucket.known[1].then_some(bucket.fresh_input),
                    output: bucket.known[2].then_some(bucket.output),
                    cache_write: bucket.known[3].then_some(bucket.cache_write),
                    cache_read: bucket.known[4].then_some(bucket.cache_read),
                    cost: bucket.cost_known.then_some(bucket.cost),
                }
            })
            .collect(),
        bucket,
    })
}

#[derive(Debug, Serialize)]
#[cfg(test)]
pub struct LogRow {
    pub native_outcome: Option<String>,
    pub id: i64,
    pub time: String,
    pub date: String,
    pub model: Option<String>,
    pub input_semantics: String,
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub total: Option<i64>,
    pub session_id: Option<String>,
    pub platform: String,
    /// 估算金额（美元）。本地记录不含账单，故必须标注「估算」；
    /// 模型不在价目表内时为 None，界面显示「—」而不是 0
    pub cost_estimate: Option<f64>,
    /// 思考强度，来源原样透传。加列之前的历史记录为 None，界面显示「—」
    pub effort: Option<String>,
    /// 请求总耗时（毫秒）。只有经本地代理转发的请求才有，否则 None
    pub duration_ms: Option<i64>,
    /// 首字延迟（毫秒），同上
    pub first_token_ms: Option<i64>,
    /// 上游真实 HTTP 响应码，同上。没有时前端按「成功」展示（能写入用量即响应已返回），不补造 200
    pub status_code: Option<i64>,
}

#[derive(Debug, Serialize)]
#[cfg(test)]
pub struct LogPage {
    pub rows: Vec<LogRow>,
    /// 总条数；本地来源可精确计数，因此不为 null
    pub total_count: i64,
    pub page: u32,
    pub page_size: u32,
    pub page_count: u32,
}

/// 区间内的估算费用合计（§2.4）。
/// `uncovered_tokens` 是价目表未覆盖模型的 Token 数——不为零时界面需说明
/// 「部分模型无价目」，否则用户会以为这就是全部花费。
#[derive(Debug, Serialize)]
pub struct CostEstimate {
    pub amount: f64,
    /// 恒为 true：本地记录推算出的都是估算值，不是账单
    pub estimated: bool,
    pub uncovered_tokens: i64,
    /// false 表示区间内至少一条记录缺少计价所需 Token 字段。
    pub complete: bool,
}

/// 测试沿用的旧签名：不按连接筛选。
#[cfg(test)]
pub fn cost_estimate_at(
    conn: &Connection,
    platform: &str,
    period: &str,
    custom: Option<CustomRange>,
    model: Option<&str>,
    query_end_ms: i64,
) -> rusqlite::Result<CostEstimate> {
    cost_estimate_at_scoped(conn, platform, period, custom, model, None, query_end_ms)
}

pub fn cost_estimate_at_scoped(
    conn: &Connection,
    platform: &str,
    period: &str,
    custom: Option<CustomRange>,
    model: Option<&str>,
    connection: Option<&str>,
    query_end_ms: i64,
) -> rusqlite::Result<CostEstimate> {
    let (start, end) = resolve_range_at(conn, platform, period, custom, query_end_ms);
    let filter = Filter::new(platform, model, connection, 3);
    let snapshot = crate::attribution::Snapshot::load(conn).ok();
    let mut resolver = crate::pricing::PriceResolver::load(conn);
    // 逐条按单次请求计价，避免分组求和把普通短请求误判成超长上下文阶梯。
    let sql = format!(
        "SELECT requests.platform, requests.model, requests.upstream_model, requests.served_model,
                requests.input_tokens, requests.output_tokens, requests.cache_write_tokens, requests.cache_read_tokens,
                requests.cache_write_1h_tokens, requests.total_tokens,
                requests.input_known, requests.output_known, requests.cache_write_known, requests.cache_read_known,
                requests.total_known, requests.input_semantics, requests.connection_id, requests.provider_key,
                COALESCE(m.reasoning, 0), COALESCE(m.tool, 0), requests.ts
         FROM requests LEFT JOIN request_metrics m ON m.request_id = requests.id
         WHERE requests.ts >= ?1 AND requests.ts < ?2{EXCLUDE_DEGENERATE_SQL}{}",
        filter.clause
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(filter.params(&[start, end])), |r| {
        let opt = |v: usize, k: usize| -> rusqlite::Result<Option<i64>> {
            Ok(if r.get::<_, i64>(k)? != 0 { Some(r.get(v)?) } else { None })
        };
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            opt(4, 10)?,
            opt(5, 11)?,
            opt(6, 12)?,
            opt(7, 13)?,
            r.get::<_, Option<i64>>(8)?.unwrap_or(0),
            opt(9, 14)?,
            r.get::<_, String>(15)?,
            r.get::<_, Option<String>>(16)?,
            r.get::<_, Option<String>>(17)?,
            r.get::<_, i64>(18)?,
            r.get::<_, i64>(19)?,
            r.get::<_, i64>(20)?,
        ))
    })?;

    let mut amount = 0.0;
    let mut uncovered = 0i64;
    let mut complete = true;
    for row in rows {
        let (row_platform, model, upstream, served, input, output, cw, cr, cw1h, total, semantics, conn_id, provider_key, reasoning, tool, ts) = row?;
        let attribution = snapshot
            .as_ref()
            .map_or_else(crate::attribution::AttributionView::platform, |snap| {
                snap.resolve(&row_platform, conn_id.as_deref(), provider_key.as_deref(), model.as_deref())
            });
        let effective_conn = attribution.is_connection().then(|| attribution.connection_id.as_deref()).flatten();
        let at = chrono::DateTime::from_timestamp_millis(ts).unwrap_or_else(chrono::Utc::now);
        let res = resolver.price_request(&crate::pricing::PriceRequest {
            platform: &row_platform,
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
            Some(v) => amount += v,
            None => match res.unpriced_reason {
                Some(crate::pricing::UnpricedReason::NoUsage) => complete = false,
                _ => uncovered += total.unwrap_or(0),
            },
        }
    }
    Ok(CostEstimate {
        amount,
        estimated: true,
        uncovered_tokens: uncovered,
        complete,
    })
}

/// 默认每页 10 条（界面不显示该文案）；按时间从新到旧
#[cfg(test)]
pub const PAGE_SIZE: u32 = 10;

/// 测试沿用的旧签名：不按连接筛选。
#[cfg(test)]
pub fn request_log_at(
    conn: &Connection,
    platform: &str,
    period: &str,
    page: u32,
    custom: Option<CustomRange>,
    model: Option<&str>,
    query_end_ms: i64,
) -> rusqlite::Result<LogPage> {
    request_log_at_scoped(conn, platform, period, page, custom, model, None, query_end_ms)
}

#[cfg(test)]
pub fn request_log_at_scoped(
    conn: &Connection,
    platform: &str,
    period: &str,
    page: u32,
    custom: Option<CustomRange>,
    model: Option<&str>,
    connection: Option<&str>,
    query_end_ms: i64,
) -> rusqlite::Result<LogPage> {
    let (start, end) = resolve_range_at(conn, platform, period, custom, query_end_ms);
    let filter = Filter::new(platform, model, connection, 3);
    let clause = filter.clause.as_str();

    let count_sql =
        format!("SELECT COUNT(*) FROM requests WHERE ts >= ?1 AND ts < ?2{clause}");
    let total_count: i64 = conn.query_row(
        &count_sql,
        rusqlite::params_from_iter(filter.params(&[start, end])),
        |r| r.get(0),
    )?;

    let page = page.max(1);
    let offset = ((page - 1) * PAGE_SIZE) as i64;
    let sql = format!(
        "SELECT id, ts, model,
                CASE WHEN input_known = 1 THEN input_tokens END,
                CASE WHEN output_known = 1 THEN output_tokens END,
                CASE WHEN cache_read_known = 1 THEN cache_read_tokens END,
                CASE WHEN cache_write_known = 1 THEN cache_write_tokens END,
                CASE WHEN total_known = 1 THEN total_tokens END,
                session_id, platform, effort,
                duration_ms, first_token_ms, status_code, input_semantics, native_outcome,
                m.reasoning, m.tool
         FROM requests LEFT JOIN request_metrics m ON m.request_id = requests.id
         WHERE ts >= ?1 AND ts < ?2{clause}
         ORDER BY ts DESC LIMIT {PAGE_SIZE} OFFSET {offset}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let map = |r: &rusqlite::Row| -> rusqlite::Result<LogRow> {
        let ts: i64 = r.get(1)?;
        let dt = Local.timestamp_millis_opt(ts).single();
        let model: Option<String> = r.get(2)?;
        let input: Option<i64> = r.get(3)?;
        let output: Option<i64> = r.get(4)?;
        let cache_read: Option<i64> = r.get(5)?;
        let cache_write: Option<i64> = r.get(6)?;
        let row_platform: String = r.get(9)?;
        let reasoning: Option<i64> = r.get(16)?;
        let tool: Option<i64> = r.get(17)?;
        Ok(LogRow {
            id: r.get(0)?,
            time: dt.map(|d| d.format("%H:%M:%S").to_string()).unwrap_or_default(),
            date: dt.map(|d| d.format("%Y/%m/%d").to_string()).unwrap_or_default(),
            cost_estimate: input.zip(output).zip(cache_write).zip(cache_read)
                .and_then(|(((input, output), cache_write), cache_read)| crate::pricing::estimate_usage_at(
                    Some(&row_platform), model.as_deref(), crate::pricing::Usage {
                        input, output, cache_write, cache_read,
                        reasoning: reasoning.unwrap_or(0), tool: tool.unwrap_or(0),
                        long_prompt: input > crate::pricing::GEMINI_LONG_PROMPT_TOKENS,
                        ..Default::default()
                    },
                    // 按请求自身时间计价（峰谷价），不随查看时刻变化
                    chrono::DateTime::from_timestamp_millis(ts).unwrap_or_else(chrono::Utc::now),
                )),
            model,
            input_semantics: r.get(14)?,
            native_outcome:r.get(15)?,
            input,
            output,
            cache_read,
            cache_write,
            total: r.get(7)?,
            session_id: r.get(8)?,
            platform: row_platform,
            effort: r.get(10)?,
            duration_ms: r.get(11)?,
            first_token_ms: r.get(12)?,
            status_code: r.get(13)?,
        })
    };
    let rows: Vec<LogRow> = stmt
        .query_map(rusqlite::params_from_iter(filter.params(&[start, end])), map)?
        .collect::<Result<_, _>>()?;

    let page_count = ((total_count as f64) / PAGE_SIZE as f64).ceil() as u32;
    Ok(LogPage {
        rows,
        total_count,
        page,
        page_size: PAGE_SIZE,
        page_count: page_count.max(1),
    })
}

