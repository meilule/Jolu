//! 本地 Claude Code / Codex 会话记录增量采集。

use crate::db::{self, Store};
use chrono::DateTime;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub(crate) mod claude;
pub(crate) mod codex;
#[cfg(test)]
mod tests;

use claude::{
    apply_claude_value, claude_projects_dir, parse_claude_cache_write_1h, parse_claude_line,
    parse_claude_title, record_claude_timing_line, resolve_claude_start_ms, seed_claude_chain,
    ClaudeChainNode, ClaudeMsgTiming,
};
use codex::{apply_codex_file_state, is_codex_boundary, parse_codex_line};
#[cfg(test)]
use claude::apply_claude_file_state;
#[cfg(test)]
use codex::parse_codex_file_state;

pub const SOURCE_LOCAL: &str = crate::db::LOCAL_SESSION_SOURCE;

#[derive(Debug, Serialize, Default)]
pub struct ScanResult {
    pub files_scanned: usize,
    pub records_inserted: usize,
    pub claude_found: bool,
    pub codex_found: bool,
    pub errors: Vec<String>,
    /// 出错文件所属的平台（claude / codex）。采集异常按平台定界：
    /// 一个文件的扫描失败只说明该平台的会话状态暂不可判定，
    /// 不应把其他平台的会话也打成「状态未知」。
    pub failed_sources: Vec<String>,
}

pub(crate) fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

pub(crate) fn iso_to_millis(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
}

pub(crate) fn n(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

/// 递归收集某目录下的 .jsonl。目录判断用枚举结果自带的类型，不再逐项 stat。
pub(crate) fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let is_dir = match e.file_type() {
            Ok(kind) if kind.is_symlink() => p.is_dir(),
            Ok(kind) => kind.is_dir(),
            Err(_) => p.is_dir(),
        };
        if is_dir {
            collect_jsonl(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            out.push(p);
        }
    }
}

pub(crate) fn modified_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// 与 [`scan_file`] 的续读判定同口径：位点已追平且文件未被重写时，整份跳过，
/// 不必为它拿写锁。拿不到文件信息时交给 `scan_file` 报告真实错误。
/// 大小与修改时间取自文件句柄而非目录枚举：正在追加的文件，目录项里的大小可能滞后。
pub(crate) fn needs_scan(path: &Path, saved: Option<&db::ScanStamp>) -> bool {
    let Some(saved) = saved else { return true };
    let Ok(meta) = std::fs::metadata(path) else { return true };
    let size = meta.len() as i64;
    let rewritten = saved.offset > size
        || (saved.offset == size && saved.offset > 0 && saved.mtime != modified_ms(&meta));
    let rebuild = saved.offset > 0 && !saved.parser_initialized;
    rewritten || rebuild || saved.offset != size
}

/// 从上次偏移继续读；文件被截断或重写（当前大小 < 已读偏移）则整文件重扫
#[cfg(test)]
pub(crate) fn read_new_lines(conn: &Connection, path: &Path) -> std::io::Result<(Vec<String>, i64, i64)> {
    let key = path.to_string_lossy().to_string();
    let meta = std::fs::metadata(path)?;
    let size = meta.len() as i64;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let previous = db::get_scan_state(conn, &key);
    let mut offset = previous.map_or(0, |state| state.0);
    if offset > size {
        offset = 0; // 文件被重写，重新来过
    }
    if offset == size && previous.is_some_and(|(_, old_mtime)| old_mtime != mtime) {
        offset = 0; // 大小相同但修改时间变化：文件被等长替换
    }
    if offset == size {
        return Ok((vec![], offset, mtime));
    }

    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(offset as u64))?;
    let mut reader = BufReader::new(f);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    // 只消费到最后一个真实 LF；没有换行的尾部可能仍在写入，留给下次扫描。
    let complete_len = bytes.iter().rposition(|byte| *byte == b'\n').map_or(0, |i| i + 1);
    let mut lines = Vec::new();
    for raw in bytes[..complete_len].split(|byte| *byte == b'\n') {
        if raw.is_empty() { continue; }
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let line = std::str::from_utf8(raw)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if !line.trim().is_empty() { lines.push(line.to_string()); }
    }
    Ok((lines, offset + complete_len as i64, mtime))
}

/// Claude Code：`~/.claude/projects/<project>/<session>.jsonl`
///
/// assistant 行结构（实测）：
///   timestamp, sessionId, message.id, message.model, message.usage{
///     input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens }
///
/// 该来源**不提供** costUSD —— 对应界面上成本标「估算」。请求耗时 / 首字 / HTTP 码

