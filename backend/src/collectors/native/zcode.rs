use std::collections::{HashMap, HashSet};
use std::path::Path;
use rusqlite::params;
use serde_json::Value;

use crate::db::Store;
use crate::source_store::{self, Record};
use super::{db_stamps, db_unchanged, readonly, save_db_stamps, save_stamp, Source};

pub(crate) const ZCODE_RUNNING_GRACE_MS: i64 = 10 * 60 * 1_000;

/// 展示已采集的状态时，旧的运行状态也会随时间过期。
pub(crate) fn zcode_observed_state(state: Option<&str>, updated: Option<i64>, now_ms: i64) -> &'static str {
    match (state, updated) {
        (Some("running"), Some(ts)) if ts <= now_ms + 60_000 && now_ms.saturating_sub(ts) <= ZCODE_RUNNING_GRACE_MS => "running",
        (Some("done"), Some(ts)) if ts <= now_ms + 60_000 => "done",
        (Some("failed"), Some(ts)) if ts <= now_ms + 60_000 => "failed",
        _ => "recent",
    }
}

/// `session.time_updated` 比 `model_usage` 最新一行新出多少才算「有新活动」。
///
/// ZCode 只在请求结束时写 model_usage，一轮对话进行中（下一个请求在流式跑）时，
/// 最后一行还是上一条请求的 completed——单看行状态永远显示「已完成」。
/// `session.time_updated` 随会话活动实时前进（新消息、流式 part 都会写），
/// 是行数据之外唯一的运行痕迹。实测（2026-11-14）：一轮结束后 usage 行总是最后
/// 落库，`time_updated − usage` 只有 13–33ms；正在执行的会话差值持续增长。
/// 2 秒能干净分开这两种形态，不会把「请求刚结束的收尾写库」误判成新一轮。
pub(crate) const ZCODE_TURN_GAP_MS: i64 = 2_000;

pub(crate) fn zcode_session_state(status: &str, ts: i64, now_ms: i64) -> &'static str {
    match status {
        "completed" => "done",
        "error" | "cancelled" => "failed",
        _ if ts <= now_ms + 60_000 && now_ms - ts <= ZCODE_RUNNING_GRACE_MS => "running",
        _ => "unknown",
    }
}

/// ZCode `model_usage.status` → `requests.native_outcome`。
///
/// 值域是四值 `success` / `failed` / `cancelled` / `unknown`，与备份导入校验
/// （`db::metric_valid`）和用量统计的失败口径保持一致。`cancelled` 单独成一档，
/// 而不是并进 `failed`：
///
/// - 取消是用户主动中止（或客户端收尾），不是上游错误；并进失败会让「失败次数」
///   和「成功率」同时失真（2026-11-21 本机 1599 条里只有 2 条 error，却因为 3 条
///   cancelled 显示成 5 次失败）。
/// - 也不并进 `success`：没有完成的请求不能算成功。
/// - `running` / `pending` 等还没有结论的行保持 `unknown`，不猜成功。
///
/// 会话状态仍按 [`zcode_session_state`] 折叠成 done / failed / running：
/// 会话卡片只说「这一轮结束得怎么样」，不区分取消与报错。
pub(crate) fn zcode_native_outcome(status: &str) -> &'static str {
    match status {
        "completed" => "success",
        "error" => "failed",
        "cancelled" => "cancelled",
        _ => "unknown",
    }
}

