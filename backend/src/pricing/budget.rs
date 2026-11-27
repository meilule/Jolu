//! 全局与连接级费用预算计算、官方实账快照存取与网关硬限额校验（P2）。

use std::collections::HashMap;

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::pricing::{BillingMode, PriceRequest, PriceResolver};
use crate::settings::Settings;

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionBudgetRow {
    pub connection_id: String,
    pub name: String,
    pub platform: String,
    pub kind: String,
    pub billing_mode: String,
    pub price_multiplier: f64,
    pub serve_enabled: bool,
    pub daily_limit: Option<f64>,
    pub monthly_limit: Option<f64>,
    pub action: String,
    pub today_spend: f64,
    pub month_spend: f64,
    pub warned: bool,
    pub exceeded: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BudgetStatus {
    pub global_enabled: bool,
    pub global_daily_limit: Option<f64>,
    pub global_monthly_limit: Option<f64>,
    pub warn_percent: u8,
    pub global_today_spend: f64,
    pub global_month_spend: f64,
    pub projected_month_spend: Option<f64>,
    pub global_warned: bool,
    pub global_exceeded: bool,
    pub exceeded_connections: usize,
    pub has_any_budget: bool,
    pub connections: Vec<ConnectionBudgetRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionBudgetInput {
    pub connection_id: String,
    pub daily_limit: Option<f64>,
    pub monthly_limit: Option<f64>,
    pub action: String,
}

#[derive(Debug, Clone)]
pub struct OfficialCostSum {
    pub cost_usd: f64,
    pub synced_ms: i64,
}

pub fn validate_optional_usd(label: &str, value: Option<f64>) -> Result<Option<f64>, String> {
    match value {
        None => Ok(None),
        Some(v) if v.is_finite() && v > 0.0 && v <= 1_000_000.0 => Ok(Some(v)),
        Some(_) => Err(format!("{label} 须为大于 0 且不超过 1,000,000 的有效金额")),
    }
}

pub fn record_official_cost(
    conn: &Connection,
    connection_id: &str,
    cost_usd: f64,
    source: &str,
    now_ms: i64,
) -> rusqlite::Result<()> {
    if !cost_usd.is_finite() || cost_usd < 0.0 {
        return Ok(());
    }
    let day = Local
        .timestamp_millis_opt(now_ms)
        .single()
        .unwrap_or_else(Local::now)
        .format("%Y-%m-%d")
        .to_string();
    conn.execute(
        "INSERT INTO official_cost_snapshots(connection_id, day, cost_usd, source, synced_ms)
         VALUES(?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(connection_id, day) DO UPDATE SET
            cost_usd = excluded.cost_usd,
            source = excluded.source,
            synced_ms = excluded.synced_ms",
        params![connection_id, day, cost_usd, source, now_ms],
    )?;
    Ok(())
}

pub fn load_official_costs_in_range(
    conn: &Connection,
    start_ms: i64,
    end_ms: i64,
) -> rusqlite::Result<HashMap<String, OfficialCostSum>> {
    let start_day = local_day_str(start_ms);
    let end_day = local_day_str(end_ms.saturating_sub(1).max(start_ms));
    let mut stmt = conn.prepare(
        "SELECT connection_id, SUM(cost_usd), MAX(synced_ms)
         FROM official_cost_snapshots
         WHERE day >= ?1 AND day <= ?2
         GROUP BY connection_id",
    )?;
    let rows = stmt.query_map(params![start_day, end_day], |r| {
        Ok((
            r.get::<_, String>(0)?,
            OfficialCostSum {
                cost_usd: r.get(1)?,
                synced_ms: r.get(2)?,
            },
        ))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, sum) = row?;
        out.insert(id, sum);
    }
    Ok(out)
}

pub fn save_connection_budgets(conn: &Connection, items: &[ConnectionBudgetInput]) -> Result<(), String> {
    for item in items {
        let daily = validate_optional_usd("连接日上限", item.daily_limit)?;
        let monthly = validate_optional_usd("连接月上限", item.monthly_limit)?;
        if !matches!(item.action.as_str(), "alert" | "block") {
            return Err(format!("未知的超限动作：{}", item.action));
        }
        let serve_enabled: i64 = conn
            .query_row(
                "SELECT serve_enabled FROM connections WHERE id = ?1",
                [&item.connection_id],
                |r| r.get(0),
            )
            .map_err(|_| format!("连接不存在：{}", item.connection_id))?;
        let effective_action = if serve_enabled != 0 { item.action.as_str() } else { "alert" };
        conn.execute(
            "UPDATE connections SET budget_daily_usd = ?2, budget_monthly_usd = ?3, budget_action = ?4 WHERE id = ?1",
            params![item.connection_id, daily, monthly, effective_action],
        )
        .map_err(|e| format!("保存连接预算失败：{e}"))?;
    }
    Ok(())
}

fn local_day_str(ms: i64) -> String {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Local::now)
        .format("%Y-%m-%d")
        .to_string()
}

fn local_midnight_ms(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .and_then(|dt| Local.from_local_datetime(&dt).earliest())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

fn month_bounds_ms(now_ms: i64) -> (i64, i64, i64) {
    let local = Local
        .timestamp_millis_opt(now_ms)
        .single()
        .unwrap_or_else(Local::now);
    let today = local.date_naive();
    let month_start_date = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let next_month_date = if today.month() == 12 {
        NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
    }
    .unwrap_or(today);
    (
        local_midnight_ms(today),
        local_midnight_ms(month_start_date),
        local_midnight_ms(next_month_date),
    )
}

fn check_threshold(spend: f64, limit: Option<f64>, warn_percent: u8) -> (bool, bool) {
    let Some(lim) = limit.filter(|v| *v > 0.0) else {
        return (false, false);
    };
    let ratio = spend / lim;
    (ratio >= f64::from(warn_percent) / 100.0, spend >= lim)
}

fn accumulate_month_spend(
    conn: &Connection,
    today_start: i64,
    month_start: i64,
    now_ms: i64,
) -> Result<(f64, f64, HashMap<String, f64>, HashMap<String, f64>), String> {
    let snapshot = crate::attribution::Snapshot::load(conn).ok();
    let mut resolver = PriceResolver::load(conn);
    let mut global_today = 0.0;
    let mut global_month = 0.0;
    let mut per_conn_today: HashMap<String, f64> = HashMap::new();
    let mut per_conn_month: HashMap<String, f64> = HashMap::new();

    let sql = format!(
        "SELECT r.platform, r.model, r.upstream_model, r.served_model, r.ts,
                r.input_tokens, r.output_tokens, r.cache_read_tokens, r.cache_write_tokens,
                r.cache_write_1h_tokens, r.input_known, r.output_known, r.cache_read_known, r.cache_write_known,
                r.input_semantics, r.connection_id, r.provider_key, m.reasoning, m.tool
         FROM requests r LEFT JOIN request_metrics m ON m.request_id = r.id
         WHERE r.ts >= ?1 AND r.ts < ?2{}",
        crate::db::EXCLUDE_DEGENERATE_SQL
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![month_start, now_ms.max(month_start + 1)], |r| {
            let opt = |v: usize, k: usize| -> rusqlite::Result<Option<i64>> {
                Ok(if r.get::<_, i64>(k)? != 0 { Some(r.get(v)?) } else { None })
            };
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
                opt(5, 10)?,
                opt(6, 11)?,
                opt(7, 12)?,
                opt(8, 13)?,
                r.get::<_, Option<i64>>(9)?.unwrap_or(0),
                r.get::<_, String>(14)?,
                r.get::<_, Option<String>>(15)?,
                r.get::<_, Option<String>>(16)?,
                r.get::<_, Option<i64>>(17)?.unwrap_or(0),
                r.get::<_, Option<i64>>(18)?.unwrap_or(0),
            ))
        })
        .map_err(|e| e.to_string())?;

    for row in rows {
        let (platform, model, upstream, served, ts, input, output, cr, cw, cw1h, semantics, conn_id, provider_key, reasoning, tool) =
            row.map_err(|e| e.to_string())?;
        let attribution = snapshot
            .as_ref()
            .map_or_else(crate::attribution::AttributionView::platform, |snap| {
                snap.resolve(&platform, conn_id.as_deref(), provider_key.as_deref(), model.as_deref())
            });
        let effective_conn = attribution
            .is_connection()
            .then(|| attribution.connection_id.as_deref())
            .flatten();
        let (billing_mode, _) = resolver.billing_for(effective_conn);
        if billing_mode != BillingMode::Api {
            continue;
        }
        let at = chrono::DateTime::from_timestamp_millis(ts).unwrap_or_else(chrono::Utc::now);
        let res = resolver.price_request(&PriceRequest {
            platform: &platform,
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
        let Some(cost) = res.cost else { continue };
        global_month += cost;
        if ts >= today_start {
            global_today += cost;
        }
        if let Some(cid) = effective_conn {
            *per_conn_month.entry(cid.to_string()).or_default() += cost;
            if ts >= today_start {
                *per_conn_today.entry(cid.to_string()).or_default() += cost;
            }
        }
    }
    Ok((global_today, global_month, per_conn_today, per_conn_month))
}

pub fn compute_budget_status(
    conn: &Connection,
    settings: &Settings,
    now_ms: i64,
) -> Result<BudgetStatus, String> {
    let (today_start, month_start, next_month_start) = month_bounds_ms(now_ms);
    let warn_percent = settings.global_budget_warn_percent.clamp(50, 99);

    let (global_today, global_month, per_conn_today, per_conn_month) =
        accumulate_month_spend(conn, today_start, month_start, now_ms)?;

    let mut connections = Vec::new();
    let mut exceeded_connections = 0usize;
    let mut any_conn_budget = false;
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, platform, kind, billing_mode, price_multiplier, serve_enabled,
                        budget_daily_usd, budget_monthly_usd, budget_action
                 FROM connections ORDER BY created_ms ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?.unwrap_or_else(|| "auto".into()),
                    r.get::<_, Option<f64>>(5)?.unwrap_or(1.0),
                    r.get::<_, Option<i64>>(6)?.unwrap_or(0) != 0,
                    r.get::<_, Option<f64>>(7)?,
                    r.get::<_, Option<f64>>(8)?,
                    r.get::<_, Option<String>>(9)?.unwrap_or_else(|| "alert".into()),
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, name, platform, kind, mode_raw, mult, serve_enabled, daily_limit, monthly_limit, action) =
                row.map_err(|e| e.to_string())?;
            let resolved_mode = BillingMode::resolve(&mode_raw, &kind).as_str().to_string();
            let today_spend = per_conn_today.get(&id).copied().unwrap_or(0.0);
            let month_spend = per_conn_month.get(&id).copied().unwrap_or(0.0);
            let (dw, de) = check_threshold(today_spend, daily_limit, warn_percent);
            let (mw, me) = check_threshold(month_spend, monthly_limit, warn_percent);
            let warned = dw || mw;
            let exceeded = de || me;
            if daily_limit.is_some() || monthly_limit.is_some() {
                any_conn_budget = true;
            }
            if exceeded {
                exceeded_connections += 1;
            }
            connections.push(ConnectionBudgetRow {
                connection_id: id,
                name,
                platform,
                kind,
                billing_mode: resolved_mode,
                price_multiplier: mult,
                serve_enabled,
                daily_limit,
                monthly_limit,
                action: if serve_enabled { action } else { "alert".into() },
                today_spend,
                month_spend,
                warned,
                exceeded,
            });
        }
    }

    let (gdw, gde) = if settings.global_budget_enabled {
        check_threshold(global_today, settings.global_budget_daily_usd, warn_percent)
    } else {
        (false, false)
    };
    let (gmw, gme) = if settings.global_budget_enabled {
        check_threshold(global_month, settings.global_budget_monthly_usd, warn_percent)
    } else {
        (false, false)
    };

    let elapsed_ms = (now_ms - month_start).max(0);
    let total_month_ms = (next_month_start - month_start).max(1);
    const SIX_HOURS_MS: i64 = 6 * 3_600_000;
    let projected_month_spend = if elapsed_ms >= SIX_HOURS_MS && global_month > 0.0 {
        Some(global_month * (total_month_ms as f64 / elapsed_ms as f64))
    } else {
        None
    };

    let has_global = settings.global_budget_enabled
        && (settings.global_budget_daily_usd.is_some() || settings.global_budget_monthly_usd.is_some());

    Ok(BudgetStatus {
        global_enabled: settings.global_budget_enabled,
        global_daily_limit: settings.global_budget_daily_usd,
        global_monthly_limit: settings.global_budget_monthly_usd,
        warn_percent,
        global_today_spend: global_today,
        global_month_spend: global_month,
        projected_month_spend,
        global_warned: gdw || gmw,
        global_exceeded: gde || gme,
        exceeded_connections,
        has_any_budget: has_global || any_conn_budget,
        connections,
    })
}

