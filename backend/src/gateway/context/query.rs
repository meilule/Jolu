//! 上下文快照的查询（第 3 期，执行文档 §3.4 的 5 个命令都落在这里，命令层只做转发）。
//!
//! 链路信息按持久请求身份和会话身份取路由流水里这次请求的最后一条。
//! 历史数据缺少持久请求身份时链路保留未知。
//! 未知一律返回 `null`：没有上游用量、模型窗口查不到、流水已被清理，都不写成 0 或空串。

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;

use super::{Kind, PromptPart, ToolResult};

/// 会话列表最多返回多少个会话。
pub const SESSIONS_LIMIT: i64 = 100;
/// 历史最多返回多少条（与每个会话保留的条数一致）。
pub const HISTORY_LIMIT: i64 = super::store::KEEP_PER_SESSION;

/// 一条快照的概要与链路。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SnapshotSummary {
    pub id: i64,
    pub session_platform: String,
    pub session_id: String,
    pub request_id: Option<i64>,
    pub ts: i64,
    /// analyzing / ready / unsupported / no_body / failed
    pub state: String,
    pub protocol: String,
    /// 上游实测的输入总量（含缓存读写）
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read: Option<i64>,
    pub window_tokens: Option<i64>,
    /// 本地估算总量
    pub estimated: Option<i64>,
    pub counted: bool,
    pub error: Option<String>,
    pub agent: Option<String>,
    pub requested_model: Option<String>,
    pub upstream_model: Option<String>,
    pub source: Option<String>,
    /// 路由结果：ok / failed / rejected；流水已被清理时为 null
    pub route_result: Option<String>,
    pub status_code: Option<i64>,
    pub duration_ms: Option<i64>,
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub connection_platform: Option<String>,
}

/// 会话列表的一行：最近一次快照，加上这个会话最近一次 ready 的时间（失败时界面写「上次成功 HH:MM」）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionRow {
    #[serde(flatten)]
    pub latest: SnapshotSummary,
    pub last_ready_ts: Option<i64>,
    /// 这个会话保留的快照条数
    pub snapshots: i64,
}

/// 一条快照的全部内容。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SnapshotDetail {
    #[serde(flatten)]
    pub summary: SnapshotSummary,
    /// 八类组成；没有（unsupported / no_body / failed / 还在 analyzing）时为 null
    pub parts: Option<Vec<PromptPart>>,
    /// 按工具聚合的结果：工具名、条数、token
    pub tools: Vec<ToolGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolGroup {
    pub tool: String,
    pub count: i64,
    pub tokens: i64,
}

/// 明细表格的一行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemRow {
    pub kind: Kind,
    pub name: String,
    pub tag: String,
    pub n: Option<i64>,
    pub tokens: i64,
    /// 占这次输入的比例（0–1）；总量为 0 时为 null
    pub share: Option<f64>,
}

const SUMMARY_COLUMNS: &str = "s.id, s.session_platform, s.session_id, s.request_id, s.ts, s.state, s.protocol,
    s.input_tokens, s.output_tokens, s.cache_read, s.window_tokens, s.estimated, s.counted, s.error,
    r.agent, r.requested_model, r.upstream_model, r.source, r.result, r.status_code, r.duration_ms,
    s.connection_id, c.name, c.platform";

const SUMMARY_JOINS: &str = "LEFT JOIN gateway_route_log r ON r.id = (
        SELECT MAX(l.id) FROM gateway_route_log l
        WHERE l.request_uid = s.request_uid AND s.request_uid IS NOT NULL
          AND l.session_platform = s.session_platform AND l.session_id = s.session_id
          AND (SELECT COUNT(*) FROM gateway_context_snapshot x WHERE x.request_uid = s.request_uid) = 1
    )
    LEFT JOIN connections c ON c.id = s.connection_id";

