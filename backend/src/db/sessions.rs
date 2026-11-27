//! 会话元数据（标题、工作目录、子代理、生命周期）与灵动岛实时增量快照。

use chrono::Local;
use rusqlite::{params, Connection};
use serde::Serialize;

use super::metrics::{period_range, platform_clause_n, EXCLUDE_DEGENERATE_SQL, FRESH_TOKENS_SQL};

pub fn upsert_session_title(
    conn: &Connection,
    platform: &str,
    session_id: &str,
    title: &str,
) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT INTO sessions(platform, session_id, title) VALUES(?1,?2,?3)
         ON CONFLICT(platform, session_id) DO UPDATE SET title = ?3
         WHERE sessions.title IS NOT excluded.title",
    )?.execute(params![platform, session_id, title])?;
    Ok(())
}

/// 记录会话的项目目录（cwd）。库型来源（ZCode）没有源文件可读，只能靠这里落库。
/// 已存在非空值时不覆盖为空——真实路径丢失比保留旧值更糟。
pub fn set_session_cwd(conn: &Connection, platform: &str, session_id: &str, cwd: &str) -> rusqlite::Result<()> {
    let trimmed = cwd.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let mut stmt = conn.prepare_cached(
        "INSERT INTO sessions(platform, session_id, cwd) VALUES(?1, ?2, ?3)
         ON CONFLICT(platform, session_id) DO UPDATE SET cwd = COALESCE(sessions.cwd, excluded.cwd)",
    )?;
    stmt.execute(rusqlite::params![platform, session_id, trimmed])?;
    Ok(())
}

/// 标记会话是否属于子代理（如 DeepSeek Harness 的 `origin = "subagent"`）。
/// 用量仍完整计入 `requests`；会话线程页据此隐藏子代理，避免单个任务刷出大量会话。
pub fn set_session_subagent(
    conn: &Connection,
    platform: &str,
    session_id: &str,
    is_subagent: bool,
) -> rusqlite::Result<()> {
    let flag = i32::from(is_subagent);
    conn.prepare_cached(
        "INSERT INTO sessions(platform, session_id, is_subagent) VALUES(?1, ?2, ?3)
         ON CONFLICT(platform, session_id) DO UPDATE SET is_subagent = excluded.is_subagent
         WHERE sessions.is_subagent IS NOT excluded.is_subagent",
    )?
    .execute(params![platform, session_id, flag])?;
    Ok(())
}

pub fn set_session_activity(
    conn: &Connection,
    platform: &str,
    session_id: &str,
    state: &str,
    updated_at_ms: i64,
) -> rusqlite::Result<()> {
    let state = if matches!(state, "running" | "done" | "failed" | "unknown" | "waiting") { state } else { "unknown" };
    conn.prepare_cached(
        "INSERT INTO sessions(platform, session_id, is_running, activity_state, activity_updated_ms)
         VALUES(?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(platform, session_id) DO UPDATE SET
           is_running = excluded.is_running,
           activity_state = excluded.activity_state,
           activity_updated_ms = excluded.activity_updated_ms
         WHERE sessions.activity_updated_ms IS NULL
            OR excluded.activity_updated_ms >= sessions.activity_updated_ms",
    )?.execute(params![platform, session_id, (state == "running") as i32, state, updated_at_ms])?;
    Ok(())
}

pub fn set_session_turn_start(
    conn: &Connection,
    platform: &str,
    session_id: &str,
    started_at_ms: i64,
) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "UPDATE sessions SET turn_started_ms = ?1 WHERE platform = ?3 AND session_id = ?2",
    )?
    .execute(params![started_at_ms, session_id, platform])?;
    Ok(())
}

/* ------------------------------------------------------------------------ */
/*  实时增量与最近活跃会话（§2.1.1 / §2.1.2）                                */
/* ------------------------------------------------------------------------ */

#[derive(Debug, Clone, Serialize)]
pub struct ActiveSession {
    pub session_id: String,
    /// 来源提供的真实标题；缺失时显示未命名会话，不把 ID 当标题
    pub title: String,
    /// 本段提示内该会话新增的 Token
    pub delta_tokens: i64,
    pub last_seen_ms: i64,
    pub started_at_ms: Option<i64>,
    /// 当前会话本轮累计 Token；没有可信任务起点时未知。
    pub running_tokens: Option<i64>,
    /// running / done / failed；没有可信生命周期的来源使用 recent。
    pub state: String,
    /// 这条会话最近一条请求归到的已存连接（`exact` / `mapped`）；归不出唯一结论时是
    /// `platform` 档，不带连接 id。见 `attribution` 模块。
    pub attribution: crate::attribution::AttributionView,
}

