//! 灵动岛窗口：尺寸、拖动、停靠、置顶、样式与显示来源。

#[allow(unused_imports)]
use crate::*;

/// 调用方窗口必须是灵动岛（本体或分身），返回其分身 id（本体为 None）。
pub(crate) fn island_id_of(window: &WebviewWindow) -> Result<Option<String>, String> {
    island_clones::id_from_label(window.label()).ok_or_else(|| "只有灵动岛窗口能执行此操作".to_string())
}

pub(crate) fn island_dock(settings: &settings::Settings, id: Option<&str>) -> dock::DockState {
    island_clones::profile(settings, id).map(|profile| profile.dock).unwrap_or_default()
}

pub(crate) fn island_window(app: &tauri::AppHandle, id: Option<&str>) -> Option<WebviewWindow> {
    app.get_webview_window(&island_clones::label(id))
}

/// 当前存活的全部灵动岛窗口（本体 + 分身）。
pub(crate) fn island_windows(app: &tauri::AppHandle) -> Vec<WebviewWindow> {
    app.webview_windows().into_iter()
        .filter(|(label, _)| island_clones::is_island_label(label))
        .map(|(_, window)| window)
        .collect()
}

/// 本体在前、分身在后的全部岛 id。
pub(crate) fn island_ids(settings: &settings::Settings) -> Vec<Option<String>> {
    std::iter::once(None).chain(settings.island_clones.iter().map(|clone| Some(clone.id.clone()))).collect()
}

/// 停靠事件带上岛 id；各岛窗口只处理自己的，不会被别的岛的停靠变化重置形态。
#[derive(Clone, serde::Serialize)]
pub(crate) struct DockChangedEvent { pub(crate) island: Option<String>, pub(crate) dock: dock::DockState }

#[derive(Clone, serde::Serialize)]
pub(crate) struct DockHintEvent { pub(crate) island: Option<String>, pub(crate) hint: Option<dock::SnapHint> }

pub(crate) fn emit_dock_changed(app: &tauri::AppHandle, id: Option<&str>, dock: &dock::DockState) {
    let _ = app.emit(crate::events::DOCK_CHANGED, DockChangedEvent { island: id.map(str::to_string), dock: dock.clone() });
}

pub(crate) fn store_island_dock(app: &tauri::AppHandle, id: Option<&str>, dock: dock::DockState) -> settings::Settings {
    app.state::<Cfg>().0.update(|settings| { island_clones::set_dock(settings, id, dock); })
}

/// 分身的自由态位置只在拖动结束、创建与重置时记录，不在每次移动回调里落盘。
pub(crate) fn remember_clone_position(app: &tauri::AppHandle, id: &str, window: &WebviewWindow) {
    let Ok(position) = window.outer_position() else { return };
    let store = &app.state::<Cfg>().0;
    let changed = store.get().island_clones.iter()
        .any(|clone| clone.id == id && clone.position != Some((position.x, position.y)));
    if changed {
        store.update(|settings| { island_clones::set_position(settings, id, (position.x, position.y)); });
    }
}

/// 灵动岛窗口尺寸跟随内容（仅经典样式）。
/// 胶囊样式已切到固定舞台窗口（§3），形变期间不再改原生窗口大小，只返回工作区高度上限。
#[tauri::command]
pub(crate) fn resize_island(app: tauri::AppHandle, window: WebviewWindow, width: f64, height: f64) -> Result<Option<f64>, String> {
    let id = island_id_of(&window)?;
    let w = window;
    if !width.is_finite() || !height.is_finite() || width < 8.0 || height < 8.0 {
        return Err("灵动岛尺寸无效".into());
    }
    let cfg = app.state::<Cfg>().0.get();
    if cfg.island_style == "capsule" {
        return Ok(dock::work_area(&w).map(|area| area.3));
    }
    // 用户主动展开本体后，不再按首次安装计时强制收成停靠条（经典样式仍按窗口高度判断）。
    if id.is_none() && height > 150.0 { FIRST_INSTALL_INTRO.store(false, Ordering::Release); }
    // 改尺寸后重新确认置顶：对应设置里默认开启的「灵动岛置顶」
    let _ = w.set_always_on_top(cfg.always_on_top);
    let dock_state = island_dock(&cfg, id.as_deref());
    if let Some(edge) = dock_state.edge {
        dock::anchor(&w, edge, dock_state.offset, width, height)?;
        // 把停靠窗口的实际工作区上限交给前端，避免动画等待不可达高度。
        return Ok(dock::work_area(&w).map(|area| area.3));
    } else {
        dock::resize_free(&w, width, height)?;
        if id.is_none() && FIRST_INSTALL_INTRO.load(Ordering::Acquire) {
            dock::place_free_top_center(&w)?;
        }
    }
    Ok(None)
}

/// 胶囊舞台窗口上报当前可见轮廓（含四角圆角）与强制交互态（如下拉菜单打开中）。
/// 后端 `island_pointer` 据此精确切换鼠标穿透，透明区域点击落到下方程序。
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

