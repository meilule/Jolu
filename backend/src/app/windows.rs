//! 主面板、启动窗口、协议唤醒与原生窗口事件响应。

use std::collections::HashSet;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::app::island::{clones as island_clones, dock, island_dock, pointer as island_pointer, DockHintEvent};
use crate::app::state::{
    Cfg, MainIntent, MAIN, MAIN_INTENT, MAIN_PLACEMENT, MAIN_READY, STARTUP, STARTUP_ERROR,
    STARTUP_READY, STARTUP_REQUESTED,
};
use crate::app::{main_window, toast_link, tray};
use crate::{keep_awake, proxy, session_handoff, settings};

pub(crate) fn persist_main_placement(app: &tauri::AppHandle, placement: main_window::Placement) {
    if !placement.valid() {
        return;
    }
    app.state::<Cfg>()
        .0
        .update(|settings| settings.main_window = Some(placement));
}

static PENDING_PLACEMENT_MS: AtomicI64 = AtomicI64::new(0);

fn schedule_persist_placement(window: &tauri::Window, placement: main_window::Placement) {
    let now = chrono::Utc::now().timestamp_millis();
    PENDING_PLACEMENT_MS.store(now, Ordering::Release);
    let handle = window.app_handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(800));
        if PENDING_PLACEMENT_MS.load(Ordering::Acquire) != now {
            return;
        }
        persist_main_placement(&handle, placement);
    });
}

fn record_main_window_event(window: &tauri::Window, event: &WindowEvent) {
    if !MAIN_READY.load(Ordering::Acquire) {
        return;
    }
    let placement = {
        let Ok(mut current) = MAIN_PLACEMENT.lock() else {
            return;
        };
        let Some(placement) = current.as_mut() else {
            return;
        };
        let mut next = placement.clone();
        match event {
            WindowEvent::Moved(position) => {
                next.x = position.x;
                next.y = position.y;
            }
            WindowEvent::Resized(size) => {
                next.width = size.width;
                next.height = size.height;
            }
            WindowEvent::CloseRequested { .. } => {}
            _ => return,
        }
        if !next.valid() {
            return;
        }
        *placement = next.clone();
        next
    };
    if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) {
        schedule_persist_placement(window, placement.clone());
    }
    if matches!(event, WindowEvent::CloseRequested { .. }) {
        persist_main_placement(window.app_handle(), placement);
    }
}

pub(crate) fn show_startup(app: &tauri::AppHandle) -> tauri::Result<()> {
    let window = if let Some(window) = app.get_webview_window(STARTUP) {
        window
    } else {
        let theme = app
            .try_state::<Cfg>()
            .map(|cfg| cfg.0.get().theme)
            .filter(|t| matches!(t.as_str(), "light" | "dark" | "system"));
        let url = format!("startup.html?theme={}", theme.as_deref().unwrap_or("system"));
        let (width, height) = startup_size(app);
        WebviewWindowBuilder::new(app, STARTUP, WebviewUrl::App(url.into()))
            .title("Jolu · 启动中")
            .inner_size(width, height)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .skip_taskbar(true)
            .center()
            .build()?
    };
    window.show()?;
    let _ = window.set_focus();
    Ok(())
}

fn startup_size(app: &tauri::AppHandle) -> (f64, f64) {
    let scale = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|monitor| {
            let factor = monitor.scale_factor();
            let area = monitor.work_area().size;
            fit_scale(area.width as f64 / factor, area.height as f64 / factor)
        })
        .unwrap_or(1.0);
    (1280.0 * scale, 720.0 * scale)
}

fn fit_scale(width: f64, height: f64) -> f64 {
    (width * 0.9 / 1280.0)
        .min(height * 0.9 / 720.0)
        .clamp(0.4, 1.0)
}

