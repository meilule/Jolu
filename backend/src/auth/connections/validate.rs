/// 上游错误体里给人看的短消息：`{"message":…}` 或 OpenAI 形态的 `{"error":{"message":…}}`。
/// 超过 120 字、空串、或不是对象都不取——宁可用通用文案，也不把整页 HTML 塞进提示。
pub fn upstream_message(body: &str) -> Option<String> {
    let body: serde_json::Value = serde_json::from_str(body).ok()?;
    let message = body
        .get("message")
        .and_then(|value| value.as_str())
        .or_else(|| body.get("error").and_then(|error| error.get("message")).and_then(|value| value.as_str()))?;
    let message = message.trim();
    (!message.is_empty() && message.chars().count() <= 120).then(|| message.to_string())
}

/// 「获取模型」（画布 18）：从网关拉取可用模型列表。
/// Claude 与 Anthropic 兼容的自定义供应商走 x-api-key + anthropic-version，OpenAI 兼容网关走 Bearer；
/// 两家的 /v1/models 响应同为 {"data": [{"id": ...}]}，取 id 即可。
pub fn fetch_models(platform: &str, protocol: Option<&str>, base_url: &str, secret: &str) -> Result<Vec<String>, String> {
    let protocol = auth_protocol(platform, protocol);
    let url = models_endpoint(base_url, protocol, None)?;
    let resp = apply_auth(keyed_client(10)?.get(&url), protocol, secret)
        .send()
        .map_err(|e| format!("无法连接网关：{}", e.without_url()))?;
    let status = resp.status();
    if status.is_redirection() {
        return Err(redirect_error(status));
    }
    if !status.is_success() {
        // 上游的错误体（`{"code":"GROUP_DISABLED","message":"…"}`）比状态码有用得多：
        // 403 可能是 Key 无效、也可能是这把 Key 被上游停用了，用户据此才能判断该找谁。
        // 只在能解析出短文本时附加，且不含任何凭证。
        let detail = upstream_message(&resp.text().unwrap_or_default());
        return Err(match detail {
            Some(message) => format!("获取模型失败：HTTP {status}（{message}），请检查 Key 与地址"),
            None => format!("获取模型失败：HTTP {status}，请检查 Key 与地址"),
        });
    }
    let body: serde_json::Value = resp.json().map_err(|e| format!("响应解析失败：{}", e.without_url()))?;
    let ids = body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(str::to_string))
                .fold(Vec::new(), |mut ids, id| {
                    // 模型 ID 去重（Jolu.pen 画板 29），保留服务端返回的顺序
                    if !ids.contains(&id) { ids.push(id); }
                    ids
                })
        });
    match ids {
        Some(ids) if !ids.is_empty() => Ok(ids),
        _ => Err("网关返回了空模型列表".into()),
    }
}

/// 「获取强度」已移除：思考强度没有网关端点，由用户按模型直接选择
/// off/low/medium/high（与请求日志 effort 列同口径），无需网络请求。

/// {base} → {base}/v1/models[/{model}]。用户可能填 https://gw.test
/// 或已带 /v1 的地址，两种都归一；仅接受 HTTP(S)。
/// {base} → {base}/v1/models[/{model}]。用户可能填 https://gw.test
/// 或已带 /v1 的地址，两种都归一；仅接受 HTTP(S)。
/// 「获取模型」与「获取强度」（effort_map）共用。
pub fn models_endpoint(base_url: &str, protocol: &str, model: Option<&str>) -> Result<String, String> {
    let tail = match model {
        Some(m) => format!("/models/{m}"),
        None => "/models".to_string(),
    };
    api_endpoint(base_url, protocol, &tail)
}

/// {base} → {base}/v1{tail}；已带 /v1（或 Gemini 的 /openai）时直接拼接。仅接受 HTTP(S)。
/// OpenAI 兼容地址以 /v<数字> 结尾（智谱 /paas/v4、火山 /api/v3 等）视为已带版本，不再补 /v1；
/// Anthropic 兼容保持只认 /v1 的规则。
pub fn api_endpoint(base_url: &str, protocol: &str, tail: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(trimmed).map_err(|_| "网关地址格式无效".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("网关地址必须使用 HTTP 或 HTTPS".into());
    }
    let versioned = protocol != "anthropic" && trimmed.rsplit('/').next().is_some_and(|last| {
        last.len() > 1 && last.starts_with('v') && last[1..].bytes().all(|b| b.is_ascii_digit())
    });
    // Gemini 的 OpenAI 兼容地址以 /openai 结尾，模型列表直接挂在其下（/v1beta/openai/models）
    if versioned || trimmed.ends_with("/v1") || trimmed.ends_with("/openai") {
        Ok(format!("{trimmed}{tail}"))
    } else {
        Ok(format!("{trimmed}/v1{tail}"))
    }
}

