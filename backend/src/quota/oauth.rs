use super::*;
use serde::Deserialize;
use std::time::Duration;

pub const TIMEOUT: Duration = Duration::from_secs(15);

/// 官方 OAuth 用量端点当前需要该 beta 头
pub const ANTHROPIC_BASE: &str = "https://api.anthropic.com";
pub const ANTHROPIC_USAGE_PATH: &str = "/api/oauth/usage";
pub const ANTHROPIC_BETA: &str = "oauth-2025-04-20";

#[derive(Deserialize)]
pub struct CodexWindow {
    pub used_percent: Option<f64>,
    pub limit_window_seconds: i64,
    pub reset_at: Option<i64>,
}

#[derive(Deserialize)]
pub struct CodexLimits {
    pub primary_window: Option<CodexWindow>,
    pub secondary_window: Option<CodexWindow>,
}

#[derive(Deserialize)]
pub struct CodexUsage {
    pub plan_type: Option<String>,
    pub rate_limit: Option<CodexLimits>,
    pub rate_limit_reset_credits: Option<serde_json::Value>,
}

pub fn parse_codex_usage(body: &str) -> QuotaState {
    let usage: CodexUsage = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return QuotaState::Failed { reason: "Codex 额度响应格式无效".into() },
    };
    let windows: Vec<QuotaWindow> = usage.rate_limit.into_iter()
        .flat_map(|r| [r.primary_window, r.secondary_window]).flatten()
        .filter(|w| w.limit_window_seconds > 0)
        .map(|w| {
            // primary 不一定是 5h：当前 Pro 账号的 primary 就是 7d。
            let (key, name) = match w.limit_window_seconds {
                18000 => ("5h".into(), "5 小时额度".into()),
                604800 => ("7d".into(), "周额度".into()),
                seconds => (format!("{}m", seconds / 60), format!("{} 分钟额度", seconds / 60)),
            };
            QuotaWindow {
                key, window_name: name,
                used_percent: w.used_percent.filter(|v| v.is_finite() && (0.0..=100.0).contains(v)),
                amount_text: None,
                remaining_text: None,
                resets_at: w.reset_at.and_then(|s| chrono::DateTime::from_timestamp(s, 0))
                    .map(|t| t.to_rfc3339()),
            }
        }).collect();
    if windows.is_empty() { return QuotaState::Failed { reason: "Codex 未返回可识别的额度窗口".into() }; }
    let reset_credits = usage.rate_limit_reset_credits.as_ref().and_then(ResetCredits::parse);
    QuotaState::Ok { windows, plan: usage.plan_type, account: Some(AccountMetadata {
        reset_credits, ..AccountMetadata::default()
    }) }
}

pub fn codex_oauth_quota(access_token: &str, account: Option<&str>) -> QuotaState {
    // 固定官方地址并禁止重定向，凭证不会发送到自定义地址或重定向目标。
    let client = match client() {
        Ok(c) => c,
        Err(_) => return QuotaState::Failed { reason: "无法创建 Codex 额度查询客户端".into() },
    };
    let mut request = client.get("https://chatgpt.com/backend-api/wham/usage")
        .bearer_auth(access_token).header("Accept", "application/json");
    if let Some(account) = account { request = request.header("ChatGPT-Account-Id", account); }
    match request.send() {
        Ok(r) => match read(r) {
            HttpOutcome::Body(body) => {
                let mut state = parse_codex_usage(&body);
                if let QuotaState::Ok { account: Some(metadata), .. } = &mut state {
                    if let Some((expires_at, observed_at)) = crate::login_detect::codex_subscription_for_secret(access_token) {
                        metadata.expires_at = Some(expires_at);
                        metadata.observed_at = observed_at;
                        metadata.subscription_source = "local_snapshot";
                    }
                }
                state
            },
            HttpOutcome::Unauthorized(reason) => QuotaState::Unauthorized { reason },
            HttpOutcome::Forbidden(reason) => QuotaState::Forbidden { reason },
            HttpOutcome::RateLimited(reason) => QuotaState::RateLimited { reason },
            HttpOutcome::Failed(reason) => QuotaState::Failed { reason },
        },
        Err(_) => QuotaState::Failed { reason: "Codex 额度查询失败，请检查网络后刷新".into() },
    }
}

