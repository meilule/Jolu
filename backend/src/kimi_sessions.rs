//! Kimi Code 的本地会话读取（只读）。
//!
//! 为什么单独一个模块：Kimi 的会话**不在 `requests` 表里**——`native_sources` 从来没有
//! 采集过它（那套采集器只覆盖 ZCode / Qoder / WorkBuddy / Gemini / OpenCode）。
//! 所以 `session_threads::list` 的主查询看不到它，只能直接从 Kimi 自己的目录读。
//!
//! ## 目录结构与实测格式（2026-11-22 本机核实）
//!
//! ```text
//! ~/.kimi-code/
//!   session_index.jsonl          每行 { sessionId, sessionDir, workDir }，是索引
//!   sessions/<wd_xxx>/<sessionId>/
//!     state.json                 { createdAt, updatedAt, title, workDir, agents{main{homedir}} }
//!     agents/main/wire.jsonl     事件流，每个事件一行
//! ```
//!
//! `wire.jsonl` 里我们只用两类事件：
//!
//! ```json
//! {"type":"llm.request","modelAlias":"moonshot-cn/kimi-k3","time":1785308428008}
//! {"type":"usage.record","model":"moonshot-cn/kimi-k3","time":1785308451368,
//!  "usage":{"inputOther":1994,"output":197,"inputCacheRead":19200,"inputCreation":0}}
//! ```
//!
//! 一次请求对应一条 `llm.request` + 一条 `usage.record`（同一次调用里 usage 晚于 request）。
//! 只有 `usage.record` 带用量；`inputOther` 不含缓存读取，可以直接当「新输入」。
//!
//! ## 不做什么
//!
//! - **不解析对话正文**：`wire.jsonl` 的 `context.append_message` 里有内容，但会话线程页
//!   只要求列出会话（模块 C「活跃会话」与模块 D 列表），正文读取是另一个量级的工作。
//!   这里 `has_source` 给 false，正文页会如实显示「不可读」，不假装能打开。
//! - **不猜缺失值**：没有 `usage.record` 的请求不计 Token（None 而不是 0）。
//! - **不读子代理的 wire**：只读 `agents/main/`，子代理有自己的 wire 文件但不算用户会话。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::db::{RequestRecord, Store};
use crate::session_threads::SessionItem;
use crate::source_store::{self, Record};

/// 索引里的一行。
#[derive(Debug, Deserialize)]
struct IndexEntry {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "sessionDir")]
    session_dir: String,
}

/// `state.json`（只取我们用的字段）。
#[derive(Debug, Default, Deserialize)]
struct State {
    /// 两种格式都有（RFC3339 字符串 / 整数毫秒），见 `ts_of`
    #[serde(default, rename = "updatedAt")]
    updated_at: Option<serde_json::Value>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "workDir")]
    work_dir: Option<String>,
    /// 创建时间：`updatedAt` 缺失时的回退
    #[serde(default, rename = "createdAt")]
    created_at: Option<serde_json::Value>,
}

/// `wire.jsonl` 里的一个事件（只关心 `llm.request` 与 `usage.record`，其余忽略）。
#[derive(Debug, Deserialize)]
struct WireEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    time: Option<i64>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default, rename = "modelAlias")]
    model_alias: Option<String>,
    #[serde(default, rename = "thinkingEffort")]
    thinking_effort: Option<String>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct WireUsage {
    #[serde(default, rename = "inputOther")]
    input_other: Option<i64>,
    #[serde(default)]
    output: Option<i64>,
    #[serde(default, rename = "inputCacheRead")]
    cache_read: Option<i64>,
    /// 实测 Kimi Code 2.x 写入的键名是 `inputCacheCreation`，兼容旧键名 `inputCreation`。
    #[serde(default, rename = "inputCacheCreation", alias = "inputCreation")]
    cache_write: Option<i64>,
}

