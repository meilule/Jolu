//! 主窗口、启动与页面跳转。

#[allow(unused_imports)]
use crate::*;

#[tauri::command]
pub(crate) fn take_main_intent() -> Option<MainIntent> {
    MAIN_INTENT.lock().ok().and_then(|mut intent| intent.take())
}

/// 灵动岛上的快捷跳转：打开主窗口并切到对应页面。只接受已知页面。
#[tauri::command]
pub(crate) fn open_main_section(app: tauri::AppHandle, section: String) -> Result<(), String> {
    if !matches!(section.as_str(), "gateway" | "island" | "connections" | "checkin" | "sessions" | "automation") {
        return Err(format!("未知的主窗口页面：{section}"));
    }
    show_main_with_intent(&app, MainIntent::Settings { section });
    Ok(())
}

/// 灵动岛点会话：主窗口打开会话线程并定位（会话已删时会话页自己回到列表）。
#[tauri::command]
pub(crate) fn open_main_session(app: tauri::AppHandle, platform: String, session_id: String) -> Result<(), String> {
    let (platform, session_id) = (platform.trim().to_string(), session_id.trim().to_string());
    if platform.is_empty() || session_id.is_empty() {
        return Err("缺少要打开的会话".into());
    }
    show_main_with_intent(&app, MainIntent::Sessions { platform, session_id });
    Ok(())
}

/// 灵动岛「查看用量」：主窗口打开用量统计，按今天与给定的平台 / 连接预筛（连接已删时只带平台）。
#[tauri::command]
pub(crate) fn open_main_usage(app: tauri::AppHandle, platform: Option<String>, connection_id: Option<String>) -> Result<(), String> {
    let clean = |value: Option<String>| value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    show_main_with_intent(&app, MainIntent::Usage { platform: clean(platform), connection_id: clean(connection_id) });
    Ok(())
}

/// 灵动岛快捷动作「建自动化」（R2-I-06）：打开主窗口自动化页并唤起新建任务弹窗。
#[tauri::command]
pub(crate) fn open_main_automation_new(app: tauri::AppHandle, platform: Option<String>, session_id: Option<String>) -> Result<(), String> {
    let clean = |value: Option<String>| value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    show_main_with_intent(&app, MainIntent::AutomationNew { platform: clean(platform), session_id: clean(session_id) });
    Ok(())
}

// 任务 2.1 开始
#[tauri::command]
pub(crate) fn set_default_page(app: tauri::AppHandle, page: String) -> Result<settings::Settings, String> {
    settings::validate_default_page(&page)?;
    let next = app.state::<Cfg>().0.try_update(|settings| settings.default_page = page)?;
    crate::events::settings_changed(&app);
    Ok(next)
}

/// 只记录上次访问的页面；切页很频繁，不广播 settings-changed，托盘打开时读快照即可。
#[tauri::command]
pub(crate) fn remember_page(app: tauri::AppHandle, page: String) -> Result<(), String> {
    settings::validate_last_page(&page)?;
    app.state::<Cfg>().0.try_update(|settings| settings.last_page = Some(page))?;
    Ok(())
}

#[tauri::command]
pub(crate) fn main_drag(app: tauri::AppHandle) -> Result<(), String> {
    let window = app.get_webview_window(MAIN).ok_or("找不到主面板")?;
    window.start_dragging().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn startup_error() -> Option<String> {
    STARTUP_ERROR.lock().ok().and_then(|current| current.clone())
}

#[tauri::command]
pub(crate) fn quit_startup(app: tauri::AppHandle) { app.exit(1); }

#[tauri::command]
pub(crate) fn retry_startup(app: tauri::AppHandle) -> Result<&'static str, String> {
    if STARTUP_INITIALIZING.load(Ordering::Acquire) {
        return Ok("initializing");
    }
    if let Ok(mut current) = STARTUP_ERROR.lock() { *current = None; }
    if STARTUP_READY.load(Ordering::Acquire) {
        if !MAIN_READY.load(Ordering::Acquire) {
            if let Some(window) = app.get_webview_window(MAIN) {
                window.destroy().map_err(|error| error.to_string())?;
            }
            show_main(&app);
        } else {
            show_main(&app);
        }
    } else {
        let (data_dir, _) = data_paths(&app)?;
        start_initialization(app, data_dir);
    }
    Ok("restarting")
}

#[tauri::command]
pub(crate) fn main_panel_ready(app: tauri::AppHandle) -> Result<(), String> {
    let window = app.get_webview_window(MAIN).ok_or("主面板窗口不存在")?;
    window.show().map_err(|error| error.to_string())?;
    window.unminimize().map_err(|error| error.to_string())?;
    let captured = main_window::capture(&window);
    if let Ok(mut placement) = MAIN_PLACEMENT.lock() {
        *placement = captured;
    }
    MAIN_READY.store(true, Ordering::Release);
    let _ = window.set_focus();
    if let Some(splash) = app.get_webview_window(STARTUP) { let _ = splash.close(); }
    Ok(())
}

// 任务 2.2 开始
/// 界面缩放只作用于 `main` 窗口（D-21）；灵动岛、菜单等窗口各自独立，保持原大小。
pub(crate) fn apply_main_zoom(window: &WebviewWindow, zoom: u16) -> Result<(), String> {
    window.set_zoom(f64::from(zoom) / 100.0).map_err(|error| {
        eprintln!("[窗口] 设置主面板缩放失败: {error}");
        error.to_string()
    })
}

/// 外观（画板 88）：界面缩放、列表密度、减少动效一起保存；主窗口已打开时立即应用缩放。
#[tauri::command]
pub(crate) fn set_appearance_preferences(app: tauri::AppHandle, zoom: u16, density: String, reduce_motion: bool) -> Result<settings::Settings, String> {
    settings::validate_appearance(zoom, &density)?;
    let next = app.state::<Cfg>().0.try_update(|settings| {
        settings.main_zoom = zoom;
        settings.list_density = density;
        settings.reduce_motion = reduce_motion;
    })?;
    crate::events::settings_changed(&app);
    if let Some(window) = app.get_webview_window(MAIN) {
        apply_main_zoom(&window, zoom).map_err(|error| format!("界面缩放已保存，但应用到主窗口失败：{error}"))?;
    }
    Ok(next)
}
