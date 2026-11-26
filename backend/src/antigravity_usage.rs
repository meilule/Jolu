//! Antigravity CLI（`agy`）本地用量与会话元数据只读采集。
//!
//! 数据根目录自动发现（按本机实测 2026-11-26）：
//! - 环境变量覆盖：`ANTIGRAVITY_HOME` / `AGY_HOME`
//! - 官方默认目录：`~/.gemini/antigravity/`
//! - CCS 托管多账号目录：`~/.cc-switch/agy/<profile-id>/antigravity-cli/`
//!
//! 采集来源（纯只读、零新增第三方 crate）：
//! 1. `conversations/<conv-id>.db`（SQLite WAL）：读取 `steps` 表中 `step_type = 15`
//!    （`PLANNER_RESPONSE`）的 Protobuf `metadata` 二进制字段，精确提取：
//!    - field 1 (`created_at`) → 请求时间 `ts`
//!    - field 6 (`first_token_at`) → 首字延迟 `first_token_ms = t6 - t1`
//!    - field 7 / 8 (`completed_at`) → 总耗时 `duration_ms = t7 - t1`
//!    - field 9 (`model_usage`) → `9.1` model enum、`9.2` input（不含缓存）、
//!      `9.3` output、`9.5` cache_read、`9.7`/`9.11` 唯一 `responseId`
//! 2. `brain/<conv-id>/.system_generated/logs/transcript.jsonl`：
//!    按 `step_index` 关联人类可读模型名（如 `gemini-4-neon-xhigh`）与状态；
//!    当 `.db` 不存在时自动回退从 `transcript.jsonl` 采集用量。
//! 3. `conversation_summaries.db`：
//!    读取 `conversation_summaries` 表的标题与 `workspace_uris`，写入 `sessions.title` / `sessions.cwd`。
//!
//! 隐私：仅读用量统计、时间戳、模型名、会话标题与工作区路径，绝不读取对话正文、代码片段或凭证。

use crate::db::{RequestRecord, Store};
use crate::source_store::{self, Record};
use rusqlite::{params, Connection, OpenFlags};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// `steps.step_type` 中模型规划响应（`PLANNER_RESPONSE`）的枚举值。
const STEP_TYPE_PLANNER_RESPONSE: i64 = 15;

#[derive(Debug, Clone)]
pub struct Root {
    pub source_id: String,
    pub name: String,
    pub path: PathBuf,
}

/// 发现本机全部 Antigravity CLI 数据根目录（官方默认 + 环境变量 + CCS 托管多账号）。
/// 按目录规范路径去重，避免同一目录重复入库。
pub fn discover_roots(home: &Path) -> Vec<Root> {
    let mut candidates: Vec<(String, String, PathBuf)> = Vec::new();
    for env_key in ["ANTIGRAVITY_HOME", "AGY_HOME"] {
        if let Some(dir) = std::env::var_os(env_key).filter(|v| !v.is_empty()) {
            candidates.push(("env".into(), "Antigravity CLI".into(), PathBuf::from(dir)));
        }
    }
    candidates.push((
        "default".into(),
        "Antigravity CLI".into(),
        home.join(".gemini").join("antigravity"),
    ));

    let ccs_root = home.join(".cc-switch").join("agy");
    if let Ok(entries) = std::fs::read_dir(&ccs_root) {
        let mut profiles: Vec<(String, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let is_dir = entry.file_type().ok()?.is_dir();
                if !is_dir {
                    return None;
                }
                let profile = entry.file_name().to_string_lossy().into_owned();
                let cli_dir = entry.path().join("antigravity-cli");
                if cli_dir.is_dir() {
                    Some((profile, cli_dir))
                } else {
                    None
                }
            })
            .collect();
        profiles.sort_by(|a, b| a.0.cmp(&b.0));
        for (profile, cli_dir) in profiles {
            let short: String = profile.chars().take(8).collect();
            candidates.push((
                format!("ccs-{profile}"),
                format!("Antigravity CLI ({short})"),
                cli_dir,
            ));
        }
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (instance, name, path) in candidates {
        let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen.insert(key) {
            continue;
        }
        out.push(Root {
            source_id: format!("native:antigravity:{instance}"),
            name,
            path,
        });
    }
    out
}

/// 根目录是否包含可采集的 Antigravity 数据目录或摘要库。
pub fn root_has_data(root: &Path) -> bool {
    root.join("conversations").is_dir()
        || root.join("brain").is_dir()
        || root.join("conversation_summaries.db").is_file()
}

/// 供 `watcher` 实时监听的目录列表（含未创建目录与 `.cc-switch/agy` 兜底目录）。
pub fn watch_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |p: PathBuf| {
        if seen.insert(p.clone()) {
            dirs.push(p);
        }
    };
    for root in discover_roots(home) {
        push(root.path.join("conversations"));
        push(root.path.join("brain"));
    }
    push(home.join(".cc-switch").join("agy"));
    dirs
}

/// 打开只读 SQLite 连接，不写入也不阻塞源客户端。
fn readonly(path: &Path) -> Result<Connection, String> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .and_then(|c| {
        c.busy_timeout(std::time::Duration::from_millis(200))?;
        Ok(c)
    })
    .map_err(|e| format!("只读数据库无法打开：{e}"))
}

fn file_stamp(path: &Path) -> (i64, i64) {
    let Ok(m) = std::fs::metadata(path) else {
        return (0, 0);
    };
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    (m.len() as i64, mtime)
}

