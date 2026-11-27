//! Qwen Code（`@qwen-code/qwen-code`）本地会话与用量只读采集。
//!
//! 同时覆盖 Qwen Code 两代官方目录布局：
//! 1. Gemini CLI 同源快照布局：`~/.qwen/tmp/<hash>/chats/session-*.{json,jsonl}`
//!    （支持 `messages` 数组、`$set`、`$rewindTo` 回退去重及 `tokens.{input,output,cached,thoughts,tool,total}`）；
//! 2. 新版项目会话流布局：`~/.qwen/projects/<project>/chats/*.jsonl`
//!    （支持 `tokens`、`usageMetadata` 与 OpenAI/Anthropic 风格 `message.usage`，并提取 `cwd` 与首条用户提问标题）。
//!
//! 纪律（AGENTS.md §4）：只读原文件，不写回 `~/.qwen`；缺失字段保持 `None`（未知不补零）。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde_json::Value;

use crate::collectors::native::{self, parse};
use crate::db::{RequestRecord, Store};
use crate::source_store::{self, Record};

pub(crate) const SOURCE_ID: &str = "native:qwen:cli";

pub(crate) fn qwen_home(home: &Path) -> PathBuf {
    crate::gateway::wiring_qwen::qwen_dir(home)
}

pub(crate) fn has_data(home: &Path) -> bool {
    let root = qwen_home(home);
    root.join("tmp").is_dir() || root.join("projects").is_dir()
}

fn extract_user_title(v: &Value) -> Option<String> {
    let raw = match v.get("content").or_else(|| v.pointer("/message/content"))? {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let clean: String = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if clean.is_empty() {
        return None;
    }
    let mut clipped: String = clean.chars().take(60).collect();
    if clean.chars().count() > 60 {
        clipped.push('…');
    }
    Some(clipped)
}

fn make_base(session: &str, msg_id: &str, ts: i64) -> Record {
    Record {
        request: RequestRecord {
            platform: "qwen".into(),
            source: SOURCE_ID.into(),
            dedup_key: format!("{session}:{msg_id}"),
            session_id: Some(format!("{SOURCE_ID}:{session}")),
            ts,
            model: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            total_tokens: None,
            effort: None,
        },
        input_semantics: "unknown",
        credits: None,
        original_credits: None,
        billable: None,
        context_ratio: None,
        reasoning: None,
        tool: None,
        duration_ms: None,
        first_token_ms: None,
    }
}

fn message_has_pending_tool(v: &Value) -> bool {
    if v.get("toolCalls").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
        return true;
    }
    if v.pointer("/message/stop_reason")
        .or_else(|| v.get("stop_reason"))
        .and_then(Value::as_str)
        == Some("tool_use")
    {
        return true;
    }
    v.pointer("/message/content")
        .or_else(|| v.get("content"))
        .and_then(Value::as_array)
        .is_some_and(|parts| {
            parts.iter().any(|p| {
                matches!(
                    p.get("type").and_then(Value::as_str),
                    Some("tool_use" | "functionCall" | "toolCall")
                )
            })
        })
}

