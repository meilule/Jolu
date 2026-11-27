use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use rusqlite::{types::Value, Connection};

use super::parsers::zcode::{zcode_child_session, zcode_child_sessions};
use super::source::{read_cwd, source_files};
use super::types::{SessionFilter, SessionItem, SessionList, UNATTRIBUTED};

/// 正文搜索跳过过大的源文件
pub(crate) const SEARCH_FILE_LIMIT: u64 = 32 * 1024 * 1024;

/// Antigravity 对话记录标题缓存：按文件修改时间失效，用于已入库但尚未 rescan 的会话即时展示标题
pub(crate) static AGY_TITLE_CACHE: LazyLock<Mutex<HashMap<PathBuf, (u64, Option<String>)>>> =
    LazyLock::new(Default::default);

pub(crate) fn cached_antigravity_title(path: &Path) -> Option<String> {
    if crate::validation::isolated() {
        return None;
    }
    let stamp = crate::sessions::source::file_mtime_secs(path)?;
    if let Some((cached, title)) = AGY_TITLE_CACHE.lock().ok()?.get(path) {
        if *cached == stamp {
            return title.clone();
        }
    }
    let title = crate::antigravity_usage::read_transcript_title(path);
    if let Ok(mut cache) = AGY_TITLE_CACHE.lock() {
        cache.insert(path.to_path_buf(), (stamp, title.clone()));
    }
    title
}

pub(crate) fn file_contains(path: &Path, needle: &str) -> bool {
    if crate::validation::isolated() { return false; }
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if meta.len() > SEARCH_FILE_LIMIT {
        return false;
    }
    let mut text = String::new();
    std::fs::File::open(path).and_then(|mut f| f.read_to_string(&mut text)).is_ok() && text.to_lowercase().contains(needle)
}

pub fn list(conn: &Connection, f: &SessionFilter) -> rusqlite::Result<SessionList> {
    let mut sql = String::from(
        "SELECT r.platform, r.session_id, MIN(r.ts), MAX(r.ts), COUNT(*),
                SUM(CASE r.input_semantics WHEN 'includes_cache' THEN max(r.input_tokens - r.cache_read_tokens, 0) WHEN 'excludes_cache' THEN r.input_tokens ELSE 0 END),
                MIN(CASE r.input_semantics WHEN 'includes_cache' THEN r.input_known * r.cache_read_known WHEN 'excludes_cache' THEN r.input_known ELSE 0 END),
                SUM(r.cache_read_tokens), MIN(r.cache_read_known), SUM(r.cache_write_tokens), MIN(r.cache_write_known),
                SUM(r.output_tokens), MIN(r.output_known),
                MIN(r.connection_id), MAX(r.connection_id), SUM(r.connection_id IS NULL),
                s.title, s.activity_state, s.cwd, s.activity_updated_ms,
                MAX(CASE WHEN r.source LIKE 'native:zcode:%' THEN 1 ELSE 0 END)
         FROM requests r LEFT JOIN sessions s ON s.platform = r.platform AND s.session_id = r.session_id
         WHERE r.session_id IS NOT NULL AND r.session_id <> '' AND COALESCE(s.is_subagent, 0) = 0",
    );
    let mut args: Vec<Value> = Vec::new();
    if let Some(p) = &f.platform {
        sql.push_str(" AND r.platform = ?");
        args.push(Value::Text(p.clone()));
    }
    if let Some(ids) = &f.session_ids {
        if ids.is_empty() {
            return Ok(SessionList { sessions: Vec::new(), files: 0 });
        }
        sql.push_str(&format!(" AND r.session_id IN ({})", vec!["?"; ids.len()].join(",")));
        args.extend(ids.iter().cloned().map(Value::Text));
    }
    sql.push_str(" GROUP BY r.platform, r.session_id");
    let files = source_files(conn)?;
    // Antigravity 的对话记录在 `<root>/brain/<conv_id>/.system_generated/logs/transcript.jsonl`，
    // 统一在 list() 前发现一次根目录，避免按会话逐条遍历磁盘。
    let agy_roots = (!crate::validation::isolated())
        .then(crate::creds::home)
        .flatten()
        .map(|h| crate::antigravity_usage::discover_roots(&h))
        .unwrap_or_default();
    // ZCode 的子代理会话不当作主会话展示（规则见 `zcode_child_sessions`）；
    // 库不可读时不加这层过滤，宁可多显示也不要吞掉真实会话
    let zcode_children = zcode_child_sessions();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
        let known = |sum: usize, flag: usize| -> rusqlite::Result<Option<i64>> {
            Ok(if r.get::<_, Option<i64>>(flag)?.unwrap_or(0) != 0 { r.get(sum)? } else { None })
        };
        let platform: String = r.get(0)?;
        let min_conn: Option<String> = r.get(13)?;
        let max_conn: Option<String> = r.get(14)?;
        let nulls: i64 = r.get(15)?;
        Ok((
            SessionItem {
                session_id: r.get(1)?,
                first_ts: r.get(2)?,
                last_ts: r.get(3)?,
                requests: r.get(4)?,
                fresh_input: known(5, 6)?,
                cache_read: known(7, 8)?,
                cache_write: known(9, 10)?,
                output: known(11, 12)?,
                connection_id: if nulls == 0 && min_conn == max_conn { min_conn } else { None },
                title: r.get::<_, Option<String>>(16)?.filter(|t| !t.trim().is_empty()),
                // 库里的 cwd 优先（ZCode 这类库型来源没有源文件可读）；
                // 源文件能读到时会覆盖它（Claude / Codex 的 cwd 更权威）
                cwd: r.get::<_, Option<String>>(18)?.filter(|c| !c.trim().is_empty()),
                status: String::new(),
                has_source: false,
                platform,
            },
            r.get::<_, Option<String>>(17)?,
            r.get::<_, Option<i64>>(19)?,
            r.get::<_, i64>(20)? != 0,
        ))
    })?;
    let needle = f.search.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_lowercase);
    let mut used = std::collections::HashSet::new();
    let mut sessions = Vec::new();
    for row in rows {
        let (mut item, activity, activity_updated, zcode_native) = row?;
        if zcode_child_session(zcode_children.as_ref(), &item.platform, &item.session_id) {
            continue;
        }
        if f.since.is_some_and(|since| item.last_ts < since) {
            continue;
        }
        match f.connection.as_deref() {
            Some(UNATTRIBUTED) if item.connection_id.is_some() => continue,
            Some(id) if id != UNATTRIBUTED && item.connection_id.as_deref() != Some(id) => continue,
            _ => {}
        }
        let agy_file = (item.platform == "antigravity")
            .then(|| {
                files
                    .get(&(item.platform.clone(), item.session_id.clone()))
                    .cloned()
                    .or_else(|| crate::antigravity_usage::find_transcript_in(&agy_roots, &item.session_id))
            })
            .flatten();
        let file = agy_file
            .as_ref()
            .or_else(|| files.get(&(item.platform.clone(), item.session_id.clone())));
        item.has_source = file.is_some_and(|p| p.exists());
        if item.platform == "antigravity" && item.title.is_none() {
            if let Some(p) = file.filter(|_| item.has_source) {
                item.title = cached_antigravity_title(p);
            }
        }
        if item.platform != "antigravity" {
            if let Some(from_file) = file.filter(|_| item.has_source).and_then(|p| read_cwd(p)) {
                item.cwd = Some(from_file);
            }
        }
        if let Some(project) = &f.project {
            if item.cwd.as_deref().unwrap_or("") != project {
                continue;
            }
        }
        // 数据库来源也能提供状态。旧库中的默认 done 没有时间戳，不能当作真实事件。
        // ZCode / Antigravity 的运行状态可能在工具异常退出后停留，按采集器的宽限期保守降级。
        let database_activity = item.platform == "zcode" && zcode_native && activity_updated.is_some();
        item.status = if !item.has_source && !database_activity {
            if crate::platforms::native(&item.platform) { "recent".into() } else { "usage_only".into() }
        } else if database_activity {
            crate::native_sources::zcode_observed_state(activity.as_deref(), activity_updated, chrono::Utc::now().timestamp_millis()).into()
        } else if item.platform == "antigravity" && activity.as_deref() == Some("running") {
            crate::native_sources::zcode_observed_state(activity.as_deref(), activity_updated, chrono::Utc::now().timestamp_millis()).into()
        } else {
            match activity.as_deref() {
                Some("running") => "running".into(),
                Some("failed") => "failed".into(),
                Some("done") => "done".into(),
                _ => "recent".into(),
            }
        };
        if let Some(q) = &needle {
            let hit = item.session_id.to_lowercase().contains(q)
                || item.title.as_deref().is_some_and(|t| t.to_lowercase().contains(q))
                || file.is_some_and(|p| file_contains(p, q));
            if !hit {
                continue;
            }
        }
        if let Some(p) = file.filter(|_| item.has_source) {
            used.insert(p.clone());
        }
        sessions.push(item);
    }
    append_external_sessions(conn, f, &mut sessions);
    sessions.sort_by(|a, b| b.last_ts.cmp(&a.last_ts));
    Ok(SessionList { files: used.len() as i64, sessions })
}

