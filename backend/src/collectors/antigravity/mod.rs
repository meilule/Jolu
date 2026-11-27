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


pub(crate) mod proto;
use proto::{parse_first_workspace_cwd, parse_step_metadata, StepMetadata};
#[cfg(test)]
#[allow(unused_imports)]
use proto::*;

// ---------------- Transcript & Summaries 关联 ----------------

#[derive(Debug, Clone)]
pub(crate) struct TranscriptStep {
    pub(crate) step_index: i64,
    pub(crate) model: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) created_at_ms: Option<i64>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_read_tokens: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TranscriptActivity {
    pub(crate) state: &'static str,
    pub(crate) updated_ms: i64,
    pub(crate) turn_started_ms: Option<i64>,
}

pub(crate) fn iso_to_ms(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with("0001-01-01") {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(trimmed)
        .or_else(|_| {
            let normalized = trimmed.replacen(' ', "T", 1);
            chrono::DateTime::parse_from_rfc3339(&normalized)
        })
        .ok()
        .map(|d| d.timestamp_millis())
}

pub(crate) fn transcript_path(root: &Path, conv_id: &str) -> PathBuf {
    root.join("brain")
        .join(conv_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl")
}

pub(crate) fn transcript_full_path(root: &Path, conv_id: &str) -> PathBuf {
    root.join("brain")
        .join(conv_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript_full.jsonl")
}

pub(crate) fn safe_conv_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 160
        && !id.contains("..")
        && !id.contains('/')
        && !id.contains('\\')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub(crate) fn existing_transcript(root: &Path, conv_id: &str) -> Option<PathBuf> {
    if !safe_conv_id(conv_id) {
        return None;
    }
    let compact = transcript_path(root, conv_id);
    if compact.is_file() {
        return Some(compact);
    }
    let full = transcript_full_path(root, conv_id);
    if full.is_file() {
        return Some(full);
    }
    None
}

/// 在已发现的 Antigravity 根目录列表中查找指定 `session_id` 对应的对话记录文件
/// （优先 `transcript.jsonl`，缺失时回退 `transcript_full.jsonl`）。
pub fn find_transcript_in(roots: &[Root], session_id: &str) -> Option<PathBuf> {
    for root in roots {
        if let Some(rest) = session_id.strip_prefix(&root.source_id) {
            if let Some(conv_id) = rest.strip_prefix(':') {
                if let Some(found) = existing_transcript(&root.path, conv_id) {
                    return Some(found);
                }
            }
        }
    }
    // 兜底：若传入裸会话 ID 或 source_id 前缀变化，按末段 conversation_id 在全部根目录中查找
    let bare = session_id.rsplit(':').next().unwrap_or(session_id);
    for root in roots {
        if let Some(found) = existing_transcript(&root.path, bare) {
            return Some(found);
        }
    }
    None
}

/// 在当前用户的本机 Antigravity 根目录中查找会话对话记录文件。
pub fn find_transcript(session_id: &str) -> Option<PathBuf> {
    if crate::validation::isolated() {
        return None;
    }
    let home = crate::creds::home()?;
    let roots = discover_roots(&home);
    find_transcript_in(&roots, session_id)
}

/// 从 Antigravity `USER_INPUT` 的 `content` 字段中提取用户实际提问文本：
/// 真实日志通常包裹在 `<USER_REQUEST>...</USER_REQUEST>` 标签中，并附带 `<ADDITIONAL_METADATA>` 或系统指令。
pub(crate) fn extract_user_request_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    const OPEN: &str = "<USER_REQUEST>";
    const CLOSE: &str = "</USER_REQUEST>";
    if let Some(start) = trimmed.find(OPEN) {
        let after = &trimmed[start + OPEN.len()..];
        let body = match after.find(CLOSE) {
            Some(end) => &after[..end],
            None => after,
        };
        let cleaned = body.trim();
        return (!cleaned.is_empty()).then(|| cleaned.to_string());
    }
    // 无 <USER_REQUEST> 标签时：跳过纯系统/上下文注入块，保留普通用户输入
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("<system-instruction")
        || lower.starts_with("<context_summary")
        || lower.starts_with("<system_message")
        || lower.starts_with("<additional_metadata")
    {
        return None;
    }
    Some(trimmed.to_string())
}

/// 从对话记录中提取首条有效用户提问的首行作为会话标题（最多 80 字符）。
pub(crate) fn read_transcript_title(path: &Path) -> Option<String> {
    if crate::validation::isolated() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(200).map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.contains("USER_INPUT") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("USER_INPUT") {
            continue;
        }
        let Some(raw) = v.get("content").and_then(Value::as_str) else {
            continue;
        };
        let Some(text) = extract_user_request_text(raw) else {
            continue;
        };
        if let Some(first_line) = text.lines().map(str::trim).find(|l| !l.is_empty()) {
            let title: String = first_line.chars().take(80).collect();
            if !title.is_empty() {
                return Some(title);
            }
        }
    }
    None
}

pub(crate) fn read_transcript_with_activity(path: &Path) -> (Vec<TranscriptStep>, Option<TranscriptActivity>) {
    let Ok(file) = std::fs::File::open(path) else {
        return (Vec::new(), None);
    };
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    let mut state: Option<&'static str> = None;
    let mut updated_ms: Option<i64> = None;
    let mut turn_started_ms: Option<i64> = None;
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let step_type = v.get("type").and_then(Value::as_str).unwrap_or("");
        let status = v.get("status").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        let status_upper = status.unwrap_or("").to_ascii_uppercase();
        let created_ms = v.get("created_at").and_then(Value::as_str).and_then(iso_to_ms);
        let completed_ms = v.get("completed_at").and_then(Value::as_str).and_then(iso_to_ms);
        let event_ts = completed_ms.or(created_ms);
        if let Some(ts) = event_ts {
            match step_type {
                "USER_INPUT" => {
                    state = Some("running");
                    turn_started_ms = Some(ts);
                    updated_ms = Some(updated_ms.map_or(ts, |old| old.max(ts)));
                }
                "PLANNER_RESPONSE" => {
                    let has_tools = v
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .is_some_and(|a| !a.is_empty());
                    state = Some(if matches!(status_upper.as_str(), "ERROR" | "FAILED") {
                        "failed"
                    } else if matches!(status_upper.as_str(), "CANCELED" | "CANCELLED") {
                        "done"
                    } else if has_tools || matches!(status_upper.as_str(), "RUNNING" | "PENDING" | "GENERATING") {
                        if state != Some("running") {
                            turn_started_ms = Some(turn_started_ms.unwrap_or(ts));
                        }
                        "running"
                    } else {
                        "done"
                    });
                    updated_ms = Some(updated_ms.map_or(ts, |old| old.max(ts)));
                }
                _ => {
                    if matches!(status_upper.as_str(), "RUNNING" | "PENDING" | "GENERATING") {
                        if state != Some("running") {
                            turn_started_ms = Some(turn_started_ms.unwrap_or(ts));
                        }
                        state = Some("running");
                        updated_ms = Some(updated_ms.map_or(ts, |old| old.max(ts)));
                    } else if state == Some("running") {
                        updated_ms = Some(updated_ms.map_or(ts, |old| old.max(ts)));
                    }
                }
            }
        }
        if step_type != "PLANNER_RESPONSE" {
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
            status: status.map(String::from),
            created_at_ms: created_ms,
            input_tokens: num("input_tokens"),
            output_tokens: num("output_tokens"),
            cache_read_tokens: num("cache_read_tokens"),
        });
    }
    let act = state.zip(updated_ms).map(|(state, updated_ms)| TranscriptActivity {
        state,
        updated_ms,
        turn_started_ms,
    });
    (out, act)
}