pub(crate) fn report_startup_error(app: &tauri::AppHandle, error: String) {
    eprintln!("[启动] {error}");
    if let Ok(mut current) = STARTUP_ERROR.lock() {
        *current = Some(error.clone());
    }
    if STARTUP_REQUESTED.load(Ordering::Acquire) {
        let _ = show_startup(app);
        let _ = app.emit(crate::events::STARTUP_ERROR, error);
    }
}

pub(crate) fn link_intent(app: &tauri::AppHandle, target: toast_link::LinkTarget) -> MainIntent {
    match target {
        toast_link::LinkTarget::Task(task_id) => MainIntent::Automation { task_id },
        toast_link::LinkTarget::Connection(connection_id) => {
            MainIntent::Providers { connection_id }
        }
        toast_link::LinkTarget::Main => MainIntent::Overview {
            settings: app
                .try_state::<Cfg>()
                .map(|cfg| cfg.0.get())
                .unwrap_or_default(),
        },
    }
}

pub(crate) fn activate_with_link(app: &tauri::AppHandle, target: toast_link::LinkTarget) {
    if STARTUP_READY.load(Ordering::Acquire) {
        let intent = link_intent(app, target);
        show_main_with_intent(app, intent);
        return;
    }
    let intent = link_intent(app, target);
    if let Ok(mut pending) = MAIN_INTENT.lock() {
        *pending = Some(intent);
    }
    STARTUP_REQUESTED.store(true, Ordering::Release);
    if let Err(error) = show_startup(app) {
        eprintln!("[启动] 无法显示启动窗口: {error}");
    }
}

pub(crate) fn activate_existing_instance(app: &tauri::AppHandle) {
    STARTUP_REQUESTED.store(true, Ordering::Release);
    if STARTUP_READY.load(Ordering::Acquire) {
        let settings = app
            .try_state::<Cfg>()
            .map(|cfg| cfg.0.get())
            .unwrap_or_default();
        show_main_with_intent(app, MainIntent::Overview { settings });
    } else if let Err(error) = show_startup(app) {
        eprintln!("[启动] 无法显示启动窗口: {error}");
    }
}

/// 界面缩放只作用于 `main` 窗口（D-21）；灵动岛、菜单等窗口各自独立，保持原大小。
pub(crate) fn apply_main_zoom(window: &WebviewWindow, zoom: u16) -> Result<(), String> {
    window.set_zoom(f64::from(zoom) / 100.0).map_err(|error| {
        eprintln!("[窗口] 设置主面板缩放失败: {error}");
        error.to_string()
    })
}

pub(crate) fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        if MAIN_READY.load(Ordering::Acquire) {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        } else {
            let _ = show_startup(app);
        }
        return;
    }
    MAIN_READY.store(false, Ordering::Release);
    if let Err(error) = show_startup(app) {
        eprintln!("[启动] 无法显示启动窗口: {error}");
    }
    let window = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("Jolu")
        .inner_size(1280.0, 900.0)
        .min_inner_size(900.0, 600.0)
        .decorations(false)
        .visible(false)
        .center()
        .build();
    match window {
        Ok(window) => {
            let _ = window.set_icon(tray::icon::taskbar(window.scale_factor().unwrap_or(1.0)));
            if let Some(cfg) = app.try_state::<Cfg>() {
                let _ = apply_main_zoom(&window, settings::normalize_zoom(cfg.0.get().main_zoom));
            }
            if let Some(saved) = app.try_state::<Cfg>().and_then(|cfg| cfg.0.get().main_window) {
                if let Err(error) = main_window::restore(&window, &saved) {
                    eprintln!("[窗口] 恢复主面板位置失败: {error}");
                }
            }
        }
        Err(error) => report_startup_error(app, format!("创建主面板失败：{error}")),
    }
}

pub(crate) fn show_main_with_intent(app: &tauri::AppHandle, intent: MainIntent) {
    if let Ok(mut pending) = MAIN_INTENT.lock() {
        *pending = Some(intent);
    }
    show_main(app);
    let _ = app.emit(crate::events::MAIN_INTENT, ());
}

