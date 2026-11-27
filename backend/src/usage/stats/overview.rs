use std::collections::{BTreeMap, HashMap};
use chrono::{Local, TimeZone};
use rusqlite::Connection;
use serde::Serialize;
use crate::db::EXCLUDE_DEGENERATE_SQL;
use super::*;

#[derive(Debug, Serialize)]
pub struct ModelRow {
    pub platform: String,
    pub model: Option<String>,
    pub requests: i64,
    /// 组内**全部**记录都有 Token 时的合计；任何一条未知就是 None（未知不当零）
    pub tokens: Option<i64>,
    /// 已知部分的合计：即使 `tokens` 为 None 也有值。取消/失败的请求常常没有 Token，
    /// 一条未知就让整组变成破折号，会把这个模型的真实用量藏起来。
    /// 全部未知时为 0，界面须配合 `known_requests` 判断，不能直接当 0 显示。
    pub known_tokens: i64,
    /// 有 Token 的请求数（`known_tokens` 覆盖了多少条）
    pub known_requests: i64,
    pub cache_hit_rate: Option<f64>,
    /// 只统计带耗时的请求（经本地代理转发）；没有则为 None
    pub avg_duration_ms: Option<f64>,
    /// 已知可计价部分的估算小计；0 条可计价时为 None
    pub cost: Option<f64>,
    /// 该模型已计价请求条数，配合 `requests` 表达部分覆盖
    pub priced_requests: i64,
    /// 价目出处与核对日期；本模型未收录价目时为 None（4.4）
    pub price_source: Option<String>,
    pub price_checked: Option<String>,
    /// 该模型命中的统一非 1.0 连接倍率（如中转 0.2×），用于分析表微标提示
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiplier_hint: Option<f64>,
}

/// 按归因连接（账号）维度的费用与积分汇总（R1-48）。
#[derive(Debug, Serialize)]
pub struct ConnectionCostRow {
    pub connection_id: Option<String>,
    pub name: String,
    pub platform: String,
    pub billing_mode: String,
    pub price_multiplier: f64,
    pub requests: i64,
    pub priced_requests: i64,
    pub known_tokens: i64,
    pub cost: f64,
    pub credits: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub official_cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub official_synced_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_daily_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_monthly_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ProviderRow {
    pub platform: String,
    pub requests: i64,
    /// 组内**全部**记录都有 Token 时的合计；任何一条未知就是 None（未知不当零）
    pub tokens: Option<i64>,
    /// 已知部分的合计：即使 `tokens` 为 None 也有值，供界面显示已知小计。
    /// 全部未知时为 0，但配套的 `known_requests` 也是 0，界面必须据此显示「—」而不是 0。
    pub known_tokens: i64,
    /// 有 Token 的请求数（`known_tokens` 覆盖了多少条）
    pub known_requests: i64,
    pub cost: CostSummary,
}

/// 按已存连接分解的一行（4.2 灵动岛「今日按来源」）。
#[derive(Debug, Serialize, PartialEq)]
pub struct SourceRow {
    pub connection_id: String,
    /// 连接名；连接已删除时为 None（界面显示「已删除的连接」）
    pub name: Option<String>,
    pub requests: i64,
    /// 已知 Token 之和；该连接的记录全部未知时为 None，不当作 0
    pub tokens: Option<i64>,
}

/// 区间内按连接分解：只认网关 / 本地代理写入的精确归属（`requests.connection_id`），
/// 没有归属的记录单独计数，不并进任何一行。
#[derive(Debug, Serialize, PartialEq)]
pub struct SourceBreakdown {
    pub rows: Vec<SourceRow>,
    pub total_requests: i64,
    pub unattributed_requests: i64,
}

///
/// Token 用**新鲜口径**并排除退化行（[`crate::db::FRESH_TOKENS_SQL`]），与同一张灵动岛卡片上的
/// 「本地今日 Token」（`today_fresh_tokens`）一致；以前用含缓存的总量，各来源相加远大于底部
/// 合计，用户无法对账（P4 审计 D6）。只有灵动岛用这个查询。
pub fn source_breakdown(conn: &Connection, platform: Option<&str>, start_ms: i64, end_ms: i64) -> rusqlite::Result<SourceBreakdown> {
    let fresh = crate::db::FRESH_TOKENS_SQL;
    let mut stmt = conn.prepare(&format!(
        "SELECT r.connection_id, c.name, r.n, r.fresh
         FROM (SELECT connection_id, COUNT(*) AS n,
                      SUM(CASE WHEN total_known = 1 THEN {fresh} END) AS fresh
               FROM requests
               WHERE ts >= ?1 AND ts < ?2 AND (?3 IS NULL OR platform = ?3){EXCLUDE_DEGENERATE_SQL}
               GROUP BY connection_id) r
         LEFT JOIN connections c ON c.id = r.connection_id"
    ))?;
    let mut rows = Vec::new();
    let (mut total, mut unattributed) = (0, 0);
    let mapped = stmt.query_map(rusqlite::params![start_ms, end_ms, platform], |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<i64>>(3)?))
    })?;
    for row in mapped {
        let (id, name, requests, tokens) = row?;
        total += requests;
        match id {
            Some(connection_id) => rows.push(SourceRow { connection_id, name, requests, tokens }),
            None => unattributed += requests,
        }
    }
    rows.sort_by(|a, b| b.tokens.unwrap_or(-1).cmp(&a.tokens.unwrap_or(-1)).then(b.requests.cmp(&a.requests)));
    Ok(SourceBreakdown { rows, total_requests: total, unattributed_requests: unattributed })
}

