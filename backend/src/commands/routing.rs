//! 模型路由：网关状态、服务池、分组、预设、流水与上下文观察。

use tauri::{Emitter, Manager, State};

use crate::app::state::{with_reader, Cfg, Db, DbRead};
use crate::{connections, service, settings};

pub(crate) type GatewayStatus = service::gateway::status::GatewayStatus;

#[tauri::command]
pub(crate) fn gateway_set_default_source(
    app: tauri::AppHandle,
    connection_id: Option<String>,
) -> Result<Option<String>, String> {
    if let Some(id) = connection_id.as_deref() {
        let state = app.state::<Db>();
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        service::gateway::status::validate_default_source(&conn, id)?;
    }
    app.state::<Cfg>()
        .0
        .try_update(|settings| {
            settings.gateway_default_connection = connection_id.clone();
        })
        .map_err(|error| format!("保存默认来源失败：{error}"))?;
    crate::events::settings_changed(&app);
    Ok(connection_id)
}

#[tauri::command]
pub(crate) fn gateway_status(app: tauri::AppHandle) -> GatewayStatus {
    let settings = app.state::<Cfg>().0.get();
    let db = app.state::<Db>();
    let guard = db.0.lock();
    match guard {
        Ok(conn) => service::gateway::status::status(&settings, &conn),
        Err(_) => service::gateway::status::status_without_db(&settings),
    }
}

#[tauri::command]
pub(crate) fn gateway_set_mode(
    app: tauri::AppHandle,
    mode: String,
    default_connection: Option<String>,
) -> Result<settings::Settings, String> {
    if !matches!(mode.as_str(), "observe" | "relay") {
        return Err("模式只能是 observe 或 relay".into());
    }
    let settings = app.state::<Cfg>().0.get();
    if mode == "observe" && settings.proxy_mode == "relay" {
        crate::gateway::escape::manual(&app);
    }
    if mode == "relay" && settings.proxy_mode != "relay" {
        crate::cli_switch::with_switch(|| {
            crate::proxy::stop();
            let _ = crate::proxy_config::restore_all_saved(&app);
            Ok::<(), String>(())
        })?;
    }
    let previous_mode = settings.proxy_mode.clone();
    let previous_enabled = settings.proxy_enabled;
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| {
            settings.proxy_mode = mode.clone();
            settings.gateway_default_connection = default_connection;
            if mode == "relay" {
                settings.proxy_enabled = true;
            }
        })
        .map_err(|error| format!("保存网关设置失败：{error}"))?;
    if mode == "relay" {
        if let Err(error) = crate::proxy::start(&app) {
            app.state::<Cfg>().0.update(|settings| {
                settings.proxy_mode = previous_mode;
                settings.proxy_enabled = previous_enabled;
            });
            crate::events::settings_changed(&app);
            if previous_enabled {
                let _ = crate::proxy::set_proxy(&app, settings.proxy_port, true, settings.proxy_fallback_direct);
            }
            return Err(format!("启动模型入口失败：{error}"));
        }
    }
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) fn gateway_set_serve(
    app: tauri::AppHandle,
    connection_id: String,
    enabled: bool,
) -> Result<(), String> {
    let state = app.state::<Db>();
    let db = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    connections::set_serve(&db, &connection_id, enabled)?;
    drop(db);
    crate::events::catalog_changed(&app);
    Ok(())
}

#[derive(serde::Serialize)]
pub(crate) struct ServeBatchItem {
    connection_id: String,
    ok: bool,
    error: Option<String>,
}

#[tauri::command]
pub(crate) fn gateway_set_serve_many(
    app: tauri::AppHandle,
    connection_ids: Vec<String>,
    enabled: bool,
) -> Result<Vec<ServeBatchItem>, String> {
    let state = app.state::<Db>();
    let items: Vec<ServeBatchItem> = {
        let db = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        connection_ids
            .into_iter()
            .map(|id| match connections::set_serve(&db, &id, enabled) {
                Ok(()) => ServeBatchItem { connection_id: id, ok: true, error: None },
                Err(error) => ServeBatchItem { connection_id: id, ok: false, error: Some(error) },
            })
            .collect()
    };
    if items.iter().any(|item| item.ok) {
        crate::events::catalog_changed(&app);
    }
    Ok(items)
}

#[tauri::command]
pub(crate) fn gateway_source_usage(
    app: tauri::AppHandle,
) -> Result<Vec<crate::gateway::route_log::SourceUsage>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::route_log::source_usage(&conn).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn gateway_source_capabilities(
    app: tauri::AppHandle,
) -> Result<Vec<crate::gateway::subscription::Capability>, String> {
    let state = app.state::<Db>();
    let db = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::subscription::capabilities(&db)
}

