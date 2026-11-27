use super::files::write_atomic;
use std::path::{Path, PathBuf};

/// ZCode：`provider` JSON 的 `options.baseURL` + `apiKey`（结构对齐 magpie zcode.go 的字段名）。
/// 接给 ZCode 的一个网关模型（2.9）。
#[derive(Debug, Clone, PartialEq)]
pub struct ZcodeModel {
    /// 网关模型 id（`来源/模型` 或 `group/分组`）
    pub id: String,
    pub name: String,
    /// 上下文窗口；0 = 不知道
    pub context: i64,
    /// 最大输出；0 = 不知道（不写，ZCode 按自己的 32000 截断）
    pub output: i64,
    pub image: bool,
}

/// ZCode 规则与 config.json 里 Jolu 的 provider id。
pub(crate) const ZCODE_PROVIDER: &str = "jolu";
/// 上下文不知道时写的值：ZCode 要一个数，magpie 同样退回 200000。
pub(crate) const ZCODE_DEFAULT_CONTEXT: i64 = 200_000;

/// ZCode 的数据目录：与 Agent 探测同源（`creds::zcode_data_dir`：`ZCODE_HOME` 覆盖 →
/// ZCode 主目录 → 设置里改过的「数据存储路径」`dataBaseDir`），保证探测到的就是写的。
pub(crate) fn zcode_dir_public(home: &Path) -> PathBuf { zcode_dir(home) }

pub(crate) fn zcode_dir(home: &Path) -> PathBuf {
    crate::creds::zcode_data_dir(home)
}

pub(crate) fn parse_object(text: &str, what: &str) -> Result<serde_json::Value, String> {
    if text.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(|error| format!("{what}不是有效 JSON：{error}"))?;
    if !value.is_object() {
        return Err(format!("{what}顶层不是对象"));
    }
    Ok(value)
}

/// 在对象里按路径取子对象，沿途缺的补空对象；路径上有非对象值时报错（不覆盖用户数据）。
pub(crate) fn object_path<'a>(value: &'a mut serde_json::Value, path: &[&str], what: &str) -> Result<&'a mut serde_json::Map<String, serde_json::Value>, String> {
    let mut current = value;
    for key in path {
        let object = current.as_object_mut().ok_or_else(|| format!("{what}里「{key}」的上级不是对象"))?;
        current = object.entry(key.to_string()).or_insert_with(|| serde_json::json!({}));
    }
    current.as_object_mut().ok_or_else(|| format!("{what}里「{}」不是对象", path.join(".")))
}

pub(crate) fn array_at<'a>(object: &'a mut serde_json::Map<String, serde_json::Value>, key: &str, what: &str) -> Result<&'a mut Vec<serde_json::Value>, String> {
    object
        .entry(key.to_string())
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| format!("{what}里「{key}」不是数组"))
}

pub(crate) fn is_ours(rule: &serde_json::Value) -> bool {
    rule.get("providerId").and_then(|v| v.as_str()) == Some(ZCODE_PROVIDER)
}

/// 新版 ZCode（3.14 起）的 `v2/provider_config.json`：provider 规则 + 每个模型的规则。
///
/// 为什么每个模型都要规则：ZCode 不知道模型的输出上限时把每次回复截到 32000 token，
/// 长输出会在中途断掉；上下文窗口与是否收图片也靠模型规则告诉它。用户在 ZCode 里手动设过
/// 的模型（`manualProviderModelRules`）不动。参照 magpie `internal/agent/zcode.go`。
pub(crate) fn zcode_rules_wire(text: &str, base: &str, token: &str, models: &[ZcodeModel]) -> Result<String, String> {
    let what = "ZCode 的 provider_config.json ";
    let mut root = parse_object(text, what)?;
    if root.get("schemaVersion").is_none() {
        root["schemaVersion"] = serde_json::json!(1);
    }
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    {
        let config = object_path(&mut root, &["config"], what)?;
        let order = array_at(config, "providerOrder", what)?;
        if !order.iter().any(|v| v.as_str() == Some(ZCODE_PROVIDER)) {
            order.push(serde_json::json!(ZCODE_PROVIDER));
        }
    }
    {
        let rules = object_path(&mut root, &["config", "providerConfigRules"], what)?;
        let list = array_at(rules, "providerRules", what)?;
        list.retain(|rule| !is_ours(rule));
        list.push(serde_json::json!({
            "providerId": ZCODE_PROVIDER,
            "providerName": "Jolu",
            "enabled": true,
            "config": {
                "group": "standard-personal",
                "access": {"type": "api-key", "apiKey": token},
                "api": {"type": "anthropic-messages", "baseUrl": base},
                "personalModelIds": ids,
                "modelOrder": ids,
            },
        }));
    }
    {
        let rules = object_path(&mut root, &["config", "modelConfigRules"], what)?;
        array_at(rules, "manualProviderModelRules", what)?;
        let list = array_at(rules, "providerModelRules", what)?;
        list.retain(|rule| !is_ours(rule));
        for model in models {
            let mut config = serde_json::json!({
                "properties": {
                    "contextWindow": if model.context > 0 { model.context } else { ZCODE_DEFAULT_CONTEXT },
                    "inputFormat": {"supportsImage": model.image},
                },
            });
            if model.output > 0 {
                config["optionSpecs"] = serde_json::json!({"maxOutputTokens": {"max": model.output}});
            }
            list.push(serde_json::json!({"providerId": ZCODE_PROVIDER, "modelId": model.id, "config": config}));
        }
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("{what}序列化失败：{error}"))
}

