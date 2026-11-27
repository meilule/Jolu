//! 灵动岛窗口命令层：转发至 `app::island` 与 `app::tray`。

use std::sync::atomic::Ordering;

use tauri::{Emitter, Manager, State, WebviewWindow};

use crate::app::island::{
    apply_island_platform, apply_island_position, apply_island_topmost, dock_release_for,
    dock_undock_for, island_dock, island_id_of, island_windows, remember_clone_position,
    set_island_visible, DockHintEvent,
};
use crate::app::state::{
    invalidate_network_cache, local_source, with_database, with_reader, Cfg, Db, DbRead,
    FIRST_INSTALL_INTRO, MAIN,
};
use crate::{
    agent_hooks, connections, context_menu, dock, handle_tray_menu, island_clones, island_pointer,
    local_active, local_enroll, native_sources, refresh_tray_menu, settings,
};

#[tauri::command]
pub(crate) fn resize_island(
    app: tauri::AppHandle,
    window: WebviewWindow,
    width: f64,
    height: f64,
) -> Result<Option<f64>, String> {
    let id = island_id_of(&window)?;
    let w = window;
    if !width.is_finite() || !height.is_finite() || width < 8.0 || height < 8.0 {
        return Err("灵动岛尺寸无效".into());
    }
    let cfg = app.state::<Cfg>().0.get();
    if cfg.island_style == "capsule" {
        return Ok(dock::work_area(&w).map(|area| area.3));
    }
    if id.is_none() && height > 150.0 {
        FIRST_INSTALL_INTRO.store(false, Ordering::Release);
    }
    let _ = w.set_always_on_top(cfg.always_on_top);
    let dock_state = island_dock(&cfg, id.as_deref());
    if let Some(edge) = dock_state.edge {
        dock::anchor(&w, edge, dock_state.offset, width, height)?;
        return Ok(dock::work_area(&w).map(|area| area.3));
    } else {
        dock::resize_free(&w, width, height)?;
        if id.is_none() && FIRST_INSTALL_INTRO.load(Ordering::Acquire) {
            dock::place_free_top_center(&w)?;
        }
    }
    Ok(None)
}

#[tauri::command]
pub(crate) fn set_island_hit_region(
    window: WebviewWindow,
    rects: Vec<island_pointer::HitRect>,
    interactive: bool,
) -> Result<(), String> {
    let _ = island_id_of(&window)?;
    island_pointer::set_hit_region(window.label(), rects, interactive);
    Ok(())
}

