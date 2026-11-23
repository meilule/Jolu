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

/// 会话的平台名 → 接线注册表的 agent id。
///
/// 为什么需要这张表：`gateway_route_log` 的会话链接里带 `agent`，但**只在本地会话文件里**
/// 的会话没有链接——它的平台名来自 `requests.platform`（`claude` / `zcode` / `workbuddy`…），
/// 而前端要用 agent id 才能画图标、查接线状态。两个命名空间不完全一样（`claude` vs
/// `claude-code`），所以显式列出来而不是做字符串变换。
///
/// 只在平台确实对应一个**已接线**工具时给值；对不上的返回 None，界面显示「—」，
/// 不猜一个近似的 agent。
fn agent_of_platform(platform: &str) -> Option<&'static str> {
    Some(match platform {
        "claude" => "claude-code",
        "codex" => "codex",
        "gemini" => "gemini",
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

/// 「活跃会话」只看最近这段时间内有动静的本机会话。
///
/// 24 小时：够覆盖「今天在用」的范围，又不会把几个月前的历史会话全倒进这张表
/// （本机 ZCode 有 2 万多条历史请求，全列出来反而看不出现在在跑什么）。
const RECENT_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

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

    // 第三路：本机会话文件里活跃的会话（方案 C，2026-11-22）。
    //
    // 为什么需要：`session_identity` 只能从请求头取会话 ID，而实测 ZCode 的模型请求
    // 完全不带会话标识（`bootstrap/src/model-config.ts` 的 defaultHeaders 里没有任何
    // session 字段），WorkBuddy 同理。它们接线成功、流水也记了，但**建不了上下文快照**，
    // 于是活跃会话表里永远看不到。
    //
    // 这里的做法是并列而不是合并：网关观测到的会话（有快照、有来源关联）与本机会话
    // 文件读到的会话（有标题、有 cwd、有 Token 累计）各自是真实数据，只是维度不同。
    // 用时间或 token 去「猜」它们的对应关系会在并发请求上关联错，所以不猜——
    // 同一平台同一会话若两边都有，按 (platform, session_id) 自然合并成一行；
    // 只有本机会话文件的那些，靠下面的 `threads` 查询补上标题与 Token。
    let now_ms = chrono::Local::now().timestamp_millis();
    // 只取最近活跃的：把几个月没动过的历史会话全塞进「活跃会话」没有意义
    let recent_since = now_ms - RECENT_WINDOW_MS;
    let local = crate::session_threads::list(conn, &crate::session_threads::SessionFilter {
        since: Some(recent_since),
        ..Default::default()
    })?;
    for item in &local.sessions {
        let key = (item.platform.clone(), item.session_id.clone());
        if seen.insert(key.clone()) {
            order.push(key);
        }
    }

    let ids: Vec<String> = order.iter().map(|(_, id)| id.clone()).collect::<HashSet<_>>().into_iter().collect();
    // 第二次查询只为「已知的这批会话」补标题与 Token。它带着 `session_ids`，
    // 而 Kimi / Pi / dsh 的读取器要遍历磁盘（dsh 还要 zstd 解压，本机实测约 1 秒），
    // 且它们不按 `session_ids` 过滤——所以必须显式跳过，否则整个 overview 白跑两遍扫描。
    let filter = crate::session_threads::SessionFilter {
        session_ids: Some(ids),
        skip_external: true,
        ..Default::default()
    };
    let threads: HashMap<(String, String), crate::session_threads::SessionItem> = crate::session_threads::list(conn, &filter)?
        .sessions
        .into_iter()
        .map(|item| ((item.platform.clone(), item.session_id.clone()), item))
        .collect();

    let mut sessions: Vec<GatewaySession> = order
        .into_iter()
        .map(|key| {
            let links = by_key.remove(&key).unwrap_or_default();
            let thread = threads.get(&key);
            let live = live_agent.get(&key);
            // 优先级：在途请求记的 agent > 流水链接里的 agent > 平台名反查。
            // 最后一个分支是方案 C 加的：只在本地会话文件里的会话没有链接，
            // 若不反查，Agent 列会空着（截图上就是这个现象）。
            let agent = live
                .and_then(|(_, a)| a.clone())
                .or_else(|| links.first().and_then(|l| l.agent.clone()))
                .or_else(|| agent_of_platform(&key.0).map(str::to_string));
            GatewaySession {
                agent,
                title: thread.and_then(|t| t.title.clone()),
                cwd: thread.and_then(|t| t.cwd.clone()),
                thread_last_ts: thread.map(|t| t.last_ts),
                gateway_last_ts: links.iter().map(|l| l.last_ts).max(),
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
            session_platform: Some(platform), session_id: Some(session), client_key: None, request_id: None, request_uid: None, resolved_from: None, profile: None }
    }

    /// 方案 C：只在本地会话文件里、从未经过网关的会话也要出现在活跃会话里。
    ///
    /// 现场：ZCode 的模型请求不带任何会话标识（`bootstrap/src/model-config.ts` 的
    /// defaultHeaders 里没有 session 字段），所以它永远建不了上下文快照。可它是接了线的，
    /// 用户预期在活跃会话里能看到它 —— 本机会话文件里明明有记录。
    ///
    /// 这条锁三件事：本机会话会进来、Agent 从平台名反查得到、子会话被过滤掉。
    #[test]
    fn local_only_sessions_appear_with_their_agent() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let now = chrono::Local::now().timestamp_millis();
        // 一条只在 requests 表里、没有走网关（没有 route_log 链接）的 ZCode 会话
        conn.execute(
            "INSERT INTO requests(platform, source, dedup_key, session_id, ts, model)
             VALUES('zcode', 'native:zcode:main', 'u-1', 'native:zcode:main:sess_root', ?1, 'glm-4.7')",
            [now],
        ).unwrap();
        let view = overview(&conn).unwrap();
        let row = view.sessions.iter().find(|s| s.session_id == "native:zcode:main:sess_root");
        let row = row.expect("只在本地会话文件里的会话也要出现在活跃会话里");
        assert_eq!(
            row.agent.as_deref(),
            Some("zcode"),
            "Agent 列不能空着：平台名要反查成接线注册表的 agent id"
        );
        assert!(row.links.is_empty(), "它确实没有网关来源关联");
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
