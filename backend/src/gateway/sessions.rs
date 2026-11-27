//! 统一网关「模型来源 → 最近会话」的聚合数据（统一网关开发说明 §3.2–3.4、§6.1）。
//!
//! - 会话身份：平台 + 原始会话 ID，只来自请求里已验证的会话标识（见 `wire::session_identity`）；
//! - 来源关系：路由流水里按（会话, 连接）聚合的历史关联 + 当前在途请求；
//! - 显示信息与 Token：沿用会话线程的权威统计口径（可能含经其他入口的消耗，不是「仅经网关」）；
//! - 运行状态：在途请求集合由前端结合快照序号 `seq` 与实时事件维护，这里只给快照。
//!
//! 采集器还没扫到的会话照样返回，显示信息与 Token 标为缺失，不拿示例补齐。

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde::Serialize;

use crate::gateway::activity::LiveRequest;
use crate::gateway::route_log::SessionLink;

#[derive(Debug, Serialize)]
pub struct SessionsOverview {
    /// 在途快照对应的事件序号：前端只应用序号更大的开始 / 结束事件
    pub seq: u64,
    /// 快照时刻（毫秒）
    pub at: i64,
    pub in_flight: Vec<LiveRequest>,
    pub sessions: Vec<GatewaySession>,
}

/// Token 可用性：known = 四个分项都可信；partial = 有分项缺失；missing = 会话线程里还没有这条会话。
#[derive(Debug, Serialize)]
pub struct SessionTokens {
    pub fresh_input: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub output: Option<i64>,
    /// 只有四个分项都可信时才有值（会话线程已把含缓存的输入换算成普通输入，分项互斥）
    pub total: Option<i64>,
    pub state: &'static str,
}

#[derive(Debug, Serialize)]
pub struct GatewaySession {
    pub platform: String,
    pub session_id: String,
    /// 最近一次经网关的 Agent
    pub agent: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    /// 会话线程的最近活动；还没采集到时为 None
    pub thread_last_ts: Option<i64>,
    /// 经网关的最近请求时间（保留范围内）；只有在途请求时为 None
    pub gateway_last_ts: Option<i64>,
    /// 会话线程状态：running / done / failed / recent / usage_only；还没采集到时为 None
    pub thread_status: Option<String>,
    pub tokens: SessionTokens,
    /// 历史来源关联，最近的在前
    pub links: Vec<SessionLink>,
}

/// 会话的平台名 → 接线注册表的 agent id（作为旧流水缺少 `agent` 字段时的兜底反查）。
fn agent_of_platform(platform: &str) -> Option<&'static str> {
    Some(match platform {
        "claude" => "claude-code",
        "codex" => "codex",
        "gemini" => "gemini",
        "antigravity" => "antigravity",
        "opencode" => "opencode",
        "zcode" => "zcode",
        "kimi" => "kimi",
        "dsh" => "dsh",
        "pi" => "pi",
        "pencil" => "pencil",
        "qoder" => "qoder",
        "qoder-cn" => "qoder-cn",
        "qwen" => "qwen",
        "workbuddy" => "workbuddy",
        _ => return None,
    })
}

fn tokens(item: Option<&crate::session_threads::SessionItem>) -> SessionTokens {
    let Some(item) = item else {
        return SessionTokens { fresh_input: None, cache_read: None, cache_write: None, output: None, total: None, state: "missing" };
    };
    let parts = [item.fresh_input, item.cache_read, item.cache_write, item.output];
    // 用共享契约判「完整 / 部分 / 缺失」：与用量页 `token_coverage` 同一套规则
    // （未知不是零、负数当未知）。四个分项都齐全才算 known。
    let coverage = crate::data_contract::Coverage::of(parts);
    let state = coverage.state();
    // 注意语义差别：`Coverage.total` 是「已知部分之和」（部分覆盖时也有值），
    // 而这里的 `total` 要求**四个分项全部可信**——缺一项就不给合计，
    // 因为会话卡片要显示「总 Token」，把已知三项加起来会低估真实用量。
    let total = if coverage.complete() { coverage.total() } else { None };
    SessionTokens {
        fresh_input: item.fresh_input,
        cache_read: item.cache_read,
        cache_write: item.cache_write,
        output: item.output,
        total,
        state,
    }
}

