//! 请求级记录写入、扫描位点管理、代理计时回填与历史清理。

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use super::sessions::{set_session_activity, upsert_session_title};

/// 本地会话日志采集的统一来源名。
pub const LOCAL_SESSION_SOURCE: &str = "local-session";

/// 一条请求级记录。字段缺失一律用 Option，不伪造为 0（§2.4）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRecord {
    pub platform: String,
    pub source: String,
    /// 去重键：Claude 用 message.id，Codex 用 response_id
    pub dedup_key: String,
    pub session_id: Option<String>,
    /// UTC 毫秒
    pub ts: i64,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    /// 总 Token：优先采用来源提供且语义明确的总值，否则由采集器按该来源口径计算
    pub total_tokens: Option<i64>,
    /// 思考强度（low / medium / high 等），来源原样透传。
    /// Claude 在 assistant 行自带；Codex 只在 turn_context 上给出，由采集器按会话继承。
    /// 来源未提供时为 None —— 界面显示「—」，不猜测也不补默认档位。
    pub effort: Option<String>,
}


#[cfg(test)]
pub fn get_offset(conn: &Connection, path: &str) -> i64 {
    get_scan_state(conn, path).map_or(0, |state| state.0)
}

#[derive(Debug, Serialize)]
pub struct CleanupPreview {
    pub cutoff_ms: i64,
    pub requests: i64,
    pub usage_events: i64,
    pub sessions: i64,
}


pub fn get_scan_state(conn: &Connection, path: &str) -> Option<(i64, i64)> {
    conn.prepare_cached("SELECT offset, mtime FROM scan_state WHERE path = ?1").ok()?
        .query_row(params![path], |row| Ok((row.get(0)?, row.get(1)?)))
        .ok()
}

/// 扫描位点的精简视图：心跳在锁外比对文件大小与修改时间，只把真正变化的文件交给采集。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanStamp {
    pub offset: i64,
    pub mtime: i64,
    pub parser_initialized: bool,
}

/// 一次读出全部位点，代替逐文件查询。
pub fn scan_stamps(conn: &Connection) -> rusqlite::Result<std::collections::HashMap<String, ScanStamp>> {
    let mut stmt = conn.prepare_cached("SELECT path, offset, mtime, parser_initialized FROM scan_state")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, ScanStamp {
        offset: row.get(1)?,
        mtime: row.get(2)?,
        parser_initialized: row.get::<_, i32>(3)? != 0,
    })))?;
    rows.collect()
}

#[derive(Default)]
pub struct ScanCheckpoint {
    pub offset: i64,
    pub mtime: i64,
    pub parser_model: Option<String>,
    pub parser_session_id: Option<String>,
    pub parser_activity_state: Option<String>,
    pub parser_updated_at_ms: Option<i64>,
    pub parser_initialized: bool,
    /// Codex 的思考强度（会话级，随文件顺序继承）
    pub parser_effort: Option<String>,
}

pub fn get_scan_checkpoint(conn: &Connection, path: &str) -> Option<ScanCheckpoint> {
    conn.prepare_cached(
        "SELECT offset, mtime, parser_model, parser_session_id,
                parser_activity_state, parser_updated_at_ms, parser_initialized,
                parser_effort
         FROM scan_state WHERE path = ?1",
    ).ok()?.query_row(
        params![path],
        |row| Ok(ScanCheckpoint {
            offset: row.get(0)?,
            mtime: row.get(1)?,
            parser_model: row.get(2)?,
            parser_session_id: row.get(3)?,
            parser_activity_state: row.get(4)?,
            parser_updated_at_ms: row.get(5)?,
            parser_initialized: row.get::<_, i32>(6)? != 0,
            parser_effort: row.get(7)?,
        }),
    ).ok()
}