fn db_stamps(path: &Path) -> [(i64, i64); 2] {
    let wal = PathBuf::from(format!("{}-wal", path.to_string_lossy()));
    [file_stamp(path), if wal.exists() { file_stamp(&wal) } else { (0, 0) }]
}

fn save_stamp(conn: &Connection, key: &str, size: i64, time: i64) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT INTO scan_state(path,offset,mtime,parser_initialized) VALUES(?1,?2,?3,1)
         ON CONFLICT(path) DO UPDATE SET offset=?2,mtime=?3,parser_initialized=1",
    )?
    .execute(params![key, size, time])?;
    Ok(())
}

// ---------------- 轻量 Protobuf 线格式解析器 ----------------

fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut val = 0u64;
    let mut shift = 0u32;
    for _ in 0..10 {
        let byte = *buf.get(*pos)?;
        *pos += 1;
        val |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(val);
        }
        shift += 7;
    }
    None
}

enum FieldValue<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Skipped,
}

fn next_field<'a>(buf: &'a [u8], pos: &mut usize) -> Option<(u32, FieldValue<'a>)> {
    if *pos >= buf.len() {
        return None;
    }
    let tag = read_varint(buf, pos)?;
    let field_no = (tag >> 3) as u32;
    let wire_type = (tag & 0x07) as u8;
    let value = match wire_type {
        0 => FieldValue::Varint(read_varint(buf, pos)?),
        1 => {
            if *pos + 8 > buf.len() {
                return None;
            }
            *pos += 8;
            FieldValue::Skipped
        }
        2 => {
            let len = read_varint(buf, pos)? as usize;
            let end = pos.checked_add(len)?;
            if end > buf.len() {
                return None;
            }
            let slice = &buf[*pos..end];
            *pos = end;
            FieldValue::Bytes(slice)
        }
        5 => {
            if *pos + 4 > buf.len() {
                return None;
            }
            *pos += 4;
            FieldValue::Skipped
        }
        _ => return None,
    };
    Some((field_no, value))
}

fn parse_timestamp_ms(buf: &[u8]) -> Option<i64> {
    let mut pos = 0;
    let mut seconds: Option<i64> = None;
    let mut nanos: i64 = 0;
    while let Some((field, val)) = next_field(buf, &mut pos) {
        if let FieldValue::Varint(v) = val {
            match field {
                1 => seconds = Some(v as i64),
                2 => nanos = v as i64,
                _ => {}
            }
        }
    }
    let secs = seconds?;
    // 合理区间校验（2020-01-01 ~ 2100-01-01），避免把非时间戳子消息误判为时间
    if !(1_577_836_800..4_102_444_800).contains(&secs) {
        return None;
    }
    secs.checked_mul(1_000)?.checked_add(nanos / 1_000_000)
}

#[derive(Debug, Default)]
struct ProtoModelUsage {
    model_enum: Option<u64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    response_id: Option<String>,
}

