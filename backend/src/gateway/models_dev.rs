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
pub struct ModelMeta {
    /// 上下文窗口（tokens）；0 = 未知
    #[serde(default)]
    pub context_window: i64,
    /// 最大输出（tokens）；0 = 未知
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_output: i64,
    /// 显示名（Claude Code 档位的 _NAME 用）；空 = 未知
    #[serde(default)]
    pub label: String,
    /// 描述（档位的 _DESCRIPTION 用）；空 = 未知
    #[serde(default)]
    pub description: String,
    /// 是否推理模型；None = models.dev 没说
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    /// 可设的推理档位（models.dev `reasoning_options` 里的 effort 值）；空 = 没给
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    /// 输入模态（text / image / pdf / audio / video）；空 = 没给
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modalities: Vec<String>,
    /// 是否支持工具调用；None = 没给
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<bool>,
}

/// 内置兜底表：models.dev 主要厂商的快照（`scripts/dev/models-dev-builtin.mjs` 生成，表头有日期）。
/// 只在从没拉到过 models.dev（首次离线启动）时用；拉到过就用缓存，哪怕过期。
static BUILTIN: std::sync::LazyLock<CachedCatalog> = std::sync::LazyLock::new(|| {
    #[derive(serde::Deserialize)]
    struct Snapshot {
        models: std::collections::BTreeMap<String, ModelMeta>,
    }
    serde_json::from_str::<Snapshot>(include_str!("models_dev_builtin.json"))
        .map(|snapshot| CachedCatalog { fetched_ms: 0, models: snapshot.models })
        .unwrap_or_default()
});

/// 内置兜底表（见 [`BUILTIN`]）。
pub fn builtin() -> &'static CachedCatalog {
    &BUILTIN
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct CachedCatalog {
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

/// 主模型 id 是否超过 200K 上下文（wiring 写入时的 1M 标记依据，magpie `mark1M` 同款）。
/// 只读缓存：元数据缺失（缓存还没被目录页的后台刷新填上）按 false 处理——宁可少标记，不虚标。
pub(crate) fn exceeds_200k(app: &tauri::AppHandle, model: &str) -> bool {
    cached(app)
        .and_then(|catalog| lookup_meta(&catalog, model))
        .is_some_and(|meta| meta.context_window > 200_000)
}

/// 模型的显示名（档位 _NAME 用）：缓存里的 label 优先，缺失时退回模型 id 本身。
pub(crate) fn display_label(app: &tauri::AppHandle, model: &str) -> String {
    cached(app)
        .and_then(|catalog| lookup_meta(&catalog, model))
        .map(|meta| meta.label)
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| model.to_string())
}