/// 「验证模型」（画板 27「接口不支持模型列表」）：模型接口不可用、用户手填模型 ID 时，
/// 由用户确认后发送**一次**最小请求（max_tokens = 1）确认地址、密钥与模型可用。可能产生少量 API 用量，
/// 只在用户点击并确认后调用；Key 只用于本次请求，错误信息不回显 Key。
pub fn verify_model(protocol: &str, base_url: &str, secret: &str, model: &str) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("请填写模型 ID".into());
    }
    let anthropic = protocol == "anthropic";
    let protocol = if anthropic { "anthropic" } else { "openai" };
    let url = api_endpoint(base_url, protocol, if anthropic { "/messages" } else { "/chat/completions" })?;
    let body = serde_json::json!({ "model": model, "max_tokens": 1, "messages": [{ "role": "user", "content": "ping" }] });
    let resp = apply_auth(keyed_client(30)?.post(&url), protocol, secret)
        .json(&body)
        .send()
        .map_err(|e| format!("无法连接服务：{}", e.without_url()))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    if status.is_redirection() {
        return Err(redirect_error(status));
    }
    // 服务返回的错误说明（截断）有助于判断是模型 ID 还是参数问题；服务若回显了 Key，先打码
    let detail = resp.json::<serde_json::Value>().ok()
        .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(str::to_string))
        .map(|m| if secret.is_empty() { m } else { m.replace(secret, "****") })
        .map(|m| format!("：{}", m.chars().take(160).collect::<String>()))
        .unwrap_or_default();
    Err(match status.as_u16() {
        401 | 403 => format!("验证失败：HTTP {status}，API Key 无效或没有该模型的权限{detail}"),
        404 => format!("验证失败：HTTP {status}，服务未找到该模型或接口，请检查模型 ID 与 Base URL{detail}"),
        429 => format!("验证失败：HTTP {status}，服务限流或额度不足，请稍后再试{detail}"),
        _ => format!("验证失败：HTTP {status}{detail}"),
    })
}

/// 带 Key 请求用的客户端：**不跟随重定向**。reqwest 跨主机跳转只删 Authorization 等头、不删 x-api-key，
/// https 跳到同主机 http 时还会明文发出 Key（先例：provider_key.rs、quota.rs）。
pub fn keyed_client(timeout_secs: u64) -> Result<reqwest::blocking::Client, String> {
    // 代理按应用联网方式（任务 2.3）
    crate::net::blocking_builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.without_url().to_string())
}

/// 3xx 一律不跟随，也不回显 Location（目标地址可能带凭证）
pub fn redirect_error(status: reqwest::StatusCode) -> String {
    format!("服务返回重定向（HTTP {status}），请填写最终地址")
}

/// 自定义供应商更换 API Key 时必须同时带上模型：后端校验只看「有模型」（D-11），
/// 新 Key 的可达性由前端用新 Key 获取模型或「验证模型」保证（画板 27）。
pub fn check_custom_key_change(platform: &str, model: Option<&str>) -> Result<(), String> {
    if platform != "custom" || model.is_some_and(|m| !m.trim().is_empty()) {
        return Ok(());
    }
    Err("自定义供应商更换 API Key 后，请在「编辑」中用新密钥重新获取模型或验证手填模型，再保存".into())
}

/// 鉴权协议：Claude 固定为 anthropic；其他平台按保存的协议，未记录时按 OpenAI 兼容。
pub fn auth_protocol(platform: &str, protocol: Option<&str>) -> &'static str {
    if platform == "claude" || protocol == Some("anthropic") { "anthropic" } else { "openai" }
}

/// 鉴权头：anthropic 走 x-api-key + anthropic-version，其余（OpenAI 兼容）走 Bearer。
pub fn apply_auth(req: reqwest::blocking::RequestBuilder, protocol: &str, secret: &str) -> reqwest::blocking::RequestBuilder {
    if protocol == "anthropic" {
        req.header("x-api-key", secret).header("anthropic-version", "2023-06-01")
    } else {
        req.header("Authorization", format!("Bearer {secret}"))
    }
}
