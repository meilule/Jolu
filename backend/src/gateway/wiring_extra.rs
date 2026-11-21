//! 5.11 补接的四家 JSON 工具：Pencil、Qoder、Qoder CN、Crush（参照 magpie 对应实现）。
//!
//! 四家都只改一个 JSON 文件，Jolu 的东西放在固定的 `providers.jolu` 对象下（provider 整段写，
//! 含网关模型目录生成的 models 数组），另外 Qoder 写选中模型 `model.name`、Crush 写
//! `models.large` / `models.small` 选择。断开时删掉 `providers.jolu`，选择键只在仍指向 Jolu 时
//! 还原成接线前的值（用户在工具里改选了别的模型就不动）。
//!
//! 出处（magpie `internal/agent/`）：`pencil.go:34-119`（pencilProviderJSON）、
//! `qoder.go:71-204,298-323`（qoderAt / qoderProviderAt）、`agents.go:697-729,2057-2218`
//! （magpieProviderJSONAt 的 crush 形状 / crushAt）。与 magpie 的差异：不保留用户在
//! `providers.jolu` 里手改的字段（magpie theirsKept），每次接入整段重写；不写推理强度。

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::wiring::ZcodeModel;

/// Jolu 在这几家配置里的 provider 段名
const SLOT: &str = "jolu";

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub(crate) fn config_path(agent: &str, home: &Path) -> Option<PathBuf> {
    Some(match agent {
        "pencil" => home.join(".pencil").join("models.json"),
        "qoder" => env_dir("QODER_CONFIG_DIR").unwrap_or_else(|| home.join(".qoder")).join("settings.json"),
        "qoder-cn" => env_dir("QODERCN_CONFIG_DIR").unwrap_or_else(|| home.join(".qoder-cn")).join("settings.json"),
        _ => return None,
    })
}

fn parse(text: &str) -> Result<Value, String> {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(text).map_err(|error| format!("配置不是有效 JSON：{error}"))?;
    if !value.is_object() {
        return Err("配置顶层不是对象".into());
    }
    Ok(value)
}

fn render(root: &Value) -> Result<String, String> {
    let mut text = serde_json::to_string_pretty(root).map_err(|error| format!("配置序列化失败：{error}"))?;
    text.push('\n');
    Ok(text)
}

fn slot(root: &Value) -> Option<&Value> {
    root.get("providers")?.get(SLOT)
}

fn str_at<'a>(value: Option<&'a Value>, key: &str) -> Option<&'a str> {
    value?.get(key)?.as_str()
}

/// 当前 `providers.jolu` 里写的网关地址
pub(crate) fn current_base(_agent: &str, text: &str) -> Option<String> {
    let root = parse(text).ok()?;
    let key = "baseUrl";
    str_at(slot(&root), key).map(|s| s.trim().to_string())
}

pub(crate) fn current_token(_agent: &str, text: &str) -> Option<String> {
    let root = parse(text).ok()?;
    let key = "apiKey";
    str_at(slot(&root), key).map(|s| s.trim().to_string())
}

/// 工具当前选中的是不是 Jolu 的模型：Qoder 看 `model.name` 的 `jolu/` 前缀，Crush 看
/// `models.large` / `models.small` 的 provider；Pencil 没有选中模型的键（在它界面里选），provider 在就算。
pub(crate) fn selected_ours(agent: &str, text: &str) -> bool {
    let Ok(root) = parse(text) else { return false };
    match agent {
        "qoder" | "qoder-cn" => root.pointer("/model/name").and_then(Value::as_str).is_some_and(|name| name.starts_with("jolu/")),
        _ => slot(&root).is_some(),
    }
}

/// 选中的网关模型：给了就用，否则取目录第一个
fn pick<'a>(model: Option<&'a str>, catalog: &'a [ZcodeModel]) -> Option<&'a str> {
    model.map(str::trim).filter(|m| !m.is_empty()).or_else(|| catalog.first().map(|m| m.id.as_str()))
}

fn pencil_provider(base: &str, token: &str, catalog: &[ZcodeModel]) -> Value {
    let models: Vec<Value> = catalog
        .iter()
        .map(|m| {
            let window = if m.context > 0 { m.context } else { 128_000 };
            let out = if m.output > 0 { m.output.min(window) } else { 16_384.min(window) };
            json!({
                "id": m.id, "name": m.name, "reasoning": false,
                "input": if m.image { json!(["text", "image"]) } else { json!(["text"]) },
                "contextWindow": window, "maxTokens": out,
                "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
                "compat": {},
            })
        })
        .collect();
    json!({"name": "Jolu", "baseUrl": format!("{base}/v1"), "api": "openai-completions", "apiKey": token, "models": models})
}