#[tauri::command]
pub(crate) fn gateway_groups(app: tauri::AppHandle) -> Result<crate::gateway::groups::GroupsDto, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::list(&conn)
}

#[tauri::command]
pub(crate) fn gateway_group_create(
    app: tauri::AppHandle,
    kind: String,
    name: String,
) -> Result<crate::gateway::groups::GroupDto, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::create(&conn, &kind, &name)
}

#[tauri::command]
pub(crate) fn gateway_group_rename(
    app: tauri::AppHandle,
    kind: String,
    id: String,
    name: String,
) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::rename(&conn, &kind, &id, &name)
}

#[tauri::command]
pub(crate) fn gateway_group_delete(app: tauri::AppHandle, kind: String, id: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::delete(&conn, &kind, &id)
}

#[tauri::command]
pub(crate) fn gateway_group_reorder(
    app: tauri::AppHandle,
    kind: String,
    ids: Vec<String>,
) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::reorder(&conn, &kind, &ids)
}

#[tauri::command]
pub(crate) fn gateway_group_move(
    app: tauri::AppHandle,
    kind: String,
    member: String,
    group_id: Option<String>,
) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::move_member(&conn, &kind, &member, group_id.as_deref())
}

#[tauri::command]
pub(crate) fn gateway_set_slug(
    app: tauri::AppHandle,
    connection_id: String,
    slug: String,
) -> Result<String, String> {
    let state = app.state::<Db>();
    let (old, new, record) = {
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        let old: Option<String> = conn
            .query_row("SELECT slug FROM connections WHERE id = ?1", [&connection_id], |r| r.get(0))
            .ok()
            .flatten();
        let new = connections::set_slug(&conn, &connection_id, &slug)?;
        (old, new, crate::gateway::wiring_store::load(&conn, "claude-code"))
    };
    crate::events::catalog_changed(&app);
    if let (Some(old), Some(record)) = (old.filter(|o| *o != new), record.filter(|r| r.state == "wired")) {
        if let Some((model, subagent, tiers)) = record
            .applied()
            .and_then(|applied| crate::gateway::wiring_store::renamed_slug(&applied, &old, &new))
        {
            let model = model.map(|m| m.trim_end_matches("[1m]").trim_end_matches("[1M]").to_string());
            if let Err(error) = crate::gateway::escape::wire_agents(
                &app,
                &["claude-code".to_string()],
                model.as_deref(),
                subagent.as_deref(),
                tiers.as_ref(),
            ) {
                eprintln!("[接线] 来源改 id 后重写 Claude Code 接线失败：{error}");
                return Err(format!(
                    "菜单 id 已改为「{new}」，但 Claude Code 的接线没能同步更新：{error}。请在接线面板重新接线"
                ));
            }
            let _ = app.emit(crate::events::GATEWAY_WIRING_CHANGED, ());
        }
    }
    Ok(new)
}

#[tauri::command]
pub(crate) fn gateway_catalog(
    app: tauri::AppHandle,
) -> Result<Vec<crate::gateway::catalog::CatalogModel>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::models_dev::refresh_if_stale(&app);
    let meta_catalog = crate::gateway::models_dev::cached(&app);
    service::gateway::catalog::list(&conn, &|model: &str| {
        meta_catalog
            .as_ref()
            .and_then(|catalog| crate::gateway::models_dev::lookup_meta(catalog, model))
    })
}

#[tauri::command]
pub(crate) fn gateway_profiles_list(
    app: tauri::AppHandle,
) -> Result<Vec<crate::gateway::profiles::Profile>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::profiles::list(&conn).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn gateway_profiles_save(
    app: tauri::AppHandle,
    name: String,
) -> Result<crate::gateway::profiles::Profile, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let agents = crate::gateway::profiles::capture_current(&conn)?;
    let default_connection = app.state::<Cfg>().0.get().gateway_default_connection;
    let extras = crate::gateway::profiles::capture_extras(
        &conn,
        default_connection,
        chrono::Utc::now().timestamp_millis(),
    );
    crate::gateway::profiles::save_with(&conn, &name, &agents, &extras)
}

#[tauri::command]
pub(crate) fn gateway_profiles_delete(app: tauri::AppHandle, name: String) -> Result<bool, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::profiles::delete(&conn, &name)
}

