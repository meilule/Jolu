//! 网关请求的实时事件（胶囊灵动岛设计说明 §9.1；统一网关开发说明 §6.1）。
//!
//! - `gateway-route-start`：选路得到 Direct / Subscription 之后、交给上游适配器之前发出。
//!   被拒（Reject）在选路时就判定了，不发开始事件。
//! - `gateway-route`：每个请求结束时发出，含被拒与失败；与开始事件带同一个 `request_id`。
//!
//! `request_id` 是进程内事件编号，历史流水也保存了它，但不能当跨重启的标识。
//! 事件只带元数据（时间、Agent、请求模型、来源 slug、连接 id、会话标识、上游模型、状态码、结果、耗时），
//! 不带提示词、请求体与凭证。
//!
//! 在途登记：开始事件登记、结束事件注销，二者共用一个递增序号 `seq`。
//! 「统一网关」页进入时读 [`live_snapshot`]，之后只应用序号更大的事件，快照不会覆盖更新的事件；
//! 重复的结束事件只是空操作，不会产生负计数。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// 发事件的回调：生产环境转给 Tauri `emit`，测试里收集到内存。
pub(crate) type Emit = Arc<dyn Fn(&'static str, serde_json::Value) + Send + Sync>;

pub(crate) const START_EVENT: &str = crate::events::GATEWAY_ROUTE_START;
pub(crate) const END_EVENT: &str = crate::events::GATEWAY_ROUTE;

pub(crate) fn next_request_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// JavaScript 安全整数上限：`request_uid` 会传给前端，超过它就不再分配，保留未知。
const UID_MAX: i64 = 9007199254740991;
/// 每次向数据库预留的号段长度。
const UID_BLOCK: i64 = 1000;
/// 当前号段剩余不足这么多时，提前在后台预留下一段。
const UID_LOW: i64 = 200;

/// 向数据库预留 `count` 个跨进程唯一的请求身份，返回半开区间 `[start, end)`。
///
/// 单行计数器由 SQLite 原子更新，多个进程 / 连接同时预留也不会重叠。
/// 计数器接近 [`UID_MAX`] 时返回错误（没有行被更新），调用方保留未知。
pub(crate) fn reserve_request_uids(conn: &rusqlite::Connection, count: i64) -> rusqlite::Result<(i64, i64)> {
    let last: i64 = conn.query_row(
        "UPDATE gateway_request_sequence SET value = value + ?1
         WHERE singleton = 1 AND value <= ?2 - ?1 RETURNING value",
        rusqlite::params![count, UID_MAX],
        |row| row.get(0),
    )?;
    Ok((last - count + 1, last + 1))
}

/// 进程内的请求身份号段（C-1：请求路径不等数据库锁）。
///
/// 原先每个请求在选路前都要拿一次数据库锁去递增计数器；采集器扫描时持锁较久，
/// 请求线程（tokio 工作线程）会被卡住。现在一次预留一段号码，请求只在内存里取号；
/// 剩余不足 [`UID_LOW`] 时把「预留下一段」交给后台写入线程。
///
/// 用不完的号码在进程退出时作废，只会留下空号，不影响唯一性。
#[derive(Clone, Default)]
pub(crate) struct UidBlock(Arc<Mutex<UidState>>);

#[derive(Default)]
struct UidState {
    /// 当前号段 `[next, end)`
    next: i64,
    end: i64,
    /// 后台预留好、还没用上的下一段
    spare: Option<(i64, i64)>,
    /// 已经排了一次后台预留，还没完成
    refilling: bool,
}

impl UidBlock {
    /// 取一个请求身份。
    ///
    /// 号段用尽且后台预留还没到时，只用 `try_lock` 试一次数据库：拿不到锁就返回 `None`
    /// （这一个请求身份未知，按旧行处理），不在请求线程上等锁。
    pub(crate) fn take(&self, db: &Arc<Mutex<rusqlite::Connection>>) -> Option<i64> {
        let mut state = self.0.lock().ok()?;
        if state.next >= state.end {
            if let Some((start, end)) = state.spare.take() {
                (state.next, state.end) = (start, end);
            } else {
                let reserved = match db.try_lock() {
                    Ok(conn) => reserve_request_uids(&conn, UID_BLOCK).map_err(|error| error.to_string()),
                    Err(_) => Err("数据库正忙".to_string()),
                };
                match reserved {
                    Ok((start, end)) => (state.next, state.end) = (start, end),
                    Err(error) => {
                        eprintln!("[路由流水] 暂时无法分配请求身份（{error}），本次记为未知");
                        self.schedule_refill(&mut state, db);
                        return None;
                    }
                }
            }
        }
        let uid = state.next;
        state.next += 1;
        if state.end - state.next < UID_LOW && state.spare.is_none() {
            self.schedule_refill(&mut state, db);
        }
        Some(uid)
    }

    /// 在后台预留一段号码，不等它完成。网关启动时调用一次，第一个请求就不必碰数据库。
    pub(crate) fn prefetch(&self, db: &Arc<Mutex<rusqlite::Connection>>) {
        if let Ok(mut state) = self.0.lock() {
            self.schedule_refill(&mut state, db);
        }
    }

    fn schedule_refill(&self, state: &mut UidState, db: &Arc<Mutex<rusqlite::Connection>>) {
        if state.refilling {
            return;
        }
        state.refilling = true;
        let (block, db) = (self.clone(), db.clone());
        crate::gateway::route_log::submit(move || {
            // 写入线程上可以等锁；先拿号段、放开数据库锁，再登记，避免两把锁交叉持有
            let reserved = db.lock().ok().and_then(|conn| reserve_request_uids(&conn, UID_BLOCK).ok());
            if let Ok(mut state) = block.0.lock() {
                state.refilling = false;
                if reserved.is_some() {
                    state.spare = reserved;
                }
            }
        });
    }
}

/// 已验证的调用方会话：平台（与会话线程的 platform 一致）+ 原始会话 ID。
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub(crate) struct SessionRef {
    pub platform: String,
    pub session_id: String,
}

/// 一次请求在开始与结束事件里共用的元数据。
#[derive(Debug, Clone, Default)]
pub(crate) struct RouteMeta {
    /// 数据库分配的持久请求身份；无法分配时为未知。
    pub request_uid: Option<i64>,
    pub agent: Option<String>,
    /// agent 识别依据（批 6）：token / originator / ua / codex_marker / vscode_agent_host
    pub identified_by: Option<&'static str>,
    /// 识别不出时的 UA 首段（截 64 字符），诊断用
    pub unidentified_ua: Option<String>,
    pub requested_model: String,
    /// 选中的连接 id；被拒时为 None
    pub connection_id: Option<String>,
    /// 只有能从请求里验证出会话身份时才有值；亲和兜底键不算
    pub session: Option<SessionRef>,
    /// 命中的命名客户端 Key 名（局域网共享时，鉴权中间件写入；只进路由流水，
    /// 供限额计数与来源归属；实时事件与在途快照不带它，避免把 Key 名广播给前端）
    pub client_key: Option<String>,
}

impl RouteMeta {
    pub(crate) fn session_platform(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.platform.as_str())
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.session_id.as_str())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RouteStart<'a> {
    pub request_id: u64,
    pub ts: i64,
    pub agent: Option<&'a str>,
    /// agent 识别依据：`token` / `ua` / `originator` / `codex_marker` / `client_key`。
    /// None = 识别不出——让「显示不对」可以定位是识别规则的问题还是工具本身的问题。
    pub identified_by: Option<&'a str>,
    /// 识别不出时的 UA 首段（截 64 字符）。
    /// 对齐 magpie `AgentOf` 最后一行 `return name`（不返回 "other"，返回 UA 第一段）。
    pub unidentified_ua: Option<&'a str>,
    /// 局域网共享的客户端 Key 名（agent 为 None 时界面显示「远程设备 · <Key名>」）
    pub client_key: Option<&'a str>,
    pub requested_model: &'a str,
    pub source: &'a str,
    pub connection_id: Option<&'a str>,
    pub session_platform: Option<&'a str>,
    pub session_id: Option<&'a str>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RouteEnd<'a> {
    pub request_id: u64,
    pub ts: i64,
    pub agent: Option<&'a str>,
    /// 同 [`RouteStart::identified_by`]
    pub identified_by: Option<&'a str>,
    pub unidentified_ua: Option<&'a str>,
    /// 局域网共享的客户端 Key 名
    pub client_key: Option<&'a str>,
    pub requested_model: &'a str,
    pub source: Option<&'a str>,
    pub upstream_model: Option<&'a str>,
    pub status_code: Option<u16>,
    /// ok / rejected / failed
    pub result: &'a str,
    pub duration_ms: i64,
    pub connection_id: Option<&'a str>,
    pub session_platform: Option<&'a str>,
    pub session_id: Option<&'a str>,
}

/// 在途请求（快照用，字段与开始事件一致）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveRequest {
    pub request_id: u64,
    pub ts: i64,
    pub agent: Option<String>,
    pub identified_by: Option<String>,
    pub unidentified_ua: Option<String>,
    pub client_key: Option<String>,
    pub requested_model: String,
    pub source: String,
    pub connection_id: Option<String>,
    pub session_platform: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Default)]
