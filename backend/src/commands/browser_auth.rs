//! 官方浏览器授权命令薄壳。

use tauri::WebviewWindow;

use crate::provider_browser_auth::{self, Status};

#[tauri::command]
pub(crate) async fn start_browser_authorization(
    app: tauri::AppHandle,
    window: WebviewWindow,
    platform: String,
    edition: String,
) -> Result<String, String> {
    provider_browser_auth::start_browser_authorization(app, window, platform, edition).await
}

#[tauri::command]
pub(crate) async fn browser_authorization_status(
    window: WebviewWindow,
    id: String,
) -> Result<Status, String> {
    provider_browser_auth::browser_authorization_status(window, id).await
}

#[tauri::command]
pub(crate) async fn cancel_browser_authorization(
    window: WebviewWindow,
    id: String,
) -> Result<(), String> {
    provider_browser_auth::cancel_browser_authorization(window, id).await
}

#[tauri::command]
pub(crate) async fn save_browser_connection(
    app: tauri::AppHandle,
    window: WebviewWindow,
    id: String,
    name: Option<String>,
) -> Result<String, String> {
    provider_browser_auth::save_browser_connection(app, window, id, name).await
}