#[derive(Debug, Serialize)]
pub struct LiveUsage {
    /// 本次快照所属平台；前端切换平台后据此丢弃迟到事件。
    pub platform: String,
    /// 数据库游标，调用方下次带回来即可拿到「自那以后」的新增
    pub cursor: i64,
    /// 自 since 之后新增的 Token 合计
    pub delta_tokens: i64,
    /// 新增涉及的会话（用于展开态按会话展示明细）
    pub delta_sessions: Vec<ActiveSession>,
    /// Codex 为生命周期确认的运行会话；其他来源暂为近期活跃会话
    pub active_sessions: Vec<ActiveSession>,
    /// 正在执行的本轮真实累计值，可在页面重开后恢复；没有可信起点时未知。
    pub running_tokens: Option<i64>,
    /// 0 表示由生命周期驱动，不因静默过期；正数只用于“最近活跃”来源。
    pub active_window_seconds: i64,
    pub today_tokens: Option<i64>,
    /// `today_tokens` 的覆盖情况：今天有多少条请求带 Token、多少条没有。
    ///
    /// 为什么单独给：`today_tokens` 为 None 有两种可能——「今天没有记录」或者
    /// 「有记录但全都没给 Token」。灵动岛需要区分这两者（前者是真零，后者是未知），
    /// 而单一个 `Option` 表达不了。用共享契约 `data_contract::Coverage` 算，
    /// 与用量页 `token_coverage`、会话卡片 `SessionTokens` 同一套规则。
    pub today_token_coverage: crate::data_contract::Coverage,
    /// `today_tokens` 的来源元数据（P1-4）：与用量页概览同一套 [`crate::data_contract::Provenance`]，
    /// 范围是本地时区的今天（与 `today_fresh_tokens` 同一组边界）。
    pub today_provenance: crate::data_contract::Provenance,
    /// 新增记录是否来自刚发生的实时写入；历史补采不得播放增量动画。
    pub realtime_delta: bool,
    /// 首次加载：调用方不得据此播放增量动画（§2.1.1）
    pub initial: bool,
    /// 本平台的统计数据自上次推送以来有变化。仅为续期运行中会话的心跳推送为 false，
    /// 主面板等统计视图据此跳过无意义的重查；主动查询的快照恒为 false。
    pub data_changed: bool,
    /// 本平台正在跑的已存连接。`exact` / `mapped` 才带连接 id，`platform` 档表示
    /// 「归不出唯一一条」——灵动岛收缩态那一行据此显示连接名或平台名，绝不用猜的。
    pub attribution: crate::attribution::AttributionView,
}

/// 尚无可信生命周期来源的“最近活跃”窗口，不得将它显示成运行状态。
pub const ACTIVE_WINDOW_SECONDS: i64 = 90;

/// 无锁文件但具备明确请求/工具事件流的工具（Antigravity / Kimi / OpenCode / Pi / DSH），
/// 在长耗时命令执行期间保持 running 的宽限上限（15 分钟）；宿主进程退出由 `cli_liveness` 秒级截断。
pub const CLI_RUNNING_GRACE_MS: i64 = 15 * 60 * 1_000;

/// 「运行中 Token」合计只统计 running 会话；失败会话仅自带本轮最终值用于冻结展示。
/// 没有任何 running 会话时返回 None。
pub fn sum_running_tokens(sessions: &[ActiveSession]) -> Option<i64> {
    match sessions.iter()
        .filter(|session| session.state == "running")
        .map(|session| session.running_tokens)
        .collect::<Option<Vec<_>>>()
    {
        Some(values) if values.is_empty() => None,
        Some(values) => Some(values.iter().sum()),
        None => None,
    }
}

