use std::path::PathBuf;
use super::files::{dotenv_get, wire_dotenv};
use super::is_gateway_value;

/// OpenCode：`opencode.json` 的 `provider` 段。JSON 往返保留全部字段。
pub(crate) fn opencode_wire(text: &str, base: &str, token: &str) -> Result<String, String> {
    let mut root: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(text).map_err(|error| format!("opencode 配置不是有效 JSON：{error}"))?
    };
    let object = root.as_object_mut().ok_or("opencode 配置顶层不是对象")?;
    let providers = object.entry("provider").or_insert_with(|| serde_json::json!({}));
    let providers = providers.as_object_mut().ok_or("provider 段不是对象")?;
    providers.insert(
        "jolu".into(),
        serde_json::json!({
            "npm": "@ai-sdk/openai-compatible",
            "name": "Jolu 网关",
            "options": {"baseURL": base, "apiKey": token},
        }),
    );
    serde_json::to_string_pretty(&root).map_err(|error| format!("opencode 配置序列化失败：{error}"))
}

pub(crate) fn opencode_restore(text: &str, _saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut root: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("opencode 配置不是有效 JSON：{error}"))?;
    if let Some(providers) = root.get_mut("provider").and_then(|p| p.as_object_mut()) {
        providers.remove("jolu");
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("opencode 配置序列化失败：{error}"))
}

pub(crate) fn opencode_current_base(text: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.get("provider")?.get("jolu")?.get("options")?.get("baseURL")?.as_str().map(str::to_string)
}

pub(crate) fn opencode_current_token(text: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.get("provider")?.get("jolu")?.get("options")?.get("apiKey")?.as_str().map(str::to_string)
}

/// Gemini CLI：交互式按 `settings.json` 的 `security.auth.selectedType` 选鉴权。
/// 只写 base URL 对 Google 登录用户无效，所以这里做事务式接管：
/// `.env` 写地址 + 网关 Key，`settings.json` 临时把 selectedType 切成 `gemini-api-key`；
/// 断开时按快照逐项还原。OAuth 凭据文件本身不碰。
#[derive(Debug, Clone)]
pub(crate) struct GeminiPlan {
    pub(crate) env: String,
    pub(crate) settings: String,
    pub(crate) snapshot: serde_json::Value,
}

pub(crate) fn gemini_settings_path() -> Option<PathBuf> {
    Some(crate::creds::home()?.join(".gemini").join("settings.json"))
}

pub(crate) fn gemini_selected_type(settings_text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(settings_text)
        .ok()?
        .pointer("/security/auth/selectedType")?
        .as_str()
        .map(str::to_string)
}

pub(crate) fn gemini_plan(env_text: &str, settings_text: &str, base: &str, token: &str) -> Result<GeminiPlan, String> {
    let mut env_saved = serde_json::Map::new();
    for key in ["GOOGLE_GEMINI_BASE_URL", "GEMINI_API_KEY"] {
        if let Some(value) = dotenv_get(env_text, key).filter(|value| !is_gateway_value(value)) {
            env_saved.insert(key.into(), serde_json::Value::String(value));
        }
    }
    let mut settings_saved = serde_json::Map::new();
    if let Some(selected) = gemini_selected_type(settings_text) {
        settings_saved.insert("selectedType".into(), serde_json::Value::String(selected));
    }
    // 即使原来什么都没有也要记快照：断开时才知道「这两个键是我们加的，要删掉」。
    let snapshot = serde_json::json!({
        "env": serde_json::Value::Object(env_saved),
        "settings": serde_json::Value::Object(settings_saved),
    });
    let env = wire_dotenv(
        &wire_dotenv(env_text, "GOOGLE_GEMINI_BASE_URL", Some(base)),
        "GEMINI_API_KEY",
        Some(token),
    );
    let mut settings: serde_json::Value = if settings_text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(settings_text)
            .map_err(|error| format!("~/.gemini/settings.json 不是有效 JSON，未改动：{error}"))?
    };
    let object = settings.as_object_mut().ok_or("~/.gemini/settings.json 顶层不是对象，未改动")?;
    let security = object.entry("security").or_insert_with(|| serde_json::json!({}));
    let security = security
        .as_object_mut()
        .ok_or("~/.gemini/settings.json 的 security 段不是对象，未改动")?;
    let auth = security.entry("auth").or_insert_with(|| serde_json::json!({}));
    let auth = auth
        .as_object_mut()
        .ok_or("~/.gemini/settings.json 的 security.auth 段不是对象，未改动")?;
    auth.insert("selectedType".into(), serde_json::Value::String("gemini-api-key".into()));
    Ok(GeminiPlan {
        env,
        settings: serde_json::to_string_pretty(&settings).unwrap_or_else(|_| settings_text.to_string()),
        snapshot,
    })
}

/// 还原 `.env` 与 settings.json；旧版快照（只有 `value`）只还原地址，不碰 settings。
pub(crate) fn gemini_restore(env_text: &str, settings_text: &str, saved: Option<&serde_json::Value>) -> (String, String) {
    let legacy_base = saved.and_then(|s| s.get("value")).and_then(|v| v.as_str());
    let saved_env = saved.and_then(|s| s.get("env"));
    let saved_settings = saved.and_then(|s| s.get("settings"));
    let env = if let Some(env) = saved_env {
        let base = env.get("GOOGLE_GEMINI_BASE_URL").and_then(|v| v.as_str());
        let key = env.get("GEMINI_API_KEY").and_then(|v| v.as_str());
        let next = wire_dotenv(env_text, "GOOGLE_GEMINI_BASE_URL", base);
        wire_dotenv(&next, "GEMINI_API_KEY", key)
    } else {
        wire_dotenv(env_text, "GOOGLE_GEMINI_BASE_URL", legacy_base)
    };
    let settings = match saved_settings {
        Some(saved_settings) => {
            let mut settings: serde_json::Value = if settings_text.trim().is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_str(settings_text) {
                    Ok(value) => value,
                    // 文件坏了就不写回，交给用户自己修
                    Err(_) => return (env, settings_text.to_string()),
                }
            };
            let selected = saved_settings.get("selectedType").and_then(|v| v.as_str());
            let auth = settings
                .pointer_mut("/security/auth")
                .and_then(|v| v.as_object_mut());
            if let Some(auth) = auth {
                match selected {
                    Some(value) => {
                        auth.insert("selectedType".into(), serde_json::Value::String(value.to_string()));
                    }
                    None => {
                        auth.remove("selectedType");
                    }
                }
            }
            serde_json::to_string_pretty(&settings).unwrap_or_else(|_| settings_text.to_string())
        }
        None => settings_text.to_string(),
    };
    (env, settings)
}

pub(crate) fn gemini_current_base(text: &str) -> Option<String> {
    dotenv_get(text, "GOOGLE_GEMINI_BASE_URL")
}

pub(crate) fn gemini_current_token(text: &str) -> Option<String> {
    dotenv_get(text, "GEMINI_API_KEY")
}

pub(crate) fn gemini_settings_text() -> Option<String> {
    crate::steady::read_to_string(gemini_settings_path()?).ok()
}