/// 胶囊形态通知后端展开 / 收起：替代原依赖窗口高度 > 150px 判断首次安装引导的逻辑（§3）。
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
    if id.is_none() { FIRST_INSTALL_INTRO.store(false, Ordering::Release); }
    let w = window;
    let is_capsule = app.state::<Cfg>().0.get().island_style == "capsule";
    if is_capsule {
        let _ = w.set_ignore_cursor_events(false);
    }
    dock::DRAGGING.store(true, std::sync::atomic::Ordering::Relaxed);
    let result = dock::drag(&w);
    dock::DRAGGING.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = app.emit(crate::events::DOCK_HINT, DockHintEvent { island: id.clone(), hint: None });
    result?;
    let snapped = dock_release_for(&app, &w, id.as_deref());
    if let (Some(id), None) = (id.as_deref(), snapped) { remember_clone_position(&app, id, &w); }
    crate::events::settings_changed(&app);
    Ok(())
}

/// 置顶是全局开关：对本体与全部分身同时生效。
#[tauri::command]
pub(crate) fn island_topmost(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    let windows = island_windows(&app);
    if windows.is_empty() { return Err("找不到灵动岛".into()); }
    let previous = app.state::<Cfg>().0.get().always_on_top;
    for w in &windows { w.set_always_on_top(on).map_err(|e| e.to_string())?; }
    let next = match app.state::<Cfg>().0.try_update(|s| s.always_on_top = on) {
        Ok(next) => next,
        Err(error) => { for w in &windows { let _ = w.set_always_on_top(previous); } return Err(error); }
    };
    crate::events::settings_changed(&app);
    Ok(next)
}

/// 右键菜单「显示位置」：`island` 为目标岛（None 为本体）。
#[tauri::command]
pub(crate) fn island_position(app: tauri::AppHandle, edge: Option<dock::Edge>, island: Option<String>) {
    let id = island.as_deref();
    if id.is_none() { FIRST_INSTALL_INTRO.store(false, Ordering::Release); }
    let Some(w) = island_window(&app, id) else { return };
    let is_capsule = app.state::<Cfg>().0.get().island_style == "capsule";
    if let Some(edge) = edge {
        let Some(offset) = dock::centered_offset(&w, edge) else { return };
        if is_capsule {
            dock::apply_stage_dock(&w, edge, offset);
        } else {
            dock::apply_dock(&w, edge, offset);
        }
        // 窗口 API 会跨线程等待主事件循环；禁止在设置锁内调用，移动回调也会读取设置。
        let monitor = w.current_monitor().ok().flatten().and_then(|m| m.name().cloned());
        store_island_dock(&app, id, dock::DockState { edge: Some(edge), offset, monitor });
    } else {
        dock_undock_for(&app, &w, id);
    }
    let settings = app.state::<Cfg>().0.get();
    crate::events::settings_changed(&app);
    emit_dock_changed(&app, id, &island_dock(&settings, id));
}

