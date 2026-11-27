//! Pi 的本地会话读取（只读）。
//!
//! 与 [`crate::kimi_sessions`] 同一个起因：Pi 的会话不在 `requests` 表里
//! （`native_sources` 只覆盖 ZCode / Qoder / WorkBuddy / Gemini / OpenCode），
//! 所以 `session_threads::list` 的主查询看不到它，只能直接读 Pi 自己的目录。
//!
//! ## 目录结构与实测格式（2026-11-22 本机核实）
//!
//! ```text
//! ~/.pi/agent/sessions/<项目目录名>/<时间戳>_<uuid>.jsonl
//! ```
//!
//! 一个文件一个会话，每行一个事件。用得上的两类：
//!
//! ```json
//! {"type":"session","version":3,"id":"01a05200-…","timestamp":"2026-08-30T09:29:15.297Z","cwd":"D:\\develop\\shushu"}
//! {"type":"message","timestamp":"…","message":{"role":"assistant","model":"deepseek-v4-flash",
//!  "usage":{"input":5828,"output":152,"cacheRead":0,"cacheWrite":0,"reasoning":42,"totalTokens":5980}}}
//! ```
//!
//! 会话身份取 `session` 事件的 `id`（不是文件名里的 uuid —— 文件名是
//! `<时间戳>_<uuid>`，两者在本机实测一致，但前者是官方字段，更可靠）。
//!
//! ## 与 Kimi 的差异
//!
//! - Pi 用 `usage.input` **含**缓存读取与否要看 `cacheRead` 是否为 0：本机样本里
//!   `input=5828, cacheRead=0`，说明 `input` 就是新输入。这里按「`input` 含 cacheRead」
//!   处理更安全——若上游把缓存算进 input，直接相加会重复计数。所以新输入 = `input - cacheRead`。
//! - 会话标题：Pi 没有 `title` 字段，用首个 user 消息的前若干字代替（与 Codex 的做法一致）。

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::db::{RequestRecord, Store};
use crate::sessions::types::SessionItem;
use crate::source_store::{self, Record};

/// 一个会话文件的首行 `session` 事件。
#[derive(Debug, Deserialize)]
struct SessionEvent {
    #[serde(default)]
    id: String,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

/// `message` 事件。
#[derive(Debug, Deserialize)]
struct MessageEvent {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    message: Option<InnerMessage>,
    #[serde(default)]
    timestamp: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct InnerMessage {
    #[serde(default)]
    role: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default, rename = "stopReason")]
    stop_reason: Option<String>,
    #[serde(default)]
    content: serde_json::Value,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    #[serde(default)]
    input: Option<i64>,
    #[serde(default)]
    output: Option<i64>,
    #[serde(default, rename = "cacheRead")]
    cache_read: Option<i64>,
    #[serde(default, rename = "cacheWrite")]
    cache_write: Option<i64>,
    #[serde(default)]
    reasoning: Option<i64>,
    #[serde(default, rename = "totalTokens")]
    total_tokens: Option<i64>,
}

/// 判定 Pi 的 `usage.input` 是否包含缓存读取。
/// 实测 Pi (2026-11-26) 样本：`input=6641, output=388, cacheRead=5888, totalTokens=12917`，
/// `6641 + 388 + 5888 = 12917`，明确为 `excludes_cache`（不能再扣一次 `cacheRead`）。
/// 只有当 `totalTokens` 明确等于 `input + output + cacheWrite`（或旧格式缺 `totalTokens` 且 `input >= cacheRead`）时按 `includes_cache` 处理。
fn pi_input_includes_cache(usage: &Usage) -> bool {
    match (usage.input, usage.output, usage.cache_read, usage.total_tokens) {
        (Some(i), Some(o), Some(r), Some(t)) if r > 0 => {
            let excl = i + o + r + usage.cache_write.unwrap_or(0);
            let incl = i + o + usage.cache_write.unwrap_or(0);
            if excl == t {
                false
            } else {
                incl == t
            }
        }
        (Some(i), _, Some(r), None) if r > 0 => i >= r,
        _ => false,
    }
}

