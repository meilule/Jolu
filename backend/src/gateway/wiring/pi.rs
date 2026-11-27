//! Pi（badlogic/pi-mono 的 coding agent）的专用接线。
//!
//! 参照 magpie `internal/agent/agents.go` 的 `piLike` / `piModelJSON`（commit `e9dd8e8`）。
//!
//! Pi 的 agent 目录（`PI_CODING_AGENT_DIR` 覆盖，缺省 `~/.pi/agent`）里有两个文件：
//!   - `models.json`：`{"providers": {"<名字>": {name, baseUrl, api, apiKey, models: [...]}}}`
//!     —— provider 与模型清单，密钥**明文**写 `apiKey`（Pi 不读环境变量、也没有凭据库）；
//!   - `settings.json`：`defaultProvider` / `defaultModel`（新会话起点）+ 用户自己的其余设置。
//!
//! 接线 = 在 models.json 写 `providers.jolu`，在 settings.json 写
//! `defaultProvider: "jolu"` + `defaultModel: <网关模型 id>`。
//!
//! 与 magpie 的差异：
//!   - 不写 `compat.sendSessionAffinityHeaders`（magpie 用它让 Pi 带上会话头；v1 不做会话归因）；
//!   - 不写 `cost` / `promptCache`（目录暂无价格与缓存时长数据）；
//!   - 不处理 `enabledModels`（Pi 的 Ctrl+P 名单）：那是用户自己的列表，magpie 会把自己的模型
//!     追加进去；Jolu v1 不改它——代价是用户若已设置名单，新会话可能仍按名单第一个跑，
//!     断开时也不会残留。
//!   - 不写 `thinkingLevelMap`（目录暂无推理档位数据）。
//!
//! 还原：删 `providers.jolu`；`defaultProvider` / `defaultModel` 只在仍是 `jolu` / 我们写的
//! 模型时才还原成快照里的原值（用户改过就不动）。

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::ZcodeModel;

/// Pi 里的 provider 名（magpie 用 magpieID）。
const PI_PROVIDER: &str = "jolu";

/// Pi 的 agent 目录：`PI_CODING_AGENT_DIR`（`~` 展开）覆盖，缺省 `~/.pi/agent`。
/// 与探测 `agent_discovery::dir_pi` 同一套判定。
pub(crate) fn pi_dir(home: &Path) -> PathBuf {
    if let Some(dir) = std::env::var_os("PI_CODING_AGENT_DIR").filter(|v| !v.is_empty()) {
        let raw = PathBuf::from(&dir);
        let expanded = match raw.to_str().and_then(|text| text.strip_prefix('~')) {
            Some(rest) => {
                let rest = rest.trim_start_matches(['/', '\\']);
                if rest.is_empty() { home.to_path_buf() } else { home.join(rest) }
            }
            None => raw,
        };
        if expanded.is_absolute() {
            return expanded;
        }
    }
    home.join(".pi").join("agent")
}

/// 主配置文件（探测与接线共用）：`models.json`（provider 与模型清单在这里）。
pub(crate) fn config_path(home: &Path) -> PathBuf {
    pi_dir(home).join("models.json")
}

/// 设置文件：`defaultProvider` / `defaultModel` 在这里。
pub(crate) fn settings_path(home: &Path) -> PathBuf {
    pi_dir(home).join("settings.json")
}

fn parse_object(text: &str, what: &str) -> Result<Value, String> {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(text).map_err(|error| format!("{what} 不是有效 JSON：{error}"))?;
    if !value.is_object() {
        return Err(format!("{what} 顶层不是对象"));
    }
    Ok(value)
}

fn render(root: &Value) -> Result<String, String> {
    let mut text = serde_json::to_string_pretty(root).map_err(|error| format!("配置序列化失败：{error}"))?;
    text.push('\n');
    Ok(text)
}

fn providers_at(root: &mut Value) -> Result<&mut Map<String, Value>, String> {
    let providers = root
        .as_object_mut()
        .ok_or("models.json 顶层不是对象")?
        .entry("providers")
        .or_insert_with(|| json!({}));
    providers.as_object_mut().ok_or_else(|| "models.json 的 providers 不是对象，Jolu 不改写它".to_string())
}