fn classify_qwen_line(v: &Value) -> Option<(&'static str, i64, bool)> {
    let ts = v.get("timestamp").and_then(parse::timestamp)?;
    if v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
        return Some(("failed", ts, false));
    }
    let role = parse::text(v, "type")
        .or_else(|| v.pointer("/message/role").and_then(Value::as_str))
        .unwrap_or("");
    let is_tool_res = v.get("toolUseResult").is_some()
        || v.pointer("/message/content")
            .or_else(|| v.get("content"))
            .and_then(Value::as_array)
            .is_some_and(|parts| parts.iter().any(|p| p.get("type").and_then(Value::as_str) == Some("tool_result")));
    match role {
        "user" => Some(("running", ts, !is_tool_res)),
        "tool" => Some(("running", ts, false)),
        "qwen" | "gemini" | "assistant" | "model" => {
            if message_has_pending_tool(v) {
                Some(("running", ts, false))
            } else {
                Some(("done", ts, false))
            }
        }
        _ => None,
    }
}
/// 解析单条助手消息（兼容 `tokens`、`usageMetadata` 与 `message.usage` 三种字段形态）。
fn parse_assistant_record(v: &Value, fallback_session: &str) -> Option<Record> {
    let role_or_type = parse::text(v, "type")
        .or_else(|| v.pointer("/message/role").and_then(Value::as_str))
        .unwrap_or("");
    if !matches!(role_or_type, "qwen" | "gemini" | "assistant" | "model") {
        return None;
    }
    if v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let session = parse::text(v, "sessionId").unwrap_or(fallback_session);
    let msg_id = parse::text(v, "id")
        .or_else(|| parse::text(v, "uuid"))
        .or_else(|| v.pointer("/message/id").and_then(Value::as_str).filter(|s| !s.is_empty()))?;
    let ts = v.get("timestamp").and_then(parse::timestamp)?;
    let mut rec = make_base(session, msg_id, ts);
    rec.request.model = parse::text(v, "model")
        .or_else(|| v.pointer("/message/model").and_then(Value::as_str).filter(|s| !s.is_empty()))
        .map(str::to_string);

    if let Some(u) = v.get("tokens").filter(|u| u.is_object()) {
        rec.input_semantics = "includes_cache";
        rec.request.input_tokens = parse::num(u, "input");
        rec.request.output_tokens = parse::num(u, "output");
        rec.request.cache_read_tokens = parse::num(u, "cached");
        rec.request.total_tokens = parse::num(u, "total");
        rec.reasoning = parse::num(u, "thoughts");
        rec.tool = parse::num(u, "tool");
        return Some(rec);
    }

    if let Some(meta) = v.get("usageMetadata").filter(|m| m.is_object()) {
        rec.input_semantics = "includes_cache";
        rec.request.input_tokens = parse::num(meta, "promptTokenCount");
        rec.request.output_tokens = parse::num(meta, "candidatesTokenCount");
        rec.request.cache_read_tokens = parse::num(meta, "cachedContentTokenCount");
        rec.request.total_tokens = parse::num(meta, "totalTokenCount");
        rec.reasoning = parse::num(meta, "thoughtsTokenCount");
        rec.tool = parse::num(meta, "toolUsePromptTokenCount");
        return Some(rec);
    }

    let u = v.pointer("/message/usage").or_else(|| v.get("usage"))?;
    if !u.is_object() {
        return None;
    }
    if u.get("prompt_tokens").is_some() {
        rec.input_semantics = "includes_cache";
        rec.request.input_tokens = parse::num(u, "prompt_tokens");
        rec.request.output_tokens = parse::num(u, "completion_tokens");
        rec.request.cache_read_tokens = u
            .get("prompt_tokens_details")
            .and_then(|d| parse::num(d, "cached_tokens"))
            .or_else(|| parse::num(u, "cached_tokens"));
        rec.request.total_tokens = parse::num(u, "total_tokens");
        rec.reasoning = u
            .get("completion_tokens_details")
            .and_then(|d| parse::num(d, "reasoning_tokens"));
    } else {
        rec.input_semantics = "excludes_cache";
        rec.request.input_tokens = parse::num(u, "input_tokens");
        rec.request.output_tokens = parse::num(u, "output_tokens");
        rec.request.cache_read_tokens = parse::num(u, "cache_read_input_tokens");
        rec.request.cache_write_tokens = parse::num(u, "cache_creation_input_tokens");
        rec.request.total_tokens = parse::num(u, "total_tokens").or_else(|| {
            rec.request
                .input_tokens
                .zip(rec.request.output_tokens)
                .map(|(i, o)| i + o + rec.request.cache_read_tokens.unwrap_or(0) + rec.request.cache_write_tokens.unwrap_or(0))
        });
    }
    if rec.request.input_tokens.is_none() && rec.request.output_tokens.is_none() && rec.request.total_tokens.is_none() {
        return None;
    }
    Some(rec)
}