/// 在行状态之上叠加会话级信号：最新一行的状态不是 running、但 `time_updated`
/// 比它新出 [`ZCODE_TURN_GAP_MS`] 且仍在宽限期内，说明上一条请求结束后会话又有了
/// 新活动（新一轮已经开跑），判为 running。CLI 异常退出时 `time_updated` 一并
/// 停走，同样被宽限期拦住，不会长期假running。
pub(crate) fn zcode_boost_running(
    state: &str,
    latest_row_ts: i64,
    session_updated_ms: Option<i64>,
    now_ms: i64,
) -> &'static str {
    if state == "running" {
        return "running";
    }
    match session_updated_ms {
        Some(updated)
            if updated > latest_row_ts + ZCODE_TURN_GAP_MS
                && updated <= now_ms + 60_000
                && now_ms - updated <= ZCODE_RUNNING_GRACE_MS =>
        {
            "running"
        }
        _ => match state {
            "done" => "done",
            "failed" => "failed",
            _ => "unknown",
        },
    }
}

/// Qoder 桌面客户端用量账本的一轮扫描（C-3：主库与 WAL 都没变时直接跳过）。
///
/// 原先每轮都把整张 `chat_message` 读出、逐行解析、再整批写回，账本越大越慢，
/// 而且写回期间一直占着数据库锁。现在沿用 ZCode 的内容戳记：变了才整份重建。
/// 戳记与记录在同一个事务里提交，写入失败时戳记不前进，下一轮会重读。
///