#[tauri::command]
pub(crate) fn gateway_profiles_apply(
    app: tauri::AppHandle,
    name: String,
) -> Result<crate::gateway::profiles::ApplyResult, String> {
    let state = app.state::<Db>();
    let profile = {
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        crate::gateway::profiles::get(&conn, &name)?.ok_or_else(|| format!("没有名为「{name}」的 Profile"))?
    };
    let wired = {
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        crate::gateway::profiles::capture_current(&conn)?
    };
    let mut switched = Vec::new();
    let mut skipped = Vec::new();
    for (agent, model) in &profile.agents {
        if !wired.contains_key(agent) {
            skipped.push(agent.clone());
            continue;
        }
        let current = {
            let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
            crate::gateway::wiring_store::load(&conn, agent).and_then(|record| record.applied())
        };
        let saved = profile.extras.agents.get(agent);
        let tiers = saved
            .and_then(|s| s.tiers.clone())
            .or_else(|| current.as_ref().and_then(|a| a.tiers.clone()));
        let subagent = saved
            .map(|s| s.subagent_model.clone())
            .unwrap_or_else(|| current.as_ref().and_then(|a| a.subagent_model.clone()));
        match crate::gateway::escape::wire_agents(
            &app,
            std::slice::from_ref(agent),
            Some(model),
            subagent.as_deref(),
            tiers.as_ref(),
        ) {
            Ok(_) => switched.push(format!("{agent} → {model}")),
            Err(error) => skipped.push(format!("{agent}（{error}）")),
        }
    }
    if switched.is_empty() {
        return Err(format!(
            "没有可切换的 agent：{}",
            if skipped.is_empty() {
                "Profile 里没有已接线的成员".into()
            } else {
                format!("跳过：{}", skipped.join("、"))
            }
        ));
    }
    if let Some(default_connection) = profile.extras.default_connection.clone() {
        let _ = app
            .state::<Cfg>()
            .0
            .try_update(|settings| settings.gateway_default_connection = Some(default_connection));
    }
    {
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        crate::gateway::profiles::set_active(&conn, Some(&profile.name));
    }
    let mut message = format!("已切换 {}", switched.join("、"));
    if !skipped.is_empty() {
        message.push_str(&format!("；跳过 {}", skipped.join("、")));
    }
    Ok(crate::gateway::profiles::ApplyResult { switched, skipped, message })
}

#[tauri::command]
pub(crate) fn gateway_routing_group_list(
    app: tauri::AppHandle,
) -> Result<Vec<crate::gateway::routing_groups::RoutingGroup>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::routing_groups::list(&conn).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn gateway_routing_group_create(
    app: tauri::AppHandle,
    slug: String,
    name: String,
    strategy: String,
    members: Vec<crate::gateway::routing_groups::GroupMember>,
    intent: Option<crate::gateway::routing_groups::IntentConfig>,
) -> Result<crate::gateway::routing_groups::RoutingGroup, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let group = crate::gateway::routing_groups::create(&conn, &slug, &name, &strategy, &members, intent.as_ref())?;
    crate::events::catalog_changed(&app);
    Ok(group)
}