/// 解析快照式 `session-*.{json,jsonl}`（支持 `$set` 与 `$rewindTo` 回滚剔除）。
pub(crate) fn parse_snapshot_lines(
    lines: &[Value],
    fallback_session: &str,
) -> (String, Option<String>, Vec<Record>, Option<(&'static str, i64, Option<i64>)>) {
    let mut session = String::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();
    let reindex = |msgs: &[Value]| {
        let mut pos = HashMap::new();
        for (idx, m) in msgs.iter().enumerate() {
            if let Some(id) = parse::text(m, "id") {
                pos.entry(id.to_string()).or_insert(idx);
            }
        }
        pos
    };
    for line in lines {
        if let Some(id) = parse::text(line, "sessionId") {
            session = id.into();
        }
        if let Some(all) = line.get("messages").and_then(Value::as_array) {
            messages = all.clone();
            positions = reindex(&messages);
            continue;
        }
        if let Some(set) = line.get("$set") {
            if let Some(id) = parse::text(set, "sessionId") {
                session = id.into();
            }
            if let Some(all) = set.get("messages").and_then(Value::as_array) {
                messages = all.clone();
                positions = reindex(&messages);
            }
            continue;
        }
        if let Some(target) = parse::text(line, "$rewindTo") {
            let idx = positions.get(target).copied().unwrap_or(0);
            messages.truncate(idx);
            positions.retain(|_, p| *p < idx);
            continue;
        }
        if let Some(id) = parse::text(line, "id") {
            if let Some(&idx) = positions.get(id) {
                messages[idx] = line.clone();
            } else {
                positions.insert(id.to_string(), messages.len());
                messages.push(line.clone());
            }
        }
    }
    if session.is_empty() {
        session = fallback_session.to_string();
    }
    let mut out = Vec::new();
    let mut first_title = None;
    let mut prev_user_ts: Option<i64> = None;
    let mut state: Option<&'static str> = None;
    let mut state_ts: Option<i64> = None;
    let mut turn_start: Option<i64> = None;
    for m in &messages {
        if let Some((st, ts, starts_turn)) = classify_qwen_line(m) {
            if starts_turn && state != Some("running") {
                turn_start = Some(ts);
            } else if st == "running" && turn_start.is_none() {
                turn_start = Some(ts);
            }
            state = Some(st);
            state_ts = Some(state_ts.map_or(ts, |old| old.max(ts)));
        }
        let ty = parse::text(m, "type").or_else(|| m.pointer("/message/role").and_then(Value::as_str));
        if ty == Some("user") {
            prev_user_ts = m.get("timestamp").and_then(parse::timestamp);
            if first_title.is_none() {
                first_title = extract_user_title(m);
            }
            continue;
        }
        if let Some(mut rec) = parse_assistant_record(m, &session) {
            if let Some(start) = prev_user_ts.take() {
                let dur = rec.request.ts.saturating_sub(start);
                if (100..=3_600_000).contains(&dur) {
                    rec.duration_ms = Some(dur);
                }
            }
            out.push(rec);
        }
    }
    let activity = state.zip(state_ts).map(|(st, ts)| (st, ts, turn_start));
    (session, first_title, out, activity)
}

fn scan_snapshot_dir<S: Store>(db: &mut S, tmp_dir: &Path) -> Result<(usize, Vec<String>), String> {
    if !tmp_dir.is_dir() {
        return Ok((0, Vec::new()));
    }
    let mut paths = Vec::new();
    native::files(tmp_dir, &mut paths)?;
    paths.sort();
    let mut total_changed = 0;
    let mut errors = Vec::new();
    for path in paths {
        let is_session = path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.starts_with("session-"));
        if !is_session {
            continue;
        }
        if path.extension().is_some_and(|s| s == "json") && path.with_extension("jsonl").exists() {
            continue;
        }
        let step = (|| -> Result<usize, String> {
            let (size, time) = native::stamp(&path)?;
            let key = path.to_string_lossy().to_string();
            let saved = db.write(|conn| crate::db::get_scan_checkpoint(conn, &key));
            if saved.as_ref().is_some_and(|s| (s.offset, s.mtime) == (size, time)) {
                return Ok(0);
            }
            let (lines, consumed) = native::json_lines_from(&path, 0)?;
            let fallback = path.file_stem().and_then(|s| s.to_str()).unwrap_or("qwen-session");
            let (raw_sid, title, records, activity) = parse_snapshot_lines(&lines, fallback);
            let full_sid = format!("{SOURCE_ID}:{raw_sid}");
            db.write(|conn| -> Result<usize, String> {
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                let mut changed = source_store::write(&tx, &records).map_err(|e| e.to_string())?;
                if let Some(t) = title.as_deref() {
                    crate::db::upsert_session_title(&tx, "qwen", &full_sid, t).map_err(|e| e.to_string())?;
                }
                if let Some((st, ts, turn_start)) = activity {
                    crate::db::set_session_activity(&tx, "qwen", &full_sid, st, ts)
                        .map_err(|e| e.to_string())?;
                    if let Some(start_ms) = turn_start {
                        crate::db::set_session_turn_start(&tx, "qwen", &full_sid, start_ms)
                            .map_err(|e| e.to_string())?;
                    }
                } else {
                    for r in &records {
                        crate::db::set_session_activity(&tx, "qwen", &full_sid, "done", r.request.ts)
                            .map_err(|e| e.to_string())?;
                    }
                }
                // 快照 rewind 清理已撤回记录
                let keep_keys: HashSet<&str> = records.iter().map(|r| r.request.dedup_key.as_str()).collect();
                let old_rows: Vec<(i64, String)> = {
                    let mut q = tx
                        .prepare("SELECT id, dedup_key FROM requests WHERE source = ?1 AND session_id = ?2")
                        .map_err(|e| e.to_string())?;
                    let rows = q
                        .query_map(params![SOURCE_ID, full_sid], |r| Ok((r.get(0)?, r.get(1)?)))
                        .map_err(|e| e.to_string())?;
                    rows.collect::<rusqlite::Result<_>>().map_err(|e| e.to_string())?
                };
                for (id, dkey) in old_rows {
                    if !keep_keys.contains(dkey.as_str()) {
                        tx.execute("DELETE FROM request_metrics WHERE request_id = ?1", [id]).map_err(|e| e.to_string())?;
                        tx.execute("DELETE FROM usage_events WHERE request_id = ?1", [id]).map_err(|e| e.to_string())?;
                        changed += tx.execute("DELETE FROM requests WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
                    }
                }
                native::save_stamp(&tx, &key, consumed, time).map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| e.to_string())?;
                Ok(changed)
            })
        })();
        match step {
            Ok(n) => total_changed += n,
            Err(err) => errors.push(format!("Qwen Code（{}）：{err}", path.display())),
        }
    }
    Ok((total_changed, errors))
}

