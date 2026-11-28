//! Tauri 命令层：仅负责提取状态、转发到 `service/` 或领域层并触发前端广播事件。
//!
//! 禁止在任何命令模块中使用 `use crate::*;`。

pub(crate) mod agents;
pub(crate) mod capabilities;
pub(crate) mod gateway_access;
pub(crate) mod island_window;
pub(crate) mod panel;
pub(crate) mod preferences;
pub(crate) mod pricing;
pub(crate) mod providers;
pub(crate) mod quotas;
pub(crate) mod routing;
pub(crate) mod usage_sessions;
pub(crate) mod automation;
pub(crate) mod handoff;
pub(crate) mod skills;
pub(crate) mod browser_auth;
pub(crate) mod checkin;
pub(crate) mod system_data;
pub(crate) mod tray_menu;
pub(crate) mod updater;

pub(crate) fn invoke_handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    let handler: fn(tauri::ipc::Invoke) -> bool = tauri::generate_handler![
        tray_menu::tray_summary_get, tray_menu::tray_summary_fit, usage_sessions::scan_local_sessions,
        usage_sessions::collection_status, agents::agent_versions, agents::agent_discover,
        agents::wsl_probe_agents, agents::wsl_wiring_state, agents::tune_advice,
        agents::wsl_wire, agents::wsl_unwire, usage_sessions::otlp_export_usage,
        preferences::data_info, preferences::open_data_directory, preferences::autostart_status,
        preferences::set_autostart, preferences::set_silent_startup, capabilities::library_install,
        capabilities::library_remove, capabilities::library_list, capabilities::library_import_preview,
        capabilities::library_shared_state, capabilities::library_instructions_save, capabilities::library_subagent_save,
        capabilities::library_subagent_remove, agents::gateway_wiring_applied, agents::gateway_wiring_preflight,
        agents::gateway_wiring_preview, agents::gateway_long_context_set, gateway_access::gateway_client_key_limits_set,
        gateway_access::gateway_models_dev_refresh, gateway_access::gateway_models_dev_status, gateway_access::gateway_allow_ips_set,
        gateway_access::gateway_allow_ips_from_env, gateway_access::gateway_client_key_concurrency_set, capabilities::search_vendors,
        capabilities::search_apis_set, capabilities::search_try, preferences::set_keep_awake,
        preferences::set_warmup, preferences::warmup_last_outcome, preferences::set_otlp_endpoint,
        island_window::resize_island, island_window::set_island_hit_region, island_window::island_set_expanded,
        island_window::island_active_accounts, island_window::island_agent_hooks_state, island_window::island_enroll_local_account,
        usage_sessions::live_usage, usage_sessions::island_live_overview, usage_sessions::connection_kind, usage_sessions::source_metrics,
        providers::list_connections, checkin::daily_checkin_overview, checkin::daily_checkin_status,
        checkin::daily_checkin_claim, providers::add_connection, providers::update_connection,
        providers::enable_connection, providers::remove_connection, providers::set_connection_paused,
        providers::fetch_credentials, providers::replace_api_key, providers::rename_connection,
        providers::discover_connections, providers::detect_local_login, providers::current_connections,
        providers::read_local_connections, system_data::export_data_file, system_data::export_calls_csv,
        usage_sessions::stats_overview, usage_sessions::island_source_breakdown, usage_sessions::stats_activity,
        usage_sessions::stats_calls, pricing::pricing_catalog, pricing::pricing_models_dev_catalog,
        pricing::pricing_upsert_override, pricing::pricing_delete_override, pricing::pricing_disable_override,
        pricing::pricing_upsert_alias, pricing::pricing_delete_alias, pricing::pricing_set_rules,
        pricing::pricing_refresh_models_dev, pricing::pricing_budget_status, pricing::pricing_save_budgets,
        pricing::connection_update_billing, usage_sessions::sessions_list, usage_sessions::session_messages,
        usage_sessions::session_continue_check, usage_sessions::session_continue, automation::automation_overview,
        automation::automation_waiting_connections, automation::automation_task, automation::automation_create,
        automation::automation_update, automation::automation_action, automation::automation_runs,
        automation::automation_run, automation::automation_accounts, automation::automation_models,
        automation::automation_quota, automation::automation_tools, automation::automation_pick_dir,
        automation::automation_delete, skills::skills_overview, skills::skill_detail,
        skills::skill_file, skills::skill_set_enabled, skills::skill_save,
        skills::skill_copy, skills::skill_remove, skills::skills_import_scan,
        skills::skills_import, skills::skills_set_projects, skills::skills_ignore_project,
        skills::skills_pick_folder, skills::skill_open_dir, system_data::storage_counts,
        preferences::get_settings, island_window::set_island_platform, island_window::set_island_source,
        island_window::set_island_connection, preferences::set_dnd, preferences::set_proxy,
        preferences::proxy_status, routing::gateway_status, gateway_access::gateway_access_key,
        gateway_access::gateway_copy_key, gateway_access::gateway_shared_lan_set, gateway_access::gateway_client_keys_list,
        gateway_access::gateway_client_key_issue, gateway_access::gateway_client_key_revoke, gateway_access::gateway_middleware_list,
        gateway_access::zcode_version_check, gateway_access::gateway_middleware_save, gateway_access::gateway_middleware_delete,
        preferences::fx_rate, preferences::set_cost_currency, routing::gateway_set_mode,
        routing::gateway_suggest_port, routing::gateway_set_port,
        routing::gateway_set_default_source, providers::gateway_refresh_models, gateway_access::zcode_set_channel,
        routing::gateway_routing_group_list, routing::gateway_routing_group_create, routing::gateway_routing_group_update,
        routing::gateway_routing_group_delete, providers::providers_import_scan, providers::providers_import_apply,
        routing::gateway_profiles_list, routing::gateway_profiles_save, routing::gateway_profiles_delete,
        routing::gateway_profiles_apply, routing::gateway_set_serve, routing::gateway_set_serve_many,
        routing::gateway_source_usage, routing::gateway_source_capabilities, agents::gateway_wiring_set,
        agents::gateway_escape, capabilities::privacy_masking_set, capabilities::privacy_policy_set,
        capabilities::privacy_records, capabilities::privacy_records_clear, gateway_access::zcode_free_first_set,
        routing::gateway_groups, routing::gateway_group_create, routing::gateway_group_rename,
        routing::gateway_group_delete, routing::gateway_group_reorder, routing::gateway_group_move,
        routing::gateway_set_slug, agents::gateway_wiring_accept, agents::open_zcode_client,
        routing::gateway_catalog, routing::gateway_route_log, routing::gateway_route_log_page,
        routing::gateway_sessions_overview, routing::gateway_today_counts, routing::gateway_day_overview,
        routing::gateway_context_sessions, routing::gateway_context_snapshot, routing::gateway_context_items,
        routing::gateway_context_tool_items, routing::gateway_context_history, gateway_access::gateway_lan_address,
        agents::custom_agents_list, agents::custom_agents_add, agents::custom_agents_remove,
        agents::custom_agents_wire, agents::custom_agents_unwire, agents::custom_agents_preview,
        island_window::dock_undock, usage_sessions::cost_estimate, quotas::connection_quota,
        browser_auth::start_browser_authorization, browser_auth::browser_authorization_status, browser_auth::cancel_browser_authorization,
        browser_auth::save_browser_connection, quotas::connection_api_usage, quotas::local_codex_quota,
        quotas::local_grok_quota, quotas::local_zcode_quota, quotas::connection_balance,
        island_window::island_menu, tray_menu::menu_action, tray_menu::menu_fit,
        tray_menu::menu_show, tray_menu::menu_close, tray_menu::menu_refreshing,
        island_window::island_drag, panel::main_drag, island_window::island_position,
        island_window::set_island_visibility, island_window::island_action, island_window::island_topmost,
        preferences::set_dock_enabled, preferences::set_island_horizontal_dock_full, preferences::set_theme,
        preferences::set_display_preferences, island_window::set_island_style, island_window::set_island_linkage,
        island_window::set_main_context, island_window::main_context, panel::open_main_section,
        panel::open_main_session, panel::open_main_usage, panel::open_main_automation_new,
        preferences::set_balance_alert, system_data::take_app_notices, preferences::set_retention_days,
        preferences::cleanup_preview, preferences::cleanup_history, panel::main_panel_ready,
        panel::startup_error, panel::retry_startup, panel::quit_startup,
        panel::take_main_intent, providers::fetch_remote_models, system_data::preview_import_file,
        system_data::confirm_import, system_data::discard_import, updater::set_auto_check_updates,
        updater::set_update_interval, updater::check_app_update_available, updater::download_app_update,
        updater::cancel_app_update_download, updater::install_downloaded_update_and_restart, panel::set_default_page,
        panel::remember_page, panel::set_appearance_preferences, skills::skills_import_git,
        skills::skills_import_git_cleanup, providers::verify_custom_model, system_data::set_network,
        system_data::diagnose_network, automation::automation_skills, skills::skills_set_global_dir,
        skills::skills_save_dir_settings, preferences::set_task_notify, handoff::handoff_state,
        handoff::handoff_start, handoff::handoff_refine, handoff::handoff_estimate_refine,
        handoff::handoff_finish, handoff::handoff_cancel, handoff::handoff_render,
        handoff::handoff_local_export, handoff::handoff_save, handoff::validate_file_name,
    ];
    move |invoke: tauri::ipc::Invoke| {
        if invoke.message.webview().label().starts_with("provider-auth-") {
            invoke.resolver.reject("官方授权页面不能访问 Jolu 应用命令");
            true
        } else {
            handler(invoke)
        }
    }
}
