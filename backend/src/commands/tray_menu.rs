//! 托盘摘要窗与右键上下文菜单命令薄壳。

use crate::context_menu::{self, MenuLayout};
use crate::tray_summary::{self, Summary};

#[tauri::command]
pub async fn tray_summary_get(app: tauri::AppHandle) -> Result<Summary, String> {
    tray_summary::tray_summary_get(app).await
}

#[tauri::command]
pub fn tray_summary_fit(app: tauri::AppHandle, width: f64, height: f64) -> Result<(), String> {
    tray_summary::tray_summary_fit(app, width, height)
}

#[tauri::command]
pub fn menu_close(app: tauri::AppHandle) {
    context_menu::menu_close(app)
}

#[tauri::command]
pub fn menu_refreshing() -> bool {
    context_menu::menu_refreshing()
}

#[tauri::command]
pub fn menu_fit(app: tauri::AppHandle, root_height: f64) -> Result<MenuLayout, String> {
    context_menu::menu_fit(app, root_height)
}

#[tauri::command]
pub fn menu_show(app: tauri::AppHandle) -> Result<(), String> {
    context_menu::menu_show(app)
}

#[tauri::command]
pub async fn menu_action(app: tauri::AppHandle, id: String) -> Result<(), String> {
    context_menu::menu_action(app, id).await
}