/// Pi 配置根目录：`PI_CODING_AGENT_DIR` 覆盖，否则 `~/.pi/agent`。
///
/// 与 `wiring_pi::pi_dir` 用同一套判定（那边还处理 `~` 展开，这里会话目录只可能是绝对路径）。
pub(crate) fn pi_agent_dir(home: &Path) -> PathBuf {
    std::env::var_os("PI_CODING_AGENT_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".pi").join("agent"))
}

fn ts_of(text: Option<&str>) -> Option<i64> {
    let text = text?.trim();
    chrono::DateTime::parse_from_rfc3339(text).ok().map(|dt| dt.timestamp_millis())
}

/// 从 user 消息的 content 里取一段可读文本，用作标题。
///
/// Pi 的 `content` 可能是字符串，也可能是 `[{type:"text",text:"…"}]` 数组。
fn first_text(content: &serde_json::Value) -> Option<String> {
    let text = match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // 标题只留一行、最多 60 字：与 Codex / Claude 的标题量级一致
    let one_line = text.lines().next().unwrap_or("").trim();
    (!one_line.is_empty()).then(|| one_line.chars().take(60).collect())
}

#[derive(Debug, Default)]
struct Totals {
    requests: i64,
    fresh_input: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
    output: Option<i64>,
    first_ts: Option<i64>,
    last_ts: Option<i64>,
    title: Option<String>,
}

/// 扫一个 Pi 会话文件。
fn scan(path: &Path) -> Option<(String, Option<String>, Totals)> {
    let text = crate::steady::read_to_string(path).ok()?;
    let mut session_id = None;
    let mut cwd = None;
    let mut totals = Totals::default();
    let mut sum = (0i64, 0i64, 0i64, 0i64);
    let mut known = (false, false, false, false);
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        match value.get("type").and_then(|t| t.as_str()) {
            Some("session") => {
                let Ok(event) = serde_json::from_value::<SessionEvent>(value) else { continue };
                if !event.id.trim().is_empty() {
                    session_id = Some(event.id);
                }
                cwd = event.cwd.filter(|c| !c.trim().is_empty());
                if let Some(ts) = ts_of(event.timestamp.as_deref()) {
                    totals.first_ts = Some(ts);
                    totals.last_ts = Some(ts);
                }
            }
            Some("message") => {
                let Ok(event) = serde_json::from_value::<MessageEvent>(value) else { continue };
                let Some(message) = event.message else { continue };
                if let Some(ts) = ts_of(event.timestamp.as_deref()) {
                    totals.first_ts = Some(totals.first_ts.map_or(ts, |v| v.min(ts)));
                    totals.last_ts = Some(totals.last_ts.map_or(ts, |v| v.max(ts)));
                }
                if message.role == "user" && totals.title.is_none() {
                    totals.title = first_text(&message.content);
                }
                let Some(usage) = message.usage else { continue };
                // 一次 assistant 消息 = 一次模型调用
                if message.role == "assistant" {
                    totals.requests += 1;
                }
                if let Some(input) = usage.input {
                    let fresh = if pi_input_includes_cache(&usage) {
                        (input - usage.cache_read.unwrap_or(0)).max(0)
                    } else {
                        input
                    };
                    sum.0 += fresh;
                    known.0 = true;
                }
                if let Some(v) = usage.output {
                    sum.1 += v;
                    known.1 = true;
                }
                if let Some(v) = usage.cache_read {
                    sum.2 += v;
                    known.2 = true;
                }
                if let Some(v) = usage.cache_write {
                    sum.3 += v;
                    known.3 = true;
                }
            }
            _ => {}
        }
    }
    let id = session_id?;
    totals.fresh_input = known.0.then_some(sum.0);
    totals.output = known.1.then_some(sum.1);
    totals.cache_read = known.2.then_some(sum.2);
    totals.cache_write = known.3.then_some(sum.3);
    Some((id, cwd, totals))
}

