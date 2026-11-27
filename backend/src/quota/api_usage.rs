use super::*;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedApiUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
}

/// API Key 的官方组织用量查询结果。普通调用 Key 通常会得到 403；界面据此回退到
/// 本机同平台会话统计，而不是伪造订阅额度或余额。
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[derive(Clone)]
pub enum ApiUsageState {
    Ok {
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        total_tokens: u64,
        cost_usd: Option<f64>,
        cost_reason: Option<String>,
        source: String,
    },
    Unsupported { reason: String },
    Forbidden { reason: String },
    Unauthorized { reason: String },
    RateLimited { reason: String },
    Failed { reason: String },
}

pub fn sum_u64(value: &serde_json::Value, key: &str) -> u64 {
    value.get(key).and_then(serde_json::Value::as_u64).unwrap_or(0)
}

pub fn result_rows(root: &serde_json::Value) -> Result<Vec<&serde_json::Value>, String> {
    if root.get("has_more").and_then(serde_json::Value::as_bool) == Some(true)
        || root.get("next_page").and_then(serde_json::Value::as_str).is_some_and(|s| !s.is_empty()) {
        return Err("响应仍有分页，不能将部分结果显示为今日总量".into());
    }
    let buckets = root.get("data").and_then(serde_json::Value::as_array).ok_or("响应缺少 data 数组")?;
    let mut rows = Vec::new();
    for bucket in buckets {
        rows.extend(bucket.get("results").and_then(serde_json::Value::as_array).ok_or("响应缺少 results 数组")?);
    }
    Ok(rows)
}

pub fn required_tokens(row: &serde_json::Value, key: &str) -> Result<u64, String> {
    row.get(key).and_then(serde_json::Value::as_u64).ok_or_else(|| format!("缺少或无效的 {key}"))
}

pub fn cost_amount(value: Option<&serde_json::Value>) -> Result<f64, String> {
    value.and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .filter(|v| v.is_finite() && *v >= 0.0).ok_or_else(|| "缺少或无效的费用金额".into())
}

pub fn parse_anthropic_admin_usage(body: &str) -> Result<ParsedApiUsage, String> {
    let root: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let mut parsed = ParsedApiUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        total_tokens: 0,
    };
    for row in result_rows(&root)? {
        let uncached = required_tokens(row, "uncached_input_tokens")?;
        let cache_read = sum_u64(row, "cache_read_input_tokens");
        let cache_write = row
            .get("cache_creation")
            .map(|v| {
                sum_u64(v, "ephemeral_1h_input_tokens")
                    + sum_u64(v, "ephemeral_5m_input_tokens")
            })
            .unwrap_or(0);
        let output = required_tokens(row, "output_tokens")?;
        parsed.input_tokens += uncached + cache_read + cache_write;
        parsed.output_tokens += output;
        parsed.cache_read_tokens += cache_read;
        parsed.cache_write_tokens += cache_write;
    }
    parsed.total_tokens = parsed.input_tokens + parsed.output_tokens;
    Ok(parsed)
}

pub fn parse_openai_admin_usage(body: &str) -> Result<ParsedApiUsage, String> {
    let root: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let mut parsed = ParsedApiUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        total_tokens: 0,
    };
    for row in result_rows(&root)? {
        // OpenAI 的 input_tokens 已包含 cached input，不能再把 input_cached_tokens 加进总量。
        parsed.input_tokens += required_tokens(row, "input_tokens")?;
        parsed.output_tokens += required_tokens(row, "output_tokens")?;
        parsed.cache_read_tokens += sum_u64(row, "input_cached_tokens");
    }
    parsed.total_tokens = parsed.input_tokens + parsed.output_tokens;
    Ok(parsed)
}

pub fn parse_anthropic_admin_cost(body: &str) -> Result<f64, String> {
    let root: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let mut cents = 0.0;
    for row in result_rows(&root)? {
        if row.get("currency").and_then(serde_json::Value::as_str).is_some_and(|v| !v.eq_ignore_ascii_case("usd")) { return Err("费用币种不是 USD".into()); }
        cents += cost_amount(row.get("amount"))?;
    }
    Ok(cents / 100.0)
}

pub fn parse_openai_admin_cost(body: &str) -> Result<f64, String> {
    let root: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let mut total = 0.0;
    for row in result_rows(&root)? {
        let amount = row.get("amount").ok_or("缺少费用金额")?;
        if amount.get("currency").and_then(serde_json::Value::as_str).is_some_and(|v| !v.eq_ignore_ascii_case("usd")) { return Err("费用币种不是 USD".into()); }
        total += cost_amount(amount.get("value"))?;
    }
    Ok(total)
}


