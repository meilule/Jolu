//! 模型价目目录组装与计费规则校验服务（零 Tauri 依赖，桌面端与 CLI 可共用）。

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;
use serde::Serialize;

use crate::pricing::store::{PricingAlias, PricingOverride, GLOBAL_SCOPE};

#[derive(Debug, Clone, Serialize)]
pub struct PricingEntryDto {
    pub model_id: String,
    pub display_name: String,
    pub vendor: String,
    /// `user_custom` | `builtin` | `models_dev` | `unpriced`
    pub source_kind: String,
    pub source: String,
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
    pub cache_write_1h: Option<f64>,
    pub long_ctx_threshold: Option<i64>,
    pub long_ctx_input: Option<f64>,
    pub long_ctx_output: Option<f64>,
    pub long_ctx_cache_read: Option<f64>,
    pub long_ctx_cache_write: Option<f64>,
    pub has_override: bool,
    pub disabled: bool,
    pub recent_requests: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelsDevPriceDto {
    pub model_id: String,
    pub display_name: String,
    pub vendor: String,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: Option<f64>,
    pub cache_write_1h: Option<f64>,
    pub long_ctx_threshold: Option<i64>,
    pub long_ctx_input: Option<f64>,
    pub long_ctx_output: Option<f64>,
    pub long_ctx_cache_read: Option<f64>,
    pub long_ctx_cache_write: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PricingCatalogView {
    pub entries: Vec<PricingEntryDto>,
    pub connection_overrides: Vec<PricingOverride>,
    pub aliases: Vec<PricingAlias>,
    pub models_dev_fetched_ms: i64,
    pub models_dev_count: usize,
    pub models_dev_auto_sync: bool,
    pub pricing_model_source: String,
    pub fx_mode: String,
    pub fx_fixed_usd_cny: Option<f64>,
}

fn load_observed_models(conn: &Connection) -> Result<HashMap<String, (String, i64)>, String> {
    let since_ms = chrono::Utc::now().timestamp_millis() - 90 * 24 * 60 * 60 * 1000;
    let mut observed: HashMap<String, (String, i64)> = HashMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT platform, COALESCE(NULLIF(TRIM(upstream_model), ''), NULLIF(TRIM(model), '')) AS m, COUNT(*)
             FROM requests
             WHERE ts >= ?1 AND COALESCE(NULLIF(TRIM(upstream_model), ''), NULLIF(TRIM(model), '')) IS NOT NULL
             GROUP BY platform, m",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([since_ms], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (platform, model, cnt) = row.map_err(|e| e.to_string())?;
        let key = model.trim().to_ascii_lowercase();
        let entry = observed.entry(key).or_insert((platform, 0));
        entry.1 += cnt;
    }
    Ok(observed)
}

fn populate_models_dev_entries<'a>(
    map: &mut BTreeMap<String, PricingEntryDto>,
    models: impl IntoIterator<Item = (&'a String, &'a crate::gateway::models_dev::ModelMeta)>,
    observed: &HashMap<String, (String, i64)>,
) {
    for (id, meta) in models {
        let Some(cost) = meta.cost.as_ref() else { continue };
        let key = id.to_ascii_lowercase();
        let tier = cost.tiers.first();
        let recent = observed.get(&key).map(|(_, c)| *c).unwrap_or(0);
        let is_claude = key.starts_with("claude-");
        map.insert(
            key.clone(),
            PricingEntryDto {
                model_id: key,
                display_name: if meta.label.is_empty() { id.clone() } else { meta.label.clone() },
                vendor: meta.vendor.clone(),
                source_kind: "models_dev".into(),
                source: if meta.vendor.is_empty() {
                    "models.dev".into()
                } else {
                    format!("models.dev/{}", meta.vendor)
                },
                input: Some(cost.input),
                output: Some(cost.output),
                cache_read: Some(cost.cache_read),
                cache_write: cost.cache_write.or_else(|| is_claude.then_some(cost.input * 1.25)),
                cache_write_1h: cost.cache_write_1h.or_else(|| is_claude.then_some(cost.input * 2.0)),
                long_ctx_threshold: tier.map(|t| t.above),
                long_ctx_input: tier.map(|t| t.input),
                long_ctx_output: tier.map(|t| t.output),
                long_ctx_cache_read: tier.map(|t| t.cache_read),
                long_ctx_cache_write: tier.and_then(|t| t.cache_write),
                has_override: false,
                disabled: false,
                recent_requests: recent,
            },
        );
    }
}