/// Kimi 配置根目录：`KIMI_CODE_HOME` 覆盖，否则 `~/.kimi-code`。
///
/// 与 `wiring::kimi_dir` 保持同一套判定（新版目录优先）。这里不调它是因为那个函数还
/// 处理旧版 `~/.kimi` 的回退，而会话索引只有新版目录才有。
pub(crate) fn kimi_home(home: &Path) -> PathBuf {
    std::env::var_os("KIMI_CODE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".kimi-code"))
}

/// 读 `session_index.jsonl`，返回 (sessionId, sessionDir)。
fn index(home: &Path) -> Vec<IndexEntry> {
    let path = kimi_home(home).join("session_index.jsonl");
    let Ok(text) = crate::steady::read_to_string(&path) else { return Vec::new() };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<IndexEntry>(line).ok())
        .collect()
}

/// `state.json` 的时间字段 → 毫秒。
///
/// **实测两种格式都有**（2026-11-22 本机 14 个会话里 1 个是数字）：
/// - 字符串 RFC3339：`"2026-11-20T01:00:00.000Z"`
/// - 整数毫秒：`1784281243255`
///
/// 两种都要认，否则那条会话会被当成「时间未知」，在「最近 24 小时」筛选里被错误排除。
fn ts_of(value: Option<&serde_json::Value>) -> Option<i64> {
    let value = value?;
    if let Some(ms) = value.as_i64() {
        return (ms > 0).then_some(ms);
    }
    let text = value.as_str()?.trim();
    chrono::DateTime::parse_from_rfc3339(text).ok().map(|dt| dt.timestamp_millis())
}

/// 一个 Kimi 会话的聚合结果。
#[derive(Debug, Default)]
struct Totals {
    requests: i64,
    fresh_input: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
    output: Option<i64>,
    first_ts: Option<i64>,
    last_ts: Option<i64>,
}

/// 扫一个会话的 `wire.jsonl`。
///
/// 请求数按 `llm.request` 计（那是真正发出去的一次模型调用）；用量按 `usage.record` 累加。
/// 两者条数不等时以 `llm.request` 为准——没有用量的请求是真的调用过、只是没记 token。
fn scan_wire(path: &Path) -> Totals {
    let mut out = Totals::default();
    let Ok(text) = crate::steady::read_to_string(path) else { return out };
    // 累加用「已知才加」：某一项缺失就不参与合计，最后由调用方按 None 表达未知
    let mut sum = (0i64, 0i64, 0i64, 0i64);
    let mut known = (false, false, false, false);
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<WireEvent>(line) else { continue };
        match event.kind.as_str() {
            "llm.request" => {
                out.requests += 1;
                if let Some(ts) = event.time {
                    out.first_ts = Some(out.first_ts.map_or(ts, |v| v.min(ts)));
                    out.last_ts = Some(out.last_ts.map_or(ts, |v| v.max(ts)));
                }
            }
            "usage.record" => {
                let Some(usage) = event.usage else { continue };
                let parts = [usage.input_other, usage.output, usage.cache_read, usage.cache_write];
                for (i, value) in parts.into_iter().enumerate() {
                    if let Some(v) = value {
                        sum.0 += if i == 0 { v } else { 0 };
                        sum.1 += if i == 1 { v } else { 0 };
                        sum.2 += if i == 2 { v } else { 0 };
                        sum.3 += if i == 3 { v } else { 0 };
                        match i {
                            0 => known.0 = true,
                            1 => known.1 = true,
                            2 => known.2 = true,
                            _ => known.3 = true,
                        }
                    }
                }
                if let Some(ts) = event.time {
                    out.last_ts = Some(out.last_ts.map_or(ts, |v| v.max(ts)));
                }
            }
            _ => {}
        }
    }
    out.fresh_input = known.0.then_some(sum.0);
    out.output = known.1.then_some(sum.1);
    out.cache_read = known.2.then_some(sum.2);
    out.cache_write = known.3.then_some(sum.3);
    out
}