/* ---------------- 官方 API Key：组织用量与费用（Admin 权限） ---------------- */

pub fn api_usage_error(outcome: HttpOutcome, provider: &str) -> Result<String, ApiUsageState> {
    match outcome {
        HttpOutcome::Body(body) => Ok(body),
        HttpOutcome::Unauthorized(reason) => Err(ApiUsageState::Unauthorized { reason }),
        HttpOutcome::Forbidden(_) => Err(ApiUsageState::Forbidden {
            reason: format!("{provider} 拒绝组织用量查询（需要 Admin 权限）；当前显示本机会话统计"),
        }),
        HttpOutcome::RateLimited(reason) => Err(ApiUsageState::RateLimited { reason }),
        HttpOutcome::Failed(reason) => Err(ApiUsageState::Failed { reason }),
    }
}

pub fn send_api_usage(
    request: reqwest::blocking::RequestBuilder,
    provider: &str,
) -> Result<String, ApiUsageState> {
    let response = request.send().map_err(|e| ApiUsageState::Failed {
        reason: if e.is_timeout() {
            format!("{provider} 用量查询超时（15 秒）")
        } else {
            format!("{provider} 用量查询网络错误: {e}")
        },
    })?;
    api_usage_error(read(response), provider)
}

pub fn today_bounds() -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    let now = chrono::Local::now();
    let start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|v| v.and_local_timezone(chrono::Local).single())
        .unwrap_or(now);
    (start.with_timezone(&chrono::Utc), now.with_timezone(&chrono::Utc))
}

pub fn cost_result(
    response: Result<reqwest::blocking::Response, reqwest::Error>,
    parse: fn(&str) -> Result<f64, String>,
) -> (Option<f64>, Option<String>) {
    let response = match response {
        Ok(response) => response,
        Err(e) => return (None, Some(format!("费用查询失败: {e}"))),
    };
    match read(response) {
        HttpOutcome::Body(body) => match parse(&body) {
            Ok(value) => (Some(value), None),
            Err(e) => (None, Some(format!("费用响应格式无法解析: {e}"))),
        },
        HttpOutcome::Forbidden(reason)
        | HttpOutcome::Unauthorized(reason)
        | HttpOutcome::RateLimited(reason)
        | HttpOutcome::Failed(reason) => (None, Some(reason)),
    }
}

pub fn anthropic_api_usage(api_key: &str) -> ApiUsageState {
    let client = match client() {
        Ok(client) => client,
        Err(reason) => return ApiUsageState::Failed { reason },
    };
    let (start, end) = today_bounds();
    let start = start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let end = end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let headers = |request: reqwest::blocking::RequestBuilder| {
        request
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Accept", "application/json")
    };
    let usage_body = match send_api_usage(
        headers(client.get(format!(
            "{ANTHROPIC_BASE}/v1/organizations/usage_report/messages"
        )))
        .query(&[
            ("starting_at", start.as_str()),
            ("ending_at", end.as_str()),
            ("bucket_width", "1d"),
            ("limit", "31"),
        ]),
        "Claude",
    ) {
        Ok(body) => body,
        Err(state) => return state,
    };
    let parsed = match parse_anthropic_admin_usage(&usage_body) {
        Ok(parsed) => parsed,
        Err(e) => return ApiUsageState::Failed { reason: format!("Claude 用量响应格式无法解析: {e}") },
    };
    let (cost_usd, cost_reason) = cost_result(
        headers(client.get(format!("{ANTHROPIC_BASE}/v1/organizations/cost_report")))
            .query(&[
                ("starting_at", start.as_str()),
                ("ending_at", end.as_str()),
                ("bucket_width", "1d"),
                ("limit", "31"),
            ])
            .send(),
        parse_anthropic_admin_cost,
    );
    ApiUsageState::Ok {
        input_tokens: parsed.input_tokens,
        output_tokens: parsed.output_tokens,
        cache_read_tokens: parsed.cache_read_tokens,
        cache_write_tokens: parsed.cache_write_tokens,
        total_tokens: parsed.total_tokens,
        cost_usd,
        cost_reason,
        source: "Claude 组织用量接口（Admin API Key）".into(),
    }
}

