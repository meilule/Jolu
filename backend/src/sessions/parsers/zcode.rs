use std::collections::HashSet;
use std::path::PathBuf;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value as Json;
use crate::sessions::types::{Message, MessagePage, TokenUsage, ToolCall};
use super::{clip, output_text, session_cost, tool_paths, tool_summary, Limits, VIEW};

/* ------------------------------ ZCode 库型来源 ------------------------------ */

/// 单会话最多解析的 part 条数。本机 477 MB 的库里单个会话最多有 14,404 条 part，
/// 一次读全会把整个会话 JSON 拉进内存；超出后只解析最近的部分，并在页面上标注截断。
pub(crate) const ZCODE_PART_LIMIT: usize = 2_000;
/// 单会话正文解析的字节上限：`part.data` 是原样的工具输出（本机单个 part 最大约 290 KB），
/// 只按条数收口挡不住「几千条大 part」——2000 条 × 数百 KB 会一次占住数百 MB 甚至更多。
/// 与条数上限一起生效，先到哪个算哪个，仍然只保留最近的记录。
pub(crate) const ZCODE_PART_BYTES: usize = 32 * 1024 * 1024;
/// 库型来源的等待上限：原工具可能正在写 WAL，宁可稍等也不认为会话正文不可读
pub(crate) const ZCODE_BUSY_MS: u64 = 400;

/// ZCode 会话库：`~/.zcode/cli/db/db.sqlite`。缺失时返回 None，由调用方降级。
pub(crate) fn zcode_db_path() -> Option<PathBuf> {
    if crate::validation::isolated() { return None; }
    Some(crate::creds::home()?.join(".zcode").join("cli").join("db").join("db.sqlite"))
}

/// 只读打开 ZCode 库。`SQLITE_OPEN_READ_ONLY` 与 `mode=ro` 等价：连接不持有写锁、
/// 不会创建或修改 `.zcode` 下任何文件（WAL 的 `-shm` 已由原工具创建，只读取用）。
pub(crate) fn open_zcode_db() -> Option<Connection> {
    let path = zcode_db_path().filter(|p| p.is_file())?;
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(ZCODE_BUSY_MS));
    Some(conn)
}

/// 结构探测：原工具升级会让列漂移，探测不到就当作「没有正文」，绝不猜列名或 mock 数据。
pub(crate) fn zcode_schema_ok(conn: &Connection) -> bool {
    conn.prepare("SELECT id, session_id, sequence, time_created, data FROM message LIMIT 0").is_ok()
        && conn.prepare("SELECT id, message_id, sequence, time_created, data FROM part LIMIT 0").is_ok()
}

/// 子会话 id 集合（`parent_id` 非空）。ZCode 把子代理会话也写进同一张 `session` 表：
/// 171 条里 146 条 `parent_id` 非空（workflow_child 100 / subagent_child 41 / fork 2 /
/// selection_side_chat 3），只有 25 条是用户真正打开的根会话。
///
/// 列表规则：只把根会话当主会话展示（子会话不进列表，因此也不会刷屏）；子会话不折叠展示，
/// 因为它的正文只是父会话里一次 Agent 调用的展开，父会话正文已包含该调用的结果。
/// 子会话自己的正文仍可按 id 只读查询（例如从别处拿到 id），只是不会出现在列表里。
pub(crate) fn zcode_child_sessions() -> Option<HashSet<String>> {
    let conn = open_zcode_db()?;
    let mut stmt = conn.prepare("SELECT id FROM session WHERE parent_id IS NOT NULL AND parent_id <> ''").ok()?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).ok()?;
    let out: HashSet<String> = rows.filter_map(Result::ok).collect();
    (!out.is_empty()).then_some(out)
}

/// Jolu 的请求表把 ZCode 会话记成 `{来源 id}:{原始会话 id}`，例如
/// `native:zcode:main:sess_0a15…`；查原库前必须还原成裸 id，否则一条 message 都匹配不上。
/// 没有 `native:` 前缀（含直接传裸 id）时按原样使用。
pub(crate) fn zcode_raw_session_id(session_id: &str) -> &str {
    if session_id.starts_with("native:") {
        session_id.rsplit(':').next().filter(|id| !id.is_empty()).unwrap_or(session_id)
    } else {
        session_id
    }
}