/// 我们的 provider 段（只读）。
fn slot<'a>(root: &'a Value) -> Option<&'a Value> {
    root.get("providers")?.get(PI_PROVIDER)
}

/// 当前 `providers.jolu.baseUrl`（状态判定与「外部已改就不覆盖」共用）。
pub(crate) fn current_base(text: &str) -> Option<String> {
    let root = parse_object(text, "models.json").ok()?;
    slot(&root)?.get("baseUrl")?.as_str().map(|s| s.trim().to_string())
}

pub(crate) fn current_token(text: &str) -> Option<String> {
    let root = parse_object(text, "models.json").ok()?;
    slot(&root)?.get("apiKey")?.as_str().map(|s| s.trim().to_string())
}

/// 默认模型是否指向我们（`defaultProvider == "jolu"`）。
pub(crate) fn selected_ours(settings_text: &str) -> bool {
    parse_object(settings_text, "settings.json")
        .ok()
        .and_then(|root| root.get("defaultProvider").and_then(Value::as_str).map(|p| p == PI_PROVIDER))
        .unwrap_or(false)
}

/// 生成 `providers.jolu` 的完整对象（与 magpie `piModelJSON` 同形，缺档位/价格/缓存）。
/// `base` 是网关根地址，Pi 的 `openai-completions` 走 `<base>/v1`。
fn provider_block(base: &str, token: &str, catalog: &[ZcodeModel]) -> Value {
    let models: Vec<Value> = catalog
        .iter()
        .map(|m| {
            let mut entry = Map::new();
            entry.insert("id".into(), json!(m.id));
            entry.insert("name".into(), json!(m.name));
            // reasoning：目录无档位数据时写 false（Pi 不提供 thinking 切换，按默认跑）
            entry.insert("reasoning".into(), json!(false));
            if m.context > 0 {
                entry.insert("contextWindow".into(), json!(m.context));
            }
            if m.output > 0 {
                entry.insert("maxTokens".into(), json!(m.output));
            }
            if m.image {
                entry.insert("input".into(), json!(["text", "image"]));
            }
            Value::Object(entry)
        })
        .collect();
    json!({
        "name": "Jolu",
        "baseUrl": format!("{}/v1", base.trim_end_matches('/')),
        "api": "openai-completions",
        "apiKey": token,
        "models": models,
    })
}

/// 接线：写 models.json 的 `providers.jolu` + settings.json 的默认 provider / model。
/// 返回（models.json 新内容, settings.json 新内容, 快照）。
/// 快照只记 settings.json 里用户原来的 `defaultProvider` / `defaultModel` ——
/// `providers.jolu` 不存（指向网关的就是我们或别人替我们写的，断开时直接删）。
pub(crate) fn wire(
    models_text: &str,
    settings_text: &str,
    base: &str,
    token: &str,
    model: Option<&str>,
    catalog: &[ZcodeModel],
) -> Result<(String, String, Option<Value>), String> {
    if catalog.is_empty() {
        return Err("网关模型清单为空；先在「模型池」里启用要提供给 Pi 的模型".into());
    }
    let selected = model
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .or_else(|| catalog.first().map(|m| m.id.clone()))
        .ok_or("没有可选的模型")?;
    // models.json：只动 providers.jolu，用户其它 provider 一行不动
    let mut models_root = parse_object(models_text, "models.json")?;
    providers_at(&mut models_root)?.insert(PI_PROVIDER.into(), provider_block(base, token, catalog));
    let models_next = render(&models_root)?;
    // settings.json：快照用户原来的默认（是我们的就不存）
    let mut settings_root = parse_object(settings_text, "settings.json")?;
    let prior_provider = settings_root.get("defaultProvider").and_then(Value::as_str).map(str::to_string);
    let prior_model = settings_root.get("defaultModel").and_then(Value::as_str).map(str::to_string);
    let snapshot = (!prior_provider.as_deref().is_some_and(|p| p == PI_PROVIDER))
        .then(|| json!({ "defaultProvider": prior_provider, "defaultModel": prior_model }))
        .filter(|value| value.get("defaultProvider").is_some_and(|v| !v.is_null()) || value.get("defaultModel").is_some_and(|v| !v.is_null()));
    {
        let object = settings_root.as_object_mut().ok_or("settings.json 顶层不是对象")?;
        object.insert("defaultProvider".into(), json!(PI_PROVIDER));
        object.insert("defaultModel".into(), json!(selected));
    }
    let settings_next = render(&settings_root)?;
    Ok((models_next, settings_next, snapshot))
}

