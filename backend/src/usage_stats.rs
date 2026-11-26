//! 用量统计页（Jolu.pen 模块 C，画板 30–39）的跨供应商聚合查询。
//!
//! 与 db.rs 的单平台查询口径一致：
//! - 普通输入已扣除单独计量的缓存读取（Codex 的 input 含缓存，Claude 的不含）；
//! - 任一请求缺某个分项，区间内该分项就是未知（None），不补零；
//! - 费用只汇总能精确计价的请求，同时返回已计价条数，界面据此说明覆盖率；
//! - 统计聚合跳过退化行（[`EXCLUDE_DEGENERATE_SQL`]），调用明细照常列出。
//!
//! 一次查询把区间内的请求读进内存再分组：本机记录量级（一年数万条）下比多次 SQL 扫描更简单，
//! 也让概览、分析两个页签共用同一份口径。

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};
use rusqlite::{types::Value, Connection};
use serde::{Deserialize, Serialize};

use crate::db::EXCLUDE_DEGENERATE_SQL;

/// 账号筛选里的「未归属账号」：没有经本地代理归账到任何连接的本机记录
pub const UNATTRIBUTED: &str = "__unattributed__";

/// 两次请求间隔不超过 5 分钟视为连续活跃，用来估算今日活跃时长
const ACTIVE_GAP_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StatsFilter {
    /// [start, end) 的 UTC 毫秒
    pub start: i64,
    pub end: i64,
    #[serde(default)]
    pub platform: Option<String>,
    /// 空表示全部账号；可包含 [`UNATTRIBUTED`]
    #[serde(default)]
    pub connections: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// success / error / cancelled / unknown；仅调用明细使用，各状态互斥。
    #[serde(default)]
    pub status: Option<String>,
    /// 调用 ID / 会话 ID 片段；仅调用明细使用
    #[serde(default)]
    pub search: Option<String>,
    /// 计费口径筛选：`all`（默认）| `api`（仅实付 API）| `subscription`（仅订阅等价）
    #[serde(default)]
    pub billing_scope: Option<String>,
}

impl StatsFilter {
    pub fn validate(&self) -> Result<(), String> {
        if self.start >= self.end {
            return Err("日期范围无效".into());
        }
        if let Some(p) = &self.platform {
            if !crate::platforms::known(p) {
                return Err("未知平台".into());
            }
        }
        if self.status.as_deref().is_some_and(|s| !matches!(s, "success" | "error" | "cancelled" | "unknown")) {
            return Err("未知状态筛选".into());
        }
        if self.billing_scope.as_deref().is_some_and(|s| !matches!(s, "all" | "api" | "subscription")) {
            return Err("未知计费口径筛选".into());
        }
        Ok(())
    }

    fn with_range(&self, start: i64, end: i64) -> Self {
        Self { start, end, ..self.clone() }
    }
}

/// 拼 WHERE 子句。`calls = false` 时只用区间、供应商、账号与模型——概览和分析不受状态 / 搜索影响。
fn where_clause(f: &StatsFilter, calls: bool) -> (String, Vec<Value>) {
    let mut sql = String::from("r.ts >= ? AND r.ts < ?");
    let mut args = vec![Value::Integer(f.start), Value::Integer(f.end)];
    if let Some(p) = &f.platform {
        sql.push_str(" AND r.platform = ?");
        args.push(Value::Text(p.clone()));
    }
    if !f.connections.is_empty() {
        let ids: Vec<&String> = f.connections.iter().filter(|c| c.as_str() != UNATTRIBUTED).collect();
        let mut parts = Vec::new();
        if !ids.is_empty() {
            parts.push(format!("r.connection_id IN ({})", vec!["?"; ids.len()].join(",")));
            args.extend(ids.into_iter().map(|id| Value::Text(id.clone())));
        }
        if f.connections.iter().any(|c| c == UNATTRIBUTED) {
            parts.push("r.connection_id IS NULL".into());
        }
        sql.push_str(&format!(" AND ({})", parts.join(" OR ")));
    }
    if let Some(m) = &f.model {
        sql.push_str(" AND r.model = ?");
        args.push(Value::Text(m.clone()));
    }
    if calls {
        match f.status.as_deref() {
            // 成功 = 既不是失败也不是取消。取消是独立一档，不能靠「不是失败」蒙进成功里。
            Some("success") => sql.push_str(&format!(" AND NOT ({FAILED_SQL}) AND NOT ({CANCELLED_SQL}) AND NOT ({UNKNOWN_SQL})")),
            Some("error") => sql.push_str(&format!(" AND ({FAILED_SQL})")),
            Some("cancelled") => sql.push_str(&format!(" AND ({CANCELLED_SQL})")),
            Some("unknown") => sql.push_str(&format!(" AND ({UNKNOWN_SQL})")),
            _ => {}
        }
        if let Some(q) = f.search.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            sql.push_str(" AND (instr(r.dedup_key, ?) > 0 OR instr(COALESCE(r.session_id, ''), ?) > 0)");
            args.push(Value::Text(q.to_string()));
            args.push(Value::Text(q.to_string()));
        }
    } else {
        sql.push_str(EXCLUDE_DEGENERATE_SQL);
    }
    (sql, args)
}

/// 失败请求：上游返回非 2xx，或原生来源明确标记为 `failed`。
///
/// 没有状态码的本机记录按成功处理（能写入用量即已返回）。
///
/// **取消不算失败（有意的口径）**：`native_outcome='cancelled'` 是用户主动中止
/// 或客户端收尾，不是上游错误。ZCode 的 `error` 记 `failed`、`cancelled` 记
/// `cancelled`（见 `native_sources::zcode_native_outcome`），所以这条 SQL 只认
/// `failed`，取消的请求既不进「失败数」，也不进「成功」筛选（[`CANCELLED_SQL`] 单独筛）。
/// 若将来把取消并进失败，`Overview.failed`、成功率与 CSV 的「状态」列都会同时变化，
/// 届时必须同步修改 `usage_stats.test` 里的口径测试。
const FAILED_SQL: &str = "(COALESCE(r.native_outcome, '') <> 'cancelled') AND ((r.status_code IS NOT NULL AND (r.status_code < 200 OR r.status_code >= 300)) OR COALESCE(r.native_outcome, '') = 'failed')";

/// 被取消的请求：只有原生来源会明确给出这一档。
/// 取优先级高于 [`FAILED_SQL`]：取消发生在客户端，上游可能还没给出状态码，
/// 也可能留下一个非 2xx（例如中止导致连接断开被记为 5xx）。同一行的三档必须互斥，
/// 否则「成功 + 失败 + 取消」会大于总调用数，成功率被扣两次。
const CANCELLED_SQL: &str = "COALESCE(r.native_outcome, '') = 'cancelled'";
const UNKNOWN_SQL: &str = "COALESCE(r.native_outcome, '') = 'unknown' AND (r.status_code IS NULL OR (r.status_code >= 200 AND r.status_code < 300))";

/// 某次请求命中的单价快照（每百万 token，USD）。仅用于界面展示拆解，金额以 `cost` 为准。
#[derive(Debug, Clone, Serialize)]
pub struct PriceSnapshot {
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
    pub cache_write: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_1h: Option<f64>,
    /// 价目来源（官方定价页、`models.dev/...` 或自定义/映射说明）
    pub source: String,
    pub source_kind: String,
    pub pricing_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias_from: Option<String>,
    pub multiplier: f64,
    pub billing_mode: String,
    /// 是否命中长上下文加价
    pub long_context: bool,
    /// 是否命中谷时折价
    pub off_peak: bool,
}