/// 列表里的这一条是否是 ZCode 子代理会话（不在主列表展示）。子会话集合不可用时一律当作根会话。
pub(crate) fn zcode_child_session(children: Option<&HashSet<String>>, platform: &str, session_id: &str) -> bool {
    platform == "zcode" && children.is_some_and(|children| children.contains(zcode_raw_session_id(session_id)))
}

/// 一条消息的重建结果：`Ok(None)` 表示这条不该展示（系统注入 / 模型专用），`Err(())` 表示 data 坏了
pub(crate) fn zcode_message(data: &str, parts: &[String], limits: Limits) -> Result<Option<Message>, ()> {
    let Ok(meta) = serde_json::from_str::<Json>(data) else { return Err(()) };
    // 注入给模型、没写进用户可见记录的内容不显示（与 Claude 的 isMeta / isInjected 同一口径）
    let hidden = meta.get("synthetic").and_then(Json::as_bool) == Some(true)
        || meta.pointer("/metadata/visibility").and_then(Json::as_str) == Some("model-only")
        || matches!(meta.pointer("/semantics/uiVisibility").and_then(Json::as_str), Some("hidden"))
        || matches!(meta.pointer("/semantics/transcriptVisibility").and_then(Json::as_str), Some("hidden"));
    if hidden { return Ok(None); }
    let role = meta.get("role").and_then(Json::as_str).unwrap_or("");
    if !matches!(role, "user" | "assistant") { return Ok(None); }

    let ts = meta.pointer("/time/created").and_then(Json::as_i64);
    let mut texts: Vec<String> = Vec::new();
    let mut thoughts: Vec<String> = Vec::new();
    let mut tools: Vec<ToolCall> = Vec::new();
    // step-finish 是每条模型调用的结算节点，消息级 tokens 缺失时用它累计
    let mut step_tokens = TokenUsage::default();
    let mut step_cost = 0.0f64;
    let mut step_seen = false;
    for raw in parts {
        let Ok(part) = serde_json::from_str::<Json>(raw) else { continue };
        match part.get("type").and_then(Json::as_str).unwrap_or("") {
            "text" => {
                if let Some(text) = part.get("text").and_then(Json::as_str).map(str::trim).filter(|t| !t.is_empty()) {
                    texts.push(text.to_string());
                }
            }
            "reasoning" => {
                if let Some(text) = part.get("text").and_then(Json::as_str).map(str::trim).filter(|t| !t.is_empty()) {
                    thoughts.push(text.to_string());
                }
            }
            "tool" => {
                if let Some(tool) = zcode_tool(&part, limits) { tools.push(tool); }
            }
            "step-finish" => {
                step_seen = true;
                step_cost += part.get("cost").and_then(Json::as_f64).unwrap_or(0.0);
                zcode_add_tokens(&mut step_tokens, part.get("tokens"));
            }
            // step-start：只有计时，没有内容；file / timeline / compaction：附件与分隔标记，
            // 不属于消息正文（附件路径已在工具调用摘要里出现），不拼进对话
            _ => {}
        }
    }

    let text = clip(&texts.join("\n\n"), limits.text);
    let tokens = zcode_tokens(meta.get("tokens")).or(step_seen.then(|| step_tokens.clone()));
    let cost = meta.get("cost").and_then(Json::as_f64).or(step_seen.then_some(step_cost));
    let reasoning = (!thoughts.is_empty()).then(|| clip(&thoughts.join("\n\n"), limits.text));
    if text.is_empty() && tools.is_empty() && reasoning.is_none() {
        return Ok(None);
    }
    Ok(Some(Message {
        role: role.into(),
        ts,
        text,
        tools,
        reasoning,
        model: meta.get("modelId").and_then(Json::as_str).map(String::from),
        provider: meta.get("providerId").and_then(Json::as_str).map(String::from),
        tokens,
        cost,
    }))
}

