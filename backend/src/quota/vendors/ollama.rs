//! Ollama Cloud 订阅识别与额度查询。
//!
//! 官方有两条路径：
//! - 本机 `ollama signin`：本机服务代为认证，额度从 `/api/balance` 读取；
//! - `https://ollama.com/api` + API Key：直接访问 Cloud。
//!
//! 这里只读官方接口，不复制或刷新 Ollama 的密钥。

use crate::{
    login_detect::LoginDetection,
    quota::{AccountMetadata, QuotaState, QuotaWindow},
};
use serde::Deserialize;
use std::time::Duration;

const DEFAULT_LOCAL_BASE: &str = "http://127.0.0.1:11434";

#[derive(Debug, Deserialize, Default)]
pub struct OllamaUser {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub plan: String,
}

impl OllamaUser {
    pub fn account(&self) -> Option<String> {
        [&self.email, &self.name]
            .into_iter()
            .map(|value| value.trim())
            .find(|value| !value.is_empty())
            .map(str::to_string)
    }

    pub fn plan(&self) -> Option<String> {
        (!self.plan.trim().is_empty()).then(|| self.plan.trim().to_string())
    }
}

pub enum UserError {
    Unauthorized(String),
    Failed(String),
}

impl UserError {
    pub fn reason(self) -> String {
        match self {
            Self::Unauthorized(reason) | Self::Failed(reason) => reason,
        }
    }
}

fn local_base() -> String {
    std::env::var("OLLAMA_HOST")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.starts_with("http://") || value.starts_with("https://") {
                value
            } else {
                format!("http://{value}")
            }
        })
        .unwrap_or_else(|| DEFAULT_LOCAL_BASE.to_string())
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        // 本机 Ollama 不能走用户配置的系统代理。
        .no_proxy()
        .build()
        .map_err(|error| format!("无法创建 Ollama 查询客户端: {error}"))
}

pub fn local_user() -> Result<OllamaUser, UserError> {
    let client = client().map_err(UserError::Failed)?;
    let response = client
        .post(format!("{}/api/me", local_base()))
        .send()
        .map_err(|error| {
            UserError::Failed(if error.is_timeout() {
                "连接本机 Ollama 超时，请确认服务已启动".into()
            } else {
                format!("连接本机 Ollama 失败: {error}")
            })
        })?;
    match response.status().as_u16() {
        200..=299 => response
            .json::<OllamaUser>()
            .map_err(|error| UserError::Failed(format!("Ollama 账号响应无法解析: {error}")))
            .and_then(|user| {
                if user.account().is_some() {
                    Ok(user)
                } else {
                    Err(UserError::Unauthorized(
                        "本机 Ollama 尚未登录，请先运行 `ollama signin`".into(),
                    ))
                }
            }),
        401 => Err(UserError::Unauthorized(
            "本机 Ollama 尚未登录，请先运行 `ollama signin`".into(),
        )),
        404 => Err(UserError::Failed(
            "当前 Ollama 版本过旧，请更新后使用订阅登录".into(),
        )),
        status => Err(UserError::Failed(format!(
            "读取 Ollama 账号失败（HTTP {status}）"
        ))),
    }
}

pub fn detect() -> LoginDetection {
    match local_user() {
        Ok(user) => LoginDetection {
            found: true,
            account: user.account(),
            plan: user.plan(),
            sources: vec!["本机 Ollama 登录".into()],
            ..Default::default()
        },
        Err(error) => LoginDetection {
            message: Some(error.reason()),
            ..Default::default()
        },
    }
}

#[derive(Debug, Deserialize, Default)]
struct BalanceResponse {
    #[serde(default)]
    included: Option<IncludedBalance>,
    #[serde(default)]
    purchased: Option<PurchasedBalance>,
}

#[derive(Debug, Deserialize, Default)]
struct IncludedBalance {
    #[serde(default)]
    balance_usd: Option<f64>,
    #[serde(default)]
    allowance_usd: Option<f64>,
    #[serde(default)]
    period: Option<BalancePeriod>,
    #[serde(default)]
    session: Option<LegacyLimit>,
    #[serde(default)]
    weekly: Option<LegacyLimit>,
}