#[tauri::command]
pub(crate) fn gateway_routing_group_update(
    app: tauri::AppHandle,
    slug: String,
    name: Option<String>,
    strategy: Option<String>,
    members: Option<Vec<crate::gateway::routing_groups::GroupMember>>,
    intent: Option<crate::gateway::routing_groups::IntentConfig>,
) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::routing_groups::update(
        &conn,
        &slug,
        name.as_deref(),
        strategy.as_deref(),
        members.as_deref(),
        intent.as_ref(),
    )?;
    crate::events::catalog_changed(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn gateway_routing_group_delete(app: tauri::AppHandle, slug: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::routing_groups::delete(&conn, &slug)?;
    crate::events::catalog_changed(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn gateway_route_log(
    app: tauri::AppHandle,
) -> Result<Vec<crate::gateway::route_log::RouteLogEntry>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::route_log::recent(&conn, 20).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn gateway_route_log_page(
    app: tauri::AppHandle,
    page: i64,
    page_size: i64,
    upper_id: Option<i64>,
) -> Result<crate::gateway::route_log::RouteLogPage, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::route_log::page(&conn, page, page_size, upper_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn gateway_sessions_overview(
    app: tauri::AppHandle,
) -> Result<crate::gateway::sessions::SessionsOverview, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::sessions::overview(&conn).map_err(|error| error.to_string())
}

#[derive(serde::Serialize)]
pub(crate) struct GatewayTodayCounts {
    pub(crate) since: i64,
    pub(crate) at: i64,
    pub(crate) ok: i64,
    pub(crate) rejected: i64,
    pub(crate) failed: i64,
    pub(crate) partial: bool,
}

#[tauri::command]
pub(crate) fn gateway_today_counts(app: tauri::AppHandle) -> Result<GatewayTodayCounts, String> {
    let since = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|midnight| midnight.and_local_timezone(chrono::Local).earliest())
        .map(|midnight| midnight.timestamp_millis())
        .unwrap_or(0);
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let (ok, rejected, failed) = crate::gateway::route_log::counts_since(&conn, since).map_err(|e| e.to_string())?;
    let partial = crate::gateway::route_log::truncated_since(&conn, since).map_err(|e| e.to_string())?;
    let at = chrono::Local::now().timestamp_millis();
    Ok(GatewayTodayCounts { since, at, ok, rejected, failed, partial })
}

#[tauri::command]
pub(crate) fn gateway_day_overview(
    app: tauri::AppHandle,
    range: Option<String>,
) -> Result<crate::gateway::route_log::DayOverview, String> {
    let today = chrono::Local::now().date_naive();
    let range = crate::gateway::route_log::Range::parse(range.as_deref());
    let (current_start, prior_start) = range.bounds(today);
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let prior_end = range.prior_end(today, chrono::Local::now().timestamp_millis());
    crate::gateway::route_log::day_overview_until(
        &conn,
        current_start,
        prior_start,
        prior_end,
        range.end_exclusive(today),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn gateway_context_sessions(
    db: State<'_, DbRead>,
) -> Result<Vec<crate::gateway::context::query::SessionRow>, String> {
    with_reader(db.0.clone(), |conn| crate::gateway::context::query::sessions(conn).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub(crate) async fn gateway_context_snapshot(
    db: State<'_, DbRead>,
    platform: String,
    session_id: String,
    snapshot_id: Option<i64>,
) -> Result<Option<crate::gateway::context::query::SnapshotDetail>, String> {
    with_reader(db.0.clone(), move |conn| {
        crate::gateway::context::query::snapshot(conn, &platform, &session_id, snapshot_id).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub(crate) async fn gateway_context_items(
    db: State<'_, DbRead>,
    snapshot_id: i64,
    kind: Option<crate::gateway::context::Kind>,
    search: Option<String>,
) -> Result<Option<Vec<crate::gateway::context::query::ItemRow>>, String> {
    with_reader(db.0.clone(), move |conn| {
        crate::gateway::context::query::items(conn, snapshot_id, kind, search.as_deref()).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub(crate) async fn gateway_context_tool_items(
    db: State<'_, DbRead>,
    snapshot_id: i64,
    tool: Option<String>,
) -> Result<Option<Vec<crate::gateway::context::ToolResult>>, String> {
    with_reader(db.0.clone(), move |conn| {
        crate::gateway::context::query::tool_items(conn, snapshot_id, tool.as_deref()).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub(crate) async fn gateway_context_history(
    db: State<'_, DbRead>,
    platform: String,
    session_id: String,
) -> Result<Vec<crate::gateway::context::query::SnapshotSummary>, String> {
    with_reader(db.0.clone(), move |conn| {
        crate::gateway::context::query::history(conn, &platform, &session_id).map_err(|e| e.to_string())
    })
    .await
}

/// 5.5：执行命令行代办请求（仅限 `cli_requests::ACTIONS` 白名单动作）。
pub(crate) fn handle_cli_request(
    app: &tauri::AppHandle,
    request: &crate::cli_requests::Request,
) -> crate::cli_requests::Response {
    let reply = |result: Result<String, String>| match result {
        Ok(message) => crate::cli_requests::Response { ok: true, message },
        Err(message) => crate::cli_requests::Response { ok: false, message },
    };
    let name = request.name.clone().unwrap_or_default();
    match request.action.as_str() {
        "profile_use" => reply(
            gateway_profiles_apply(app.clone(), name).map(|result| result.message),
        ),
        "profile_save" => reply(gateway_profiles_save(app.clone(), name).map(|profile| {
            format!("已保存 Profile「{}」：{} 个工具", profile.name, profile.agents.len())
        })),
        "serve" => {
            let settings = app.state::<Cfg>().0.get();
            let (mode, _) = crate::gateway::mode_from(&settings);
            if settings.proxy_enabled && mode == crate::gateway::Mode::Relay {
                return crate::cli_requests::Response {
                    ok: true,
                    message: format!("模型路由已在运行：http://127.0.0.1:{}", settings.proxy_port),
                };
            }
            let result = gateway_set_mode(
                app.clone(),
                "relay".into(),
                settings.gateway_default_connection.clone(),
            )
            .map(|next| format!("已开启模型路由：http://127.0.0.1:{}", next.proxy_port));
            crate::events::settings_changed(app);
            reply(result)
        }
        other => crate::cli_requests::Response {
            ok: false,
            message: format!("不支持的请求：{other}"),
        },
    }
}
