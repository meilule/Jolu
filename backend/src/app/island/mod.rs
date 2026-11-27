//! 灵动岛宿主层：本体与分身窗口生命周期、停靠几何计算、状态广播与自适应指针命中。

use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::{Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::app::state::{Cfg, FIRST_INSTALL, FIRST_INSTALL_INTRO, ISLAND};
use crate::{platforms, settings};

pub(crate) mod clones;
pub(crate) mod dock;
pub(crate) mod pointer;

/// 调用方窗口必须是灵动岛（本体或分身），返回其分身 id（本体为 None）。
pub(crate) fn island_id_of(window: &WebviewWindow) -> Result<Option<String>, String> {
    clones::id_from_label(window.label()).ok_or_else(|| "只有灵动岛窗口能执行此操作".to_string())
}

pub(crate) fn island_dock(settings: &settings::Settings, id: Option<&str>) -> dock::DockState {
    clones::profile(settings, id)
        .map(|profile| profile.dock)
        .unwrap_or_default()
}

pub(crate) fn island_window(app: &tauri::AppHandle, id: Option<&str>) -> Option<WebviewWindow> {
    app.get_webview_window(&clones::label(id))
}

/// 当前存活的全部灵动岛窗口（本体 + 分身）。
pub(crate) fn island_windows(app: &tauri::AppHandle) -> Vec<WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _)| clones::is_island_label(label))
        .map(|(_, window)| window)
        .collect()
}

/// 本体在前、分身在后的全部岛 id。
pub(crate) fn island_ids(settings: &settings::Settings) -> Vec<Option<String>> {
    std::iter::once(None)
        .chain(settings.island_clones.iter().map(|clone| Some(clone.id.clone())))
        .collect()
}

