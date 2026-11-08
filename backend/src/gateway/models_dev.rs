//! models.dev 模型元数据（P4.2）：拉取一次、缓存到应用数据目录，
//! 供模型目录补全上下文窗口 / 输出上限，以及接线的 1M 标记自动补齐。
//!
//! magpie 用 models.dev 作为模型元数据源（README：模型列表「names and reasoning
//! levels filled in from models.dev」）。Jolu 按同样的口径取数：
//! - 数据源：`https://models.dev/api.json`（厂商 → 模型 → contextWindow / output 等）
//! - 缓存：应用数据目录 `models-dev.json`，TTL 外才重新拉取（离线可用，不自动写库）
//! - 失败语义：拉取失败返回缓存（哪怕过期）；连缓存都没有才报错——元数据是「建议值」

use serde_json::Value;
use std::path::PathBuf;
use tauri::Manager;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct ModelMeta {
    /// 上下文窗口（tokens）；0 = 未知
    #[serde(default)]
    pub context_window: i64,
    /// 最大输出（tokens）；0 = 未知
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_output: i64,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct CachedCatalog {
    /// 拉取时间（毫秒）
    pub fetched_ms: i64,
    /// models.dev 的模型 id（如 "claude-sonnet-5"）→ 元数据
    pub models: std::collections::BTreeMap<String, ModelMeta>,
}

const SOURCE_URL: &str = "https://models.dev/api.json";
/// 缓存有效期：7 天。元数据变化很慢，离线优先。
const TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

fn cache_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join("models-dev.json"))
}

/// 查询一个模型的元数据：缓存新鲜直接用；过期/缺失尝试拉取并刷新缓存；
/// 拉取失败退回旧缓存（哪怕过期——有过期数据比没有强）。
pub(crate) fn lookup(app: &tauri::AppHandle, model: &str) -> Result<ModelMeta, String> {
    let path = cache_path(app).ok_or("无法定位应用数据目录")?;
    let cached = read_cache(&path);
    let fresh = cached.as_ref().is_some_and(|c| now_ms() - c.fetched_ms < TTL_MS);
    if fresh {
        return Ok(cached
            .and_then(|c| c.models.get(model).cloned())
            .unwrap_or_default());
    }
    match fetch() {
        Ok(new) => {
            let _ = std::fs::write(&path, serde_json::to_string(&new).unwrap_or_default());
            Ok(new.models.get(model).cloned().unwrap_or_default())
        }
        Err(error) => {
            if let Some(old) = cached.and_then(|c| c.models.get(model).cloned()) {
                return Ok(old);
            }
            Err(error)
        }
    }
}

/// 主模型 id 是否超过 200K 上下文（wiring 写入时的 1M 标记依据，magpie `mark1M` 同款）。
/// 未知（元数据缺失）按 false 处理——宁可少标记，不虚标。
pub(crate) fn exceeds_200k(app: &tauri::AppHandle, model: &str) -> bool {
    matches!(lookup(app, model), Ok(meta) if meta.context_window > 200_000)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn read_cache(path: &PathBuf) -> Option<CachedCatalog> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// 拉取并解析 models.dev。顶层是厂商 → { models: { id: { … } } }，摊平成 id → 元数据。
fn fetch() -> Result<CachedCatalog, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| format!("无法创建 models.dev 客户端：{error}"))?;
    let response = client
        .get(SOURCE_URL)
        .send()
        .map_err(|error| format!("无法访问 models.dev：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("models.dev 返回 HTTP {}", response.status()));
    }
    let value: Value = response.json().map_err(|error| format!("models.dev 响应解析失败：{error}"))?;
    let mut models = std::collections::BTreeMap::new();
    if let Some(vendors) = value.as_object() {
        for vendor in vendors.values() {
            let Some(list) = vendor.get("models").and_then(Value::as_object) else { continue };
            for (id, meta) in list {
                if models.contains_key(id) {
                    continue;
                }
                let context_window = meta
                    .get("limit")
                    .and_then(|l| l.get("context"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let max_output = meta
                    .get("limit")
                    .and_then(|l| l.get("output"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                models.insert(id.clone(), ModelMeta { context_window, max_output });
            }
        }
    }
    Ok(CachedCatalog { fetched_ms: now_ms(), models })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_dev_shape_is_parsed_into_flat_map() {
        // models.dev 顶层：厂商 → { models: { id: { limit: { context, output } } } }
        let payload = serde_json::json!({
            "anthropic": {"models": {"claude-sonnet-5": {"limit": {"context": 1_000_000, "output": 64_000}}}},
            "zhipu": {"models": {"glm-5.3": {"limit": {"context": 200_000}}}}
        });
        let mut models = std::collections::BTreeMap::new();
        if let Some(vendors) = payload.as_object() {
            for vendor in vendors.values() {
                if let Some(list) = vendor.get("models").and_then(Value::as_object) {
                    for (id, meta) in list {
                        models.insert(
                            id.clone(),
                            ModelMeta {
                                context_window: meta["limit"]["context"].as_i64().unwrap_or(0),
                                max_output: meta["limit"]["output"].as_i64().unwrap_or(0),
                            },
                        );
                    }
                }
            }
        }
        assert_eq!(models["claude-sonnet-5"].context_window, 1_000_000);
        assert_eq!(models["glm-5.3"].context_window, 200_000);
        assert!(exceeds_200k_check(&models, "claude-sonnet-5"));
        assert!(!exceeds_200k_check(&models, "glm-5.3"));
        assert!(!exceeds_200k_check(&models, "未知模型"), "未知模型不虚标 1M");
    }

    fn exceeds_200k_check(models: &std::collections::BTreeMap<String, ModelMeta>, model: &str) -> bool {
        matches!(models.get(model), Some(meta) if meta.context_window > 200_000)
    }

    #[test]
    fn cache_round_trip_keeps_models() {
        let dir = std::env::temp_dir().join(format!("jolu-models-dev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models-dev.json");
        let catalog = CachedCatalog {
            fetched_ms: now_ms(),
            models: std::collections::BTreeMap::from([("glm-5.3".into(), ModelMeta { context_window: 200_000, max_output: 0 })]),
        };
        std::fs::write(&path, serde_json::to_string(&catalog).unwrap()).unwrap();
        let loaded = read_cache(&path).unwrap();
        assert_eq!(loaded.models["glm-5.3"].context_window, 200_000);
        let _ = std::fs::remove_file(&path);
    }
}