/// 断开：删 `providers.jolu`；`defaultProvider` 仍是 `jolu` 时才还原快照（否则用户改过，不动）。
/// 返回（models.json 新内容, settings.json 新内容）。
pub(crate) fn restore(models_text: &str, settings_text: &str, saved: Option<&Value>) -> Result<(String, String), String> {
    let mut models_root = parse_object(models_text, "models.json")?;
    if let Some(providers) = models_root.get_mut("providers").and_then(Value::as_object_mut) {
        providers.remove(PI_PROVIDER);
    }
    let models_next = render(&models_root)?;

    let mut settings_root = parse_object(settings_text, "settings.json")?;
    let ours = settings_root.get("defaultProvider").and_then(Value::as_str).is_some_and(|p| p == PI_PROVIDER);
    if ours {
        if let Some(object) = settings_root.as_object_mut() {
            let saved_provider = saved.and_then(|s| s.get("defaultProvider")).and_then(Value::as_str).map(str::to_string);
            let saved_model = saved.and_then(|s| s.get("defaultModel")).and_then(Value::as_str).map(str::to_string);
            match saved_provider {
                Some(provider) => {
                    object.insert("defaultProvider".into(), json!(provider));
                    match saved_model {
                        Some(model) => {
                            object.insert("defaultModel".into(), json!(model));
                        }
                        None => {
                            object.remove("defaultModel");
                        }
                    }
                }
                None => {
                    // 原来没有 defaultProvider：两个键一起删（只删 provider 会留下指向 jolu 的 model）
                    object.remove("defaultProvider");
                    object.remove("defaultModel");
                }
            }
        }
    }
    let settings_next = render(&settings_root)?;
    Ok((models_next, settings_next))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<ZcodeModel> {
        vec![
            ZcodeModel { id: "zcode/GLM-5.3-Flash".into(), name: "GLM-5.3-Flash".into(), context: 200_000, output: 32_768, image: true },
            ZcodeModel { id: "grok/grok-4".into(), name: "grok/grok-4".into(), context: 0, output: 0, image: false },
        ]
    }

    /// 本机 models.json 的形状缩影：用户的 hualong provider 在最前。
    fn models() -> String {
        let value = json!({
            "providers": {
                "hualong": {
                    "name": "华龙",
                    "baseUrl": "https://api.hualong.online",
                    "api": "openai-responses",
                    "apiKey": "sk-user-key",
                    "models": [{ "id": "gpt-5.6-sol", "name": "gpt-5.6-sol" }]
                }
            }
        });
        serde_json::to_string_pretty(&value).unwrap()
    }

    fn settings() -> String {
        json!({ "theme": "dark", "defaultProvider": "deepseek", "defaultModel": "deepseek-v4-flash" }).to_string()
    }

    #[test]
    fn wire_writes_provider_and_default_keeps_user_providers() {
        let (models_next, settings_next, snapshot) = wire(&models(), &settings(), "http://127.0.0.1:12731", "jolu-pi.abcd1234", None, &catalog()).unwrap();
        let models_root: Value = serde_json::from_str(&models_next).unwrap();
        // 用户的 hualong 原样保留
        assert_eq!(models_root.pointer("/providers/hualong/name").and_then(Value::as_str), Some("华龙"));
        assert_eq!(models_root.pointer("/providers/hualong/apiKey").and_then(Value::as_str), Some("sk-user-key"));
        // 我们的 provider：baseUrl 带 /v1、密钥明文、模型清单来自目录
        let ours = &models_root.pointer("/providers/jolu").unwrap();
        assert_eq!(ours.get("baseUrl").and_then(Value::as_str), Some("http://127.0.0.1:12731/v1"));
        assert_eq!(ours.get("api").and_then(Value::as_str), Some("openai-completions"));
        assert_eq!(ours.get("apiKey").and_then(Value::as_str), Some("jolu-pi.abcd1234"));
        let entries = ours.get("models").and_then(Value::as_array).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].get("contextWindow").and_then(Value::as_i64), Some(200_000));
        assert_eq!(entries[0].get("input"), Some(&json!(["text", "image"])));
        // 目录缺上下文/输出/图片时不写那些键
        assert!(entries[1].get("contextWindow").is_none());
        assert!(entries[1].get("input").is_none());
        // settings：默认指向我们，用户的 theme 保留
        let settings_root: Value = serde_json::from_str(&settings_next).unwrap();
        assert_eq!(settings_root.get("defaultProvider").and_then(Value::as_str), Some("jolu"));
        assert_eq!(settings_root.get("defaultModel").and_then(Value::as_str), Some("zcode/GLM-5.3-Flash"));
        assert_eq!(settings_root.get("theme").and_then(Value::as_str), Some("dark"));
        // 快照记了用户原来的默认
        let snap = snapshot.expect("用户原来的默认要进快照");
        assert_eq!(snap.get("defaultProvider").and_then(Value::as_str), Some("deepseek"));
        assert_eq!(snap.get("defaultModel").and_then(Value::as_str), Some("deepseek-v4-flash"));
        // 状态判定
        assert!(selected_ours(&settings_next));
    }

    #[test]
    fn restore_removes_our_provider_and_brings_back_user_default() {
        let (models_wired, settings_wired, snapshot) = wire(&models(), &settings(), "http://127.0.0.1:12731", "jolu-pi.abcd1234", None, &catalog()).unwrap();
        let (models_back, settings_back) = restore(&models_wired, &settings_wired, snapshot.as_ref()).unwrap();
        let models_root: Value = serde_json::from_str(&models_back).unwrap();
        assert!(models_root.pointer("/providers/jolu").is_none(), "我们的 provider 要删干净");
        assert_eq!(models_root.pointer("/providers/hualong/name").and_then(Value::as_str), Some("华龙"), "用户的保留");
        let settings_root: Value = serde_json::from_str(&settings_back).unwrap();
        assert_eq!(settings_root.get("defaultProvider").and_then(Value::as_str), Some("deepseek"), "还原成用户原来的");
        assert_eq!(settings_root.get("defaultModel").and_then(Value::as_str), Some("deepseek-v4-flash"));
    }

    #[test]
    fn restore_leaves_a_default_the_user_switched_to() {
        let (models_wired, settings_wired, snapshot) = wire(&models(), &settings(), "http://127.0.0.1:12731", "jolu-pi.abcd1234", None, &catalog()).unwrap();
        // 用户在 Pi 里把默认换成了自己的 provider：断开不能改回去
        let changed = settings_wired.replace("\"defaultProvider\": \"jolu\"", "\"defaultProvider\": \"hualong\"");
        let (_, settings_back) = restore(&models_wired, &changed, snapshot.as_ref()).unwrap();
        let root: Value = serde_json::from_str(&settings_back).unwrap();
        assert_eq!(root.get("defaultProvider").and_then(Value::as_str), Some("hualong"));
    }

    #[test]
    fn wire_without_prior_default_snapshots_nothing_and_restore_clears_both() {
        let empty = "{}";
        let (models_wired, settings_wired, snapshot) = wire(empty, empty, "http://127.0.0.1:12731", "t", None, &catalog()).unwrap();
        assert!(snapshot.is_none(), "原来没有默认 provider 不用存");
        let (_, settings_back) = restore(&models_wired, &settings_wired, None).unwrap();
        let root: Value = serde_json::from_str(&settings_back).unwrap();
        assert!(root.get("defaultProvider").is_none());
        assert!(root.get("defaultModel").is_none(), "不能留下指向 jolu 的 model");
    }

    #[test]
    fn wire_rejects_an_empty_catalog() {
        assert!(wire(&models(), &settings(), "http://127.0.0.1:12731", "t", None, &[]).is_err());
    }
}