/// 停靠事件带上岛 id；各岛窗口只处理自己的，不会被别的岛的停靠变化重置形态。
#[derive(Clone, serde::Serialize)]
pub(crate) struct DockChangedEvent {
    pub(crate) island: Option<String>,
    pub(crate) dock: dock::DockState,
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct DockHintEvent {
    pub(crate) island: Option<String>,
    pub(crate) hint: Option<dock::SnapHint>,
}

pub(crate) fn emit_dock_changed(app: &tauri::AppHandle, id: Option<&str>, dock: &dock::DockState) {
    let _ = app.emit(
        crate::events::DOCK_CHANGED,
        DockChangedEvent {
            island: id.map(str::to_string),
            dock: dock.clone(),
        },
    );
}

pub(crate) fn store_island_dock(
    app: &tauri::AppHandle,
    id: Option<&str>,
    dock: dock::DockState,
) -> settings::Settings {
    app.state::<Cfg>().0.update(|settings| {
        clones::set_dock(settings, id, dock);
    })
}

/// 分身的自由态位置只在拖动结束、创建与重置时记录，不在每次移动回调里落盘。
pub(crate) fn remember_clone_position(app: &tauri::AppHandle, id: &str, window: &WebviewWindow) {
    let Ok(position) = window.outer_position() else { return };
    let store = &app.state::<Cfg>().0;
    let changed = store
        .get()
        .island_clones
        .iter()
        .any(|clone| clone.id == id && clone.position != Some((position.x, position.y)));
    if changed {
        store.update(|settings| {
            clones::set_position(settings, id, (position.x, position.y));
        });
    }
}

pub(crate) fn dock_release_for(
    app: &tauri::AppHandle,
    w: &WebviewWindow,
    id: Option<&str>,
) -> Option<dock::SnapResult> {
    let store = &app.state::<Cfg>().0;
    let cfg = store.get();
    if !cfg.dock_enabled {
        store.update(|settings| {
            clones::set_dock(settings, id, Default::default());
        });
        emit_dock_changed(app, id, &Default::default());
        return None;
    }
    let r = if cfg.island_style == "capsule" {
        dock::snap_on_release_stage(w, pointer::primary_contour(w.label()))
    } else {
        dock::snap_on_release(w)
    };
    let monitor = w.current_monitor().ok().flatten().and_then(|m| m.name().cloned());
    let dock_state = match &r {
        Some(sr) => dock::DockState {
            edge: sr.edge,
            offset: sr.offset,
            monitor,
        },
        None => Default::default(),
    };
    store.update(|settings| {
        clones::set_dock(settings, id, dock_state.clone());
    });
    emit_dock_changed(app, id, &dock_state);
    r
}

pub(crate) fn dock_undock_for(app: &tauri::AppHandle, w: &WebviewWindow, id: Option<&str>) {
    let cfg = app.state::<Cfg>().0.get();
    let prev_edge = island_dock(&cfg, id).edge;
    if cfg.island_style == "capsule" {
        dock::undock_stage(w, prev_edge);
    } else {
        dock::undock(w);
    }
    app.state::<Cfg>().0.update(|settings| {
        clones::set_dock(settings, id, Default::default());
    });
    crate::events::settings_changed(app);
    emit_dock_changed(app, id, &Default::default());
}

pub(crate) fn apply_island_topmost(
    app: &tauri::AppHandle,
    on: bool,
) -> Result<settings::Settings, String> {
    let windows = island_windows(app);
    if windows.is_empty() {
        return Err("找不到灵动岛".into());
    }
    let previous = app.state::<Cfg>().0.get().always_on_top;
    for w in &windows {
        w.set_always_on_top(on).map_err(|e| e.to_string())?;
    }
    let next = match app.state::<Cfg>().0.try_update(|s| s.always_on_top = on) {
        Ok(next) => next,
        Err(error) => {
            for w in &windows {
                let _ = w.set_always_on_top(previous);
            }
            return Err(error);
        }
    };
    crate::events::settings_changed(app);
    Ok(next)
}

pub(crate) fn apply_island_position(
    app: &tauri::AppHandle,
    edge: Option<dock::Edge>,
    island: Option<&str>,
) {
    if island.is_none() {
        FIRST_INSTALL_INTRO.store(false, Ordering::Release);
    }
    let Some(w) = island_window(app, island) else { return };
    let is_capsule = app.state::<Cfg>().0.get().island_style == "capsule";
    if let Some(edge) = edge {
        let Some(offset) = dock::centered_offset(&w, edge) else { return };
        if is_capsule {
            dock::apply_stage_dock(&w, edge, offset);
        } else {
            dock::apply_dock(&w, edge, offset);
        }
        let monitor = w.current_monitor().ok().flatten().and_then(|m| m.name().cloned());
        store_island_dock(app, island, dock::DockState { edge: Some(edge), offset, monitor });
    } else {
        dock_undock_for(app, &w, island);
    }
    let settings = app.state::<Cfg>().0.get();
    crate::events::settings_changed(app);
    emit_dock_changed(app, island, &island_dock(&settings, island));
}

pub(crate) fn apply_island_platform(
    app: &tauri::AppHandle,
    platform: String,
) -> Result<settings::Settings, String> {
    if !platforms::known(&platform) {
        return Err("未知平台".into());
    }
    let next = app.state::<Cfg>().0.try_update(|s| {
        s.island_platform = platform.clone();
        s.island_connection_id = None;
        s.island_connection_name = None;
        s.island_source_id = Some(format!("local:{platform}"));
    })?;
    crate::events::settings_changed(app);
    crate::app::tray::refresh_tray_menu(app);
    Ok(next)
}

pub(crate) fn ensure_island_window_for(
    app: &tauri::AppHandle,
    id: Option<&str>,
) -> Result<WebviewWindow, String> {
    let label = clones::label(id);
    if let Some(window) = app.get_webview_window(&label) {
        return Ok(window);
    }
    let settings = app.state::<Cfg>().0.get();
    let profile = clones::profile(&settings, id).ok_or("灵动岛配置不存在")?;
    let url = match id {
        Some(id) => format!("index.html?window=island&clone={id}"),
        None => "index.html?window=island".to_string(),
    };
    let is_capsule = settings.island_style == "capsule";
    let (init_w, init_h) = if is_capsule {
        (dock::STAGE_W, dock::STAGE_H)
    } else {
        (dock::FREE_W, dock::FREE_H)
    };
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title("Jolu")
        .inner_size(init_w, init_h)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(settings.always_on_top)
        .skip_taskbar(true)
        .visible(false)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    if is_capsule {
        let _ = window.set_ignore_cursor_events(true);
    }
    if settings.dock_enabled && profile.dock.edge.is_some() {
        let _ = if is_capsule {
            dock::restore_stage_dock(&window, &profile.dock)
        } else {
            dock::restore_dock(&window, &profile.dock)
        };
    } else if let Some((x, y)) = id
        .and_then(|id| settings.island_clones.iter().find(|clone| clone.id == id))
        .and_then(|clone| clone.position)
    {
        let _ = window.set_position(PhysicalPosition::new(x, y));
        if is_capsule {
            let _ = dock::apply_stage_free(&window);
        } else {
            let _ = dock::resize_free(&window, dock::FREE_W, dock::FREE_H);
        }
    } else if is_capsule {
        let _ = dock::apply_stage_free(&window);
    }
    Ok(window)
}

pub(crate) fn create_island_clone(app: &tauri::AppHandle, source: Option<&str>) -> Result<(), String> {
    let store = &app.state::<Cfg>().0;
    clones::create_clone(&mut store.get(), source)?;
    let mut created = None;
    let next = store.try_update(|settings| {
        created = clones::create_clone(settings, source).ok();
    })?;
    let clone = created.ok_or("创建分身失败")?;
    let window = ensure_island_window_for(app, Some(&clone.id))?;
    let is_capsule = next.island_style == "capsule";
    if let Some(origin) = island_window(app, source) {
        if let (Ok(position), Ok(scale)) = (origin.outer_position(), origin.scale_factor()) {
            let offset = (24.0 * scale).round() as i32;
            let _ = window.set_position(PhysicalPosition::new(position.x + offset, position.y + offset));
            if is_capsule {
                let _ = dock::apply_stage_free(&window);
            } else {
                let _ = dock::resize_free(&window, dock::FREE_W, dock::FREE_H);
            }
        }
    }
    remember_clone_position(app, &clone.id, &window);
    let _ = window.set_always_on_top(next.always_on_top);
    if next.island_visible {
        window.show().map_err(|e| e.to_string())?;
        let _ = window.set_focus();
    }
    crate::events::settings_changed(app);
    let _ = app.emit(crate::events::MENU_STATE_CHANGED, ());
    Ok(())
}

pub(crate) fn destroy_island(app: &tauri::AppHandle, target: Option<&str>) -> Result<(), String> {
    let store = &app.state::<Cfg>().0;
    clones::destroy(&mut store.get(), target)?;
    let mut plan = None;
    store.try_update(|settings| {
        plan = clones::destroy(settings, target).ok();
    })?;
    let is_capsule = store.get().island_style == "capsule";
    match plan.ok_or("销毁分身失败")? {
        clones::DestroyPlan::CloseClone(id) => {
            pointer::clear_hit_region(&clones::label(Some(&id)));
            if let Some(window) = island_window(app, Some(&id)) {
                window.destroy().map_err(|e| e.to_string())?;
            }
        }
        clones::DestroyPlan::PromoteClone { closed } => {
            pointer::clear_hit_region(&clones::label(Some(&closed.id)));
            let clone_window = island_window(app, Some(&closed.id));
            if let Some(primary) = app.get_webview_window(ISLAND) {
                if let Some(window) = &clone_window {
                    if let (Ok(position), Ok(size)) = (window.outer_position(), window.outer_size()) {
                        let _ = primary.set_size(size);
                        let _ = primary.set_position(position);
                    }
                }
                if closed.dock.edge.is_some() {
                    let _ = if is_capsule {
                        dock::restore_stage_dock(&primary, &closed.dock)
                    } else {
                        dock::restore_dock(&primary, &closed.dock)
                    };
                }
                emit_dock_changed(app, None, &closed.dock);
            }
            if let Some(window) = clone_window {
                window.destroy().map_err(|e| e.to_string())?;
            }
            crate::app::tray::refresh_tray_menu(app);
        }
    }
    crate::events::settings_changed(app);
    let _ = app.emit(crate::events::MENU_STATE_CHANGED, ());
    Ok(())
}

pub(crate) fn setup_initial_islands(app: &tauri::AppHandle) {
    let cfg = app.state::<Cfg>().0.get();
    let is_capsule = cfg.island_style == "capsule";
    let first_intro = FIRST_INSTALL.swap(false, Ordering::AcqRel)
        && cfg.island_visible
        && cfg.dock_enabled
        && cfg.dock.edge.is_none();
    if let Some(w) = app.get_webview_window(ISLAND) {
        let _ = w.set_always_on_top(cfg.always_on_top);
        if is_capsule {
            let _ = w.set_ignore_cursor_events(true);
        }
        if first_intro {
            FIRST_INSTALL_INTRO.store(true, Ordering::Release);
            let placed = if is_capsule {
                dock::place_stage_free_top_center(&w).map(|_| ())
            } else {
                dock::place_free_top_center(&w)
            };
            if let Err(error) = placed {
                eprintln!("[启动] 灵动岛首次居中失败: {error}");
                let _ = w.center();
            }
        } else if is_capsule && !(cfg.dock_enabled && cfg.dock.edge.is_some()) {
            let _ = dock::apply_stage_free(&w);
        }
    }
    if let (true, Some(_edge), Some(w)) =
        (cfg.dock_enabled, cfg.dock.edge, app.get_webview_window(ISLAND))
    {
        let restored = if is_capsule {
            dock::restore_stage_dock(&w, &cfg.dock)
        } else {
            dock::restore_dock(&w, &cfg.dock)
        };
        if let Some(monitor) = restored {
            if cfg.dock.monitor.as_deref() != Some(&monitor) {
                app.state::<Cfg>()
                    .0
                    .update(|settings| settings.dock.monitor = Some(monitor));
            }
        }
    }
    if let Some(w) = app.get_webview_window(ISLAND) {
        if cfg.island_visible {
            let _ = w.show();
        } else {
            let _ = w.hide();
        }
    }
    for clone in &cfg.island_clones {
        match ensure_island_window_for(app, Some(&clone.id)) {
            Ok(w) => {
                if cfg.island_visible {
                    let _ = w.show();
                }
            }
            Err(error) => eprintln!("[启动] 重建灵动岛分身 {} 失败: {error}", clone.id),
        }
    }
    if first_intro {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if !FIRST_INSTALL_INTRO.swap(false, Ordering::AcqRel) {
                return;
            }
            let settings = app.state::<Cfg>().0.get();
            if settings.island_visible && settings.dock_enabled && settings.dock.edge.is_none() {
                apply_island_position(&app, Some(dock::Edge::Top), None);
            }
        });
    }
}

