//! 检查更新与下载安装命令薄壳。

use tauri::{AppHandle, State};

use crate::settings::Settings;
use crate::updater::{self, UpdateCheck, UpdateDownloadState};

#[tauri::command]
pub fn set_auto_check_updates(app: AppHandle, on: bool) -> Result<Settings, String> {
    updater::set_auto_check_updates(app, on)
}

#[tauri::command]
pub fn set_update_interval(app: AppHandle, minutes: u32) -> Result<Settings, String> {
    updater::set_update_interval(app, minutes)
}

#[tauri::command]
pub async fn check_app_update_available(app: AppHandle) -> Result<UpdateCheck, String> {
    updater::check_app_update_available(app).await
}

#[tauri::command]
pub async fn download_app_update(
    app: AppHandle,
    state: State<'_, UpdateDownloadState>,
) -> Result<UpdateCheck, String> {
    updater::download_app_update(app, state).await
}

#[tauri::command]
pub fn cancel_app_update_download(state: State<'_, UpdateDownloadState>) -> Result<(), String> {
    updater::cancel_app_update_download(state)
}

#[tauri::command]
pub async fn install_downloaded_update_and_restart(app: AppHandle) -> Result<(), String> {
    updater::install_downloaded_update_and_restart(app).await
}
