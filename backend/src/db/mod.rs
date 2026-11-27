//! SQLite 存储与统计查询门面（子模块按职责拆分：`conn` / `schema` / `requests` / `sessions` / `metrics` / `backup`）。

pub mod backup;
pub mod conn;
pub mod metrics;
pub mod requests;
pub mod schema;
pub mod sessions;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_breakdown;
#[cfg(test)]
mod tests_fresh_token;

use rusqlite::Connection;

pub use backup::{
    export_backup, import_parsed, preview_backup, ImportPreview, ImportSummary, ParsedBackup,
};
#[cfg(test)]
pub use backup::{import_backup, native_outcome_valid, NATIVE_OUTCOMES};
pub use conn::{open_reader, Store};
pub use metrics::{
    cost_estimate_at_scoped, period_range, period_range_at, resolve_range_at, CostEstimate,
    CustomRange,
};
pub(crate) use metrics::{EXCLUDE_DEGENERATE_SQL, FRESH_TOKENS_SQL};
#[cfg(test)]
pub use metrics::{
    cost_estimate_at, list_models_at, list_models_at_scoped, request_log_at,
    request_log_at_scoped, token_totals_at, token_totals_at_scoped, trend, trend_at,
    trend_at_scoped, usage_breakdown_at, usage_breakdown_at_scoped, LogPage, LogRow, PeriodTotals,
    Trend, TrendPoint, UsageBreakdown, PAGE_SIZE,
};
pub use requests::{
    apply_proxy_timing, apply_proxy_timing_with_models, cleanup_history, cleanup_preview,
    get_scan_checkpoint, get_scan_state, scan_stamps, set_scan_checkpoint, CleanupPreview,
    RequestRecord, ScanCheckpoint, ScanStamp, LOCAL_SESSION_SOURCE,
};
pub(crate) use requests::{
    insert_gateway_record_with_timing, insert_scanned_records, set_cache_write_1h,
    set_request_duration_if_missing, stamp_active_config_attribution,
};
#[cfg(test)]
pub use requests::{
    attributed_request_count, commit_scanned_file, get_offset, insert_records, set_offset,
};
pub use sessions::{
    live_usage, set_session_activity, set_session_cwd, set_session_subagent,
    set_session_turn_start, sum_running_tokens, today_fresh_tokens, today_provenance,
    today_token_coverage, upsert_session_title, ActiveSession, LiveUsage, ACTIVE_WINDOW_SECONDS,
    CLI_RUNNING_GRACE_MS,
};

pub fn open(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = conn::open_raw(path)?;
    init(&conn)?;
    let _ = conn.execute_batch("PRAGMA analysis_limit=400; PRAGMA optimize=0x10002;");
    Ok(conn)
}

pub(crate) fn init(conn: &Connection) -> rusqlite::Result<()> {
    schema::init_tables(conn)?;
    let migration = crate::gateway::identity::migrate(conn);
    #[cfg(debug_assertions)]
    migration?;
    #[cfg(not(debug_assertions))]
    if let Err(error) = migration {
        eprintln!("[模型入口] 连接身份迁移失败，本次按降级状态启动：{error}");
    }
    sync_agent_seeds(conn)?;
    Ok(())
}

fn sync_agent_seeds(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("INSERT OR IGNORE INTO agents(id) VALUES(?1)")?;
    for agent in crate::gateway::wiring::agents() {
        stmt.execute([agent])?;
    }
    Ok(())
}