pub(crate) fn map_outcome(status: Option<&str>) -> Option<&'static str> {
    match status?.to_ascii_uppercase().as_str() {
        "DONE" | "COMPLETED" | "SUCCESS" => Some("success"),
        "ERROR" | "FAILED" => Some("failed"),
        "CANCELED" | "CANCELLED" => Some("cancelled"),
        "RUNNING" | "PENDING" | "GENERATING" => Some("unknown"),
        _ => None,
    }
}

/// 单会话读取结果（记录 + 可选原生结果状态）。
pub(crate) struct ParsedStep {
    pub(crate) record: Record,
    pub(crate) outcome: Option<&'static str>,
}

pub(crate) fn read_conversation_db(
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

pub(crate) fn read_transcript_fallback(
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

pub(crate) fn map_summary_activity(status: Option<&str>) -> &'static str {
    match status.unwrap_or("").trim().to_ascii_uppercase().as_str() {
        "CASCADE_RUN_STATUS_RUNNING" | "RUNNING" | "GENERATING" => "running",
        "CASCADE_RUN_STATUS_ERROR" | "ERROR" | "FAILED" => "failed",
        _ => "done",
    }
}

/// 同步 `conversation_summaries.db` 的会话标题、项目工作区路径、运行状态与子代理标记（按主库 + WAL 戳记增量跳过）。
pub(crate) fn sync_summaries(db: &mut impl Store, root: &Root) -> Result<(), String> {
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
    // 兼容旧版/测试最小表结构：探测可选列 preview / status / parent_conversation_id
    let cols: HashSet<String> = input
        .prepare("PRAGMA table_info(conversation_summaries)")
        .and_then(|mut s| {
            let names = s.query_map([], |r| r.get::<_, String>(1))?;
            names.collect()
        })
        .unwrap_or_default();
    let preview_expr = if cols.contains("preview") { "preview" } else { "NULL" };
    let status_expr = if cols.contains("status") { "status" } else { "NULL" };
    let parent_expr = if cols.contains("parent_conversation_id") {
        "parent_conversation_id"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT conversation_id, title, {preview_expr}, workspace_uris, last_modified_time, {status_expr}, {parent_expr} FROM conversation_summaries"
    );
    let mut stmt = input.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<(String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1).ok().flatten(),
                r.get::<_, Option<String>>(2).ok().flatten(),
                r.get::<_, Option<String>>(3).ok().flatten(),
                r.get::<_, Option<String>>(4).ok().flatten(),
                r.get::<_, Option<String>>(5).ok().flatten(),
                r.get::<_, Option<String>>(6).ok().flatten(),
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    drop(input);

    db.write(|conn| -> Result<(), String> {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (conv_id, title, preview, uris, last_mod, status, parent_id) in rows {
            let sid = format!("{}:{conv_id}", root.source_id);
            let effective_title = title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(String::from)
                .or_else(|| {
                    preview
                        .as_deref()
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .map(|p| p.lines().next().unwrap_or(p).chars().take(80).collect())
                })
                .or_else(|| {
                    existing_transcript(&root.path, &conv_id)
                        .and_then(|p| read_transcript_title(&p))
                });
            if let Some(title) = effective_title {
                crate::db::upsert_session_title(&tx, "antigravity", &sid, &title)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(cwd) = parse_first_workspace_cwd(uris.as_deref()) {
                crate::db::set_session_cwd(&tx, "antigravity", &sid, &cwd)
                    .map_err(|e| e.to_string())?;
            }
            if parent_id.as_deref().is_some_and(|p| !p.trim().is_empty()) {
                crate::db::set_session_subagent(&tx, "antigravity", &sid, true)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(ts) = last_mod.as_deref().and_then(iso_to_ms) {
                let state = map_summary_activity(status.as_deref());
                crate::db::set_session_activity(&tx, "antigravity", &sid, state, ts)
                    .map_err(|e| e.to_string())?;
            }
        }
        save_stamp(&tx, &key, stamps[0].0, stamps[0].1).map_err(|e| e.to_string())?;
        save_stamp(&tx, &wal_key, stamps[1].0, stamps[1].1).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// 为当前根目录下已入库但标题仍为空的 Antigravity 会话回填首个用户提问作为标题。
pub(crate) fn backfill_missing_titles(db: &mut impl Store, root: &Root) -> Result<(), String> {
    let prefix = format!("{}:", root.source_id);
    let pattern = format!("{prefix}%");
    let missing: Vec<String> = db.write(|conn| {
        let Ok(mut stmt) = conn.prepare(
            "SELECT DISTINCT r.session_id FROM requests r
             LEFT JOIN sessions s ON s.platform = r.platform AND s.session_id = r.session_id
             WHERE r.platform = 'antigravity' AND r.session_id LIKE ?1
               AND (s.title IS NULL OR TRIM(s.title) = '')",
        ) else {
            return Vec::new();
        };
        stmt.query_map([&pattern], |r| r.get::<_, String>(0))
            .ok()
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    });
    if missing.is_empty() {
        return Ok(());
    }
    let mut updates = Vec::new();
    for sid in missing {
        let Some(conv_id) = sid.strip_prefix(&prefix) else {
            continue;
        };
        if let Some(title) = existing_transcript(&root.path, conv_id).and_then(|p| read_transcript_title(&p)) {
            updates.push((sid, title));
        }
    }
    if updates.is_empty() {
        return Ok(());
    }
    db.write(|conn| -> Result<(), String> {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (sid, title) in updates {
            crate::db::upsert_session_title(&tx, "antigravity", &sid, &title)
                .map_err(|e| e.to_string())?;
        }
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

            let (tr_steps, tr_act) = if tr_path.is_file() {
                read_transcript_with_activity(&tr_path)
            } else {
                (Vec::new(), None)
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
                    let mut latest_end_ts: Option<i64> = None;
                    for p in &parsed {
                        let end_ts = p.record.request.ts.saturating_add(p.record.duration_ms.unwrap_or(0));
                        latest_end_ts = Some(latest_end_ts.map_or(end_ts, |prev| prev.max(end_ts)));
                        if let Some(outcome) = p.outcome {
                            outcome_stmt
                                .execute(params![outcome, p.record.request.source, p.record.request.dedup_key])
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    drop(outcome_stmt);
                    let sid = format!("{}:{conv_id}", root.source_id);
                    if let Some(act) = tr_act {
                        let ts = latest_end_ts.map_or(act.updated_ms, |end| end.max(act.updated_ms));
                        crate::db::set_session_activity(&tx, "antigravity", &sid, act.state, ts)
                            .map_err(|e| e.to_string())?;
                        if let Some(start_ms) = act.turn_started_ms {
                            crate::db::set_session_turn_start(&tx, "antigravity", &sid, start_ms)
                                .map_err(|e| e.to_string())?;
                        }
                    } else if let Some(ts) = latest_end_ts {
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
                let (steps, tr_act) = read_transcript_with_activity(&tr_path);
                let parsed = read_transcript_fallback(&steps, &root.source_id, &conv_id);
                let n = db
                    .write(|conn| -> Result<usize, String> {
                        let tx = conn.transaction().map_err(|e| e.to_string())?;
                        let records: Vec<Record> = parsed.iter().map(|p| p.record.clone()).collect();
                        let written = source_store::write(&tx, &records).map_err(|e| e.to_string())?;
                        let sid = format!("{}:{conv_id}", root.source_id);
                        if let Some(act) = tr_act {
                            crate::db::set_session_activity(&tx, "antigravity", &sid, act.state, act.updated_ms)
                                .map_err(|e| e.to_string())?;
                            if let Some(start_ms) = act.turn_started_ms {
                                crate::db::set_session_turn_start(&tx, "antigravity", &sid, start_ms)
                                    .map_err(|e| e.to_string())?;
                            }
                        } else if let Some(ts) = parsed.iter().map(|p| p.record.request.ts).max() {
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

    // 3. 同步会话标题与项目工作区路径，并为标题缺失的会话从 transcript.jsonl 回填标题
    if let Err(err) = sync_summaries(db, root) {
        errors.push(format!("conversation_summaries.db：{err}"));
    }
    if let Err(err) = backfill_missing_titles(db, root) {
        errors.push(format!("antigravity 标题回填：{err}"));
    }

    Ok((changed, errors))
}

#[cfg(test)]
mod tests;
