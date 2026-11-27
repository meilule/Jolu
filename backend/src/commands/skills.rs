//! Skills 管理（模块 F）命令薄壳。

use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::app::state::{DbRead, MAIN};
use crate::skills::{
    blocking, cleanup_checkout, config_guard, context, context_mut, copy_with, data_dir,
    detail_with, file_with, git_temp_root, homes, ignore_project_with, import_git_with,
    import_scan_with, import_with, load_config, locate, overview_with, remove_with, save_config,
    save_dir_settings_with, save_with, set_enabled_impl, set_global_dir_with, set_projects_with,
    skill_name, CopyResult, Detail, DirSettings, FileContent, GitImportScan, ImportChoice,
    ImportResult, ImportScan, Overview, SaveInput, SaveResult, ScanError, Scope,
};

#[tauri::command]
pub async fn skills_overview(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    platform: String,
    force: bool,
) -> Result<Overview, String> {
    let (dir, cfg) = context(&app, &db, force).await?;
    blocking(move || {
        let mut overview = overview_with(&homes()?, &cfg, &platform)?;
        if let Err(reason) = config_guard(&dir) {
            overview.errors.insert(
                0,
                ScanError {
                    path: dir.join("skills.json").display().to_string(),
                    reason,
                },
            );
        }
        Ok(overview)
    })
    .await
}

#[tauri::command]
pub async fn skill_detail(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    path: String,
) -> Result<Detail, String> {
    let (_, cfg) = context(&app, &db, false).await?;
    blocking(move || detail_with(&homes()?, &cfg, &path)).await
}

#[tauri::command]
pub async fn skill_file(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    path: String,
    rel: String,
) -> Result<FileContent, String> {
    let (_, cfg) = context(&app, &db, false).await?;
    blocking(move || file_with(&homes()?, &cfg, &path, &rel)).await
}

#[tauri::command]
pub async fn skill_set_enabled(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    path: String,
    enabled: bool,
) -> Result<String, String> {
    let (dir, mut cfg) = context_mut(&app, &db).await?;
    blocking(move || {
        let h = homes()?;
        let (root, skill_dir) = locate(&h, &cfg, &path)?;
        let name = skill_name(&skill_dir);
        set_enabled_impl(&h, &mut cfg, &root, &skill_dir, &name, enabled, &dir)
    })
    .await
}

#[tauri::command]
pub async fn skill_save(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    input: SaveInput,
) -> Result<SaveResult, String> {
    let (dir, mut cfg) = context_mut(&app, &db).await?;
    blocking(move || save_with(&homes()?, &mut cfg, input, &dir)).await
}

#[tauri::command]
pub async fn skill_copy(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    path: String,
    platform: String,
    scope: Scope,
    project: Option<String>,
    dry_run: bool,
) -> Result<CopyResult, String> {
    let (_, cfg) = if dry_run {
        context(&app, &db, false).await?
    } else {
        context_mut(&app, &db).await?
    };
    blocking(move || {
        copy_with(
            &homes()?,
            &cfg,
            &path,
            &platform,
            scope,
            project.as_deref(),
            dry_run,
        )
    })
    .await
}

#[tauri::command]
pub async fn skill_remove(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    path: String,
) -> Result<String, String> {
    let (dir, mut cfg) = context_mut(&app, &db).await?;
    blocking(move || remove_with(&homes()?, &mut cfg, &path, &dir)).await
}

#[tauri::command]
pub async fn skills_import_scan(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    source: String,
    platform: String,
    scope: Scope,
    project: Option<String>,
) -> Result<ImportScan, String> {
    let (_, cfg) = context(&app, &db, false).await?;
    blocking(move || {
        import_scan_with(&homes()?, &cfg, &source, &platform, scope, project.as_deref())
    })
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn skills_import(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    source: String,
    platform: String,
    scope: Scope,
    project: Option<String>,
    choices: Vec<ImportChoice>,
    enable: bool,
) -> Result<ImportResult, String> {
    let (dir, mut cfg) = context_mut(&app, &db).await?;
    blocking(move || {
        let result = import_with(
            &homes()?,
            &mut cfg,
            &source,
            &platform,
            scope,
            project.as_deref(),
            choices,
            enable,
            &dir,
        )?;
        save_config(&dir, &cfg)?;
        Ok(result)
    })
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn skills_import_git(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    url: String,
    branch: Option<String>,
    platform: String,
    scope: Scope,
    project: Option<String>,
) -> Result<GitImportScan, String> {
    let (_, cfg) = context(&app, &db, false).await?;
    blocking(move || {
        import_git_with(
            &homes()?,
            &cfg,
            &git_temp_root(),
            "git",
            &url,
            branch.as_deref(),
            &platform,
            scope,
            project.as_deref(),
        )
    })
    .await
}

#[tauri::command]
pub async fn skills_import_git_cleanup(checkout_dir: String) -> Result<(), String> {
    blocking(move || cleanup_checkout(&git_temp_root(), &checkout_dir)).await
}

#[tauri::command]
pub async fn skills_set_projects(
    app: tauri::AppHandle,
    projects: Vec<String>,
) -> Result<Vec<String>, String> {
    let dir = data_dir(&app)?;
    blocking(move || {
        let mut cfg = load_config(&dir);
        let list = set_projects_with(&mut cfg, projects)?;
        save_config(&dir, &cfg)?;
        Ok(list)
    })
    .await
}

#[tauri::command]
pub async fn skills_ignore_project(
    app: tauri::AppHandle,
    path: String,
    ignored: bool,
) -> Result<(), String> {
    let dir = data_dir(&app)?;
    blocking(move || {
        let mut cfg = load_config(&dir);
        ignore_project_with(&mut cfg, &path, ignored);
        save_config(&dir, &cfg)
    })
    .await
}

#[tauri::command]
pub async fn skills_set_global_dir(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    platform: String,
    path: Option<String>,
) -> Result<Overview, String> {
    let (dir, mut cfg) = context_mut(&app, &db).await?;
    blocking(move || {
        let h = homes()?;
        set_global_dir_with(&h, &mut cfg, &platform, path.as_deref())?;
        save_config(&dir, &cfg)?;
        overview_with(&h, &cfg, &platform)
    })
    .await
}

#[tauri::command]
pub async fn skills_save_dir_settings(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    input: DirSettings,
) -> Result<(), String> {
    let (dir, mut cfg) = context_mut(&app, &db).await?;
    blocking(move || {
        save_dir_settings_with(&homes()?, &mut cfg, input)?;
        save_config(&dir, &cfg)
    })
    .await
}

#[tauri::command]
pub async fn skills_pick_folder(
    app: tauri::AppHandle,
    title: String,
) -> Result<Option<String>, String> {
    let window = app.get_webview_window(MAIN).ok_or("请从主面板选择目录")?;
    let dialog = app.dialog().file().set_parent(&window).set_title(title);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
        .await
        .map_err(|e| e.to_string())?;
    match picked {
        None => Ok(None),
        Some(path) => path
            .into_path()
            .map(|p| Some(p.display().to_string()))
            .map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub async fn skill_open_dir(
    app: tauri::AppHandle,
    db: State<'_, DbRead>,
    path: String,
) -> Result<(), String> {
    let (_, cfg) = context(&app, &db, false).await?;
    let target = blocking(move || locate(&homes()?, &cfg, &path).map(|(_, d)| d)).await?;
    app.opener()
        .open_path(target.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}