fn scan_projects_dir<S: Store>(db: &mut S, projects_dir: &Path) -> Result<(usize, Vec<String>), String> {
    if !projects_dir.is_dir() {
        return Ok((0, Vec::new()));
    }
    let mut paths = Vec::new();
    native::files(projects_dir, &mut paths)?;
    paths.sort();
    let mut total_changed = 0;
    let mut errors = Vec::new();
    for path in paths {
        if !path.extension().is_some_and(|s| s == "jsonl") {
            continue;
        }
        let step = (|| -> Result<usize, String> {
            let (size, time) = native::stamp(&path)?;
            let key = path.to_string_lossy().to_string();
            let saved = db.write(|conn| crate::db::get_scan_checkpoint(conn, &key));
            let start = native::resume_offset(&path, saved.as_ref(), size, time);
            if start == size && saved.as_ref().is_some_and(|s| s.mtime == time) {
                return Ok(0);
            }
            let (lines, consumed) = native::json_lines_from(&path, start)?;
            let fallback = path.file_stem().and_then(|s| s.to_str()).unwrap_or("qwen-chat");
            let mut latest: HashMap<String, Record> = HashMap::new();
            let mut title: Option<String> = None;
            let mut cwd: Option<String> = None;
            let mut session_id = fallback.to_string();
            let mut state: Option<&'static str> = None;
            let mut state_ts: Option<i64> = None;
            let mut turn_start: Option<i64> = None;
            for line in &lines {
                if let Some(sid) = parse::text(line, "sessionId") {
                    session_id = sid.to_string();
                }
                if cwd.is_none() {
                    cwd = parse::text(line, "cwd").map(str::to_string);
                }
                if let Some((st, ts, starts_turn)) = classify_qwen_line(line) {
                    if starts_turn && state != Some("running") {
                        turn_start = Some(ts);
                    } else if st == "running" && turn_start.is_none() {
                        turn_start = Some(ts);
                    }
                    state = Some(st);
                    state_ts = Some(state_ts.map_or(ts, |old| old.max(ts)));
                }
                let role = parse::text(line, "type").or_else(|| line.pointer("/message/role").and_then(Value::as_str));
                if role == Some("user") && title.is_none() {
                    title = extract_user_title(line);
                }
                if let Some(rec) = parse_assistant_record(line, &session_id) {
                    latest.insert(rec.request.dedup_key.clone(), rec);
                }
            }
            let records: Vec<Record> = latest.into_values().collect();
            let full_sid = format!("{SOURCE_ID}:{session_id}");
            db.write(|conn| -> Result<usize, String> {
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                let changed = source_store::write(&tx, &records).map_err(|e| e.to_string())?;
                if let Some(t) = title.as_deref() {
                    crate::db::upsert_session_title(&tx, "qwen", &full_sid, t).map_err(|e| e.to_string())?;
                }
                if let Some(dir) = cwd.as_deref() {
                    crate::db::set_session_cwd(&tx, "qwen", &full_sid, dir).map_err(|e| e.to_string())?;
                }
                if let Some((st, ts, turn_start)) = state.zip(state_ts).map(|(st, ts)| (st, ts, turn_start)) {
                    crate::db::set_session_activity(&tx, "qwen", &full_sid, st, ts)
                        .map_err(|e| e.to_string())?;
                    if let Some(start_ms) = turn_start {
                        crate::db::set_session_turn_start(&tx, "qwen", &full_sid, start_ms)
                            .map_err(|e| e.to_string())?;
                    }
                } else {
                    for r in &records {
                        crate::db::set_session_activity(&tx, "qwen", &full_sid, "done", r.request.ts)
                            .map_err(|e| e.to_string())?;
                    }
                }
                native::save_stamp(&tx, &key, consumed, time).map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| e.to_string())?;
                Ok(changed)
            })
        })();
        match step {
            Ok(n) => total_changed += n,
            Err(err) => errors.push(format!("Qwen Code（{}）：{err}", path.display())),
        }
    }
    Ok((total_changed, errors))
}