struct Live {
    seq: u64,
    requests: BTreeMap<u64, LiveRequest>,
}

static LIVE: LazyLock<Mutex<Live>> = LazyLock::new(Default::default);

fn live() -> std::sync::MutexGuard<'static, Live> {
    LIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 当前在途请求与对应的事件序号：序号不大于它的事件已经反映在快照里。
pub(crate) fn live_snapshot() -> (u64, Vec<LiveRequest>) {
    let guard = live();
    (guard.seq, guard.requests.values().cloned().collect())
}

fn with_seq<T: serde::Serialize>(event: &T, seq: u64) -> Option<serde_json::Value> {
    let mut value = serde_json::to_value(event).ok()?;
    value.as_object_mut()?.insert("seq".into(), seq.into());
    Some(value)
}

pub(crate) fn emit_start(emit: &Emit, event: &RouteStart<'_>) {
    let seq = {
        let mut guard = live();
        guard.seq += 1;
        guard.requests.insert(
            event.request_id,
            LiveRequest {
                request_id: event.request_id,
                ts: event.ts,
                agent: event.agent.map(str::to_string),
                identified_by: event.identified_by.map(str::to_string),
                unidentified_ua: event.unidentified_ua.map(str::to_string),
                client_key: event.client_key.map(str::to_string),
                requested_model: event.requested_model.to_string(),
                source: event.source.to_string(),
                connection_id: event.connection_id.map(str::to_string),
                session_platform: event.session_platform.map(str::to_string),
                session_id: event.session_id.map(str::to_string),
            },
        );
        guard.seq
    };
    if let Some(value) = with_seq(event, seq) {
        emit(START_EVENT, value);
    }
}