#[derive(Default)]
pub(crate) struct SessionFileState {
    pub(crate) model: Option<String>,
    /// Codex 的思考强度只在 turn_context 上出现，随文件顺序继承到后续用量行
    pub(crate) effort: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) activity_state: Option<String>,
    pub(crate) updated_at_ms: Option<i64>,
    pub(crate) turn_started_ms: Option<i64>,
}

pub(crate) fn scan_files<I>(db: &mut impl Store, files: I) -> ScanResult
where
    I: IntoIterator<Item = (PathBuf, &'static str)>,
{
    let mut result = ScanResult::default();
    for (path, kind) in files {
        result.files_scanned += 1;
        // 每个文件单独持锁：文件之间让出写锁，长时间回填也不会堵住用户操作。
        match db.write(|conn| scan_file(conn, &path, kind)) {
            Ok(inserted) => result.records_inserted += inserted,
            Err(error) => {
                result.errors.push(format!(
                    "{}: {error}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
                result.failed_sources.push(kind.to_string());
            }
        }
    }

    result
}

pub(crate) const RECORD_BATCH_SIZE: usize = 256;
pub(crate) const MIN_ESTIMATED_DURATION_MS: i64 = 100;
pub(crate) const MAX_ESTIMATED_DURATION_MS: i64 = 60 * 60 * 1000;

pub(crate) fn estimated_duration_ms(start_ms: i64, end_ms: i64) -> Option<i64> {
    let diff = end_ms.checked_sub(start_ms)?;
    (MIN_ESTIMATED_DURATION_MS..=MAX_ESTIMATED_DURATION_MS)
        .contains(&diff)
        .then_some(diff)
}

/// 单个文件以固定大小批次解析并在同一事务内提交记录、解析上下文和字节偏移。
/// 失败时整个文件回滚；重试仍从旧偏移开始，数据库去重继续兜底。
pub(crate) fn scan_file(conn: &mut Connection, path: &Path, kind: &str) -> Result<usize, String> {
    let key = path.to_string_lossy().to_string();
    let meta = std::fs::metadata(path).map_err(|error| error.to_string())?;
    let size = meta.len() as i64;
    let mtime = modified_ms(&meta);
    let saved = db::get_scan_checkpoint(conn, &key).unwrap_or_default();
    let rewritten = saved.offset > size
        || (saved.offset == size && saved.offset > 0 && saved.mtime != mtime);
    // 旧版本未保存对应平台的解析上下文。升级后首次扫描从头安全重建一次，
    // 随后都只处理新增字节；请求去重保证这次迁移重扫不会重复计数。
    let rebuild_lifecycle = matches!(kind, "codex" | "claude") && saved.offset > 0 && !saved.parser_initialized;
    let offset = if rewritten || rebuild_lifecycle { 0 } else { saved.offset };
    if offset == size {
        return Ok(0);
    }

    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut claude_chain: HashMap<String, ClaudeChainNode> = HashMap::new();
    let mut claude_timings: HashMap<String, ClaudeMsgTiming> = HashMap::new();
    if kind == "claude" && offset > 0 {
        seed_claude_chain(&mut file, offset, &mut claude_chain, &mut claude_timings);
    }
    file.seek(SeekFrom::Start(offset as u64)).map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(file.take((size - offset) as u64));
    let mut parser = if matches!(kind, "codex" | "claude") && offset > 0 {
        SessionFileState {
            model: saved.parser_model,
            // 与 model 同样随位点恢复：思考强度只在 turn_context 上出现，
            // 续扫时若从空开始，到下一条 turn_context 之间的记录会平白缺值。
            effort: saved.parser_effort,
            session_id: saved.parser_session_id,
            activity_state: saved.parser_activity_state,
            updated_at_ms: saved.parser_updated_at_ms,
            turn_started_ms: None,
        }
    } else {
        SessionFileState::default()
    };
    let mut codex_boundary_ms: Option<i64> = if kind == "codex" && offset > 0 { parser.updated_at_ms } else { None };
    let mut pending_durations: Vec<(String, String, i64)> = Vec::new();
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    let mut batch = Vec::with_capacity(RECORD_BATCH_SIZE);
    let mut inserted = 0usize;
    let mut consumed = 0i64;
    let mut bytes = Vec::new();

    loop {
        bytes.clear();
        let read = reader.read_until(b'\n', &mut bytes).map_err(|error| error.to_string())?;
        if read == 0 || !bytes.ends_with(b"\n") {
            break;
        }
        consumed += read as i64;
        let raw = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        // 日志文件可能在 CLI 重写/追加的瞬间被观察到非 UTF-8 的中间行。
        // 该行无法构成 JSON，跳过它并继续消费后续完整行，避免一条坏行阻断整份日志。
        let line = match std::str::from_utf8(raw) {
            Ok(line) => line,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        // 每行只解析一次，标题、生命周期与用量记录共用同一份结构；
        // 工具输出动辄几十 KB，重复解析是回填时的主要开销。
        let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
        if kind == "claude" {
            if let Some(title) = parse_claude_title(&value) {
                if !title.title.is_empty() {
                    db::upsert_session_title(&tx, title.platform, &title.session_id, &title.title)
                        .map_err(|error| error.to_string())?;
                }
            }
            apply_claude_value(&mut parser, &value);
            record_claude_timing_line(&value, &mut claude_chain, &mut claude_timings);
        } else if kind == "codex" {
            apply_codex_file_state(&mut parser, &value);
            if is_codex_boundary(&value) {
                if let Some(ts) = value.get("timestamp").and_then(Value::as_str).and_then(iso_to_millis) {
                    codex_boundary_ms = Some(ts);
                }
            }
        }
        let record = match kind {
            "claude" => parse_claude_line(&value),
            _ => parse_codex_line(&value, &mut parser.model, &mut parser.effort),
        };
        let claude_cw_1h = if kind == "claude" {
            parse_claude_cache_write_1h(&value)
        } else {
            None
        };
        if let Some(record) = record {
            if kind == "codex" {
                if let Some(dur) = codex_boundary_ms.and_then(|start| estimated_duration_ms(start, record.ts)) {
                    pending_durations.push((record.source.clone(), record.dedup_key.clone(), dur));
                }
                codex_boundary_ms = Some(record.ts);
            }
            let key_for_1h = claude_cw_1h.map(|tokens| (record.source.clone(), record.dedup_key.clone(), tokens));
            batch.push(record);
            if batch.len() >= RECORD_BATCH_SIZE {
                inserted += db::insert_scanned_records(&tx, &batch)
                    .map_err(|error| error.to_string())?;
                batch.clear();
            }
            if let Some((source, dedup_key, tokens)) = key_for_1h {
                if !batch.is_empty() {
                    inserted += db::insert_scanned_records(&tx, &batch)
                        .map_err(|error| error.to_string())?;
                    batch.clear();
                }
                db::set_cache_write_1h(&tx, &source, &dedup_key, tokens)
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    if !batch.is_empty() {
        inserted += db::insert_scanned_records(&tx, &batch).map_err(|error| error.to_string())?;
    }
    if kind == "claude" {
        for (msg_id, timing) in &claude_timings {
            if timing.completed && timing.starts_at_first_block {
                if let (Some(start), Some(end)) = (
                    resolve_claude_start_ms(&claude_chain, timing.first_parent.as_deref()),
                    timing.end_ms,
                ) {
                    if let Some(dur) = estimated_duration_ms(start, end) {
                        pending_durations.push((SOURCE_LOCAL.into(), msg_id.clone(), dur));
                    }
                }
            }
        }
    }
    for (source, dedup_key, dur) in pending_durations {
        db::set_request_duration_if_missing(&tx, &source, &dedup_key, dur)
            .map_err(|error| error.to_string())?;
    }
    if matches!(kind, "codex" | "claude") {
        if let (Some(session_id), Some(state), Some(updated_at_ms)) = (
            parser.session_id.as_deref(),
            parser.activity_state.as_deref(),
            parser.updated_at_ms,
        ) {
            db::set_session_activity(&tx, kind, session_id, state, updated_at_ms)
                .map_err(|error| error.to_string())?;
            if let Some(started_at) = parser.turn_started_ms {
                db::set_session_turn_start(&tx, kind, session_id, started_at)
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    db::set_scan_checkpoint(&tx, &key, &db::ScanCheckpoint {
        offset: offset + consumed,
        mtime,
        parser_model: parser.model,
        parser_session_id: parser.session_id,
        parser_activity_state: parser.activity_state,
        parser_updated_at_ms: parser.updated_at_ms,
        parser_initialized: matches!(kind, "codex" | "claude"),
        parser_effort: parser.effort,
    }).map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(inserted)
}

pub fn scan(db: &mut impl Store) -> ScanResult {
    if crate::validation::isolated() {
        return ScanResult { errors: vec!["隔离验收不读取用户日志".into()], ..Default::default() };
    }
    let mut result = ScanResult::default();
    let Some(home) = home() else {
        result.errors.push("无法定位用户主目录".into());
        return result;
    };

    let claude_dir = claude_projects_dir(&home);
    let codex_home = crate::session_titles::codex_home().unwrap_or_else(|| home.join(".codex"));
    if let Err(e) = crate::session_titles::sync_codex_if_changed(db, &codex_home) {
        result.errors.push(e);
        result.failed_sources.push("codex".into());
    }
    let codex_dir = codex_home.join("sessions");
    result.claude_found = claude_dir.is_dir();
    result.codex_found = codex_dir.is_dir();

    let mut files: Vec<(PathBuf, &'static str)> = Vec::new();
    if result.claude_found {
        let mut paths = Vec::new();
        collect_jsonl(&claude_dir, &mut paths);
        files.extend(paths.into_iter().map(|path| (path, "claude")));
    }
    if result.codex_found {
        let mut paths = Vec::new();
        collect_jsonl(&codex_dir, &mut paths);
        files.extend(paths.into_iter().map(|path| (path, "codex")));
    }

    // 心跳时绝大多数文件没有新字节：一次读出全部位点，在锁外比对大小与修改时间，
    // 只把有变化的文件交给逐文件加锁的采集。
    let listed = files.len();
    let stamps = db.write(|conn| db::scan_stamps(conn)).unwrap_or_default();
    files.retain(|(path, _)| needs_scan(path, stamps.get(path.to_string_lossy().as_ref())));
    let scanned = scan_files(db, files);
    result.files_scanned = listed;
    result.records_inserted = scanned.records_inserted;
    result.errors.extend(scanned.errors);
    result.failed_sources.extend(scanned.failed_sources);
    merge(&mut result, crate::native_sources::scan(db));
    // 历史会话的项目目录回填：旧版本没有写 sessions.cwd，增量扫描不会重扫未变的库文件，
    // 这里补一次（只填空值）。失败不影响本次扫描结果。
    if let Some(home) = crate::creds::home() {
        let _ = crate::native_sources::backfill_zcode_cwd(db, &home);
        // 状态回填：旧版本把 ZCode 的 cancelled 与 error 一起写成 failed，重扫不会修正
        // （源库没变就跳过）。同样在心跳里做一次受控回填，只改状态、不删历史行；
        // 带版本标记，跑成功后不再重复读源库。
        let _ = crate::native_sources::backfill_zcode_cancelled(db, &home);
    }
    result
}

pub(crate) fn merge(result: &mut ScanResult, other: ScanResult) {
    result.files_scanned += other.files_scanned;
    result.records_inserted += other.records_inserted;
    result.errors.extend(other.errors);
    result.failed_sources.extend(other.failed_sources);
}

/// notify 已经给出具体变化路径时，只处理这些文件；路径必须位于已知 CLI
/// 会话目录内，避免把任意外部文件送进解析器。周期心跳仍调用 `scan` 全量补漏。
/// 本机来源（Gemini / Qoder / WorkBuddy / Zcode）只在变化落在其目录内时才扫描，
/// Claude / Codex 的每次写入不再连带重扫它们。
pub fn scan_paths(db: &mut impl Store, changed: impl IntoIterator<Item = PathBuf>) -> ScanResult {
    if crate::validation::isolated() {
        return ScanResult { errors: vec!["隔离验收不读取用户日志".into()], ..Default::default() };
    }
    let mut result = ScanResult::default();
    let Some(home) = home() else {
        result.errors.push("无法定位用户主目录".into());
        return result;
    };
    let claude_dir = claude_projects_dir(&home);
    let codex_home = crate::session_titles::codex_home().unwrap_or_else(|| home.join(".codex"));
    let codex_dir = codex_home.join("sessions");
    let index = codex_home.join("session_index.jsonl");
    result.claude_found = claude_dir.is_dir();
    result.codex_found = codex_dir.is_dir();

    let mut unique = HashSet::new();
    let mut files = Vec::new();
    let mut native = Vec::new();
    let mut sync_titles = false;
    for path in changed {
        if !unique.insert(path.clone()) {
            continue;
        }
        let is_jsonl = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"));
        if crate::native_sources::path_under(&path, &index) && path.file_name() == index.file_name() {
            sync_titles = true;
        } else if is_jsonl
            && crate::native_sources::path_under(&path, &claude_dir)
            && path.is_file()
        {
            files.push((path, "claude"));
        } else if is_jsonl
            && crate::native_sources::path_under(&path, &codex_dir)
            && path.is_file()
        {
            files.push((path, "codex"));
        } else {
            native.push(path);
        }
    }
    if sync_titles {
        if let Err(error) = crate::session_titles::sync_codex_if_changed(db, &codex_home) {
            result.errors.push(error);
            // 标题索引属于 Codex 来源，失败同样只定界到该平台
            result.failed_sources.push("codex".into());
        }
    }
    let scanned = scan_files(db, files);
    result.files_scanned = scanned.files_scanned;
    result.records_inserted = scanned.records_inserted;
    result.errors.extend(scanned.errors);
    result.failed_sources.extend(scanned.failed_sources);
    if !native.is_empty() {
        merge(&mut result, crate::native_sources::scan_changed(db, &native));
    }
    result
}