/// 一条请求的统计视图；`None` 表示来源没提供
#[derive(Debug, Clone)]
struct Req {
    id: i64,
    call_id: String,
    source: String,
    platform: String,
    model: Option<String>,
    upstream_model: Option<String>,
    served_model: Option<String>,
    effective_model: Option<String>,
    ts: i64,
    session_id: Option<String>,
    fresh_input: Option<i64>,
    output: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
    cache_write_1h: i64,
    reasoning: Option<i64>,
    tool: Option<i64>,
    total: Option<i64>,
    hit_denominator: Option<i64>,
    cost: Option<f64>,
    credits: Option<f64>,
    billing_mode: crate::pricing::BillingMode,
    price_multiplier: f64,
    unpriced_reason: Option<&'static str>,
    /// 该次请求命中的单价（每百万 token，USD），用于界面的费用拆解；无价目时为 None
    price: Option<PriceSnapshot>,
    effort: Option<String>,
    duration_ms: Option<i64>,
    first_token_ms: Option<i64>,
    status_code: Option<i64>,
    /// 上游错误（非 2xx）或原生来源给出 `failed`。**不包含**取消，见 [`FAILED_SQL`]。
    failed: bool,
    /// 原生来源明确标记 `cancelled`。取消不计入失败，也不计入成功。
    cancelled: bool,
    unknown: bool,
    attribution: crate::attribution::AttributionView,
}

fn load(conn: &Connection, f: &StatsFilter, calls: bool, need_pricing: bool) -> rusqlite::Result<Vec<Req>> {
    let (clause, args) = where_clause(f, calls);
    let sql = format!(
        "SELECT r.id, r.dedup_key, r.platform, r.model, r.ts, r.session_id,
                r.input_tokens, r.output_tokens, r.cache_read_tokens, r.cache_write_tokens, r.total_tokens,
                r.input_known, r.output_known, r.cache_read_known, r.cache_write_known, r.total_known,
                r.input_semantics, r.effort, r.duration_ms, r.first_token_ms, r.status_code, r.native_outcome,
                m.reasoning, m.tool, r.connection_id, r.provider_key,
                r.source, r.upstream_model, r.served_model, r.cache_write_1h_tokens, m.credits
         FROM requests r LEFT JOIN request_metrics m ON m.request_id = r.id
         WHERE {clause}
         ORDER BY r.ts DESC, r.id DESC"
    );
    let snapshot = crate::attribution::Snapshot::load(conn).ok();
    let mut resolver = crate::pricing::PriceResolver::load(conn);
    let billing_scope = f.billing_scope.as_deref();
    let mut stmt = conn.prepare(&sql)?;
    let raw_rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
        let known = |i: usize| -> rusqlite::Result<bool> { Ok(r.get::<_, i64>(i)? != 0) };
        let value = |i: usize, k: usize| -> rusqlite::Result<Option<i64>> {
            Ok(if known(k)? { Some(r.get(i)?) } else { None })
        };
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, Option<String>>(5)?,
            value(6, 11)?,
            value(7, 12)?,
            value(8, 13)?,
            value(9, 14)?,
            value(10, 15)?,
            r.get::<_, String>(16)?,
            r.get::<_, Option<String>>(17)?,
            r.get::<_, Option<i64>>(18)?,
            r.get::<_, Option<i64>>(19)?,
            r.get::<_, Option<i64>>(20)?,
            r.get::<_, Option<String>>(21)?,
            r.get::<_, Option<i64>>(22)?,
            r.get::<_, Option<i64>>(23)?,
            r.get::<_, Option<String>>(24)?,
            r.get::<_, Option<String>>(25)?,
            r.get::<_, String>(26)?,
            r.get::<_, Option<String>>(27)?,
            r.get::<_, Option<String>>(28)?,
            r.get::<_, Option<i64>>(29)?.unwrap_or(0),
            r.get::<_, Option<f64>>(30)?,
        ))
    })?;

    let mut out = Vec::new();
    for raw in raw_rows {
        let (
            id, call_id, platform, model, ts, session_id,
            input, output, cache_read, cache_write, total,
            semantics, effort, duration_ms, first_token_ms, status_code, outcome,
            reasoning, tool, connection_id, provider_key,
            source, upstream_model, served_model, cache_write_1h, credits,
        ) = raw?;

        let attribution = snapshot
            .as_ref()
            .map_or_else(crate::attribution::AttributionView::platform, |snap| {
                snap.resolve(&platform, connection_id.as_deref(), provider_key.as_deref(), model.as_deref())
            });
        // Exact 与 Mapped（唯一命中）都套用连接计费口径与倍率；Platform 档不猜测连接
        let effective_conn_id = attribution
            .is_connection()
            .then(|| attribution.connection_id.as_deref())
            .flatten();

        let (billing_mode, price_multiplier) = resolver.billing_for(effective_conn_id);
        match billing_scope {
            Some("api") if billing_mode != crate::pricing::BillingMode::Api => continue,
            Some("subscription") if billing_mode != crate::pricing::BillingMode::Subscription => continue,
            _ => {}
        }

        let fresh_input = match semantics.as_str() {
            "includes_cache" => input.zip(cache_read).map(|(i, c)| (i - c).max(0)),
            "excludes_cache" => input,
            _ => None,
        };
        let hit_denominator = match semantics.as_str() {
            "includes_cache" => input.zip(cache_read).map(|(i, _)| i),
            // 不强制要求 cache_write 必须已知：Antigravity / DSH 等原生来源无缓存写字段，
            // 缺省按 0 参与分母，避免缓存命中率恒为 None。
            "excludes_cache" => input.zip(cache_read).map(|(i, r)| i + r + cache_write.unwrap_or(0)),
            _ => None,
        };
        let has_usage = [input, output, cache_read, cache_write, total, reasoning, tool]
            .iter()
            .any(Option::is_some);

        let (cost, price, unpriced_reason, effective_model) = if need_pricing {
            let at = chrono::DateTime::from_timestamp_millis(ts).unwrap_or_else(chrono::Utc::now);
            let res = resolver.price_request(&crate::pricing::PriceRequest {
                platform: &platform,
                model: model.as_deref(),
                upstream_model: upstream_model.as_deref(),
                served_model: served_model.as_deref(),
                connection_id: effective_conn_id,
                input_semantics: &semantics,
                input,
                output,
                cache_read,
                cache_write,
                cache_write_1h,
                reasoning: reasoning.unwrap_or(0),
                tool: tool.unwrap_or(0),
                at,
            });
            let snap = res.applied.map(|p| PriceSnapshot {
                input: p.input,
                cached_input: p.cached_input,
                output: p.output,
                cache_write: p.cache_write,
                cache_write_1h: p.cache_write_1h,
                source: p.source,
                source_kind: p.source_kind,
                pricing_model: p.pricing_model,
                alias_from: p.alias_from,
                multiplier: p.multiplier,
                billing_mode: p.billing_mode,
                long_context: p.long_context,
                off_peak: p.off_peak,
            });
            let reason = match res.unpriced_reason {
                None => None,
                Some(crate::pricing::UnpricedReason::NoPrice) => Some("no_price"),
                Some(crate::pricing::UnpricedReason::NoModel) => {
                    Some(if has_usage { "no_model" } else { "no_usage" })
                }
                Some(crate::pricing::UnpricedReason::NoUsage) => {
                    Some(if has_usage { "partial_usage" } else { "no_usage" })
                }
            };
            (res.cost, snap, reason, res.effective_model)
        } else {
            (None, None, None, model.clone())
        };

        out.push(Req {
            id,
            call_id,
            source,
            cancelled: outcome.as_deref() == Some("cancelled"),
            unknown: outcome.as_deref() == Some("unknown") && status_code.is_none_or(|c| (200..300).contains(&c)),
            failed: outcome.as_deref() != Some("cancelled")
                && (status_code.is_some_and(|c| !(200..300).contains(&c)) || outcome.as_deref() == Some("failed")),
            platform,
            model,
            upstream_model,
            served_model,
            effective_model,
            ts,
            session_id,
            fresh_input,
            output,
            cache_read,
            cache_write,
            cache_write_1h,
            reasoning,
            tool,
            total,
            hit_denominator,
            cost,
            credits,
            billing_mode,
            price_multiplier,
            unpriced_reason,
            price,
            effort,
            duration_ms,
            first_token_ms,
            status_code,
            attribution,
        });
    }
    Ok(out)
}