pub(crate) fn append_external_sessions(conn: &Connection, f: &SessionFilter, sessions: &mut Vec<SessionItem>) {
    let mut seen_keys: std::collections::HashSet<(String, String)> = sessions
        .iter()
        .map(|s| (s.platform.clone(), s.session_id.clone()))
        .collect();
    let db_ingested: std::collections::HashSet<String> = conn
        .prepare("SELECT DISTINCT platform FROM requests WHERE platform IN ('kimi', 'pi', 'dsh')")
        .and_then(|mut q| {
            let rows = q.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect()
        })
        .unwrap_or_default();
    let external: Vec<SessionItem> = if f.connection.is_none() && !f.skip_external {
        match f.platform.as_deref() {
            None => crate::creds::home()
                .map(|home| {
                    let mut all = Vec::new();
                    if !db_ingested.contains("kimi") {
                        all.extend(crate::kimi_sessions::sessions(&home, f.since));
                    }
                    if !db_ingested.contains("pi") {
                        all.extend(crate::pi_sessions::sessions(&home, f.since));
                    }
                    if !db_ingested.contains("dsh") {
                        all.extend(crate::dsh_sessions::sessions(&home, f.since));
                    }
                    all
                })
                .unwrap_or_default(),
            Some("kimi") if !db_ingested.contains("kimi") => crate::creds::home()
                .map(|h| crate::kimi_sessions::sessions(&h, f.since))
                .unwrap_or_default(),
            Some("pi") if !db_ingested.contains("pi") => crate::creds::home()
                .map(|h| crate::pi_sessions::sessions(&h, f.since))
                .unwrap_or_default(),
            Some("dsh") if !db_ingested.contains("dsh") => crate::creds::home()
                .map(|h| crate::dsh_sessions::sessions(&h, f.since))
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    for item in external {
        if let Some(ids) = &f.session_ids {
            if !ids.contains(&item.session_id) {
                continue;
            }
        }
        if seen_keys.insert((item.platform.clone(), item.session_id.clone())) {
            sessions.push(item);
        }
    }
}