fn summary_of(r: &Row<'_>) -> rusqlite::Result<SnapshotSummary> {
    Ok(SnapshotSummary {
        id: r.get(0)?,
        session_platform: r.get(1)?,
        session_id: r.get(2)?,
        request_id: r.get(3)?,
        ts: r.get(4)?,
        state: r.get(5)?,
        protocol: r.get(6)?,
        input_tokens: r.get(7)?,
        output_tokens: r.get(8)?,
        cache_read: r.get(9)?,
        window_tokens: r.get(10)?,
        estimated: r.get(11)?,
        counted: r.get::<_, i64>(12)? != 0,
        error: r.get(13)?,
        agent: r.get(14)?,
        requested_model: r.get(15)?,
        upstream_model: r.get(16)?,
        source: r.get(17)?,
        route_result: r.get(18)?,
        status_code: r.get(19)?,
        duration_ms: r.get(20)?,
        connection_id: r.get(21)?,
        connection_name: r.get(22)?,
        connection_platform: r.get(23)?,
    })
}

/// `gateway_context_sessions`：每个会话最近一次快照，按时间倒序。
pub fn sessions(conn: &Connection) -> rusqlite::Result<Vec<SessionRow>> {
    let sql = format!(
        "WITH ranked AS (
           SELECT *, ROW_NUMBER() OVER (PARTITION BY session_platform, session_id ORDER BY id DESC) AS rn,
                  COUNT(*) OVER (PARTITION BY session_platform, session_id) AS total
           FROM gateway_context_snapshot
         )
         SELECT {SUMMARY_COLUMNS}, s.total,
                (SELECT MAX(ts) FROM gateway_context_snapshot x
                  WHERE x.session_platform = s.session_platform AND x.session_id = s.session_id AND x.state = 'ready')
         FROM ranked s {SUMMARY_JOINS}
         WHERE s.rn = 1
         ORDER BY s.ts DESC, s.id DESC
         LIMIT ?1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([SESSIONS_LIMIT], |r| Ok(SessionRow { latest: summary_of(r)?, snapshots: r.get(24)?, last_ready_ts: r.get(25)? }))?;
    rows.collect()
}

/// `gateway_context_history`：一个会话保留的快照，新的在前。
pub fn history(conn: &Connection, platform: &str, session_id: &str) -> rusqlite::Result<Vec<SnapshotSummary>> {
    let sql = format!(
        "SELECT {SUMMARY_COLUMNS} FROM gateway_context_snapshot s {SUMMARY_JOINS}
         WHERE s.session_platform = ?1 AND s.session_id = ?2
         ORDER BY s.id DESC LIMIT ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![platform, session_id, HISTORY_LIMIT], summary_of)?;
    rows.collect()
}

/// 读一条快照的概要与原始 JSON。
fn load(conn: &Connection, id: i64) -> rusqlite::Result<Option<(SnapshotSummary, Option<String>, Option<String>)>> {
    let sql = format!("SELECT {SUMMARY_COLUMNS}, s.parts_json, s.calls_json FROM gateway_context_snapshot s {SUMMARY_JOINS} WHERE s.id = ?1");
    conn.query_row(&sql, [id], |r| Ok((summary_of(r)?, r.get(24)?, r.get(25)?))).optional()
}

fn calls_of(json: Option<&str>) -> Vec<ToolResult> {
    json.and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default()
}

/// `gateway_context_snapshot`：指定 id；没给 id 时取这个会话最近一次。没有这一行返回 `None`（界面写「暂无请求」）。
pub fn snapshot(conn: &Connection, platform: &str, session_id: &str, id: Option<i64>) -> rusqlite::Result<Option<SnapshotDetail>> {
    let id = match id {
        Some(id) => Some(id),
        None => conn
            .query_row(
                "SELECT id FROM gateway_context_snapshot WHERE session_platform = ?1 AND session_id = ?2 ORDER BY id DESC LIMIT 1",
                params![platform, session_id],
                |r| r.get(0),
            )
            .optional()?,
    };
    let Some(id) = id else { return Ok(None) };
    let Some((summary, parts, calls)) = load(conn, id)? else { return Ok(None) };
    // 指定的 id 不属于这个会话时当作没有，免得界面串会话
    if summary.session_platform != platform || summary.session_id != session_id {
        return Ok(None);
    }
    let parts: Option<Vec<PromptPart>> = parts.and_then(|t| serde_json::from_str(&t).ok());
    let mut tools: Vec<ToolGroup> = Vec::new();
    for call in calls_of(calls.as_deref()) {
        match tools.iter_mut().find(|g| g.tool == call.tool) {
            Some(g) => {
                g.count += 1;
                g.tokens += call.tokens;
            }
            None => tools.push(ToolGroup { tool: call.tool, count: 1, tokens: call.tokens }),
        }
    }
    tools.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.tool.cmp(&b.tool)));
    Ok(Some(SnapshotDetail { summary, parts, tools }))
}