/// 已知值求和：任一请求缺该项则整体未知；没有请求时为 `empty`。
/// 这是刻意的保守口径——不把「部分来源的已知值」当成全量展示。
/// 供界面参考的「已知值小计」由 [`known_subtotal`] 单独提供。
fn sum_known<'a>(reqs: impl IntoIterator<Item = &'a Req>, pick: impl Fn(&Req) -> Option<i64>, empty: Option<i64>) -> Option<i64> {
    let mut any = false;
    let mut total = 0i64;
    for r in reqs {
        any = true;
        total += pick(r)?;
    }
    if any { Some(total) } else { empty }
}

/// 已知值小计 + 未知条数：当 [`sum_known`] 返回 `None` 时，界面用它展示
/// 「已知部分有多少、多少条来源未提供」，而不是只给一个破折号。
fn known_subtotal<'a>(
    reqs: impl IntoIterator<Item = &'a Req>,
    pick: impl Fn(&Req) -> Option<i64>,
) -> (i64, i64) {
    let mut known = 0i64;
    let mut unknown = 0i64;
    for r in reqs {
        match pick(r) {
            Some(value) => known += value,
            None => unknown += 1,
        }
    }
    (known, unknown)
}

fn hit_rate<'a>(reqs: impl IntoIterator<Item = &'a Req> + Clone) -> Option<f64> {
    let read = sum_known(reqs.clone(), |r| r.cache_read, None)?;
    let denominator = sum_known(reqs, |r| r.hit_denominator, None)?;
    (denominator > 0 && read <= denominator).then(|| read as f64 / denominator as f64)
}

#[derive(Debug, Serialize, PartialEq)]
pub struct TokenSplit {
    pub fresh_input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub total: Option<i64>,
    /// 分项为未知时，已知部分的合计与缺该字段的请求数。
    /// 界面据此显示「已知小计 X（N 条来源未提供）」，而不是只给破折号。
    pub fresh_input_known: Option<i64>,
    pub fresh_input_unknown: i64,
    pub output_known: Option<i64>,
    pub output_unknown: i64,
    pub cache_read_known: Option<i64>,
    pub cache_read_unknown: i64,
    pub cache_write_known: Option<i64>,
    pub cache_write_unknown: i64,
}

fn split<'a>(reqs: impl IntoIterator<Item = &'a Req> + Clone, empty: Option<i64>) -> TokenSplit {
    let coverage = |pick: &dyn Fn(&Req) -> Option<i64>| {
        let (known, unknown) = known_subtotal(reqs.clone(), pick);
        (if unknown > 0 { Some(known) } else { None }, unknown)
    };
    let (fresh_input_known, fresh_input_unknown) = coverage(&|r| r.fresh_input);
    let (output_known, output_unknown) = coverage(&|r| r.output);
    let (cache_read_known, cache_read_unknown) = coverage(&|r| r.cache_read);
    let (cache_write_known, cache_write_unknown) = coverage(&|r| r.cache_write);
    TokenSplit {
        fresh_input: sum_known(reqs.clone(), |r| r.fresh_input, empty),
        output: sum_known(reqs.clone(), |r| r.output, empty),
        cache_read: sum_known(reqs.clone(), |r| r.cache_read, empty),
        cache_write: sum_known(reqs.clone(), |r| r.cache_write, empty),
        total: sum_known(reqs, |r| r.total, empty),
        fresh_input_known,
        fresh_input_unknown,
        output_known,
        output_unknown,
        cache_read_known,
        cache_read_unknown,
        cache_write_known,
        cache_write_unknown,
    }
}

#[derive(Debug, Serialize)]
pub struct CostSummary {
    /// 已计价请求的估算合计（USD）
    pub amount: f64,
    /// 其中按 API 实付口径的估算合计（USD）
    pub api_amount: f64,
    /// 其中按订阅等价口径的估算合计（USD）
    pub subscription_amount: f64,
    pub free_requests: i64,
    pub priced_requests: i64,
    pub requests: i64,
    /// 独立计量的上游计划积分（Qoder / WorkBuddy 等），绝不折算为美元混入 amount
    pub credits: f64,
    pub credited_requests: i64,
    /// 官方 Admin API 实账对照（仅存在已同步快照时有值）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub official_cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub official_synced_ms: Option<i64>,
    /// 仅实付 API 视图下附带同周期的订阅参考（不计入实付合计）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_ref_amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_ref_credits: Option<f64>,
    /// 未计价原因细分：让用户知道是「模型没收录」还是「上游没给用量」
    pub unpriced_no_price: Vec<String>,
    pub unpriced_no_usage: i64,
    pub unpriced_partial: i64,
}

fn cost<'a>(reqs: impl IntoIterator<Item = &'a Req>) -> CostSummary {
    let mut out = CostSummary {
        amount: 0.0,
        api_amount: 0.0,
        subscription_amount: 0.0,
        free_requests: 0,
        priced_requests: 0,
        requests: 0,
        credits: 0.0,
        credited_requests: 0,
        official_cost_usd: None,
        official_synced_ms: None,
        subscription_ref_amount: None,
        subscription_ref_credits: None,
        unpriced_no_price: Vec::new(),
        unpriced_no_usage: 0,
        unpriced_partial: 0,
    };
    for r in reqs {
        out.requests += 1;
        if let Some(cr) = r.credits.filter(|v| v.is_finite() && *v > 0.0) {
            out.credits += cr;
            out.credited_requests += 1;
        }
        if let Some(c) = r.cost {
            out.amount += c;
            out.priced_requests += 1;
            match r.billing_mode {
                crate::pricing::BillingMode::Api => out.api_amount += c,
                crate::pricing::BillingMode::Subscription => out.subscription_amount += c,
                crate::pricing::BillingMode::Free => out.free_requests += 1,
            }
            continue;
        }
        match r.unpriced_reason {
            Some("no_price") => {
                if let Some(model) = r.effective_model.as_deref().or(r.model.as_deref()) {
                    let key = format!("{}:{}", r.platform, model);
                    if !out.unpriced_no_price.contains(&key) {
                        out.unpriced_no_price.push(key);
                    }
                } else {
                    out.unpriced_partial += 1;
                }
            }
            Some("no_usage") => out.unpriced_no_usage += 1,
            _ => out.unpriced_partial += 1,
        }
    }
    out.unpriced_no_price.sort();
    out
}

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

fn local_ms(date: NaiveDate) -> i64 {
    let naive = date.and_hms_opt(0, 0, 0).expect("零点总是有效");
    Local.from_local_datetime(&naive).earliest().map(|d| d.timestamp_millis()).unwrap_or_else(|| naive.and_utc().timestamp_millis())
}

fn local_date(ms: i64) -> NaiveDate {
    Local.timestamp_millis_opt(ms).single().map(|d| d.date_naive()).unwrap_or_default()
}