pub fn overview(conn: &Connection) -> rusqlite::Result<SessionsOverview> {
    // 先取在途快照：之后结束的请求会以更大的序号到达前端，不会被这份快照覆盖
    let (seq, in_flight) = crate::gateway::activity::live_snapshot();
    let at = chrono::Local::now().timestamp_millis();
    let links = crate::gateway::route_log::session_links(conn)?;

    let mut order: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut by_key: HashMap<(String, String), Vec<SessionLink>> = HashMap::new();
    for link in links {
        let key = (link.session_platform.clone(), link.session_id.clone());
        if seen.insert(key.clone()) {
            order.push(key.clone());
        }
        by_key.entry(key).or_default().push(link);
    }
    let mut live_agent: HashMap<(String, String), (i64, Option<String>)> = HashMap::new();
    for request in &in_flight {
        if let (Some(platform), Some(session)) = (&request.session_platform, &request.session_id) {
            let key = (platform.clone(), session.clone());
            if seen.insert(key.clone()) {
                order.push(key.clone());
            }
            let entry = live_agent.entry(key).or_insert((request.ts, request.agent.clone()));
            if request.ts >= entry.0 {
                *entry = (request.ts, request.agent.clone());
            }
        }
    }

    // 第三路：网关上下文快照表里保留的已路由会话。
    // 该表只由模型路由入口（wire / gemini）写入，不含未经路由的本机会话；
    // 当路由流水滚动清理掉旧行后，仍保留其会话标识以便回查标题、项目与状态。
    let mut snap_last_ts: HashMap<(String, String), i64> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT session_platform, session_id, MAX(ts) AS last_ts
             FROM gateway_context_snapshot
             WHERE session_platform IS NOT NULL AND session_id IS NOT NULL AND session_id <> ''
             GROUP BY session_platform, session_id
             ORDER BY last_ts DESC",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
        for row in rows {
            let (platform, session_id, ts) = row?;
            let key = (platform, session_id);
            if seen.insert(key.clone()) {
                order.push(key.clone());
            }
            snap_last_ts.insert(key, ts);
        }
    }

    // 只为已确认经过模型路由的会话补标题、项目与 Token，不再扫全量本机会话。
    let threads: HashMap<(String, String), crate::session_threads::SessionItem> = if order.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<String> = order.iter().map(|(_, id)| id.clone()).collect::<HashSet<_>>().into_iter().collect();
        // Kimi / Pi / dsh 的外部扫描不按 `session_ids` 过滤，必须跳过避免白跑磁盘扫描。
        let filter = crate::session_threads::SessionFilter {
            session_ids: Some(ids),
            skip_external: true,
            ..Default::default()
        };
        crate::session_threads::list(conn, &filter)?
            .sessions
            .into_iter()
            .map(|item| ((item.platform.clone(), item.session_id.clone()), item))
            .collect()
    };

    let mut sessions: Vec<GatewaySession> = order
        .into_iter()
        .map(|key| {
            let links = by_key.remove(&key).unwrap_or_default();
            let thread = threads.get(&key);
            let live = live_agent.get(&key);
            let snap_ts = snap_last_ts.get(&key).copied();
            // 优先级：在途请求记的 agent > 流水链接里的 agent > 平台名反查兜底。
            let agent = live
                .and_then(|(_, a)| a.clone())
                .or_else(|| links.first().and_then(|l| l.agent.clone()))
                .or_else(|| agent_of_platform(&key.0).map(str::to_string));
            let gateway_last_ts = [links.iter().map(|l| l.last_ts).max(), snap_ts]
                .into_iter()
                .flatten()
                .max();
            GatewaySession {
                agent,
                title: thread.and_then(|t| t.title.clone()),
                cwd: thread.and_then(|t| t.cwd.clone()),
                thread_last_ts: thread.map(|t| t.last_ts),
                gateway_last_ts,
                thread_status: thread.map(|t| t.status.clone()),
                tokens: tokens(thread),
                links,
                platform: key.0,
                session_id: key.1,
            }
        })
        .collect();
    let activity = |s: &GatewaySession| {
        let live = in_flight
            .iter()
            .filter(|r| r.session_platform.as_deref() == Some(&s.platform) && r.session_id.as_deref() == Some(&s.session_id))
            .map(|r| r.ts)
            .max();
        [s.thread_last_ts, s.gateway_last_ts, live].into_iter().flatten().max().unwrap_or(0)
    };
    sessions.sort_by(|a, b| {
        activity(b)
            .cmp(&activity(a))
            .then_with(|| a.platform.cmp(&b.platform))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    Ok(SessionsOverview { seq, at, in_flight, sessions })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::route_log::{record, NewEntry};

    fn link<'a>(conn_id: &'a str, platform: &'a str, session: &'a str) -> NewEntry<'a> {
        NewEntry {
            agent: Some("codex"), requested_model: "kimi/k2", source: Some("kimi"), upstream_model: Some("k2"),
            status_code: Some(200), result: "ok", duration_ms: Some(10), connection_id: Some(conn_id),
            session_platform: Some(platform), session_id: Some(session), client_key: None, request_id: None, request_uid: None, resolved_from: None, profile: None, decision: None }
    }

    /// 模型路由的活跃会话只监听和监测经过模型路由的会话，不包含未经路由的本机会话。
    #[test]
    fn local_only_sessions_without_gateway_routing_are_excluded() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let now = chrono::Local::now().timestamp_millis();
        // 只在本地 requests 表里、从未经过模型路由的会话（ZCode / Antigravity / Claude）
        conn.execute(
            "INSERT INTO requests(platform, source, dedup_key, session_id, ts, model)
             VALUES('zcode', 'native:zcode:main', 'u-1', 'native:zcode:main:sess_root', ?1, 'glm-4.7')",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO requests(platform, source, dedup_key, session_id, ts, model)
             VALUES('antigravity', 'native:antigravity:default', 'ag-1', 'native:antigravity:default:conv-1', ?1, 'gemini-4-neon-xhigh')",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO requests(platform, source, dedup_key, session_id, ts, model)
             VALUES('claude', 'claude', 'cl-1', 'local-only-claude', ?1, 'claude-sonnet-4-5')",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO sessions(platform, session_id, title, cwd)
             VALUES('claude', 'routed-claude', '经路由的会话', 'D:\\work\\Jolu')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO requests(platform, source, dedup_key, session_id, ts, model)
             VALUES('claude', 'claude', 'cl-2', 'routed-claude', ?1, 'claude-sonnet-4-5')",
            [now],
        ).unwrap();

        let before = overview(&conn).unwrap();
        assert!(
            before.sessions.is_empty(),
            "没有任何路由流水、在途请求或上下文快照时，本机会话不应混入模型路由活跃会话：{:?}",
            before.sessions.iter().map(|s| &s.session_id).collect::<Vec<_>>()
        );

        // 只有写入路由流水或上下文快照的会话才会进入活跃会话，且能补全本地标题与项目
        record(&conn, NewEntry { agent: Some("claude-code"), ..link("c1", "claude", "routed-claude") });
        crate::gateway::context::store::insert(
            &conn,
            &crate::gateway::context::store::NewSnapshot {
                session_platform: "codex",
                session_id: "snap-only-codex",
                request_id: 9,
                request_uid: Some(9),
                protocol: "responses",
                ts: now,
            },
        ).unwrap();

        let after = overview(&conn).unwrap();
        let ids: Vec<&str> = after.sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(ids.len(), 2, "只应返回 2 条经模型路由的会话，实际：{ids:?}");
        let claude = after.sessions.iter().find(|s| s.session_id == "routed-claude").unwrap();
        assert_eq!(claude.title.as_deref(), Some("经路由的会话"));
        assert_eq!(claude.cwd.as_deref(), Some("D:\\work\\Jolu"));
        assert_eq!(claude.agent.as_deref(), Some("claude-code"));
        let codex = after.sessions.iter().find(|s| s.session_id == "snap-only-codex").unwrap();
        assert_eq!(codex.agent.as_deref(), Some("codex"));
        assert_eq!(codex.gateway_last_ts, Some(now));
    }

    #[test]
    fn sessions_come_from_verified_links_and_unknown_tokens_stay_missing() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        record(&conn, link("c1", "codex", "gw-test-a"));
        record(&conn, link("c2", "codex", "gw-test-a"));
        record(&conn, link("c1", "claude", "gw-test-b"));
        record(&conn, NewEntry { session_platform: None, session_id: None, ..link("c1", "codex", "x") });
        let view = overview(&conn).unwrap();
        let mine: Vec<_> = view.sessions.iter().filter(|s| s.session_id.starts_with("gw-test-")).collect();
        assert_eq!(mine.len(), 2, "没有会话标识的流水不生成会话");
        let a = mine.iter().find(|s| s.session_id == "gw-test-a").unwrap();
        assert_eq!(a.links.len(), 2, "同一会话保留多个来源关联");
        assert_eq!(a.tokens.state, "missing");
        assert_eq!(a.tokens.total, None, "还没采集到的会话不能当零");
        assert_eq!(a.thread_status, None);
        assert_eq!(a.agent.as_deref(), Some("codex"));
    }

    #[test]
    fn token_total_requires_all_parts() {
        let mut item = crate::session_threads::SessionItem {
            platform: "codex".into(), session_id: "s".into(), title: None, cwd: None, first_ts: 0, last_ts: 0, requests: 1,
            fresh_input: Some(0), cache_read: Some(0), cache_write: Some(0), output: Some(0), connection_id: None,
            status: "recent".into(), has_source: false,
        };
        let known = tokens(Some(&item));
        assert_eq!((known.total, known.state), (Some(0), "known"), "完整数据合计为零就是 0");
        item.fresh_input = Some(1000);
        item.output = Some(234);
        assert_eq!(tokens(Some(&item)).total, Some(1234));
        item.cache_write = None;
        let partial = tokens(Some(&item));
        assert_eq!((partial.total, partial.state), (None, "partial"));
    }
}
