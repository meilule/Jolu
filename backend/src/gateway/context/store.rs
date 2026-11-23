//! 上下文快照的落库（表 `gateway_context_snapshot`，建表在 `db.rs` 的 `init`）。
//!
//! 一次请求一行：先写 `analyzing`，解析完与上游回了用量后各更新一次。只存名字、tag、次数与 token。
//! 清理与路由流水的清理在同一处（`route_log::record` 每 50 次写调用一次 [`prune`]）。

use rusqlite::{params, Connection, OptionalExtension};

use super::{Prompt, ToolResult};

/// 每个会话保留最近多少条快照。
pub const KEEP_PER_SESSION: i64 = 20;
/// 全表最多保留多少条（会话很多时兜底）。
pub const KEEP_TOTAL: i64 = 2000;

/// 快照状态（执行文档 §3.3；「无行」不是状态，是没有这一行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// 已收到请求，解析还没完成。
    Analyzing,
    /// 请求体为空，没有可分析的内容。
    NoBody,
    /// 解析出错（只在调试构建可能出现；发布版 panic 直接退出，见 mod.rs）。
    Failed,
    /// 请求体不是该协议能认的格式。
    Unsupported,
    /// 有完整的组成。
    Ready,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Analyzing => "analyzing",
            State::NoBody => "no_body",
            State::Failed => "failed",
            State::Unsupported => "unsupported",
            State::Ready => "ready",
        }
    }
}

/// 上游实测的用量。各项未知时为 `None`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// 输入总量（含缓存读写）。
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
}

/// 新建一行时已知的信息。
pub struct NewSnapshot<'a> {
    pub session_platform: &'a str,
    pub session_id: &'a str,
    pub request_id: i64,
    pub request_uid: Option<i64>,
    pub protocol: &'a str,
    pub ts: i64,
}

/// 写 `analyzing` 行，返回行 id。
pub fn insert(conn: &Connection, s: &NewSnapshot<'_>) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO gateway_context_snapshot(session_platform, session_id, request_id, ts, state, protocol, request_uid)
         VALUES(?1, ?2, ?3, ?4, 'analyzing', ?5, ?6)",
        params![s.session_platform, s.session_id, s.request_id, s.ts, s.protocol, s.request_uid],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 一行当前该写成的样子。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub state: State,
    pub connection_id: Option<String>,
    pub usage: Usage,
    pub window: Option<i64>,
    /// 已按实测校准过的组成（有实测量时）或估算（没有时）。
    pub prompt: Option<Prompt>,
    pub error: Option<String>,
}

/// 把 `row` 写进 id 那一行。
pub fn update(conn: &Connection, id: i64, row: &Row) -> rusqlite::Result<()> {
    let parts = row.prompt.as_ref().map(|p| serde_json::to_string(&p.parts).unwrap_or_else(|_| "[]".into()));
    let calls = row.prompt.as_ref().map(|p| serde_json::to_string(&p.calls).unwrap_or_else(|_| "[]".into()));
    conn.execute(
        "UPDATE gateway_context_snapshot SET state = ?2, connection_id = ?3, input_tokens = ?4, output_tokens = ?5,
                cache_read = ?6, window_tokens = ?7, estimated = ?8, counted = ?9, parts_json = ?10, calls_json = ?11, error = ?12
         WHERE id = ?1",
        params![
            id,
            row.state.as_str(),
            row.connection_id,
            row.usage.input,
            row.usage.output,
            row.usage.cache_read,
            row.window,
            row.prompt.as_ref().map(|p| p.estimated),
            row.prompt.as_ref().is_some_and(|p| p.counted),
            parts,
            calls,
            row.error,
        ],
    )?;
    Ok(())
}

/// 每个会话只留最近 [`KEEP_PER_SESSION`] 条，全表最多 [`KEEP_TOTAL`] 条。
pub fn prune(conn: &Connection) {
    let _ = conn.execute(
        "DELETE FROM gateway_context_snapshot WHERE id IN (
           SELECT id FROM (
             SELECT id, ROW_NUMBER() OVER (PARTITION BY session_platform, session_id ORDER BY id DESC) AS rn
             FROM gateway_context_snapshot
           ) WHERE rn > ?1
         )",
        [KEEP_PER_SESSION],
    );
    let _ = conn.execute(
        "DELETE FROM gateway_context_snapshot WHERE id NOT IN (SELECT id FROM gateway_context_snapshot ORDER BY id DESC LIMIT ?1)",
        [KEEP_TOTAL],
    );
}

/// 读回的一行（测试与第 3 期查询命令用）。
#[allow(dead_code)] // 第 3 期查询命令接入前只有测试在用
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub id: i64,
    pub session_platform: String,
    pub session_id: String,
    pub request_id: Option<i64>,
    pub connection_id: Option<String>,
    pub ts: i64,
    pub state: String,
    pub protocol: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read: Option<i64>,
    pub window_tokens: Option<i64>,
    pub estimated: Option<i64>,
    pub counted: bool,
    pub parts: Option<Vec<super::PromptPart>>,
    pub calls: Option<Vec<ToolResult>>,
    pub error: Option<String>,
}

/// 按 id 读一行。
#[allow(dead_code)] // 同上
pub fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Stored>> {
    conn.query_row(
        "SELECT id, session_platform, session_id, request_id, connection_id, ts, state, protocol, input_tokens, output_tokens,
                cache_read, window_tokens, estimated, counted, parts_json, calls_json, error
         FROM gateway_context_snapshot WHERE id = ?1",
        [id],
        |r| {
            let parts: Option<String> = r.get(14)?;
            let calls: Option<String> = r.get(15)?;
            Ok(Stored {
                id: r.get(0)?,
                session_platform: r.get(1)?,
                session_id: r.get(2)?,
                request_id: r.get(3)?,
                connection_id: r.get(4)?,
                ts: r.get(5)?,
                state: r.get(6)?,
                protocol: r.get(7)?,
                input_tokens: r.get(8)?,
                output_tokens: r.get(9)?,
                cache_read: r.get(10)?,
                window_tokens: r.get(11)?,
                estimated: r.get(12)?,
                counted: r.get::<_, i64>(13)? != 0,
                parts: parts.and_then(|t| serde_json::from_str(&t).ok()),
                calls: calls.and_then(|t| serde_json::from_str(&t).ok()),
                error: r.get(16)?,
            })
        },
    )
    .optional()
}

/// 某请求的快照 id。
#[allow(dead_code)] // 同上
pub fn id_of_request(conn: &Connection, request_id: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row("SELECT id FROM gateway_context_snapshot WHERE request_id = ?1 ORDER BY id DESC LIMIT 1", [request_id], |r| r.get(0))
        .optional()
}
