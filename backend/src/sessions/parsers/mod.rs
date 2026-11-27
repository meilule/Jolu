use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value as Json;

use super::source::source_files;
use super::types::{Message, MessagePage, ToolCall};

pub(crate) mod antigravity;
pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod zcode;

use antigravity::{parse_antigravity, parse_antigravity_with};
use claude::{parse_claude, parse_claude_with};
use codex::{parse_codex, parse_codex_with};
use zcode::{open_zcode_db, zcode_page};

pub(crate) const TEXT_LIMIT: usize = 20_000;
pub(crate) const TOOL_OUTPUT_LIMIT: usize = 1_200;
pub(crate) const DETAIL_OUTPUT_LIMIT: usize = 40_000;
pub(crate) const DETAIL_SUMMARY_LIMIT: usize = 2_000;

pub(crate) fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push_str("…");
    out
}

/// 保留首尾：交接要看输出末尾（测试结果、报错），中间省略
pub(crate) fn clip_middle(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    let head = limit / 4;
    let tail = limit - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start}\n…［中间省略 {} 字］…\n{end}", count - head - tail)
}

/// 解析的详细程度：会话页只要片段；交接要更长的输出、完整命令和 Codex 的命令事件
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) text: usize,
    pub(crate) output: usize,
    pub(crate) summary: usize,
    pub(crate) detail: bool,
}

pub(crate) const VIEW: Limits = Limits { text: TEXT_LIMIT, output: TOOL_OUTPUT_LIMIT, summary: 120, detail: false };
pub(crate) const DETAIL: Limits = Limits { text: TEXT_LIMIT, output: DETAIL_OUTPUT_LIMIT, summary: DETAIL_SUMMARY_LIMIT, detail: true };

impl Limits {
    pub(crate) fn summary(&self, text: &str) -> String {
        if self.detail { clip(text.trim(), self.summary) } else { clip(text.lines().next().unwrap_or(""), self.summary) }
    }
    pub(crate) fn output(&self, text: &str) -> String {
        if self.detail { clip_middle(text, self.output) } else { clip(text, self.output) }
    }
}

pub(crate) fn parse_ts(v: &Json) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(v.get("timestamp")?.as_str()?).ok().map(|d| d.timestamp_millis())
}

pub(crate) fn parse_input(input: &Json) -> Json {
    match input {
        Json::String(s) => serde_json::from_str::<Json>(s).unwrap_or(Json::String(s.clone())),
        other => other.clone(),
    }
}

/// 工具调用摘要：优先取路径、命令、模式等常见字段
pub(crate) fn tool_summary(input: &Json, limits: Limits) -> String {
    let input = parse_input(input);
    if let Json::String(s) = &input {
        return limits.summary(s);
    }
    for key in [
        "toolSummary",
        "file_path",
        "Path",
        "path",
        "CommandLine",
        "command",
        "cmd",
        "pattern",
        "url",
        "query",
        "TaskId",
        "Action",
        "description",
    ] {
        if let Some(v) = input.get(key) {
            let text = match v {
                Json::String(s) => s.clone(),
                Json::Array(items) => items.iter().filter_map(Json::as_str).collect::<Vec<_>>().join(" "),
                _ => continue,
            };
            return limits.summary(&text);
        }
    }
    String::new()
}

/// 调用涉及的文件：常见路径字段，以及 apply_patch 补丁里的文件头
pub(crate) fn tool_paths(input: &Json) -> Vec<String> {
    let input = parse_input(input);
    let mut out = Vec::new();
    let patch = match &input {
        Json::String(s) => Some(s.as_str()),
        other => other.get("patch").or_else(|| other.get("input")).and_then(Json::as_str),
    };
    if let Some(patch) = patch.filter(|p| p.contains("*** Begin Patch")) {
        for line in patch.lines() {
            for head in ["*** Add File: ", "*** Update File: ", "*** Delete File: ", "*** Move to: "] {
                if let Some(path) = line.strip_prefix(head).map(str::trim).filter(|p| !p.is_empty()) {
                    out.push(path.to_string());
                }
            }
        }
    }
    for key in ["file_path", "notebook_path", "Path", "path"] {
        if let Some(path) = input.get(key).and_then(Json::as_str).map(str::trim).filter(|p| !p.is_empty()) {
            out.push(path.to_string());
        }
    }
    out
}