pub(crate) fn scan_usage<S: Store>(db: &mut S, home: &Path) -> Result<(usize, Vec<String>), String> {
    let root = qwen_home(home);
    let (n1, mut e1) = scan_snapshot_dir(db, &root.join("tmp"))?;
    let (n2, e2) = scan_projects_dir(db, &root.join("projects"))?;
    e1.extend(e2);
    Ok((n1 + n2, e1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scans_both_qwen_snapshot_and_project_jsonl_layouts() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home = std::env::temp_dir().join(format!("jolu-qwen-scan-{}-{stamp}", std::process::id()));
        let chats_tmp = home.join(".qwen").join("tmp").join("proj-a").join("chats");
        let chats_proj = home.join(".qwen").join("projects").join("demo").join("chats");
        std::fs::create_dir_all(&chats_tmp).unwrap();
        std::fs::create_dir_all(&chats_proj).unwrap();

        // 1) Snapshot layout (session-*.json)
        let snap = json!({
            "sessionId": "s-snap-1",
            "messages": [
                {"id": "u1", "type": "user", "timestamp": "2026-11-27T06:00:00Z", "content": "帮我重构这个模块"},
                {"id": "a1", "type": "qwen", "model": "qwen3-coder-plus", "timestamp": "2026-11-27T06:00:05Z",
                 "tokens": {"input": 1200, "output": 180, "cached": 400, "thoughts": 30, "total": 1410}}
            ]
        });
        std::fs::write(chats_tmp.join("session-1.json"), serde_json::to_string(&snap).unwrap()).unwrap();

        // 2) Project JSONL layout (*.jsonl)
        let lines = [
            json!({"uuid": "u2", "sessionId": "s-proj-2", "cwd": "D:\\develop\\demo", "type": "user", "timestamp": "2026-11-27T06:10:00Z", "message": {"role": "user", "content": "检查单元测试"}}),
            json!({"uuid": "a2", "sessionId": "s-proj-2", "cwd": "D:\\develop\\demo", "type": "assistant", "timestamp": "2026-11-27T06:10:08Z", "model": "qwen3-coder-plus",
                   "usageMetadata": {"promptTokenCount": 2000, "candidatesTokenCount": 250, "cachedContentTokenCount": 1000, "totalTokenCount": 2250}}),
        ];
        let body = lines.iter().map(|v| serde_json::to_string(v).unwrap()).collect::<Vec<_>>().join("\n") + "\n";
        std::fs::write(chats_proj.join("s-proj-2.jsonl"), body).unwrap();

        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let (inserted, errs) = scan_usage(&mut conn, &home).unwrap();
        assert!(errs.is_empty(), "errors: {errs:?}");
        assert_eq!(inserted, 2);

        // 重复扫描不重复入库
        let (again, _) = scan_usage(&mut conn, &home).unwrap();
        assert_eq!(again, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM requests WHERE platform = 'qwen'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);

        let (title, cwd): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT title, cwd FROM sessions WHERE platform = 'qwen' AND session_id = 'native:qwen:cli:s-proj-2'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("检查单元测试"));
        assert_eq!(cwd.as_deref(), Some("D:\\develop\\demo"));

        let _ = std::fs::remove_dir_all(&home);
    }
}