pub fn set_scan_checkpoint(
    conn: &Connection,
    path: &str,
    checkpoint: &ScanCheckpoint,
) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT INTO scan_state(
             path, offset, mtime, parser_model, parser_session_id,
             parser_activity_state, parser_updated_at_ms, parser_initialized,
             parser_effort
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(path) DO UPDATE SET
             offset = excluded.offset,
             mtime = excluded.mtime,
             parser_model = excluded.parser_model,
             parser_session_id = excluded.parser_session_id,
             parser_activity_state = excluded.parser_activity_state,
             parser_updated_at_ms = excluded.parser_updated_at_ms,
             parser_initialized = excluded.parser_initialized,
             parser_effort = excluded.parser_effort",
    )?.execute(
        params![
            path,
            checkpoint.offset,
            checkpoint.mtime,
            checkpoint.parser_model,
            checkpoint.parser_session_id,
            checkpoint.parser_activity_state,
            checkpoint.parser_updated_at_ms,
            checkpoint.parser_initialized as i32,
            checkpoint.parser_effort,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
pub fn set_offset(conn: &Connection, path: &str, offset: i64, mtime: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO scan_state(path, offset, mtime) VALUES(?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET offset = ?2, mtime = ?3",
        params![path, offset, mtime],
    )?;
    Ok(())
}

/// 实时统计使用的新鲜 Token 口径。
/// Claude 的 input_tokens 不含缓存，Codex 的 input_tokens 已包含 cached_input_tokens，
/// 所以只有 Codex 需要从输入量中扣除缓存字段。
fn fresh_tokens(platform: &str, input: i64, output: i64, cache_read: i64, cache_write: i64) -> i64 {
    let input = input.max(0);
    let output = output.max(0);
    let uncached_input = if platform == "codex" {
        input
            .saturating_sub(cache_read.max(0))
            .saturating_sub(cache_write.max(0))
    } else {
        input
    };
    uncached_input.saturating_add(output)
}

pub(super) fn insert_records_into(conn: &Connection, recs: &[RequestRecord]) -> rusqlite::Result<usize> {
    fn merge(old: i64, known: bool, next: Option<i64>) -> (i64, bool) {
        match next {
            Some(value) => (if known { old.max(value) } else { value }, true),
            None => (old, known),
        }
    }
    let mut changed = 0usize;
    for r in recs {
        let previous = conn.prepare_cached(
            "SELECT id, input_tokens, output_tokens, cache_read_tokens,
                    cache_write_tokens, total_tokens, input_known, output_known,
                    cache_read_known, cache_write_known, total_known
             FROM requests WHERE source = ?1 AND dedup_key = ?2",
        )?.query_row(
            params![r.source, r.dedup_key],
            |row| Ok((
                row.get::<_, i64>(0)?,
                [row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?],
                [row.get::<_, i32>(6)? != 0, row.get::<_, i32>(7)? != 0,
                 row.get::<_, i32>(8)? != 0, row.get::<_, i32>(9)? != 0,
                 row.get::<_, i32>(10)? != 0],
            )),
        ).optional()?;
        let incoming = [r.input_tokens, r.output_tokens, r.cache_read_tokens, r.cache_write_tokens, r.total_tokens];
        let (request_id, delta_total, delta_fresh) = if let Some((id, old, known)) = previous {
            let next: [(i64, bool); 5] =
                std::array::from_fn(|index| merge(old[index], known[index], incoming[index]));
            let delta = if next[4].1 { next[4].0 - if known[4] { old[4] } else { 0 } } else { 0 };
            let old_fresh = fresh_tokens(
                &r.platform,
                if known[0] { old[0] } else { 0 },
                if known[1] { old[1] } else { 0 },
                if known[2] { old[2] } else { 0 },
                if known[3] { old[3] } else { 0 },
            );
            let next_fresh = fresh_tokens(&r.platform, next[0].0, next[1].0, next[2].0, next[3].0);
            let delta_fresh = (next_fresh - old_fresh).max(0);
            if next.iter().enumerate().any(|(index, value)| value.0 != old[index] || value.1 != known[index]) {
                conn.prepare_cached(
                    "UPDATE requests SET session_id = COALESCE(?2, session_id), ts = MAX(ts, ?3),
                        model = COALESCE(?4, model), input_tokens = ?5, output_tokens = ?6,
                        cache_read_tokens = ?7, cache_write_tokens = ?8, total_tokens = ?9,
                        input_known = ?10, output_known = ?11, cache_read_known = ?12,
                        cache_write_known = ?13, total_known = ?14
                     WHERE id = ?1",
                )?.execute(
                    params![id, r.session_id, r.ts, r.model,
                        next[0].0, next[1].0, next[2].0, next[3].0, next[4].0,
                        next[0].1 as i32, next[1].1 as i32, next[2].1 as i32,
                        next[3].1 as i32, next[4].1 as i32],
                )?;
                changed += 1;
            }
            (id, delta.max(0), delta_fresh)
        } else {
            conn.prepare_cached(
                "INSERT INTO requests
                 (platform, source, dedup_key, session_id, ts, model,
                  input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, total_tokens,
                  input_known, output_known, cache_read_known, cache_write_known, total_known, effort)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            )?.execute(
                params![r.platform, r.source, r.dedup_key, r.session_id, r.ts, r.model,
                    r.input_tokens.unwrap_or(0), r.output_tokens.unwrap_or(0),
                    r.cache_read_tokens.unwrap_or(0), r.cache_write_tokens.unwrap_or(0),
                    r.total_tokens.unwrap_or(0), r.input_tokens.is_some() as i32,
                    r.output_tokens.is_some() as i32, r.cache_read_tokens.is_some() as i32,
                    r.cache_write_tokens.is_some() as i32, r.total_tokens.is_some() as i32,
                    r.effort],
            )?;
            changed += 1;
            (conn.last_insert_rowid(), r.total_tokens.unwrap_or(0).max(0),
                fresh_tokens(
                    &r.platform,
                    r.input_tokens.unwrap_or(0),
                    r.output_tokens.unwrap_or(0),
                    r.cache_read_tokens.unwrap_or(0),
                    r.cache_write_tokens.unwrap_or(0),
                ))
        };
        if delta_total > 0 || delta_fresh > 0 {
            conn.prepare_cached(
                "INSERT INTO usage_events(request_id, platform, session_id, ts, delta_total, delta_fresh)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?.execute(
                params![request_id, r.platform, r.session_id, r.ts, delta_total, delta_fresh],
            )?;
        }
    }
    Ok(changed)
}

pub(crate) fn insert_scanned_records(
    conn: &Connection,
    recs: &[RequestRecord],
) -> rusqlite::Result<usize> {
    insert_records_into(conn, recs)
}

/// 本地代理计时回填（proxy.rs）：按去重键关联本机请求记录。
/// 返回受影响行数；0 表示记录尚未由采集器入库，调用方应转入待关联队列稍后重试。
/// source 必须与采集器写入的 [`LOCAL_SESSION_SOURCE`] 一致，硬编码字面量会静默匹配不到。
pub fn apply_proxy_timing(
    conn: &Connection,
    dedup_key: &str,
    first_token_ms: i64,
    duration_ms: i64,
    status_code: i64,
    connection_id: Option<&str>,
) -> rusqlite::Result<usize> {
    apply_proxy_timing_with_models(conn, dedup_key, Some(first_token_ms), duration_ms, status_code, connection_id, None, None)
}

#[allow(clippy::too_many_arguments)]
pub fn apply_proxy_timing_with_models(
    conn: &Connection,
    dedup_key: &str,
    first_token_ms: Option<i64>,
    duration_ms: i64,
    status_code: i64,
    connection_id: Option<&str>,
    upstream_model: Option<&str>,
    served_model: Option<&str>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE requests SET duration_ms = ?3,
                first_token_ms = COALESCE(?4, first_token_ms),
                status_code = ?5,
                connection_id = COALESCE(?6, connection_id),
                attribution_source = CASE WHEN ?6 IS NOT NULL THEN NULL ELSE attribution_source END,
                upstream_model = COALESCE(?7, upstream_model),
                served_model = COALESCE(?8, served_model)
         WHERE dedup_key = ?2 AND (source = ?1 OR source LIKE 'native:%')",
        params![
            LOCAL_SESSION_SOURCE,
            dedup_key,
            duration_ms,
            first_token_ms,
            status_code,
            connection_id,
            upstream_model,
            served_model,
        ],
    )
}

/// 2B 实时入库归因打标：刚从本机日志采入、且尚未经网关/本地代理绑定 `connection_id` 的请求，
/// 标记为当前确认生效的本机连接（`attribution_source = 'active_config'`）。
/// 绝不改写历史旧行（只看 `since_ms` 窗口内），绝不覆盖已有 `connection_id`。
pub(crate) fn stamp_active_config_attribution(
    conn: &Connection,
    platform: &str,
    since_ms: i64,
    connection_id: &str,
) -> rusqlite::Result<usize> {
    if connection_id.trim().is_empty() {
        return Ok(0);
    }
    conn.execute(
        "UPDATE requests
            SET connection_id = ?3,
                attribution_source = 'active_config'
          WHERE platform = ?1
            AND source = ?4
            AND connection_id IS NULL
            AND ts >= ?2",
        params![platform, since_ms, connection_id, LOCAL_SESSION_SOURCE],
    )
}

/// 回填本机/原生来源估算或源端自带的请求耗时；绝不覆盖已有（如代理/网关实测）的耗时。
pub(crate) fn set_request_duration_if_missing(
    conn: &Connection,
    source: &str,
    dedup_key: &str,
    duration_ms: i64,
) -> rusqlite::Result<()> {
    if duration_ms <= 0 {
        return Ok(());
    }
    conn.prepare_cached(
        "UPDATE requests SET duration_ms = COALESCE(duration_ms, ?3)
         WHERE source = ?1 AND dedup_key = ?2",
    )?
    .execute(params![source, dedup_key, duration_ms])?;
    Ok(())
}

/// 记录 Claude 1h TTL 缓存写入量；多次扫描同一请求时取较大值，与 token 合并口径一致。
pub(crate) fn set_cache_write_1h(
    conn: &Connection,
    source: &str,
    dedup_key: &str,
    tokens: i64,
) -> rusqlite::Result<()> {
    if tokens <= 0 {
        return Ok(());
    }
    conn.prepare_cached(
        "UPDATE requests SET cache_write_1h_tokens = MAX(cache_write_1h_tokens, ?3)
         WHERE source = ?1 AND dedup_key = ?2",
    )?
    .execute(params![source, dedup_key, tokens])?;
    Ok(())
}

/// 经本地代理归账到某条连接的请求数；为 0 时界面不提供「仅此连接」统计范围。
#[cfg(test)]
pub fn attributed_request_count(conn: &Connection, connection_id: &str) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM requests WHERE connection_id = ?1", [connection_id], |r| r.get(0))
}

