//! Jolu —— Tauri 应用入口与顶层领域模块导出。
//!
//! 职责边界见需求文档 §7.7：
//!   Tauri 2 + Rust —— 窗口与托盘、只读网络查询、本地增量采集、去重、
//!                     统计计算与数据库查询；集中管理必要后台任务。

pub(crate) mod agents;
pub(crate) mod app;
pub(crate) mod auth;
pub(crate) mod automation;
pub(crate) mod checkin;
pub(crate) mod collectors;
pub(crate) mod commands;
pub mod db;
pub mod gateway;
pub(crate) mod handoff;
pub(crate) mod infra;
pub mod library;
pub(crate) mod ops;
#[cfg(test)]
mod perf_bench;
pub(crate) mod platforms;
pub(crate) mod pricing;
pub(crate) mod proxy;
pub(crate) mod quota;
pub mod service;
pub(crate) mod sessions;
pub(crate) mod settings;
pub(crate) mod skills;
pub(crate) mod usage;

// ── 多二进制公共 API 导出（main.rs / jolu_cli / handoff_eval） ──
pub use app::state::{Cfg, Db, DbRead, ReadPool};
#[cfg(all(debug_assertions, feature = "handoff-eval"))]
pub use handoff::eval as handoff_eval;
pub use ops::{cli_requests, tune};

// ── crate 内部模块路径兼容别名 ──
pub(crate) use agents::{
    custom as custom_agents, discovery as agent_discovery, hooks as agent_hooks,
    version as agent_version, wsl_probe, zcode_version,
};
pub(crate) use app::island::{clones as island_clones, dock, pointer as island_pointer};
pub(crate) use app::state::{
    cached_value, current_for_identity, emit_collection_status, invalidate_network_cache,
    platform_name, with_database, with_reader, CacheBackoff, MainIntent, QueryCache,
    API_USAGE_CACHE, BALANCE_CACHE, MAIN, MAIN_READY, QUOTA_CACHE, TRAY_REFRESHING,
};
pub(crate) use app::tray::{
    context_menu, handle_tray_menu, refresh_tray_menu, summary as tray_summary, update_tray_icon,
};
pub(crate) use app::windows::{
    activate_existing_instance, activate_with_link, show_main, show_main_with_intent,
};
pub(crate) use app::{
    background::start_initialization, events, main_window, notify, startup, toast_link,
};
pub(crate) use auth::browser::{
    self as provider_browser_auth, cli as cli_browser_auth, grok as grok_browser_auth,
    trae as trae_browser_auth, workbuddy as workbuddy_browser_auth, zcode as zcode_browser_auth,
};
pub(crate) use auth::{
    agent_configs, auth_snapshot, catalog_backfill, claude_auth, cli_apply, cli_switch,
    connections, creds, current_connection, local_active, local_enroll, login_detect, opencode,
    provider_key, qoder_desktop,
};
pub(crate) use checkin::vendors::{
    qoder as checkin_qoder, trae as checkin_trae, workbuddy as checkin_workbuddy,
    zcode as checkin_zcode,
};
pub(crate) use checkin::{self as daily_checkin, stats as checkin_stats, types as checkin_types};
pub(crate) use collectors::{
    antigravity as antigravity_usage, claude_codex as collector, dsh as dsh_sessions,
    kimi as kimi_sessions, native as native_sources, opencode as opencode_usage, pi as pi_sessions,
    qoder as qoder_usage, qwen as qwen_sessions,
};
#[allow(unused_imports)]
pub(crate) use handoff::{
    self as session_handoff, baseline as handoff_baseline, digest as handoff_digest,
    facts as handoff_facts, input as handoff_input,
};
pub(crate) use infra::{filelock, net, proc, stall, steady, updater, validation};
pub(crate) use ops::{data_files, keep_awake, storage_stats, warmup, warmup_run};
pub(crate) use pricing::fx;
pub(crate) use proxy::config as proxy_config;
pub(crate) use quota::vendors::{
    gemini as gemini_quota, grok as grok_quota, ollama as ollama_quota, qoder as qoder_quota,
    trae as trae_quota, workbuddy as workbuddy_quota, zcode as zcode_quota,
};
pub(crate) use quota::{
    coding_plan, gateway_balance, registry as quota_registry, service_balance,
};
pub(crate) use sessions as session_threads;
pub(crate) use usage::{
    attribution, data_contract, otlp, session_presence, session_titles, source_store,
    stats as usage_stats,
};

pub fn restore_agents_headless() -> i32 {
    let (directory, database) = match crate::service::paths::locate() {
        Ok(paths) => paths,
        Err(reason) => {
            eprintln!("[卸载] {reason}");
            return 0;
        }
    };
    let settings = crate::service::settings::read_settings(&directory);
    let port = if settings.proxy_port > 0 {
        settings.proxy_port
    } else {
        12731
    };
    let mut conn = match crate::db::open(&database) {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("[卸载] 打开数据库失败：{error}");
            return 1;
        }
    };
    let report = crate::service::gateway::wiring_restore::restore_all(&mut conn, port);
    for agent in &report.restored {
        eprintln!("[卸载] 已还原 {agent}");
    }
    for line in &report.failed {
        eprintln!("[卸载] 失败：{line}");
    }
    let mut library_lines = crate::library::restore_all_library(&conn);
    library_lines.extend(crate::library::shared::restore_all(&conn));
    library_lines.extend(crate::gateway::wiring_wsl::restore_all(
        &conn,
        &directory.join("gateway-backups"),
    ));
    library_lines.extend(crate::agents::hooks::restore_all(&directory));
    for line in &library_lines {
        eprintln!("[卸载] {line}");
    }
    let failed =
        report.failed.len() + library_lines.iter().filter(|l| l.starts_with("失败")).count();
    if failed == 0 { 0 } else { 1 }
}

pub fn run_agent_hook() -> i32 {
    let Ok((directory, _)) = crate::service::paths::locate() else {
        return 0;
    };
    agents::hooks::run_agent_hook_cli(&directory)
}

pub fn run_mcp_search() {
    use std::io::{BufRead, Write};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("搜索 MCP 需要一个小异步运行时");
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Some(response) = runtime.block_on(library::mcp_search::handle_line(&line)) else {
            continue;
        };
        let mut stdout = std::io::stdout();
        if writeln!(stdout, "{response}")
            .and_then(|_| stdout.flush())
            .is_err()
        {
            break;
        }
    }
}

fn spawn_quota_refresh(app: &tauri::AppHandle, connection_id: &str) {
    let app = app.clone();
    let connection_id = connection_id.to_string();
    tauri::async_runtime::spawn(async move {
        if let Err(error) =
            commands::quotas::query_connection_quota(app, connection_id.clone(), false).await
        {
            eprintln!("[路由分组] 后台补查「{connection_id}」额度失败：{error}");
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    proxy::register_quota_refresher(spawn_quota_refresh);
    automation::register_quota_querier(|app, id, force| {
        Box::pin(commands::quotas::query_connection_quota(app, id, force))
    });

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            match toast_link::from_args(&args) {
                Some(target) => activate_with_link(app, target),
                None => activate_existing_instance(app),
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(commands::invoke_handler())
        .setup(|app| app::background::setup_app(app, commands::routing::handle_cli_request))
        .on_window_event(app::windows::handle_window_event)
        .build(tauri::generate_context!())
        .expect("error while building Jolu");

    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit { .. }) {
            app::windows::handle_run_exit(app);
        }
    });
}