fn apply_builtin_entries(
    map: &mut BTreeMap<String, PricingEntryDto>,
    observed: &HashMap<String, (String, i64)>,
) {
    for (model_id, input, output, cache_read, cache_write, source, threshold) in crate::pricing::builtin_entries() {
        let key = model_id.to_ascii_lowercase();
        let existing = map.get(&key);
        let display_name = existing
            .map(|e| e.display_name.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| model_id.clone());
        let vendor = existing.map(|e| e.vendor.clone()).unwrap_or_default();
        let cache_write_1h = existing
            .and_then(|e| e.cache_write_1h)
            .or_else(|| key.starts_with("claude-").then_some(input * 2.0));
        let recent = observed.get(&key).map(|(_, c)| *c).unwrap_or(0);
        map.insert(
            key.clone(),
            PricingEntryDto {
                model_id: key,
                display_name,
                vendor,
                source_kind: "builtin".into(),
                source: source.to_string(),
                input: Some(input),
                output: Some(output),
                cache_read: Some(cache_read),
                cache_write,
                cache_write_1h,
                long_ctx_threshold: threshold.or_else(|| existing.and_then(|e| e.long_ctx_threshold)),
                long_ctx_input: existing.and_then(|e| e.long_ctx_input),
                long_ctx_output: existing.and_then(|e| e.long_ctx_output),
                long_ctx_cache_read: existing.and_then(|e| e.long_ctx_cache_read),
                long_ctx_cache_write: existing.and_then(|e| e.long_ctx_cache_write),
                has_override: false,
                disabled: false,
                recent_requests: recent,
            },
        );
    }
}

fn apply_global_overrides(
    map: &mut BTreeMap<String, PricingEntryDto>,
    global_overrides: HashMap<String, PricingOverride>,
    observed: &HashMap<String, (String, i64)>,
) {
    for (key, ov) in global_overrides {
        let existing = map.get(&key);
        let recent = observed.get(&key).map(|(_, c)| *c).unwrap_or(0);
        let vendor = existing.map(|e| e.vendor.clone()).unwrap_or_default();
        let display_name = if !ov.display_name.is_empty() {
            ov.display_name.clone()
        } else {
            existing.map(|e| e.display_name.clone()).unwrap_or_else(|| key.clone())
        };
        map.insert(
            key.clone(),
            PricingEntryDto {
                model_id: key,
                display_name,
                vendor,
                source_kind: if ov.disabled { "unpriced".into() } else { "user_custom".into() },
                source: if ov.disabled { "已停用".into() } else { "自定义价目".into() },
                input: (!ov.disabled).then_some(ov.input),
                output: (!ov.disabled).then_some(ov.output),
                cache_read: (!ov.disabled).then_some(ov.cache_read),
                cache_write: if ov.disabled { None } else { ov.cache_write },
                cache_write_1h: if ov.disabled { None } else { ov.cache_write_1h },
                long_ctx_threshold: if ov.disabled { None } else { ov.long_ctx_threshold },
                long_ctx_input: if ov.disabled { None } else { ov.long_ctx_input },
                long_ctx_output: if ov.disabled { None } else { ov.long_ctx_output },
                long_ctx_cache_read: if ov.disabled { None } else { ov.long_ctx_cache_read },
                long_ctx_cache_write: if ov.disabled { None } else { ov.long_ctx_cache_write },
                has_override: true,
                disabled: ov.disabled,
                recent_requests: recent,
            },
        );
    }
}

pub fn build_catalog_view(
    conn: &Connection,
    auto_sync: bool,
    model_source: String,
    fx_mode: String,
    fx_fixed_usd_cny: Option<f64>,
) -> Result<PricingCatalogView, String> {
    let overrides = crate::pricing::store::list_overrides(conn).map_err(|e| e.to_string())?;
    let aliases = crate::pricing::store::list_aliases(conn).map_err(|e| e.to_string())?;
    let md_catalog = crate::gateway::models_dev::active_catalog();
    let observed = load_observed_models(conn)?;

    let mut global_overrides: HashMap<String, PricingOverride> = HashMap::new();
    let mut connection_overrides = Vec::new();
    for item in overrides {
        if item.scope == GLOBAL_SCOPE {
            global_overrides.insert(item.model_id.clone(), item);
        } else {
            connection_overrides.push(item);
        }
    }

    let mut map: BTreeMap<String, PricingEntryDto> = BTreeMap::new();
    populate_models_dev_entries(&mut map, &md_catalog.models, &observed);
    apply_builtin_entries(&mut map, &observed);
    apply_global_overrides(&mut map, global_overrides, &observed);

    let mut resolver = crate::pricing::PriceResolver::load(conn);
    for (model_key, (platform, cnt)) in observed {
        if map.contains_key(&model_key) {
            continue;
        }
        if !resolver.has_price_for(None, &platform, &model_key) {
            map.insert(
                model_key.clone(),
                PricingEntryDto {
                    model_id: model_key.clone(),
                    display_name: model_key,
                    vendor: platform,
                    source_kind: "unpriced".into(),
                    source: "未收录价目".into(),
                    input: None,
                    output: None,
                    cache_read: None,
                    cache_write: None,
                    cache_write_1h: None,
                    long_ctx_threshold: None,
                    long_ctx_input: None,
                    long_ctx_output: None,
                    long_ctx_cache_read: None,
                    long_ctx_cache_write: None,
                    has_override: false,
                    disabled: false,
                    recent_requests: cnt,
                },
            );
        }
    }

    let mut entries: Vec<PricingEntryDto> = map.into_values().collect();
    entries.sort_by(|a, b| {
        let rank = |e: &PricingEntryDto| match (e.source_kind.as_str(), e.recent_requests > 0) {
            ("unpriced", true) => 0u8,
            ("user_custom", _) => 1,
            (_, true) => 2,
            ("builtin", _) => 3,
            ("models_dev", _) => 4,
            _ => 5,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| b.recent_requests.cmp(&a.recent_requests))
            .then_with(|| a.model_id.cmp(&b.model_id))
    });

    Ok(PricingCatalogView {
        entries,
        connection_overrides,
        aliases,
        models_dev_fetched_ms: md_catalog.fetched_ms,
        models_dev_count: md_catalog.models.values().filter(|m| m.cost.is_some()).count(),
        models_dev_auto_sync: auto_sync,
        pricing_model_source: model_source,
        fx_mode,
        fx_fixed_usd_cny,
    })
}