#[tauri::command]
pub(crate) fn set_island_style(app: tauri::AppHandle, style: String) -> Result<settings::Settings, String> {
    settings::validate_island_style(&style)?;
    let next = app.state::<Cfg>().0.try_update(|settings| settings.island_style = style.clone())?;
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

/// 主窗口当前在看的页面与连接（4.2b #9）。只在内存里，重启清空；本体岛开了「跟随主窗口」才用它
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct MainContext {
    pub page: String,
    pub connection_id: Option<String>,
}

static MAIN_CONTEXT: std::sync::Mutex<Option<MainContext>> = std::sync::Mutex::new(None);

/// 主窗口报告「正在看哪个连接」（离开时报 connection_id = null）；变了才广播 `main-context`
#[tauri::command]
pub(crate) fn set_main_context(app: tauri::AppHandle, page: String, connection_id: Option<String>) -> Result<(), String> {
    use tauri::Emitter;
    let next = MainContext { page, connection_id: connection_id.filter(|id| !id.trim().is_empty()) };
    let mut slot = MAIN_CONTEXT.lock().map_err(|e| e.to_string())?;
    if slot.as_ref() == Some(&next) {
        return Ok(());
    }
    *slot = Some(next.clone());
    drop(slot);
    let _ = app.emit(crate::events::MAIN_CONTEXT, next);
    Ok(())
}

/// 岛打开时读一次当前的主窗口上下文
#[tauri::command]
pub(crate) fn main_context() -> Option<MainContext> {
    MAIN_CONTEXT.lock().ok().and_then(|slot| slot.clone())
}

/// 灵动岛联动开关（4.2b）：严格模式、信号动效、跟随主窗口与自动选主
#[tauri::command]
pub(crate) fn set_island_linkage(app: tauri::AppHandle, linkage: settings::IslandLinkage) -> Result<settings::Settings, String> {
    let next = app.state::<Cfg>().0.try_update(|settings| settings.island_linkage = linkage)?;
    crate::events::settings_changed(&app);
    Ok(next)
}

#[tauri::command]
pub(crate) async fn island_menu(app: tauri::AppHandle, window: WebviewWindow, refreshing: bool) -> Result<(), String> {
    let _ = refreshing;
    let id = island_id_of(&window)?;
    context_menu::open(&app, true, id)
}

pub(crate) fn dock_release_for(app: &tauri::AppHandle, w: &WebviewWindow, id: Option<&str>) -> Option<dock::SnapResult> {
    let store = &app.state::<Cfg>().0;
    let cfg = store.get();
    if !cfg.dock_enabled {
        store.update(|settings| { island_clones::set_dock(settings, id, Default::default()); });
        emit_dock_changed(app, id, &Default::default());
        return None;
    }
    let r = if cfg.island_style == "capsule" {
        dock::snap_on_release_stage(w, island_pointer::primary_contour(w.label()))
    } else {
        dock::snap_on_release(w)
    };
    let monitor = w.current_monitor().ok().flatten().and_then(|m| m.name().cloned());
    let dock_state = match &r {
        Some(sr) => dock::DockState { edge: sr.edge, offset: sr.offset, monitor },
        None => Default::default(),
    };
    store.update(|settings| { island_clones::set_dock(settings, id, dock_state.clone()); });
    emit_dock_changed(app, id, &dock_state);
    r
}

/// 解除停靠，回到自由态
#[tauri::command]
pub(crate) fn dock_undock(app: tauri::AppHandle, window: WebviewWindow) -> Result<(), String> {
    let id = island_id_of(&window)?;
    dock_undock_for(&app, &window, id.as_deref());
    Ok(())
}

pub(crate) fn dock_undock_for(app: &tauri::AppHandle, w: &WebviewWindow, id: Option<&str>) {
    let cfg = app.state::<Cfg>().0.get();
    let prev_edge = island_dock(&cfg, id).edge;
    if cfg.island_style == "capsule" {
        dock::undock_stage(w, prev_edge);
    } else {
        dock::undock(w);
    }
    app.state::<Cfg>().0.update(|settings| { island_clones::set_dock(settings, id, Default::default()); });
    crate::events::settings_changed(&app);
    emit_dock_changed(app, id, &Default::default());
}

/// 设置界面改灵动岛平台：与托盘子菜单是同一个值，改完要刷新托盘勾选态
#[tauri::command]
pub(crate) fn set_island_platform(
    app: tauri::AppHandle,
    cfg: State<Cfg>,
    platform: String,
) -> Result<settings::Settings, String> {
    if !platforms::known(&platform) {
        return Err("未知平台".into());
    }
    let next = cfg.0.try_update(|s| {
        s.island_platform = platform.clone();
        s.island_connection_id = None;
        s.island_connection_name = None;
        s.island_source_id = Some(format!("local:{platform}"));
    })?;
    crate::events::settings_changed(&app);
    refresh_tray_menu(&app);
    Ok(next)
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

/// `island` 为目标岛（None 为本体）。托盘摘要与主面板默认连接只跟随本体。
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
            }).await? else {
                return Err("所选连接不存在".into());
            };
            let source_id = native_sources::monitor_id(id).map(str::to_string).unwrap_or_else(|| format!("local:{}", c.platform));
            Some(island_clones::SelectedConnection { id: id.clone(), name: c.name, platform: c.platform, kind: c.kind, source_id })
        }
        None => None,
    };
    let mut found = false;
    let next = cfg.0.try_update(|s| { found = island_clones::apply_connection(s, island.as_deref(), selected.as_ref()); })?;
    if !found { return Err("目标灵动岛不存在".into()); }
    crate::events::settings_changed(&app);
    if island.is_none() {
        refresh_tray_menu(&app);
    }
    Ok(next)
}

/// 主面板「显示灵动岛」开关与右上角按钮；与托盘的显示 / 隐藏是同一个值。
/// 显示时可能要重建窗口，WebView2 不允许在同步 IPC 回调里建窗口，所以是 async。
#[tauri::command]
pub(crate) async fn set_island_visibility(app: tauri::AppHandle, visible: bool) -> Result<settings::Settings, String> {
    set_island_visible(&app, visible)?;
    Ok(app.state::<Cfg>().0.get())
}

/// 主面板「灵动岛」页里的操作，复用托盘 / 右键菜单的同一套实现。
/// `island` 是目标岛：开启分身时为来源窗口，销毁时为被销毁的窗口，None 为本体。
#[tauri::command]
pub(crate) async fn island_action(app: tauri::AppHandle, id: String, island: Option<String>) -> Result<(), String> {
    if !matches!(id.as_str(), "refresh" | "reset_layout" | "clone_create" | "clone_destroy" | "clone_promote") {
        return Err("不支持的灵动岛操作".into());
    }
    handle_tray_menu(&app, &id, island.as_deref())
}
