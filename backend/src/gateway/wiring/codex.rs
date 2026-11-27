use super::files::prune_empty_table;
use super::is_gateway_value;

pub(crate) const CODEX_PROVIDER: &str = "jolu-gateway";

/// Codex 有两种接法：
/// - 已登录 ChatGPT（`auth.json` 有 tokens）：保留内置 openai provider，只写顶层
///   `openai_base_url = <base>/backend-api/codex`，OAuth 头原样发到网关。
/// - 未登录 ChatGPT：写自定义 provider + `experimental_bearer_token`。
///
/// `wire_api` 必须是 `responses`：当前 Codex 只接受这个值，写 `chat` 会让它拒绝加载整份配置。
pub(crate) fn codex_wire(
    text: &str,
    base: &str,
    provider: &str,
    oauth: bool,
    token: Option<&str>,
) -> Result<String, String> {
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("config.toml 格式无效：{error}"))?;
    if oauth {
        // 内置 provider 优先：删掉 Jolu 自己的 provider 段与 model_provider 覆盖。
        if let Some(providers) = document.get_mut("model_providers").and_then(|p| p.as_table_mut()) {
            providers.remove(provider);
        }
        document.remove("model_provider");
        document["openai_base_url"] = toml_edit::value(format!("{base}/backend-api/codex"));
        return Ok(document.to_string());
    }
    if !document.contains_key("model_providers") {
        document["model_providers"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or("config.toml 的 model_providers 不是表")?;
    let mut entry = toml_edit::Table::new();
    entry["name"] = toml_edit::value(provider);
    entry["base_url"] = toml_edit::value(base);
    entry["wire_api"] = toml_edit::value("responses");
    if let Some(token) = token {
        entry["experimental_bearer_token"] = toml_edit::value(token);
    }
    providers[provider] = toml_edit::Item::Table(entry);
    document["model_provider"] = toml_edit::value(provider);
    Ok(document.to_string())
}

/// 还原 Codex：删掉 Jolu 建的 provider 段，恢复 `openai_base_url` 与原 `model_provider`。
/// 外部改过的值不动（当前 provider 是别人的、current base 不是网关地址时跳过对应字段）。
pub(crate) fn codex_restore(text: &str, saved: Option<&serde_json::Value>, provider: &str) -> Result<String, String> {
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("config.toml 格式无效：{error}"))?;
    let current_provider = codex_current_provider(text);
    let current_base = document.get("openai_base_url").and_then(|v| v.as_str()).map(str::to_string);
    if let Some(providers) = document.get_mut("model_providers").and_then(|p| p.as_table_mut()) {
        providers.remove(provider);
    }
    // 空表头要一起清掉：`codex_wire` 在文件原本没有 `model_providers` 时是新建的，
    // 只删子项会留下一个空的 `[model_providers]`，用户断开后配置里凭空多一截。
    prune_empty_table(&mut document, "model_providers");
    // openai_base_url：只有当前仍指向本机网关（或为空）时才还原
    if current_base.as_deref().is_none_or(is_gateway_value) {
        match saved.and_then(|s| s.get("openai_base_url")).and_then(|v| v.as_str()) {
            Some(original) => {
                document["openai_base_url"] = toml_edit::value(original);
            }
            None => {
                document.remove("openai_base_url");
            }
        }
    }
    // model_provider：只有当前是我们删掉后的空值、或仍指向我们时才还原
    if current_provider.as_deref().is_none() || current_provider.as_deref() == Some(provider) {
        match saved.and_then(|s| s.get("model_provider")).and_then(|v| v.as_str()) {
            Some(original) => {
                document["model_provider"] = toml_edit::value(original);
            }
            None => {
                document.remove("model_provider");
            }
        }
    }
    Ok(document.to_string())
}

/// Codex 实际生效的 base：顶层 openai_base_url 优先，其次当前 provider 的 base_url。
pub(crate) fn codex_effective_base(text: &str) -> Option<String> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    if let Some(base) = document.get("openai_base_url").and_then(|v| v.as_str()) {
        let base = base.trim();
        if !base.is_empty() {
            return Some(base.to_string());
        }
    }
    let provider = document.get("model_provider")?.as_str()?;
    codex_provider_base(text, provider)
}

/// 是否已登录 ChatGPT（`auth.json` 里有 OAuth tokens，而不是只有 API Key）。
pub(crate) fn codex_oauth_logged_in() -> bool {
    let Some(dir) = crate::creds::codex_dir() else { return false };
    let Ok(text) = crate::steady::read_to_string(dir.join("auth.json")) else { return false };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
    value
        .pointer("/tokens/access_token")
        .and_then(|v| v.as_str())
        .is_some_and(|token| !token.trim().is_empty())
}

pub(crate) fn codex_current_provider(text: &str) -> Option<String> {
    text.parse::<toml_edit::DocumentMut>()
        .ok()?
        .get("model_provider")?
        .as_str()
        .map(str::to_string)
}

pub(crate) fn codex_provider_base(text: &str, provider: &str) -> Option<String> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    document
        .get("model_providers")?
        .get(provider)?
        .get("base_url")?
        .as_str()
        .map(str::to_string)
}

/// 读 Codex 的 `openai_base_url` 与当前 provider 作为原值快照；网关值一律不收。
pub(crate) fn codex_snapshot(text: &str) -> Option<serde_json::Value> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    let mut saved = serde_json::Map::new();
    if let Some(base) = document.get("openai_base_url").and_then(|v| v.as_str()) {
        if !is_gateway_value(base) {
            saved.insert("openai_base_url".into(), serde_json::Value::String(base.to_string()));
        }
    }
    if let Some(current) = document.get("model_provider").and_then(|v| v.as_str()) {
        if current != CODEX_PROVIDER
            && !codex_provider_base(text, current).is_some_and(|base| is_gateway_value(&base))
        {
            saved.insert("model_provider".into(), serde_json::Value::String(current.to_string()));
        }
    }
    if saved.is_empty() { None } else { Some(serde_json::Value::Object(saved)) }
}
