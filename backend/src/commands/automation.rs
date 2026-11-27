//! 自动化（模块 E）命令薄壳。

use tauri::AppHandle;

use crate::automation::{
    self, Account, Overview, QuotaVerdict, RunDetail, RunFilter, RunPage, TaskDetail, TaskInput,
    ToolInfo,
};

#[tauri::command]
pub async fn automation_overview(app: AppHandle) -> Result<Overview, String> {
    automation::cmd_overview(app).await
}

#[tauri::command]
pub async fn automation_waiting_connections(app: AppHandle) -> Result<Vec<String>, String> {
    automation::cmd_waiting_connections(app).await
}

#[tauri::command]
pub async fn automation_task(app: AppHandle, id: i64) -> Result<TaskDetail, String> {
    automation::cmd_task(app, id).await
}

#[tauri::command]
pub async fn automation_create(app: AppHandle, input: TaskInput) -> Result<i64, String> {
    automation::cmd_create(app, input).await
}

#[tauri::command]
pub async fn automation_update(app: AppHandle, id: i64, input: TaskInput) -> Result<(), String> {
    automation::cmd_update(app, id, input).await
}

#[tauri::command]
pub async fn automation_delete(app: AppHandle, task_id: i64) -> Result<(), String> {
    automation::cmd_delete(app, task_id).await
}

#[tauri::command]
pub async fn automation_action(app: AppHandle, id: i64, action: String) -> Result<(), String> {
    automation::cmd_action(app, id, action).await
}

#[tauri::command]
pub async fn automation_runs(
    app: AppHandle,
    filter: RunFilter,
    page: u32,
) -> Result<RunPage, String> {
    automation::cmd_runs(app, filter, page).await
}

#[tauri::command]
pub async fn automation_run(app: AppHandle, id: i64) -> Result<RunDetail, String> {
    automation::cmd_run(app, id).await
}

#[tauri::command]
pub async fn automation_accounts(
    app: AppHandle,
    platform: String,
) -> Result<Vec<Account>, String> {
    let db = automation::shared(&app);
    tauri::async_runtime::spawn_blocking(move || automation::accounts_for(&db, &platform))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn automation_models(
    app: AppHandle,
    platform: String,
    connection_id: Option<String>,
) -> Result<Vec<String>, String> {
    let db = automation::shared(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        automation::models_for(&conn, &platform, connection_id.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn automation_quota(
    app: AppHandle,
    connection_id: String,
    force: bool,
) -> Result<QuotaVerdict, String> {
    automation::cmd_quota(app, connection_id, force).await
}

#[tauri::command]
pub async fn automation_skills(
    _app: AppHandle,
    platform: String,
    cwd: Option<String>,
) -> Result<Vec<crate::skills::SkillOption>, String> {
    automation::cmd_skills(platform, cwd).await
}

#[tauri::command]
pub async fn automation_tools() -> Vec<ToolInfo> {
    automation::tools_list()
}

#[tauri::command]
pub async fn automation_pick_dir(app: AppHandle) -> Result<Option<String>, String> {
    automation::cmd_pick_dir(app).await
}