pub(crate) fn output_text(v: &Json) -> Option<String> {
    let text = match v {
        Json::String(s) => s.clone(),
        Json::Array(items) => items.iter().filter_map(|b| b.get("text").and_then(Json::as_str)).collect::<Vec<_>>().join("\n"),
        Json::Object(_) => v.get("output").and_then(Json::as_str).map(String::from).unwrap_or_else(|| v.to_string()),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// 工具结果：输出片段、失败标记与退出码
#[derive(Debug, Default)]
pub(crate) struct ToolResult {
    pub(crate) output: Option<String>,
    pub(crate) failed: Option<bool>,
    pub(crate) exit_code: Option<i32>,
}

/// 文本里行首 `<前缀> N` 形式的退出码
pub(crate) fn exit_line(text: &str, prefixes: &[&str]) -> Option<i32> {
    text.lines().find_map(|line| {
        let line = line.trim();
        prefixes.iter().find_map(|p| line.strip_prefix(p)).and_then(|rest| {
            let digits: String = rest.trim_start_matches([':', ' ']).chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
            digits.parse().ok()
        })
    })
}

/// Claude `tool_result`：`is_error` 是失败标记，失败时正文以 `Exit code N` 开头
pub(crate) fn claude_result(block: &Json, limits: Limits) -> ToolResult {
    let text = block.get("content").and_then(output_text);
    let exit_code = text.as_deref().and_then(|t| t.lines().next()).and_then(|first| exit_line(first, &["Exit code"]));
    ToolResult {
        output: text.map(|t| limits.output(&t)),
        failed: block.get("is_error").and_then(Json::as_bool),
        exit_code,
    }
}

/// Codex 输出没有错误字段：旧版 shell 的 JSON `metadata.exit_code`、`exec_command` 的 `Process exited with code N`、
/// `exec` 脚本首行 `Script failed|completed`；都没有时为 None
pub(crate) fn codex_result(output: Option<&Json>, limits: Limits) -> ToolResult {
    let Some(text) = output.and_then(output_text) else { return ToolResult::default() };
    let json_exit = serde_json::from_str::<Json>(&text).ok().and_then(|j| j.pointer("/metadata/exit_code").and_then(Json::as_i64)).map(|n| n as i32);
    let exit_code = json_exit.or_else(|| exit_line(&text, &["Process exited with code", "Exit code"]));
    let script = match text.lines().next().map(str::trim) {
        Some(l) if l.starts_with("Script failed") => Some(true),
        Some(l) if l.starts_with("Script completed") => Some(false),
        _ => None,
    };
    ToolResult { failed: exit_code.map(|c| c != 0).or(script), exit_code, output: Some(limits.output(&text)) }
}

/// 注入给模型的系统性内容（环境、指令、命令回显）不是用户写的话，不显示
pub(crate) fn is_injected(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('<') || t.starts_with("# AGENTS.md") || t.is_empty()
}

pub(crate) fn push_assistant(out: &mut Vec<Message>, ts: Option<i64>, text: Option<String>, tool: Option<ToolCall>, limit: usize) {
    let reuse = out.last().is_some_and(|m| m.role == "assistant");
    if !reuse {
        out.push(Message { role: "assistant".into(), ts, text: String::new(), tools: Vec::new(), ..Message::default() });
    }
    let last = out.last_mut().expect("刚放入");
    if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
        if !last.text.is_empty() {
            last.text.push_str("\n\n");
        }
        last.text.push_str(&clip(t.trim(), limit));
    }
    if let Some(tool) = tool {
        last.tools.push(tool);
    }
    if ts.is_some() {
        last.ts = ts;
    }
}

pub(crate) fn attach_output(out: &mut [Message], pending: &mut HashMap<String, (usize, usize)>, id: &str, result: ToolResult) {
    if let Some((m, t)) = pending.remove(id) {
        if let Some(tool) = out.get_mut(m).and_then(|msg| msg.tools.get_mut(t)) {
            tool.output = result.output;
            tool.failed = result.failed;
            tool.exit_code = result.exit_code;
        }
    }
}


pub(crate) fn read_lines(path: &Path) -> Result<Vec<String>, String> {
    if crate::validation::isolated() { return Err("隔离验收不读取用户会话文件".into()); }
    let file = std::fs::File::open(path).map_err(|error| format!("无法打开会话文件：{error}"))?;
    // 任一行读取失败都不能把此前内容当作完整会话，更不能据此生成交接。
    BufReader::new(file).lines().collect::<std::io::Result<Vec<_>>>()
        .map_err(|error| format!("读取会话文件失败：{error}"))
}

/// 逐行喂给解析器，不把整份会话文件读成 `Vec<String>`。
///
/// 本机会话单个文件可达数十 MB（本机最大 53 MB / 97 MB），整份读入再加一层 `Vec<String>`
/// 等于把文件体积翻倍占住内存；会话页每次翻页都会重解析，峰值会跟着文件大小线性增长。
/// 读取失败仍然整体报错：调用方拿到的是 `Err`，半截内容不会被当成完整会话（同 [`read_lines`]）。
pub(crate) struct LineSource {
    pub(crate) reader: BufReader<std::fs::File>,
    pub(crate) buffer: String,
    pub(crate) failed: Option<String>,
}

impl LineSource {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|error| format!("无法打开会话文件：{error}"))?;
        Ok(Self { reader: BufReader::new(file), buffer: String::new(), failed: None })
    }
}

