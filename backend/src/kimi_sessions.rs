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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::session_threads::SessionItem;

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

/// `wire.jsonl` 里的一个事件（只关心两个 type，其余忽略）。
#[derive(Debug, Deserialize)]
struct WireEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    time: Option<i64>,
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
    #[serde(default, rename = "inputCreation")]
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
}
