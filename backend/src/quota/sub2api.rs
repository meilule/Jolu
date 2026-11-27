use super::*;
use serde::Deserialize;

/* ---------------------- sub2api 网关：GET /v1/usage ---------------------- */

#[derive(Debug, Deserialize)]
pub struct S2Quota {
    pub limit: f64,
    pub used: f64,
    pub unit: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct S2RateLimit {
    /// 5h / 1d / 7d
    pub window: String,
    pub limit: f64,
    pub used: f64,
    pub reset_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct S2Subscription {
    pub daily_usage_usd: Option<f64>,
    pub weekly_usage_usd: Option<f64>,
    pub monthly_usage_usd: Option<f64>,
    pub daily_limit_usd: Option<f64>,
    pub weekly_limit_usd: Option<f64>,
    pub monthly_limit_usd: Option<f64>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct S2Usage {
    #[serde(rename = "isValid")]
    pub is_valid: Option<bool>,
    #[serde(rename = "planName")]
    pub plan_name: Option<String>,
    pub balance: Option<f64>,
    pub unit: Option<String>,
    pub quota: Option<S2Quota>,
    pub subscription: Option<S2Subscription>,
    #[serde(default)]
    pub rate_limits: Vec<S2RateLimit>,
}

pub fn s2_can_validate(data: &S2Usage) -> bool {
    data.is_valid != Some(false) && (data.balance.is_some_and(f64::is_finite)
        || data.quota.is_some() || data.subscription.is_some() || !data.rate_limits.is_empty())
}

/// 拼出 `{base}/v1/usage`，容忍用户把 base 填成 `.../v1` 或 `.../v1/usage`
pub fn s2_endpoint(base: &str) -> String {
    let mut b = base.trim_end_matches('/').to_string();
    if !(b.ends_with("/v1") || b.ends_with("/v1/usage")) {
        b.push_str("/v1");
    }
    if !b.ends_with("/usage") {
        b.push_str("/usage");
    }
    b
}

/// 百分比：分母缺失或为 0 时返回 None —— 除零会得到 inf，把进度条画坏
pub fn pct(used: f64, limit: f64) -> Option<f64> {
    if !used.is_finite() || !limit.is_finite() || used < 0.0 || limit <= 0.0 {
        None
    } else {
        Some((used / limit * 100.0).clamp(0.0, 100.0))
    }
}

pub fn amount(used: f64, limit: f64, unit: &str) -> String {
    format!("{used:.2} / {limit:.2} {unit}")
}

/// fetch_s2 对 HTTP 404 的固定文案；分流层（lib.rs）据此识别「网关没有账单端点」
/// 并转入编程套餐自动探测。404 ≠ 查询失败：Claude 式反代、本地中转都没有账单接口
pub const S2_ENDPOINT_MISSING: &str = "该网关未提供 sub2api 账单接口（HTTP 404）";

pub fn fetch_s2(base: &str, api_key: &str) -> Result<S2Usage, (u16, String)> {
    let c = client().map_err(|e| (0, e))?;
    let resp = c
        .get(s2_endpoint(base))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
        .map_err(|e| {
            if e.is_timeout() {
                (0, "查询超时（15 秒）".to_string())
            } else {
                (0, format!("网络错误: {e}"))
            }
        })?;

    // 404 = 网关没有此端点（Claude 式反代、本地中转都没有账单接口），单独给码，
    // 避免落进笼统的「查询失败」让用户误以为是配置错误
    if resp.status().as_u16() == 404 {
        return Err((404, S2_ENDPOINT_MISSING.into()));
    }

    match read(resp) {
        HttpOutcome::Body(b) => {
            let data: S2Usage = serde_json::from_str(&b).map_err(|e| (200, format!("响应格式无法解析: {e}")))?;
            if data.is_valid == Some(false) { return Err((401, "网关标记当前凭证无效".into())); }
            Ok(data)
        }
        HttpOutcome::Unauthorized(r) => Err((401, r)),
        HttpOutcome::Forbidden(r) => Err((403, r)),
        HttpOutcome::RateLimited(r) => Err((429, r)),
        HttpOutcome::Failed(r) => Err((500, r)),
    }
}

/// 余额侧的错误映射，与 [`s2_quota_err`] 对称。
/// 404 归为「不支持」：没有该端点的网关（Claude 式反代、本地中转）不是配置错误
pub fn s2_balance_err(code: u16, reason: String) -> BalanceState {
    match code {
        401 => BalanceState::Unauthorized { reason },
        403 => BalanceState::Forbidden { reason },
        429 => BalanceState::RateLimited { reason },
        404 => BalanceState::Unsupported { reason },
        _ => BalanceState::Failed { reason },
    }
}

/// 按 HTTP 语义把错误映射到状态，401/403/429 各自成态
pub fn s2_quota_err(code: u16, reason: String) -> QuotaState {
    match code {
        401 => QuotaState::Unauthorized { reason },
        403 => QuotaState::Forbidden { reason },
        429 => QuotaState::RateLimited { reason },
        404 => QuotaState::Unsupported { reason },
        _ => QuotaState::Failed { reason },
    }
}

pub fn sub2api_quota(base: &str, api_key: &str) -> QuotaState {
    let d = match fetch_s2(base, api_key) {
        Ok(d) => d,
        Err((code, reason)) => return s2_quota_err(code, reason),
    };

    s2_quota_from_usage(d)
}

pub fn s2_quota_from_usage(d: S2Usage) -> QuotaState {
    let unit = d.unit.clone().unwrap_or_else(|| "USD".into());
    let mut windows = Vec::new();

    // 订阅型分组：日 / 周 / 月消费对上限
    if let Some(s) = &d.subscription {
        for (key, name, used, limit) in [
            ("1d", "日消费", s.daily_usage_usd, s.daily_limit_usd),
            ("7d", "周消费", s.weekly_usage_usd, s.weekly_limit_usd),
            ("30d", "月消费", s.monthly_usage_usd, s.monthly_limit_usd),
        ] {
            // 缺失用量必须保持未知，不能以 0.00 冒充服务端返回的真实零值。
            let (percent, text) = match (used, limit) {
                (Some(used), Some(limit)) if limit > 0.0 => {
                    (pct(used, limit), Some(amount(used, limit, &unit)))
                }
                (Some(used), _) => (None, Some(format!("{used:.2} {unit}"))),
                (None, Some(limit)) if limit > 0.0 => {
                    (None, Some(format!("— / {limit:.2} {unit}")))
                }
                (None, _) => (None, None),
            };
            windows.push(QuotaWindow {
                key: key.into(),
                window_name: name.into(),
                used_percent: percent,
                amount_text: text,
                // expires_at 是订阅到期时间，不是日/周/月额度的重置时刻。
                remaining_text: None,
                resets_at: None,
            });
        }
    }

    // 配额型分组：总配额
    if let Some(q) = &d.quota {
        let u = q.unit.clone().unwrap_or_else(|| unit.clone());
        windows.push(QuotaWindow {
            key: "quota".into(),
            window_name: "总配额".into(),
            used_percent: pct(q.used, q.limit),
            amount_text: Some(amount(q.used, q.limit, &u)),
            remaining_text: None,
            resets_at: None,
        });
    }

    // 限流窗口：来源已给出 window 标识，按其原样映射名称
    for r in &d.rate_limits {
        let name = match r.window.to_ascii_lowercase().as_str() {
            "5h" => "5 小时窗口".to_string(),
            "1d" => "日限额".to_string(),
            "7d" => "7 天窗口".to_string(),
            other => format!("{other} 窗口"),
        };
        windows.push(QuotaWindow {
            key: r.window.clone(),
            window_name: name,
            used_percent: pct(r.used, r.limit),
            amount_text: Some(amount(r.used, r.limit, &unit)),
            remaining_text: None,
            resets_at: r.reset_at.clone(),
        });
    }

    if windows.is_empty() {
        return QuotaState::Unsupported {
            reason: "该分组未提供额度窗口；钱包余额可在主面板查看".into(),
        };
    }
    QuotaState::Ok {
        windows,
        plan: d.plan_name,
        account: d.subscription.as_ref().map(|subscription| AccountMetadata {
            expires_at: subscription.expires_at.as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.to_rfc3339()),
            // 服务端的到期时间可以展示，但不能据此猜测自动续费、停用或订阅状态。
            subscription_source: "live",
            observed_at: Some(chrono::Utc::now().to_rfc3339()),
            ..AccountMetadata::default()
        }),
    }
}

pub fn sub2api_balance(base: &str, api_key: &str) -> BalanceState {
    let d = match fetch_s2(base, api_key) {
        Ok(d) => d,
        Err((code, reason)) => return s2_balance_err(code, reason),
    };

    match d.balance {
        // 余额 0 是真实值，必须显示 0.00，不能当成「查不到」
        Some(b) => BalanceState::Ok {
            balance: b,
            currency: d.unit.unwrap_or_else(|| "USD".into()),
            used: d.quota.map(|q| q.used),
        },
        // 该分组不是钱包型，来源本就不提供余额
        None => BalanceState::Unsupported {
            reason: "该分组未提供钱包余额（可能是配额型或订阅型分组）".into(),
        },
    }
}