/// `part.type == "tool"`：摘要、输出、状态都在 `state` 里，调用与结果是同一条记录
pub(crate) fn zcode_tool(part: &Json, limits: Limits) -> Option<ToolCall> {
    let state = part.get("state")?;
    let input = state.get("input").cloned().unwrap_or(Json::Null);
    let summary = {
        let from_input = tool_summary(&input, limits);
        if from_input.is_empty() {
            state.get("title").and_then(Json::as_str).map(|t| limits.summary(t)).unwrap_or_default()
        } else {
            from_input
        }
    };
    // `state.output` 只在工具结束时才有；运行中没有输出就返回 None，不补写结果
    let output = state.get("output").and_then(output_text).map(|t| limits.output(&t));
    let failed = match state.get("status").and_then(Json::as_str) {
        Some("error") => Some(true),
        Some("completed") => Some(false),
        _ => None,
    };
    Some(ToolCall {
        name: part.get("tool").and_then(Json::as_str).unwrap_or("工具").to_string(),
        summary,
        output,
        failed,
        // ZCode 不单独记退出码，输出文本里也不保证有；不推断
        exit_code: None,
        paths: tool_paths(&input),
    })
}

pub(crate) fn zcode_tokens(value: Option<&Json>) -> Option<TokenUsage> {
    let value = value?;
    let number = |v: Option<&Json>| v.and_then(Json::as_i64);
    let usage = TokenUsage {
        total: number(value.get("total")),
        input: number(value.get("input")),
        output: number(value.get("output")),
        reasoning: number(value.get("reasoning")),
        cache_read: number(value.pointer("/cache/read")),
        cache_write: number(value.pointer("/cache/write")),
    };
    (usage.total.is_some() || usage.input.is_some() || usage.output.is_some()).then_some(usage)
}

pub(crate) fn zcode_add_tokens(into: &mut TokenUsage, value: Option<&Json>) {
    let Some(usage) = zcode_tokens(value) else { return };
    let acc = |slot: &mut Option<i64>, add: Option<i64>| {
        if let Some(add) = add { *slot = Some(slot.unwrap_or(0) + add); }
    };
    acc(&mut into.total, usage.total);
    acc(&mut into.input, usage.input);
    acc(&mut into.output, usage.output);
    acc(&mut into.reasoning, usage.reasoning);
    acc(&mut into.cache_read, usage.cache_read);
    acc(&mut into.cache_write, usage.cache_write);
}

#[derive(Default)]
pub(crate) struct ZcodeStream {
    pub(crate) messages: Vec<Message>,
    /// 达到 part 上限、只解析了最近的部分
    pub(crate) truncated: bool,
    /// `data` 坏了被跳过的消息条数
    pub(crate) skipped: usize,
}

/// 截取最近的 part：条数与字节双重上限；至少保留最新一条
/// （单条 part 自己就超过字节预算时，也不能让最新一次对话整段消失）。
pub(crate) fn keep_recent_parts(rows: &mut Vec<String>, part_budget: usize, byte_budget: usize) -> bool {
    let mut truncated = false;
    if rows.len() > part_budget {
        rows.drain(..rows.len() - part_budget);
        truncated = true;
    }
    if rows.iter().map(String::len).sum::<usize>() > byte_budget {
        let mut keep_from = rows.len();
        let mut kept = 0usize;
        while keep_from > 0 {
            let size = rows[keep_from - 1].len();
            if kept + size > byte_budget { break; }
            kept += size;
            keep_from -= 1;
        }
        if keep_from == rows.len() && !rows.is_empty() {
            keep_from = rows.len() - 1;
        }
        rows.drain(..keep_from);
        truncated = true;
    }
    truncated
}

/// 按会话 id 走索引重建消息流。
///
/// 索引：`message(session_id, sequence, ...)`（`message_session_sequence_idx`）与
/// `part(message_id, sequence, ...)`（`part_message_sequence_idx`），不使用全表扫描。
/// 方向：先按 `sequence` 倒序取消息、再倒序还原，保证被截断时留下的是最近的对话；
/// `sequence` 可能为 NULL（旧记录），用最大值兜底让它排在末尾。
pub(crate) fn zcode_load(conn: &Connection, session_id: &str, limits: Limits, part_budget: usize) -> Result<ZcodeStream, String> {
    zcode_load_with(conn, session_id, limits, part_budget, ZCODE_PART_BYTES)
}