pub fn openai_api_usage(api_key: &str) -> ApiUsageState {
    let client = match client() {
        Ok(client) => client,
        Err(reason) => return ApiUsageState::Failed { reason },
    };
    let (start, end) = today_bounds();
    let start = start.timestamp().to_string();
    let end = end.timestamp().to_string();
    let auth = |request: reqwest::blocking::RequestBuilder| {
        request.bearer_auth(api_key).header("Accept", "application/json")
    };
    let usage_body = match send_api_usage(
        auth(client.get("https://api.openai.com/v1/organization/usage/completions"))
            .query(&[
                ("start_time", start.as_str()),
                ("end_time", end.as_str()),
                ("bucket_width", "1d"),
                ("limit", "31"),
            ]),
        "Codex",
    ) {
        Ok(body) => body,
        Err(state) => return state,
    };
    let parsed = match parse_openai_admin_usage(&usage_body) {
        Ok(parsed) => parsed,
        Err(e) => return ApiUsageState::Failed { reason: format!("Codex 用量响应格式无法解析: {e}") },
    };
    let (cost_usd, cost_reason) = cost_result(
        auth(client.get("https://api.openai.com/v1/organization/costs"))
            .query(&[
                ("start_time", start.as_str()),
                ("end_time", end.as_str()),
                ("bucket_width", "1d"),
                ("limit", "31"),
            ])
            .send(),
        parse_openai_admin_cost,
    );
    ApiUsageState::Ok {
        input_tokens: parsed.input_tokens,
        output_tokens: parsed.output_tokens,
        cache_read_tokens: parsed.cache_read_tokens,
        cache_write_tokens: parsed.cache_write_tokens,
        total_tokens: parsed.total_tokens,
        cost_usd,
        cost_reason,
        source: "OpenAI 组织用量接口（Admin API Key）".into(),
    }
}

/// 只有结构正确的成功响应才能确认模型列表查询有效。
pub fn validate_models_body(body: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| "模型列表响应不是有效 JSON")?;
    if value.get("data").and_then(serde_json::Value::as_array).is_none() { return Err("模型列表响应格式无效".into()); }
    Ok(())
}

/// 普通 API Key 使用不产生模型调用费用的模型列表接口验证，403 本身不是凭证有效的证据。
pub fn official_models(platform: &str, secret: &str) -> HttpOutcome {
    let client = match client() { Ok(client) => client, Err(reason) => return HttpOutcome::Failed(reason) };
    let request = if platform == "claude" {
        client.get("https://api.anthropic.com/v1/models").header("x-api-key", secret).header("anthropic-version", "2023-06-01")
    } else {
        client.get("https://api.openai.com/v1/models").bearer_auth(secret)
    };
    request.send().map(read).unwrap_or_else(|_| HttpOutcome::Failed("验证连接失败，请检查网络后重试".into()))
}

pub fn validate_official_api_key(platform: &str, secret: &str) -> Result<(), String> {
    match official_models(platform, secret) {
        HttpOutcome::Body(body) => validate_models_body(&body),
        HttpOutcome::Forbidden(reason) | HttpOutcome::Unauthorized(reason) => {
            // Admin Key 可能没有模型调用权限，但组织接口成功仍可确认其有效性。
            let usage = if platform == "claude" { anthropic_api_usage(secret) } else { openai_api_usage(secret) };
            if matches!(usage, ApiUsageState::Ok { .. }) { Ok(()) } else { Err(reason) }
        }
        HttpOutcome::RateLimited(reason) | HttpOutcome::Failed(reason) => Err(reason),
    }
}

pub fn denied_admin_usage(models: HttpOutcome) -> ApiUsageState {
    match models {
        HttpOutcome::Body(body) => match validate_models_body(&body) {
            Ok(()) => ApiUsageState::Forbidden { reason: "当前 Key 可用，但没有组织统计权限；当前显示本机会话统计".into() },
            Err(reason) => ApiUsageState::Failed { reason },
        },
        HttpOutcome::Unauthorized(reason) => ApiUsageState::Unauthorized { reason },
        HttpOutcome::Forbidden(reason) => ApiUsageState::Forbidden { reason },
        HttpOutcome::RateLimited(reason) => ApiUsageState::RateLimited { reason },
        HttpOutcome::Failed(reason) => ApiUsageState::Failed { reason },
    }
}

/// 组织接口拒绝普通 Key 时，用模型列表确认，避免把缺少 Admin 权限当成凭证过期。
pub fn official_api_usage(platform: &str, secret: &str) -> ApiUsageState {
    let result = match platform {
        "claude" => anthropic_api_usage(secret),
        "codex" => openai_api_usage(secret),
        _ => return ApiUsageState::Unsupported { reason: "该平台暂不支持组织统计".into() },
    };
    if matches!(result, ApiUsageState::Unauthorized { .. }) {
        denied_admin_usage(official_models(platform, secret))
    } else { result }
}
