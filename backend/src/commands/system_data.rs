//! 数据导入导出、存储计数、联网诊断与应用内通知命令薄壳。

use tauri::{AppHandle, State};

use crate::app::state::{Db, DbRead};
use crate::data_files::{self, CsvExport};
use crate::db::{ImportPreview, ImportSummary};
use crate::net::{self, NetworkDiagnosis};
use crate::notify::{self, AppNotice};
use crate::settings::Settings;
use crate::storage_stats::{self, StorageCounts};
use crate::usage_stats::StatsFilter;

#[tauri::command]
pub async fn preview_import_file(
    app: AppHandle,
    db: State<'_, DbRead>,
) -> Result<Option<ImportPreview>, String> {
    data_files::preview_import_file(app, db).await
}

#[tauri::command]
pub fn discard_import(token: String) -> Result<(), String> {
    data_files::discard_import(token)
}

#[tauri::command]
pub async fn confirm_import(
    app: AppHandle,
    db: State<'_, Db>,
    token: String,
) -> Result<ImportSummary, String> {
    data_files::confirm_import(app, db, token).await
}

#[tauri::command]
pub async fn export_data_file(
    app: AppHandle,
    db: State<'_, DbRead>,
    platform: Option<String>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<Option<String>, String> {
    data_files::export_data_file(app, db, platform, start_ms, end_ms).await
}

#[tauri::command]
pub async fn export_calls_csv(
    app: AppHandle,
    db: State<'_, DbRead>,
    filter: StatsFilter,
) -> Result<Option<CsvExport>, String> {
    data_files::export_calls_csv(app, db, filter).await
}

#[tauri::command]
pub async fn storage_counts(db: State<'_, DbRead>) -> Result<StorageCounts, String> {
    storage_stats::storage_counts(db).await
}

#[tauri::command]
pub fn set_network(
    app: AppHandle,
    mode: String,
    proxy_url: Option<String>,
) -> Result<Settings, String> {
    net::set_network(app, mode, proxy_url)
}

#[tauri::command]
pub async fn diagnose_network(
    app: AppHandle,
    mode: String,
    proxy_url: Option<String>,
) -> Result<NetworkDiagnosis, String> {
    net::diagnose_network(app, mode, proxy_url).await
}

#[tauri::command]
pub fn take_app_notices(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Vec<AppNotice>, String> {
    notify::take_app_notices(app, window)
}