/// 网关转发前检查连接硬限额：仅当连接 `budget_action = 'block'` 且设了日/月上限时触发。
pub fn check_gateway_connection_budget(
    conn: &Connection,
    connection_id: &str,
    now_ms: i64,
) -> Option<String> {
    let (name, kind, mode_raw, daily_limit, monthly_limit, action): (
        String,
        String,
        String,
        Option<f64>,
        Option<f64>,
        String,
    ) = conn
        .query_row(
            "SELECT name, kind, COALESCE(billing_mode, 'auto'), budget_daily_usd, budget_monthly_usd, COALESCE(budget_action, 'alert')
             FROM connections WHERE id = ?1",
            [connection_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .ok()?;
    if action != "block" || (daily_limit.is_none() && monthly_limit.is_none()) {
        return None;
    }
    if BillingMode::resolve(&mode_raw, &kind) == BillingMode::Free {
        return None;
    }

    let (today_start, month_start, _) = month_bounds_ms(now_ms);
    let query_start = if monthly_limit.is_some() { month_start } else { today_start };
    let mut resolver = PriceResolver::load(conn);
    let sql = format!(
        "SELECT r.platform, r.model, r.upstream_model, r.served_model, r.ts,
                r.input_tokens, r.output_tokens, r.cache_read_tokens, r.cache_write_tokens,
                r.cache_write_1h_tokens, r.input_known, r.output_known, r.cache_read_known, r.cache_write_known,
                r.input_semantics, m.reasoning, m.tool
         FROM requests r LEFT JOIN request_metrics m ON m.request_id = r.id
         WHERE r.connection_id = ?1 AND r.ts >= ?2 AND r.ts < ?3{}",
        crate::db::EXCLUDE_DEGENERATE_SQL
    );
    let mut stmt = conn.prepare(&sql).ok()?;
    let rows = stmt
        .query_map(params![connection_id, query_start, now_ms.max(query_start + 1)], |r| {
            let opt = |v: usize, k: usize| -> rusqlite::Result<Option<i64>> {
                Ok(if r.get::<_, i64>(k)? != 0 { Some(r.get(v)?) } else { None })
            };
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
                opt(5, 10)?,
                opt(6, 11)?,
                opt(7, 12)?,
                opt(8, 13)?,
                r.get::<_, Option<i64>>(9)?.unwrap_or(0),
                r.get::<_, String>(14)?,
                r.get::<_, Option<i64>>(15)?.unwrap_or(0),
                r.get::<_, Option<i64>>(16)?.unwrap_or(0),
            ))
        })
        .ok()?;

    let mut today_spend = 0.0;
    let mut month_spend = 0.0;
    for row in rows.flatten() {
        let (platform, model, upstream, served, ts, input, output, cr, cw, cw1h, semantics, reasoning, tool) = row;
        let at = chrono::DateTime::from_timestamp_millis(ts).unwrap_or_else(chrono::Utc::now);
        let res = resolver.price_request(&PriceRequest {
            platform: &platform,
            model: model.as_deref(),
            upstream_model: upstream.as_deref(),
            served_model: served.as_deref(),
            connection_id: Some(connection_id),
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
        if let Some(cost) = res.cost {
            month_spend += cost;
            if ts >= today_start {
                today_spend += cost;
            }
        }
    }

    if let Some(lim) = daily_limit.filter(|l| today_spend >= *l) {
        return Some(format!("连接「{name}」今日花费已达预算上限（${today_spend:.2} / ${lim:.2}）"));
    }
    if let Some(lim) = monthly_limit.filter(|l| month_spend >= *l) {
        return Some(format!("连接「{name}」本月花费已达预算上限（${month_spend:.2} / ${lim:.2}）"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_status_tracks_api_only_and_gateway_blocks_when_exceeded() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let now = Local::now().timestamp_millis();
        db.execute(
            "INSERT INTO connections(id,platform,kind,name,status,created_ms,billing_mode,price_multiplier,serve_enabled,budget_daily_usd,budget_monthly_usd,budget_action)
             VALUES('c-api','claude','api','API 主号','connected',1,'api',1.0,1,2.0,10.0,'block'),
                   ('c-sub','claude','auth','订阅号','connected',1,'subscription',1.0,0,1.0,5.0,'alert')",
            [],
        )
        .unwrap();
        // 1M input on claude-sonnet-4-5 = $3.00 → exceeds $2.00 daily limit on c-api
        db.execute(
            "INSERT INTO requests(platform,source,dedup_key,ts,model,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,total_tokens,connection_id)
             VALUES('claude','local-session','k1',?1,'claude-sonnet-4-5',1000000,0,0,0,1000000,'c-api'),
                   ('claude','local-session','k2',?1,'claude-sonnet-4-5',1000000,0,0,0,1000000,'c-sub')",
            params![now - 1_000],
        )
        .unwrap();

        let mut settings = Settings::default();
        settings.global_budget_enabled = true;
        settings.global_budget_monthly_usd = Some(10.0);
        settings.global_budget_warn_percent = 80;

        let status = compute_budget_status(&db, &settings, now).unwrap();
        assert!((status.global_today_spend - 3.0).abs() < 0.01, "全局预算只算实付 API，不含订阅等价");
        assert_eq!(status.exceeded_connections, 1);
        assert!(status.has_any_budget);

        let blocked = check_gateway_connection_budget(&db, "c-api", now);
        assert!(blocked.is_some(), "开启 block 且超限的连接应被网关拦截");
        assert!(check_gateway_connection_budget(&db, "c-sub", now).is_none(), "alert 模式连接不拦截");
    }

    #[test]
    fn official_cost_snapshots_aggregate_by_day_range() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let now = Local::now().timestamp_millis();
        record_official_cost(&db, "c1", 12.34, "Claude Admin API", now).unwrap();
        let map = load_official_costs_in_range(&db, now - 3_600_000, now + 3_600_000).unwrap();
        assert!((map["c1"].cost_usd - 12.34).abs() < 1e-6);
    }
}