/// 趋势分桶：单日内按 1 小时，两天内按 2 小时，62 天内按天，更长按周；边界跟随本地时区
fn buckets(start: i64, end: i64) -> (&'static str, Vec<(i64, i64, String)>) {
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

fn trend(reqs: &[Req], start: i64, end: i64) -> Trend {
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

fn models(reqs: &[Req]) -> Vec<ModelRow> {
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

fn providers(reqs: &[Req]) -> Vec<ProviderRow> {
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

fn top_sessions(conn: &Connection, reqs: &[Req], limit: usize) -> rusqlite::Result<Vec<SessionRow>> {
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
    let files = if ranked.is_empty() { HashMap::new() } else { crate::session_threads::source_files(conn)? };
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
                .or_else(|| files.get(&(platform.to_string(), session_id.to_string())).and_then(|p| crate::session_threads::read_cwd(p)));
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
fn empty_value(f: &StatsFilter) -> Option<i64> {
    if f.platform.as_deref().is_some_and(crate::platforms::native) { None } else { Some(0) }
}

fn connection_costs(
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
fn previous_window(start: i64, end: i64) -> (i64, i64) {
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
fn provenance(conn: &Connection, f: &StatsFilter) -> rusqlite::Result<crate::data_contract::Provenance> {
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

fn count_attributed(conn: &Connection, f: &StatsFilter) -> rusqlite::Result<i64> {
    let (clause, args) = where_clause(f, false);
    conn.query_row(
        &format!("SELECT COUNT(*) FROM requests r WHERE {clause} AND r.connection_id IS NOT NULL"),
        rusqlite::params_from_iter(args),
        |r| r.get(0),
    )
}

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

#[derive(Debug, Serialize)]
pub struct CallRow {
    /// 本次请求的单价快照（用于费用拆解）；无价目时为 None
    pub price: Option<PriceSnapshot>,
    pub id: i64,
    pub call_id: String,
    pub source: String,
    pub ts: i64,
    pub platform: String,
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub served_model: Option<String>,
    pub session_id: Option<String>,
    pub fresh_input: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub cache_write_1h: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<i64>,
    pub output: Option<i64>,
    pub total: Option<i64>,
    pub cost: Option<f64>,
    pub billing_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unpriced_reason: Option<String>,
    pub effort: Option<String>,
    pub duration_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub status_code: Option<i64>,
    /// 上游错误或原生 `failed`；**不含取消**
    pub failed: bool,
    /// 原生来源明确标记的取消；与 `failed` 互斥
    pub cancelled: bool,
    pub unknown: bool,
    /// 这条调用归到的已存连接（`exact` / `mapped`）；归不出唯一结论时是 `platform` 档。
    pub attribution: crate::attribution::AttributionView,
}

impl From<Req> for CallRow {
    fn from(r: Req) -> Self {
        Self {
            price: r.price,
            id: r.id,
            call_id: r.call_id,
            source: r.source,
            ts: r.ts,
            platform: r.platform,
            model: r.model,
            upstream_model: r.upstream_model,
            served_model: r.served_model,
            session_id: r.session_id,
            fresh_input: r.fresh_input,
            cache_read: r.cache_read,
            cache_write: r.cache_write,
            cache_write_1h: r.cache_write_1h,
            reasoning: r.reasoning,
            tool: r.tool,
            output: r.output,
            total: r.total,
            cost: r.cost,
            billing_mode: r.billing_mode.as_str().to_string(),
            unpriced_reason: r.unpriced_reason.map(str::to_string),
            effort: r.effort,
            duration_ms: r.duration_ms,
            first_token_ms: r.first_token_ms,
            status_code: r.status_code,
            failed: r.failed,
            cancelled: r.cancelled,
            unknown: r.unknown,
            attribution: r.attribution,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CallPage {
    pub rows: Vec<CallRow>,
    pub total_count: i64,
    pub unpriced_count: i64,
    pub page: u32,
    pub page_size: u32,
    pub page_count: u32,
    /// 当前区间、供应商与账号下出现过的模型，供「全部模型」下拉使用
    pub models: Vec<String>,
}

pub const CALL_PAGE_SIZE: u32 = 20;

pub fn calls(conn: &Connection, f: &StatsFilter, page: u32) -> rusqlite::Result<CallPage> {
    let reqs = load(conn, f, true, true)?;
    let total_count = reqs.len() as i64;
    let page_count = (reqs.len() as u32).div_ceil(CALL_PAGE_SIZE).max(1);
    let page = page.clamp(1, page_count);
    let unpriced_count = reqs.iter().filter(|r| r.cost.is_none()).count() as i64;
    let rows = reqs
        .into_iter()
        .skip(((page - 1) * CALL_PAGE_SIZE) as usize)
        .take(CALL_PAGE_SIZE as usize)
        .map(CallRow::from)
        .collect();
    let (clause, args) = where_clause(&StatsFilter { model: None, status: None, search: None, ..f.clone() }, true);
    let mut stmt = conn.prepare(&format!("SELECT DISTINCT r.model FROM requests r WHERE {clause} AND r.model IS NOT NULL ORDER BY r.model"))?;
    let models = stmt.query_map(rusqlite::params_from_iter(args), |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(CallPage { rows, total_count, unpriced_count, page, page_size: CALL_PAGE_SIZE, page_count, models })
}

fn source_label(source: &str) -> &'static str {
    if source.starts_with("gateway") {
        "模型路由"
    } else if source.starts_with("native:") {
        "原生采集"
    } else {
        "本机日志"
    }
}

fn billing_mode_label(mode: crate::pricing::BillingMode) -> &'static str {
    match mode {
        crate::pricing::BillingMode::Api => "API 实付",
        crate::pricing::BillingMode::Subscription => "订阅等价",
        crate::pricing::BillingMode::Free => "免费 / $0",
    }
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// 按当前筛选导出全部调用。不含 API Key 和对话正文；未知值留空，费用列标明估算与币种。
pub fn calls_csv(conn: &Connection, f: &StatsFilter) -> rusqlite::Result<(String, i64)> {
    let reqs = load(conn, f, true, true)?;
    let mut out = String::from("\u{feff}时间,供应商,模型,普通输入,缓存读取,缓存写入,输出,总 Token,估算费用（USD）,计费口径,计价模型,首字（毫秒）,总耗时（毫秒）,思考强度,状态,会话 ID,调用 ID,数据来源\r\n");
    let opt = |v: Option<i64>| v.map(|v| v.to_string()).unwrap_or_default();
    for r in &reqs {
        let time = Local.timestamp_millis_opt(r.ts).single().map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
        let status = match (r.cancelled, r.failed, r.unknown, r.status_code) {
            (true, _, _, _) => "已取消".to_string(),
            (false, true, _, Some(code)) => format!("失败 {code}"),
            (false, true, _, None) => "失败".to_string(),
            (false, false, true, _) => "状态未知".to_string(),
            (false, false, false, _) => "成功".to_string(),
        };
        let pricing_model = r.price.as_ref().map(|p| p.pricing_model.clone()).unwrap_or_default();
        let cells = [
            time,
            crate::platforms::get(&r.platform).map_or(r.platform.clone(), |p| p.name.clone()),
            r.model.clone().unwrap_or_default(),
            opt(r.fresh_input),
            opt(r.cache_read),
            opt(r.cache_write),
            opt(r.output),
            opt(r.total),
            r.cost.map(|c| format!("{c:.6}")).unwrap_or_default(),
            billing_mode_label(r.billing_mode).to_string(),
            pricing_model,
            opt(r.first_token_ms),
            opt(r.duration_ms),
            r.effort.clone().unwrap_or_default(),
            status,
            r.session_id.clone().unwrap_or_default(),
            r.call_id.clone(),
            source_label(&r.source).to_string(),
        ];
        out.push_str(&cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Ok((out, reqs.len() as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{insert_records, RequestRecord};

    #[test]
    fn unknown_outcomes_are_separate_from_success_and_exported() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        for (key, outcome, status) in [("pending", Some("unknown"), None), ("old", None, None),
            ("error", Some("unknown"), Some(500)), ("cancel", Some("cancelled"), Some(500))] {
            conn.execute("INSERT INTO requests(platform,source,dedup_key,ts,total_tokens,total_known,native_outcome,status_code) VALUES('zcode','test',?1,10,5,1,?2,?3)",
                rusqlite::params![key, outcome, status]).unwrap();
        }
        let mut f = StatsFilter { start: 0, end: 100, ..Default::default() };
        let overview = overview(&conn, &f).unwrap();
        assert_eq!((overview.requests, overview.failed, overview.cancelled, overview.unknown), (4, 1, 1, 1));
        for (status, key) in [("success", "old"), ("error", "error"), ("cancelled", "cancel"), ("unknown", "pending")] {
            f.status = Some(status.into());
            assert!(f.validate().is_ok());
            let page = calls(&conn, &f, 1).unwrap();
            assert_eq!(page.rows.len(), 1);
            assert_eq!(page.rows[0].call_id, key);
        }
        let (csv, _) = calls_csv(&conn, &f).unwrap();
        assert!(csv.contains("状态未知"));
    }

    #[test]
    fn model_rows_carry_price_source_and_check_date() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        for (key, model) in [("1", "claude-sonnet-4-5"), ("2", "made-up-model")] {
            conn.execute(
                "INSERT INTO requests(platform,source,dedup_key,ts,model,total_tokens) VALUES('claude','test',?1,10,?2,5)",
                rusqlite::params![key, model],
            )
            .unwrap();
        }
        let filter: StatsFilter = serde_json::from_value(serde_json::json!({"start": 0, "end": 100})).unwrap();
        let rows = overview(&conn, &filter).unwrap().models;
        let known = rows.iter().find(|row| row.model.as_deref() == Some("claude-sonnet-4-5")).unwrap();
        assert!(known.price_source.as_deref().is_some_and(|url| url.starts_with("https://")));
        assert_eq!(known.price_checked.as_deref(), Some("2026-11-03"));
        let unknown = rows.iter().find(|row| row.model.as_deref() == Some("made-up-model")).unwrap();
        assert_eq!((unknown.price_source.as_deref(), unknown.price_checked.as_deref()), (None, None), "未收录不编出处");
    }

    #[test]
    fn source_breakdown_groups_by_connection_and_counts_unattributed_apart() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,status,created_ms) VALUES('c-a','codex','auth','工作号','connected',1)", []).unwrap();
        let insert = |key: &str, platform: &str, ts: i64, total: i64, known: i64, connection: Option<&str>| {
            conn.execute(
                "INSERT INTO requests(platform,source,dedup_key,ts,input_tokens,total_tokens,total_known,connection_id) VALUES(?1,'test',?2,?3,?4,?4,?5,?6)",
                rusqlite::params![platform, key, ts, total, known, connection],
            )
            .unwrap();
        };
        insert("1", "codex", 100, 50, 1, Some("c-a"));
        insert("2", "codex", 110, 30, 1, Some("c-a"));
        insert("3", "codex", 120, 0, 0, Some("c-gone"));
        insert("4", "codex", 130, 70, 1, None);
        insert("5", "claude", 140, 90, 1, Some("c-a"));
        insert("6", "codex", 999, 10, 1, Some("c-a"));
        let b = source_breakdown(&conn, Some("codex"), 0, 500).unwrap();
        assert_eq!(b.total_requests, 4);
        assert_eq!(b.unattributed_requests, 1, "没有归属的不并进任何一行");
        assert_eq!(b.rows.len(), 2);
        assert_eq!(b.rows[0], SourceRow { connection_id: "c-a".into(), name: Some("工作号".into()), requests: 2, tokens: Some(80) });
        assert_eq!(b.rows[1], SourceRow { connection_id: "c-gone".into(), name: None, requests: 1, tokens: None }, "全未知不当 0；已删连接没有名字");
        assert_eq!(source_breakdown(&conn, None, 0, 500).unwrap().total_requests, 5);
    }

    /// P4 审计 D6：「今日按来源」与「本地今日 Token」同为新鲜口径，缓存重读不计。
    #[test]
    fn source_breakdown_uses_the_same_fresh_measure_as_todays_tokens() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,status,created_ms) VALUES('c1','claude','api','主号','connected',1)", []).unwrap();
        let now = chrono::Local::now().timestamp_millis() - 1;
        conn.execute(
            "INSERT INTO requests(platform,source,dedup_key,ts,input_tokens,output_tokens,cache_read_tokens,total_tokens,connection_id)
             VALUES('claude','test','k',?1,10,5,1000,1015,'c1')",
            [now],
        )
        .unwrap();
        let (start, end) = crate::db::period_range(&conn, "claude", "today");
        let row = &source_breakdown(&conn, Some("claude"), start, end).unwrap().rows[0];
        assert_eq!(row.tokens, Some(15), "不能把 1000 缓存重读算进来源行");
        assert_eq!(row.tokens, crate::db::today_fresh_tokens(&conn, "claude").unwrap(), "与底部合计同口径");
    }

    fn rec(platform: &str, key: &str, ts: i64, model: &str, tokens: [Option<i64>; 5], session: Option<&str>) -> RequestRecord {
        RequestRecord {
            platform: platform.into(),
            source: format!("test-{platform}"),
            dedup_key: key.into(),
            session_id: session.map(String::from),
            ts,
            model: Some(model.into()),
            input_tokens: tokens[0],
            output_tokens: tokens[1],
            cache_read_tokens: tokens[2],
            cache_write_tokens: tokens[3],
            total_tokens: tokens[4],
            effort: None,
        }
    }

    fn seeded(records: &[RequestRecord]) -> (Connection, std::path::PathBuf) {
        // 并行测试的记录条数可能相同，按序号区分临时库，避免互相删除或写入
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("jolu-usage-stats-{}-{seq}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut conn = crate::db::open(&path).unwrap();
        insert_records(&mut conn, records).unwrap();
        (conn, path)
    }

    fn filter(start: i64, end: i64) -> StatsFilter {
        StatsFilter { start, end, ..Default::default() }
    }

    #[test]
    fn call_cost_is_priced_at_request_time_like_its_price_snapshot() {
        // DeepSeek 周末全天谷时（半价），工作日 07:00 UTC 是峰时。按请求时间计价，
        // 两条同量请求的费用比应为 0.5，且与单价拆解的「谷时」标记一致——不能随查看时刻变化。
        let saturday = chrono::Utc.with_ymd_and_hms(2026, 11, 7, 12, 0, 0).unwrap().timestamp_millis();
        let monday = chrono::Utc.with_ymd_and_hms(2026, 11, 9, 7, 0, 0).unwrap().timestamp_millis();
        let tokens = [Some(1_000_000), Some(100_000), Some(0), Some(0), Some(1_100_000)];
        let (conn, path) = seeded(&[
            rec("zcode", "off-peak", saturday, "deepseek-v4-flash", tokens, None),
            rec("zcode", "peak", monday, "deepseek-v4-flash", tokens, None),
        ]);
        let page = calls(&conn, &filter(saturday - 1, monday + 1), 1).unwrap();
        let row = |key: &str| page.rows.iter().find(|row| row.call_id == key).unwrap();
        let (off, peak) = (row("off-peak"), row("peak"));
        assert_eq!(off.price.as_ref().map(|p| p.off_peak), Some(true));
        assert_eq!(peak.price.as_ref().map(|p| p.off_peak), Some(false));
        let ratio = off.cost.unwrap() / peak.cost.unwrap();
        assert!((ratio - 0.5).abs() < 1e-9, "谷时应半价，实际比值 {ratio}");
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    /// 模型行在部分请求没有 Token 时仍要给已知小计。
    ///
    /// 现场（2026-11-22 本机）：ZCode 的 `deepseek-v4.1-flash-1017` 有 4987 条记录，
    /// 其中 29 条是取消/失败、没有 Token。`sum_known` 遇到一条未知就整组返回 None，
    /// 于是整行显示破折号，真实的 23 亿 Token 完全看不见。
    #[test]
    fn a_model_row_keeps_its_known_subtotal_when_some_requests_have_none() {
        let base = chrono::Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap().timestamp_millis();
        let done = [Some(1_000), Some(500), Some(0), Some(0), Some(1_500)];
        let unknown = [Some(0), Some(0), Some(0), Some(0), None];
        let (conn, path) = seeded(&[
            rec("zcode", "m-ok-1", base + 1_000, "deepseek-v4.1-flash-1017", done, None),
            rec("zcode", "m-ok-2", base + 2_000, "deepseek-v4.1-flash-1017", done, None),
            // 取消/失败：Token 全零且标记为未知，正是「没跑完所以没有用量」
            rec("zcode", "m-cancel", base + 3_000, "deepseek-v4.1-flash-1017", unknown, None),
            // 另一个 model id：两个中转商给同一个上游模型起了不同名字（:1017 vs -1017），
            // 它们是真实的两个来源，不该被合并
            rec("zcode", "m-colon", base + 4_000, "deepseek-v4.1-flash:1017", done, None),
        ]);
        let view = overview(&conn, &filter(base, base + 10_000)).unwrap();
        let row = |model: &str| view.models.iter().find(|m| m.model.as_deref() == Some(model)).unwrap();

        let dashed = row("deepseek-v4.1-flash-1017");
        assert_eq!(dashed.requests, 3);
        assert_eq!(dashed.tokens, None, "有一条未知，整组合计不能当成完整值");
        assert_eq!(dashed.known_tokens, 3_000, "已知小计必须是真实值，不能因一条未知就归零");
        assert_eq!(dashed.known_requests, 2, "覆盖 3 条里的 2 条");

        let colon = row("deepseek-v4.1-flash:1017");
        assert_eq!(colon.requests, 1);
        assert_eq!(colon.tokens, Some(1_500), "全覆盖的家保持原有语义");
        assert_eq!((colon.known_tokens, colon.known_requests), (1_500, 1));

        // 排序按已知小计：部分未知的行（3000）不能因为 tokens 是 None 就落到全覆盖行（1500）后面
        let order: Vec<_> = view.models.iter().map(|m| m.model.clone().unwrap()).collect();
        assert_eq!(
            order,
            vec!["deepseek-v4.1-flash-1017", "deepseek-v4.1-flash:1017"],
            "排序用已知小计，不是可能为 None 的合计：{order:?}"
        );
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unpriced_reasons_separate_absent_usage_from_partial_fields() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("claude", "missing-price", base + 1, "unknown-model", [Some(1); 5], None),
            rec("claude", "absent", base + 2, "claude-sonnet-4-5", [None; 5], None),
            rec("codex", "partial", base + 3, "gpt-5", [Some(100), None, None, None, None], None),
            rec("claude", "total-only", base + 4, "claude-sonnet-4-5", [None, None, None, None, Some(100)], None),
            rec("claude", "zero", base + 5, "claude-sonnet-4-5", [Some(0); 5], None),
        ]);
        let summary = serde_json::to_value(overview(&conn, &filter(base, base + 100)).unwrap().cost).unwrap();
        assert_eq!(summary["unpriced_no_usage"], 1);
        assert_eq!(summary["unpriced_partial"], 2);
        assert_eq!(summary["unpriced_no_price"], serde_json::json!(["claude:unknown-model"]));
        assert_eq!(summary["priced_requests"], 1, "真实零仍应计价");
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn overview_normalizes_input_and_keeps_unknown_fields_unknown() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("codex", "c1", base + 1_000, "gpt-5", [Some(1_000), Some(200), Some(800), Some(0), Some(1_200)], Some("s1")),
            rec("claude", "a1", base + 2_000, "claude-sonnet-4-5", [Some(100), Some(50), Some(300), Some(0), Some(450)], Some("s2")),
            rec("claude", "a0", base - 86_400_000 + 1_000, "claude-sonnet-4-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
        ]);
        let o = overview(&conn, &filter(base, base + 86_400_000)).unwrap();
        assert_eq!(o.requests, 2);
        assert_eq!(o.tokens.fresh_input, Some(200 + 100), "Codex 扣掉缓存读取，Claude 原样");
        assert_eq!(o.tokens.cache_read, Some(1_100));
        assert_eq!(o.tokens.total, Some(1_650));
        assert_eq!(o.previous_total, Some(15));
        assert_eq!(o.cost.requests, 2);
        assert_eq!(o.providers.len(), 2);
        assert_eq!(o.sessions[0].session_id, "s1");
        assert_eq!(o.trend.bucket, "1h");
        assert_eq!(o.trend.points.len(), 24);
        // 来源元数据与统计同一批行：数据时间是范围内最后一条，最早可用记录不受范围限制
        assert_eq!(o.provenance.data_at, Some(base + 2_000));
        assert_eq!(o.provenance.earliest_available, Some(base - 86_400_000 + 1_000));
        assert_eq!((o.provenance.window_start, o.provenance.window_end), (base, base + 86_400_000));
        assert_eq!(o.provenance.unit, crate::data_contract::Unit::Tokens);
        assert_eq!(
            o.provenance.completeness,
            crate::data_contract::Completeness::AsRecorded,
            "本机采集无法对照源端证明完整，只能是「已采到的部分」"
        );
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    /// P4 审计 D8：今天上午 10 点对比的应是昨天 0–10 点，而不是昨天最后 10 小时。
    #[test]
    fn todays_comparison_uses_the_same_hours_of_yesterday() {
        let today = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let yesterday = local_ms(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap());
        let hour = 3_600_000;
        let (conn, path) = seeded(&[
            rec("claude", "y-morning", yesterday + 5 * hour, "claude-sonnet-4-5", [Some(60), Some(40), Some(0), Some(0), Some(100)], None),
            rec("claude", "y-evening", yesterday + 20 * hour, "claude-sonnet-4-5", [Some(5), Some(2), Some(0), Some(0), Some(7)], None),
            rec("claude", "t-morning", today + 2 * hour, "claude-sonnet-4-5", [Some(1), Some(1), Some(0), Some(0), Some(2)], None),
        ]);
        let o = overview(&conn, &filter(today, today + 10 * hour)).unwrap();
        assert_eq!(o.previous_total, Some(100), "应对比昨天 0–10 点（100），不是昨天 14–24 点（7）");
        // 整天区间平移结果不变
        assert_eq!(previous_window(today, today + 24 * hour), (yesterday, today));
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn overview_keeps_known_token_subtotal_when_native_requests_have_no_tokens() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("codex", "coverage-c", base + 1_000, "gpt-5", [Some(100), Some(20), Some(60), Some(0), Some(120)], None),
            rec("claude", "coverage-a", base + 2_000, "claude-sonnet-4-5", [Some(20), Some(10), Some(0), Some(0), Some(30)], None),
            rec("qoder", "coverage-q", base + 3_000, "qfmodel", [None; 5], None),
        ]);
        let all = overview(&conn, &filter(base, base + 3_600_000)).unwrap();
        assert_eq!(all.tokens.total, None, "未知请求不能伪装成完整总量");
        let json = serde_json::to_value(&all).unwrap();
        assert_eq!(json["token_coverage"]["known_total"], 150, "未知原生记录不能抹掉已知 Token 合计");
        assert_eq!(json["token_coverage"]["known_requests"], 2);
        assert_eq!(json["token_coverage"]["cache_hit_rate"], 0.5, "缓存分子与分母必须来自同一批可计算请求");
        assert_eq!(json["token_coverage"]["cache_requests"], 2);
        let qoder = overview(&conn, &StatsFilter { platform: Some("qoder".into()), ..filter(base, base + 3_600_000) }).unwrap();
        let json = serde_json::to_value(qoder).unwrap();
        assert!(json["token_coverage"]["known_total"].is_null(), "全部未知时不能显示零");
        assert_eq!(json["token_coverage"]["known_requests"], 0);
        assert!(json["token_coverage"]["cache_hit_rate"].is_null());
        assert_eq!(json["token_coverage"]["cache_requests"], 0);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_cache_field_makes_that_split_unknown_and_platform_filter_applies() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("claude", "a1", base + 1_000, "claude-sonnet-4-5", [Some(100), Some(50), None, Some(0), Some(150)], None),
            rec("codex", "c1", base + 2_000, "gpt-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
        ]);
        let all = overview(&conn, &filter(base, base + 3_600_000)).unwrap();
        assert_eq!(all.tokens.cache_read, None);
        assert_eq!(all.cache_hit_rate, None);
        assert_eq!(all.trend.bucket, "1h");
        let codex = overview(&conn, &StatsFilter { platform: Some("codex".into()), ..filter(base, base + 3_600_000) }).unwrap();
        assert_eq!(codex.tokens.cache_read, Some(0));
        assert_eq!(codex.requests, 1);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn calls_paginate_newest_first_and_search_by_session() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let records: Vec<_> = (0..22)
            .map(|i| rec("claude", &format!("k{i}"), base + i * 1_000, "claude-sonnet-4-5", [Some(1), Some(1), Some(0), Some(0), Some(2)], Some(if i == 3 { "needle" } else { "other" })))
            .collect();
        let (conn, path) = seeded(&records);
        let page = calls(&conn, &filter(base, base + 60_000), 1).unwrap();
        assert_eq!(page.total_count, 22);
        assert_eq!(page.page_count, 2);
        assert_eq!(page.rows.len(), 20);
        assert_eq!(page.rows[0].call_id, "k21");
        assert_eq!(page.models, vec!["claude-sonnet-4-5".to_string()]);
        let found = calls(&conn, &StatsFilter { search: Some("needle".into()), ..filter(base, base + 60_000) }, 1).unwrap();
        assert_eq!(found.total_count, 1);
        assert_eq!(found.rows[0].call_id, "k3");
        let (csv, count) = calls_csv(&conn, &filter(base, base + 60_000)).unwrap();
        assert_eq!(count, 22);
        assert!(csv.starts_with('\u{feff}'));
        assert_eq!(csv.lines().count(), 23);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn day_buckets_follow_local_midnight() {
        let start = local_ms(NaiveDate::from_ymd_opt(2026, 10, 15).unwrap());
        let end = local_ms(NaiveDate::from_ymd_opt(2026, 10, 22).unwrap());
        let (kind, ranges) = buckets(start, end);
        assert_eq!(kind, "day");
        assert_eq!(ranges.len(), 7);
        assert_eq!(ranges[0].2, "10/15");
        let (kind, ranges) = buckets(start, start + 12 * 3_600_000 + 1);
        assert_eq!(kind, "1h");
        assert_eq!(ranges.len(), 13);
        assert_eq!(ranges[12].2, "12:00");
        let (kind, ranges) = buckets(start, start + 2 * 86_400_000);
        assert_eq!(kind, "2h");
        assert_eq!(ranges.len(), 24);
    }

    #[test]
    fn csv_cells_are_quoted_when_needed() {
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("plain"), "plain");
    }

    /// P4 审计 D11：会话里有记录没给 Token 时，榜单给已知部分并带上未提供的次数，不把未知当 0。
    #[test]
    fn top_sessions_keep_unknown_requests_apart() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("claude", "k1", base + 1_000, "claude-sonnet-4-5", [Some(60), Some(40), Some(0), Some(0), Some(100)], Some("s")),
            rec("claude", "k2", base + 2_000, "claude-sonnet-4-5", [None; 5], Some("s")),
            rec("claude", "k3", base + 3_000, "claude-sonnet-4-5", [None; 5], Some("only-unknown")),
        ]);
        let o = overview(&conn, &filter(base, base + 3_600_000)).unwrap();
        assert_eq!(o.sessions.len(), 1, "一条已知都没有的会话无从排名，不进榜");
        assert_eq!((o.sessions[0].tokens, o.sessions[0].unknown_requests), (100, 1));
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn top_sessions_fills_cwd_from_source() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("claude", "a1", base + 1_000, "claude-sonnet-4-5", [Some(100), Some(50), Some(0), Some(0), Some(150)], Some("with-cwd")),
            rec("claude", "a2", base + 2_000, "claude-sonnet-4-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], Some("gone")),
            rec("claude", "a3", base + 3_000, "claude-sonnet-4-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], Some("with-cwd")),
            rec("codex", "c1", base + 4_000, "gpt-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], Some("unregistered")),
        ]);
        let dir = std::env::temp_dir().join(format!("jolu-top-sessions-cwd-{}", std::process::id()));
        let projects = dir.join(".claude").join("projects").join("demo");
        std::fs::create_dir_all(&projects).unwrap();
        let file = projects.join("with-cwd.jsonl");
        std::fs::write(&file, "{\"type\":\"user\",\"cwd\":\"D:\\\\work\\\\demo-app\",\"sessionId\":\"with-cwd\"}\n").unwrap();
        let missing = projects.join("gone.jsonl");
        for p in [&file, &missing] {
            conn.execute("INSERT INTO scan_state(path, offset, mtime) VALUES(?1, 0, 0)", [p.to_string_lossy()]).unwrap();
        }
        let o = overview(&conn, &filter(base, base + 86_400_000)).unwrap();
        let with = o.sessions.iter().find(|s| s.session_id == "with-cwd").unwrap();
        assert_eq!(with.cwd.as_deref(), Some("D:\\work\\demo-app"));
        let gone = o.sessions.iter().find(|s| s.session_id == "gone").unwrap();
        assert_eq!(gone.cwd, None, "来源文件不存在时不猜项目");
        let unregistered = o.sessions.iter().find(|s| s.session_id == "unregistered").unwrap();
        assert_eq!(unregistered.cwd, None, "没有登记来源文件时不猜项目");
        drop(conn);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 失败口径：error 计失败，cancelled 不算失败也不算成功，三种状态筛选互斥且合起来
    /// 覆盖全部有结论的行。这条测试锁定「取消不算失败」是有意为之，改动口径时必须同步改它。
    #[test]
    fn cancelled_is_neither_failure_nor_success() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 11, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("zcode", "ok", base + 1_000, "glm-4.7", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
            rec("zcode", "err", base + 2_000, "glm-4.7", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
            rec("zcode", "cancel", base + 3_000, "glm-4.7", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
            // 没有原生结论、也没有状态码的本机记录按成功处理
            rec("claude", "native-ok", base + 4_000, "claude-sonnet-4-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
            // 取消时上游可能留下非 2xx：cancelled 优先，不能既算取消又算失败
            rec("zcode", "cancel-500", base + 5_000, "glm-4.7", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
        ]);
        for (key, outcome) in [("ok", "success"), ("err", "failed"), ("cancel", "cancelled"), ("cancel-500", "cancelled")] {
            conn.execute(
                "UPDATE requests SET native_outcome = ?2 WHERE platform = 'zcode' AND dedup_key = ?1",
                rusqlite::params![key, outcome],
            ).unwrap();
        }
        conn.execute(
            "UPDATE requests SET status_code = 502 WHERE platform = 'zcode' AND dedup_key = 'cancel-500'",
            [],
        ).unwrap();

        let range = filter(base, base + 86_400_000);
        let o = overview(&conn, &range).unwrap();
        assert_eq!((o.requests, o.failed, o.cancelled), (5, 1, 2), "取消不并进失败数（即便带了 502）");
        assert_eq!(
            (o.requests - o.failed - o.cancelled) as f64 / o.requests as f64, 0.4,
            "成功率 = 成功 / 全部：2 条成功（含无状态码的本机记录）、1 失败、2 取消",
        );

        // 状态筛选：三档互斥，合起来正好是全部行
        let keys = |status: &str| -> Vec<String> {
            let mut f = filter(base, base + 86_400_000);
            f.status = Some(status.into());
            let mut ids: Vec<String> = calls(&conn, &f, 1).unwrap().rows.into_iter().map(|r| r.call_id).collect();
            ids.sort();
            ids
        };
        assert_eq!(keys("success"), vec!["native-ok", "ok"], "取消不能落进成功筛选");
        assert_eq!(keys("error"), vec!["err"], "取消不能落进失败筛选");
        assert_eq!(keys("cancelled"), vec!["cancel", "cancel-500"]);
        assert_eq!(
            keys("success").len() + keys("error").len() + keys("cancelled").len(), 5,
            "三档必须正好划分全部行：重复会让成功率被扣两次，遗漏会让某行哪档都不在",
        );

        // 明细行与 CSV 也带出独立状态
        let row = |key: &str| {
            calls(&conn, &range, 1).unwrap().rows.into_iter().find(|r| r.call_id == key).unwrap()
        };
        assert_eq!((row("cancel").failed, row("cancel").cancelled), (false, true));
        assert_eq!((row("err").failed, row("err").cancelled), (true, false));
        assert_eq!((row("cancel-500").failed, row("cancel-500").cancelled), (false, true), "cancel 优先于 502");
        let (csv, _) = calls_csv(&conn, &range).unwrap();
        assert!(csv.contains("已取消"), "CSV 状态列不能把取消写成失败");
        assert!(csv.contains("失败") && !csv.contains("失败 已取消"));

        // 非法状态值仍要被校验拦住；cancelled 必须放行
        let mut bad = filter(base, base + 1);
        bad.status = Some("canceled".into());
        assert!(bad.validate().is_err());
        let mut good = filter(base, base + 1);
        good.status = Some("cancelled".into());
        assert!(good.validate().is_ok());
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn billing_scope_splits_api_and_subscription_and_gemini_prices_without_cache_write() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap());
        let (conn, path) = seeded(&[
            // Gemini 缺 cache_write_tokens（cache_write_known = 0），仍应计价
            rec("gemini", "g1", base + 1_000, "gemini-2.5-pro", [Some(100_000), Some(20_000), Some(0), None, Some(120_000)], None),
            // API 连接（倍率 0.5）
            rec("claude", "c-api", base + 2_000, "claude-sonnet-4-5", [Some(1_000_000), Some(0), Some(0), Some(0), Some(1_000_000)], None),
            // 订阅连接
            rec("claude", "c-sub", base + 3_000, "claude-sonnet-4-5", [Some(1_000_000), Some(0), Some(0), Some(0), Some(1_000_000)], None),
            // 同模型一条取消无用量，模型行仍应保留已知费用小计
            rec("claude", "c-cancel", base + 4_000, "claude-sonnet-4-5", [None; 5], None),
        ]);
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,status,created_ms,billing_mode,price_multiplier)
             VALUES('conn-api','claude','api','半价中转','connected',1,'api',0.5),
                   ('conn-sub','claude','auth','官方订阅','connected',1,'auto',1.0)",
            [],
        ).unwrap();
        conn.execute("UPDATE requests SET connection_id='conn-api' WHERE dedup_key='c-api'", []).unwrap();
        conn.execute("UPDATE requests SET connection_id='conn-sub' WHERE dedup_key='c-sub'", []).unwrap();

        let all = overview(&conn, &filter(base, base + 86_400_000)).unwrap();
        assert_eq!(all.cost.priced_requests, 3, "Gemini 缺 cache_write 也应成功计价");
        assert!((all.cost.subscription_amount - 3.0).abs() < 1e-6);
        assert!(all.cost.api_amount > 1.5, "包含半价 API ($1.5) + 未归属 Gemini API");

        let claude_row = all.models.iter().find(|m| m.model.as_deref() == Some("claude-sonnet-4-5")).unwrap();
        assert_eq!(claude_row.priced_requests, 2);
        assert!((claude_row.cost.unwrap() - 4.5).abs() < 1e-6, "部分取消不应抹掉模型行已知费用（$1.5 + $3.0 = $4.5）");

        let mut only_api = filter(base, base + 86_400_000);
        only_api.billing_scope = Some("api".into());
        let api_view = overview(&conn, &only_api).unwrap();
        assert_eq!(api_view.cost.subscription_amount, 0.0);
        assert_eq!(api_view.cost.priced_requests, 2);

        let mut only_sub = filter(base, base + 86_400_000);
        only_sub.billing_scope = Some("subscription".into());
        let sub_view = overview(&conn, &only_sub).unwrap();
        assert_eq!(sub_view.requests, 1);
        assert!((sub_view.cost.amount - 3.0).abs() < 1e-6);

        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    /// P0 回归：
    /// 1) excludes_cache 缺 cache_write 时仍可算命中率（如 Antigravity / DSH）；
    /// 2) 趋势分桶混入单条 unknown 语义记录时，不把整根柱子抹零，并回报 unknown_requests 与 cache_hit_rate；
    /// 3) ModelRow.cache_hit_rate 不因单条取消/未知行归零；
    /// 4) 存量 Claude `<synthetic>` 零 Token 伪行自动从统计中排除。
    #[test]
    fn p0_excludes_cache_hit_rate_trend_partial_bucket_and_synthetic_exclusion() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 11, 21).unwrap());
        let (conn, path) = seeded(&[
            // 正常 Claude 行：有缓存读 800、新增输入 200 → 命中率 80%
            rec("claude", "c-ok", base + 1_000, "claude-sonnet-4-5", [Some(200), Some(50), Some(800), Some(0), Some(1_050)], None),
            // 同模型一条取消行（无 Token）
            rec("claude", "c-cancel", base + 2_000, "claude-sonnet-4-5", [None; 5], None),
            // Claude `<synthetic>` 零 Token 伪行（应被排除）
            rec("claude", "c-syn", base + 3_000, "<synthetic>", [Some(0), Some(0), Some(0), Some(0), Some(0)], None),
        ]);
        // 一条 excludes_cache 但 cache_write_known=0 的原生记录（如 Antigravity）
        crate::source_store::write(&conn, &[crate::source_store::Record {
            request: RequestRecord {
                platform: "antigravity".into(),
                source: "native:antigravity:default".into(),
                dedup_key: "ag-1".into(),
                session_id: Some("native:antigravity:default:s1".into()),
                ts: base + 4_000,
                model: Some("gemini-4-neon-xhigh".into()),
                input_tokens: Some(3_000),
                output_tokens: Some(800),
                cache_read_tokens: Some(177_000),
                cache_write_tokens: None,
                total_tokens: Some(180_800),
                effort: None,
            },
            input_semantics: "excludes_cache",
            credits: None,
            original_credits: None,
            billable: None,
            context_ratio: None,
            reasoning: None,
            tool: None,
            duration_ms: Some(15_000),
            first_token_ms: Some(10_000),
        }]).unwrap();
        // 一条 input_semantics='unknown' 的 workbuddy 记录（曾导致整日趋势桶归零）
        crate::source_store::write(&conn, &[crate::source_store::Record {
            request: RequestRecord {
                platform: "workbuddy".into(),
                source: "native:workbuddy:main".into(),
                dedup_key: "wb-1".into(),
                session_id: None,
                ts: base + 5_000,
                model: Some("glm-5".into()),
                input_tokens: Some(100),
                output_tokens: Some(20),
                cache_read_tokens: None,
                cache_write_tokens: None,
                total_tokens: Some(120),
                effort: None,
            },
            input_semantics: "unknown",
            credits: None,
            original_credits: None,
            billable: None,
            context_ratio: None,
            reasoning: None,
            tool: None,
            duration_ms: None,
            first_token_ms: None,
        }]).unwrap();

        let o = overview(&conn, &filter(base, base + 86_400_000)).unwrap();
        // `<synthetic>` 应被排除：共 4 条有效请求（c-ok, c-cancel, ag-1, wb-1）
        assert_eq!(o.requests, 4);
        assert!(!o.cost.unpriced_no_price.iter().any(|m| m.contains("<synthetic>")));

        // 趋势图第 0 个 1h 桶不能被 wb-1 / c-cancel 吞零
        let b0 = &o.trend.points[0];
        assert_eq!(b0.fresh_input, Some(3_200), "已知新鲜输入 = 200 + 3000");
        assert_eq!(b0.cache_read, Some(177_800));
        assert_eq!(b0.unknown_requests, 2, "c-cancel 和 wb-1 缺分项或语义未知");
        assert!(b0.cache_hit_rate.unwrap() > 0.98);

        // Antigravity excludes_cache 缺 cache_write 仍可计算准确命中率：177_000 / 180_000 = 98.333%
        let ag = overview(&conn, &StatsFilter { platform: Some("antigravity".into()), ..filter(base, base + 86_400_000) }).unwrap();
        let rate = ag.cache_hit_rate.expect("excludes_cache 缺 cache_write 不应导致命中率为空");
        assert!((rate - (177_000.0 / 180_000.0)).abs() < 1e-6);

        // Claude 模型行虽含 1 条取消行，命中率仍保留 80%
        let claude_m = o.models.iter().find(|m| m.model.as_deref() == Some("claude-sonnet-4-5")).unwrap();
        assert!((claude_m.cache_hit_rate.unwrap() - 0.8).abs() < 1e-6);

        // Antigravity 写入的 first_token_ms 应完整落库并透传到调用明细
        let ag_calls = calls(&conn, &StatsFilter { platform: Some("antigravity".into()), ..filter(base, base + 86_400_000) }, 1).unwrap();
        assert_eq!(ag_calls.rows[0].first_token_ms, Some(10_000));
        assert_eq!(ag_calls.rows[0].duration_ms, Some(15_000));

        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}
