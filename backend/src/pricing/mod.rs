//! 统一费用估算引擎。
//!
//! 价格解析优先级（由高到低）：
//! 1. 连接 `billing_mode = free` → `$0`（与无价 `None` 严格区分）
//! 2. 连接专属到手覆盖（`conn/model` → `conn/*`，不乘连接倍率）
//! 3. 基准单价（命中后乘连接 `price_multiplier`）：
//!    - 用户全局自定义价（`*/model`，支持 `disabled = 1` 墓碑）
//!    - 内置规则表（[`table`]）与 `models.dev` 合并（跨平台兜底，保留长上下文阶梯与峰谷半价）
//!    - 同价映射（`pricing_aliases`：连接级优先于全局）
//! 4. 上游回显模型（`served_model`）回退

pub mod budget;
pub mod store;
pub(crate) mod table;

use crate::gateway::models_dev::{self, CachedCatalog, ModelCost, ModelCostTier};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};
use store::{ConnectionBilling, PricingOverride, GLOBAL_SCOPE};
use table::PRICES;

#[derive(Clone, Copy)]
pub(super) struct Price {
    pub(super) platform: &'static str,
    pub(super) model: &'static str,
    pub(super) input: f64,
    pub(super) cached_input: f64,
    pub(super) output: f64,
    pub(super) cache_write: Option<f64>,
    pub(super) source: &'static str,
    pub(super) long_context: Option<LongContext>,
    pub(super) off_peak: Option<OffPeak>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct LongContext {
    pub(super) threshold: i64,
    pub(super) input_multiplier: f64,
    pub(super) output_multiplier: f64,
    pub(super) cached_multiplier: f64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct OffPeak {
    pub(super) multiplier: f64,
    pub(super) windows: &'static [OffPeakWindow],
}

#[derive(Debug, Clone, Copy)]
pub(super) struct OffPeakWindow {
    pub(super) start_hour: u8,
    pub(super) end_hour: u8,
    pub(super) weekdays: &'static [u8],
    pub(super) full_day: bool,
}

#[derive(Clone, Copy)]
struct LongTier {
    model: &'static str,
    input: f64,
    cached_input: f64,
    output: f64,
}

pub const GEMINI_LONG_PROMPT_TOKENS: i64 = 200_000;
const GEMINI_LONG: &[LongTier] = &[
    LongTier { model: "gemini-2.5-pro", input: 2.5, cached_input: 0.25, output: 15.0 },
    LongTier { model: "gemini-3.1-pro-preview", input: 4.0, cached_input: 0.4, output: 18.0 },
];

/// 网关路由计价模型基准（`upstream` | `request`），启动与保存设置时同步。
static MODEL_SOURCE: OnceLock<RwLock<String>> = OnceLock::new();

fn model_source_slot() -> &'static RwLock<String> {
    MODEL_SOURCE.get_or_init(|| RwLock::new("upstream".into()))
}

pub fn set_pricing_model_source(source: &str) {
    let normalized = if source == "request" { "request" } else { "upstream" };
    if let Ok(mut guard) = model_source_slot().write() {
        *guard = normalized.to_string();
    }
}

pub fn pricing_model_source() -> String {
    model_source_slot()
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "upstream".into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingMode {
    Api,
    Subscription,
    Free,
}

impl BillingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            BillingMode::Api => "api",
            BillingMode::Subscription => "subscription",
            BillingMode::Free => "free",
        }
    }

    pub fn resolve(mode: &str, connection_kind: &str) -> Self {
        match mode {
            "api" => BillingMode::Api,
            "subscription" => BillingMode::Subscription,
            "free" => BillingMode::Free,
            _ => {
                if connection_kind == "auth" {
                    BillingMode::Subscription
                } else {
                    BillingMode::Api
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnpricedReason {
    NoModel,
    NoPrice,
    NoUsage,
}

/// 日期快照后缀：`-2025-09-29`（11 字符）或 `-20250929`（9 字符）。
fn is_date_snapshot_suffix(value: &str) -> bool {
    let bytes = value.as_bytes();
    match bytes.len() {
        11 => {
            bytes[0] == b'-'
                && bytes[1..].iter().enumerate().all(|(index, byte)| {
                    if matches!(index, 4 | 7) { *byte == b'-' } else { byte.is_ascii_digit() }
                })
        }
        9 => bytes[0] == b'-' && bytes[1..].iter().all(u8::is_ascii_digit),
        _ => false,
    }
}

fn strip_date_suffix(model: &str) -> Option<String> {
    for suffix_len in [11usize, 9usize] {
        if model.len() > suffix_len {
            let split = model.len() - suffix_len;
            if model.is_char_boundary(split) && is_date_snapshot_suffix(&model[split..]) {
                return Some(model[..split].to_string());
            }
        }
    }
    None
}

fn strip_bedrock_prefix(model: &str) -> Option<String> {
    const PREFIXES: &[&str] = &[
        "global.", "us-gov.", "us.", "eu.", "apac.", "jp.", "au.", "ca.", "in.",
        "anthropic.", "openai.", "google.", "meta.", "deepseek.", "xai.",
    ];
    for prefix in PREFIXES {
        if let Some(rest) = model.strip_prefix(prefix) {
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

fn strip_window_marker(model: &str) -> Option<String> {
    if let Some(idx) = model.rfind('[') {
        if model.ends_with(']') && idx > 0 {
            let inside = &model[idx + 1..model.len() - 1];
            if inside.ends_with('m') || inside.ends_with('k') {
                return Some(model[..idx].trim_end_matches('-').to_string());
            }
        }
    }
    if let Some(idx) = model.rfind('@') {
        if idx > 0 {
            let tail = &model[idx + 1..];
            if tail.chars().take_while(|c| c.is_ascii_digit()).count() > 0
                && (tail.ends_with('k') || tail.ends_with('m'))
            {
                return Some(model[..idx].to_string());
            }
        }
    }
    None
}

fn strip_colon_tag(model: &str) -> Option<String> {
    let idx = model.rfind(':')?;
    if idx == 0 {
        return None;
    }
    let tail = &model[idx + 1..];
    // 兼容 Bedrock `-v1:0`
    if let Some(base) = model[..idx].strip_suffix("-v1").or_else(|| model[..idx].strip_suffix("-v2")) {
        if tail.chars().all(|c| c.is_ascii_digit()) {
            return Some(base.to_string());
        }
    }
    Some(model[..idx].to_string())
}

fn colon_to_dash(model: &str) -> Option<String> {
    if model.contains(':') {
        Some(model.replace(':', "-"))
    } else {
        None
    }
}

fn strip_effort_dash_suffix(model: &str) -> Option<String> {
    const EFFORTS: &[&str] = &["-minimal", "-low", "-medium", "-high", "-xhigh", "-max"];
    for suffix in EFFORTS {
        if let Some(rest) = model.strip_suffix(suffix) {
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// 生成按优先级排序的规范化模型候选名（不随意剥离未知 `-custom` 后缀，防误匹配）。
pub fn normalize_model_candidates(raw: &str) -> Vec<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(trimmed);

    while let Some(cand) = queue.pop_front() {
        let cand = cand.trim().to_string();
        if cand.is_empty() || !seen.insert(cand.clone()) {
            continue;
        }
        out.push(cand.clone());

        if let Some((_, tail)) = cand.rsplit_once('/') {
            if !tail.is_empty() {
                queue.push_back(tail.to_string());
            }
        }
        if let Some(s) = strip_bedrock_prefix(&cand) {
            queue.push_back(s);
        }
        if let Some(s) = strip_window_marker(&cand) {
            queue.push_back(s);
        }
        if let Some(s) = colon_to_dash(&cand) {
            queue.push_back(s);
        }
        if let Some(s) = strip_colon_tag(&cand) {
            queue.push_back(s);
        }
        if let Some(s) = strip_date_suffix(&cand) {
            queue.push_back(s);
        }
        if let Some(s) = strip_effort_dash_suffix(&cand) {
            queue.push_back(s);
        }
        if cand.starts_with("claude-") {
            if cand.contains('.') {
                queue.push_back(cand.replace('.', "-"));
            }
            // 将 `claude-opus-4-6` 同步尝试 `claude-opus-4.6`（兼容 models.dev 部分点号条目）
            if let Some(dotted) = claude_dash_to_dot(&cand) {
                queue.push_back(dotted);
            }
        }
    }
    out
}

fn claude_dash_to_dot(model: &str) -> Option<String> {
    let bytes = model.as_bytes();
    for i in 1..bytes.len().saturating_sub(2) {
        if bytes[i] == b'-' && bytes[i - 1].is_ascii_digit() && bytes[i + 1].is_ascii_digit() {
            let after_end = i + 2 == bytes.len() || !bytes[i + 2].is_ascii_digit();
            let before_start = i == 1 || bytes[i - 2] == b'-';
            if before_start && after_end {
                let mut s = model.to_string();
                s.replace_range(i..=i, ".");
                return Some(s);
            }
        }
    }
    None
}

fn is_claude_model(model: &str, platform: Option<&str>) -> bool {
    model.starts_with("claude-") || platform == Some("claude")
}

fn is_gemini_model(model: &str, platform: Option<&str>) -> bool {
    model.starts_with("gemini-") || platform == Some("gemini")
}

fn lookup_builtin_candidate(platform: Option<&str>, candidate: &str) -> Option<Price> {
    PRICES.iter().copied().find(|price| {
        platform.is_none_or(|value| value == price.platform)
            && (candidate == price.model
                || candidate.strip_prefix(price.model).is_some_and(is_date_snapshot_suffix))
    })
}

fn lookup_models_dev_candidate<'a>(
    catalog: &'a CachedCatalog,
    candidate: &str,
) -> Option<(&'a str, &'a str, &'a ModelCost)> {
    if let Some((k, meta)) = catalog.models.get_key_value(candidate) {
        if let Some(cost) = meta.cost.as_ref() {
            return Some((k.as_str(), meta.vendor.as_str(), cost));
        }
    }
    for (id, meta) in &catalog.models {
        let Some(cost) = meta.cost.as_ref() else { continue };
        if id.eq_ignore_ascii_case(candidate) {
            return Some((id.as_str(), meta.vendor.as_str(), cost));
        }
        if let Some((_, tail)) = id.rsplit_once('/') {
            if tail.eq_ignore_ascii_case(candidate) {
                return Some((id.as_str(), meta.vendor.as_str(), cost));
            }
        }
    }
    None
}

#[derive(Clone, Debug)]
struct ResolvedTariff {
    pricing_model: String,
    source_kind: &'static str,
    source: String,
    alias_from: Option<String>,
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: Option<f64>,
    cache_write_1h: Option<f64>,
    tiers: Vec<ModelCostTier>,
    long_context: Option<LongContext>,
    off_peak: Option<OffPeak>,
    gemini_long_model: Option<&'static str>,
    is_claude: bool,
    is_gemini: bool,
    /// 连接专属覆盖价已含最终到手价，不乘 `price_multiplier`
    scale_by_multiplier: bool,
}

impl ResolvedTariff {
    fn from_builtin(p: Price, md_cost: Option<&ModelCost>) -> Self {
        let is_claude = p.model.starts_with("claude-") || p.platform == "claude";
        let is_gemini = p.model.starts_with("gemini-") || p.platform == "gemini";
        let cache_write = p
            .cache_write
            .or_else(|| md_cost.and_then(|c| c.cache_write))
            .or_else(|| is_claude.then_some(p.input * 1.25));
        let cache_write_1h = md_cost
            .and_then(|c| c.cache_write_1h)
            .or_else(|| is_claude.then_some(p.input * 2.0));
        let gemini_long_model = GEMINI_LONG
            .iter()
            .find(|t| t.model == p.model)
            .map(|t| t.model);
        let tiers = if p.long_context.is_none() && gemini_long_model.is_none() {
            md_cost.map(|c| c.tiers.clone()).unwrap_or_default()
        } else {
            Vec::new()
        };
        Self {
            pricing_model: p.model.to_string(),
            source_kind: "builtin",
            source: p.source.to_string(),
            alias_from: None,
            input: p.input,
            output: p.output,
            cache_read: p.cached_input,
            cache_write,
            cache_write_1h,
            tiers,
            long_context: p.long_context,
            off_peak: p.off_peak,
            gemini_long_model,
            is_claude,
            is_gemini,
            scale_by_multiplier: true,
        }
    }

    fn from_models_dev(id: &str, vendor: &str, cost: &ModelCost, platform: Option<&str>) -> Self {
        let lower = id.to_ascii_lowercase();
        let is_claude = is_claude_model(&lower, platform);
        let is_gemini = is_gemini_model(&lower, platform);
        let cache_write = cost.cache_write.or_else(|| is_claude.then_some(cost.input * 1.25));
        let cache_write_1h = cost.cache_write_1h.or_else(|| is_claude.then_some(cost.input * 2.0));
        let source = if vendor.is_empty() {
            "models.dev".to_string()
        } else {
            format!("models.dev/{vendor}")
        };
        Self {
            pricing_model: id.to_string(),
            source_kind: "models_dev",
            source,
            alias_from: None,
            input: cost.input,
            output: cost.output,
            cache_read: cost.cache_read,
            cache_write,
            cache_write_1h,
            tiers: cost.tiers.clone(),
            long_context: None,
            off_peak: None,
            gemini_long_model: None,
            is_claude,
            is_gemini,
            scale_by_multiplier: true,
        }
    }

    fn from_override(ov: &PricingOverride, source_kind: &'static str, scale_by_multiplier: bool) -> Self {
        let lower = ov.model_id.to_ascii_lowercase();
        let is_claude = lower.starts_with("claude-");
        let is_gemini = lower.starts_with("gemini-");
        let mut tiers = Vec::new();
        if let Some(above) = ov.long_ctx_threshold {
            if above > 0 {
                tiers.push(ModelCostTier {
                    above,
                    input: ov.long_ctx_input.unwrap_or(ov.input),
                    output: ov.long_ctx_output.unwrap_or(ov.output),
                    cache_read: ov.long_ctx_cache_read.unwrap_or(ov.cache_read),
                    cache_write: ov.long_ctx_cache_write.or(ov.cache_write),
                    cache_write_1h: ov.cache_write_1h,
                });
            }
        }
        let cache_write = ov.cache_write.or_else(|| is_claude.then_some(ov.input * 1.25));
        let cache_write_1h = ov.cache_write_1h.or_else(|| is_claude.then_some(ov.input * 2.0));
        Self {
            pricing_model: ov.model_id.clone(),
            source_kind,
            source: if source_kind == "connection_override" {
                "连接自定义价目".into()
            } else {
                "自定义价目".into()
            },
            alias_from: None,
            input: ov.input,
            output: ov.output,
            cache_read: ov.cache_read,
            cache_write,
            cache_write_1h,
            tiers,
            long_context: None,
            off_peak: None,
            gemini_long_model: None,
            is_claude,
            is_gemini,
            scale_by_multiplier,
        }
    }
}

/// 一条请求的用量。`reasoning` / `tool` 只对 Gemini 参与计价；
/// `cache_write_1h` 是 `cache_write` 中的 1h TTL 子集；
/// `input_semantics` 决定输入是否需要扣减缓存部分。
#[cfg(test)]
#[derive(Clone, Copy, Default)]
pub struct Usage {
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_write_1h: i64,
    pub cache_read: i64,
    pub reasoning: i64,
    pub tool: i64,
    pub long_prompt: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppliedPrice {
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
    pub cache_write: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_1h: Option<f64>,
    pub source: String,
    pub source_kind: String,
    pub pricing_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias_from: Option<String>,
    pub multiplier: f64,
    pub billing_mode: String,
    pub long_context: bool,
    pub off_peak: bool,
}

pub struct PriceRequest<'a> {
    pub platform: &'a str,
    pub model: Option<&'a str>,
    pub upstream_model: Option<&'a str>,
    pub served_model: Option<&'a str>,
    pub connection_id: Option<&'a str>,
    pub input_semantics: &'a str,
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub cache_write_1h: i64,
    pub reasoning: i64,
    pub tool: i64,
    pub at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct PriceResult {
    pub cost: Option<f64>,
    pub applied: Option<AppliedPrice>,
    pub unpriced_reason: Option<UnpricedReason>,
    pub effective_model: Option<String>,
}

/// 单次统计查询复用的计价解析器（内部记忆化模型查价，避免万条请求重复查表）。
pub struct PriceResolver {
    overrides: HashMap<(String, String), PricingOverride>,
    aliases: HashMap<(String, String), String>,
    connections: HashMap<String, ConnectionBilling>,
    catalog: Arc<CachedCatalog>,
    model_source: String,
    tariff_cache: HashMap<(Option<String>, String, String), Option<ResolvedTariff>>,
}

impl PriceResolver {
    pub fn load(conn: &Connection) -> Self {
        let mut overrides = HashMap::new();
        if let Ok(list) = store::list_overrides(conn) {
            for item in list {
                overrides.insert((item.scope.clone(), item.model_id.clone()), item);
            }
        }
        let mut aliases = HashMap::new();
        if let Ok(list) = store::list_aliases(conn) {
            for item in list {
                aliases.insert((item.scope, item.alias_model), item.target_model);
            }
        }
        let connections = store::load_connection_billing(conn).unwrap_or_default();
        Self {
            overrides,
            aliases,
            connections,
            catalog: models_dev::active_catalog(),
            model_source: pricing_model_source(),
            tariff_cache: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub fn with_model_source(mut self, source: impl Into<String>) -> Self {
        self.model_source = source.into();
        self
    }

    pub fn billing_for(&self, connection_id: Option<&str>) -> (BillingMode, f64) {
        let Some(id) = connection_id else {
            return (BillingMode::Api, 1.0);
        };
        let Some(conn) = self.connections.get(id) else {
            return (BillingMode::Api, 1.0);
        };
        let mode = BillingMode::resolve(&conn.billing_mode, &conn.kind);
        let mult = if conn.price_multiplier.is_finite() && conn.price_multiplier >= 0.0 {
            conn.price_multiplier
        } else {
            1.0
        };
        (mode, mult)
    }

    pub fn select_models<'a>(
        &self,
        model: Option<&'a str>,
        upstream: Option<&'a str>,
        served: Option<&'a str>,
    ) -> (Option<&'a str>, Option<&'a str>) {
        let clean = |v: Option<&'a str>| v.map(str::trim).filter(|s| !s.is_empty());
        let (model, upstream, served) = (clean(model), clean(upstream), clean(served));
        if self.model_source == "request" {
            let primary = model.or(upstream).or(served);
            let fallback = upstream.or(served).filter(|f| Some(*f) != primary);
            (primary, fallback)
        } else {
            let primary = upstream.or(served).or(model);
            let fallback = served
                .filter(|f| Some(*f) != primary)
                .or_else(|| model.filter(|f| Some(*f) != primary));
            (primary, fallback)
        }
    }

    pub fn has_price_for(&mut self, connection_id: Option<&str>, platform: &str, model: &str) -> bool {
        let (mode, _) = self.billing_for(connection_id);
        if mode == BillingMode::Free {
            return true;
        }
        self.resolve_tariff_cached(connection_id, platform, model).is_some()
    }

    pub fn price_request(&mut self, req: &PriceRequest<'_>) -> PriceResult {
        let (billing_mode, multiplier) = self.billing_for(req.connection_id);
        let (primary, fallback) = self.select_models(req.model, req.upstream_model, req.served_model);

        let (Some(input), Some(output)) = (req.input, req.output) else {
            return PriceResult {
                cost: None,
                applied: None,
                unpriced_reason: Some(UnpricedReason::NoUsage),
                effective_model: primary.map(str::to_string),
            };
        };

        if billing_mode == BillingMode::Free {
            return PriceResult {
                cost: Some(0.0),
                applied: Some(AppliedPrice {
                    input: 0.0,
                    cached_input: 0.0,
                    output: 0.0,
                    cache_write: Some(0.0),
                    cache_write_1h: Some(0.0),
                    source: "连接设为免费 / 计 $0".into(),
                    source_kind: "connection_free".into(),
                    pricing_model: primary.unwrap_or("free").to_string(),
                    alias_from: None,
                    multiplier: 0.0,
                    billing_mode: billing_mode.as_str().into(),
                    long_context: false,
                    off_peak: false,
                }),
                unpriced_reason: None,
                effective_model: primary.map(str::to_string),
            };
        }

        let Some(primary_model) = primary else {
            return PriceResult {
                cost: None,
                applied: None,
                unpriced_reason: Some(UnpricedReason::NoModel),
                effective_model: None,
            };
        };

        let tariff = self
            .resolve_tariff_cached(req.connection_id, req.platform, primary_model)
            .or_else(|| {
                fallback.and_then(|fb| self.resolve_tariff_cached(req.connection_id, req.platform, fb))
            });

        let Some(tariff) = tariff else {
            return PriceResult {
                cost: None,
                applied: None,
                unpriced_reason: Some(UnpricedReason::NoPrice),
                effective_model: Some(primary_model.to_string()),
            };
        };

        let cache_read = req.cache_read.unwrap_or(0).max(0);
        let cache_write = req.cache_write.unwrap_or(0).max(0);
        let cache_write_1h = req.cache_write_1h.clamp(0, cache_write);
        let cache_write_5m = cache_write - cache_write_1h;

        let fresh_input = compute_fresh_input(req.input_semantics, req.platform, input, cache_read, cache_write);
        let prompt_tokens = fresh_input + cache_read + cache_write;
        let applied_mult = if tariff.scale_by_multiplier { multiplier } else { 1.0 };

        let (cost, applied) = compute_tariff_cost(
            &tariff,
            fresh_input,
            output.max(0),
            cache_read,
            cache_write_5m,
            cache_write_1h,
            req.reasoning.max(0),
            req.tool.max(0),
            prompt_tokens,
            prompt_tokens > GEMINI_LONG_PROMPT_TOKENS,
            req.at,
            applied_mult,
            billing_mode,
        );

        PriceResult {
            cost: Some(cost),
            applied: Some(applied),
            unpriced_reason: None,
            effective_model: Some(primary_model.to_string()),
        }
    }

    fn resolve_tariff_cached(
        &mut self,
        connection_id: Option<&str>,
        platform: &str,
        model: &str,
    ) -> Option<ResolvedTariff> {
        let key = (
            connection_id.map(str::to_string),
            platform.to_string(),
            model.to_ascii_lowercase(),
        );
        if let Some(cached) = self.tariff_cache.get(&key) {
            return cached.clone();
        }
        let resolved = self.resolve_tariff_uncached(connection_id, platform, model, 0);
        self.tariff_cache.insert(key, resolved.clone());
        resolved
    }

    fn resolve_tariff_uncached(
        &self,
        connection_id: Option<&str>,
        platform: &str,
        model: &str,
        depth: u8,
    ) -> Option<ResolvedTariff> {
        if depth > 4 {
            return None;
        }
        let candidates = normalize_model_candidates(model);
        if candidates.is_empty() {
            return None;
        }

        // 1) 连接专属覆盖（不乘连接倍率）：先精确模型，再通配 `*`
        if let Some(cid) = connection_id {
            for cand in &candidates {
                if let Some(ov) = self.overrides.get(&(cid.to_string(), cand.clone())) {
                    if ov.disabled {
                        return None;
                    }
                    return Some(ResolvedTariff::from_override(ov, "connection_override", false));
                }
            }
            if let Some(ov) = self.overrides.get(&(cid.to_string(), "*".to_string())) {
                if !ov.disabled {
                    return Some(ResolvedTariff::from_override(ov, "connection_override", false));
                }
            }
            // 连接级同价映射：指向目标模型的基准价（受倍率缩放）
            for cand in &candidates {
                if let Some(target) = self.aliases.get(&(cid.to_string(), cand.clone())) {
                    if let Some(mut resolved) = self.resolve_base_tariff(platform, target, depth + 1) {
                        resolved.source_kind = "same_as";
                        resolved.alias_from = Some(target.clone());
                        resolved.source = format!("映射自 {target}");
                        return Some(resolved);
                    }
                }
            }
        }

        self.resolve_base_tariff(platform, model, depth)
    }

    fn resolve_base_tariff(&self, platform: &str, model: &str, depth: u8) -> Option<ResolvedTariff> {
        if depth > 4 {
            return None;
        }
        let candidates = normalize_model_candidates(model);
        if candidates.is_empty() {
            return None;
        }

        // 2) 全局用户覆盖（含墓碑）
        for cand in &candidates {
            if let Some(ov) = self.overrides.get(&(GLOBAL_SCOPE.to_string(), cand.clone())) {
                if ov.disabled {
                    return None;
                }
                return Some(ResolvedTariff::from_override(ov, "user_custom", true));
            }
        }

        // 3) 内置规则表（本平台优先，再跨平台）+ models.dev 补充
        let plat = (!platform.is_empty()).then_some(platform);
        for cand in &candidates {
            if let Some(p) = lookup_builtin_candidate(plat, cand) {
                let md = lookup_models_dev_candidate(&self.catalog, cand).map(|(_, _, c)| c);
                return Some(ResolvedTariff::from_builtin(p, md));
            }
        }
        for cand in &candidates {
            if let Some(p) = lookup_builtin_candidate(None, cand) {
                let md = lookup_models_dev_candidate(&self.catalog, cand).map(|(_, _, c)| c);
                return Some(ResolvedTariff::from_builtin(p, md));
            }
        }

        // 4) models.dev 价格目录
        for cand in &candidates {
            if let Some((id, vendor, cost)) = lookup_models_dev_candidate(&self.catalog, cand) {
                return Some(ResolvedTariff::from_models_dev(id, vendor, cost, plat));
            }
        }

        // 5) 全局同价映射（same_as）
        for cand in &candidates {
            if let Some(target) = self.aliases.get(&(GLOBAL_SCOPE.to_string(), cand.clone())) {
                if let Some(mut resolved) = self.resolve_base_tariff(platform, target, depth + 1) {
                    resolved.source_kind = "same_as";
                    resolved.alias_from = Some(target.clone());
                    resolved.source = format!("映射自 {target}");
                    return Some(resolved);
                }
            }
        }

        None
    }
}

fn compute_fresh_input(
    input_semantics: &str,
    platform: &str,
    input: i64,
    cache_read: i64,
    cache_write: i64,
) -> i64 {
    match input_semantics {
        "includes_cache" => input.saturating_sub(cache_read).saturating_sub(cache_write).max(0),
        "excludes_cache" => input.max(0),
        _ => match platform {
            "codex" => input.saturating_sub(cache_read).saturating_sub(cache_write).max(0),
            "gemini" => input.saturating_sub(cache_read).max(0),
            _ => input.max(0),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_tariff_cost(
    tariff: &ResolvedTariff,
    fresh_input: i64,
    output: i64,
    cache_read: i64,
    cache_write_5m: i64,
    cache_write_1h: i64,
    reasoning: i64,
    tool: i64,
    prompt_tokens: i64,
    force_gemini_long: bool,
    at: chrono::DateTime<chrono::Utc>,
    multiplier: f64,
    billing_mode: BillingMode,
) -> (f64, AppliedPrice) {
    use chrono::{Datelike, Timelike};

    let mut base_input = tariff.input;
    let mut base_output = tariff.output;
    let mut base_read = tariff.cache_read;
    let mut base_write = tariff.cache_write;
    let mut base_write_1h = tariff.cache_write_1h;
    let mut long_context = false;

    if let Some(gmodel) = tariff.gemini_long_model {
        if force_gemini_long || prompt_tokens > GEMINI_LONG_PROMPT_TOKENS {
            if let Some(tier) = GEMINI_LONG.iter().find(|t| t.model == gmodel) {
                base_input = tier.input;
                base_output = tier.output;
                base_read = tier.cached_input;
                long_context = true;
            }
        }
    } else if let Some(lc) = tariff.long_context {
        if prompt_tokens > lc.threshold {
            base_input *= lc.input_multiplier;
            base_output *= lc.output_multiplier;
            base_read *= lc.cached_multiplier;
            base_write = base_write.map(|v| v * lc.input_multiplier);
            base_write_1h = base_write_1h.map(|v| v * lc.input_multiplier);
            long_context = true;
        }
    } else if !tariff.tiers.is_empty() {
        let mut matched: Option<&ModelCostTier> = None;
        for tier in &tariff.tiers {
            if prompt_tokens > tier.above {
                matched = Some(tier);
            }
        }
        if let Some(tier) = matched {
            base_input = tier.input;
            base_output = tier.output;
            base_read = tier.cache_read;
            if tier.cache_write.is_some() {
                base_write = tier.cache_write;
            }
            if tier.cache_write_1h.is_some() {
                base_write_1h = tier.cache_write_1h;
            } else if tariff.is_claude && base_write_1h.is_some() {
                base_write_1h = Some(base_input * 2.0);
            }
            long_context = true;
        }
    }

    let off_peak = tariff.off_peak.is_some_and(|off| {
        off.windows
            .iter()
            .any(|w| in_off_peak(w, at.hour() as u8, at.weekday().number_from_monday() as u8))
    });
    let off_mult = tariff
        .off_peak
        .filter(|_| off_peak)
        .map_or(1.0, |off| off.multiplier);

    let scale = off_mult * multiplier;
    let eff_input = base_input * scale;
    let eff_output = base_output * scale;
    let eff_read = base_read * scale;
    let eff_write = base_write.map(|w| w * scale);
    let eff_write_1h = base_write_1h.or(base_write).map(|w| w * scale);

    let write_5m_unit = eff_write.unwrap_or(eff_input);
    let write_1h_unit = eff_write_1h.unwrap_or(write_5m_unit);

    let cost = if tariff.is_gemini {
        let ordinary_input = fresh_input + tool;
        let total_output = output + reasoning;
        (ordinary_input as f64 * eff_input
            + cache_read as f64 * eff_read
            + cache_write_5m as f64 * write_5m_unit
            + cache_write_1h as f64 * write_1h_unit
            + total_output as f64 * eff_output)
            / 1_000_000.0
    } else {
        (fresh_input as f64 * eff_input
            + output as f64 * eff_output
            + cache_read as f64 * eff_read
            + cache_write_5m as f64 * write_5m_unit
            + cache_write_1h as f64 * write_1h_unit)
            / 1_000_000.0
    };

    let applied = AppliedPrice {
        input: eff_input,
        cached_input: eff_read,
        output: eff_output,
        cache_write: eff_write,
        cache_write_1h: (cache_write_1h > 0).then_some(write_1h_unit),
        source: tariff.source.clone(),
        source_kind: tariff.source_kind.to_string(),
        pricing_model: tariff.pricing_model.clone(),
        alias_from: tariff.alias_from.clone(),
        multiplier,
        billing_mode: billing_mode.as_str().into(),
        long_context,
        off_peak,
    };

    (cost, applied)
}

fn in_off_peak(window: &OffPeakWindow, hour: u8, weekday: u8) -> bool {
    if !window.weekdays.contains(&weekday) {
        return false;
    }
    if window.full_day {
        return true;
    }
    if window.start_hour <= window.end_hour {
        hour >= window.start_hour && hour < window.end_hour
    } else {
        hour >= window.start_hour || hour < window.end_hour
    }
}

/* ───────────────────────── 无库快速入口（供网关预算、测试与无连接场景使用） ───────────────────────── */

fn resolve_stateless_tariff(platform: Option<&str>, model: &str) -> Option<ResolvedTariff> {
    let candidates = normalize_model_candidates(model);
    if candidates.is_empty() {
        return None;
    }
    let catalog = models_dev::active_catalog();
    if let Some(plat) = platform {
        for cand in &candidates {
            if let Some(p) = lookup_builtin_candidate(Some(plat), cand) {
                let md = lookup_models_dev_candidate(&catalog, cand).map(|(_, _, c)| c);
                return Some(ResolvedTariff::from_builtin(p, md));
            }
        }
    }
    for cand in &candidates {
        if let Some(p) = lookup_builtin_candidate(None, cand) {
            let md = lookup_models_dev_candidate(&catalog, cand).map(|(_, _, c)| c);
            return Some(ResolvedTariff::from_builtin(p, md));
        }
    }
    for cand in &candidates {
        if let Some((id, vendor, cost)) = lookup_models_dev_candidate(&catalog, cand) {
            return Some(ResolvedTariff::from_models_dev(id, vendor, cost, platform));
        }
    }
    None
}

#[cfg(test)]
pub fn estimate_usage(platform: Option<&str>, model: Option<&str>, usage: Usage) -> Option<f64> {
    estimate_usage_at(platform, model, usage, chrono::Utc::now())
}

#[cfg(test)]
pub fn estimate_usage_at(
    platform: Option<&str>,
    model: Option<&str>,
    usage: Usage,
    at: chrono::DateTime<chrono::Utc>,
) -> Option<f64> {
    let tariff = resolve_stateless_tariff(platform, model?)?;
    let plat = platform.unwrap_or("");
    let cache_read = usage.cache_read.max(0);
    let cache_write = usage.cache_write.max(0);
    let cache_write_1h = usage.cache_write_1h.clamp(0, cache_write);
    let cache_write_5m = cache_write - cache_write_1h;
    let fresh_input = compute_fresh_input("unknown", plat, usage.input, cache_read, cache_write);
    let prompt_tokens = fresh_input + cache_read + cache_write;
    let (cost, _) = compute_tariff_cost(
        &tariff,
        fresh_input,
        usage.output.max(0),
        cache_read,
        cache_write_5m,
        cache_write_1h,
        usage.reasoning.max(0),
        usage.tool.max(0),
        prompt_tokens,
        usage.long_prompt,
        at,
        1.0,
        BillingMode::Api,
    );
    Some(cost)
}

#[cfg(test)]
pub fn estimate(
    platform: Option<&str>,
    model: Option<&str>,
    input: i64,
    output: i64,
    cache_write: i64,
    cache_read: i64,
) -> Option<f64> {
    estimate_usage(
        platform,
        model,
        Usage { input, output, cache_write, cache_read, ..Default::default() },
    )
}

#[cfg(test)]
pub fn has_price(platform: Option<&str>, model: &str) -> bool {
    resolve_stateless_tariff(platform, model).is_some()
}

pub fn verified_on() -> &'static str {
    table::VERIFIED_ON
}

pub fn price_source(platform: Option<&str>, model: &str) -> Option<(String, &'static str)> {
    let tariff = resolve_stateless_tariff(platform, model)?;
    let checked = if tariff.source_kind == "builtin" {
        table::VERIFIED_ON
    } else {
        ""
    };
    Some((tariff.source, checked))
}

#[cfg(test)]
pub fn applied_price(
    platform: Option<&str>,
    model: &str,
    input: i64,
    at: chrono::DateTime<chrono::Utc>,
) -> Option<AppliedPrice> {
    let tariff = resolve_stateless_tariff(platform, model)?;
    let (_, applied) = compute_tariff_cost(
        &tariff,
        input.max(0),
        0,
        0,
        0,
        0,
        0,
        0,
        input.max(0),
        input > GEMINI_LONG_PROMPT_TOKENS,
        at,
        1.0,
        BillingMode::Api,
    );
    Some(applied)
}

/// 列出内置规则表的去重模型基线（供价目管理弹窗展示）。
pub(crate) fn builtin_entries() -> Vec<(String, f64, f64, f64, Option<f64>, &'static str, Option<i64>)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in PRICES {
        if !seen.insert(p.model) {
            continue;
        }
        let is_claude = p.model.starts_with("claude-") || p.platform == "claude";
        let cw = p.cache_write.or_else(|| is_claude.then_some(p.input * 1.25));
        let threshold = p.long_context.map(|l| l.threshold).or_else(|| {
            GEMINI_LONG
                .iter()
                .any(|g| g.model == p.model)
                .then_some(GEMINI_LONG_PROMPT_TOKENS)
        });
        out.push((p.model.to_string(), p.input, p.output, p.cached_input, cw, p.source, threshold));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_utc(y: i32, m: u32, d: u32, h: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn new_models_are_priced() {
        for (platform, model) in [
            ("claude", "claude-opus-5-5"),
            ("claude", "claude-sonnet-5-5"),
            ("claude", "claude-haiku-5-5"),
            ("codex", "gpt-6.1-sol"),
            ("codex", "gpt-6-sol"),
            ("zcode", "glm-5.3"),
            ("zcode", "deepseek-v4-flash"),
            ("zcode", "kimi-k3"),
        ] {
            assert!(has_price(Some(platform), model), "{platform}/{model} 应有价目");
            let cost = estimate_usage(
                Some(platform),
                Some(model),
                Usage { input: 1_000_000, output: 0, ..Default::default() },
            );
            assert!(cost.is_some_and(|c| c > 0.0), "{platform}/{model} 应能算出金额");
        }
    }

    #[test]
    fn date_snapshot_suffix_still_matches() {
        assert!(has_price(Some("claude"), "claude-sonnet-4-5-20250929"));
        assert!(has_price(Some("claude"), "claude-opus-4-6-20260205"));
        assert!(!has_price(Some("claude"), "claude-sonnet-4-5-custom"));
    }

    #[test]
    fn cross_platform_and_normalized_ids_are_priced() {
        assert!(has_price(Some("opencode"), "anthropic/claude-sonnet-4-5[1m]"));
        assert!(has_price(Some("custom"), "us.anthropic.claude-opus-4.6:high"));
        assert!(has_price(Some("workbuddy"), "glm-5.3"));
        assert!(has_price(Some("trae"), "deepseek-v4.1-flash:1017"));
    }

    #[test]
    fn claude_4x_cache_write_no_longer_drops_cost() {
        let cost = estimate_usage(
            Some("claude"),
            Some("claude-sonnet-4-5"),
            Usage { input: 1_000_000, output: 0, cache_write: 1_000_000, cache_read: 0, ..Default::default() },
        );
        assert!(cost.is_some_and(|c| c > 6.0), "Claude 4.5 有 cache_write 应按 1.25x 计入：{cost:?}");
    }

    #[test]
    fn long_context_multiplies_whole_request() {
        let short = estimate_usage_at(
            Some("codex"), Some("gpt-6.1-sol"),
            Usage { input: 272_000, output: 0, ..Default::default() },
            at_utc(2026, 11, 3, 12),
        ).unwrap();
        let long = estimate_usage_at(
            Some("codex"), Some("gpt-6.1-sol"),
            Usage { input: 272_001, output: 0, ..Default::default() },
            at_utc(2026, 11, 3, 12),
        ).unwrap();
        assert!(long > short * 1.99, "越界后整条请求应按 2 倍价计：{short} vs {long}");
    }

    #[test]
    fn glm_long_context_threshold_is_32k() {
        let short = estimate_usage_at(
            Some("zcode"), Some("glm-5.3"),
            Usage { input: 32_000, output: 0, ..Default::default() },
            at_utc(2026, 11, 3, 12),
        ).unwrap();
        let long = estimate_usage_at(
            Some("zcode"), Some("glm-5.3"),
            Usage { input: 32_001, output: 0, ..Default::default() },
            at_utc(2026, 11, 3, 12),
        ).unwrap();
        assert!((long / short - 1.5).abs() < 0.01, "GLM 越界后应 ×1.5：{short} vs {long}");
    }

    #[test]
    fn deepseek_off_peak_is_half_price_on_weekend() {
        let weekend = estimate_usage_at(
            Some("zcode"), Some("deepseek-v4-flash"),
            Usage { input: 1_000_000, output: 0, ..Default::default() },
            at_utc(2026, 11, 7, 12),
        ).unwrap();
        let weekday_peak = estimate_usage_at(
            Some("zcode"), Some("deepseek-v4-flash"),
            Usage { input: 1_000_000, output: 0, ..Default::default() },
            at_utc(2026, 11, 4, 8),
        ).unwrap();
        assert!((weekday_peak / weekend - 2.0).abs() < 0.01, "谷时应为半价：峰 {weekday_peak} 谷 {weekend}");
    }

    #[test]
    fn unknown_model_returns_none_not_zero() {
        assert_eq!(estimate_usage(Some("zcode"), Some("unknown-model-xyz"), Usage { input: 1000, output: 100, ..Default::default() }), None);
        assert!(!has_price(Some("zcode"), "unknown-model-xyz"));
    }

    #[test]
    fn applied_price_carries_official_source_and_gemini_long() {
        let at = at_utc(2026, 11, 3, 12);
        assert!(applied_price(Some("claude"), "claude-opus-5-5", 1000, at).unwrap().source.contains("platform.claude.com"));
        let g_short = applied_price(Some("gemini"), "gemini-2.5-pro", 100_000, at).unwrap();
        let g_long = applied_price(Some("gemini"), "gemini-2.5-pro", 250_000, at).unwrap();
        assert!(g_long.long_context && g_long.input > g_short.input);
    }

    #[test]
    fn resolver_precedence_multiplier_free_override_and_alias() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO connections(id,platform,kind,name,status,created_ms,billing_mode,price_multiplier)
             VALUES('c-relay','custom','api','二折中转','connected',1,'api',0.2),
                   ('c-free','claude','auth','免费号','connected',1,'free',1.0)",
            [],
        ).unwrap();
        // 全局自定义基准价（会乘连接倍率）
        store::upsert_override(&db, PricingOverride {
            scope: "*".into(),
            model_id: "my-custom-m".into(),
            display_name: "My Custom".into(),
            input: 10.0,
            output: 20.0,
            cache_read: 1.0,
            cache_write: Some(12.5),
            cache_write_1h: None,
            long_ctx_threshold: None,
            long_ctx_input: None,
            long_ctx_output: None,
            long_ctx_cache_read: None,
            long_ctx_cache_write: None,
            disabled: false,
            updated_ms: 0,
        }).unwrap();
        // 连接专属到手覆盖价（不乘连接倍率）
        store::upsert_override(&db, PricingOverride {
            scope: "c-relay".into(),
            model_id: "special-fixed".into(),
            display_name: "Fixed".into(),
            input: 4.0,
            output: 8.0,
            cache_read: 0.4,
            cache_write: None,
            cache_write_1h: None,
            long_ctx_threshold: None,
            long_ctx_input: None,
            long_ctx_output: None,
            long_ctx_cache_read: None,
            long_ctx_cache_write: None,
            disabled: false,
            updated_ms: 0,
        }).unwrap();
        store::upsert_alias(&db, "*", "relay-alias", "my-custom-m").unwrap();

        let mut resolver = PriceResolver::load(&db);
        let at = at_utc(2026, 11, 3, 12);

        // 全局基准价 × 0.2 倍率
        let r1 = resolver.price_request(&PriceRequest {
            platform: "custom",
            model: Some("my-custom-m"),
            upstream_model: None,
            served_model: None,
            connection_id: Some("c-relay"),
            input_semantics: "excludes_cache",
            input: Some(1_000_000),
            output: Some(0),
            cache_read: Some(0),
            cache_write: None, // 缺 cache_write 不再丢费用
            cache_write_1h: 0,
            reasoning: 0,
            tool: 0,
            at,
        });
        assert!((r1.cost.unwrap() - 2.0).abs() < 1e-6);
        assert!((r1.applied.unwrap().multiplier - 0.2).abs() < 1e-6);

        // 连接专属覆盖不乘倍率
        let r2 = resolver.price_request(&PriceRequest {
            platform: "custom",
            model: Some("special-fixed"),
            upstream_model: None,
            served_model: None,
            connection_id: Some("c-relay"),
            input_semantics: "excludes_cache",
            input: Some(1_000_000),
            output: Some(0),
            cache_read: Some(0),
            cache_write: Some(0),
            cache_write_1h: 0,
            reasoning: 0,
            tool: 0,
            at,
        });
        assert!((r2.cost.unwrap() - 4.0).abs() < 1e-6);
        assert_eq!(r2.applied.unwrap().source_kind, "connection_override");

        // 同价映射 × 0.2 倍率
        let r3 = resolver.price_request(&PriceRequest {
            platform: "custom",
            model: Some("relay-alias"),
            upstream_model: None,
            served_model: None,
            connection_id: Some("c-relay"),
            input_semantics: "excludes_cache",
            input: Some(1_000_000),
            output: Some(0),
            cache_read: Some(0),
            cache_write: Some(0),
            cache_write_1h: 0,
            reasoning: 0,
            tool: 0,
            at,
        });
        assert!((r3.cost.unwrap() - 2.0).abs() < 1e-6);
        assert_eq!(r3.applied.unwrap().source_kind, "same_as");

        // 免费连接返回 Some(0.0) 而非 None
        let r4 = resolver.price_request(&PriceRequest {
            platform: "claude",
            model: Some("unknown-free-model"),
            upstream_model: None,
            served_model: None,
            connection_id: Some("c-free"),
            input_semantics: "excludes_cache",
            input: Some(500),
            output: Some(100),
            cache_read: None,
            cache_write: None,
            cache_write_1h: 0,
            reasoning: 0,
            tool: 0,
            at,
        });
        assert_eq!(r4.cost, Some(0.0));
        assert_eq!(r4.applied.as_ref().map(|a| a.billing_mode.as_str()), Some("free"));

        // ZCode includes_cache 语义不重复计费缓存读
        let r5 = resolver.price_request(&PriceRequest {
            platform: "zcode",
            model: Some("my-custom-m"),
            upstream_model: None,
            served_model: None,
            connection_id: None,
            input_semantics: "includes_cache",
            input: Some(1_000_000),
            output: Some(0),
            cache_read: Some(800_000),
            cache_write: Some(0),
            cache_write_1h: 0,
            reasoning: 0,
            tool: 0,
            at,
        });
        // 200k fresh * 10 + 800k cache_read * 1 = 2.0 + 0.8 = 2.8
        assert!((r5.cost.unwrap() - 2.8).abs() < 1e-6);

        // 1h 缓存写按 2.0x 独立计费（5m 1.25x，1h 2.0x）
        let r_1h = resolver.price_request(&PriceRequest {
            platform: "claude",
            model: Some("claude-sonnet-4-5"),
            upstream_model: None,
            served_model: None,
            connection_id: None,
            input_semantics: "excludes_cache",
            input: Some(0),
            output: Some(0),
            cache_read: Some(0),
            cache_write: Some(1_000_000),
            cache_write_1h: 1_000_000,
            reasoning: 0,
            tool: 0,
            at,
        });
        assert!((r_1h.cost.unwrap() - 6.0).abs() < 1e-6, "Claude Sonnet 4.5 输入 $3，1h 缓存写应 $6/M：{:?}", r_1h.cost);
        assert!(r_1h.applied.unwrap().cache_write_1h.is_some());

        // 墓碑停用（disabled = 1）阻断内置与 models.dev 兜底
        store::disable_override(&db, "*", "claude-sonnet-4-5").unwrap();
        let mut resolver2 = PriceResolver::load(&db);
        assert!(!resolver2.has_price_for(None, "claude", "claude-sonnet-4-5"));

        // upstream vs request 计价模型来源切换
        let mut up_resolver = PriceResolver::load(&db).with_model_source("upstream");
        let mut req_resolver = PriceResolver::load(&db).with_model_source("request");
        let make_routed = || PriceRequest {
            platform: "claude",
            model: Some("my-custom-m"),
            upstream_model: Some("gpt-5"),
            served_model: None,
            connection_id: None,
            input_semantics: "excludes_cache",
            input: Some(1_000_000),
            output: Some(0),
            cache_read: Some(0),
            cache_write: Some(0),
            cache_write_1h: 0,
            reasoning: 0,
            tool: 0,
            at,
        };
        assert_eq!(up_resolver.price_request(&make_routed()).applied.unwrap().pricing_model, "gpt-5");
        assert_eq!(req_resolver.price_request(&make_routed()).applied.unwrap().pricing_model, "my-custom-m");
    }
}
