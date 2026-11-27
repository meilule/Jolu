//! 额度与余额的只读查询（§2.5 / §7.4）
//!
//! ## 查询来源
//!
//! | 来源 | 端点 | 凭证 | 能读到 |
//! |---|---|---|---|
//! | Claude 官方订阅 | `GET https://api.anthropic.com/api/oauth/usage` | OAuth accessToken | 5h / 7d 等窗口的已用百分比与重置时间 |
//! | Codex 官方订阅 | `GET https://chatgpt.com/backend-api/wham/usage` | Codex Auth accessToken | 5h / 7d 等实际返回的额度窗口 |
//! | Claude API | `GET /v1/organizations/usage_report/messages` 与 `cost_report` | Admin API Key | 组织今日 Token 与费用 |
//! | Codex API | `GET /v1/organization/usage/completions` 与 `costs` | Admin API Key | 组织今日 Token 与费用 |
//! | sub2api 网关 | `GET {base}/v1/usage` | 分组 API Key（Bearer） | 配额、限流窗口、钱包余额、今日/累计用量 |
//!
//! 订阅额度端点只接受 Auth 凭证；API Key 走组织用量端点。普通调用 Key 没有
//! Admin 权限时返回 403，界面回退本机会话统计，不把它误报为订阅额度或钱包余额。
//!
//! ## 纪律
//!
//! - 全部是 GET 只读，不发送任何提示内容或用量数据到第三方
//! - 查询失败**绝不渲染为 0% 或满格**：用 `QuotaState` 的五种状态区分，
//!   界面显示「—」并给出具体原因（§2.5）
//! - 遵守频率限制：429 单独成一种状态，附带退避提示
//! - 请求超时 15 秒，与 CodexBar 的取值一致

use serde::Serialize;
use std::time::Duration;


pub mod api_usage;
pub(crate) mod coding_plan;
pub(crate) mod gateway_balance;
pub mod oauth;
pub(crate) mod registry;
pub(crate) mod service_balance;
pub mod sub2api;
pub(crate) mod vendors;
#[cfg(test)]
mod tests;

pub use api_usage::*;
pub use oauth::*;
pub use sub2api::*;

/// 一个额度窗口。`used_percent` 为 None 时界面**不画进度条**（§7.4）
#[derive(Debug, Clone, Serialize)]
pub struct QuotaWindow {
    /// 窗口标识，如 5h / 7d / 1d
    pub key: String,
    /// 窗口说明，如「5 小时窗口」
    pub window_name: String,
    pub used_percent: Option<f64>,
    /// 已用 / 总量的文字描述，来源没给分母时为 None
    pub amount_text: Option<String>,
    /// 来源直接返回的精确剩余量；无此字段时不推断绝对数量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_text: Option<String>,
    /// 重置时间（来源原样透传的 ISO 字符串）
    pub resets_at: Option<String>,
}

/// 来源明确给出的独立额度范围；缺失的总量或剩余量不从其他字段推算。
#[derive(Debug, Clone, Serialize)]
pub struct QuotaAmountRange {
    pub key: String,
    pub name: String,
    pub total: Option<f64>,
    pub remaining: Option<f64>,
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    /// 仅在来源总量、已用量均明确且相容时提供，界面必须标注计算。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculated_remaining: Option<f64>,
    /// 来源明确表示不限量，不能与缺总量混为一类。
    pub unlimited: bool,
}