/// 模型的描述（档位 _DESCRIPTION 用）；未知返回空。
pub(crate) fn description_of(app: &tauri::AppHandle, model: &str) -> String {
    cached(app)
        .and_then(|catalog| lookup_meta(&catalog, model))
        .map(|meta| meta.description)
        .unwrap_or_default()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 在 models.dev 目录里找一个模型：精确 id → 末段匹配（`zai-org/GLM-5.3-Flash` 的
/// 末段是 `GLM-5.3-Flash`）→ 忽略大小写。Jolu 侧的模型 id 大小写不保证一致。
pub fn lookup_meta(catalog: &CachedCatalog, model: &str) -> Option<ModelMeta> {
    let wanted = model.trim();
    if let Some(meta) = catalog.models.get(wanted) {
        return Some(meta.clone());
    }
    let lower = wanted.to_ascii_lowercase();
    for (id, meta) in &catalog.models {
        if id.eq_ignore_ascii_case(wanted) {
            return Some(meta.clone());
        }
        if let Some((_, tail)) = id.rsplit_once('/') {
            if tail.eq_ignore_ascii_case(wanted) || id.to_ascii_lowercase().ends_with(&format!("/{lower}")) {
                return Some(meta.clone());
            }
        }
    }
    None
}

/// 只读缓存（不联网）：接线 / 目录展示用——阻塞 20 秒的拉取只能发生在后台。
pub(crate) fn cached(app: &tauri::AppHandle) -> Option<CachedCatalog> {
    let path = cache_path(app)?;
    // 从没拉到过时用内置快照：首次离线也能显示上下文窗口与档位（2.8）
    read_cache(&path).or_else(|| Some(builtin().clone()))
}

/// 立即拉取并写缓存（「刷新模型元数据」按钮用，阻塞，调用方放到后台线程）。返回（模型数，拉取时间）。
pub(crate) fn refresh_now(app: &tauri::AppHandle) -> Result<(usize, i64), String> {
    let path = cache_path(app).ok_or("无法定位应用数据目录")?;
    let catalog = fetch()?;
    std::fs::write(&path, serde_json::to_string(&catalog).map_err(|error| error.to_string())?)
        .map_err(|error| format!("写入模型元数据缓存失败：{error}"))?;
    Ok((catalog.models.len(), catalog.fetched_ms))
}

/// 缓存过期或缺失时在后台拉取刷新（不阻塞调用方）；失败静默——下次再试。
pub(crate) fn refresh_if_stale(app: &tauri::AppHandle) {
    let Some(path) = cache_path(app) else { return };
    let stale = read_cache(&path).is_none_or(|c| now_ms() - c.fetched_ms >= TTL_MS);
    if !stale {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || {
        if let Ok(new) = fetch() {
            let _ = std::fs::write(&path, serde_json::to_string(&new).unwrap_or_default());
        }
    });
}

fn read_cache(path: &PathBuf) -> Option<CachedCatalog> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// 解析 models.dev：顶层是厂商 → { models: { id: { … } } }，摊平成 id → 元数据（同名先到先得）。
///
/// 以前只读了 limit，`name` 没读——各档显示名一律退回模型 id；现在名称、描述、推理档位、
/// 输入模态与工具调用都读出来（2.8）。
pub(crate) fn parse_catalog(value: &Value) -> std::collections::BTreeMap<String, ModelMeta> {
    const LEVELS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];
    let mut models = std::collections::BTreeMap::new();
    let Some(vendors) = value.as_object() else { return models };
    for vendor in vendors.values() {
        let Some(list) = vendor.get("models").and_then(Value::as_object) else { continue };
        for (id, meta) in list {
            if models.contains_key(id) {
                continue;
            }
            let text = |key: &str| meta.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            let limit = |key: &str| meta.pointer(&format!("/limit/{key}")).and_then(Value::as_i64).unwrap_or(0);
            let efforts = meta
                .get("reasoning_options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|option| option.get("type").and_then(Value::as_str) == Some("effort"))
                .flat_map(|option| option.get("values").and_then(Value::as_array).cloned().unwrap_or_default())
                .filter_map(|v| v.as_str().map(str::to_string))
                .filter(|v| LEVELS.contains(&v.as_str()))
                .collect();
            let input_modalities = meta
                .pointer("/modalities/input")
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            models.insert(
                id.clone(),
                ModelMeta {
                    context_window: limit("context"),
                    max_output: limit("output"),
                    label: text("name"),
                    description: text("description"),
                    reasoning: meta.get("reasoning").and_then(Value::as_bool),
                    efforts,
                    input_modalities,
                    tool_call: meta.get("tool_call").and_then(Value::as_bool),
                },
            );
        }
    }
    models
}

/// 拉取并解析 models.dev。
fn fetch() -> Result<CachedCatalog, String> {
    // retry_blocking_get 内部走 net::blocking_builder：尊重设置里的联网方式（裸
    // builder 会按 reqwest 默认探测环境变量与注册表，用户选直连也照样走系统代理）。
    // 目录是后台低频刷新，对 CDN 的 5xx/429 重试两次不构成负担（magpie a46190e 同思路）
    let response = crate::net::retry_blocking_get(SOURCE_URL, 3, std::time::Duration::from_secs(20))
        .map_err(|error| format!("无法访问 models.dev：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("models.dev 返回 HTTP {}", response.status()));
    }
    let value: Value = response.json().map_err(|error| format!("models.dev 响应解析失败：{error}"))?;
    let models = parse_catalog(&value);
    Ok(CachedCatalog { fetched_ms: now_ms(), models })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_descriptions_efforts_and_modalities_are_read() {
        let payload = serde_json::json!({
            "anthropic": {"models": {"claude-opus-4-5": {
                "name": "Claude Opus 4.5", "description": "旗舰", "reasoning": true, "tool_call": true,
                "reasoning_options": [{"type": "effort", "values": ["low", "medium", "high", "turbo"]}, {"type": "budget_tokens", "min": 1024}],
                "modalities": {"input": ["text", "image", "pdf"]}, "limit": {"context": 200000, "output": 64000}
            }}},
            "other": {"models": {"claude-opus-4-5": {"name": "重复的不覆盖"}, "bare": {}}}
        });
        let models = parse_catalog(&payload);
        let opus = &models["claude-opus-4-5"];
        assert_eq!(opus.label, "Claude Opus 4.5", "以前名称没读，显示名一律退回 id");
        assert_eq!(opus.description, "旗舰");
        assert_eq!(opus.efforts, vec!["low", "medium", "high"], "只收档位词");
        assert_eq!(opus.input_modalities, vec!["text", "image", "pdf"]);
        assert_eq!((opus.reasoning, opus.tool_call), (Some(true), Some(true)));
        let bare = &models["bare"];
        assert_eq!((bare.context_window, bare.reasoning, bare.efforts.len()), (0, None, 0), "没给的字段是未知，不是假");
    }

    #[test]
    fn the_builtin_snapshot_loads_and_covers_the_main_families() {
        let builtin = builtin();
        assert!(builtin.models.len() > 100, "内置表加载失败");
        assert!(builtin.models.values().any(|m| !m.efforts.is_empty()), "带档位数据");
        assert!(lookup_meta(builtin, "claude-opus-4-5").is_some_and(|m| m.context_window > 0));
    }

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
                                label: meta["name"].as_str().unwrap_or_default().to_string(),
                                description: String::new(),
                                ..Default::default()
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
            models: std::collections::BTreeMap::from([("glm-5.3".into(), ModelMeta { context_window: 200_000, max_output: 0, label: String::new(), description: String::new(), ..Default::default() })]),
        };
        std::fs::write(&path, serde_json::to_string(&catalog).unwrap()).unwrap();
        let loaded = read_cache(&path).unwrap();
        assert_eq!(loaded.models["glm-5.3"].context_window, 200_000);
        let _ = std::fs::remove_file(&path);
    }
}
