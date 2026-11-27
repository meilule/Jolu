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

use serde::{Deserialize, Serialize};

pub use crate::sessions::types::UNATTRIBUTED;


pub mod activity;
pub mod calls;
pub mod csv;
pub mod overview;
pub mod query;
pub mod trend;
#[cfg(test)]
mod tests;

pub use activity::*;
pub use calls::*;
pub use csv::*;
pub use overview::*;
pub use query::*;
pub use trend::*;

/// 两次请求间隔不超过 5 分钟视为连续活跃，用来估算今日活跃时长
pub const ACTIVE_GAP_MS: i64 = 5 * 60 * 1000;

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

    pub fn with_range(&self, start: i64, end: i64) -> Self {
        Self { start, end, ..self.clone() }
    }
}


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
pub struct Req {
    pub id: i64,
    pub call_id: String,
    pub source: String,
    pub platform: String,
    pub model: Option<String>,
    pub upstream_model: Option<String>,
    pub served_model: Option<String>,
    pub effective_model: Option<String>,
    pub ts: i64,
    pub session_id: Option<String>,
    pub fresh_input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub cache_write_1h: i64,
    pub reasoning: Option<i64>,
    pub tool: Option<i64>,
    pub total: Option<i64>,
    pub hit_denominator: Option<i64>,
    pub cost: Option<f64>,
    pub credits: Option<f64>,
    pub billing_mode: crate::pricing::BillingMode,
    pub price_multiplier: f64,
    pub unpriced_reason: Option<&'static str>,
    /// 该次请求命中的单价（每百万 token，USD），用于界面的费用拆解；无价目时为 None
    pub price: Option<PriceSnapshot>,
    pub effort: Option<String>,
    pub duration_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub status_code: Option<i64>,
    /// 上游错误（非 2xx）或原生来源给出 `failed`。**不包含**取消，见 [`FAILED_SQL`]。
    pub failed: bool,
    /// 原生来源明确标记 `cancelled`。取消不计入失败，也不计入成功。
    pub cancelled: bool,
    pub unknown: bool,
    pub attribution: crate::attribution::AttributionView,
}


/// 已知值求和：任一请求缺该项则整体未知；没有请求时为 `empty`。
/// 这是刻意的保守口径——不把「部分来源的已知值」当成全量展示。
/// 供界面参考的「已知值小计」由 [`known_subtotal`] 单独提供。
pub fn sum_known<'a>(reqs: impl IntoIterator<Item = &'a Req>, pick: impl Fn(&Req) -> Option<i64>, empty: Option<i64>) -> Option<i64> {
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
pub fn known_subtotal<'a>(
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

pub fn hit_rate<'a>(reqs: impl IntoIterator<Item = &'a Req> + Clone) -> Option<f64> {
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

pub fn split<'a>(reqs: impl IntoIterator<Item = &'a Req> + Clone, empty: Option<i64>) -> TokenSplit {
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

pub fn cost<'a>(reqs: impl IntoIterator<Item = &'a Req>) -> CostSummary {
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
