//! 模型价目、同价映射、计费规则与连接计费配置的 Tauri 命令。

#[allow(unused_imports)]
use crate::*;
use crate::pricing::store::{PricingAlias, PricingOverride, GLOBAL_SCOPE};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

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

pub(crate) fn build_catalog_view(
    conn: &rusqlite::Connection,
    auto_sync: bool,
    model_source: String,
    fx_mode: String,
    fx_fixed_usd_cny: Option<f64>,
) -> Result<PricingCatalogView, String> {
    let overrides = crate::pricing::store::list_overrides(conn).map_err(|e| e.to_string())?;
    let aliases = crate::pricing::store::list_aliases(conn).map_err(|e| e.to_string())?;
    let md_catalog = crate::gateway::models_dev::active_catalog();

    // 统计近 90 天内真实出现过的模型及调用次数，便于在界面把「待补价」与高频模型排在前面
    let since_ms = chrono::Utc::now().timestamp_millis() - 90 * 24 * 60 * 60 * 1000;
    let mut observed: HashMap<String, (String, i64)> = HashMap::new();
    {
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
    }

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

    // 1) models.dev 价格基座
    for (id, meta) in &md_catalog.models {
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
                source: if meta.vendor.is_empty() { "models.dev".into() } else { format!("models.dev/{}", meta.vendor) },
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

    // 2) 内置核验规则表（优先级高于 models.dev）
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

    // 3) 用户全局覆盖（最高优先级，含删除墓碑）
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

    // 4) 将近期真实出现但当前仍无价的模型暴露到表里，供「待补价」筛选一键处理
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
    // 排序：待补价且有调用 > 自定义 > 高频 > 其余按模型名
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

#[tauri::command]
pub(crate) async fn pricing_catalog(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
) -> Result<PricingCatalogView, String> {
    let settings = app.state::<Cfg>().0.get();
    let auto_sync = settings.models_dev_auto_sync;
    let model_source = settings.pricing_model_source;
    let fx_mode = settings.fx_mode;
    let fx_fixed = settings.fx_fixed_usd_cny;
    with_reader(db.0.clone(), move |conn| build_catalog_view(conn, auto_sync, model_source, fx_mode, fx_fixed)).await
}

#[tauri::command]
pub(crate) fn pricing_models_dev_catalog() -> Vec<ModelsDevPriceDto> {
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

#[tauri::command]
pub(crate) async fn pricing_upsert_override(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    item: PricingOverride,
) -> Result<PricingOverride, String> {
    let saved = with_database(db.0.clone(), move |conn| {
        crate::pricing::store::upsert_override(conn, item)
    })
    .await?;
    crate::events::pricing_changed(&app);
    Ok(saved)
}

#[tauri::command]
pub(crate) async fn pricing_delete_override(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    scope: String,
    model_id: String,
) -> Result<(), String> {
    with_database(db.0.clone(), move |conn| {
        crate::pricing::store::delete_override(conn, &scope, &model_id)
    })
    .await?;
    crate::events::pricing_changed(&app);
    Ok(())
}

#[tauri::command]
pub(crate) async fn pricing_disable_override(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    scope: String,
    model_id: String,
) -> Result<(), String> {
    with_database(db.0.clone(), move |conn| {
        crate::pricing::store::disable_override(conn, &scope, &model_id)
    })
    .await?;
    crate::events::pricing_changed(&app);
    Ok(())
}

#[tauri::command]
pub(crate) async fn pricing_upsert_alias(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    scope: String,
    alias_model: String,
    target_model: String,
) -> Result<PricingAlias, String> {
    let saved = with_database(db.0.clone(), move |conn| {
        crate::pricing::store::upsert_alias(conn, &scope, &alias_model, &target_model)
    })
    .await?;
    crate::events::pricing_changed(&app);
    Ok(saved)
}

#[tauri::command]
pub(crate) async fn pricing_delete_alias(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    scope: String,
    alias_model: String,
) -> Result<(), String> {
    with_database(db.0.clone(), move |conn| {
        crate::pricing::store::delete_alias(conn, &scope, &alias_model)
    })
    .await?;
    crate::events::pricing_changed(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn pricing_set_rules(
    app: tauri::AppHandle,
    pricing_model_source: Option<String>,
    models_dev_auto_sync: Option<bool>,
    fx_mode: Option<String>,
    fx_fixed_usd_cny: Option<Option<f64>>,
) -> Result<crate::settings::Settings, String> {
    if let Some(ref src) = pricing_model_source {
        if !matches!(src.as_str(), "upstream" | "request") {
            return Err("计价模型来源只支持 upstream 或 request".into());
        }
    }
    if let Some(ref mode) = fx_mode {
        if !matches!(mode.as_str(), "auto" | "fixed") {
            return Err("汇率来源只支持 auto 或 fixed".into());
        }
    }
    if let Some(Some(rate)) = fx_fixed_usd_cny {
        if !rate.is_finite() || !(1.0..=20.0).contains(&rate) {
            return Err("固定汇率须在 1.0 到 20.0 之间".into());
        }
    }
    let next = app.state::<Cfg>().0.try_update(|settings| {
        if let Some(ref src) = pricing_model_source {
            settings.pricing_model_source = src.clone();
        }
        if let Some(auto) = models_dev_auto_sync {
            settings.models_dev_auto_sync = auto;
        }
        if let Some(ref mode) = fx_mode {
            settings.fx_mode = mode.clone();
        }
        if let Some(rate) = fx_fixed_usd_cny {
            settings.fx_fixed_usd_cny = rate;
        }
    })?;
    crate::pricing::set_pricing_model_source(&next.pricing_model_source);
    crate::events::settings_changed(&app);
    crate::events::pricing_changed(&app);
    Ok(next)
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SaveBudgetsInput {
    pub global_enabled: bool,
    pub global_daily_limit: Option<f64>,
    pub global_monthly_limit: Option<f64>,
    pub warn_percent: u8,
    pub connections: Vec<crate::pricing::budget::ConnectionBudgetInput>,
}

#[tauri::command]
pub(crate) async fn pricing_budget_status(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
) -> Result<crate::pricing::budget::BudgetStatus, String> {
    let settings = app.state::<Cfg>().0.get();
    let now = chrono::Utc::now().timestamp_millis();
    let status = with_reader(db.0.clone(), move |conn| {
        crate::pricing::budget::compute_budget_status(conn, &settings, now)
    })
    .await?;
    crate::notify::check_budgets(&app, &status);
    Ok(status)
}

#[tauri::command]
pub(crate) async fn pricing_save_budgets(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    input: SaveBudgetsInput,
) -> Result<crate::pricing::budget::BudgetStatus, String> {
    let daily = crate::pricing::budget::validate_optional_usd("全局日上限", input.global_daily_limit)?;
    let monthly = crate::pricing::budget::validate_optional_usd("全局月上限", input.global_monthly_limit)?;
    let warn = input.warn_percent.clamp(50, 99);
    let conns = input.connections;
    let enabled = input.global_enabled;

    let next_settings = app.state::<Cfg>().0.try_update(|settings| {
        settings.global_budget_enabled = enabled;
        settings.global_budget_daily_usd = daily;
        settings.global_budget_monthly_usd = monthly;
        settings.global_budget_warn_percent = warn;
    })?;
    let settings_for_status = next_settings.clone();
    let now = chrono::Utc::now().timestamp_millis();
    let status = with_database(db.0.clone(), move |conn| {
        crate::pricing::budget::save_connection_budgets(conn, &conns)?;
        crate::pricing::budget::compute_budget_status(conn, &settings_for_status, now)
    })
    .await?;
    crate::events::settings_changed(&app);
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    crate::events::pricing_changed(&app);
    crate::notify::check_budgets(&app, &status);
    Ok(status)
}

#[tauri::command]
pub(crate) async fn pricing_refresh_models_dev(app: tauri::AppHandle) -> Result<(usize, i64), String> {
    let handle = app.clone();
    let res = tauri::async_runtime::spawn_blocking(move || crate::gateway::models_dev::refresh_now(&handle))
        .await
        .map_err(|e| e.to_string())??;
    crate::events::pricing_changed(&app);
    Ok(res)
}

#[tauri::command]
pub(crate) async fn connection_update_billing(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    id: String,
    billing_mode: String,
    price_multiplier: f64,
) -> Result<(), String> {
    with_database(db.0.clone(), move |conn| {
        crate::connections::set_billing(conn, &id, &billing_mode, price_multiplier)
    })
    .await?;
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    crate::events::pricing_changed(&app);
    Ok(())
}