/// 旧版 ZCode 读的 `v2/config.json`：OpenCode 形状的 provider。
pub(crate) fn zcode_config_wire(text: &str, base: &str, token: &str, models: &[ZcodeModel]) -> Result<String, String> {
    let what = "ZCode 的 config.json ";
    let mut root = parse_object(text, what)?;
    let mut entries = serde_json::Map::new();
    for model in models {
        let mut limit = serde_json::json!({"context": if model.context > 0 { model.context } else { ZCODE_DEFAULT_CONTEXT }});
        if model.output > 0 {
            limit["output"] = serde_json::json!(model.output);
        }
        let input = if model.image { serde_json::json!(["text", "image"]) } else { serde_json::json!(["text"]) };
        entries.insert(model.id.clone(), serde_json::json!({
            "name": model.name,
            "limit": limit,
            "modalities": {"input": input, "output": ["text"]},
        }));
    }
    let providers = object_path(&mut root, &["provider"], what)?;
    // 用户在 ZCode 里关掉过的保持关闭
    let enabled = providers.get(ZCODE_PROVIDER).and_then(|p| p.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(true);
    providers.insert(ZCODE_PROVIDER.into(), serde_json::json!({
        "name": "Jolu",
        "kind": "anthropic",
        "enabled": enabled,
        "source": "custom",
        "options": {"apiKey": token, "baseURL": base},
        "models": entries,
    }));
    serde_json::to_string_pretty(&root).map_err(|error| format!("{what}序列化失败：{error}"))
}

/// ZCode 规则文件的快照：记下接线前**哪几处结构本来就有**。
///
/// 不存整份原文是因为 restore 仍要在「用户后来自己改过」的文件上操作（外部意图优先），
/// 只需要知道「这些壳是不是我们建的」——三处新建 + schemaVersion 一个布尔就够了。
pub(crate) fn zcode_rules_snapshot(text: &str) -> Option<serde_json::Value> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    Some(serde_json::json!({
        "schemaVersion": root.get("schemaVersion").is_some(),
        "providerOrder": root.pointer("/config/providerOrder").is_some(),
        "providerRules": root.pointer("/config/providerConfigRules/providerRules").is_some(),
        "modelConfigRules": root.pointer("/config/modelConfigRules").is_some(),
    }))
}

