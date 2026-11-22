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

use crate::session_threads::SessionItem;

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
    message: Option<InnerMessage>,
    #[serde(default)]
    timestamp: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct InnerMessage {
    #[serde(default)]
    role: String,
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
                // `input` 可能已含缓存读取：扣掉才是不重复的新输入
                if let Some(input) = usage.input {
                    let cache = usage.cache_read.unwrap_or(0);
                    sum.0 += (input - cache).max(0);
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
}
