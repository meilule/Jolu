//! 网关对外访问、命名客户端 Key、IP 白名单、请求中间件与模型元数据管理命令。

use tauri::Manager;

use crate::app::state::{Cfg, Db};
use crate::{connections, service, settings, zcode_version};

#[tauri::command]
pub(crate) fn gateway_access_key(app: tauri::AppHandle, refresh: bool) -> Result<String, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    crate::gateway::access::masked(&conn, refresh)
}

#[tauri::command]
pub(crate) fn gateway_copy_key(app: tauri::AppHandle) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    crate::gateway::access::copy(&conn)
}

/// 开关局域网共享（P8.3）。
#[tauri::command]
pub(crate) fn gateway_shared_lan_set(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| {
            settings.gateway_shared_lan = on;
        })
        .map_err(|error| format!("保存局域网共享设置失败：{error}"))?;
    crate::events::settings_changed(&app);
    Ok(next)
}

/// 本机的局域网地址（画板 V5-4「地址」行）。
#[tauri::command]
pub(crate) fn gateway_lan_address() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then(|| ip.to_string())
}

/// 局域网共享的来源地址白名单（P2-9）。
#[tauri::command]
pub(crate) fn gateway_allow_ips_set(
    app: tauri::AppHandle,
    list: Vec<String>,
) -> Result<settings::Settings, String> {
    let list: Vec<String> = list
        .iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect();
    crate::gateway::allow::parse(&list.join(","))?;
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| settings.gateway_allow_ips = list.clone())
        .map_err(|error| format!("保存白名单失败：{error}"))?;
    crate::gateway::allow::load(&next.gateway_allow_ips)?;
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) fn gateway_allow_ips_from_env() -> bool {
    crate::gateway::allow::from_env()
}

/// 列出命名客户端 Key：名称 + 掩码 + 限额 + 最近使用与当日计数。
#[tauri::command]
pub(crate) fn gateway_client_keys_list(
    app: tauri::AppHandle,
) -> Result<Vec<crate::gateway::access::ClientKeySummary>, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    service::gateway::client_keys::list(&conn)
}

#[tauri::command]
pub(crate) fn gateway_client_key_issue(
    app: tauri::AppHandle,
    name: String,
    tokens_per_day: Option<i64>,
    requests_per_day: Option<i64>,
    expires_ms: Option<i64>,
    models: Option<Vec<String>>,
    read_only: Option<bool>,
) -> Result<crate::gateway::access::IssuedClientKey, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    service::gateway::client_keys::issue(
        &conn,
        &name,
        tokens_per_day,
        requests_per_day,
        expires_ms,
        models,
        read_only.unwrap_or(false),
    )
}

#[tauri::command]
pub(crate) fn gateway_client_key_revoke(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    service::gateway::client_keys::revoke(&conn, &name)
}

#[tauri::command]
pub(crate) fn gateway_client_key_concurrency_set(
    app: tauri::AppHandle,
    name: String,
    cap: Option<i64>,
) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::access::client_key_set_concurrency(&conn, &name, cap)
}

#[tauri::command]
pub(crate) fn gateway_client_key_limits_set(
    app: tauri::AppHandle,
    name: String,
    budget: crate::gateway::budget::KeyBudget,
    accounts: Option<Vec<String>>,
) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::access::client_key_set_limits(&conn, &name, &budget, accounts)
}

#[tauri::command]
pub(crate) fn gateway_middleware_list(
    app: tauri::AppHandle,
) -> Result<Vec<crate::gateway::middleware::Middleware>, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    service::gateway::middleware::list(&conn)
}

#[tauri::command]
pub(crate) fn gateway_middleware_save(
    app: tauri::AppHandle,
    name: String,
    kind: String,
    config: serde_json::Value,
    enabled: bool,
    sort: i64,
) -> Result<crate::gateway::middleware::Middleware, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法写入网关状态")?;
    service::gateway::middleware::save(&conn, &name, &kind, config, enabled, sort)
}

#[tauri::command]
pub(crate) fn gateway_middleware_delete(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法写入网关状态")?;
    service::gateway::middleware::delete(&conn, &name)
}

#[tauri::command]
pub(crate) async fn gateway_models_dev_refresh(app: tauri::AppHandle) -> Result<(usize, i64), String> {
    tauri::async_runtime::spawn_blocking(move || crate::gateway::models_dev::refresh_now(&app))
        .await
        .map_err(|error| format!("刷新任务失败：{error}"))?
}

#[tauri::command]
pub(crate) fn gateway_models_dev_status(app: tauri::AppHandle) -> (i64, usize) {
    crate::gateway::models_dev::cached(&app)
        .map(|c| (c.fetched_ms, c.models.len()))
        .unwrap_or((0, 0))
}

#[tauri::command]
pub(crate) async fn zcode_version_check() -> Result<zcode_version::VersionCheck, String> {
    tauri::async_runtime::spawn_blocking(|| zcode_version::check(zcode_version::installed_version()))
        .await
        .map_err(|_| "无法读取 ZCode 版本".to_string())
}

#[tauri::command]
pub(crate) fn zcode_free_first_set(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| settings.zcode_free_first = on)
        .map_err(|error| format!("保存 ZCode Start Plan 优先设置失败：{error}"))?;
    crate::gateway::adapter::zcode::set_free_first(next.zcode_free_first);
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) fn zcode_set_channel(
    app: tauri::AppHandle,
    connection_id: String,
    channel: Option<String>,
) -> Result<(), String> {
    {
        let state = app.state::<Db>();
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        connections::set_start_channel(&conn, &connection_id, channel.as_deref())?;
    }
    crate::gateway::adapter::zcode::clear_channel_memory(&connection_id);
    Ok(())
}