pub fn cleanup_preview(conn: &Connection, cutoff_ms: i64) -> rusqlite::Result<CleanupPreview> {
    let requests = conn.query_row(
        "SELECT COUNT(*) FROM requests WHERE ts < ?1",
        params![cutoff_ms],
        |row| row.get(0),
    )?;
    let usage_events = conn.query_row(
        "SELECT COUNT(*) FROM usage_events
         WHERE ts < ?1 OR request_id IN (SELECT id FROM requests WHERE ts < ?1)",
        params![cutoff_ms],
        |row| row.get(0),
    )?;
    let sessions = conn.query_row(
        "SELECT COUNT(*) FROM sessions s
         WHERE s.is_running = 0
           AND s.activity_updated_ms IS NOT NULL
           AND s.activity_updated_ms < ?1
           AND NOT EXISTS (
             SELECT 1 FROM requests r
             WHERE r.platform = s.platform AND r.session_id = s.session_id AND r.ts >= ?1
           )",
        params![cutoff_ms],
        |row| row.get(0),
    )?;
    Ok(CleanupPreview { cutoff_ms, requests, usage_events, sessions })
}

pub fn cleanup_history(conn: &mut Connection, cutoff_ms: i64) -> rusqlite::Result<CleanupPreview> {
    let preview = cleanup_preview(conn, cutoff_ms)?;
    let tx = conn.transaction()?;
    tx.execute("UPDATE collection_policy SET cutoff_ms=max(cutoff_ms,?1) WHERE singleton=1",[cutoff_ms])?;
    tx.execute(
        "DELETE FROM usage_events WHERE ts < ?1 OR request_id IN (SELECT id FROM requests WHERE ts < ?1)",
        params![cutoff_ms],
    )?;
    tx.execute("DELETE FROM native_context WHERE ts < ?1",[cutoff_ms])?;
    tx.execute("DELETE FROM request_metrics WHERE request_id IN (SELECT id FROM requests WHERE ts < ?1)", params![cutoff_ms])?;
    tx.execute("DELETE FROM requests WHERE ts < ?1", params![cutoff_ms])?;
    tx.execute(
        "DELETE FROM sessions
         WHERE is_running = 0
           AND activity_updated_ms IS NOT NULL
           AND activity_updated_ms < ?1
           AND NOT EXISTS (
             SELECT 1 FROM requests r
             WHERE r.platform = sessions.platform AND r.session_id = sessions.session_id
           )",
        params![cutoff_ms],
    )?;
    // scan_state 故意保留：清理后的历史日志不会在下一轮采集时重新导入。
    tx.commit()?;
    let _ = conn.execute_batch("PRAGMA optimize;");
    Ok(preview)
}