#[derive(Debug, Serialize)]
pub struct SessionRow {
    pub platform: String,
    pub session_id: String,
    pub title: Option<String>,
    /// 范围内已知 Token 之和（只算有 Token 的记录）
    pub tokens: i64,
    /// 范围内没有提供 Token 的记录数；大于 0 时 `tokens` 只是已知部分（P4 审计 D11）
    pub unknown_requests: i64,
    /// 项目目录：从会话来源文件读取（D-30，不改用量表）；来源文件缺失或没写 cwd 时为 None
    pub cwd: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenCoverage {
    /// 仅汇总有真实 Token 总量的记录；全部未知时为 None，不将未知记为零。
    pub known_total: Option<i64>,
    pub known_requests: i64,
    /// 命中率仅使用同时提供缓存读取与完整输入口径的请求。
    pub cache_hit_rate: Option<f64>,
    pub cache_requests: i64,
}

#[derive(Debug, Serialize)]
pub struct Overview {
    pub tokens: TokenSplit,
    pub token_coverage: TokenCoverage,
    /// 来源元数据（P1-4）：本机记录 / 网关记录 / 混合、数据最新时间、现存最早记录。
    /// 本机采集只能说明「已采到的部分」，覆盖程度恒为 `as_recorded`。
    pub provenance: crate::data_contract::Provenance,
    pub requests: i64,
    /// 上游错误数。**不含取消**：取消单独计在 `cancelled`。
    pub failed: i64,
    /// 原生来源明确标记的取消数。与 `failed` 互斥，两者都不算进成功率分子。
    pub cancelled: i64,
    pub unknown: i64,
    pub cache_hit_rate: Option<f64>,
    pub cost: CostSummary,
    /// 紧邻的上一段同长度区间的总 Token；没有记录或总量未知时为 None
    pub previous_total: Option<i64>,
    pub trend: Trend,
    pub models: Vec<ModelRow>,
    pub providers: Vec<ProviderRow>,
    pub connections: Vec<ConnectionCostRow>,
    /// 能归属到某个连接的请求数
    pub attributed: i64,
    pub sessions: Vec<SessionRow>,
}


pub fn models(reqs: &[Req]) -> Vec<ModelRow> {
    let mut groups: BTreeMap<(String, Option<String>), Vec<&Req>> = BTreeMap::new();
    for r in reqs {
        groups.entry((r.platform.clone(), r.model.clone())).or_default().push(r);
    }
    let mut rows: Vec<ModelRow> = groups
        .into_iter()
        .map(|((platform, model), group)| {
            let durations: Vec<i64> = group.iter().filter_map(|r| r.duration_ms).collect();
            let priced = cost(group.iter().copied());
            let sample_price = group.iter().find_map(|r| r.price.as_ref());
            let (price_source, price_checked) = match sample_price {
                Some(p) => (
                    Some(p.source.clone()),
                    (p.source_kind == "builtin").then(|| crate::pricing::verified_on().to_string()),
                ),
                None => match model.as_deref().and_then(|name| crate::pricing::price_source(Some(platform.as_str()), name)) {
                    Some((src, date)) => (Some(src), (!date.is_empty()).then(|| date.to_string())),
                    None => (None, None),
                },
            };
            let (known_tokens, unknown) = known_subtotal(group.iter().copied(), |r| r.total);
            let mut mults = group
                .iter()
                .filter_map(|r| r.price.as_ref().map(|p| p.multiplier))
                .filter(|m| (*m - 1.0).abs() > 1e-6);
            let multiplier_hint = mults.next().filter(|first| mults.all(|m| (m - *first).abs() < 1e-6));
            let cache_known: Vec<&Req> = group
                .iter()
                .copied()
                .filter(|r| r.cache_read.is_some() && r.hit_denominator.is_some())
                .collect();
            ModelRow {
                price_source,
                price_checked,
                multiplier_hint,
                platform,
                model,
                requests: group.len() as i64,
                tokens: sum_known(group.iter().copied(), |r| r.total, Some(0)),
                known_tokens,
                known_requests: group.len() as i64 - unknown,
                cache_hit_rate: hit_rate(cache_known.iter().copied()),
                avg_duration_ms: (!durations.is_empty()).then(|| durations.iter().sum::<i64>() as f64 / durations.len() as f64),
                cost: (priced.priced_requests > 0).then_some(priced.amount),
                priced_requests: priced.priced_requests,
            }
        })
        .collect();
    rows.sort_by(|a, b| b.known_tokens.cmp(&a.known_tokens).then(b.requests.cmp(&a.requests)));
    rows
}

pub fn providers(reqs: &[Req]) -> Vec<ProviderRow> {
    let mut groups: HashMap<&str, Vec<&Req>> = HashMap::new();
    for r in reqs {
        groups.entry(r.platform.as_str()).or_default().push(r);
    }
    let mut rows: Vec<ProviderRow> = groups
        .into_iter()
        .map(|(platform, group)| {
            // 组里只要有任意一条 Token 未知，`sum_known` 就是 None（未知不能当零）。
            // 但已知那部分仍然是真实的：`known_tokens` / `known_requests` 让界面显示
            // 「已知 754,323,790，覆盖 1594 / 1599」，而不是只给一个破折号。
            let (known_tokens, unknown) = known_subtotal(group.iter().copied(), |r| r.total);
            ProviderRow {
                platform: platform.to_string(),
                requests: group.len() as i64,
                tokens: sum_known(group.iter().copied(), |r| r.total, Some(0)),
                known_tokens,
                known_requests: group.len() as i64 - unknown,
                cost: cost(group.iter().copied()),
            }
        })
        .collect();
    rows.sort_by(|a, b| b.known_tokens.cmp(&a.known_tokens).then(b.requests.cmp(&a.requests)).then(a.platform.cmp(&b.platform)));
    rows
}

pub fn top_sessions(conn: &Connection, reqs: &[Req], limit: usize) -> rusqlite::Result<Vec<SessionRow>> {
    // 按共享契约累加：未知不当 0 混进合计，而是单独计数；只能按已知部分排名，
    // 一条已知都没有的会话无从比较消耗，不进榜（不是因为它是 0）
    let mut groups: HashMap<(&str, &str), Vec<Option<i64>>> = HashMap::new();
    for r in reqs {
        if let Some(id) = r.session_id.as_deref() {
            groups.entry((r.platform.as_str(), id)).or_default().push(r.total);
        }
    }
    let mut ranked: Vec<_> = groups
        .into_iter()
        .map(|(key, values)| (key, crate::data_contract::Coverage::of(values)))
        .filter(|(_, coverage)| coverage.total().is_some_and(|t| t > 0))
        .map(|(key, coverage)| (key, (coverage.total().unwrap_or(0), coverage.unknown())))
        .collect();
    ranked.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(&b.0)));
    ranked.truncate(limit);
    let mut meta = conn.prepare("SELECT title, cwd FROM sessions WHERE platform = ?1 AND session_id = ?2")?;
    // 只对排名后的几行读 cwd；read_cwd 按文件修改时间缓存，不会每次重读源文件
    let files = if ranked.is_empty() { HashMap::new() } else { crate::sessions::source::source_files(conn)? };
    ranked
        .into_iter()
        .map(|((platform, session_id), (tokens, unknown_requests))| {
            let found: Option<(Option<String>, Option<String>)> = meta
                .query_row(rusqlite::params![platform, session_id], |r| Ok((r.get(0)?, r.get(1)?)))
                .map(Some)
                .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e) })?;
            let (title, stored_cwd) = found.unwrap_or((None, None));
            let cwd = stored_cwd
                .filter(|c| !c.trim().is_empty())
                .or_else(|| files.get(&(platform.to_string(), session_id.to_string())).and_then(|p| crate::sessions::source::read_cwd(p)));
            Ok(SessionRow {
                platform: platform.to_string(),
                session_id: session_id.to_string(),
                title: title.filter(|t| !t.trim().is_empty()),
                tokens,
                unknown_requests,
                cwd,
            })
        })
        .collect()
}