/// 同 [`zcode_load`]，但字节预算由调用方给定（测试用）。
pub(crate) fn zcode_load_with(
    conn: &Connection,
    session_id: &str,
    limits: Limits,
    part_budget: usize,
    byte_budget: usize,
) -> Result<ZcodeStream, String> {
    let mut messages = conn.prepare(
        "SELECT id, data FROM message WHERE session_id = ?1
         ORDER BY COALESCE(sequence, 9223372036854775807) DESC, time_created DESC, id DESC",
    ).map_err(|error| error.to_string())?;
    let mut parts = conn.prepare(
        "SELECT data FROM part WHERE message_id = ?1
         ORDER BY COALESCE(sequence, 9223372036854775807), time_created, id",
    ).map_err(|error| error.to_string())?;
    let rows = messages.query_map([session_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|error| error.to_string())?;
    let mut out = ZcodeStream::default();
    let mut used = 0usize;
    let mut bytes_used = 0usize;
    for row in rows {
        let (message_id, data) = row.map_err(|error| error.to_string())?;
        if used >= part_budget || bytes_used >= byte_budget {
            out.truncated = true;
            break;
        }
        let mut rows: Vec<String> = parts.query_map([&message_id], |r| r.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .collect();
        // 单条消息自己就超过上限时也要留下内容（否则最新一次对话会整段消失）：
        // 只保留它最近的 `part_budget` 条 / 字节预算内的 part，并标记截断
        if out.messages.is_empty()
            && (rows.len() > part_budget || rows.iter().map(String::len).sum::<usize>() > byte_budget)
        {
            out.truncated = true;
            keep_recent_parts(&mut rows, part_budget, byte_budget);
        }
        let message_bytes: usize = rows.iter().map(String::len).sum();
        // 第一条消息按上面的裁剪结果照常解析：单条 part 就超预算时也要能打开对话。
        let over_parts = used > 0 && used + rows.len() > part_budget;
        let over_bytes = bytes_used > 0 && bytes_used + message_bytes > byte_budget;
        if over_parts || over_bytes {
            out.truncated = true;
            break;
        }
        used += rows.len();
        bytes_used += message_bytes;
        match zcode_message(&data, &rows, limits) {
            Ok(Some(message)) => out.messages.push(message),
            Ok(None) => {}
            // 单条 data 解析失败只跳过该条，不牵连整页
            Err(()) => out.skipped += 1,
        }
    }
    out.messages.reverse();
    Ok(out)
}

/// 库型来源的分页：正文来自 ZCode 库，`models` / 费用仍来自 Jolu 自己的请求表
pub(crate) fn zcode_page(conn: &Connection, zdb: Option<&Connection>, session_id: &str, before: Option<usize>, limit: usize) -> Result<MessagePage, String> {
    zcode_page_with(conn, zdb, session_id, before, limit, ZCODE_PART_LIMIT)
}

pub(crate) fn zcode_page_with(conn: &Connection, zdb: Option<&Connection>, session_id: &str, before: Option<usize>, limit: usize, part_budget: usize) -> Result<MessagePage, String> {
    // 费用与模型来自 Jolu 自己的请求表（用带前缀的原始 session_id），正文来自 ZCode 的裸 id
    let (cost, priced_requests, requests, models) = session_cost(conn, "zcode", session_id).map_err(|error| error.to_string())?;
    let raw_id = zcode_raw_session_id(session_id);
    let (stream, available) = match zdb {
        Some(zdb) if zcode_schema_ok(zdb) => (zcode_load(zdb, raw_id, VIEW, part_budget)?, true),
        _ => (ZcodeStream::default(), false),
    };
    let mut note = Vec::new();
    if stream.truncated {
        note.push(format!(
            "只解析了本会话最近的部分记录（上限 {ZCODE_PART_LIMIT} 条 / {} MB），更早的正文请在原工具查看",
            ZCODE_PART_BYTES / (1024 * 1024)
        ));
    }
    if stream.skipped > 0 {
        note.push(format!("{} 条记录格式无法解析，已跳过", stream.skipped));
    }
    let total = stream.messages.len();
    let end = before.unwrap_or(total).min(total);
    let start = end.saturating_sub(limit);
    Ok(MessagePage {
        available,
        total,
        offset: start,
        messages: stream.messages[start..end].to_vec(),
        models,
        cost,
        priced_requests,
        requests,
        note: (!note.is_empty()).then(|| note.join("；")),
    })
}
