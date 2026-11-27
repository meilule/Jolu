//! 会话交接（模块 D 交接弹窗）命令薄壳。

use std::path::Path;
use tauri::AppHandle;

use crate::handoff::{self, FileNameCheck, HandoffResult, HandoffState};

#[tauri::command]
pub async fn handoff_state(
    app: AppHandle,
    platform: String,
    session_id: String,
    focus: Option<String>,
) -> Result<HandoffState, String> {
    handoff::cmd_state(app, platform, session_id, focus).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn handoff_start(
    app: AppHandle,
    platform: String,
    session_id: String,
    connection_id: String,
    model: Option<String>,
    focus: String,
    target: String,
    channel: Option<String>,
) -> Result<String, String> {
    handoff::cmd_start(
        app,
        platform,
        session_id,
        connection_id,
        model,
        focus,
        target,
        channel,
    )
    .await
}

#[tauri::command]
pub async fn handoff_refine(
    app: AppHandle,
    job_id: String,
    instruction: String,
) -> Result<(), String> {
    handoff::cmd_refine(app, job_id, instruction).await
}

#[tauri::command]
pub async fn handoff_estimate_refine(job_id: String, instruction: String) -> Result<usize, String> {
    handoff::cmd_estimate_refine(job_id, instruction).await
}

#[tauri::command]
pub async fn handoff_finish(
    app: AppHandle,
    job_id: String,
    round: u32,
    project: String,
) -> Result<HandoffResult, String> {
    handoff::cmd_finish(app, job_id, round, project).await
}

#[tauri::command]
pub async fn handoff_cancel(job_id: String) -> Result<(), String> {
    handoff::cmd_cancel(job_id).await
}

#[tauri::command]
pub async fn handoff_local_export(
    app: AppHandle,
    platform: String,
    session_id: String,
    project: String,
    topic: String,
    target: String,
) -> Result<String, String> {
    handoff::cmd_local_export(app, platform, session_id, project, topic, target).await
}

#[tauri::command]
pub async fn handoff_render(
    app: AppHandle,
    platform: String,
    session_id: String,
    project: String,
    topic: String,
    target: String,
) -> Result<String, String> {
    handoff::cmd_render(app, platform, session_id, project, topic, target).await
}

#[tauri::command]
pub async fn validate_file_name(
    file_name: String,
    dir: Option<String>,
) -> Result<FileNameCheck, String> {
    tauri::async_runtime::spawn_blocking(move || {
        handoff::file_name_check(&file_name, dir.as_deref().map(Path::new))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn handoff_save(
    app: AppHandle,
    markdown: String,
    file_name: String,
    prefer_project_dir: Option<String>,
) -> Result<Option<String>, String> {
    handoff::cmd_save(app, markdown, file_name, prefer_project_dir).await
}