/// 原生来源（ZCode / Qoder / WorkBuddy 等）覆盖不完整，没有记录不等于真实的 0
pub fn empty_value(f: &StatsFilter) -> Option<i64> {
    if f.platform.as_deref().is_some_and(crate::platforms::native) { None } else { Some(0) }
}

pub fn connection_costs(
    conn: &Connection,
    reqs: &[Req],
    official: &HashMap<String, crate::pricing::budget::OfficialCostSum>,
) -> Vec<ConnectionCostRow> {
    let mut meta: HashMap<String, (String, String, Option<f64>, Option<f64>)> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, name, platform, budget_daily_usd, budget_monthly_usd FROM connections",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<f64>>(3)?,
                r.get::<_, Option<f64>>(4)?,
            ))
        }) {
            for row in rows.flatten() {
                meta.insert(row.0, (row.1, row.2, row.3, row.4));
            }
        }
    }

    let mut groups: HashMap<Option<String>, Vec<&Req>> = HashMap::new();
    for r in reqs {
        let key = r
            .attribution
            .is_connection()
            .then(|| r.attribution.connection_id.clone())
            .flatten();
        groups.entry(key).or_default().push(r);
    }

    let mut rows: Vec<ConnectionCostRow> = groups
        .into_iter()
        .map(|(cid, group)| {
            let priced = cost(group.iter().copied());
            let (known_tokens, _) = known_subtotal(group.iter().copied(), |r| r.total);
            let first = group[0];
            let (name, platform, budget_daily_usd, budget_monthly_usd) = match cid.as_deref() {
                Some(id) => match meta.get(id) {
                    Some((n, p, d, m)) => (n.clone(), p.clone(), *d, *m),
                    None => (
                        first.attribution.name.clone().unwrap_or_else(|| "已删除的连接".into()),
                        first.platform.clone(),
                        None,
                        None,
                    ),
                },
                None => ("未归属本机日志".into(), first.platform.clone(), None, None),
            };
            let off = cid.as_deref().and_then(|id| official.get(id));
            ConnectionCostRow {
                connection_id: cid,
                name,
                platform,
                billing_mode: first.billing_mode.as_str().to_string(),
                price_multiplier: first.price_multiplier,
                requests: group.len() as i64,
                priced_requests: priced.priced_requests,
                known_tokens,
                cost: priced.amount,
                credits: priced.credits,
                official_cost_usd: off.map(|o| o.cost_usd),
                official_synced_ms: off.map(|o| o.synced_ms),
                budget_daily_usd,
                budget_monthly_usd,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.cost
            .total_cmp(&a.cost)
            .then_with(|| b.credits.total_cmp(&a.credits))
            .then_with(|| b.requests.cmp(&a.requests))
    });
    rows
}