/// 今天有多少条请求带 Token、多少条没有。
///
/// 与 [`today_fresh_tokens`] 配套：那个函数只给总量（全未知时是 None），
/// 这个给覆盖情况，让灵动岛能区分「今天没有记录」（真零）与「有记录但全没给 Token」（未知）。
/// 规则与 `data_contract::Coverage` 一致：不把未知当零。
///
/// 总量由调用方另算后传入（`today_fresh_tokens` 才有各平台口径：Codex 要扣缓存），
/// 由 `Coverage::from_counts` 保证「零条已知时总量未知」。
pub fn today_token_coverage(
    conn: &Connection,
    platform: &str,
    total_tokens: Option<i64>,
) -> rusqlite::Result<crate::data_contract::Coverage> {
    let (start, end) = period_range(conn, platform, "today");
    let (tc, te) = platform_clause_n(platform, 3);
    let sql = format!(
        "SELECT COUNT(*), COALESCE(SUM(total_known), 0)
         FROM requests WHERE ts >= ?1 AND ts < ?2{EXCLUDE_DEGENERATE_SQL}{tc}"
    );
    let mut args: Vec<&dyn rusqlite::ToSql> = vec![&start, &end];
    args.extend(te.iter().map(|value| value as &dyn rusqlite::ToSql));
    let (rows, known): (i64, i64) =
        conn.query_row(&sql, rusqlite::params_from_iter(args), |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(crate::data_contract::Coverage::from_counts(known, rows - known, total_tokens))
}

/// 今日 Token 的来源元数据：范围与 [`today_token_coverage`] 相同（`period_range` 的今天，
/// 排除退化行），保证灵动岛的「今天」与用量页、路由概览用的是同一组本地自然日边界。
pub fn today_provenance(conn: &Connection, platform: &str) -> rusqlite::Result<crate::data_contract::Provenance> {
    use rusqlite::types::Value;
    let (start, end) = period_range(conn, platform, "today");
    let (scope, scope_args) = if platform == "all" {
        ("1 = 1".to_string(), vec![])
    } else {
        ("r.platform = ?".to_string(), vec![Value::Text(platform.to_string())])
    };
    let range = format!("r.ts >= ? AND r.ts < ? AND {scope}{EXCLUDE_DEGENERATE_SQL}");
    let mut range_args = vec![Value::Integer(start), Value::Integer(end)];
    range_args.extend(scope_args.iter().cloned());
    crate::data_contract::requests_provenance(
        conn,
        start,
        end,
        &range,
        &range_args,
        &scope,
        &scope_args,
        Local::now().timestamp_millis(),
    )
}

/// 本机今日 Token（新鲜口径）：Claude 取输入+输出，Codex 扣除缓存重读与缓存写入，
/// 与灵动岛「本机今日 Token」同源（§2.1.2）；托盘摘要的 API Key 无套餐视图也复用。
///
/// 未知行（`total_known = 0`）只把自己排除在求和之外，不把整天打成未知：当天哪怕只有
/// 一条失败请求、其余行都已知，也照样把已知行加起来（这是当天的下界）。只有当**一条已
/// 知行都没有**时才返回 None（界面显示「—」，不补零）。真实零行照常参与求和，所以
/// 「确实是 0」仍然是 `Some(0)`，与未知分开。
///
/// 旧写法用 `MIN(total_known) = 1` 做守卫，等于「一行未知污染全天」：一条抓不到用量的
/// 失败请求就能让整天的合计变成 null（本机 zcode 就出现过当天有 2,025 万 Token、
/// 却因为凌晨一条失败记录返回 null）。
pub fn today_fresh_tokens(conn: &Connection, platform: &str) -> rusqlite::Result<Option<i64>> {
    if crate::platforms::native(platform) {
        let (start,end)=period_range(conn,platform,"today");
        // 新鲜口径与灵动岛增量（`source_store` 写 `usage_events.delta_fresh`）一致：
        // `includes_cache` 的输入扣掉缓存读取，`excludes_cache` 直接取输入 + 输出。
        // 语义未知或分项缺失的行拆不出缓存，只能沿用来源给的总量——这是上界，不是新鲜值；
        // 以前所有原生行都用总量，ZCode / Qoder 这类缓存占大头的平台今日数字被放大十几倍，
        // 而同一张卡片上的增量却是新鲜值（P4 审计 D5）。
        return conn.query_row(
            &format!(
                "SELECT CASE WHEN COUNT(*) = 0 OR COALESCE(SUM(total_known), 0) = 0 THEN NULL
                             ELSE SUM(CASE WHEN total_known = 1 THEN {FRESH_TOKENS_SQL} END)
                        END
                   FROM requests WHERE platform = ?1 AND ts >= ?2 AND ts < ?3"
            ),
            params![platform, start, end],
            |r| r.get(0),
        );
    }
    let (today_start, today_end) = period_range(conn, platform, "today");
    let (tc, te) = platform_clause_n(platform, 3);
    // 灵动岛「本机今日 Token」与终端口径一致：新鲜 Token。
    // Claude 的缓存单列，Codex 的 input_tokens 已含缓存，因此 Codex 需要扣除缓存重读；
    // 主面板统计仍用 total_tokens（含缓存）并另行标注。
    // 求和同样只取已知行；当天一条已知行都没有时才是未知（None），无记录沿用本机来源
    // 之外的约定：非原生平台没有记录就是真实的 0。
    let today_sql = format!(
        "SELECT CASE WHEN COUNT(*) = 0 THEN 0
                     WHEN COALESCE(SUM(total_known), 0) = 0 THEN NULL
                     ELSE SUM(
                         CASE WHEN total_known = 1 THEN
                             CASE WHEN platform = 'codex'
                                  THEN MAX(input_tokens - cache_read_tokens - cache_write_tokens, 0) + output_tokens
                                  ELSE input_tokens + output_tokens
                             END
                         END)
                     END
         FROM requests WHERE ts >= ?1 AND ts < ?2{EXCLUDE_DEGENERATE_SQL}{tc}"
    );
    if te.is_empty() {
        conn.query_row(&today_sql, params![today_start, today_end], |r| r.get(0))
    } else {
        conn.query_row(&today_sql, params![today_start, today_end, te[0]], |r| r.get(0))
    }
}

fn query_delta_sessions(
    conn: &Connection,
    platform: &str,
    since: Option<i64>,
) -> rusqlite::Result<Vec<ActiveSession>> {
    let Some(id) = since else { return Ok(vec![]) };
    let delta_clause = if platform == "all" { "" } else { " AND e.platform = ?2" };
    let sql = format!(
        "SELECT e.session_id, s.title, SUM(COALESCE(e.delta_fresh, e.delta_total)), MAX(e.ts)
         FROM usage_events e
         LEFT JOIN sessions s
           ON s.platform = e.platform AND s.session_id = e.session_id
         WHERE e.id > ?1{delta_clause}
         GROUP BY e.session_id ORDER BY MAX(e.ts) DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let map = |row: &rusqlite::Row| -> rusqlite::Result<ActiveSession> {
        let title: Option<String> = row.get(1)?;
        Ok(ActiveSession {
            session_id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
            title: title
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "未命名会话".into()),
            delta_tokens: row.get(2)?,
            last_seen_ms: row.get(3)?,
            started_at_ms: None,
            running_tokens: None,
            state: "recent".into(),
            attribution: crate::attribution::AttributionView::platform(),
        })
    };
    let out = if platform == "all" {
        stmt.query_map(params![id], map)?.collect::<Result<_, _>>()?
    } else {
        stmt.query_map(params![id, platform], map)?.collect::<Result<_, _>>()?
    };
    Ok(out)
}

fn apply_live_attribution(
    conn: &Connection,
    platform: &str,
    active_sessions: &mut [ActiveSession],
) -> crate::attribution::AttributionView {
    let (attribution, session_views) = if platform == "all" {
        (crate::attribution::AttributionView::platform(), std::collections::HashMap::new())
    } else {
        let since = Local::now().timestamp_millis() - crate::attribution::SESSION_ATTRIBUTION_WINDOW_MS;
        match crate::attribution::Snapshot::load(conn) {
            Ok(snapshot) => {
                let views = crate::attribution::session_attribution(conn, platform, since, &snapshot)
                    .unwrap_or_default();
                let total = crate::attribution::aggregate(&views);
                (total, views)
            }
            Err(_) => (crate::attribution::AttributionView::platform(), std::collections::HashMap::new()),
        }
    };
    for session in active_sessions {
        session.attribution = session_views
            .get(&session.session_id)
            .cloned()
            .unwrap_or_else(crate::attribution::AttributionView::platform);
    }
    attribution
}

fn query_lifecycle_sessions(
    conn: &Connection,
    platform: &str,
    running_from: Option<i64>,
    failed_from: i64,
) -> rusqlite::Result<Vec<ActiveSession>> {
    let min_running = running_from.unwrap_or(i64::MIN);
    let mut stmt = conn.prepare(
        "SELECT session_id, title, 0, activity_updated_ms, activity_state, turn_started_ms,
                (SELECT MIN(r.ts) FROM requests r
                  WHERE r.platform = sessions.platform AND r.session_id = sessions.session_id)
                  AS first_seen_ms
         FROM sessions
         WHERE platform = ?1
           AND ((activity_state = 'running' AND activity_updated_ms >= ?3)
                OR (activity_state = 'failed' AND activity_updated_ms >= ?2))
         ORDER BY first_seen_ms DESC, activity_updated_ms DESC",
    )?;
    let rows = stmt
        .query_map(params![platform, failed_from, min_running], |r| {
            let title: Option<String> = r.get(1)?;
            Ok(ActiveSession {
                session_id: r.get(0)?,
                title: title
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| "未命名会话".into()),
                delta_tokens: r.get(2)?,
                last_seen_ms: r.get(3)?,
                started_at_ms: r.get(5)?,
                running_tokens: None,
                state: r.get(4)?,
                attribution: crate::attribution::AttributionView::platform(),
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn query_typed_or_recent_sessions(
    conn: &Connection,
    platform: &str,
    now_ms: i64,
    active_from: i64,
) -> rusqlite::Result<Vec<ActiveSession>> {
    let (clause, extra) = if platform == "all" {
        (String::new(), vec![])
    } else {
        (" AND r.platform = ?2".to_string(), vec![platform.to_string()])
    };
    let typed_states = matches!(platform, "zcode" | "qwen" | "gemini");
    let running_from = now_ms - crate::native_sources::ZCODE_RUNNING_GRACE_MS;
    let recent_tokens = if typed_states {
        "SUM(CASE WHEN r.ts >= ?1 THEN r.total_tokens ELSE 0 END)"
    } else {
        "SUM(r.total_tokens)"
    };
    let last_seen = if typed_states {
        "MAX(MAX(r.ts), COALESCE(CASE WHEN s.activity_updated_ms <= ?3 THEN s.activity_updated_ms END, 0))"
    } else {
        "MAX(r.ts + COALESCE(r.duration_ms, 0))"
    };
    let cond = if typed_states {
        "(r.ts >= ?1
          OR (s.activity_state = 'running' AND s.activity_updated_ms BETWEEN ?4 AND ?3)
          OR (s.activity_state IN ('done', 'failed') AND s.activity_updated_ms BETWEEN ?1 AND ?3))"
    } else {
        "(r.ts + COALESCE(r.duration_ms, 0)) >= ?1"
    };
    let sql = format!(
        "SELECT r.session_id, s.title, {recent_tokens}, {last_seen}, s.activity_state, s.activity_updated_ms, s.turn_started_ms
         FROM requests r
         LEFT JOIN sessions s
           ON s.platform = r.platform AND s.session_id = r.session_id
         WHERE {cond}{clause}
         GROUP BY r.session_id ORDER BY {last_seen} DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let map = |r: &rusqlite::Row| -> rusqlite::Result<ActiveSession> {
        let sid: Option<String> = r.get(0)?;
        let title: Option<String> = r.get(1)?;
        let activity: Option<String> = r.get(4)?;
        let state = if typed_states {
            crate::native_sources::zcode_observed_state(activity.as_deref(), r.get(5)?, now_ms)
        } else {
            "recent"
        };
        Ok(ActiveSession {
            title: title
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "未命名会话".to_string()),
            session_id: sid.unwrap_or_default(),
            delta_tokens: r.get(2)?,
            last_seen_ms: r.get(3)?,
            started_at_ms: if state == "running" { r.get(6)? } else { None },
            running_tokens: None,
            state: state.into(),
            attribution: crate::attribution::AttributionView::platform(),
        })
    };
    let mut out: Vec<ActiveSession> = if typed_states {
        stmt.query_map(params![active_from, extra[0], now_ms + 60_000, running_from], map)?
            .collect::<Result<_, _>>()?
    } else if extra.is_empty() {
        stmt.query_map(params![active_from], map)?.collect::<Result<_, _>>()?
    } else {
        stmt.query_map(params![active_from, extra[0]], map)?.collect::<Result<_, _>>()?
    };

    // 新会话刚提交首轮提问、尚未产出第一条 requests 行时，仍按 sessions 的 running 状态即时透出。
    if typed_states {
        let mut fresh_stmt = conn.prepare(
            "SELECT session_id, title, activity_updated_ms, turn_started_ms
             FROM sessions
             WHERE platform = ?1
               AND activity_state = 'running'
               AND activity_updated_ms BETWEEN ?2 AND ?3
               AND NOT EXISTS (
                   SELECT 1 FROM requests r
                    WHERE r.platform = sessions.platform AND r.session_id = sessions.session_id
               )
             ORDER BY activity_updated_ms DESC",
        )?;
        let fresh = fresh_stmt.query_map(
            params![platform, running_from, now_ms + 60_000],
            |r| {
                let title: Option<String> = r.get(1)?;
                Ok(ActiveSession {
                    session_id: r.get(0)?,
                    title: title
                        .filter(|t| !t.trim().is_empty())
                        .unwrap_or_else(|| "未命名会话".into()),
                    delta_tokens: 0,
                    last_seen_ms: r.get(2)?,
                    started_at_ms: r.get(3)?,
                    running_tokens: None,
                    state: "running".into(),
                    attribution: crate::attribution::AttributionView::platform(),
                })
            },
        )?;
        for row in fresh {
            out.insert(0, row?);
        }
    }
    Ok(out)
}

pub fn live_usage(
    conn: &Connection,
    platform: &str,
    since: Option<i64>,
) -> rusqlite::Result<LiveUsage> {
    let cursor: i64 = conn.query_row("SELECT value FROM event_sequence WHERE singleton=1", [], |r| r.get(0))?;
    let now_ms = Local::now().timestamp_millis();

    let delta_sessions = query_delta_sessions(conn, platform, since)?;
    let delta_tokens = delta_sessions.iter().map(|s| s.delta_tokens).sum();
    let realtime_delta = delta_sessions
        .iter()
        .map(|session| session.last_seen_ms)
        .max()
        .is_some_and(|last_seen| last_seen >= now_ms - 30_000);

    // 思考、等待输入、压缩与长耗时命令执行期间可能长时间没有新用量行；静默不代表本轮结束。
    let lock_lifecycle = matches!(platform, "codex" | "claude");
    let cli_lifecycle = matches!(platform, "antigravity" | "kimi" | "opencode" | "pi" | "dsh");
    let lifecycle = lock_lifecycle || cli_lifecycle;
    let active_window_seconds = if lifecycle { 0 } else { ACTIVE_WINDOW_SECONDS };
    let active_from = now_ms - active_window_seconds * 1_000;
    let failed_from = now_ms - ACTIVE_WINDOW_SECONDS * 1_000;

    let mut active_sessions = if lock_lifecycle {
        query_lifecycle_sessions(conn, platform, None, failed_from)?
    } else if cli_lifecycle {
        query_lifecycle_sessions(conn, platform, Some(now_ms - CLI_RUNNING_GRACE_MS), failed_from)?
    } else {
        query_typed_or_recent_sessions(conn, platform, now_ms, active_from)?
    };

    for session in &mut active_sessions {
        if (lifecycle || session.state == "running") && session.started_at_ms.is_some() {
            // 实时口径对齐终端：只算新鲜 Token（input+output），缓存重读不计。
            session.running_tokens = Some(conn.query_row(
                "SELECT COALESCE(SUM(COALESCE(delta_fresh, delta_total)), 0) FROM usage_events
                 WHERE platform = ?3 AND session_id = ?1 AND ts >= ?2",
                params![session.session_id, session.started_at_ms, platform],
                |row| row.get::<_, i64>(0),
            )?);
        }
    }
    let running_tokens = sum_running_tokens(&active_sessions);

    let attribution = apply_live_attribution(conn, platform, &mut active_sessions);
    let today_tokens = today_fresh_tokens(conn, platform)?;
    let today_token_coverage = today_token_coverage(conn, platform, today_tokens)?;
    let today_provenance = today_provenance(conn, platform)?;

    Ok(LiveUsage {
        platform: platform.to_string(),
        cursor,
        delta_tokens,
        delta_sessions,
        active_sessions,
        running_tokens,
        active_window_seconds,
        today_tokens,
        today_token_coverage,
        today_provenance,
        realtime_delta,
        initial: since.is_none(),
        data_changed: false,
        attribution,
    })
}