/// `gateway_context_items`：把八类组成摊平成表格行，可按类别与名称（不区分大小写）筛选。
/// 占比的分母是这次的总量（校准后即上游实测输入）。快照不存在返回 `None`。
pub fn items(conn: &Connection, id: i64, kind: Option<Kind>, search: Option<&str>) -> rusqlite::Result<Option<Vec<ItemRow>>> {
    let Some((_, parts, _)) = load(conn, id)? else { return Ok(None) };
    let parts: Vec<PromptPart> = parts.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let total: i64 = parts.iter().map(|p| p.tokens).sum();
    let needle = search.map(str::trim).filter(|s| !s.is_empty()).map(str::to_lowercase);
    let mut rows = Vec::new();
    for part in parts.iter().filter(|p| kind.is_none_or(|k| p.kind == k)) {
        for it in &part.items {
            if needle.as_ref().is_some_and(|n| !it.name.to_lowercase().contains(n.as_str())) {
                continue;
            }
            rows.push(ItemRow {
                kind: part.kind,
                name: it.name.clone(),
                tag: it.tag.clone(),
                n: (it.n > 0).then_some(it.n),
                tokens: it.tokens,
                share: (total > 0).then(|| it.tokens as f64 / total as f64),
            });
        }
    }
    Ok(Some(rows))
}