pub fn overview(conn: &Connection, f: &StatsFilter) -> rusqlite::Result<Overview> {
    let mut sub_ref_amount: Option<f64> = None;
    let mut sub_ref_credits: Option<f64> = None;
    let reqs = if f.billing_scope.as_deref() == Some("api") {
        let all = load(conn, &StatsFilter { billing_scope: None, ..f.clone() }, false, true)?;
        let (mut sa, mut sc, mut any_sub) = (0.0, 0.0, false);
        let mut kept = Vec::with_capacity(all.len());
        for r in all {
            if r.billing_mode == crate::pricing::BillingMode::Api {
                kept.push(r);
            } else {
                if r.billing_mode == crate::pricing::BillingMode::Subscription {
                    if let Some(c) = r.cost {
                        sa += c;
                        any_sub = true;
                    }
                }
                if let Some(cr) = r.credits.filter(|v| v.is_finite() && *v > 0.0) {
                    sc += cr;
                    any_sub = true;
                }
            }
        }
        if any_sub {
            sub_ref_amount = Some(sa);
            sub_ref_credits = Some(sc);
        }
        kept
    } else {
        load(conn, f, false, true)?
    };
    let empty = empty_value(f);
    let (previous_start, previous_end) = previous_window(f.start, f.end);
    let previous = load(conn, &f.with_range(previous_start, previous_end), false, false)?;
    let cache_known: Vec<&Req> = reqs.iter().filter(|r| r.cache_read.is_some() && r.hit_denominator.is_some()).collect();
    let official = crate::pricing::budget::load_official_costs_in_range(conn, f.start, f.end).unwrap_or_default();
    let connection_rows = connection_costs(conn, &reqs, &official);
    let mut summary = cost(reqs.iter());
    summary.subscription_ref_amount = sub_ref_amount;
    summary.subscription_ref_credits = sub_ref_credits;
    // 仅汇总落在当前可见连接集内的官方实账快照，保持口径一致
    let mut off_sum = 0.0;
    let mut off_synced: Option<i64> = None;
    let mut off_any = false;
    for row in &connection_rows {
        if let Some(v) = row.official_cost_usd {
            off_sum += v;
            off_any = true;
            if let Some(s) = row.official_synced_ms {
                off_synced = Some(off_synced.map_or(s, |prev| prev.max(s)));
            }
        }
    }
    if off_any {
        summary.official_cost_usd = Some(off_sum);
        summary.official_synced_ms = off_synced;
    }
    Ok(Overview {
        provenance: provenance(conn, f)?,
        tokens: split(reqs.iter(), empty),
        token_coverage: {
            let coverage = crate::data_contract::Coverage::of(reqs.iter().map(|r| r.total));
            TokenCoverage {
                known_total: coverage.total(),
                known_requests: coverage.known(),
                cache_hit_rate: hit_rate(cache_known.iter().copied()),
                cache_requests: cache_known.len() as i64,
            }
        },
        requests: reqs.len() as i64,
        failed: reqs.iter().filter(|r| r.failed).count() as i64,
        cancelled: reqs.iter().filter(|r| r.cancelled).count() as i64,
        unknown: reqs.iter().filter(|r| r.unknown).count() as i64,
        cache_hit_rate: hit_rate(reqs.iter()),
        cost: summary,
        previous_total: if previous.is_empty() { None } else { sum_known(previous.iter(), |r| r.total, None) },
        trend: trend(&reqs, f.start, f.end),
        models: models(&reqs),
        providers: providers(&reqs),
        connections: connection_rows,
        attributed: count_attributed(conn, f)?,
        sessions: top_sessions(conn, &reqs, 5)?,
    })
}

