//! 用量统计、本机会话与数据导出。

#[allow(unused_imports)]
use crate::*;

#[tauri::command]
pub(crate) fn collection_status() -> CollectionStatus {
    COLLECTION_STATUS.lock().map(|status| status.clone()).unwrap_or(CollectionStatus {
        ok: false,
        errors: vec!["采集状态锁不可用".into()],
        last_success_ms: None,
        failed_sources: Vec::new(),
        consecutive_failures: 0,
    })
}

#[tauri::command]
pub(crate) async fn scan_local_sessions(app: tauri::AppHandle, db: State<'_, Db>) -> Result<collector::ScanResult, String> {
    if validation::isolated() { return Err("隔离验收不读取用户日志".into()); }
    let result = with_database(db.0.clone(), move |conn| {
        Ok(collector::scan(conn))
    })
    .await?;
    emit_collection_status(&app, &result);
    Ok(result)
}

/// 区间内的 Token 分项（新增输入 / 输出 / 缓存写入 / 缓存命中）、请求数与命中率。
/// 未知字段保持 null 交给界面显示「—」，不在这里补零。
#[tauri::command]
pub(crate) async fn source_metrics(db: State<'_, DbRead>, platform:String, period:String,
    custom:Option<db::CustomRange>, model:Option<String>, query_end_ms:i64) -> Result<source_store::Metrics,String> {
    validate_query_range(custom,query_end_ms)?;
    if !platforms::known(&platform) {return Err("未知平台".into());}
    with_reader(db.0.clone(),move |conn| {
        let (start,end)=db::resolve_range_at(conn,&platform,&period,custom,query_end_ms);
        source_store::metrics(conn,&platform,start,end,model.as_deref()).map_err(|e|e.to_string())
    }).await
}

/// 本机为该平台配置的接入方式：绿色 Auth / 蓝色 API（§6.3）
/// 只返回方式与来源说明，绝不返回凭证内容
#[tauri::command]
pub(crate) fn connection_kind(platform: String) -> creds::ConnectionInfo {
    creds::detect(&platform)
}

/// 灵动岛实时用量：自 `since` 游标以来的新增 Token、涉及的会话、最近活跃会话
#[tauri::command]
pub(crate) async fn live_usage(
    db: State<'_, DbRead>,
    platform: String,
    since: Option<i64>,
) -> Result<db::LiveUsage, String> {
    with_reader(db.0.clone(), move |conn| {
        let r = session_presence::live_usage(conn, &platform, since).map_err(|e| e.to_string())?;
        if since.is_none() || r.delta_tokens > 0 {
            println!(
                "[实时] since={:?} 新增={} token, 运行中={} 个会话, 今日={:?}",
                since, r.delta_tokens, r.active_sessions.len(), r.today_tokens
            );
        }
        Ok(r)
    })
    .await
}

