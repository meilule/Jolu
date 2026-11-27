use super::*;
use rusqlite::{types::Value, Connection};
use crate::db::EXCLUDE_DEGENERATE_SQL;

/// 拼 WHERE 子句。`calls = false` 时只用区间、供应商、账号与模型——概览和分析不受状态 / 搜索影响。
pub fn where_clause(f: &StatsFilter, calls: bool) -> (String, Vec<Value>) {
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
pub const FAILED_SQL: &str = "(COALESCE(r.native_outcome, '') <> 'cancelled') AND ((r.status_code IS NOT NULL AND (r.status_code < 200 OR r.status_code >= 300)) OR COALESCE(r.native_outcome, '') = 'failed')";

/// 被取消的请求：只有原生来源会明确给出这一档。
/// 取优先级高于 [`FAILED_SQL`]：取消发生在客户端，上游可能还没给出状态码，
/// 也可能留下一个非 2xx（例如中止导致连接断开被记为 5xx）。同一行的三档必须互斥，
/// 否则「成功 + 失败 + 取消」会大于总调用数，成功率被扣两次。
pub const CANCELLED_SQL: &str = "COALESCE(r.native_outcome, '') = 'cancelled'";
pub const UNKNOWN_SQL: &str = "COALESCE(r.native_outcome, '') = 'unknown' AND (r.status_code IS NULL OR (r.status_code >= 200 AND r.status_code < 300))";


pub fn resolve_row_pricing(
    resolver: &mut crate::pricing::PriceResolver,
    req: &crate::pricing::PriceRequest<'_>,
    has_usage: bool,
) -> (Option<f64>, Option<PriceSnapshot>, Option<&'static str>, Option<String>) {
    let res = resolver.price_request(req);
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
}

pub fn load(conn: &Connection, f: &StatsFilter, calls: bool, need_pricing: bool) -> rusqlite::Result<Vec<Req>> {
    let (clause, args) = where_clause(f, calls);
    let sql = format!(
        "SELECT r.id, r.dedup_key, r.platform, r.model, r.ts, r.session_id,
                r.input_tokens, r.output_tokens, r.cache_read_tokens, r.cache_write_tokens, r.total_tokens,
                r.input_known, r.output_known, r.cache_read_known, r.cache_write_known, r.total_known,
                r.input_semantics, r.effort, r.duration_ms, r.first_token_ms, r.status_code, r.native_outcome,
                m.reasoning, m.tool, r.connection_id, r.provider_key,
                r.source, r.upstream_model, r.served_model, r.cache_write_1h_tokens, m.credits,
                r.attribution_source
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
            r.get::<_, Option<String>>(31)?,
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
            attribution_source,
        ) = raw?;

        let attribution = snapshot
            .as_ref()
            .map_or_else(crate::attribution::AttributionView::platform, |snap| {
                snap.resolve_with_source(
                    &platform,
                    connection_id.as_deref(),
                    attribution_source.as_deref(),
                    provider_key.as_deref(),
                    model.as_deref(),
                )
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
            resolve_row_pricing(
                &mut resolver,
                &crate::pricing::PriceRequest {
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
                },
                has_usage,
            )
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