/// 账号信息保留来源。令牌快照只能提供历史到期记录，不能证明当前订阅有效。
#[derive(Debug, Clone, Serialize)]
pub struct AccountMetadata {
    pub subscription_status: &'static str,
    pub expires_at: Option<String>,
    pub subscription_source: &'static str,
    pub observed_at: Option<String>,
    pub reset_credits: Option<ResetCredits>,
    /// 来源返回或本机客户端记录的账号标识（邮箱 / 用户名）；连接本身没有邮箱时用于展示
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// 精确额度的原始范围，供需要区分来源与推算值的展示使用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_ranges: Option<Vec<QuotaAmountRange>>,
}
impl Default for AccountMetadata {
    fn default() -> Self {
        Self { subscription_status: "unknown", expires_at: None, subscription_source: "unavailable", observed_at: None, reset_credits: None, email: None, amount_ranges: None }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ResetCredits {
    pub available_count: Option<u64>,
    pub applicable_available_count: Option<u64>,
}
impl ResetCredits {
    pub fn parse(value: &serde_json::Value) -> Option<Self> {
        if !value.is_object() { return None; }
        let count = |key: &str| value.get(key).and_then(serde_json::Value::as_u64)
            .filter(|number| *number <= 9_007_199_254_740_991);
        let available_count = count("available_count");
        let applicable_available_count = count("applicable_available_count");
        (available_count.is_some() || applicable_available_count.is_some())
            .then_some(Self { available_count, applicable_available_count })
    }
}

/// 额度查询状态。**刻意不用 `Option<f64>`** —— 那样无法区分
/// 「查不到」与「真的是 0」，也无法把失败原因带到界面上（§2.5）
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[derive(Clone)]
pub enum QuotaState {
    /// 查询成功
    Ok {
        windows: Vec<QuotaWindow>,
        /// 套餐名原样透传，不由百分比推断
        plan: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        account: Option<AccountMetadata>,
    },
    /// 没有官方额度可查时，网关转发响应里限流头的最近一次观测（4.3，`confidence = derived`）。
    /// 与 `Ok` 分开：界面必须能和官方值区分，自动化等需要官方口径的地方按未知处理。
    Observed {
        windows: Vec<QuotaWindow>,
        observed_at_ms: i64,
        /// 本来的结论（为什么没有官方值），界面可一并说明
        reason: String,
    },
    /// 该接入方式不支持额度查询（如用 API Key 查官方订阅额度）
    Unsupported { reason: String },
    /// 凭证有效但无权限
    Forbidden { reason: String },
    /// 凭证失效或过期，需要重新授权
    Unauthorized { reason: String },
    /// 触发频率限制，需退避后重试
    RateLimited { reason: String },
    /// 网络错误、解析失败等
    Failed { reason: String },
}

/// 余额查询状态。真实余额为零时显示 `0.00`，查不到才显示「—」（§2.5）
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[derive(Clone)]
pub enum BalanceState {
    Ok {
        balance: f64,
        currency: String,
        /// 已用额度（若来源提供）
        used: Option<f64>,
    },
    Unsupported { reason: String },
    Forbidden { reason: String },
    Unauthorized { reason: String },
    RateLimited { reason: String },
    Failed { reason: String },
}

/// 官方直连在不同凭证类型下可提供的能力。订阅额度与 API 组织用量是两条独立链路，
/// 不能用 API Key 冒充 Auth，也不能把 Auth 当成组织管理 Key。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg(test)]
pub struct OfficialCapabilities {
    pub quota: bool,
    pub admin_usage: bool,
    pub local_sessions: bool,
}

#[cfg(test)]
pub fn official_capabilities(platform: &str, kind: &str) -> OfficialCapabilities {
    match (platform, kind) {
        ("claude" | "codex", "auth") => OfficialCapabilities {
            quota: true,
            admin_usage: false,
            local_sessions: true,
        },
        ("claude" | "codex", "api") => OfficialCapabilities {
            quota: false,
            admin_usage: true,
            local_sessions: true,
        },
        _ => OfficialCapabilities {
            quota: false,
            admin_usage: false,
            local_sessions: false,
        },
    }
}


/// 额度、余额与套餐查询共用的客户端。blocking 客户端每次新建都要起一条后台运行时线程、
/// 加载 TLS 根证书，且无法复用连接；同一轮刷新里的多次查询共用一个。
/// 系统代理只在构建时读取，因此定期重建，用户切换系统代理后很快生效；
/// 在设置里修改应用联网方式（`net::generation()` 变化）时立即重建。
/// 调用方都在阻塞线程池里（spawn_blocking），替换时丢弃旧客户端不会触发运行时析构限制。
pub fn client() -> Result<reqwest::blocking::Client, String> {
    static SHARED: std::sync::Mutex<Option<(reqwest::blocking::Client, std::time::Instant, u64)>> = std::sync::Mutex::new(None);
    // 先取代数再构建：构建期间联网方式又变了，存下的旧代数会让下一次调用重建。
    let generation = crate::net::generation();
    if let Ok(shared) = SHARED.lock() {
        if let Some((client, built, built_generation)) = shared.as_ref() {
            if cache_valid(*built, *built_generation, std::time::Instant::now(), generation) {
                return Ok(client.clone());
            }
        }
    }
    let client = crate::net::blocking_builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("无法创建 HTTP 客户端: {e}"))?;
    // 旧客户端在锁释放后才析构，它的后台线程收尾不占着锁。
    let previous = SHARED.lock().ok().and_then(|mut shared| shared.replace((client.clone(), std::time::Instant::now(), generation)));
    drop(previous);
    Ok(client)
}

/// 共享客户端是否还能复用：联网方式没变，且距上次构建不到 120 秒。
pub fn cache_valid(built_at: std::time::Instant, built_generation: u64, now: std::time::Instant, generation: u64) -> bool {
    const REBUILD_AFTER: Duration = Duration::from_secs(120);
    built_generation == generation && now.saturating_duration_since(built_at) < REBUILD_AFTER
}

/// HTTP 状态码 → 面向用户的原因文案。
/// 401 / 403 / 429 必须分开，含义与后续动作都不同。
pub enum HttpOutcome {
    Body(String),
    Unauthorized(String),
    Forbidden(String),
    RateLimited(String),
    Failed(String),
}

pub fn read(resp: reqwest::blocking::Response) -> HttpOutcome {
    let status = resp.status();
    let code = status.as_u16();
    match code {
        200..=299 => match resp.text() {
            Ok(t) => HttpOutcome::Body(t),
            Err(e) => HttpOutcome::Failed(format!("读取响应失败: {e}")),
        },
        401 => HttpOutcome::Unauthorized("凭证已失效或过期，需要重新授权".into()),
        403 => HttpOutcome::Forbidden("该凭证没有查询此数据的权限".into()),
        429 => {
            // Retry-After 若存在则原样告知，让用户知道等多久
            let after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .map(|s| format!("，请等待 {s} 秒后重试"))
                .unwrap_or_default();
            HttpOutcome::RateLimited(format!("已触发平台频率限制{after}"))
        }
        500..=599 => HttpOutcome::Failed(format!("服务端错误（HTTP {code}）")),
        _ => HttpOutcome::Failed(format!("查询失败（HTTP {code}）")),
    }
}


/// 保存 / 恢复 / 更换 Key 前的连接校验。`model` 是连接的默认模型（获取所得或手填）。
pub fn validate_connection(
    platform: &str,
    kind: &str,
    secret: &str,
    base_url: Option<&str>,
    model: Option<&str>,
) -> Result<(), String> {
    validate_connection_with(platform, kind, model, || validate_remote(platform, kind, secret, base_url))
}

/// 自定义供应商（D-11）：以「获取模型成功」或「手填模型」作为接口可达的依据，不调用额度接口；
/// 其他平台照旧走 `remote`（额度 / 模型接口）校验。
pub fn validate_connection_with(
    platform: &str,
    kind: &str,
    model: Option<&str>,
    remote: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if !matches!(platform,"custom"|"opencode"|"ollama") {
        return remote();
    }
    if kind != "api" {
        return Err("自定义供应商只支持 API Key 连接".into());
    }
    if model.is_some_and(|m| !m.trim().is_empty()) {
        Ok(())
    } else {
        Err("请先获取模型并选择，或手动填写模型 ID".into())
    }
}

pub fn validate_remote(
    platform: &str,
    kind: &str,
    secret: &str,
    base_url: Option<&str>,
) -> Result<(), String> {
    if kind == "api" && crate::provider_key::supported(platform) {
        return crate::provider_key::validate(platform, secret, base_url);
    }
    if let Some(base) = base_url.filter(|base| kind == "api" && !base.is_empty()) {
        // 套餐供应商（智谱/Kimi/MiniMax/ZenMux/OpenCode Go/火山）优先于 sub2api 网关路由
        if crate::coding_plan::detect_provider(base).is_some() {
            return match crate::coding_plan::coding_plan_quota(base, secret) {
                QuotaState::Ok { .. } => Ok(()),
                QuotaState::Unsupported { reason }
                | QuotaState::Forbidden { reason }
                | QuotaState::Unauthorized { reason }
                | QuotaState::RateLimited { reason }
                | QuotaState::Failed { reason }
                | QuotaState::Observed { reason, .. } => Err(reason),
            };
        }
        return match fetch_s2(base, secret) {
            Ok(data) if s2_can_validate(&data) => Ok(()),
            Ok(_) => Err("网关未返回可识别的用量、额度或余额数据".into()),
            Err((_, reason)) => Err(reason),
        };
    }

    match (platform, kind) {
        ("claude", "auth") => match claude_oauth_quota(secret) {
            QuotaState::Ok { .. } => Ok(()),
            QuotaState::Unsupported { reason }
            | QuotaState::Forbidden { reason }
            | QuotaState::Unauthorized { reason }
            | QuotaState::RateLimited { reason }
            | QuotaState::Failed { reason }
            | QuotaState::Observed { reason, .. } => Err(reason),
        },
        ("codex", "auth") => {
            let account = crate::creds::local_codex_auth()
                .and_then(|(token, account)| (token == secret).then_some(account).flatten());
            match codex_oauth_quota(secret, account.as_deref()) {
                QuotaState::Ok { .. } => Ok(()),
                QuotaState::Unsupported { reason }
                | QuotaState::Forbidden { reason }
                | QuotaState::Unauthorized { reason }
                | QuotaState::RateLimited { reason }
                | QuotaState::Failed { reason }
                | QuotaState::Observed { reason, .. } => Err(reason),
            }
        }
        ("claude" | "codex", "api") => validate_official_api_key(platform, secret),
        _ => Err("当前平台或接入方式尚未支持".into()),
    }
}