pub(crate) fn handle_window_event(window: &tauri::Window, event: &WindowEvent) {
    if window.label() == MAIN {
        record_main_window_event(window, event);
        if let WindowEvent::ScaleFactorChanged { scale_factor, .. } = event {
            let _ = window.set_icon(tray::icon::taskbar(*scale_factor));
        }
        if let WindowEvent::CloseRequested { api, .. } = event {
            if MAIN_READY.load(Ordering::Acquire) {
                api.prevent_close();
                if let Err(error) = window.hide() {
                    eprintln!("[窗口] 隐藏主面板失败: {error}");
                }
                return;
            }
        }
    }
    if island_clones::is_island_label(window.label()) {
        if let WindowEvent::Focused(false) = event {
            let id = island_clones::id_from_label(window.label()).flatten();
            let _ = window.app_handle().emit(crate::events::ISLAND_BLUR, id);
        }
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Err(error) = window.hide() {
                eprintln!("[窗口] 隐藏灵动岛失败: {error}");
            }
            return;
        }
    }
    if island_clones::is_island_label(window.label())
        && matches!(
            event,
            WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. }
        )
    {
        static PENDING: LazyLock<Mutex<HashSet<String>>> =
            LazyLock::new(|| Mutex::new(HashSet::new()));
        let label = window.label().to_string();
        let queued = PENDING
            .lock()
            .map(|mut set| set.insert(label.clone()))
            .unwrap_or(false);
        if queued {
            let app = window.app_handle().clone();
            let scale_changed = matches!(event, WindowEvent::ScaleFactorChanged { .. });
            tauri::async_runtime::spawn_blocking(move || {
                if let (Some(webview), Some(store)) =
                    (app.get_webview_window(&label), app.try_state::<Cfg>())
                {
                    let id = island_clones::id_from_label(&label).flatten();
                    let cfg = store.0.get();
                    let dock_state = island_dock(&cfg, id.as_deref());
                    let is_capsule = cfg.island_style == "capsule";
                    if dock::DRAGGING.load(Ordering::Relaxed) {
                        let hint = if cfg.dock_enabled {
                            if is_capsule {
                                dock::hover_hint_rect(
                                    &webview,
                                    island_pointer::primary_contour(&label),
                                )
                            } else {
                                dock::hover_hint(&webview)
                            }
                        } else {
                            None
                        };
                        let _ = app.emit(
                            crate::events::DOCK_HINT,
                            DockHintEvent {
                                island: id.clone(),
                                hint,
                            },
                        );
                    } else if dock_state.edge.is_some()
                        && (scale_changed
                            || !dock::monitor_exists(&webview, dock_state.monitor.as_deref()))
                    {
                        let restored = if is_capsule {
                            dock::restore_stage_dock(&webview, &dock_state)
                        } else {
                            dock::restore_dock(&webview, &dock_state)
                        };
                        if let Some(monitor) = restored {
                            store.0.update(|settings| {
                                island_clones::set_dock(
                                    settings,
                                    id.as_deref(),
                                    dock::DockState {
                                        monitor: Some(monitor),
                                        ..dock_state.clone()
                                    },
                                );
                            });
                            crate::events::settings_changed(&app);
                        }
                    }
                }
                if let Ok(mut set) = PENDING.lock() {
                    set.remove(&label);
                }
            });
        }
    }
}

pub(crate) fn handle_run_exit(app: &tauri::AppHandle) {
    if let Ok(placement) = MAIN_PLACEMENT.lock() {
        if let Some(placement) = placement.clone() {
            persist_main_placement(app, placement);
        }
    }
    proxy::shutdown_and_restore(app);
    keep_awake::shutdown();
    session_handoff::shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_board_size_on_large_screens_and_shrinks_on_small_ones() {
        assert_eq!(fit_scale(1920.0, 1040.0), 1.0);
        let scale = fit_scale(1366.0, 728.0);
        assert!(scale < 1.0 && 720.0 * scale <= 728.0 * 0.9 + 0.01);
    }
}