/// `gateway_context_tool_items`：逐条工具结果，可只看某个工具。快照不存在返回 `None`。
pub fn tool_items(conn: &Connection, id: i64, tool: Option<&str>) -> rusqlite::Result<Option<Vec<ToolResult>>> {
    let Some((_, _, calls)) = load(conn, id)? else { return Ok(None) };
    Ok(Some(calls_of(calls.as_deref()).into_iter().filter(|c| tool.is_none_or(|t| c.tool == t)).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::context::store::{self, NewSnapshot, Row as StoreRow, State, Usage};
    use crate::gateway::context::{parse, Protocol};

    const CLAUDE: &str = include_str!("testdata/claude-code-2.1.326-3.anthropic.json");

    fn conn() -> Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn put(conn: &Connection, sid: &str, request_id: i64, ts: i64, state: State, usage: Usage) -> i64 {
        let id = store::insert(conn, &NewSnapshot { session_platform: "claude", session_id: sid, request_id, request_uid: Some(request_id), protocol: "anthropic", ts }).unwrap();
        let prompt = (state == State::Ready).then(|| {
            let p = parse(Protocol::Anthropic, CLAUDE.as_bytes()).unwrap();
            match usage.input {
                Some(n) => p.calibrated(n),
                None => p,
            }
        });
        store::update(conn, id, &StoreRow { state, connection_id: Some("c1".into()), usage, window: Some(200_000), prompt, error: None }).unwrap();
        id
    }

    fn route(conn: &Connection, request_id: i64, source: &str, result: &str) {
        crate::gateway::route_log::record(
            conn,
            crate::gateway::route_log::NewEntry {
                agent: Some("claude-code"),
                requested_model: "kimi/kimi-k2",
                source: Some(source),
                upstream_model: Some("kimi-k2"),
                status_code: Some(200),
                result,
                duration_ms: Some(900),
                connection_id: Some("c1"),
                session_platform: Some("claude"),
                session_id: Some("a"),
                client_key: None,
                request_id: Some(request_id),
                request_uid: Some(request_id),
                resolved_from: None,
                profile: None,
                decision: None,
            },
        );
    }

    #[test]
    fn sessions_take_latest_with_route_and_last_ready() {
        let conn = conn();
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug) VALUES('c1','claude','auth','工作账号','x','****',1,'claude')",
            [],
        )
        .unwrap();
        put(&conn, "a", 1, 100, State::Ready, Usage { input: Some(50_000), output: Some(10), cache_read: Some(40_000) });
        put(&conn, "a", 2, 200, State::Failed, Usage::default());
        put(&conn, "b", 3, 150, State::Unsupported, Usage::default());
        route(&conn, 1, "first-try", "failed");
        route(&conn, 1, "kimi", "ok");
        route(&conn, 2, "kimi", "ok");
        let rows = sessions(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        let a = &rows[0];
        assert_eq!((a.latest.session_id.as_str(), a.latest.state.as_str(), a.latest.request_id), ("a", "failed", Some(2)));
        assert_eq!((a.last_ready_ts, a.snapshots), (Some(100), 2));
        assert_eq!((a.latest.input_tokens, a.latest.cache_read), (None, None), "失败的那次没有用量，返回 null 不是 0");
        assert_eq!(a.latest.connection_name.as_deref(), Some("工作账号"));
        let b = &rows[1];
        assert_eq!((b.latest.session_id.as_str(), b.last_ready_ts, b.latest.route_result.clone()), ("b", None, None), "流水里没有就是 null");
        // 换候选：链路取这次请求的最后一条流水
        let hist = history(&conn, "claude", "a").unwrap();
        assert_eq!(hist.iter().map(|s| s.request_id).collect::<Vec<_>>(), vec![Some(2), Some(1)]);
        assert_eq!((hist[1].source.as_deref(), hist[1].route_result.as_deref()), (Some("kimi"), Some("ok")));
    }

    #[test]
    fn route_is_unknown_when_request_id_belongs_to_another_session_or_multiple_snapshots() {
        let conn = conn();
        put(&conn, "b", 1, 100, State::Failed, Usage::default());
        route(&conn, 1, "other-session", "ok");
        let b = history(&conn, "claude", "b").unwrap();
        assert_eq!(b[0].source, None);

        put(&conn, "a", 1, 200, State::Failed, Usage::default());
        let a = history(&conn, "claude", "a").unwrap();
        assert_eq!(a[0].source, None, "重复编号的历史快照不能猜测归属");
    }

    #[test]
    fn snapshot_items_and_tools() {
        let conn = conn();
        let id = put(&conn, "a", 1, 100, State::Ready, Usage { input: Some(30_000), output: None, cache_read: None });
        let detail = snapshot(&conn, "claude", "a", None).unwrap().unwrap();
        assert_eq!(detail.summary.id, id);
        assert_eq!(detail.parts.as_ref().unwrap().iter().map(|p| p.tokens).sum::<i64>(), 30_000);
        assert_eq!(detail.tools.iter().map(|g| (g.tool.as_str(), g.count)).collect::<Vec<_>>().len(), 2);
        assert!(snapshot(&conn, "claude", "other", Some(id)).unwrap().is_none(), "别的会话拿不到这条");
        assert!(snapshot(&conn, "claude", "none", None).unwrap().is_none());

        let all = items(&conn, id, None, None).unwrap().unwrap();
        let share: f64 = all.iter().filter_map(|r| r.share).sum();
        assert!((share - 1.0).abs() < 1e-9, "各项占比之和为 1：{share}");
        let files = items(&conn, id, Some(Kind::Files), None).unwrap().unwrap();
        assert!(!files.is_empty() && files.iter().all(|r| r.kind == Kind::Files));
        let found = items(&conn, id, None, Some("README")).unwrap().unwrap();
        assert!(!found.is_empty() && found.iter().all(|r| r.name.to_lowercase().contains("readme")));
        assert!(items(&conn, 999, None, None).unwrap().is_none());

        let bash = tool_items(&conn, id, Some("Bash")).unwrap().unwrap();
        assert_eq!(bash.len(), 1);
        assert_eq!(bash[0].target.as_deref(), Some("cat src/cart.ts"));
        assert_eq!(tool_items(&conn, id, None).unwrap().unwrap().len(), 2);
    }

    #[test]
    fn unsupported_snapshot_has_no_parts() {
        let conn = conn();
        put(&conn, "a", 1, 100, State::Unsupported, Usage::default());
        let detail = snapshot(&conn, "claude", "a", None).unwrap().unwrap();
        assert_eq!((detail.summary.state.as_str(), detail.parts.is_none(), detail.tools.len()), ("unsupported", true, 0));
        assert_eq!(items(&conn, detail.summary.id, None, None).unwrap().unwrap().len(), 0);
    }
}