impl Iterator for LineSource {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        if self.failed.is_some() { return None; }
        self.buffer.clear();
        match self.reader.read_line(&mut self.buffer) {
            Ok(0) => None,
            Ok(_) => {
                while self.buffer.ends_with('\n') || self.buffer.ends_with('\r') { self.buffer.pop(); }
                Some(std::mem::take(&mut self.buffer))
            }
            Err(error) => {
                self.failed = Some(format!("读取会话文件失败：{error}"));
                None
            }
        }
    }
}

/// 只读解析一份会话日志：逐行流式解析，读失败时整体返回 `Err`（界面显示不可读，不伪造空会话）。
pub(crate) fn parse_file(path: &Path, platform: &str) -> Result<Vec<Message>, String> {
    if crate::validation::isolated() { return Err("隔离验收不读取用户会话文件".into()); }
    let mut source = LineSource::open(path)?;
    let messages = match platform {
        "claude" => parse_claude(&mut source),
        "codex" => parse_codex(&mut source),
        "antigravity" => parse_antigravity(&mut source),
        _ => Vec::new(),
    };
    match source.failed.take() {
        Some(error) => Err(error),
        None => Ok(messages),
    }
}

/// 会话主记录文件；源文件不存在时为 None
/// 不在平台目录里、但有本地会话文件的来源（`kimi_sessions` / `pi_sessions` / `dsh_sessions`）。
/// 会话列表按平台筛选时也要认它们，否则这三家只能在「全部」里看到（P4 审计 §3.2 第 5 条）。
pub(crate) const EXTERNAL_SESSION_PLATFORMS: &[&str] = &["kimi", "pi", "dsh"];

/// 会话列表接受的平台筛选值：平台目录里的，加上外部会话来源。
pub(crate) fn session_platform_allowed(platform: &str) -> bool {
    crate::platforms::known(platform) || EXTERNAL_SESSION_PLATFORMS.contains(&platform)
}

pub(crate) fn source_path(conn: &Connection, platform: &str, session_id: &str) -> rusqlite::Result<Option<PathBuf>> {
    if let Some(found) = source_files(conn)?.remove(&(platform.to_string(), session_id.to_string())).filter(|p| p.exists()) {
        return Ok(Some(found));
    }
    if platform == "antigravity" {
        return Ok(crate::antigravity_usage::find_transcript(session_id));
    }
    Ok(None)
}

/// 交接用的详细解析：消息切分与去重规则和会话页相同，所以 `#n` 序号一致；
/// 但保留更长的工具输出与完整命令，并读取 Codex 的命令与文件修改事件（附在当前助手消息里，不新增消息）
pub(crate) fn parse_detail(platform: &str, lines: &[String]) -> Vec<Message> {
    match platform {
        "claude" => parse_claude_with(lines.iter().cloned(), DETAIL),
        "codex" => parse_codex_with(lines.iter().cloned(), DETAIL),
        "antigravity" => parse_antigravity_with(lines.iter().cloned(), DETAIL),
        _ => Vec::new(),
    }
}