pub fn models_dev_catalog() -> Vec<ModelsDevPriceDto> {
    let md = crate::gateway::models_dev::active_catalog();
    let mut out: Vec<ModelsDevPriceDto> = md
        .models
        .iter()
        .filter_map(|(id, meta)| {
            let cost = meta.cost.as_ref()?;
            let tier = cost.tiers.first();
            Some(ModelsDevPriceDto {
                model_id: id.clone(),
                display_name: if meta.label.is_empty() { id.clone() } else { meta.label.clone() },
                vendor: meta.vendor.clone(),
                input: cost.input,
                output: cost.output,
                cache_read: cost.cache_read,
                cache_write: cost.cache_write,
                cache_write_1h: cost.cache_write_1h,
                long_ctx_threshold: tier.map(|t| t.above),
                long_ctx_input: tier.map(|t| t.input),
                long_ctx_output: tier.map(|t| t.output),
                long_ctx_cache_read: tier.map(|t| t.cache_read),
                long_ctx_cache_write: tier.and_then(|t| t.cache_write),
            })
        })
        .collect();
    out.sort_by(|a, b| a.vendor.cmp(&b.vendor).then_with(|| a.model_id.cmp(&b.model_id)));
    out
}

pub fn validate_pricing_rules(
    pricing_model_source: Option<&str>,
    fx_mode: Option<&str>,
    fx_fixed_usd_cny: Option<Option<f64>>,
) -> Result<(), String> {
    if let Some(src) = pricing_model_source {
        if !matches!(src, "upstream" | "request") {
            return Err("计价模型来源只支持 upstream 或 request".into());
        }
    }
    if let Some(mode) = fx_mode {
        if !matches!(mode, "auto" | "fixed") {
            return Err("汇率来源只支持 auto 或 fixed".into());
        }
    }
    if let Some(Some(rate)) = fx_fixed_usd_cny {
        if !rate.is_finite() || !(1.0..=20.0).contains(&rate) {
            return Err("固定汇率须在 1.0 到 20.0 之间".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpriced_observed_models_rank_first_and_user_overrides_win() {
        let dir = std::env::temp_dir().join(format!(
            "jolu-svc-pricing-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = crate::db::open(&dir.join("usage.db")).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO requests(platform, source, dedup_key, ts, model, total_tokens)
             VALUES('custom', 'local-session', 'u1', ?1, 'mystery-unpriced-v9', 100)",
            [now],
        )
        .unwrap();
        crate::pricing::store::upsert_override(
            &conn,
            PricingOverride {
                scope: GLOBAL_SCOPE.into(),
                model_id: "claude-sonnet-4".into(),
                display_name: "Custom Sonnet".into(),
                input: 9.9,
                output: 19.9,
                cache_read: 0.9,
                cache_write: None,
                cache_write_1h: None,
                long_ctx_threshold: None,
                long_ctx_input: None,
                long_ctx_output: None,
                long_ctx_cache_read: None,
                long_ctx_cache_write: None,
                disabled: false,
                updated_ms: now,
            },
        )
        .unwrap();

        let view = build_catalog_view(&conn, true, "upstream".into(), "auto".into(), None).unwrap();
        assert_eq!(view.entries.first().map(|e| e.model_id.as_str()), Some("mystery-unpriced-v9"));
        assert_eq!(view.entries[0].source_kind, "unpriced");
        let custom = view.entries.iter().find(|e| e.model_id == "claude-sonnet-4").unwrap();
        assert_eq!(custom.source_kind, "user_custom");
        assert_eq!(custom.input, Some(9.9));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pricing_rule_validation_rejects_invalid_modes_and_rates() {
        assert!(validate_pricing_rules(Some("upstream"), Some("auto"), Some(Some(7.2))).is_ok());
        assert!(validate_pricing_rules(Some("bogus"), None, None).is_err());
        assert!(validate_pricing_rules(None, Some("bogus"), None).is_err());
        assert!(validate_pricing_rules(None, None, Some(Some(0.5))).is_err());
        assert!(validate_pricing_rules(None, None, Some(Some(f64::NAN))).is_err());
    }
}