#[tauri::command]
pub(crate) fn island_set_expanded(window: WebviewWindow, expanded: bool) -> Result<(), String> {
    let id = island_id_of(&window)?;
    if id.is_none() && expanded {
        FIRST_INSTALL_INTRO.store(false, Ordering::Release);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn island_drag(app: tauri::AppHandle, window: WebviewWindow) -> Result<(), String> {
    let id = island_id_of(&window)?;
    if id.is_none() {
        FIRST_INSTALL_INTRO.store(false, Ordering::Release);
    }
    let w = window;
    let is_capsule = app.state::<Cfg>().0.get().island_style == "capsule";
    if is_capsule {
        let _ = w.set_ignore_cursor_events(false);
    }
    dock::DRAGGING.store(true, Ordering::Relaxed);
    let result = dock::drag(&w);
    dock::DRAGGING.store(false, Ordering::Relaxed);
    let _ = app.emit(
        crate::events::DOCK_HINT,
        DockHintEvent {
            island: id.clone(),
            hint: None,
        },
    );
    result?;
    let snapped = dock_release_for(&app, &w, id.as_deref());
    if let (Some(id), None) = (id.as_deref(), snapped) {
        remember_clone_position(&app, id, &w);
    }
    crate::events::settings_changed(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn island_topmost(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    apply_island_topmost(&app, on)
}

#[tauri::command]
pub(crate) fn island_position(
    app: tauri::AppHandle,
    edge: Option<dock::Edge>,
    island: Option<String>,
) {
    apply_island_position(&app, edge, island.as_deref())
}

#[tauri::command]
pub(crate) fn set_island_style(
    app: tauri::AppHandle,
    style: String,
) -> Result<settings::Settings, String> {
    settings::validate_island_style(&style)?;
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| settings.island_style = style.clone())?;
    let is_capsule = style == "capsule";
    for w in island_windows(&app) {
        let id = island_clones::id_from_label(w.label()).flatten();
        let dock_state = island_dock(&next, id.as_deref());
        if is_capsule {
            let _ = w.set_ignore_cursor_events(true);
            if next.dock_enabled && dock_state.edge.is_some() {
                let _ = dock::restore_stage_dock(&w, &dock_state);
            } else {
                let _ = dock::apply_stage_free(&w);
            }
        } else {
            island_pointer::clear_hit_region(w.label());
            let _ = w.set_ignore_cursor_events(false);
            if next.dock_enabled && dock_state.edge.is_some() {
                let _ = dock::restore_dock(&w, &dock_state);
            } else {
                let _ = dock::resize_free(&w, dock::FREE_W, dock::FREE_H);
            }
        }
    }
    crate::events::settings_changed(&app);
    Ok(next)
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct MainContext {
    pub page: String,
    pub connection_id: Option<String>,
    #[serde(default)]
    pub main_center: Option<(f64, f64)>,
}

static MAIN_CONTEXT: std::sync::Mutex<Option<MainContext>> = std::sync::Mutex::new(None);

fn main_window_center(app: &tauri::AppHandle) -> Option<(f64, f64)> {
    let win = app.get_webview_window(MAIN)?;
    if !win.is_visible().unwrap_or(false) || win.is_minimized().unwrap_or(false) {
        return None;
    }
    let pos = win.outer_position().ok()?;
    let size = win.outer_size().ok()?;
    Some((
        f64::from(pos.x) + f64::from(size.width) / 2.0,
        f64::from(pos.y) + f64::from(size.height) / 2.0,
    ))
}

#[tauri::command]
pub(crate) fn set_main_context(
    app: tauri::AppHandle,
    page: String,
    connection_id: Option<String>,
) -> Result<(), String> {
    let next = MainContext {
        page,
        connection_id: connection_id.filter(|id| !id.trim().is_empty()),
        main_center: main_window_center(&app),
    };
    let mut slot = MAIN_CONTEXT.lock().map_err(|e| e.to_string())?;
    if slot.as_ref() == Some(&next) {
        return Ok(());
    }
    *slot = Some(next.clone());
    drop(slot);
    let _ = app.emit(crate::events::MAIN_CONTEXT, next);
    Ok(())
}

#[tauri::command]
pub(crate) fn main_context(app: tauri::AppHandle) -> Option<MainContext> {
    let mut current = MAIN_CONTEXT.lock().ok().and_then(|slot| slot.clone())?;
    if current.main_center.is_none() {
        current.main_center = main_window_center(&app);
    }
    Some(current)
}

#[tauri::command]
pub(crate) fn set_island_linkage(
    app: tauri::AppHandle,
    linkage: settings::IslandLinkage,
) -> Result<settings::Settings, String> {
    let previous_hooks = app.state::<Cfg>().0.get().island_linkage.agent_hooks;
    let normalized = settings::normalize_island_linkage(linkage);
    let hooks_changed = previous_hooks != normalized.agent_hooks;
    let hooks_enabled = normalized.agent_hooks;
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| settings.island_linkage = normalized)?;
    if hooks_changed {
        let data_dir = app.state::<Cfg>().0.dir().to_path_buf();
        if let Ok(exe) = std::env::current_exe() {
            agent_hooks::apply_hooks(&data_dir, &exe, hooks_enabled)?;
        }
        crate::app::watcher::broadcast_agent_hooks(&app);
    }
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) async fn island_active_accounts(
    db: State<'_, Db>,
) -> Result<Vec<local_active::LocalActiveAccount>, String> {
    with_database(db.0.clone(), move |conn| Ok(local_active::current_list(conn))).await
}

#[tauri::command]
pub(crate) async fn island_agent_hooks_state(
    cfg: State<'_, Cfg>,
    db: State<'_, DbRead>,
) -> Result<Vec<agent_hooks::AgentHookAlertDto>, String> {
    if !cfg.0.get().island_linkage.agent_hooks {
        return Ok(Vec::new());
    }
    let data_dir = cfg.0.dir().to_path_buf();
    let now_ms = chrono::Local::now().timestamp_millis();
    with_reader(db.0.clone(), move |conn| {
        Ok(agent_hooks::active_alerts(&data_dir, Some(conn), now_ms))
    })
    .await
}

/// 灵动岛展开卡片「一键收录」：后端即时重读本机生效配置并入库，明文凭证绝不跨越 IPC。
#[tauri::command]
pub(crate) async fn island_enroll_local_account(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    platform: String,
    source_agent: Option<String>,
) -> Result<local_enroll::EnrolledLocalAccountDto, String> {
    let pool = db.0.clone();
    let enrolled = tauri::async_runtime::spawn_blocking(move || {
        local_enroll::enroll_current_local(&pool, &platform, source_agent.as_deref())
    })
    .await
    .map_err(|e| e.to_string())??;

    invalidate_network_cache(&enrolled.connection_id);
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    if enrolled.models_fetched > 0 {
        crate::events::catalog_changed(&app);
    }
    let active_accounts = with_database(db.0.clone(), |conn| Ok(local_active::current_list(conn))).await?;
    let _ = app.emit(crate::events::ACTIVE_ACCOUNT_CHANGED, &active_accounts);
    crate::events::refresh_requested(&app, Some(enrolled.connection_id.as_str()));
    Ok(enrolled)
}

#[tauri::command]
pub(crate) async fn island_menu(
    app: tauri::AppHandle,
    window: WebviewWindow,
    refreshing: bool,
) -> Result<(), String> {
    let _ = refreshing;
    let id = island_id_of(&window)?;
    context_menu::open(&app, true, id)
}

#[tauri::command]
pub(crate) fn dock_undock(app: tauri::AppHandle, window: WebviewWindow) -> Result<(), String> {
    let id = island_id_of(&window)?;
    dock_undock_for(&app, &window, id.as_deref());
    Ok(())
}

#[tauri::command]
pub(crate) fn set_island_platform(
    app: tauri::AppHandle,
    _cfg: State<Cfg>,
    platform: String,
) -> Result<settings::Settings, String> {
    apply_island_platform(&app, platform)
}

#[tauri::command]
pub(crate) fn set_island_source(
    app: tauri::AppHandle,
    cfg: State<Cfg>,
    id: Option<String>,
) -> Result<settings::Settings, String> {
    let platform = cfg.0.get().island_platform;
    let valid = local_source(&platform).map(|source| source.id);
    if id.is_some() && id != valid {
        return Err("所选统计来源不存在或不属于当前平台".into());
    }
    let next = cfg.0.try_update(|settings| settings.island_source_id = id)?;
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) async fn set_island_connection(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    cfg: State<'_, Cfg>,
    id: Option<String>,
    island: Option<String>,
) -> Result<settings::Settings, String> {
    let selected = match id.as_ref() {
        Some(id) => {
            let lookup_id = id.clone();
            let Some(c) = with_database(db.0.clone(), move |conn| {
                connections::selectable_creds(conn, &lookup_id)
            })
            .await?
            else {
                return Err("所选连接不存在".into());
            };
            let source_id = native_sources::monitor_id(id)
                .map(str::to_string)
                .unwrap_or_else(|| format!("local:{}", c.platform));
            Some(island_clones::SelectedConnection {
                id: id.clone(),
                name: c.name,
                platform: c.platform,
                kind: c.kind,
                source_id,
            })
        }
        None => None,
    };
    let mut found = false;
    let next = cfg.0.try_update(|s| {
        found = island_clones::apply_connection(s, island.as_deref(), selected.as_ref());
    })?;
    if !found {
        return Err("目标灵动岛不存在".into());
    }
    crate::events::settings_changed(&app);
    if island.is_none() {
        refresh_tray_menu(&app);
    }
    Ok(next)
}

#[tauri::command]
pub(crate) async fn set_island_visibility(
    app: tauri::AppHandle,
    visible: bool,
) -> Result<settings::Settings, String> {
    set_island_visible(&app, visible)?;
    Ok(app.state::<Cfg>().0.get())
}

#[tauri::command]
pub(crate) async fn island_action(
    app: tauri::AppHandle,
    id: String,
    island: Option<String>,
) -> Result<(), String> {
    if !matches!(
        id.as_str(),
        "refresh" | "reset_layout" | "clone_create" | "clone_destroy" | "clone_promote"
    ) {
        return Err("不支持的灵动岛操作".into());
    }
    handle_tray_menu(&app, &id, island.as_deref())
}