/// 列出 Pi 的会话（只读）。`since` 用来裁掉太久没动的。
pub(crate) fn sessions(home: &Path, since: Option<i64>) -> Vec<SessionItem> {
    if crate::validation::isolated() {
        return Vec::new();
    }
    let root = pi_agent_dir(home).join("sessions");
    let Ok(dirs) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in dirs.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            // 先看 mtime 再解析：一个会话文件可能几 MB，`since` 只要最近活跃的，
            // 用最后修改时间先筛掉历史会话能省掉绝大部分 IO。留 1 小时余量防止漏掉
            // 「窗口前创建、窗口内还在追加」的会话。
            if let Some(since) = since {
                let fresh = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .is_some_and(|mtime| mtime >= since - 60 * 60 * 1000);
                if !fresh {
                    continue;
                }
            }
            let Some((id, cwd, totals)) = scan(&path) else { continue };
            let last_ts = totals.last_ts.unwrap_or(0);
            if let Some(since) = since {
                if last_ts < since {
                    continue;
                }
            }
            out.push(SessionItem {
                platform: "pi".into(),
                // 带平台前缀，与 `kimi:` / `opencode:` 同一套约定
                session_id: format!("pi:{id}"),
                title: totals.title,
                cwd,
                first_ts: totals.first_ts.unwrap_or(last_ts),
                last_ts,
                requests: totals.requests,
                fresh_input: totals.fresh_input,
                cache_read: totals.cache_read,
                cache_write: totals.cache_write,
                output: totals.output,
                connection_id: None,
                status: String::new(),
                // 正文事件在同一个文件里，但本模块只列会话；给 false 让正文页如实显示不可读
                has_source: false,
            });
        }
    }
    out
}

fn file_stamp(path: &Path) -> (i64, i64) {
    let Ok(meta) = std::fs::metadata(path) else { return (0, 0) };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    (meta.len() as i64, mtime)
}

fn save_stamp(conn: &rusqlite::Connection, key: &str, size: i64, time: i64) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT INTO scan_state(path,offset,mtime,parser_initialized) VALUES(?1,?2,?3,1)
         ON CONFLICT(path) DO UPDATE SET offset=?2,mtime=?3,parser_initialized=1",
    )?
    .execute(rusqlite::params![key, size, time])?;
    Ok(())
}