fn parse_model_usage(buf: &[u8]) -> ProtoModelUsage {
    let mut out = ProtoModelUsage::default();
    let mut pos = 0;
    while let Some((field, val)) = next_field(buf, &mut pos) {
        match (field, val) {
            (1, FieldValue::Varint(v)) => out.model_enum = Some(v),
            (2, FieldValue::Varint(v)) => out.input_tokens = i64::try_from(v).ok(),
            (3, FieldValue::Varint(v)) => out.output_tokens = i64::try_from(v).ok(),
            (5, FieldValue::Varint(v)) => out.cache_read_tokens = i64::try_from(v).ok(),
            (7 | 11, FieldValue::Bytes(raw)) => {
                if out.response_id.is_none() {
                    if let Ok(s) = std::str::from_utf8(raw) {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            out.response_id = Some(trimmed.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[derive(Debug, Default)]
struct StepMetadata {
    created_at_ms: Option<i64>,
    first_token_at_ms: Option<i64>,
    completed_at_ms: Option<i64>,
    usage: Option<ProtoModelUsage>,
}

fn parse_step_metadata(buf: &[u8]) -> StepMetadata {
    let mut out = StepMetadata::default();
    let mut completed_8: Option<i64> = None;
    let mut pos = 0;
    while let Some((field, val)) = next_field(buf, &mut pos) {
        if let FieldValue::Bytes(sub) = val {
            match field {
                1 => out.created_at_ms = parse_timestamp_ms(sub),
                6 => out.first_token_at_ms = parse_timestamp_ms(sub),
                7 => out.completed_at_ms = parse_timestamp_ms(sub),
                8 => completed_8 = parse_timestamp_ms(sub),
                9 => out.usage = Some(parse_model_usage(sub)),
                _ => {}
            }
        }
    }
    if out.completed_at_ms.is_none() {
        out.completed_at_ms = completed_8;
    }
    out
}

// ---------------- Transcript & Summaries 关联 ----------------

#[derive(Debug, Clone)]
struct TranscriptStep {
    step_index: i64,
    model: Option<String>,
    status: Option<String>,
    created_at_ms: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
}

fn iso_to_ms(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.timestamp_millis())
}

fn transcript_path(root: &Path, conv_id: &str) -> PathBuf {
    root.join("brain")
        .join(conv_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl")
}

fn read_transcript(path: &Path) -> Vec<TranscriptStep> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("PLANNER_RESPONSE") {
            continue;
        }
        let Some(step_index) = v.get("step_index").and_then(Value::as_i64) else {
            continue;
        };
        let num = |k: &str| v.get(k).and_then(Value::as_i64).filter(|n| *n >= 0);
        out.push(TranscriptStep {
            step_index,
            model: v
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
            status: v
                .get("status")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
            created_at_ms: v
                .get("created_at")
                .and_then(Value::as_str)
                .and_then(iso_to_ms),
            input_tokens: num("input_tokens"),
            output_tokens: num("output_tokens"),
            cache_read_tokens: num("cache_read_tokens"),
        });
    }
    out
}

fn map_outcome(status: Option<&str>) -> Option<&'static str> {
    match status?.to_ascii_uppercase().as_str() {
        "DONE" | "COMPLETED" | "SUCCESS" => Some("success"),
        "ERROR" | "FAILED" => Some("failed"),
        "CANCELED" | "CANCELLED" => Some("cancelled"),
        "RUNNING" | "PENDING" | "GENERATING" => Some("unknown"),
        _ => None,
    }
}

/// 将 `file:///D:/develop/Jolu` 解码成本地目录路径 `D:\develop\Jolu`（跨平台兼容）。
pub(crate) fn decode_file_uri(uri: &str) -> Option<String> {
    let raw = uri.trim();
    let rest = raw
        .strip_prefix("file:///")
        .or_else(|| raw.strip_prefix("file://"))?;
    let bytes = percent_decode(rest);
    let mut decoded = String::from_utf8(bytes).ok()?;
    if decoded.is_empty() {
        return None;
    }
    // Windows 盘符路径（如 `D:/develop/Jolu`）：统一分隔符
    if cfg!(windows) {
        decoded = decoded.replace('/', "\\");
    } else if !decoded.starts_with('/') {
        decoded.insert(0, '/');
    }
    Some(decoded)
}

fn percent_decode(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn parse_first_workspace_cwd(raw: Option<&str>) -> Option<String> {
    let text = raw?.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) {
        for item in items {
            if let Some(uri) = item.as_str() {
                if let Some(path) = decode_file_uri(uri) {
                    return Some(path);
                }
            }
        }
    }
    decode_file_uri(text)
}

/// 单会话读取结果（记录 + 可选原生结果状态）。
struct ParsedStep {
    record: Record,
    outcome: Option<&'static str>,
}

fn read_conversation_db(
    db_path: &Path,
    transcript_steps: &[TranscriptStep],
    source_id: &str,
    conv_id: &str,
) -> Result<Vec<ParsedStep>, String> {
    let conn = readonly(db_path)?;
    let has_steps: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='steps')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !has_steps {
        return Ok(Vec::new());
    }

    let transcript_by_idx: HashMap<i64, &TranscriptStep> = transcript_steps
        .iter()
        .map(|s| (s.step_index, s))
        .collect();

    let mut stmt = conn
        .prepare("SELECT idx, metadata FROM steps WHERE step_type = ?1 AND metadata IS NOT NULL ORDER BY idx")
        .map_err(|e| e.to_string())?;
    let raw_rows: Vec<(i64, Vec<u8>)> = stmt
        .query_map([STEP_TYPE_PLANNER_RESPONSE], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;

    // 第一遍：利用同一会话内已有 step_index 命中的记录建立 model_enum → model_name 映射，
    // 防止个别步骤未写入精简版 transcript 时丢掉模型名。
    let parsed_meta: Vec<(i64, StepMetadata)> = raw_rows
        .into_iter()
        .map(|(idx, blob)| (idx, parse_step_metadata(&blob)))
        .collect();
    let mut enum_to_model: HashMap<u64, String> = HashMap::new();
    for (idx, meta) in &parsed_meta {
        if let (Some(u), Some(tr)) = (&meta.usage, transcript_by_idx.get(idx)) {
            if let (Some(code), Some(model)) = (u.model_enum, tr.model.as_ref()) {
                enum_to_model.entry(code).or_insert_with(|| model.clone());
            }
        }
    }

    let session_id = format!("{source_id}:{conv_id}");
    let mut out = Vec::new();
    for (idx, meta) in parsed_meta {
        let Some(usage) = meta.usage else { continue };
        let tr = transcript_by_idx.get(&idx).copied();
        let has_any_token = usage.input_tokens.is_some_and(|v| v > 0)
            || usage.output_tokens.is_some_and(|v| v > 0)
            || usage.cache_read_tokens.is_some_and(|v| v > 0);
        if !has_any_token {
            continue;
        }
        let Some(ts) = meta.created_at_ms.or_else(|| tr.and_then(|t| t.created_at_ms)) else {
            continue;
        };
        // Proto3 会省略值为 0 的标量字段（例如冷启动首轮 cache_read_tokens = 0）。
        // 只要 model_usage 子消息存在且含正数用量，缺省的输入/输出/缓存读即为确定值 0。
        let input = usage.input_tokens.unwrap_or(0);
        let output = usage.output_tokens.unwrap_or(0);
        let cache_read = usage.cache_read_tokens.unwrap_or(0);
        let total = input.saturating_add(output).saturating_add(cache_read);

        let first_token_ms = meta
            .first_token_at_ms
            .and_then(|t6| t6.checked_sub(ts))
            .filter(|d| (1..=3_600_000).contains(d));
        let duration_ms = meta
            .completed_at_ms
            .and_then(|t7| t7.checked_sub(ts))
            .filter(|d| (10..=3_600_000).contains(d));

        let model = tr
            .and_then(|t| t.model.clone())
            .or_else(|| usage.model_enum.and_then(|code| enum_to_model.get(&code).cloned()));
        let dedup_key = usage
            .response_id
            .unwrap_or_else(|| format!("{conv_id}:step:{idx}"));

        out.push(ParsedStep {
            record: Record {
                request: RequestRecord {
                    platform: "antigravity".into(),
                    source: source_id.into(),
                    dedup_key,
                    session_id: Some(session_id.clone()),
                    ts,
                    model,
                    input_tokens: Some(input),
                    output_tokens: Some(output),
                    cache_read_tokens: Some(cache_read),
                    cache_write_tokens: None,
                    total_tokens: Some(total),
                    effort: None,
                },
                input_semantics: "excludes_cache",
                credits: None,
                original_credits: None,
                billable: None,
                context_ratio: None,
                reasoning: None,
                tool: None,
                duration_ms,
                first_token_ms,
            },
            outcome: map_outcome(tr.and_then(|t| t.status.as_deref())),
        });
    }
    Ok(out)
}

fn read_transcript_fallback(
    steps: &[TranscriptStep],
    source_id: &str,
    conv_id: &str,
) -> Vec<ParsedStep> {
    let session_id = format!("{source_id}:{conv_id}");
    let mut out = Vec::new();
    for tr in steps {
        let has_any = tr.input_tokens.is_some_and(|v| v > 0)
            || tr.output_tokens.is_some_and(|v| v > 0)
            || tr.cache_read_tokens.is_some_and(|v| v > 0);
        if !has_any {
            continue;
        }
        let Some(ts) = tr.created_at_ms else { continue };
        let input = tr.input_tokens.unwrap_or(0);
        let output = tr.output_tokens.unwrap_or(0);
        let cache_read = tr.cache_read_tokens.unwrap_or(0);
        let total = input.saturating_add(output).saturating_add(cache_read);
        out.push(ParsedStep {
            record: Record {
                request: RequestRecord {
                    platform: "antigravity".into(),
                    source: source_id.into(),
                    dedup_key: format!("{conv_id}:step:{}", tr.step_index),
                    session_id: Some(session_id.clone()),
                    ts,
                    model: tr.model.clone(),
                    input_tokens: Some(input),
                    output_tokens: Some(output),
                    cache_read_tokens: Some(cache_read),
                    cache_write_tokens: None,
                    total_tokens: Some(total),
                    effort: None,
                },
                input_semantics: "excludes_cache",
                credits: None,
                original_credits: None,
                billable: None,
                context_ratio: None,
                reasoning: None,
                tool: None,
                duration_ms: None,
                first_token_ms: None,
            },
            outcome: map_outcome(tr.status.as_deref()),
        });
    }
    out
}

/// 同步 `conversation_summaries.db` 的会话标题与项目工作区路径（按主库 + WAL 戳记增量跳过）。
fn sync_summaries(db: &mut impl Store, root: &Root) -> Result<(), String> {
    let summary_path = root.path.join("conversation_summaries.db");
    if !summary_path.is_file() {
        return Ok(());
    }
    let key = format!("ag-summary:{}", summary_path.to_string_lossy());
    let wal_key = format!("{key}-wal");
    let stamps = db_stamps(&summary_path);
    let unchanged = db.write(|conn| {
        crate::db::get_scan_state(conn, &key) == Some(stamps[0])
            && crate::db::get_scan_state(conn, &wal_key) == Some(stamps[1])
    });
    if unchanged {
        return Ok(());
    }

    let input = readonly(&summary_path)?;
    let exists: bool = input
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='conversation_summaries')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !exists {
        return Ok(());
    }
    let mut stmt = input
        .prepare("SELECT conversation_id, title, workspace_uris, last_modified_time FROM conversation_summaries")
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, Option<String>, Option<String>, Option<String>)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1).ok().flatten(),
                r.get::<_, Option<String>>(2).ok().flatten(),
                r.get::<_, Option<String>>(3).ok().flatten(),
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    drop(input);

    db.write(|conn| -> Result<(), String> {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (conv_id, title, uris, last_mod) in rows {
            let sid = format!("{}:{conv_id}", root.source_id);
            if let Some(title) = title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
                crate::db::upsert_session_title(&tx, "antigravity", &sid, &title)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(cwd) = parse_first_workspace_cwd(uris.as_deref()) {
                crate::db::set_session_cwd(&tx, "antigravity", &sid, &cwd)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(ts) = last_mod.as_deref().and_then(iso_to_ms) {
                crate::db::set_session_activity(&tx, "antigravity", &sid, "done", ts)
                    .map_err(|e| e.to_string())?;
            }
        }
        save_stamp(&tx, &key, stamps[0].0, stamps[0].1).map_err(|e| e.to_string())?;
        save_stamp(&tx, &wal_key, stamps[1].0, stamps[1].1).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// 增量扫描单个 Antigravity 数据根目录，返回 `(变更条数, 非致命跳过说明)`。
pub fn scan_root(db: &mut impl Store, root: &Root) -> Result<(usize, Vec<String>), String> {
    let mut changed = 0usize;
    let mut errors = Vec::new();
    let mut handled_convs: HashSet<String> = HashSet::new();

    // 1. 优先扫描 conversations/<conv_id>.db（含精确首字与总耗时）
    let conv_dir = root.path.join("conversations");
    if conv_dir.is_dir() {
        let mut db_files: Vec<(String, PathBuf)> = std::fs::read_dir(&conv_dir)
            .map_err(|e| format!("无法读取 Antigravity 会话目录：{e}"))?
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("db") {
                    return None;
                }
                let conv_id = path.file_stem()?.to_str()?.to_string();
                Some((conv_id, path))
            })
            .collect();
        db_files.sort_by(|a, b| a.0.cmp(&b.0));

        for (conv_id, db_path) in db_files {
            handled_convs.insert(conv_id.clone());
            let tr_path = transcript_path(&root.path, &conv_id);
            let stamps = db_stamps(&db_path);
            let tr_stamp = if tr_path.is_file() { file_stamp(&tr_path) } else { (0, 0) };
            let key = db_path.to_string_lossy().to_string();
            let wal_key = format!("{key}-wal");
            let tr_key = format!("{key}#tr");

            let unchanged = db.write(|conn| {
                crate::db::get_scan_state(conn, &key) == Some(stamps[0])
                    && crate::db::get_scan_state(conn, &wal_key) == Some(stamps[1])
                    && crate::db::get_scan_state(conn, &tr_key) == Some(tr_stamp)
            });
            if unchanged {
                continue;
            }

            let tr_steps = if tr_path.is_file() {
                read_transcript(&tr_path)
            } else {
                Vec::new()
            };
            let parsed = match read_conversation_db(&db_path, &tr_steps, &root.source_id, &conv_id) {
                Ok(rows) => rows,
                Err(err) => {
                    errors.push(format!("{conv_id}.db：{err}"));
                    continue;
                }
            };

            let n = db
                .write(|conn| -> Result<usize, String> {
                    let tx = conn.transaction().map_err(|e| e.to_string())?;
                    let records: Vec<Record> = parsed.iter().map(|p| p.record.clone()).collect();
                    let written = source_store::write(&tx, &records).map_err(|e| e.to_string())?;
                    let mut outcome_stmt = tx
                        .prepare_cached(
                            "UPDATE requests SET native_outcome=?1 WHERE source=?2 AND dedup_key=?3 AND native_outcome IS NOT ?1",
                        )
                        .map_err(|e| e.to_string())?;
                    let mut latest_ts: Option<i64> = None;
                    for p in &parsed {
                        latest_ts = Some(latest_ts.map_or(p.record.request.ts, |prev| prev.max(p.record.request.ts)));
                        if let Some(outcome) = p.outcome {
                            outcome_stmt
                                .execute(params![outcome, p.record.request.source, p.record.request.dedup_key])
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    drop(outcome_stmt);
                    if let Some(ts) = latest_ts {
                        let sid = format!("{}:{conv_id}", root.source_id);
                        crate::db::set_session_activity(&tx, "antigravity", &sid, "done", ts)
                            .map_err(|e| e.to_string())?;
                    }
                    save_stamp(&tx, &key, stamps[0].0, stamps[0].1).map_err(|e| e.to_string())?;
                    save_stamp(&tx, &wal_key, stamps[1].0, stamps[1].1).map_err(|e| e.to_string())?;
                    save_stamp(&tx, &tr_key, tr_stamp.0, tr_stamp.1).map_err(|e| e.to_string())?;
                    tx.commit().map_err(|e| e.to_string())?;
                    Ok(written)
                })
                .map_err(|e| e.to_string())?;
            changed += n;
        }
    }

    // 2. 兜底扫描仅保留 brain/<conv_id>/.../transcript.jsonl 而没有 .db 的会话
    let brain_dir = root.path.join("brain");
    if brain_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&brain_dir) {
            for entry in entries.flatten() {
                if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let conv_id = entry.file_name().to_string_lossy().into_owned();
                if handled_convs.contains(&conv_id) {
                    continue;
                }
                let tr_path = transcript_path(&root.path, &conv_id);
                if !tr_path.is_file() {
                    continue;
                }
                let tr_stamp = file_stamp(&tr_path);
                let key = tr_path.to_string_lossy().to_string();
                if db.write(|conn| crate::db::get_scan_state(conn, &key) == Some(tr_stamp)) {
                    continue;
                }
                let steps = read_transcript(&tr_path);
                let parsed = read_transcript_fallback(&steps, &root.source_id, &conv_id);
                let n = db
                    .write(|conn| -> Result<usize, String> {
                        let tx = conn.transaction().map_err(|e| e.to_string())?;
                        let records: Vec<Record> = parsed.iter().map(|p| p.record.clone()).collect();
                        let written = source_store::write(&tx, &records).map_err(|e| e.to_string())?;
                        if let Some(ts) = parsed.iter().map(|p| p.record.request.ts).max() {
                            let sid = format!("{}:{conv_id}", root.source_id);
                            crate::db::set_session_activity(&tx, "antigravity", &sid, "done", ts)
                                .map_err(|e| e.to_string())?;
                        }
                        save_stamp(&tx, &key, tr_stamp.0, tr_stamp.1).map_err(|e| e.to_string())?;
                        tx.commit().map_err(|e| e.to_string())?;
                        Ok(written)
                    })
                    .map_err(|e| e.to_string())?;
                changed += n;
            }
        }
    }

    // 3. 同步会话标题与项目工作区路径
    if let Err(err) = sync_summaries(db, root) {
        errors.push(format!("conversation_summaries.db：{err}"));
    }

    Ok((changed, errors))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
    }

    fn push_varint_field(field: u32, val: u64, out: &mut Vec<u8>) {
        encode_varint(((field as u64) << 3) | 0, out);
        encode_varint(val, out);
    }

    fn push_bytes_field(field: u32, data: &[u8], out: &mut Vec<u8>) {
        encode_varint(((field as u64) << 3) | 2, out);
        encode_varint(data.len() as u64, out);
        out.extend_from_slice(data);
    }

    fn encode_timestamp(secs: u64, nanos: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        push_varint_field(1, secs, &mut buf);
        if nanos > 0 {
            push_varint_field(2, nanos, &mut buf);
        }
        buf
    }

    fn encode_step_metadata(
        created: (u64, u64),
        first_token: (u64, u64),
        completed: (u64, u64),
        model_enum: u64,
        input: u64,
        output: u64,
        cache_read: u64,
        response_id: &str,
    ) -> Vec<u8> {
        let mut usage = Vec::new();
        push_varint_field(1, model_enum, &mut usage);
        if input > 0 {
            push_varint_field(2, input, &mut usage);
        }
        if output > 0 {
            push_varint_field(3, output, &mut usage);
        }
        if cache_read > 0 {
            push_varint_field(5, cache_read, &mut usage);
        }
        push_bytes_field(7, response_id.as_bytes(), &mut usage);

        let mut meta = Vec::new();
        push_bytes_field(1, &encode_timestamp(created.0, created.1), &mut meta);
        push_bytes_field(6, &encode_timestamp(first_token.0, first_token.1), &mut meta);
        push_bytes_field(7, &encode_timestamp(completed.0, completed.1), &mut meta);
        push_bytes_field(9, &usage, &mut meta);
        meta
    }

    #[test]
    fn parses_protobuf_metadata_joins_transcript_and_syncs_summaries() {
        let dir = std::env::temp_dir().join(format!(
            "jolu-antigravity-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let root_dir = dir.join(".cc-switch").join("agy").join("prof-12345678").join("antigravity-cli");
        let conv_dir = root_dir.join("conversations");
        let log_dir = root_dir.join("brain").join("conv-1").join(".system_generated").join("logs");
        std::fs::create_dir_all(&conv_dir).unwrap();
        std::fs::create_dir_all(&log_dir).unwrap();

        // 构造模拟 conversations/conv-1.db（含首轮冷启动 cache_read=0 与二轮缓存命中 181_000）
        let conv_db = Connection::open(conv_dir.join("conv-1.db")).unwrap();
        conv_db
            .execute_batch("CREATE TABLE steps (idx INTEGER PRIMARY KEY, step_type INTEGER, status INTEGER, metadata BLOB);")
            .unwrap();
        let meta0 = encode_step_metadata(
            (1_795_620_000, 0),
            (1_795_620_005, 500_000_000),
            (1_795_620_010, 0),
            1468,
            20_000,
            500,
            0,
            "resp-cold",
        );
        let meta1 = encode_step_metadata(
            (1_795_625_546, 0),
            (1_795_625_556, 400_000_000),
            (1_795_625_562, 900_000_000),
            1468,
            2_883,
            795,
            181_000,
            "resp-warm",
        );
        conv_db
            .execute("INSERT INTO steps VALUES (0, 15, 3, ?1), (1, 15, 3, ?2), (2, 10, 3, ?2)", params![meta0, meta1])
            .unwrap();
        drop(conv_db);

        // 构造配套 transcript.jsonl（只给 step 1 写模型名，验证 step 0 通过同 enum 1468 继承）
        std::fs::write(
            log_dir.join("transcript.jsonl"),
            "{\"type\":\"PLANNER_RESPONSE\",\"step_index\":1,\"model\":\"gemini-4-neon-xhigh\",\"status\":\"DONE\"}\n",
        )
        .unwrap();

        // 构造 conversation_summaries.db
        let sum_db = Connection::open(root_dir.join("conversation_summaries.db")).unwrap();
        sum_db
            .execute_batch(
                "CREATE TABLE conversation_summaries (conversation_id TEXT PRIMARY KEY, title TEXT, workspace_uris TEXT, last_modified_time TEXT);",
            )
            .unwrap();
        sum_db
            .execute(
                "INSERT INTO conversation_summaries VALUES ('conv-1', '测试会话标题', '[\"file:///D:/develop/Jolu\"]', '2026-11-26T00:52:26+08:00')",
                [],
            )
            .unwrap();
        drop(sum_db);

        let roots = discover_roots(&dir);
        let ccs = roots.iter().find(|r| r.source_id == "native:antigravity:ccs-prof-12345678").expect("应自动发现 CCS 托管目录");

        let mut dest = crate::db::open(Path::new(":memory:")).unwrap();
        let (n, errs) = scan_root(&mut dest, ccs).unwrap();
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(n, 2, "两条 PLANNER_RESPONSE 应全部入库，非 step_type=15 被忽略");

        // 二次扫描幂等（戳记命中直接跳过）
        let (n2, _) = scan_root(&mut dest, ccs).unwrap();
        assert_eq!(n2, 0);

        let filter = crate::usage_stats::StatsFilter {
            start: 1_795_600_000_000,
            end: 1_795_700_000_000,
            platform: Some("antigravity".into()),
            ..Default::default()
        };
        let ov = crate::usage_stats::overview(&dest, &filter).unwrap();
        assert_eq!(ov.requests, 2);
        assert_eq!(ov.tokens.fresh_input, Some(22_883));
        assert_eq!(ov.tokens.output, Some(1_295));
        assert_eq!(ov.tokens.cache_read, Some(181_000));
        assert_eq!(ov.tokens.total, Some(205_178));
        let expected_hit = 181_000.0 / (22_883.0 + 181_000.0);
        assert!((ov.cache_hit_rate.unwrap() - expected_hit).abs() < 1e-6);

        let page = crate::usage_stats::calls(&dest, &filter, 1).unwrap();
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].call_id, "resp-warm");
        assert_eq!(page.rows[0].model.as_deref(), Some("gemini-4-neon-xhigh"));
        assert_eq!(page.rows[0].first_token_ms, Some(10_400));
        assert_eq!(page.rows[0].duration_ms, Some(16_900));
        assert_eq!(page.rows[1].model.as_deref(), Some("gemini-4-neon-xhigh"), "同 enum 应关联模型名");
        assert_eq!(page.rows[1].first_token_ms, Some(5_500));

        // 核对 conversation_summaries.db 同步的标题与 cwd
        let (title, cwd): (Option<String>, Option<String>) = dest
            .query_row(
                "SELECT title, cwd FROM sessions WHERE platform = 'antigravity'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("测试会话标题"));
        assert!(cwd.as_deref().is_some_and(|p| p.ends_with("Jolu")));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn transcript_fallback_works_when_db_absent() {
        let dir = std::env::temp_dir().join(format!(
            "jolu-antigravity-fallback-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let root_dir = dir.join(".gemini").join("antigravity");
        let log_dir = root_dir.join("brain").join("conv-only-tr").join(".system_generated").join("logs");
        std::fs::create_dir_all(&log_dir).unwrap();
        std::fs::write(
            log_dir.join("transcript.jsonl"),
            "{\"type\":\"PLANNER_RESPONSE\",\"step_index\":4,\"model\":\"gemini-4-neon-xhigh\",\"status\":\"DONE\",\"created_at\":\"2026-11-26T00:52:26+08:00\",\"input_tokens\":1000,\"output_tokens\":200,\"cache_read_tokens\":8000}\n",
        )
        .unwrap();

        let roots = discover_roots(&dir);
        let def = roots.iter().find(|r| r.source_id == "native:antigravity:default").unwrap();
        let mut dest = crate::db::open(Path::new(":memory:")).unwrap();
        let (n, errs) = scan_root(&mut dest, def).unwrap();
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(n, 1);

        let total: i64 = dest.query_row("SELECT total_tokens FROM requests WHERE platform = 'antigravity'", [], |r| r.get(0)).unwrap();
        assert_eq!(total, 9200);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 真机只读对账测试（手动 `--ignored --nocapture` 执行，绝不修改真实数据库）：
    /// 1. 在内存库扫描本机 Antigravity 根目录并打印今日指标与调用明细，核对 CCS 口径；
    /// 2. 将真实 `usage.db` 复制到临时目录，验证增量迁移、2026-11-21 趋势非零修复、
    ///    Kimi / Pi / DSH 采集入库及 `session_threads::list` 无重复且隐藏 DSH subagent。
    #[test]
    #[ignore]
    fn real_machine_readonly_reconciliation() {
        use chrono::{Local, TimeZone};
        use std::collections::HashSet;

        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .expect("home_dir");
        let roots = discover_roots(&home);
        println!("[AGY] discovered roots ({}):", roots.len());
        for r in &roots {
            println!("  - {} => {}", r.source_id, r.path.display());
        }
        assert!(!roots.is_empty(), "应发现至少一个 Antigravity 根目录");

        let mut mem = crate::db::open(Path::new(":memory:")).unwrap();
        let mut total_written = 0usize;
        for r in &roots {
            let (n, errs) = scan_root(&mut mem, r).unwrap();
            println!("  scanned {}: written={}, errs={:?}", r.source_id, n, errs);
            total_written += n;
        }
        let today = Local::now().date_naive();
        let start = Local
            .from_local_datetime(&today.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .unwrap()
            .timestamp_millis();
        let end = Local::now().timestamp_millis() + 60_000;
        let agy_filter = crate::usage_stats::StatsFilter {
            start,
            end,
            platform: Some("antigravity".into()),
            ..Default::default()
        };
        let ov = crate::usage_stats::overview(&mem, &agy_filter).unwrap();
        println!(
            "[AGY TODAY] total_written={total_written}, requests={}, tokens={:?}, fresh={:?}, output={:?}, cache_read={:?}, hit_rate={:?}, cost=${:.4}, bucket={}, points={}",
            ov.requests,
            ov.tokens.total,
            ov.tokens.fresh_input,
            ov.tokens.output,
            ov.tokens.cache_read,
            ov.cache_hit_rate,
            ov.cost.amount,
            ov.trend.bucket,
            ov.trend.points.len()
        );
        let page = crate::usage_stats::calls(&mem, &agy_filter, 1).unwrap();
        println!("[AGY CALLS] total_count={}, page_size={}, showing first 5:", page.total_count, page.page_size);
        for row in page.rows.iter().take(5) {
            println!(
                "  ts={} model={:?} fresh={:?} out={:?} cache_read={:?} ttft={:?}ms dur={:?}ms cost={:?}",
                row.ts, row.model, row.fresh_input, row.output, row.cache_read, row.first_token_ms, row.duration_ms, row.cost
            );
        }
        assert!(ov.requests >= 22, "今日 Antigravity 请求应 >= 22，实际 {}", ov.requests);
        assert_eq!(ov.trend.bucket, "1h");
        assert!(ov.cache_hit_rate.is_some_and(|r| r > 0.7), "缓存命中率应 > 70%，实际 {:?}", ov.cache_hit_rate);
        assert!(page.rows.iter().any(|r| r.first_token_ms.is_some_and(|ms| ms > 0)));

        // 2. 复制真实 usage.db 到临时目录，验证迁移、2026-11-21 趋势与 P2 来源采集
        let real_db = std::env::var_os("APPDATA")
            .map(|d| PathBuf::from(d).join("dev.ningz.jolu").join("usage.db"));
        if let Some(src_db) = real_db.filter(|p| p.is_file()) {
            let tmp = std::env::temp_dir().join(format!(
                "jolu-reconcile-{}-{}.db",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
            ));
            std::fs::copy(&src_db, &tmp).unwrap();
            let mut conn = crate::db::open(&tmp).unwrap();

            // 核对 2026-11-21 趋势柱（此前因 1 条未知行被整柱清零）
            let d1121 = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
            let s21 = Local.from_local_datetime(&d1121.and_hms_opt(0, 0, 0).unwrap()).single().unwrap().timestamp_millis();
            let e21 = Local.from_local_datetime(&d1121.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap()).single().unwrap().timestamp_millis();
            let ov21 = crate::usage_stats::overview(&conn, &crate::usage_stats::StatsFilter { start: s21, end: e21, ..Default::default() }).unwrap();
            let trend_sum: i64 = ov21.trend.points.iter().filter_map(|p| p.total).sum();
            let unknown_sum: i64 = ov21.trend.points.iter().map(|p| p.unknown_requests).sum();
            println!(
                "[2026-11-21] requests={}, known_total={:?}, trend_sum={}, unknown_in_buckets={}, hit_rate={:?}",
                ov21.requests, ov21.token_coverage.known_total, trend_sum, unknown_sum, ov21.cache_hit_rate
            );
            if ov21.token_coverage.known_total.unwrap_or(0) > 0 {
                assert!(trend_sum > 0, "2026-11-21 趋势合计不应再被未知行清零");
            }

            let agy_before: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='antigravity'", [], |r| r.get(0)).unwrap();
            let kimi_before: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='kimi'", [], |r| r.get(0)).unwrap();
            let pi_before: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='pi'", [], |r| r.get(0)).unwrap();
            let dsh_before: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='dsh'", [], |r| r.get(0)).unwrap();
            println!("[BEFORE INGEST ON COPY] agy={agy_before}, kimi={kimi_before}, pi={pi_before}, dsh={dsh_before}");

            // 扫描 P1 + P2 来源到临时库副本（注意：scan_usage 接收的是 home 目录，与 native_sources::scan_sources 一致）
            let mut agy_n = 0usize;
            for r in &roots {
                agy_n += scan_root(&mut conn, r).unwrap().0;
            }
            let (kimi_n, kimi_e) = crate::kimi_sessions::scan_usage(&mut conn, &home).unwrap();
            let (pi_n, pi_e) = crate::pi_sessions::scan_usage(&mut conn, &home).unwrap();
            let (dsh_n, dsh_e) = crate::dsh_sessions::scan_usage(&mut conn, &home).unwrap();
            println!(
                "[P1/P2 INGEST ON COPY] agy={agy_n}, kimi={kimi_n} (errs={}), pi={pi_n} (errs={}), dsh={dsh_n} (errs={})",
                kimi_e.len(),
                pi_e.len(),
                dsh_e.len()
            );

            for plat in ["antigravity", "kimi", "pi", "dsh"] {
                let ov_all = crate::usage_stats::overview(
                    &conn,
                    &crate::usage_stats::StatsFilter {
                        start: 0,
                        end,
                        platform: Some(plat.into()),
                        ..Default::default()
                    },
                )
                .unwrap();
                println!(
                    "  platform={plat}: requests={}, known_total={:?}, strict_hit={:?}, cov_hit={:?} ({}/{}), cost=${:.4}",
                    ov_all.requests,
                    ov_all.token_coverage.known_total,
                    ov_all.cache_hit_rate,
                    ov_all.token_coverage.cache_hit_rate,
                    ov_all.token_coverage.cache_requests,
                    ov_all.requests,
                    ov_all.cost.amount
                );
            }

            // 验证 session_threads::list 无重复且隐藏 DSH subagent
            let list = crate::session_threads::list(&conn, &crate::session_threads::SessionFilter::default()).unwrap();
            let mut seen = HashSet::new();
            for item in &list.sessions {
                let key = (item.platform.clone(), item.session_id.clone());
                assert!(seen.insert(key.clone()), "session_threads 发现重复会话: {:?}", key);
            }
            let subagent_ids: HashSet<String> = conn
                .prepare("SELECT session_id FROM sessions WHERE platform = 'dsh' AND is_subagent = 1")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .flatten()
                .collect();
            let leaked = list
                .sessions
                .iter()
                .filter(|s| s.platform == "dsh" && subagent_ids.contains(&s.session_id))
                .count();
            println!(
                "[SESSIONS] total_listed={}, unique={}, dsh_subagents_in_db={}, leaked_subagents={}",
                list.sessions.len(),
                seen.len(),
                subagent_ids.len(),
                leaked
            );
            assert_eq!(leaked, 0, "DSH subagent 会话不得出现在 session_threads 列表");

            drop(conn);
            let _ = std::fs::remove_file(&tmp);
            let _ = std::fs::remove_file(format!("{}-wal", tmp.display()));
            let _ = std::fs::remove_file(format!("{}-shm", tmp.display()));
        }
    }
}
