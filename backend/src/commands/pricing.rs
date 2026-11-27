//! 模型价目、同价映射、计费规则与连接计费配置的 Tauri 命令薄壳。

use tauri::{Emitter, Manager, State};

use crate::app::state::{with_database, with_reader, Cfg, Db, DbRead};
use crate::pricing::store::{PricingAlias, PricingOverride};
use crate::service::pricing::{ModelsDevPriceDto, PricingCatalogView};

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
    with_reader(db.0.clone(), move |conn| {
        crate::service::pricing::build_catalog_view(conn, auto_sync, model_source, fx_mode, fx_fixed)
    })
    .await
}

#[tauri::command]
pub(crate) fn pricing_models_dev_catalog() -> Vec<ModelsDevPriceDto> {
    crate::service::pricing::models_dev_catalog()
}

#[tauri::command]
pub(crate) async fn pricing_upsert_override(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    item: PricingOverride,
) -> Result<PricingOverride, String> {
    let saved = with_database(db.0.clone(), move |conn| crate::pricing::store::upsert_override(conn, item)).await?;
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
    with_database(db.0.clone(), move |conn| crate::pricing::store::delete_override(conn, &scope, &model_id)).await?;
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
    with_database(db.0.clone(), move |conn| crate::pricing::store::disable_override(conn, &scope, &model_id)).await?;
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
    with_database(db.0.clone(), move |conn| crate::pricing::store::delete_alias(conn, &scope, &alias_model)).await?;
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
    crate::service::pricing::validate_pricing_rules(
        pricing_model_source.as_deref(),
        fx_mode.as_deref(),
        fx_fixed_usd_cny,
    )?;
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