#[derive(Debug, Deserialize, Default)]
struct PurchasedBalance {
    #[serde(default)]
    balance_usd: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
struct BalancePeriod {
    #[serde(default)]
    until: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct LegacyLimit {
    #[serde(default)]
    remaining_percent: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
}

fn money(value: f64) -> String {
    format!("${value:.2}")
}

fn windows_from_balance(balance: &BalanceResponse) -> Vec<QuotaWindow> {
    let mut windows = Vec::new();
    if let Some(included) = &balance.included {
        if let (Some(remaining), Some(allowance)) = (included.balance_usd, included.allowance_usd) {
            let used_percent = (allowance > 0.0)
                .then(|| ((allowance - remaining) / allowance * 100.0).clamp(0.0, 100.0));
            windows.push(QuotaWindow {
                key: "included".into(),
                window_name: "本期订阅额度".into(),
                used_percent,
                amount_text: Some(format!("剩余 {} / {}", money(remaining), money(allowance))),
                remaining_text: Some(money(remaining)),
                resets_at: included.period.as_ref().and_then(|period| period.until.clone()),
            });
        }
        for (key, name, limit) in [
            ("session", "会话额度", included.session.as_ref()),
            ("weekly", "每周额度", included.weekly.as_ref()),
        ] {
            let Some(limit) = limit else { continue };
            windows.push(QuotaWindow {
                key: key.into(),
                window_name: name.into(),
                used_percent: limit
                    .remaining_percent
                    .map(|remaining| (100.0 - remaining).clamp(0.0, 100.0)),
                amount_text: limit
                    .remaining_percent
                    .map(|remaining| format!("剩余 {remaining:.0}%")),
                remaining_text: limit
                    .remaining_percent
                    .map(|remaining| format!("{remaining:.0}%")),
                resets_at: limit.resets_at.clone(),
            });
        }
    }
    if let Some(remaining) = balance
        .purchased
        .as_ref()
        .and_then(|purchased| purchased.balance_usd)
    {
        windows.push(QuotaWindow {
            key: "credits".into(),
            window_name: "已购额度".into(),
            used_percent: None,
            amount_text: Some(format!("剩余 {}", money(remaining))),
            remaining_text: Some(money(remaining)),
            resets_at: None,
        });
    }
    windows
}

fn balance_endpoint(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let root = base.strip_suffix("/v1").unwrap_or(base);
    format!("{root}/api/balance")
}

fn query_balance(base: &str, secret: Option<&str>) -> Result<BalanceResponse, QuotaState> {
    let client = client().map_err(|reason| QuotaState::Failed { reason })?;
    let mut request = client.get(balance_endpoint(base));
    if let Some(secret) = secret.filter(|value| !value.trim().is_empty()) {
        request = request.bearer_auth(secret);
    }
    let response = request.send().map_err(|error| QuotaState::Failed {
        reason: if error.is_timeout() {
            "Ollama 额度查询超时".into()
        } else {
            format!("Ollama 额度查询网络错误: {error}")
        },
    })?;
    match response.status().as_u16() {
        200..=299 => response
            .json::<BalanceResponse>()
            .map_err(|error| QuotaState::Failed {
                reason: format!("Ollama 余额响应无法解析: {error}"),
            }),
        401 => Err(QuotaState::Unauthorized {
            reason: "Ollama 登录已失效，请重新运行 `ollama signin` 或更换 API Key".into(),
        }),
        403 => Err(QuotaState::Forbidden {
            reason: "当前 Ollama 账号无权读取套餐余额".into(),
        }),
        429 => Err(QuotaState::RateLimited {
            reason: "Ollama 余额接口触发频率限制，请稍后重试".into(),
        }),
        404 => Err(QuotaState::Unsupported {
            reason: "当前 Ollama 版本不提供订阅余额接口，请更新 Ollama".into(),
        }),
        status => Err(QuotaState::Failed {
            reason: format!("Ollama 余额查询失败（HTTP {status}）"),
        }),
    }
}

fn quota_from_balance(
    balance: BalanceResponse,
    plan: Option<String>,
    account: Option<String>,
) -> QuotaState {
    let windows = windows_from_balance(&balance);
    if windows.is_empty() {
        return QuotaState::Unsupported {
            reason: "Ollama 未返回可展示的套餐额度".into(),
        };
    }
    QuotaState::Ok {
        windows,
        plan,
        account: Some(AccountMetadata {
            email: account,
            ..Default::default()
        }),
    }
}

pub fn local_quota() -> QuotaState {
    let user = match local_user() {
        Ok(user) => user,
        Err(UserError::Unauthorized(reason)) => return QuotaState::Unauthorized { reason },
        Err(UserError::Failed(reason)) => return QuotaState::Failed { reason },
    };
    match query_balance(&local_base(), None) {
        Ok(balance) => quota_from_balance(balance, user.plan(), user.account()),
        Err(state) => state,
    }
}

/// Cloud API Key 的余额窗口。base 可以是 `https://ollama.com/v1` 或本机 `/v1`。
pub fn api_quota(base: &str, secret: &str) -> QuotaState {
    match query_balance(base, Some(secret)) {
        Ok(balance) => quota_from_balance(balance, None, None),
        Err(state) => state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_plan_balance_maps_included_and_purchased() {
        let balance: BalanceResponse = serde_json::from_str(r#"{
          "included":{"balance_usd":72.5,"allowance_usd":100,"period":{"from":"2026-09-15T09:30:00Z","until":"2026-10-15T09:30:00Z"}},
          "purchased":{"balance_usd":25}
        }"#).unwrap();
        let windows = windows_from_balance(&balance);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].key, "included");
        assert!((windows[0].used_percent.unwrap() - 27.5).abs() < 1e-9);
        assert_eq!(windows[0].remaining_text.as_deref(), Some("$72.50"));
        assert_eq!(windows[1].key, "credits");
        assert_eq!(windows[1].remaining_text.as_deref(), Some("$25.00"));
    }

    #[test]
    fn legacy_limits_use_remaining_percent() {
        let balance: BalanceResponse = serde_json::from_str(r#"{
          "included":{
            "session":{"remaining_percent":75,"resets_at":"2026-10-01T07:00:00Z"},
            "weekly":{"remaining_percent":40,"resets_at":"2026-10-05T00:00:00Z"}
          },
          "purchased":{"balance_usd":0}
        }"#).unwrap();
        let windows = windows_from_balance(&balance);
        assert_eq!(windows[0].key, "session");
        assert_eq!(windows[0].used_percent, Some(25.0));
        assert_eq!(windows[1].key, "weekly");
        assert_eq!(windows[1].used_percent, Some(60.0));
        assert_eq!(windows[2].remaining_text.as_deref(), Some("$0.00"));
    }

    #[test]
    fn balance_endpoint_accepts_local_and_cloud_v1() {
        assert_eq!(balance_endpoint("http://127.0.0.1:11434/v1"), "http://127.0.0.1:11434/api/balance");
        assert_eq!(balance_endpoint("https://ollama.com/v1/"), "https://ollama.com/api/balance");
    }
}