struct ParsedPiFile {
    session_id: String,
    cwd: Option<String>,
    title: Option<String>,
    records: Vec<Record>,
    activity: Option<(&'static str, i64, Option<i64>)>,
}

fn pi_content_has_tool_call(content: &serde_json::Value) -> bool {
    content.as_array().is_some_and(|items| {
        items.iter().any(|item| {
            matches!(
                item.get("type").and_then(|v| v.as_str()),
                Some("toolCall" | "tool_use" | "function_call")
            )
        })
    })
}

fn parse_usage_file(path: &Path) -> Option<ParsedPiFile> {
    let text = crate::steady::read_to_string(path).ok()?;
    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut title: Option<String> = None;
    let mut prev_turn_ts: Option<i64> = None;
    let mut state: Option<&'static str> = None;
    let mut state_ts: Option<i64> = None;
    let mut turn_started_ms: Option<i64> = None;
    let mut records = Vec::new();
    for (line_idx, line) in text.lines().enumerate() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        match value.get("type").and_then(|t| t.as_str()) {
            Some("session") => {
                if let Ok(event) = serde_json::from_value::<SessionEvent>(value) {
                    if !event.id.trim().is_empty() {
                        session_id = Some(event.id);
                    }
                    cwd = event.cwd.filter(|c| !c.trim().is_empty());
                }
            }
            Some("message") => {
                let Ok(event) = serde_json::from_value::<MessageEvent>(value) else { continue };
                let Some(message) = event.message else { continue };
                let ts = ts_of(event.timestamp.as_deref());
                if message.role == "user" {
                    if title.is_none() {
                        title = first_text(&message.content);
                    }
                    prev_turn_ts = ts;
                    if let Some(t) = ts {
                        if state != Some("running") {
                            turn_started_ms = Some(t);
                        }
                        state = Some("running");
                        state_ts = Some(state_ts.map_or(t, |old| old.max(t)));
                    }
                    continue;
                }
                if message.role != "assistant" {
                    if let Some(t) = ts {
                        prev_turn_ts = Some(t);
                        if message.role == "toolResult" || message.role == "tool" {
                            if state != Some("running") {
                                turn_started_ms = Some(turn_started_ms.unwrap_or(t));
                            }
                            state = Some("running");
                            state_ts = Some(state_ts.map_or(t, |old| old.max(t)));
                        }
                    }
                    continue;
                }
                if let Some(t) = ts {
                    let sr = message.stop_reason.as_deref().unwrap_or("");
                    let has_tool = matches!(sr, "toolUse" | "tool_use" | "tool_calls")
                        || pi_content_has_tool_call(&message.content);
                    state = Some(if matches!(sr, "error" | "aborted") {
                        "failed"
                    } else if has_tool {
                        if state != Some("running") {
                            turn_started_ms = Some(turn_started_ms.unwrap_or(t));
                        }
                        "running"
                    } else {
                        "done"
                    });
                    state_ts = Some(state_ts.map_or(t, |old| old.max(t)));
                }
                let Some(usage) = message.usage else { continue };
                let Some(ts) = ts else { continue };
                let Some(sid) = session_id.as_ref() else { continue };
                let input = usage.input.filter(|v| *v >= 0);
                let output = usage.output.filter(|v| *v >= 0);
                let cache_read = usage.cache_read.filter(|v| *v >= 0);
                let cache_write = usage.cache_write.filter(|v| *v >= 0);
                let reasoning = usage.reasoning.filter(|v| *v >= 0);
                if [input, output, cache_read, cache_write, usage.total_tokens].iter().all(Option::is_none) {
                    continue;
                }
                let includes_cache = pi_input_includes_cache(&usage);
                let semantics = if includes_cache { "includes_cache" } else { "excludes_cache" };
                let total = usage.total_tokens.filter(|v| *v >= 0).or_else(|| {
                    input.zip(output).map(|(i, o)| {
                        if includes_cache {
                            i + o + cache_write.unwrap_or(0)
                        } else {
                            i + o + cache_read.unwrap_or(0) + cache_write.unwrap_or(0)
                        }
                    })
                });
                let duration_ms = prev_turn_ts
                    .take()
                    .and_then(|start| ts.checked_sub(start))
                    .filter(|d| (100..=3_600_000).contains(d));
                let dedup_suffix = event
                    .id
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .unwrap_or_else(|| format!("line-{line_idx}-{ts}"));
                records.push(Record {
                    request: RequestRecord {
                        platform: "pi".into(),
                        source: "native:pi:agent".into(),
                        dedup_key: format!("{sid}:{dedup_suffix}"),
                        session_id: Some(format!("pi:{sid}")),
                        ts,
                        model: message.model.filter(|m| !m.trim().is_empty()),
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
                    reasoning,
                    tool: None,
                    duration_ms,
                    first_token_ms: None,
                });
            }
            _ => {}
        }
    }
    Some(ParsedPiFile {
        session_id: session_id?,
        cwd,
        title,
        records,
        activity: state.zip(state_ts).map(|(st, ts)| (st, ts, turn_started_ms)),
    })
}

/// 增量将 Pi 用量与会话元数据写入 `requests` / `sessions`（P2）。
pub(crate) fn scan_usage(db: &mut impl Store, home: &Path) -> Result<(usize, Vec<String>), String> {
    let root = pi_agent_dir(home).join("sessions");
    let Ok(dirs) = std::fs::read_dir(&root) else {
        return Ok((0, Vec::new()));
    };
    let mut changed = 0usize;
    let errors = Vec::new();
    for entry in dirs.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let stamp = file_stamp(&path);
            let key = format!("pi-usage:{}", path.to_string_lossy());
            if db.write(|conn| crate::db::get_scan_state(conn, &key) == Some(stamp)) {
                continue;
            }
            let Some(parsed) = parse_usage_file(&path) else {
                let _ = db.write(|conn| save_stamp(conn, &key, stamp.0, stamp.1));
                continue;
            };
            let prefixed_session = format!("pi:{}", parsed.session_id);
            let n = db
                .write(|conn| -> Result<usize, String> {
                    let tx = conn.transaction().map_err(|e| e.to_string())?;
                    let written = source_store::write(&tx, &parsed.records).map_err(|e| e.to_string())?;
                    if let Some(title) = parsed.title.as_deref() {
                        crate::db::upsert_session_title(&tx, "pi", &prefixed_session, title)
                            .map_err(|e| e.to_string())?;
                    }
                    if let Some(cwd) = parsed.cwd.as_deref() {
                        crate::db::set_session_cwd(&tx, "pi", &prefixed_session, cwd)
                            .map_err(|e| e.to_string())?;
                    }
                    if let Some((act_state, act_ts, turn_start)) = parsed.activity {
                        crate::db::set_session_activity(&tx, "pi", &prefixed_session, act_state, act_ts)
                            .map_err(|e| e.to_string())?;
                        if let Some(started_ms) = turn_start {
                            crate::db::set_session_turn_start(&tx, "pi", &prefixed_session, started_ms)
                                .map_err(|e| e.to_string())?;
                        }
                    } else if let Some(last_ts) = parsed.records.iter().map(|r| r.request.ts).max() {
                        crate::db::set_session_activity(&tx, "pi", &prefixed_session, "done", last_ts)
                            .map_err(|e| e.to_string())?;
                    }
                    save_stamp(&tx, &key, stamp.0, stamp.1).map_err(|e| e.to_string())?;
                    tx.commit().map_err(|e| e.to_string())?;
                    Ok(written)
                })
                .map_err(|e| e.to_string())?;
            changed += n;
        }
    }
    Ok((changed, errors))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> PathBuf {
        let dir = root.join(".pi").join("agent").join("sessions").join("--proj--");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("2026-11-20T00-00-00-000Z_abc.jsonl"),
            [
                r#"{"type":"session","version":3,"id":"sess-pi-1","timestamp":"2026-11-20T00:00:00.000Z","cwd":"D:\\proj"}"#,
                r#"{"type":"message","timestamp":"2026-11-20T00:00:01.000Z","message":{"role":"user","content":"帮我改一下入口"}}"#,
                // input 含 cacheRead：新输入应该是 1000-200=800
                r#"{"type":"message","timestamp":"2026-11-20T00:00:02.000Z","message":{"role":"assistant","model":"m","usage":{"input":1000,"output":50,"cacheRead":200,"cacheWrite":10}}}"#,
                r#"{"type":"message","timestamp":"2026-11-20T00:00:03.000Z","message":{"role":"assistant","model":"m","usage":{"input":500,"output":20,"cacheRead":100,"cacheWrite":0}}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        root.to_path_buf()
    }

    #[test]
    fn reads_a_session_and_subtracts_cache_from_input() {
        let root = std::env::temp_dir().join(format!("jolu-pi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        fixture(&root);
        let list = sessions(&root, None);
        assert_eq!(list.len(), 1);
        let item = &list[0];
        assert_eq!(item.platform, "pi");
        assert_eq!(item.session_id, "pi:sess-pi-1", "会话 ID 带平台前缀");
        assert_eq!(item.cwd.as_deref(), Some("D:\\proj"));
        assert_eq!(item.title.as_deref(), Some("帮我改一下入口"), "标题取首个 user 消息");
        assert_eq!(item.requests, 2, "按 assistant 消息计调用次数");
        // 1000-200 + 500-100 = 1200
        assert_eq!(item.fresh_input, Some(1200), "input 要扣掉 cacheRead 才是不重复的新输入");
        assert_eq!(item.output, Some(70));
        assert_eq!(item.cache_read, Some(300));
        assert_eq!(item.cache_write, Some(10));
        assert!(!item.has_source, "正文不解析就不能说能打开");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn since_filters_and_files_without_a_session_event_are_skipped() {
        let root = std::env::temp_dir().join(format!("jolu-pi-since-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        fixture(&root);
        // 会话时间是 2026-11-20；要 2026-11-21 之后的就是空的
        let cutoff = chrono::DateTime::parse_from_rfc3339("2026-11-21T00:00:00Z").unwrap().timestamp_millis();
        assert!(sessions(&root, Some(cutoff)).is_empty(), "太久没动的不进活跃列表");

        // 没有 session 事件的 jsonl（坏文件 / 别的用途）：跳过，不产生空会话
        let dir = root.join(".pi").join("agent").join("sessions").join("--proj--");
        std::fs::write(
            dir.join("broken.jsonl"),
            r#"{"type":"message","message":{"role":"user","content":"hi"}}"#,
        )
        .unwrap();
        let list = sessions(&root, None);
        assert_eq!(list.len(), 1, "只有一个有 session 事件的会话");
        assert_eq!(list[0].session_id, "pi:sess-pi-1");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scan_usage_detects_real_excludes_cache_semantics_without_double_subtraction() {
        let root = std::env::temp_dir().join(format!("jolu-pi-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join(".pi").join("agent").join("sessions").join("--proj--");
        std::fs::create_dir_all(&dir).unwrap();
        // 实测 Pi 样本：6641 + 388 + 5888 = 12917，证明 input 不含 cacheRead（excludes_cache）
        std::fs::write(
            dir.join("2026-11-26T00-00-00-000Z_real.jsonl"),
            [
                r#"{"type":"session","version":3,"id":"pi-real-1","timestamp":"2026-11-26T00:00:00.000Z","cwd":"D:\\develop\\Jolu"}"#,
                r#"{"type":"message","id":"u1","timestamp":"2026-11-26T00:00:01.000Z","message":{"role":"user","content":"检查用量"}}"#,
                r#"{"type":"message","id":"a1","timestamp":"2026-11-26T00:00:06.000Z","message":{"role":"assistant","model":"deepseek-v4-flash","usage":{"input":6641,"output":388,"cacheRead":5888,"cacheWrite":0,"totalTokens":12917}}}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let mut db = crate::db::open(Path::new(":memory:")).unwrap();
        let (n, errs) = scan_usage(&mut db, &root).unwrap();
        assert!(errs.is_empty());
        assert_eq!(n, 1);
        assert_eq!(scan_usage(&mut db, &root).unwrap().0, 0);

        let start = chrono::DateTime::parse_from_rfc3339("2026-11-25T00:00:00Z").unwrap().timestamp_millis();
        let end = chrono::DateTime::parse_from_rfc3339("2026-11-27T00:00:00Z").unwrap().timestamp_millis();
        let ov = crate::usage_stats::overview(
            &db,
            &crate::usage_stats::StatsFilter { start, end, platform: Some("pi".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(ov.requests, 1);
        assert_eq!(ov.tokens.fresh_input, Some(6641), "excludes_cache 不应误扣 cacheRead");
        assert_eq!(ov.tokens.cache_read, Some(5888));
        assert_eq!(ov.tokens.output, Some(388));
        assert_eq!(ov.tokens.total, Some(12917));
        let expected_rate = 5888.0 / (6641.0 + 5888.0);
        assert!((ov.cache_hit_rate.unwrap() - expected_rate).abs() < 1e-6);

        let page = crate::usage_stats::calls(
            &db,
            &crate::usage_stats::StatsFilter { start, end, platform: Some("pi".into()), ..Default::default() },
            1,
        )
        .unwrap();
        assert_eq!(page.rows[0].duration_ms, Some(5_000));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