/// 用量统计页（模块 C）：概览与用量分析共用的跨供应商聚合
#[tauri::command]
pub(crate) async fn island_source_breakdown(db: State<'_, DbRead>, platform: Option<String>) -> Result<usage_stats::SourceBreakdown, String> {
    // 本地时区的今天 [00:00, 明天 00:00)
    let today = chrono::Local::now().date_naive();
    let bound = |day: chrono::NaiveDate| {
        day.and_hms_opt(0, 0, 0)
            .and_then(|time| time.and_local_timezone(chrono::Local).earliest())
            .map(|time| time.timestamp_millis())
    };
    let (Some(start), Some(end)) = (bound(today), today.succ_opt().and_then(bound)) else {
        return Err("无法确定今天的时间范围".into());
    };
    with_reader(db.0.clone(), move |conn| usage_stats::source_breakdown(conn, platform.as_deref(), start, end).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub(crate) async fn stats_overview(db: State<'_, DbRead>, filter: usage_stats::StatsFilter) -> Result<usage_stats::Overview, String> {
    filter.validate()?;
    with_reader(db.0.clone(), move |conn| usage_stats::overview(conn, &filter).map_err(|e| e.to_string())).await
}

/// 全年活跃日历；日期固定为一年，只按供应商与账号筛选
#[tauri::command]
pub(crate) async fn stats_activity(db: State<'_, DbRead>, platform: Option<String>, connections: Vec<String>, now_ms: i64) -> Result<usage_stats::Activity, String> {
    if platform.as_deref().is_some_and(|p| !platforms::known(p)) { return Err("未知平台".into()); }
    with_reader(db.0.clone(), move |conn| usage_stats::activity(conn, platform, connections, now_ms).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub(crate) async fn stats_calls(db: State<'_, DbRead>, filter: usage_stats::StatsFilter, page: u32) -> Result<usage_stats::CallPage, String> {
    filter.validate()?;
    with_reader(db.0.clone(), move |conn| usage_stats::calls(conn, &filter, page).map_err(|e| e.to_string())).await
}

/// 会话线程页（模块 D）：本机会话列表
#[tauri::command]
pub(crate) async fn sessions_list(db: State<'_, DbRead>, filter: session_threads::SessionFilter) -> Result<session_threads::SessionList, String> {
    if filter.platform.as_deref().is_some_and(|p| !platforms::known(p)) { return Err("未知平台".into()); }
    with_reader(db.0.clone(), move |conn| session_threads::list(conn, &filter).map_err(|e| e.to_string())).await
}

/// 只读消息记录：`before` 为空取最后一页
#[tauri::command]
pub(crate) async fn session_messages(db: State<'_, DbRead>, platform: String, session_id: String, before: Option<usize>) -> Result<session_threads::MessagePage, String> {
    with_reader(db.0.clone(), move |conn| session_threads::messages(conn, &platform, &session_id, before, 40).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub(crate) async fn session_continue_check(db: State<'_, DbRead>, platform: String, session_id: String) -> Result<session_threads::ContinueCheck, String> {
    with_reader(db.0.clone(), move |conn| session_threads::continue_check(conn, &platform, &session_id).map_err(|e| e.to_string())).await
}

/// 在新的控制台窗口里用原工具续聊（Claude Code / Codex）
#[tauri::command]
pub(crate) async fn session_continue(db: State<'_, DbRead>, platform: String, session_id: String) -> Result<(), String> {
    with_reader(db.0.clone(), move |conn| session_threads::continue_session(conn, &platform, &session_id)).await
}

/// 导出最近 `days` 天的用量到 OTLP 端点（批次 G §3.12）。
///
/// 只导聚合值（平台 + 模型 + token 计数，以及按命名 Key 名的请求数），不含会话、连接、Key 原文或请求正文。
/// 端点与认证头由调用方传；认证头只在本函数内使用，不回显任何值。
#[tauri::command]
pub(crate) async fn otlp_export_usage(
    app: tauri::AppHandle, endpoint: String, authorization: Option<String>, days: u32,
) -> Result<otlp::OtlpExportResult, String> {
    let span_days = days.clamp(1, 90) as i64;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let since_ms = now_ms - span_days * 24 * 60 * 60 * 1000;
    let db = app.state::<Db>().0.clone();
    let (points, keys) = with_database(db, move |conn| {
        Ok::<_, String>((otlp::aggregate(conn, since_ms, now_ms)?, otlp::aggregate_keys(conn, since_ms, now_ms)?))
    })
    .await?;
    let mut headers = Vec::new();
    if let Some(value) = authorization.filter(|v| !v.trim().is_empty()) {
        headers.push(("authorization".to_string(), value));
    }
    otlp::export(&endpoint, &headers, &points, &keys, now_ms).await
}

/// 导出备份的核心逻辑，由 `data_files` 的文件导出命令调用；日期区间左闭右开
pub(crate) async fn export_data(db: State<'_, DbRead>, platform: Option<String>, start_ms: Option<i64>, end_ms: Option<i64>) -> Result<String, String> {
    data_files::valid_export_scope(platform.as_deref(), start_ms, end_ms)?;
    with_reader(db.0.clone(), move |conn| db::export_backup(conn, platform.as_deref(), start_ms, end_ms)).await
}

#[tauri::command]
pub(crate) async fn cost_estimate(
    db: State<'_, DbRead>,
    platform: String,
    period: String,
    custom: Option<db::CustomRange>,
    model: Option<String>,
    connection: Option<String>,
    query_end_ms: i64,
) -> Result<db::CostEstimate, String> {
    validate_query_range(custom, query_end_ms)?;
    with_reader(db.0.clone(), move |conn| {
        db::cost_estimate_at_scoped(conn, &platform, &period, custom, model.as_deref(), connection.as_deref(), query_end_ms)
            .map_err(|e| e.to_string())
    })
    .await
}
