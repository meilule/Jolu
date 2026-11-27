//! 应用启动编排、数据库与守护线程拉起。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use tauri::{Emitter, Manager};

use crate::app::island::{clones as island_clones, pointer as island_pointer, setup_initial_islands};
use crate::app::state::{
    data_paths, privacy_first_notice, retention_cutoff, with_database, Cfg, Db, DbRead, ReadPool,
    FIRST_INSTALL, MAIN_INTENT, STARTUP_INITIALIZING, STARTUP_READY, STARTUP_REQUESTED,
};
use crate::app::tray::{context_menu, ensure_main_tray};
use crate::app::windows::{link_intent, report_startup_error, show_main, show_startup};
use crate::app::{notify, startup, toast_link, watcher};
use crate::{
    automation, cli_requests, db, keep_awake, net, proxy, settings, updater, validation,
    warmup_run, zcode_version,
};

pub(crate) type CliRequestFn =
    fn(&tauri::AppHandle, &cli_requests::Request) -> cli_requests::Response;

pub(crate) fn start_initialization(app: tauri::AppHandle, data_dir: std::path::PathBuf) {
    if STARTUP_INITIALIZING.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let result = initialize_app(&app, &data_dir);
        STARTUP_INITIALIZING.store(false, Ordering::Release);
        match result {
            Ok(()) => {
                STARTUP_READY.store(true, Ordering::Release);
                crate::events::settings_changed(&app);
                let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
                if STARTUP_REQUESTED.load(Ordering::Acquire) {
                    show_main(&app);
                }
            }
            Err(error) => report_startup_error(&app, error),
        }
    });
}

fn initialize_app(app: &tauri::AppHandle, data_dir: &std::path::Path) -> Result<(), String> {
    if app.try_state::<Db>().is_none() {
        let conn = db::open(&data_dir.join("usage.db")).map_err(|error| error.to_string())?;
        crate::gateway::profiles::load_active(&conn);
        app.manage(Db(Arc::new(Mutex::new(conn))));
    }
    if app.try_state::<DbRead>().is_none() {
        let pool = ReadPool::open(&data_dir.join("usage.db")).map_err(|error| error.to_string())?;
        app.manage(DbRead(Arc::new(pool)));
        automation::register_tick_hook(warmup_run::tick_if_due);
        automation::start(app.clone());
        keep_awake::restore(app.state::<Cfg>().0.get().keep_awake);
    }

    setup_initial_islands(app);
    ensure_main_tray(app)?;

    if !validation::isolated() {
        if app.state::<Cfg>().0.get().island_linkage.agent_hooks {
            if let Ok(exe) = std::env::current_exe() {
                if let Err(err) = crate::agent_hooks::apply_hooks(data_dir, &exe, true) {
                    eprintln!("[Agent Hooks] 启动对齐失败：{err}");
                }
            }
        }
        watcher::start(app.clone());
        watcher::start_config_sync(app.clone());
        watcher::start_agent_hooks_sync(app.clone());
        proxy::startup(app);
    }
    context_menu::prewarm(app);
    island_pointer::start(app.clone());

    if let Some(days) = app.state::<Cfg>().0.get().retention_days {
        if let Ok(cutoff) = retention_cutoff(days) {
            let database = app.state::<Db>().0.clone();
            let cleanup_app = app.clone();
            tauri::async_runtime::spawn(async move {
                match with_database(database, move |conn| {
                    db::cleanup_history(conn, cutoff).map_err(|error| error.to_string())
                })
                .await
                {
                    Ok(result) if result.requests > 0 => {
                        println!("[数据] 自动清理 {} 条过期请求", result.requests);
                        crate::events::refresh_requested(&cleanup_app, None);
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("[数据] 自动清理失败：{error}"),
                }
            });
        }
    }

    Ok(())
}

fn spawn_island_focus_watcher(handle: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("jolu-island-focus".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(30));
            let settings = handle.state::<Cfg>().0.get();
            if !settings.island_linkage.auto_focus || settings.island_clones.is_empty() {
                continue;
            }
            let Some(db) = handle.try_state::<Db>() else {
                continue;
            };
            let latest: std::collections::HashMap<String, i64> = {
                let Ok(conn) = db.0.try_lock() else { continue };
                let mut stmt = match conn.prepare(
                    "SELECT platform, MAX(ts) FROM requests WHERE ts >= ?1 GROUP BY platform",
                ) {
                    Ok(stmt) => stmt,
                    Err(_) => continue,
                };
                let since = chrono::Utc::now().timestamp_millis() - 3_600_000;
                stmt.query_map([since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                    .map(|rows| rows.flatten().collect())
                    .unwrap_or_default()
            };
            let now = chrono::Utc::now().timestamp_millis();
            if let Some(id) =
                island_clones::auto_focus_target(&settings, &latest, now, 120_000, 60_000)
            {
                if handle
                    .state::<Cfg>()
                    .0
                    .try_update(|s| {
                        island_clones::promote(s, &id);
                    })
                    .is_ok()
                {
                    crate::events::settings_changed(&handle);
                }
            }
        })
        .ok();
}

fn spawn_settings_watcher(handle: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("jolu-settings-watch".into())
        .spawn(move || {
            let mut last: Option<(String, String)> = None;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if handle.state::<Cfg>().0.reload_external() {
                    let now = handle.state::<Cfg>().0.get();
                    crate::gateway::mask::set_enabled(now.privacy_masking);
                    crate::gateway::adapter::zcode::set_free_first(now.zcode_free_first);
                    crate::events::settings_changed(&handle);
                }
                let Some(db) = handle.try_state::<Db>() else {
                    continue;
                };
                let signature = {
                    let Ok(conn) = db.0.try_lock() else { continue };
                    let sig = |sql: &str| {
                        conn.query_row(sql, [], |r| r.get::<_, String>(0))
                            .unwrap_or_default()
                    };
                    (
                        sig("SELECT count(*) || ':' || coalesce(max(created_ms), 0) || ':' || coalesce(sum(length(name)), 0) || ':' || coalesce(sum(serve_enabled), 0) FROM connections"),
                        sig("SELECT count(*) || ':' || coalesce(sum(length(members_json) + length(strategy)), 0) FROM routing_groups"),
                    )
                };
                if let Some(previous) = last.as_ref() {
                    if previous.0 != signature.0 {
                        let _ = handle.emit(crate::events::CONNECTIONS_CHANGED, ());
                    }
                    if previous.1 != signature.1 {
                        crate::events::catalog_changed(&handle);
                    }
                }
                last = Some(signature);
            }
        })
        .ok();
}

