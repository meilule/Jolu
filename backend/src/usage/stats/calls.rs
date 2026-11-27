use rusqlite::Connection;
use serde::Serialize;
use super::*;

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
