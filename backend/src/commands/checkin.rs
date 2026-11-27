//! 一键签到（模块 B2）命令薄壳。

use tauri::WebviewWindow;

use crate::daily_checkin::{self, Overview, Update};

#[tauri::command]
pub(crate) async fn daily_checkin_overview(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> Result<Overview, String> {
    daily_checkin::daily_checkin_overview(app, window).await
}

#[tauri::command]
pub(crate) async fn daily_checkin_status(
    app: tauri::AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<Update, String> {
    daily_checkin::daily_checkin_status(app, window, id).await
}

#[tauri::command]
pub(crate) async fn daily_checkin_claim(
    app: tauri::AppHandle,
    window: WebviewWindow,
    id: String,
    expected_account_key: String,
) -> Result<Update, String> {
    daily_checkin::daily_checkin_claim(app, window, id, expected_account_key).await
}