fn qoder_provider(base: &str, token: &str, selected: &str, catalog: &[ZcodeModel]) -> Value {
    let models: Vec<Value> = catalog
        .iter()
        .map(|m| {
            let mut entry = json!({"model": m.id, "displayName": m.name, "capabilities": {"tools": true, "vision": m.image}});
            if m.context > 0 {
                entry["contextWindow"] = json!(m.context);
            }
            if m.output > 0 {
                entry["maxOutputTokens"] = json!(if m.context > 0 { m.output.min(m.context) } else { m.output });
            }
            entry
        })
        .collect();
    json!({"displayName": "Jolu", "protocol": "openai", "baseUrl": format!("{base}/v1"), "apiKey": token, "model": selected, "models": models})
}

fn object_at<'a>(root: &'a mut Value, key: &str) -> Result<&'a mut Map<String, Value>, String> {
    let object = root.as_object_mut().ok_or("配置顶层不是对象")?;
    let entry = object.entry(key.to_string()).or_insert_with(|| json!({}));
    entry.as_object_mut().ok_or_else(|| format!("{key} 不是对象，Jolu 不改写它"))
}

/// 接入：整段写 `providers.jolu`，再写选择键。目录为空时报错（写一个空模型表工具也用不了）。
pub(crate) fn wire(agent: &str, text: &str, base: &str, token: &str, model: Option<&str>, catalog: &[ZcodeModel]) -> Result<String, String> {
    if catalog.is_empty() {
        return Err("网关模型清单为空；先在「模型池」里启用要提供给这个工具的模型".into());
    }
    let mut root = parse(text)?;
    let selected = pick(model, catalog).ok_or("没有可选的模型")?.to_string();
    let provider = match agent {
        "pencil" => pencil_provider(base, token, catalog),
        "qoder" | "qoder-cn" => qoder_provider(base, token, &selected, catalog),
        _ => return Err("该工具不在接线范围".into()),
    };
    object_at(&mut root, "providers")?.insert(SLOT.into(), provider);
    match agent {
        "qoder" | "qoder-cn" => {
            object_at(&mut root, "model")?.insert("name".into(), json!(format!("{SLOT}/{selected}")));
        }
        _ => {}
    }
    render(&root)
}

/// 接线前的原值（只记 Jolu 会改的选择键与原有的 `providers.jolu`）；已经是我们写的值不算原值。
pub(crate) fn snapshot(agent: &str, text: &str) -> Option<Value> {
    let root = parse(text).ok()?;
    let theirs_slot = slot(&root).filter(|value| {
        let base = value.get("baseUrl").or_else(|| value.get("base_url")).and_then(Value::as_str).unwrap_or_default();
        !super::wiring::is_gateway_value(base)
    });
    let mut saved = Map::new();
    if let Some(value) = theirs_slot {
        saved.insert("slot".into(), value.clone());
    }
    match agent {
        "qoder" | "qoder-cn" => {
            if let Some(name) = root.pointer("/model/name").filter(|v| !v.as_str().is_some_and(|s| s.starts_with("jolu/"))) {
                saved.insert("model.name".into(), name.clone());
            }
        }
        _ => {}
    }
    Some(Value::Object(saved))
}

fn prune_empty(root: &mut Value, key: &str) {
    if root.get(key).and_then(Value::as_object).is_some_and(Map::is_empty) {
        if let Some(object) = root.as_object_mut() {
            object.remove(key);
        }
    }
}