pub(crate) type ZcodeRow = (
    Record,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

pub(crate) fn read_zcode_rows(source: &Source) -> Result<Vec<ZcodeRow>, String> {
    let input = readonly(&source.path)?;
    // 旧版 ZCode 的 session 表可能没有 directory 列：探测后决定是否取它，
    // 缺列时用 NULL 占位，保持列序不变（同一次查询内不能混用两种投影）。
    let has_directory: bool = input
        .prepare("SELECT COUNT(*) FROM pragma_table_info('session') WHERE name = 'directory'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .map(|count| count > 0)
        .unwrap_or(false);
    let directory_expr = if has_directory { "s.directory" } else { "NULL" };
    // provider_id 是归因的关键端点事实（本机 model_usage 100% 有值）。旧版 Zcode 库可能没有
    // 这一列：与 directory 同样的探测方式，缺列时用 NULL 占位，列序不变。
    let has_provider: bool = input
        .prepare("SELECT COUNT(*) FROM pragma_table_info('model_usage') WHERE name = 'provider_id'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .map(|count| count > 0)
        .unwrap_or(false);
    let provider_expr = if has_provider { "u.provider_id" } else { "NULL" };
    // time_updated 是会话级运行信号（见 ZCODE_TURN_GAP_MS 的说明）：旧版库缺列时
    // 用 NULL 占位，行为退回「只看行状态」，与 directory / provider 同一套探测方式。
    let has_time_updated: bool = input
        .prepare("SELECT COUNT(*) FROM pragma_table_info('session') WHERE name = 'time_updated'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .map(|count| count > 0)
        .unwrap_or(false);
    let time_updated_expr = if has_time_updated { "s.time_updated" } else { "NULL" };
    let mut stmt = input
        .prepare(&format!(
            "SELECT u.id,u.session_id,u.model_id,u.status,u.started_at,u.completed_at,
        u.input_tokens,u.output_tokens,u.cache_read_input_tokens,u.cache_creation_input_tokens,
        u.provider_total_tokens,u.computed_total_tokens,u.reasoning_tokens,u.raw_usage_json,s.title,{directory_expr},
        {provider_expr},{time_updated_expr}
        FROM model_usage u LEFT JOIN session s ON s.id=u.session_id",
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let session: String = row.get(1)?;
            let status: String = row.get(3)?;
            // 归日时间取 started_at（第 4 列），缺失时才回退 completed_at（第 5 列）。
            //
            // 为什么不用 completed_at（改前的做法）：ZCode 自己按 started_at 归日、
            // 也按它做 30 天清理（usage.ts 的保留期删除），用完成时间会让跨本地午夜的
            // 请求落到不同日期——本机实测有 6 条，按日对账时两边分布对不上。
            // ts 同时也是趋势桶、明细排序、价格时段估算与 CSV 导出的时间；
            // 改归日会一并影响这些，所以是有意的口径统一：以「请求何时发出」为准。
            let started_at = row.get::<_, Option<i64>>(4)?;
            let completed_at = row.get::<_, Option<i64>>(5)?;
            // 两列都为空说明这行的发生时间不可知：跳过而不是编一个当前时间冒充发生时刻
            let Some(ts) = started_at.or(completed_at) else {
                return Err(rusqlite::Error::InvalidColumnType(
                    4,
                    "started_at/completed_at 均为空，无法归日".into(),
                    rusqlite::types::Type::Null,
                ));
            };
            let raw: Option<String> = row.get(13)?;
            let raw: Value = raw
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(Value::Null);
            let known = raw.is_object() && raw.as_object().is_some_and(|v| !v.is_empty());
            let number = |i| -> rusqlite::Result<Option<i64>> {
                Ok(row
                    .get::<_, Option<i64>>(i)?
                    .filter(|n| *n >= 0 && (known || *n > 0)))
            };
            let input = number(6)?;
            let output = number(7)?;
            let cache_read = number(8)?;
            let cache_write = number(9)?;
            let provider_total = row.get::<_, Option<i64>>(10)?;
            let total = provider_total.or(if known { row.get(11)? } else { None });
            let semantics = if raw.get("prompt_tokens").is_some()
                || (raw.get("inputTokens").is_some()
                    && input
                        .zip(output)
                        .zip(total)
                        .is_some_and(|((i, o), t)| i + o == t))
            {
                "includes_cache"
            } else if raw.get("cache_creation_input_tokens").is_some()
                && input
                    .zip(output)
                    .zip(cache_read)
                    .zip(cache_write)
                    .zip(total)
                    .is_some_and(|((((i, o), r), w), t)| i + o + r + w == t)
            {
                "excludes_cache"
            } else {
                "unknown"
            };
            let duration_ms = completed_at
                .zip(started_at)
                .and_then(|(end, start)| end.checked_sub(start))
                .filter(|d| (10..=3_600_000).contains(d));
            Ok((
                Record {
                    request: crate::db::RequestRecord {
                        platform: "zcode".into(),
                        source: source.id.clone(),
                        dedup_key: id,
                        session_id: Some(format!("{}:{session}", source.id)),
                        ts,
                        model: row.get(2)?,
                        input_tokens: input,
                        output_tokens: output,
                        cache_read_tokens: cache_read,
                        cache_write_tokens: cache_write,
                        total_tokens: total,
                        effort: None,
                    },
                    input_semantics: semantics,
                    credits: None,
                    original_credits: None,
                    billable: None,
                    context_ratio: None,
                    reasoning: number(12)?,
                    tool: None,
                    duration_ms,
                    first_token_ms: None,
                },
                status,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, Option<String>>(16)?
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty()),
                row.get::<_, Option<i64>>(17)?,
                // 第 7 个元素：completed_at，**只给会话活跃判定用**。
                // 归日用 `ts`（started_at），但「这条会话最后一次真正写完是什么时候」必须是
                // 完成时间——否则 23:59 开始、00:01 完成的那条会被当成「一小时前就没动静」，
                // 活跃会话误判成空闲。两个时间各有用途，不能合并。
                completed_at,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    drop(input);
    Ok(rows)
}

pub(crate) fn zcode(db: &mut impl Store, source: &Source) -> Result<usize, String> {
    let key = source.path.to_string_lossy().to_string();
    let timestamp_migration = format!("{key}#zcode-started-at-v1");
    // 升级后重扫一次旧库，补上会话级状态时间与每条请求的源端耗时（completed_at - started_at）。
    let activity_migration = format!("{key}#zcode-activity-time-v2");
    let duration_migration = format!("{key}#zcode-duration-v1");
    let stamps = db_stamps(&source.path)?;
    if db.write(|conn| db_unchanged(conn, &key, stamps)
        && crate::db::get_scan_state(conn, &timestamp_migration) == Some((1, 1))
        && crate::db::get_scan_state(conn, &activity_migration) == Some((1, 1))
        && crate::db::get_scan_state(conn, &duration_migration) == Some((1, 1))) {
        return Ok(0);
    }
    let rows = read_zcode_rows(source)?;
    let mut sessions: HashMap<String, (&'static str, i64, Option<String>, Option<String>, Option<i64>)> = HashMap::new();
    let now_ms = chrono::Utc::now().timestamp_millis();
    for (r, status, title, directory, _, session_updated, finished_at) in &rows {
        let Some(id) = &r.request.session_id else { continue };
        // 会话行状态也按完成时间算：一行刚写完才算 done，started_at 只说明它何时开始
        let row_finished = finished_at.unwrap_or(r.request.ts);
        let state = zcode_session_state(status, row_finished, now_ms);
        let entry = sessions.entry(id.clone()).or_insert((state, row_finished, None, None, None));
        if row_finished >= entry.1 {
            entry.0 = state;
            entry.1 = row_finished;
        }
        if entry.2.is_none() {
            entry.2 = title.clone();
        }
        if entry.3.is_none() {
            entry.3 = directory.clone().filter(|d| !d.trim().is_empty());
        }
        if entry.4.is_none() {
            entry.4 = *session_updated;
        }
    }
    // 行状态只能反映「最后一条请求」，一轮进行中时它仍是上一条的 completed。
    // 会话级 time_updated 在这之后又有新活动、且仍在宽限期内，就是新一轮已经开跑。
    for entry in sessions.values_mut() {
        entry.0 = zcode_boost_running(entry.0, entry.1, entry.4, now_ms);
    }
    db.write(|conn| -> Result<usize, String> {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let records: Vec<Record> = rows.iter().map(|(r, _, _, _, _, _, _)| r.clone()).collect();
        let changed = source_store::write(&tx, &records).map_err(|e| e.to_string())?;
        let mut outcome = tx.prepare_cached("UPDATE requests SET native_outcome=?1 WHERE source=?2 AND dedup_key=?3 AND native_outcome IS NOT ?1")
            .map_err(|e| e.to_string())?;
        for (r, status, _, _, _, _, _) in &rows {
            outcome.execute(params![zcode_native_outcome(status),source.id,r.request.dedup_key])
                .map_err(|e| e.to_string())?;
        }
        drop(outcome);
        // provider 落独立列，不进 platform：归因层要拿它去查 provider_config 里的端点地址，
        // 平台字段只回答「哪套工具」，两者不能合并。旧行下一次扫描自然补齐。
        let mut provider = tx
            .prepare_cached("UPDATE requests SET provider_key=?3 WHERE source=?1 AND dedup_key=?2 AND provider_key IS NOT ?3")
            .map_err(|e| e.to_string())?;
        for (r, _, _, _, provider_key, _, _) in &rows {
            provider
                .execute(params![source.id, r.request.dedup_key, provider_key])
                .map_err(|e| e.to_string())?;
        }
        drop(provider);
        for (id, (state, ts, title, cwd, session_updated)) in &sessions {
            if let Some(title) = title {
                crate::db::upsert_session_title(&tx, "zcode", id, title).map_err(|e| e.to_string())?;
            }
            if let Some(cwd) = cwd {
                crate::db::set_session_cwd(&tx, "zcode", id, cwd).map_err(|e| e.to_string())?;
            }
            // 会话活动可能发生在上一条用量记录完成之后，保存可信的最新时间。
            // 后续状态过期时沿用这个时间，确保 set_session_activity 的时间顺序检查允许更新。
            let activity_ts = session_updated
                .filter(|updated| *updated <= now_ms + 60_000)
                .map_or(*ts, |updated| (*ts).max(updated));
            crate::db::set_session_activity(&tx, "zcode", id, state, activity_ts).map_err(|e| e.to_string())?;
        }
        save_db_stamps(&tx, &key, stamps).map_err(|e| e.to_string())?;
        save_stamp(&tx, &timestamp_migration, 1, 1).map_err(|e| e.to_string())?;
        save_stamp(&tx, &activity_migration, 1, 1).map_err(|e| e.to_string())?;
        save_stamp(&tx, &duration_migration, 1, 1).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(changed)
    })
}

pub fn backfill_zcode_cwd(db: &mut impl Store, home: &Path) -> usize {
    let path = home.join(".zcode/cli/db/db.sqlite");
    if !path.exists() {
        return 0;
    }
    let Ok(input) = readonly(&path) else { return 0 };
    let has_directory: bool = input
        .prepare("SELECT COUNT(*) FROM pragma_table_info('session') WHERE name = 'directory'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .map(|count| count > 0)
        .unwrap_or(false);
    if !has_directory {
        return 0;
    }
    let Ok(mut stmt) = input.prepare(
        "SELECT id, directory FROM session WHERE directory IS NOT NULL AND TRIM(directory) <> ''",
    ) else {
        return 0;
    };
    let rows: Vec<(String, String)> = match stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) {
        Ok(iter) => iter.flatten().collect(),
        Err(_) => return 0,
    };
    drop(stmt);
    drop(input);
    db.write(|conn| -> Result<usize, String> {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut updated = 0;
        for (session_id, directory) in &rows {
            let key = format!("native:zcode:main:{session_id}");
            // 只补空值：已有真实路径的会话不覆盖
            updated += tx
                .execute(
                    "UPDATE sessions SET cwd = ?2 WHERE platform = 'zcode' AND session_id = ?1 AND (cwd IS NULL OR cwd = '')",
                    params![key, directory],
                )
                .unwrap_or(0);
            let key_ai = format!("native:zcode:ai:{session_id}");
            updated += tx
                .execute(
                    "UPDATE sessions SET cwd = ?2 WHERE platform = 'zcode' AND session_id = ?1 AND (cwd IS NULL OR cwd = '')",
                    params![key_ai, directory],
                )
                .unwrap_or(0);
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(updated)
    })
    .unwrap_or(0)
}

/// 受控回填 ZCode 的 `native_outcome='cancelled'`。
///
/// 旧版本的映射把 ZCode 的 `cancelled` 与 `error` 一起写成 `failed`（2026-11-21 之前），
/// 而已入库的行不会再被扫描修正：`zcode()` 开头用 `db_unchanged` 比对主库与 WAL 的
/// 大小 / mtime，源库没变化就整段跳过；即使重扫，`source_store::write` 也不碰
/// `native_outcome` 这一列（它只写 Token / 时间等字段），状态单独靠后面的
/// `UPDATE requests SET native_outcome=...` 补写——而那个 UPDATE 只在 zcode() 真正
/// 跑起来时执行。
///
/// 这里读 ZCode 自己的 `model_usage` 拿原始状态，只把「原来存的是 failed、原始状态
/// 明确是 cancelled」的行改成 cancelled：
///
/// - 只更新状态列，**不删任何行**：ZCode 按 30 天清理旧 `model_usage`，Jolu 必须继续
///   保留这些历史账本（`docs/plans/2026-11-21-usage-data-accuracy-plan.md` 的 G5）。
///   ZCode 库里已经没有的行自然查不到，也就不在回填范围内，历史记录原样留存。
/// - 只动 `native_outcome='failed'` 的行：`success` / `unknown` 不动，避免把新扫描
///   已经写对的行再改一次；也不把 `error` 行误改成 cancelled。
/// - 只动 `source='native:zcode:main'` 且 `platform='zcode'`：账号来源
///   （`native:zcode:account`）只有凭据文件，没有请求行。曾经以为历史上有
///   `native:zcode:ai` 前缀，查证后那是 WorkBuddy AI 的实例名（`backfill_zcode_cwd`
///   里那两行属于历史笔误），ZCode 请求只写过 `main`，这里不扩大到不存在的来源。
/// - 幂等且只跑一次：成功后往 `migration_log` 写一条 `zcode_cancelled_backfill`，之后的
///   心跳扫描直接返回 0，不必反复读源库。标记只在**源库读取成功**后写，源库临时
///   打不开时留待下次心跳重试，不会因为一次失败永久放弃。
///   用 `migration_log` 而不是 `scan_state`：后者是逐文件采集位点，`session_threads`
///   与性能脚本会遍历它，塞一个非文件键没有意义。
///
/// 读取源库走只读连接，写本应用库走 `db.write` 事务；两步分开，源库打不开时直接返回 0，
/// 不影响启动流程。返回实际改动的行数。
pub fn backfill_zcode_cancelled(db: &mut impl Store, home: &Path) -> usize {
    // 带版本号：将来若还要修别的历史口径，换一个 kind 即可再跑一次。
    const KIND: &str = "zcode_cancelled_backfill";
    let already: bool = db.write(|conn| conn
        .query_row("SELECT EXISTS(SELECT 1 FROM migration_log WHERE kind=?1)", [KIND], |r| r.get::<_, bool>(0))
        .unwrap_or(false));
    if already {
        return 0;
    }
    let path = home.join(".zcode/cli/db/db.sqlite");
    if !path.exists() {
        return 0;
    }
    let Ok(input) = readonly(&path) else { return 0 };
    // 只有原始状态是 cancelled 的请求 id 才进入回填集合；表缺失或读取失败都当成
    // 「没有」并保留标记未写状态，不能因为这一步失败去猜状态。
    let Ok(mut stmt) = input.prepare("SELECT id FROM model_usage WHERE status = 'cancelled'") else {
        return 0;
    };
    let cancelled: HashSet<String> = match stmt.query_map([], |r| r.get::<_, String>(0))
        .and_then(|iter| iter.collect()) {
        Ok(rows) => rows,
        Err(_) => return 0,
    };
    drop(stmt);
    drop(input);
    db.write(|conn| -> usize {
        let Ok(tx) = conn.transaction() else { return 0 };
        let mut updated = 0;
        if !cancelled.is_empty() {
            // 先只取本库里仍是 failed 的 ZCode 行，逐条比对源库状态；命中才改。
            let Ok(mut pending) = tx.prepare(
                "SELECT id, dedup_key FROM requests
                 WHERE platform = 'zcode' AND source = 'native:zcode:main' AND native_outcome = 'failed'",
            ) else {
                return 0;
            };
            let rows: Vec<(i64, String)> = match pending.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .and_then(|iter| iter.collect()) {
                Ok(rows) => rows,
                Err(_) => return 0,
            };
            drop(pending);
            for (id, dedup_key) in rows {
                if !cancelled.contains(&dedup_key) {
                    continue;
                }
                // 再带一次 `native_outcome='failed'` 条件，避免与并发扫描抢写时把
                // 刚写好的 success / cancelled 覆盖掉。
                let Ok(count) = tx
                    .execute(
                        "UPDATE requests SET native_outcome = 'cancelled'
                         WHERE id = ?1 AND native_outcome = 'failed'",
                        params![id],
                    ) else { return 0 };
                updated += count;
            }
        }
        // 标记与回填同事务：要么都生效，要么都不生效，下次心跳重新尝试。
        let marked = tx.execute(
            "INSERT INTO migration_log(at_ms, kind, detail) VALUES(?1, ?2, ?3)",
            params![
                chrono::Utc::now().timestamp_millis(),
                KIND,
                format!("ZCode 取消状态回填：{updated} 行从 failed 改为 cancelled（只改状态，未删行）"),
            ],
        ).is_ok();
        if !marked || tx.commit().is_err() {
            return 0;
        }
        updated
    })
}