/// 批量写入；重复的 (source, dedup_key) 直接忽略 —— 去重在数据库层做。
#[cfg(test)]
pub fn insert_records(conn: &mut Connection, recs: &[RequestRecord]) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    let inserted = insert_records_into(&tx, recs)?;
    tx.commit()?;
    Ok(inserted)
}

/// 网关订阅没有原工具日志可扫描，直接保存无正文的请求统计并绑定所选账号。
pub(crate) fn insert_gateway_record_with_timing(
    conn: &mut Connection,
    record: &RequestRecord,
    connection_id: &str,
    duration_ms: i64,
    first_token_ms: Option<i64>,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    insert_records_into(&tx, std::slice::from_ref(record))?;
    tx.execute(
        "UPDATE requests SET connection_id=?3, duration_ms=?4, first_token_ms=COALESCE(?5, first_token_ms), status_code=200 WHERE source=?1 AND dedup_key=?2",
        params![record.source, record.dedup_key, connection_id, duration_ms, first_token_ms],
    )?;
    tx.commit()
}

/// 单个日志文件的一次扫描提交。请求、标题、生命周期与字节偏移必须同事务成功，
/// 否则回滚全部内容，下次从原偏移重读，避免“偏移已前进但记录未入库”。
#[cfg(test)]
pub fn commit_scanned_file(
    conn: &mut Connection,
    recs: &[RequestRecord],
    titles: &[(String, String, String)],
    activity: Option<(&str, &str, &str, i64)>,
    path: &str,
    offset: i64,
    mtime: i64,
) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    let inserted = insert_records_into(&tx, recs)?;
    for (platform, session_id, title) in titles {
        upsert_session_title(&tx, platform, session_id, title)?;
    }
    if let Some((platform, session_id, state, updated_at_ms)) = activity {
        set_session_activity(&tx, platform, session_id, state, updated_at_ms)?;
    }
    set_offset(&tx, path, offset, mtime)?;
    tx.commit()?;
    Ok(inserted)
}

