//! 多工具配置路径汇总、实时接线漂移指纹与本机直连 / 本地 OAuth 凭证提取。
//!
//! 复用 `gateway::wiring::specs::SPECS` 与 `agents::custom` 的路径及格式规则：
//! 1. 为 `app::watcher` 提供全量可监听配置文件清单与接线状态指纹（外部覆盖配置后 500ms 内感知 `drift`）；
//! 2. 为 `auth::local_active` 提取各 CLI 工具在直连 API Key 或本地官方登录（ZCode / Gemini / OpenCode / Grok）下
//!    当前生效的身份标识，明文密钥仅在内存栈内参与匹配或哈希摘要，绝不进入日志、缓存或跨进程载荷。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::current_connection::LocalConnection;
use crate::custom_agents::{self, CustomAgent};
use crate::gateway::wiring;

pub(crate) const LOCAL_OAUTH_PREFIX: &str = "local-oauth:";

fn oauth_fingerprint_token(platform: &str, material: &str) -> String {
    let digest = Sha256::digest(material.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{LOCAL_OAUTH_PREFIX}{platform}:{hex}")
}

/// 汇总本机所有需要监听外部改动的 Agent 配置文件（去重后返回）。
pub(crate) fn watch_files(home: &Path, custom_agents: &[CustomAgent]) -> Vec<PathBuf> {
    let mut set = BTreeSet::new();
    let claude_dir = crate::creds::claude_dir().unwrap_or_else(|| home.join(".claude"));
    let codex_dir = crate::creds::codex_dir().unwrap_or_else(|| home.join(".codex"));

    set.insert(claude_dir.join("settings.json"));
    set.insert(claude_dir.join(".credentials.json"));
    set.insert(home.join(".claude.json"));
    set.insert(codex_dir.join("auth.json"));
    set.insert(codex_dir.join("config.toml"));

    for spec in wiring::specs::SPECS {
        if let Some(main_path) = (spec.path)(home) {
            if spec.id == "antigravity" {
                set.insert(crate::gateway::wiring_antigravity::launcher_path(&main_path));
            }
            set.insert(main_path);
        }
    }

    // 多文件判定与本地 OAuth 登录凭证文件（切换默认 provider / 重新扫码登录时即时感知）
    let gem_dir = home.join(".gemini");
    set.insert(gem_dir.join("settings.json"));
    set.insert(gem_dir.join("oauth_creds.json"));
    set.insert(gem_dir.join("google_accounts.json"));
    set.insert(crate::gateway::wiring_pi::settings_path(home));
    set.insert(home.join(".dsh").join(".env"));
    if let Some(dsh_override) = std::env::var_os("DSH_HOME").filter(|v| !v.is_empty()) {
        set.insert(PathBuf::from(dsh_override).join(".env"));
    }
    set.insert(crate::gateway::wiring_qwen::qwen_dir(home).join(".env"));
    set.insert(crate::opencode::data_dir(home).join("auth.json"));
    let zcode_v2 = wiring::zcode_dir_public(home).join("v2");
    set.insert(zcode_v2.join("credentials.json"));
    set.insert(zcode_v2.join("setting.json"));
    set.insert(home.join(".grok").join("auth.json"));

    for custom in custom_agents {
        set.insert(custom_agents::expand(&custom.config_path));
    }

    set.into_iter().collect()
}

/// 计算全部内置可接线工具与自定义工具的当前接线状态指纹（不含任何密钥内容）。
pub(crate) fn wiring_fingerprint(conn: &Connection, port: u16) -> String {
    let records: std::collections::BTreeMap<String, wiring::store::WiringRecord> = wiring::store::all(conn)
        .into_iter()
        .map(|r| (r.agent.clone(), r))
        .collect();
    let mut parts = Vec::new();
    for agent in wiring::agents() {
        let st = wiring::status_with(agent, port, records.get(*agent));
        parts.push(format!(
            "{}:{}:{}:{}:{}:{}",
            st.agent,
            st.wired,
            st.state.as_deref().unwrap_or(""),
            st.drift.as_deref().unwrap_or(""),
            st.codex_mode.as_deref().unwrap_or(""),
            st.current_model.as_deref().unwrap_or(""),
        ));
    }
    if let Ok(customs) = custom_agents::list(conn) {
        for item in customs {
            let v = custom_agents::view(&item);
            parts.push(format!(
                "custom:{}:{}:{}:{}",
                v.id,
                v.wired,
                v.drift.as_deref().unwrap_or(""),
                v.model.as_deref().unwrap_or(""),
            ));
        }
    }
    parts.join("|")
}

/// 提取除 Claude Code / Codex 外各 CLI 工具在本机磁盘上当前生效的直连、本地 OAuth 或网关凭证。
pub(crate) fn read_extended_local(home: &Path, custom_agents: &[CustomAgent]) -> Vec<LocalConnection> {
    let mut out = Vec::new();
    if let Some(item) = gemini_active(home) {
        out.push(item);
    }
    if let Some(item) = opencode_active(home) {
        out.push(item);
    }
    if let Some(item) = kimi_active(home) {
        out.push(item);
    }
    if let Some(item) = pi_active(home) {
        out.push(item);
    }
    if let Some(item) = dsh_active(home) {
        out.push(item);
    }
    if let Some(item) = zcode_active(home) {
        out.push(item);
    }
    if let Some(item) = grok_active(home) {
        out.push(item);
    }
    if let Some(item) = qwen_active(home) {
        out.push(item);
    }
    out.extend(custom_agents_active(custom_agents));
    out
}

fn clean_non_empty(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn infer_protocol(hint: Option<&str>, base_url: Option<&str>, secret: &str) -> String {
    if let Some(h) = hint.map(|s| s.to_ascii_lowercase()) {
        if h.contains("anthropic") {
            return "anthropic".into();
        }
        if h.contains("openai") {
            return "openai".into();
        }
    }
    if secret.trim().starts_with("sk-ant-") {
        return "anthropic".into();
    }
    if base_url
        .map(|b| b.to_ascii_lowercase())
        .is_some_and(|b| b.contains("anthropic"))
    {
        return "anthropic".into();
    }
    "openai".into()
}

/// Gemini CLI：优先按 `settings.json` 的 `selectedType` 识别 API Key（`.env`）或本地 Google OAuth（`oauth_creds.json`）。
pub(crate) fn gemini_active(home: &Path) -> Option<LocalConnection> {
    let gem_dir = home.join(".gemini");
    let settings_text = crate::steady::read_to_string(gem_dir.join("settings.json")).unwrap_or_default();
    let selected = wiring::gemini_selected_type(&settings_text);

    if selected.as_deref().is_none_or(|t| t == "gemini-api-key") {
        if let Ok(env_text) = crate::steady::read_to_string(gem_dir.join(".env")) {
            if let Some(secret) = wiring::gemini_current_token(&env_text).and_then(|s| clean_non_empty(Some(&s))) {
                let base_url = wiring::gemini_current_base(&env_text).and_then(|s| clean_non_empty(Some(&s)));
                let protocol = Some(infer_protocol(None, base_url.as_deref(), &secret));
                return Some(LocalConnection {
                    platform: "gemini".into(),
                    kind: "api".into(),
                    secret,
                    base_url,
                    protocol,
                    source_agent: Some("Gemini CLI".into()),
                    masked_override: None,
                    detected_email: None,
                });
            }
        }
    }

    if selected.as_deref() != Some("gemini-api-key") {
        let creds_text = crate::steady::read_to_string(gem_dir.join("oauth_creds.json")).ok()?;
        let creds_val: Value = serde_json::from_str(&creds_text).ok()?;
        let token_mat = clean_non_empty(creds_val.get("refresh_token").and_then(Value::as_str))
            .or_else(|| clean_non_empty(creds_val.get("access_token").and_then(Value::as_str)))?;
        let email = crate::gemini_quota::active_email(&gem_dir);
        let fp_material = format!("{}:{token_mat}", email.as_deref().unwrap_or(""));
        let masked = email.clone().unwrap_or_else(|| "Google 官方登录".into());
        return Some(LocalConnection {
            platform: "gemini".into(),
            kind: "auth".into(),
            secret: oauth_fingerprint_token("gemini", &fp_material),
            base_url: None,
            protocol: None,
            source_agent: Some("Gemini CLI".into()),
            masked_override: Some(masked),
            detected_email: email,
        });
    }
    None
}

/// OpenCode：结合 `opencode.json` 与 `auth.json` 识别网关接线、直连 API Key 或本地 OpenAI OAuth 订阅。
pub(crate) fn opencode_active(home: &Path) -> Option<LocalConnection> {
    let config_path = wiring::specs::spec("opencode").and_then(|s| (s.path)(home))?;
    let config_text = crate::steady::read_to_string(config_path).unwrap_or_default();
    let root: Value = serde_json::from_str(&config_text).unwrap_or_else(|_| serde_json::json!({}));
    let auth_text = crate::steady::read_to_string(crate::opencode::data_dir(home).join("auth.json")).unwrap_or_default();
    let auth_json: Value = serde_json::from_str(&auth_text).unwrap_or_else(|_| serde_json::json!({}));

    let selected_provider = root
        .get("model")
        .and_then(Value::as_str)
        .and_then(|m| m.split_once('/').map(|(prov, _)| prov.trim()))
        .filter(|p| !p.is_empty());

    let providers_obj = root.get("provider").and_then(Value::as_object);

    if selected_provider == Some("jolu")
        || (selected_provider.is_none()
            && wiring::opencode_current_base(&config_text)
                .as_deref()
                .is_some_and(wiring::is_gateway_value))
    {
        if let Some(base) = wiring::opencode_current_base(&config_text) {
            let secret = wiring::opencode_current_token(&config_text).unwrap_or_else(|| "jolu-opencode".into());
            return Some(LocalConnection {
                platform: "opencode".into(),
                kind: "api".into(),
                secret,
                base_url: Some(base),
                protocol: Some("openai".into()),
                source_agent: Some("OpenCode".into()),
                masked_override: None,
                detected_email: None,
            });
        }
    }

    let pid_opt = match selected_provider {
        Some(p) => Some(p.to_string()),
        None => {
            let custom_keys: Vec<&String> = providers_obj
                .map(|m| m.keys().filter(|k| k.as_str() != "jolu").collect())
                .unwrap_or_default();
            if custom_keys.len() == 1 {
                Some(custom_keys[0].clone())
            } else {
                None
            }
        }
    };

    if let Some(pid) = pid_opt.as_deref() {
        let p_entry = providers_obj.and_then(|m| m.get(pid));
        let base_url = p_entry
            .and_then(|e| e.pointer("/options/baseURL").or_else(|| e.pointer("/options/baseUrl")))
            .and_then(Value::as_str)
            .and_then(|s| clean_non_empty(Some(s)));
        let npm_hint = p_entry.and_then(|e| e.get("npm")).and_then(Value::as_str);

        let inline_key = p_entry
            .and_then(|e| e.pointer("/options/apiKey"))
            .and_then(Value::as_str)
            .and_then(|s| clean_non_empty(Some(s)));
        let auth_key = auth_json
            .get(pid)
            .filter(|v| v.get("type").and_then(Value::as_str) == Some("api"))
            .and_then(|v| v.get("key"))
            .and_then(Value::as_str)
            .and_then(|s| clean_non_empty(Some(s)));

        if let Some(secret) = inline_key.or(auth_key) {
            let protocol = Some(infer_protocol(npm_hint, base_url.as_deref(), &secret));
            return Some(LocalConnection {
                platform: "opencode".into(),
                kind: "api".into(),
                secret,
                base_url,
                protocol,
                source_agent: Some("OpenCode".into()),
                masked_override: None,
                detected_email: None,
            });
        }
    }

    // OpenCode OpenAI OAuth 订阅登录（当选中 openai 或未指定其他 API provider 时生效）
    if pid_opt.as_deref().is_none_or(|p| p == "openai") {
        if let Some(entry) = auth_json
            .get("openai")
            .filter(|v| v.get("type").and_then(Value::as_str) == Some("oauth"))
        {
            if let Some(access) = clean_non_empty(entry.get("access").and_then(Value::as_str)) {
                let account_id = clean_non_empty(entry.get("accountId").and_then(Value::as_str));
                let fp_mat = format!("{}:{access}", account_id.as_deref().unwrap_or(""));
                return Some(LocalConnection {
                    platform: "opencode".into(),
                    kind: "auth".into(),
                    secret: oauth_fingerprint_token("opencode", &fp_mat),
                    base_url: None,
                    protocol: None,
                    source_agent: Some("OpenCode".into()),
                    masked_override: Some("OpenAI 订阅登录".into()),
                    detected_email: None,
                });
            }
        }
    }
    None
}

/// Kimi Code：从 `config.toml` 的 `default_model` → `models.<m>.provider` → `providers.<p>` 解析。
pub(crate) fn kimi_active(home: &Path) -> Option<LocalConnection> {
    let path = wiring::kimi_dir_public(home).0.join("config.toml");
    let text = crate::steady::read_to_string(path).ok()?;
    let doc = text.parse::<toml_edit::DocumentMut>().ok()?;
    let providers = doc.get("providers")?.as_table()?;

    let default_model = doc.get("default_model").and_then(|v| v.as_str()).map(str::trim);
    let provider_name = if let Some(dm) = default_model.filter(|s| !s.is_empty()) {
        doc.get("models")
            .and_then(|m| m.as_table())
            .and_then(|m| m.get(dm))
            .and_then(|entry| entry.get("provider"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .map(str::to_string)
            .or_else(|| dm.split_once('/').map(|(prefix, _)| prefix.to_string()).filter(|p| providers.contains_key(p)))
    } else if providers.len() == 1 {
        providers.iter().next().map(|(k, _)| k.to_string())
    } else {
        None
    }?;

    let table = providers.get(&provider_name)?.as_table()?;
    let secret = clean_non_empty(table.get("api_key").and_then(|v| v.as_str()))?;
    let base_url = clean_non_empty(table.get("base_url").and_then(|v| v.as_str()));
    let kind_hint = table.get("type").and_then(|v| v.as_str());
    let protocol = Some(infer_protocol(kind_hint, base_url.as_deref(), &secret));
    Some(LocalConnection {
        platform: "kimi".into(),
        kind: "api".into(),
        secret,
        base_url,
        protocol,
        source_agent: Some("Kimi Code".into()),
        masked_override: None,
        detected_email: None,
    })
}

/// Pi：结合 `settings.json` 的 `defaultProvider` 与 `models.json` 的 `providers.<p>` 解析。
pub(crate) fn pi_active(home: &Path) -> Option<LocalConnection> {
    let models_text = crate::steady::read_to_string(crate::gateway::wiring_pi::config_path(home)).ok()?;
    let models_root: Value = serde_json::from_str(models_text.trim_start_matches('\u{feff}')).ok()?;
    let providers = models_root.get("providers")?.as_object()?;

    let settings_text = crate::steady::read_to_string(crate::gateway::wiring_pi::settings_path(home)).unwrap_or_default();
    let default_prov = serde_json::from_str::<Value>(settings_text.trim_start_matches('\u{feff}'))
        .ok()
        .and_then(|v| v.get("defaultProvider").and_then(Value::as_str).map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty());

    let prov_key = match default_prov {
        Some(k) => Some(k),
        None if providers.len() == 1 => providers.keys().next().cloned(),
        None => None,
    }?;

    let slot = providers.get(&prov_key)?;
    let secret = clean_non_empty(slot.get("apiKey").and_then(Value::as_str))?;
    let base_url = clean_non_empty(slot.get("baseUrl").and_then(Value::as_str));
    let api_hint = slot.get("api").and_then(Value::as_str);
    let protocol = Some(infer_protocol(api_hint, base_url.as_deref(), &secret));
    Some(LocalConnection {
        platform: "pi".into(),
        kind: "api".into(),
        secret,
        base_url,
        protocol,
        source_agent: Some("Pi".into()),
        masked_override: None,
        detected_email: None,
    })
}

/// DeepSeek Harness（DSH）：从主 profile 的 `cordis.patch.yml` 与 `.env` 提取当前默认路由。
pub(crate) fn dsh_active(home: &Path) -> Option<LocalConnection> {
    let dsh_dir = std::env::var_os("DSH_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".dsh"));
    let patch_path = crate::gateway::wiring_dsh::dsh_main_file(&dsh_dir);
    let patch_text = crate::steady::read_to_string(patch_path).ok()?;
    let (_, items) = crate::gateway::yaml_lines::parse(&patch_text);

    let default_idx = crate::gateway::yaml_lines::find(&items, "agent-default-model")?;
    let active_provider = items[default_idx].lines.iter().find_map(|line| {
        let trimmed = line.trim();
        trimmed.strip_prefix("provider:").map(|v| v.trim().trim_matches(['"', '\'']).to_string())
    })?;
    if active_provider.is_empty() {
        return None;
    }

    let pi_idx = crate::gateway::yaml_lines::find(&items, "llm-pi-ai")?;
    let pi_lines = &items[pi_idx].lines;
    let (start, end) = crate::gateway::yaml_lines::block_range(pi_lines, 6, &active_provider)?;
    let block = &pi_lines[start..end];

    let field_of = |name: &str| -> Option<String> {
        let prefix = format!("{name}:");
        block.iter().find_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix(&prefix)
                .map(|v| v.trim().trim_matches(['"', '\'']).to_string())
                .filter(|v| !v.is_empty())
        })
    };

    let base_url = field_of("baseURL").or_else(|| field_of("baseUrl"));
    let api_hint = field_of("api");
    let secret = if let Some(inline) = field_of("apiKey") {
        Some(inline)
    } else if let Some(env_key) = field_of("apiKeyEnv") {
        let env_text = crate::steady::read_to_string(dsh_dir.join(".env")).unwrap_or_default();
        wiring::dotenv_get(&env_text, &env_key).and_then(|s| clean_non_empty(Some(&s)))
    } else {
        None
    }?;
    let protocol = Some(infer_protocol(api_hint.as_deref(), base_url.as_deref(), &secret));
    Some(LocalConnection {
        platform: "dsh".into(),
        kind: "api".into(),
        secret,
        base_url,
        protocol,
        source_agent: Some("DSH".into()),
        masked_override: None,
        detected_email: None,
    })
}

/// ZCode：优先识别 `v2/provider_config.json` 启用的首要自定义规则，否则识别 `v2/credentials.json` 官方登录。
pub(crate) fn zcode_active(home: &Path) -> Option<LocalConnection> {
    let v2_dir = wiring::zcode_dir_public(home).join("v2");
    if let Ok(text) = crate::steady::read_to_string(v2_dir.join("provider_config.json")) {
        if let Ok(root) = serde_json::from_str::<Value>(&text) {
            if let Some(rules) = root.pointer("/config/providerConfigRules/providerRules").and_then(Value::as_array) {
                let order: Vec<&str> = root
                    .pointer("/config/providerOrder")
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();

                let chosen = if !order.is_empty() {
                    order.into_iter().find_map(|id| {
                        rules.iter().find(|r| {
                            r.get("providerId").and_then(Value::as_str) == Some(id)
                                && r.get("enabled").and_then(Value::as_bool) != Some(false)
                        })
                    })
                } else {
                    rules.iter().find(|r| r.get("enabled").and_then(Value::as_bool) != Some(false))
                };

                if let Some(chosen) = chosen {
                    let base_url = clean_non_empty(chosen.pointer("/config/api/baseUrl").and_then(Value::as_str));
                    let secret_opt = clean_non_empty(chosen.pointer("/config/access/apiKey").and_then(Value::as_str)).or_else(|| {
                        if chosen.get("providerId").and_then(Value::as_str) == Some("jolu")
                            && base_url.as_deref().is_some_and(wiring::is_gateway_value)
                        {
                            Some("jolu-zcode".into())
                        } else {
                            None
                        }
                    });
                    if let Some(secret) = secret_opt {
                        let api_type = chosen.pointer("/config/api/type").and_then(Value::as_str);
                        let protocol = Some(infer_protocol(api_type, base_url.as_deref(), &secret));
                        return Some(LocalConnection {
                            platform: "zcode".into(),
                            kind: "api".into(),
                            secret,
                            base_url,
                            protocol,
                            source_agent: Some("ZCode".into()),
                            masked_override: None,
                            detected_email: None,
                        });
                    }
                }
            }
        }
    }

    zcode_oauth_active(&v2_dir)
}

fn zcode_oauth_active(v2_dir: &Path) -> Option<LocalConnection> {
    let text = crate::steady::read_to_string(v2_dir.join("credentials.json")).ok()?;
    let map: std::collections::BTreeMap<String, Value> = serde_json::from_str(&text).ok()?;
    let active_prov = map
        .get("oauth:active_provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let mut auth_entries = Vec::new();
    for (k, v) in &map {
        if k.starts_with("oauth:") || k.starts_with("account-provider:") {
            let val_str = v.as_str().map(str::trim).unwrap_or("");
            if !val_str.is_empty() {
                auth_entries.push(format!("{k}={val_str}"));
            }
        }
    }
    if auth_entries.is_empty() {
        return None;
    }
    let family_label = match active_prov {
        Some(p) if p.contains("bigmodel") => "ZCode · BigModel",
        Some(p) if p.contains("zai") => "ZCode · Z.ai",
        _ if map.keys().any(|k| k.contains("bigmodel")) => "ZCode · BigModel",
        _ if map.keys().any(|k| k.contains("zai")) => "ZCode · Z.ai",
        _ => "ZCode 官方登录",
    };
    let material = auth_entries.join("\n");
    Some(LocalConnection {
        platform: "zcode".into(),
        kind: "auth".into(),
        secret: oauth_fingerprint_token("zcode", &material),
        base_url: None,
        protocol: None,
        source_agent: Some("ZCode".into()),
        masked_override: Some(family_label.into()),
        detected_email: None,
    })
}

/// Grok CLI：读取 `~/.grok/auth.json` 的未过期 OAuth 登录指纹。
pub(crate) fn grok_active(home: &Path) -> Option<LocalConnection> {
    let token = crate::creds::local_grok_auth_in(home, chrono::Utc::now().timestamp()).ok()?;
    Some(LocalConnection {
        platform: "grok".into(),
        kind: "auth".into(),
        secret: oauth_fingerprint_token("grok", &token),
        base_url: None,
        protocol: None,
        source_agent: Some("Grok CLI".into()),
        masked_override: Some("Grok 官方登录".into()),
        detected_email: None,
    })
}

/// Qwen Code：读取 `~/.qwen/settings.json` 中当前选中的 OpenAI 兼容模型与密钥。
pub(crate) fn qwen_active(home: &Path) -> Option<LocalConnection> {
    let dir = crate::gateway::wiring_qwen::qwen_dir(home);
    let text = crate::steady::read_to_string(dir.join("settings.json")).ok()?;
    let root: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;

    let selected_auth = root.pointer("/security/auth/selectedType").and_then(Value::as_str);
    if selected_auth.is_some_and(|t| t != "openai") {
        return None;
    }

    let list = root.pointer("/modelProviders/openai")?.as_array()?;
    let model_name = root.pointer("/model/name").and_then(Value::as_str).map(str::trim);
    let model_base = root.pointer("/model/baseUrl").and_then(Value::as_str).map(str::trim);

    let entry = if let Some(name) = model_name.filter(|s| !s.is_empty()) {
        list.iter()
            .find(|item| {
                item.get("id").and_then(Value::as_str) == Some(name)
                    && model_base.is_none_or(|mb| item.get("baseUrl").and_then(Value::as_str) == Some(mb))
            })
            .or_else(|| list.iter().find(|item| item.get("id").and_then(Value::as_str) == Some(name)))
    } else if list.len() == 1 {
        list.first()
    } else {
        None
    }?;

    let base_url = clean_non_empty(entry.get("baseUrl").and_then(Value::as_str))
        .or_else(|| clean_non_empty(model_base));
    let secret = if let Some(inline) = clean_non_empty(entry.get("apiKey").and_then(Value::as_str)) {
        Some(inline)
    } else if let Some(env_key) = clean_non_empty(entry.get("envKey").and_then(Value::as_str)) {
        root.get("env")
            .and_then(|e| e.get(&env_key))
            .and_then(Value::as_str)
            .and_then(|s| clean_non_empty(Some(s)))
            .or_else(|| {
                let dotenv = crate::steady::read_to_string(dir.join(".env")).unwrap_or_default();
                wiring::dotenv_get(&dotenv, &env_key).and_then(|s| clean_non_empty(Some(&s)))
            })
    } else {
        None
    }?;

    let protocol = Some(infer_protocol(Some("openai"), base_url.as_deref(), &secret));
    Some(LocalConnection {
        platform: "qwen".into(),
        kind: "api".into(),
        secret,
        base_url,
        protocol,
        source_agent: Some("Qwen Code".into()),
        masked_override: None,
        detected_email: None,
    })
}

/// 自定义接入工具（`custom_agents` 表）：按用户声明的格式与键名读取当前配置。
pub(crate) fn custom_agents_active(custom_agents: &[CustomAgent]) -> Vec<LocalConnection> {
    let mut out = Vec::new();
    for agent in custom_agents {
        let path = custom_agents::expand(&agent.config_path);
        let Ok(text) = crate::steady::read_to_string(&path) else { continue };
        let base_url = custom_agents::get_key(&agent.format, &text, &agent.base_url_key)
            .ok()
            .flatten()
            .and_then(|s| clean_non_empty(Some(&s)));
        let Some(secret) = custom_agents::get_key(&agent.format, &text, &agent.auth_key)
            .ok()
            .flatten()
            .and_then(|s| clean_non_empty(Some(&s)))
        else {
            continue;
        };
        let protocol = Some(infer_protocol(None, base_url.as_deref(), &secret));
        out.push(LocalConnection {
            platform: "custom".into(),
            kind: "api".into(),
            secret,
            base_url,
            protocol,
            source_agent: Some(agent.name.clone()),
            masked_override: None,
            detected_email: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("jolu-agent-cfg-{tag}-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extracts_direct_keys_and_local_oauth_identities_across_tools() {
        let home = temp_home("multi");

        // 1. Kimi Code
        let kimi_dir = home.join(".kimi-code");
        std::fs::create_dir_all(&kimi_dir).unwrap();
        std::fs::write(
            kimi_dir.join("config.toml"),
            r#"default_model = "sf/deepseek-v3"
[providers.sf]
type = "openai"
base_url = "https://api.siliconflow.cn/v1"
api_key = "sk-sf-kimi-1111"
[models."sf/deepseek-v3"]
provider = "sf"
model = "deepseek-ai/DeepSeek-V3"
"#,
        )
        .unwrap();
        let kimi = kimi_active(&home).expect("kimi active");
        assert_eq!(kimi.platform, "kimi");
        assert_eq!(kimi.secret, "sk-sf-kimi-1111");

        // 2. Pi
        let pi_dir = home.join(".pi").join("agent");
        std::fs::create_dir_all(&pi_dir).unwrap();
        std::fs::write(pi_dir.join("settings.json"), r#"{"defaultProvider":"openrouter"}"#).unwrap();
        std::fs::write(
            pi_dir.join("models.json"),
            r#"{"providers":{"openrouter":{"baseUrl":"https://openrouter.ai/api/v1","api":"openai-completions","apiKey":"sk-or-pi-2222"}}}"#,
        )
        .unwrap();
        let pi = pi_active(&home).expect("pi active");
        assert_eq!(pi.platform, "pi");
        assert_eq!(pi.secret, "sk-or-pi-2222");

        // 3. Gemini (.env API Key → 切换到 oauth-personal 后自动跟踪 Google OAuth 账号变更)
        let gem_dir = home.join(".gemini");
        std::fs::create_dir_all(&gem_dir).unwrap();
        std::fs::write(gem_dir.join("settings.json"), r#"{"security":{"auth":{"selectedType":"gemini-api-key"}}}"#).unwrap();
        std::fs::write(gem_dir.join(".env"), "GEMINI_API_KEY=AIzaSyTestKey4444\n").unwrap();
        let gem_api = gemini_active(&home).expect("gemini api");
        assert_eq!(gem_api.kind, "api");

        std::fs::write(gem_dir.join("settings.json"), r#"{"security":{"auth":{"selectedType":"oauth-personal"}}}"#).unwrap();
        std::fs::write(gem_dir.join("oauth_creds.json"), r#"{"refresh_token":"1//refresh-account-a"}"#).unwrap();
        std::fs::write(gem_dir.join("google_accounts.json"), r#"{"active":"alice@example.com"}"#).unwrap();
        let gem_oauth_a = gemini_active(&home).expect("gemini oauth a");
        assert_eq!(gem_oauth_a.kind, "auth");
        assert_eq!(gem_oauth_a.detected_email.as_deref(), Some("alice@example.com"));
        assert!(!gem_oauth_a.secret.contains("refresh-account-a"));

        std::fs::write(gem_dir.join("oauth_creds.json"), r#"{"refresh_token":"1//refresh-account-b"}"#).unwrap();
        std::fs::write(gem_dir.join("google_accounts.json"), r#"{"active":"bob@example.com"}"#).unwrap();
        let gem_oauth_b = gemini_active(&home).expect("gemini oauth b");
        assert_ne!(gem_oauth_a.secret, gem_oauth_b.secret, "换号后指纹必须改变以触发缓存刷新");

        // 4. ZCode 本地官方登录切号检测
        let zc_v2 = home.join(".zcode").join("v2");
        std::fs::create_dir_all(&zc_v2).unwrap();
        std::fs::write(
            zc_v2.join("credentials.json"),
            r#"{"oauth:active_provider":"bigmodel","oauth:bigmodel:token":"enc-user-1"}"#,
        )
        .unwrap();
        let zc1 = zcode_active(&home).expect("zcode oauth 1");
        assert_eq!(zc1.kind, "auth");
        assert_eq!(zc1.masked_override.as_deref(), Some("ZCode · BigModel"));
        std::fs::write(
            zc_v2.join("credentials.json"),
            r#"{"oauth:active_provider":"zai","oauth:zai:token":"enc-user-2"}"#,
        )
        .unwrap();
        let zc2 = zcode_active(&home).expect("zcode oauth 2");
        assert_ne!(zc1.secret, zc2.secret);
        assert_eq!(zc2.masked_override.as_deref(), Some("ZCode · Z.ai"));

        // 5. Grok CLI OAuth
        let grok_dir = home.join(".grok");
        std::fs::create_dir_all(&grok_dir).unwrap();
        std::fs::write(
            grok_dir.join("auth.json"),
            r#"{"https://auth.x.ai::client":{"key":"grok-live-oauth-token","expires_at":"2099-01-01T00:00:00Z"}}"#,
        )
        .unwrap();
        let grok = grok_active(&home).expect("grok oauth");
        assert_eq!(grok.platform, "grok");
        assert_eq!(grok.kind, "auth");
        assert!(!grok.secret.contains("grok-live-oauth-token"));

        // 6. Qwen Code -> first-class platform "qwen"
        let qwen_dir = home.join(".qwen");
        std::fs::create_dir_all(&qwen_dir).unwrap();
        std::fs::write(
            qwen_dir.join("settings.json"),
            r#"{"security":{"auth":{"selectedType":"openai"}},"model":{"name":"qwen3-coder"},"modelProviders":{"openai":[{"id":"qwen3-coder","baseUrl":"https://dashscope.aliyuncs.com/compatible-mode/v1","envKey":"DASHSCOPE_API_KEY"}]},"env":{"DASHSCOPE_API_KEY":"sk-qwen-5555"}}"#,
        )
        .unwrap();
        let qwen = qwen_active(&home).expect("qwen active");
        assert_eq!(qwen.platform, "qwen");
        assert_eq!(qwen.secret, "sk-qwen-5555");

        let _ = std::fs::remove_dir_all(&home);
    }
}