/// 列出 Kimi 的会话（只读）。
///
/// `since` 用来裁掉太久没动的：会话索引里可能有几十上百条历史。
pub(crate) fn sessions(home: &Path, since: Option<i64>) -> Vec<SessionItem> {
    // 隔离验收不读用户会话文件
    if crate::validation::isolated() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen = HashMap::new();
    for entry in index(home) {
        let dir = PathBuf::from(&entry.session_dir);
        // 索引里 sessionDir 是绝对路径；相对路径的条目不可信，跳过
        if !dir.is_absolute() {
            continue;
        }
        let state_path = dir.join("state.json");
        // 先看 mtime 再读：`since` 只要最近活跃的，用最后修改时间先筛掉历史会话。
        // 留 1 小时余量，理由同 `dsh_sessions`（窗口前创建、窗口内仍在追加的会话不能漏）。
        if let Some(since) = since {
            let fresh = std::fs::metadata(&state_path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .is_some_and(|mtime| mtime >= since - 60 * 60 * 1000);
            if !fresh {
                continue;
            }
        }
        let state_text = crate::steady::read_to_string(&state_path).unwrap_or_default();
        let state: State = serde_json::from_str(&state_text).unwrap_or_default();
        let last_ts = ts_of(state.updated_at.as_ref()).or_else(|| ts_of(state.created_at.as_ref()));
        if let (Some(since), Some(ts)) = (since, last_ts) {
            if ts < since {
                continue;
            }
        }
        let wire = dir.join("agents").join("main").join("wire.jsonl");
        let totals = scan_wire(&wire);
        // 索引与实际目录可能不同步（用户删过 sessions 目录）：没有 wire 也没有 state 的跳过，
        // 不在活跃会话里塞一条什么都读不到的记录
        if totals.requests == 0 && state_text.is_empty() {
            continue;
        }
        // 会话 ID 带 `kimi:` 前缀，与 `opencode:` 同一套约定：
        // 平台 + 原始 ID，避免与其它平台的同名 ID 撞车
        let session_id = format!("kimi:{}", entry.session_id);
        if seen.insert(session_id.clone(), ()).is_some() {
            continue;
        }
        let first = totals.first_ts.or(last_ts).unwrap_or(0);
        let last = totals.last_ts.or(last_ts).unwrap_or(first);
        out.push(SessionItem {
            platform: "kimi".into(),
            session_id,
            title: state.title.filter(|t| !t.trim().is_empty()),
            cwd: state.work_dir.filter(|d| !d.trim().is_empty()),
            first_ts: first,
            last_ts: last,
            requests: totals.requests,
            fresh_input: totals.fresh_input,
            cache_read: totals.cache_read,
            cache_write: totals.cache_write,
            output: totals.output,
            connection_id: None,
            status: String::new(),
            // 正文在 wire.jsonl 的 context 事件里，本模块不解析它：给 false 让正文页
            // 如实显示「不可读」，而不是给个能点但打开是空的入口
            has_source: false,
        });
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

/// 枚举某个 Kimi 会话目录下的全部 `agents/<agent>/wire.jsonl`（包含主代理与子代理，确保用量不漏）。
fn collect_wire_files(session_dir: &Path) -> Vec<(String, PathBuf)> {
    let agents_dir = session_dir.join("agents");
    let Ok(entries) = std::fs::read_dir(&agents_dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let wire = dir.join("wire.jsonl");
        if wire.is_file() {
            let agent = entry.file_name().to_string_lossy().into_owned();
            out.push((agent, wire));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 枚举 Kimi 全部会话目录（合并 `session_index.jsonl` 与 `sessions/<wd>/<sid>`，按规范路径去重）。
fn discover_session_dirs(home: &Path) -> Vec<(String, PathBuf)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |sid: String, dir: PathBuf| {
        if !dir.is_dir() {
            return;
        }
        let canon = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if seen.insert(canon) {
            out.push((sid, dir));
        }
    };
    for entry in index(home) {
        let dir = PathBuf::from(&entry.session_dir);
        if dir.is_absolute() {
            push(entry.session_id, dir);
        }
    }
    let root = kimi_home(home).join("sessions");
    if let Ok(wds) = std::fs::read_dir(&root) {
        for wd in wds.flatten() {
            let wd_path = wd.path();
            if !wd_path.is_dir() {
                continue;
            }
            if let Ok(sids) = std::fs::read_dir(&wd_path) {
                for s in sids.flatten() {
                    let s_path = s.path();
                    if s_path.is_dir() {
                        let sid = s.file_name().to_string_lossy().into_owned();
                        push(sid, s_path);
                    }
                }
            }
        }
    }
    out
}

fn parse_wire_records(wire: &Path, session_id: &str, agent: &str) -> Vec<Record> {
    let Ok(text) = crate::steady::read_to_string(wire) else { return Vec::new() };
    let prefixed_session = format!("kimi:{session_id}");
    let mut last_req_ts: Option<i64> = None;
    let mut last_model: Option<String> = None;
    let mut last_effort: Option<String> = None;
    let mut out = Vec::new();
    for (line_idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<WireEvent>(trimmed) else { continue };
        match event.kind.as_str() {
            "llm.request" => {
                last_req_ts = event.time.filter(|t| *t > 0);
                if let Some(m) = event.model.or(event.model_alias).filter(|s| !s.trim().is_empty()) {
                    last_model = Some(m);
                }
                if let Some(e) = event.thinking_effort.filter(|s| !s.trim().is_empty()) {
                    last_effort = Some(e);
                }
            }
            "usage.record" => {
                let Some(usage) = event.usage else { continue };
                let input = usage.input_other.filter(|v| *v >= 0);
                let output = usage.output.filter(|v| *v >= 0);
                let cache_read = usage.cache_read.filter(|v| *v >= 0);
                let cache_write = usage.cache_write.filter(|v| *v >= 0);
                if [input, output, cache_read, cache_write].iter().all(Option::is_none) {
                    continue;
                }
                let Some(ts) = event.time.filter(|t| *t > 0).or(last_req_ts) else {
                    continue;
                };
                let duration_ms = event
                    .time
                    .zip(last_req_ts.take())
                    .and_then(|(end, start)| end.checked_sub(start))
                    .filter(|d| (10..=3_600_000).contains(d));
                let model = event
                    .model
                    .or(event.model_alias)
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| last_model.clone());
                let effort = event
                    .thinking_effort
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| last_effort.clone());
                let total = input
                    .zip(output)
                    .map(|(i, o)| i + o + cache_read.unwrap_or(0) + cache_write.unwrap_or(0));
                out.push(Record {
                    request: RequestRecord {
                        platform: "kimi".into(),
                        source: "native:kimi:code".into(),
                        dedup_key: format!("{session_id}:{agent}:{line_idx}:{ts}"),
                        session_id: Some(prefixed_session.clone()),
                        ts,
                        model,
                        input_tokens: input,
                        output_tokens: output,
                        cache_read_tokens: cache_read,
                        cache_write_tokens: cache_write,
                        total_tokens: total,
                        effort,
                    },
                    input_semantics: "excludes_cache",
                    credits: None,
                    original_credits: None,
                    billable: None,
                    context_ratio: None,
                    reasoning: None,
                    tool: None,
                    duration_ms,
                    first_token_ms: None,
                });
            }
            _ => {}
        }
    }
    out
}

/// 增量将 Kimi Code 用量与会话元数据写入 `requests` / `sessions`（P2）。
pub(crate) fn scan_usage(db: &mut impl Store, home: &Path) -> Result<(usize, Vec<String>), String> {
    let mut changed = 0usize;
    let errors = Vec::new();
    for (sid, dir) in discover_session_dirs(home) {
        let wires = collect_wire_files(&dir);
        if wires.is_empty() {
            continue;
        }
        let state_path = dir.join("state.json");
        let mut state_loaded: Option<State> = None;
        for (agent, wire) in wires {
            let stamp = file_stamp(&wire);
            let key = format!("kimi-usage:{}", wire.to_string_lossy());
            if db.write(|conn| crate::db::get_scan_state(conn, &key) == Some(stamp)) {
                continue;
            }
            let records = parse_wire_records(&wire, &sid, &agent);
            if state_loaded.is_none() {
                let text = crate::steady::read_to_string(&state_path).unwrap_or_default();
                state_loaded = Some(serde_json::from_str::<State>(&text).unwrap_or_default());
            }
            let state = state_loaded.as_ref();
            let prefixed_session = format!("kimi:{sid}");
            let n = db
                .write(|conn| -> Result<usize, String> {
                    let tx = conn.transaction().map_err(|e| e.to_string())?;
                    let written = source_store::write(&tx, &records).map_err(|e| e.to_string())?;
                    if let Some(st) = state {
                        if let Some(title) = st.title.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                            crate::db::upsert_session_title(&tx, "kimi", &prefixed_session, title)
                                .map_err(|e| e.to_string())?;
                        }
                        if let Some(cwd) = st.work_dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                            crate::db::set_session_cwd(&tx, "kimi", &prefixed_session, cwd)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    if let Some(last_ts) = records.iter().map(|r| r.request.ts).max() {
                        crate::db::set_session_activity(&tx, "kimi", &prefixed_session, "done", last_ts)
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
        let home = root.join(".kimi-code");
        let dir = home.join("sessions").join("wd_x").join("session_test-1");
        std::fs::create_dir_all(dir.join("agents").join("main")).unwrap();
        std::fs::write(
            home.join("session_index.jsonl"),
            format!(
                "{{\"sessionId\":\"session_test-1\",\"sessionDir\":\"{}\",\"workDir\":\"D:/proj\"}}\n",
                dir.display().to_string().replace('\\', "/")
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("state.json"),
            r#"{"createdAt":"2026-11-20T00:00:00.000Z","updatedAt":"2026-11-20T01:00:00.000Z","title":"Kimi 会话","workDir":"D:/proj"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents").join("main").join("wire.jsonl"),
            [
                r#"{"type":"metadata","protocol_version":"1.4"}"#,
                r#"{"type":"llm.request","modelAlias":"moonshot-cn/kimi-k3","time":1000}"#,
                r#"{"type":"usage.record","usage":{"inputOther":100,"output":20,"inputCacheRead":50,"inputCreation":0},"time":1100}"#,
                r#"{"type":"llm.request","modelAlias":"moonshot-cn/kimi-k3","time":2000}"#,
                r#"{"type":"usage.record","usage":{"inputOther":200,"output":30,"inputCacheRead":60,"inputCreation":5},"time":2100}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        home
    }

    #[test]
    fn reads_a_session_with_usage_and_skips_unknown_fields() {
        let root = std::env::temp_dir().join(format!("jolu-kimi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let home = fixture(&root);
        let list = sessions(&root, None);
        assert_eq!(list.len(), 1, "索引里有一条会话（fixture home: {}）", home.display());
        let item = &list[0];
        assert_eq!(item.platform, "kimi");
        assert_eq!(item.session_id, "kimi:session_test-1", "会话 ID 要带平台前缀");
        assert_eq!(item.title.as_deref(), Some("Kimi 会话"));
        assert_eq!(item.cwd.as_deref(), Some("D:/proj"));
        assert_eq!(item.requests, 2, "按 llm.request 计请求数");
        assert_eq!(item.fresh_input, Some(300), "inputOther 累加");
        assert_eq!(item.output, Some(50));
        assert_eq!(item.cache_read, Some(110));
        assert_eq!(item.cache_write, Some(5));
        assert_eq!((item.first_ts, item.last_ts), (1000, 2100));
        assert!(!item.has_source, "正文不解析，就不能说能打开");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_session_without_usage_reports_unknown_tokens_not_zero() {
        let root = std::env::temp_dir().join(format!("jolu-kimi-nou-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let home = fixture(&root);
        // 换成一个只有请求、没有 usage 的 wire
        let dir = home.join("sessions").join("wd_x").join("session_test-1");
        std::fs::write(
            dir.join("agents").join("main").join("wire.jsonl"),
            r#"{"type":"llm.request","time":1000}"#,
        )
        .unwrap();
        let list = sessions(&root, None);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].requests, 1);
        assert_eq!(
            (list[0].fresh_input, list[0].output),
            (None, None),
            "没有 usage.record 时 Token 是未知，不是 0"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn since_filters_old_sessions_and_missing_dirs_are_skipped() {
        let root = std::env::temp_dir().join(format!("jolu-kimi-since-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // 实测本机 14 个会话里 1 个的 updatedAt 是整数毫秒、其余是 RFC3339 字符串；
        // 两种都要认，否则那条会话会被当成「时间未知」而被筛选错误排除
        assert_eq!(
            ts_of(Some(&serde_json::json!("2026-11-20T01:00:00.000Z"))),
            Some(chrono::DateTime::parse_from_rfc3339("2026-11-20T01:00:00Z").unwrap().timestamp_millis())
        );
        assert_eq!(ts_of(Some(&serde_json::json!(1784281243255i64))), Some(1784281243255));
        assert_eq!(ts_of(Some(&serde_json::json!(0))), None, "0 不是有效时间戳");
        assert_eq!(ts_of(None), None);

        let home = fixture(&root);
        // updatedAt 是 2026-11-20；要 2026-11-21 之后的就是空的
        let cutoff = chrono::DateTime::parse_from_rfc3339("2026-11-21T00:00:00Z").unwrap().timestamp_millis();
        assert!(sessions(&root, Some(cutoff)).is_empty(), "太久没动的会话不进活跃列表");
        // 索引里指向不存在的目录：跳过，不产生空会话
        std::fs::write(
            home.join("session_index.jsonl"),
            "{\"sessionId\":\"gone\",\"sessionDir\":\"/definitely/not/here\",\"workDir\":\"\"}\n",
        )
        .unwrap();
        assert!(sessions(&root, None).is_empty(), "读不到的会话不列出来");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scan_usage_reads_input_cache_creation_duration_and_subagents() {
        let root = std::env::temp_dir().join(format!("jolu-kimi-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = fixture(&root);
        let dir = home.join("sessions").join("wd_x").join("session_test-1");
        // 主代理用真实字段名 inputCacheCreation + thinkingEffort
        std::fs::write(
            dir.join("agents").join("main").join("wire.jsonl"),
            [
                r#"{"type":"llm.request","modelAlias":"kimi-k2.5","thinkingEffort":"high","time":10000}"#,
                r#"{"type":"usage.record","usage":{"inputOther":1500,"output":200,"inputCacheRead":8000,"inputCacheCreation":400},"time":12500}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        // 子代理的 wire.jsonl 也要计入用量
        std::fs::create_dir_all(dir.join("agents").join("sub-1")).unwrap();
        std::fs::write(
            dir.join("agents").join("sub-1").join("wire.jsonl"),
            [
                r#"{"type":"llm.request","modelAlias":"kimi-k2.5","time":20000}"#,
                r#"{"type":"usage.record","usage":{"inputOther":500,"output":100,"inputCacheRead":2000,"inputCacheCreation":0},"time":21200}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let mut db = crate::db::open(Path::new(":memory:")).unwrap();
        let (n, errs) = scan_usage(&mut db, &root).unwrap();
        assert!(errs.is_empty());
        assert_eq!(n, 2, "主代理与子代理用量都应写入 requests");
        assert_eq!(scan_usage(&mut db, &root).unwrap().0, 0, "二次扫描按文件戳记幂等跳过");

        let ov = crate::usage_stats::overview(
            &db,
            &crate::usage_stats::StatsFilter { start: 0, end: 50_000, platform: Some("kimi".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(ov.requests, 2);
        assert_eq!(ov.tokens.fresh_input, Some(2_000));
        assert_eq!(ov.tokens.cache_read, Some(10_000));
        assert_eq!(ov.tokens.cache_write, Some(400), "inputCacheCreation 必须被正确解析");
        assert_eq!(ov.tokens.output, Some(300));
        assert_eq!(ov.tokens.total, Some(12_700));
        let page = crate::usage_stats::calls(
            &db,
            &crate::usage_stats::StatsFilter { start: 0, end: 50_000, platform: Some("kimi".into()), ..Default::default() },
            1,
        )
        .unwrap();
        assert_eq!(page.rows[1].duration_ms, Some(2_500));
        assert_eq!(page.rows[1].effort.as_deref(), Some("high"));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