/// 消息分页：`before` 为 None 取最后 `limit` 条，否则取序号 `before` 之前的 `limit` 条
pub fn messages(conn: &Connection, platform: &str, session_id: &str, before: Option<usize>, limit: usize) -> Result<MessagePage, String> {
    // 库型来源：正文在 ZCode 自己的 sqlite 里，打不开/结构不符时照常返回「仅用量」，不报错
    if platform == "zcode" {
        let zdb = open_zcode_db();
        return zcode_page(conn, zdb.as_ref(), session_id, before, limit);
    }
    let file = source_path(conn, platform, session_id).map_err(|error| error.to_string())?;
    let all = match (&file, platform) {
        (Some(p), "claude") => parse_file(p, "claude")?,
        (Some(p), "codex") => parse_file(p, "codex")?,
        (Some(p), "antigravity") => parse_file(p, "antigravity")?,
        _ => Vec::new(),
    };
    let total = all.len();
    let end = before.unwrap_or(total).min(total);
    let start = end.saturating_sub(limit);
    let (cost, priced_requests, requests, models) = session_cost(conn, platform, session_id).map_err(|error| error.to_string())?;
    Ok(MessagePage {
        available: file.is_some(),
        total,
        offset: start,
        messages: all[start..end].to_vec(),
        models,
        cost,
        priced_requests,
        requests,
        note: None,
    })
}

pub(crate) fn session_cost(conn: &Connection, platform: &str, session_id: &str) -> rusqlite::Result<(f64, i64, i64, Vec<String>)> {
    let snapshot = crate::attribution::Snapshot::load(conn).ok();
    let mut resolver = crate::pricing::PriceResolver::load(conn);
    let mut stmt = conn.prepare(
        "SELECT r.model, r.upstream_model, r.served_model, r.input_tokens, r.output_tokens,
                r.cache_write_tokens, r.cache_read_tokens, r.cache_write_1h_tokens,
                r.input_known, r.output_known, r.cache_write_known, r.cache_read_known,
                r.input_semantics, r.connection_id, r.provider_key, m.reasoning, m.tool, r.ts
         FROM requests r LEFT JOIN request_metrics m ON m.request_id = r.id
         WHERE r.platform = ?1 AND r.session_id = ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![platform, session_id], |r| {
        let opt = |v: usize, k: usize| -> rusqlite::Result<Option<i64>> {
            Ok(if r.get::<_, i64>(k)? != 0 { Some(r.get(v)?) } else { None })
        };
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            opt(3, 8)?,
            opt(4, 9)?,
            opt(5, 10)?,
            opt(6, 11)?,
            r.get::<_, Option<i64>>(7)?.unwrap_or(0),
            r.get::<_, String>(12)?,
            r.get::<_, Option<String>>(13)?,
            r.get::<_, Option<String>>(14)?,
            r.get::<_, Option<i64>>(15)?,
            r.get::<_, Option<i64>>(16)?,
            r.get::<_, i64>(17)?,
        ))
    })?;
    let (mut cost, mut priced, mut count) = (0.0, 0i64, 0i64);
    let mut models: Vec<String> = Vec::new();
    for row in rows {
        let (model, upstream, served, input, output, cache_write, cache_read, cache_write_1h, semantics, conn_id, provider_key, reasoning, tool, ts) = row?;
        count += 1;
        let attribution = snapshot
            .as_ref()
            .map_or_else(crate::attribution::AttributionView::platform, |snap| {
                snap.resolve(platform, conn_id.as_deref(), provider_key.as_deref(), model.as_deref())
            });
        let effective_conn = attribution.is_connection().then(|| attribution.connection_id.as_deref()).flatten();
        let at = chrono::DateTime::from_timestamp_millis(ts).unwrap_or_else(chrono::Utc::now);
        let res = resolver.price_request(&crate::pricing::PriceRequest {
            platform,
            model: model.as_deref(),
            upstream_model: upstream.as_deref(),
            served_model: served.as_deref(),
            connection_id: effective_conn,
            input_semantics: &semantics,
            input,
            output,
            cache_read,
            cache_write,
            cache_write_1h,
            reasoning: reasoning.unwrap_or(0),
            tool: tool.unwrap_or(0),
            at,
        });
        if let Some(c) = res.cost {
            cost += c;
            priced += 1;
        }
        if let Some(m) = model.filter(|m| !models.contains(m)) {
            models.push(m);
        }
    }
    Ok((cost, priced, count, models))
}