/// 对比用的上一周期：按**整天**往前平移，长度与本期相同。
///
/// 本期是「今天 00:00 → 此刻」时，上期是「昨天 00:00 → 昨天此刻」，与界面文案「较昨天同时段」一致。
/// 以前取 `[start - span, start)`，今天上午 10 点对比的是昨天 14:00–24:00（P4 审计 D8）。
/// 已结束的整天区间（span 恰为整天）平移结果与旧算法相同。
pub fn previous_window(start: i64, end: i64) -> (i64, i64) {
    const DAY_MS: i64 = 86_400_000;
    let span = (end - start).max(0);
    let days = ((span + DAY_MS - 1) / DAY_MS).max(1) as u64;
    // 按本地日历减天数（夏令时切换日也落在同一钟点）；时区异常时退回固定 24 小时
    let previous_start = Local
        .timestamp_millis_opt(start)
        .single()
        .and_then(|at| at.checked_sub_days(chrono::Days::new(days)))
        .map(|at| at.timestamp_millis())
        .unwrap_or(start - days as i64 * DAY_MS);
    (previous_start, previous_start + span)
}

/// 概览的来源元数据：与统计本身用同一组过滤条件（含退化行排除），只是另取一次「现存最早」。
pub fn provenance(conn: &Connection, f: &StatsFilter) -> rusqlite::Result<crate::data_contract::Provenance> {
    let (range, range_args) = where_clause(f, false);
    let (scope, scope_args) = where_clause(&f.with_range(i64::MIN, i64::MAX), false);
    crate::data_contract::requests_provenance(
        conn,
        f.start,
        f.end,
        &range,
        &range_args,
        &scope,
        &scope_args,
        Local::now().timestamp_millis(),
    )
}

pub fn count_attributed(conn: &Connection, f: &StatsFilter) -> rusqlite::Result<i64> {
    let (clause, args) = where_clause(f, false);
    conn.query_row(
        &format!("SELECT COUNT(*) FROM requests r WHERE {clause} AND r.connection_id IS NOT NULL"),
        rusqlite::params_from_iter(args),
        |r| r.get(0),
    )
}