pub fn local_codex_quota() -> QuotaState {
    let Some((token, account)) = crate::creds::local_codex_auth() else {
        return QuotaState::Unauthorized { reason: "未找到本机 Codex Auth，请先在 Codex 中登录".into() };
    };
    codex_oauth_quota(&token, account.as_deref())
}


/* ------------------- Claude 官方订阅：/api/oauth/usage ------------------- */

#[derive(Debug, Deserialize)]
pub struct OAuthWindow {
    /// 已用比例。来源给的是 0–100 的百分数
    pub utilization: Option<f64>,
    pub resets_at: Option<String>,
}

/// 只取已确认存在的窗口字段。来源新增字段时不会解析失败（serde 默认忽略未知键）。
#[derive(Debug, Deserialize)]
pub struct OAuthUsage {
    pub five_hour: Option<OAuthWindow>,
    pub seven_day: Option<OAuthWindow>,
    pub seven_day_opus: Option<OAuthWindow>,
    pub seven_day_sonnet: Option<OAuthWindow>,
}

/// 查询 Claude 官方订阅额度。
///
/// **调用方必须先确认连接类型是 `auth`**：该端点只认 OAuth accessToken，
/// 传 API Key 会返回 401，那时应报「不支持」而非「凭证失效」。
pub fn claude_oauth_quota(access_token: &str) -> QuotaState {
    let c = match client() {
        Ok(c) => c,
        Err(e) => return QuotaState::Failed { reason: e },
    };

    let resp = c
        .get(format!("{ANTHROPIC_BASE}{ANTHROPIC_USAGE_PATH}"))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .header("anthropic-beta", ANTHROPIC_BETA)
        .send();

    let body = match resp {
        Ok(r) => match read(r) {
            HttpOutcome::Body(b) => b,
            HttpOutcome::Unauthorized(reason) => return QuotaState::Unauthorized { reason },
            HttpOutcome::Forbidden(reason) => return QuotaState::Forbidden { reason },
            HttpOutcome::RateLimited(reason) => return QuotaState::RateLimited { reason },
            HttpOutcome::Failed(reason) => return QuotaState::Failed { reason },
        },
        Err(e) if e.is_timeout() => {
            return QuotaState::Failed {
                reason: "查询超时（15 秒）".into(),
            }
        }
        Err(e) => {
            return QuotaState::Failed {
                reason: format!("网络错误: {e}"),
            }
        }
    };

    let parsed: OAuthUsage = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return QuotaState::Failed {
                reason: format!("响应格式无法解析: {e}"),
            }
        }
    };

    // 只放入来源真的给了数据的窗口；缺失的窗口不补 0%（§7.4）
    let mut windows = Vec::new();
    for (key, name, w) in [
        ("5h", "5 小时窗口", parsed.five_hour),
        ("7d", "7 天窗口", parsed.seven_day),
        ("7d-opus", "7 天窗口 · Opus", parsed.seven_day_opus),
        ("7d-sonnet", "7 天窗口 · Sonnet", parsed.seven_day_sonnet),
    ] {
        let Some(w) = w else { continue };
        // utilization 缺失时保留该窗口但不给百分比，界面只显示重置时间、不画条
        windows.push(QuotaWindow {
            key: key.into(),
            window_name: name.into(),
            used_percent: w.utilization.filter(|v| v.is_finite() && (0.0..=100.0).contains(v)),
            amount_text: None,
            remaining_text: None,
            resets_at: w.resets_at,
        });
    }

    if windows.is_empty() {
        return QuotaState::Failed {
            reason: "该账号未返回任何额度窗口".into(),
        };
    }
    // 当前官方用量响应不含可信套餐档位，保持未知而不是复用连接类型标签。
    QuotaState::Ok { windows, plan: None, account: None }
}