/// 断开：删 `providers.jolu`（接线前那里有用户自己的段就放回），选择键仍指向 Jolu 时还原或删除。
pub(crate) fn restore(agent: &str, text: &str, saved: Option<&Value>) -> Result<String, String> {
    let mut root = parse(text)?;
    let saved_at = |key: &str| saved.and_then(|s| s.get(key)).cloned();
    if let Some(providers) = root.get_mut("providers").and_then(Value::as_object_mut) {
        match saved_at("slot") {
            Some(theirs) => {
                providers.insert(SLOT.into(), theirs);
            }
            None => {
                providers.remove(SLOT);
            }
        }
    }
    prune_empty(&mut root, "providers");
    match agent {
        "qoder" | "qoder-cn" => {
            let ours = root.pointer("/model/name").and_then(Value::as_str).is_some_and(|s| s.starts_with("jolu/"));
            if ours {
                if let Some(model) = root.get_mut("model").and_then(Value::as_object_mut) {
                    match saved_at("model.name") {
                        Some(name) => {
                            model.insert("name".into(), name);
                        }
                        None => {
                            model.remove("name");
                        }
                    }
                }
                prune_empty(&mut root, "model");
            }
        }
        _ => {}
    }
    render(&root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<ZcodeModel> {
        vec![
            ZcodeModel { id: "kimi/k2".into(), name: "Kimi K2".into(), context: 256_000, output: 32_768, image: false },
            ZcodeModel { id: "group/main".into(), name: "group/main".into(), context: 0, output: 0, image: true },
        ]
    }
    const BASE: &str = "http://127.0.0.1:12731";
    const TOKEN: &str = "jolu-qoder.abcd1234";

    #[test]
    fn qoder_wire_and_restore_round_trip_keeps_user_keys() {
        let original = r#"{"enabledPlugins":{"x":true},"model":{"name":"qwen3-coder","reasoningEffort":"high"},"providers":{"mine":{"baseUrl":"https://a"}}}"#;
        let saved = snapshot("qoder", original);
        let wired = wire("qoder", original, BASE, TOKEN, Some("kimi/k2"), &catalog()).unwrap();
        let root: Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(root["model"]["name"], "jolu/kimi/k2");
        assert_eq!(root["model"]["reasoningEffort"], "high", "用户的键不动");
        assert_eq!(root["providers"]["jolu"]["baseUrl"], format!("{BASE}/v1"));
        assert_eq!(root["providers"]["jolu"]["protocol"], "openai");
        assert_eq!(root["providers"]["jolu"]["models"][0]["contextWindow"], 256_000);
        assert!(root["providers"]["jolu"]["models"][1].get("contextWindow").is_none(), "不知道的窗口不编");
        assert_eq!(root["providers"]["mine"]["baseUrl"], "https://a");
        assert_eq!(current_base("qoder", &wired).as_deref(), Some("http://127.0.0.1:12731/v1"));
        assert_eq!(current_token("qoder", &wired).as_deref(), Some(TOKEN));
        assert!(selected_ours("qoder", &wired));

        let restored: Value = serde_json::from_str(&restore("qoder", &wired, saved.as_ref()).unwrap()).unwrap();
        assert_eq!(restored, serde_json::from_str::<Value>(original).unwrap());
    }

    #[test]
    fn qoder_restore_leaves_a_model_the_user_switched_to() {
        let wired = wire("qoder", "{}", BASE, TOKEN, None, &catalog()).unwrap();
        let root: Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(root["model"]["name"], "jolu/kimi/k2", "没给模型取目录第一个");
        let switched = wired.replace("jolu/kimi/k2\"", "qwen3-max\"");
        let restored: Value = serde_json::from_str(&restore("qoder", &switched, snapshot("qoder", "{}").as_ref()).unwrap()).unwrap();
        assert_eq!(restored["model"]["name"], "qwen3-max");
        assert!(restored.get("providers").is_none());
    }

    #[test]
    fn pencil_writes_full_model_entries_and_keeps_theirs_slot() {
        let original = r#"{"providers":{"jolu":{"baseUrl":"https://my-own","apiKey":"k"},"other":{}}}"#;
        let saved = snapshot("pencil", original);
        let wired = wire("pencil", original, BASE, "jolu-pencil.abcd1234", None, &catalog()).unwrap();
        let root: Value = serde_json::from_str(&wired).unwrap();
        let model = &root["providers"]["jolu"]["models"][1];
        assert_eq!(model["contextWindow"], 128_000);
        assert_eq!(model["maxTokens"], 16_384);
        assert_eq!(model["input"], json!(["text", "image"]));
        assert_eq!(root["providers"]["jolu"]["api"], "openai-completions");
        let restored: Value = serde_json::from_str(&restore("pencil", &wired, saved.as_ref()).unwrap()).unwrap();
        assert_eq!(restored, serde_json::from_str::<Value>(original).unwrap(), "用户自己的同名段放回");
    }

    #[test]
    fn refuses_empty_catalog_bad_json_and_non_object_slots() {
        assert!(wire("crush", "{}", BASE, TOKEN, None, &[]).is_err());
        assert!(wire("crush", "{oops", BASE, TOKEN, None, &catalog()).is_err());
        assert!(wire("crush", r#"{"providers":[]}"#, BASE, TOKEN, None, &catalog()).is_err());
        assert!(wire("qoder", r#"{"model":"x"}"#, BASE, TOKEN, None, &catalog()).is_err());
        assert!(wire("qoder", "\u{feff}{}", BASE, TOKEN, None, &catalog()).is_ok(), "带 BOM 的也能读");
    }

    #[test]
    fn snapshot_ignores_values_we_wrote() {
        let wired = wire("pencil", "{}", BASE, TOKEN, None, &catalog()).unwrap();
        assert_eq!(snapshot("pencil", &wired), Some(json!({})));
        let wired = wire("qoder", "{}", BASE, TOKEN, None, &catalog()).unwrap();
        assert_eq!(snapshot("qoder", &wired), Some(json!({})));
    }
}