fn spawn_cli_requests_worker(handle: tauri::AppHandle, cli_handler: CliRequestFn) {
    std::thread::Builder::new()
        .name("jolu-cli-requests".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let data_dir = handle.state::<Cfg>().0.dir().to_path_buf();
            cli_requests::process(&data_dir, &|request| cli_handler(&handle, request));
        })
        .ok();
}

fn spawn_zcode_version_check() {
    std::thread::spawn(|| {
        let check = zcode_version::check(zcode_version::installed_version());
        if check.newer {
            eprintln!(
                "[ZCode] 本机 ZCode {} 比 Jolu 适配的 {} 新：Start Plan 着装常量可能需要更新（见 docs/todo/2026-11-09-zcode-version-follow.md）",
                check.installed.as_deref().unwrap_or("?"),
                check.expected
            );
        }
    });
}

pub(crate) fn setup_app(
    app: &mut tauri::App,
    cli_handler: CliRequestFn,
) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    notify::register_click_handler(&handle)?;
    toast_link::register_protocol();
    if let Some(target) = toast_link::from_args(std::env::args())
        .filter(|target| *target != toast_link::LinkTarget::Main)
    {
        let intent = link_intent(&handle, target);
        if let Ok(mut pending) = MAIN_INTENT.lock() {
            *pending = Some(intent);
        }
    }
    app.manage(updater::UpdateDownloadState::default());
    let automatic = std::env::args_os().any(|arg| arg == "--autostart");
    if !automatic {
        STARTUP_REQUESTED.store(true, Ordering::Release);
        let _ = show_startup(&handle);
    }

    let data_dir = match data_paths(&handle) {
        Ok((path, _)) => path,
        Err(error) => {
            report_startup_error(&handle, error);
            return Ok(());
        }
    };
    let first_install =
        !data_dir.join("settings.json").exists() && !data_dir.join("usage.db").exists();
    FIRST_INSTALL.store(first_install, Ordering::Release);
    app.manage(Cfg(settings::Store::load(&data_dir)));
    net::init(&app.state::<Cfg>().0.get());
    crate::gateway::models_dev::init_from_dir(&data_dir);
    {
        let saved = app.state::<Cfg>().0.get();
        crate::pricing::set_pricing_model_source(&saved.pricing_model_source);
        if saved.models_dev_auto_sync {
            crate::gateway::models_dev::refresh_if_stale(&handle);
        }
    }
    crate::gateway::mask::set_enabled(app.state::<Cfg>().0.get().privacy_masking);
    {
        let saved = app.state::<Cfg>().0.get();
        match crate::gateway::mask::Policy::new(&saved.privacy_actions, &saved.privacy_allow) {
            Ok(policy) => crate::gateway::mask::set_policy(policy),
            Err(reason) => eprintln!("[隐私脱敏] 保存的处理方式无效，按默认全部遮住：{reason}"),
        }
        let handle = app.handle().clone();
        crate::gateway::mask::on_first_mask(move || privacy_first_notice(&handle));
    }
    crate::gateway::adapter::zcode::set_free_first(app.state::<Cfg>().0.get().zcode_free_first);

    spawn_island_focus_watcher(app.handle().clone());
    spawn_settings_watcher(app.handle().clone());
    spawn_cli_requests_worker(app.handle().clone(), cli_handler);
    spawn_zcode_version_check();

    let startup_plan = startup::plan(
        automatic,
        app.state::<Cfg>().0.get().silent_startup,
        first_install,
    );
    STARTUP_REQUESTED.store(startup_plan.show_main, Ordering::Release);
    if startup_plan.show_main {
        let _ = show_startup(&handle);
    }
    start_initialization(handle, data_dir);
    Ok(())
}