pub(crate) fn set_island_visible(app: &tauri::AppHandle, visible: bool) -> Result<(), String> {
    if !visible {
        FIRST_INSTALL_INTRO.store(false, Ordering::Release);
    }
    if visible {
        for id in island_ids(&app.state::<Cfg>().0.get()) {
            let w = ensure_island_window_for(app, id.as_deref())?;
            w.unminimize().map_err(|e| e.to_string())?;
            w.show().map_err(|e| e.to_string())?;
            let _ = w.set_focus();
        }
    } else {
        for w in island_windows(app) {
            w.hide().map_err(|e| e.to_string())?;
        }
    }
    app.state::<Cfg>().0.update(|s| s.island_visible = visible);
    crate::events::settings_changed(app);
    crate::app::tray::refresh_tray_menu(app);
    Ok(())
}

pub(crate) fn toggle_island(app: &tauri::AppHandle) -> Result<(), String> {
    let visible = match app.get_webview_window(ISLAND) {
        Some(window) => {
            !window.is_visible().map_err(|e| e.to_string())?
                || window.is_minimized().map_err(|e| e.to_string())?
        }
        None => true,
    };
    set_island_visible(app, visible)
}

pub(crate) fn reset_window_layout(app: &tauri::AppHandle) {
    FIRST_INSTALL_INTRO.store(false, Ordering::Release);
    app.state::<Cfg>()
        .0
        .update(|settings| settings.main_window = None);
    if let Ok(mut placement) = crate::app::state::MAIN_PLACEMENT.lock() {
        *placement = None;
    }
    if let Some(w) = app.get_webview_window(ISLAND) {
        let next = app.state::<Cfg>().0.update(|s| {
            s.dock = Default::default();
            for clone in &mut s.island_clones {
                clone.dock = Default::default();
                clone.position = None;
            }
        });
        crate::events::settings_changed(app);
        for id in island_ids(&next) {
            emit_dock_changed(app, id.as_deref(), &Default::default());
        }
        let is_capsule = next.island_style == "capsule";
        if is_capsule {
            let _ = dock::place_stage_free_top_center(&w);
            let _ = w.set_ignore_cursor_events(true);
        } else {
            let _ = w.set_size(tauri::LogicalSize::new(428.0, 124.0));
            let _ = w.center();
        }
        let origin = w.outer_position().ok();
        let scale = w.scale_factor().unwrap_or(1.0);
        for (index, clone) in next.island_clones.iter().enumerate() {
            let Some(cw) = island_window(app, Some(&clone.id)) else {
                continue;
            };
            if is_capsule {
                let _ = cw.set_ignore_cursor_events(true);
            } else {
                let _ = cw.set_size(tauri::LogicalSize::new(428.0, 124.0));
            }
            if let Some(position) = origin {
                let offset = ((index as f64 + 1.0) * 24.0 * scale).round() as i32;
                let _ =
                    cw.set_position(PhysicalPosition::new(position.x + offset, position.y + offset));
                if is_capsule {
                    let _ = dock::apply_stage_free(&cw);
                } else {
                    let _ = dock::resize_free(&cw, dock::FREE_W, dock::FREE_H);
                }
            }
            remember_clone_position(app, &clone.id, &cw);
        }
        if let Err(e) = set_island_visible(app, true) {
            eprintln!("[窗口] 恢复灵动岛失败: {e}");
        }
    }
    if let Some(w) = app.get_webview_window(crate::app::state::MAIN) {
        let _ = w.set_size(tauri::LogicalSize::new(1280.0, 900.0));
        let _ = w.center();
        let captured = crate::app::main_window::capture(&w);
        if let Ok(mut placement) = crate::app::state::MAIN_PLACEMENT.lock() {
            *placement = captured;
        }
    }
}