pub(crate) fn zcode_rules_restore(text: &str, saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut root = parse_object(text, "ZCode 的 provider_config.json ")?;
    // `saved` 里是接线前「这几处是否存在」的布尔；没有快照（旧版本接的线）时按
    // 「都存在」处理——宁可留着空壳，也不擅自删用户的键。
    let had = |key: &str| saved.and_then(|saved| saved.get(key)).and_then(|v| v.as_bool()).unwrap_or(true);
    if let Some(order) = root.pointer_mut("/config/providerOrder").and_then(|o| o.as_array_mut()) {
        order.retain(|v| v.as_str() != Some(ZCODE_PROVIDER));
    }
    if let Some(list) = root.pointer_mut("/config/providerConfigRules/providerRules").and_then(|l| l.as_array_mut()) {
        list.retain(|rule| !is_ours(rule));
    }
    if let Some(list) = root.pointer_mut("/config/modelConfigRules/providerModelRules").and_then(|l| l.as_array_mut()) {
        list.retain(|rule| !is_ours(rule));
    }
    // 接线时新建的空壳要收掉，理由见 `zcode_rules_snapshot`。
    if !had("providerOrder") {
        prune_empty_json_at(&mut root, &["config", "providerOrder"]);
    }
    if !had("providerRules") {
        prune_empty_json_at(&mut root, &["config", "providerConfigRules", "providerRules"]);
    }
    if !had("modelConfigRules") {
        prune_empty_json_at(&mut root, &["config", "modelConfigRules", "manualProviderModelRules"]);
        prune_empty_json_at(&mut root, &["config", "modelConfigRules", "providerModelRules"]);
        prune_empty_json_object(&mut root, &["config", "modelConfigRules"]);
    }
    if !had("schemaVersion") && root.get("schemaVersion").and_then(|v| v.as_i64()) == Some(1) {
        if let Some(object) = root.as_object_mut() {
            object.remove("schemaVersion");
        }
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("ZCode 配置序列化失败：{error}"))
}

/// 删掉一个空数组键（路径不存在或不是空数组则不动）。
pub(crate) fn prune_empty_json_at(root: &mut serde_json::Value, path: &[&str]) {
    let Some((last, parents)) = path.split_last() else { return };
    let Some(parent) = parents.iter().try_fold(root, |node, key| node.get_mut(*key)) else { return };
    let empty = parent.get(*last).and_then(|v| v.as_array()).is_some_and(|a| a.is_empty());
    if empty {
        if let Some(object) = parent.as_object_mut() {
            object.remove(*last);
        }
    }
}

/// 删掉一个空对象键（路径不存在或不是空对象则不动）。
pub(crate) fn prune_empty_json_object(root: &mut serde_json::Value, path: &[&str]) {
    let Some((last, parents)) = path.split_last() else { return };
    let Some(parent) = parents.iter().try_fold(root, |node, key| node.get_mut(*key)) else { return };
    let empty = parent.get(*last).and_then(|v| v.as_object()).is_some_and(|o| o.is_empty());
    if empty {
        if let Some(object) = parent.as_object_mut() {
            object.remove(*last);
        }
    }
}

pub(crate) fn zcode_config_restore(text: &str, saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut root = parse_object(text, "ZCode 的 config.json ")?;
    if let Some(providers) = root.get_mut("provider").and_then(|p| p.as_object_mut()) {
        providers.remove(ZCODE_PROVIDER);
    }
    if saved.and_then(|value| value.get("configProvider")).and_then(|value| value.as_bool()) == Some(false) {
        prune_empty_json_object(&mut root, &["provider"]);
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("ZCode 配置序列化失败：{error}"))
}

/// 旧版 Jolu 在 `~/.zcode/provider.json` 顶层写过 `jolu` 键（ZCode 从不读这个文件）：断开时顺带删掉。
pub(crate) fn zcode_clean_legacy(home: &Path) {
    // 旧版 Jolu 写在原位置（`ZCODE_HOME` 或 `~/.zcode`），不随 dataBaseDir 找
    let legacy = std::env::var_os("ZCODE_HOME").filter(|value| !value.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".zcode")).join("provider.json");
    let Ok(Some(text)) = crate::steady::read_optional(&legacy) else { return };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    let Some(object) = root.as_object_mut() else { return };
    if object.remove(ZCODE_PROVIDER).is_none() {
        return;
    }
    if object.is_empty() {
        let _ = std::fs::remove_file(&legacy);
    } else if let Ok(next) = serde_json::to_string_pretty(&root) {
        let _ = write_atomic(&legacy, next.into_bytes());
    }
}

pub(crate) fn zcode_rule(text: &str) -> Option<serde_json::Value> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.pointer("/config/providerConfigRules/providerRules")?
        .as_array()?
        .iter()
        .find(|rule| is_ours(rule))
        .cloned()
}

pub(crate) fn zcode_current_base(text: &str) -> Option<String> {
    zcode_rule(text)?.pointer("/config/api/baseUrl")?.as_str().map(str::to_string)
}

pub(crate) fn zcode_current_token(text: &str) -> Option<String> {
    zcode_rule(text)?.pointer("/config/access/apiKey")?.as_str().map(str::to_string)
}