pub(crate) fn emit_end(emit: &Emit, event: &RouteEnd<'_>) {
    let seq = {
        let mut guard = live();
        guard.seq += 1;
        guard.requests.remove(&event.request_id);
        guard.seq
    };
    if let Some(value) = with_seq(event, seq) {
        emit(END_EVENT, value);
    }
}

/// 保证每个已发出的开始事件都有结束事件：请求被取消（客户端断开、future 被丢弃）时，
/// 在 Drop 里补发一条 `failed`。正常结束时调用 [`EndGuard::finish`] 交由调用方写结果。
pub(crate) struct EndGuard {
    emit: Emit,
    request_id: u64,
    meta: RouteMeta,
    source: String,
    started: std::time::Instant,
    done: bool,
}

impl EndGuard {
    pub(crate) fn new(emit: Emit, request_id: u64, meta: &RouteMeta, source: &str, started: std::time::Instant) -> Self {
        Self { emit, request_id, meta: meta.clone(), source: source.to_string(), started, done: false }
    }

    pub(crate) fn finish(mut self) {
        self.done = true;
    }
}

impl Drop for EndGuard {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        emit_end(
            &self.emit,
            &RouteEnd {
                request_id: self.request_id,
                ts: chrono::Local::now().timestamp_millis(),
                agent: self.meta.agent.as_deref(),
                identified_by: self.meta.identified_by,
                unidentified_ua: self.meta.unidentified_ua.as_deref(),
                client_key: self.meta.client_key.as_deref(),
                requested_model: &self.meta.requested_model,
                source: Some(&self.source),
                upstream_model: None,
                status_code: None,
                result: "failed",
                duration_ms: self.started.elapsed().as_millis() as i64,
                connection_id: self.meta.connection_id.as_deref(),
                session_platform: self.meta.session_platform(),
                session_id: self.meta.session_id(),
            },
        );
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// 收集发出的事件，供断言顺序与字段。
    pub(crate) fn collector() -> (Emit, Arc<Mutex<Vec<(&'static str, serde_json::Value)>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let emit: Emit = Arc::new(move |name, value| sink.lock().unwrap().push((name, value)));
        (emit, events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> RouteMeta {
        RouteMeta {
            request_uid: None,
            agent: Some("codex".into()),
            identified_by: Some("token"),
            unidentified_ua: None,
            requested_model: "kimi/k2".into(),
            connection_id: Some("conn-1".into()),
            session: Some(SessionRef { platform: "codex".into(), session_id: "ses-1".into() }),
            client_key: None,
        }
    }

    fn start(emit: &Emit, id: u64) {
        emit_start(emit, &RouteStart {
            request_id: id, ts: 1, agent: None, identified_by: None, unidentified_ua: None,
            client_key: None, requested_model: "a/b", source: "a",
            connection_id: Some("conn-1"), session_platform: Some("codex"), session_id: Some("ses-1"),
        });
    }

    fn end(emit: &Emit, id: u64) {
        emit_end(emit, &RouteEnd {
            request_id: id, ts: 2, agent: None, identified_by: None, unidentified_ua: None,
            client_key: None, requested_model: "a/b", source: Some("a"), upstream_model: None,
            status_code: Some(200), result: "ok", duration_ms: 1, connection_id: Some("conn-1"),
            session_platform: Some("codex"), session_id: Some("ses-1"),
        });
    }

    fn is_live(id: u64) -> bool {
        live_snapshot().1.iter().any(|r| r.request_id == id)
    }

    #[test]
    fn ids_are_unique_and_increasing() {
        let a = next_request_id();
        let b = next_request_id();
        assert!(b > a);
    }

    #[test]
    fn persisted_ids_survive_reopen_and_concurrent_connections() {
        let path = std::env::temp_dir().join(format!(
            "jolu-request-uid-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let conn = crate::db::open(&path).unwrap();
        assert_eq!(reserve_request_uids(&conn, 1).unwrap(), (1, 2));
        drop(conn);

        // 两条连接各预留 50 段、每段 3 个：号段两两不重叠，合起来正好连续
        let left = crate::db::open(&path).unwrap();
        let right = crate::db::open(&path).unwrap();
        let threads = [left, right].map(|conn| std::thread::spawn(move || {
            (0..50).flat_map(|_| {
                let (start, end) = reserve_request_uids(&conn, 3).unwrap();
                start..end
            }).collect::<Vec<_>>()
        }));
        let mut ids: Vec<_> = threads.into_iter().flat_map(|thread| thread.join().unwrap()).collect();
        ids.sort_unstable();
        assert_eq!(ids, (2..=301).collect::<Vec<_>>());

        // 接近安全整数上限：剩余不够一整段就不分配
        let conn = crate::db::open(&path).unwrap();
        conn.execute("UPDATE gateway_request_sequence SET value = ?1 WHERE singleton = 1", [UID_MAX - 2]).unwrap();
        assert!(reserve_request_uids(&conn, 3).is_err(), "不能越过 JavaScript 安全整数");
        assert_eq!(reserve_request_uids(&conn, 2).unwrap(), (UID_MAX - 1, UID_MAX + 1));
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    /// 号段：请求在内存里取号，号段耗尽前在后台预留下一段；跨号段仍然严格递增、不重复。
    #[test]
    fn uid_block_hands_out_unique_ids_across_refills() {
        let db = Arc::new(Mutex::new(crate::db::open(std::path::Path::new(":memory:")).unwrap()));
        let block = UidBlock::default();
        let ids: Vec<i64> = (0..(UID_BLOCK * 3)).map(|_| {
            let uid = block.take(&db).expect("库空闲时总能取到号");
            // 让后台预留有机会完成，模拟请求之间的间隔
            crate::gateway::route_log::flush();
            uid
        }).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "号码不能重复");
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "同一进程内单调递增");
    }

    /// 号段用尽、数据库又被占着时，请求不等锁：这一次身份记为未知，锁放开后恢复。
    #[test]
    fn uid_block_does_not_wait_for_a_busy_database() {
        let db = Arc::new(Mutex::new(crate::db::open(std::path::Path::new(":memory:")).unwrap()));
        let block = UidBlock::default();
        let held = db.lock().unwrap();
        let started = std::time::Instant::now();
        assert_eq!(block.take(&db), None, "库忙时不阻塞，本次记未知");
        assert!(started.elapsed() < std::time::Duration::from_millis(200), "取号不能等锁");
        drop(held);
        crate::gateway::route_log::flush();
        assert!(block.take(&db).is_some(), "锁放开后后台预留补上号段");
    }

    #[test]
    fn dropped_guard_emits_failed_end_with_same_id() {
        let (emit, events) = test_support::collector();
        let id = next_request_id();
        start(&emit, id);
        {
            let _guard = EndGuard::new(emit.clone(), id, &meta(), "kimi", std::time::Instant::now());
        }
        assert!(!is_live(id), "取消的请求要从在途登记里移除");
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].0, END_EVENT);
        assert_eq!(events[1].1["request_id"], id);
        assert_eq!(events[1].1["result"], "failed");
        assert_eq!(events[1].1["source"], "kimi");
        assert_eq!(events[1].1["session_id"], "ses-1");
        assert!(events[1].1["seq"].as_u64().unwrap() > events[0].1["seq"].as_u64().unwrap());
    }

    #[test]
    fn finished_guard_emits_nothing() {
        let (emit, events) = test_support::collector();
        EndGuard::new(emit, next_request_id(), &meta(), "s", std::time::Instant::now()).finish();
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn snapshot_tracks_concurrent_requests_and_ignores_duplicate_end() {
        let (emit, _events) = test_support::collector();
        let (a, b) = (next_request_id(), next_request_id());
        start(&emit, a);
        start(&emit, b);
        assert!(is_live(a) && is_live(b));
        end(&emit, a);
        assert!(!is_live(a) && is_live(b), "同一会话结束一个请求，另一个仍在途");
        end(&emit, a);
        assert!(is_live(b), "重复的结束事件不影响其他请求");
        let (seq_before, _) = live_snapshot();
        end(&emit, b);
        let (seq_after, _) = live_snapshot();
        assert!(seq_after > seq_before);
        assert!(!is_live(b));
    }

    #[test]
    fn end_without_start_is_harmless() {
        let (emit, events) = test_support::collector();
        let id = next_request_id();
        end(&emit, id);
        assert!(!is_live(id));
        assert_eq!(events.lock().unwrap().len(), 1);
    }

    #[test]
    fn payload_has_no_secret_fields() {
        let value = serde_json::to_value(RouteStart {
            request_id: 1, ts: 0, agent: None, identified_by: None, unidentified_ua: None,
            client_key: None, requested_model: "a/b", source: "a",
            connection_id: None, session_platform: None, session_id: None,
        }).unwrap();
        let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["agent", "client_key", "connection_id", "identified_by", "request_id", "requested_model", "session_id", "session_platform", "source", "ts", "unidentified_ua"]);
    }
}
