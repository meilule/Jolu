//! 路由流水：每次网关请求一行，给「统一网关」页展示最近去向。
//!
//! 只记元数据：时间、agent、请求模型、路由到的来源与连接、已验证的调用方会话、状态码、等待上游响应的耗时。
//! 不记提示词、不记凭证。流式响应的耗时是「到响应头」的时间，不是首字时间，界面按「耗时」展示。
//!
//! 只保留最近约 1000 行（分批清理，可能暂时超过），分页统计只代表当前保留范围。

use rusqlite::{params, Connection};

/// 保留行数；清理每 50 次写执行一次。
const KEEP_ROWS: i64 = 1000;

#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteLogEntry {
    /// 自增流水 id：分页的稳定行标识与唯一次级排序键
    pub id: i64,
    pub ts: i64,
    pub agent: Option<String>,
    pub requested_model: String,
    pub source: Option<String>,
    pub upstream_model: Option<String>,
    pub status_code: Option<i64>,
    /// ok / rejected / failed / cancelled
    ///
    /// `cancelled` 是 P1 新增的一档：客户端在流式响应中途断开，请求没有完成。
    /// 它既不是成功（用量不完整）也不是失败（上游正常开始回了）。
    /// 上游在流中途报错时，原先的 `ok` 改判为 `failed`。见 [`CancelWatch`]。
    pub result: String,
    pub duration_ms: Option<i64>,
    pub connection_id: Option<String>,
    pub session_platform: Option<String>,
    pub session_id: Option<String>,
    /// 命中的命名客户端 Key 名（局域网共享，P8.3）；旧行与本机工具的请求为 None
    pub client_key: Option<String>,
    /// 同一次网关请求的编号：分组成员失败转移时，各次尝试的行共享它（进程内递增，重启后从头计）
    pub request_id: Option<i64>,
    /// 跨进程可验证的请求身份；旧流水与分配失败时为空。
    pub request_uid: Option<i64>,
    /// 模型 id 怎么解析到来源的：`bare` = 请求用的是不带来源前缀的裸名（目录或默认来源兜底）
    pub resolved_from: Option<String>,
    /// 请求发生时生效的 Profile（应用 Profile 后、手动改接线前）
    pub profile: Option<String>,
    /// 路由决策的一句话解释（P2-4）：分组策略、选中的成员、降级原因。
    ///
    /// 只在该请求确实经过分组 / 有需要说明的降级时才有值；普通单来源请求为 None。
    /// 与 `resolved_from` 互补：后者说「模型名怎么解析到来源」，这一列说
    /// 「为什么是这个成员」。两者合起来才是完整的可解释路由。
    pub decision: Option<String>,
}

pub struct NewEntry<'a> {
    pub agent: Option<&'a str>,
    pub requested_model: &'a str,
    pub source: Option<&'a str>,
    pub upstream_model: Option<&'a str>,
    pub status_code: Option<i64>,
    pub result: &'a str,
    pub duration_ms: Option<i64>,
    pub connection_id: Option<&'a str>,
    pub session_platform: Option<&'a str>,
    pub session_id: Option<&'a str>,
    /// 命中的命名客户端 Key 名（局域网共享时由鉴权中间件识别）；None 表示本机调用方
    pub client_key: Option<&'a str>,
    /// 同一次网关请求的编号（见 [`RouteLogEntry::request_id`]）
    pub request_id: Option<i64>,
    pub request_uid: Option<i64>,
    /// 见 [`RouteLogEntry::resolved_from`]
    pub resolved_from: Option<&'a str>,
    /// 见 [`RouteLogEntry::profile`]
    pub profile: Option<&'a str>,
    /// 见 [`RouteLogEntry::decision`]
    pub decision: Option<&'a str>,
}

/// 写入一条流水（时间取现在），返回新行 id（写入失败为 None）。
///
/// 返回 id 是为了「先记成功、后发现流没正常读完」的场景：流式响应的状态码在响应头到手时
/// 就已经确定，但客户端可能中途断开、上游也可能中途报错。那时这行已经写成 `ok`，
/// 只能回过头改它——没有 id 就只能靠 (ts, request_uid) 猜，猜错会改到别人的行。
pub fn record(conn: &Connection, entry: NewEntry<'_>) -> Option<i64> {
    record_at(conn, chrono::Local::now().timestamp_millis(), entry)
}

/// 同 [`record`]，时间由调用方给定。网关经 [`submit`] 排队写入，时间必须在请求结束时
/// 取好——排队期间数据库可能正被采集任务占着，到写入时再取会把时间推后。
pub fn record_at(conn: &Connection, ts: i64, entry: NewEntry<'_>) -> Option<i64> {
    let result = conn.execute(
        "INSERT INTO gateway_route_log(ts, agent, requested_model, source, upstream_model, status_code, result, duration_ms,
                                       connection_id, session_platform, session_id, client_key, request_id, resolved_from, profile, request_uid, decision)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            ts,
            entry.agent,
            entry.requested_model,
            entry.source,
            entry.upstream_model,
            entry.status_code,
            entry.result,
            entry.duration_ms,
            entry.connection_id,
            entry.session_platform,
            entry.session_id,
            entry.client_key,
            entry.request_id,
            entry.resolved_from,
            entry.profile,
            entry.request_uid,
            entry.decision,
        ],
    );
    let row_id = match result {
        Ok(_) => Some(conn.last_insert_rowid()),
        Err(error) => {
            eprintln!("[路由流水] 记录写入失败：{error}");
            return None;
        }
    };
    // 只保留最近 1000 行；每 50 次写清理一次，避免每条请求都做一次 NOT IN 子查询。
    use std::sync::atomic::{AtomicU32, Ordering};
    static WRITES: AtomicU32 = AtomicU32::new(0);
    if WRITES.fetch_add(1, Ordering::Relaxed) % 50 == 49 {
        let _ = conn.execute(
            "DELETE FROM gateway_route_log WHERE id NOT IN (SELECT id FROM gateway_route_log ORDER BY id DESC LIMIT ?1)",
            [KEEP_ROWS],
        );
        // 上下文快照与流水在同一处清理（执行文档 §6）
        crate::gateway::context::store::prune(conn);
    }
    row_id
}

type Db = std::sync::Arc<std::sync::Mutex<Connection>>;
type Job = Box<dyn FnOnce() + Send>;

/// 把一次流水写入交给后台写入线程，立即返回。
///
/// ## 为什么不在请求里直接写
///
/// 网关和采集器共用同一把数据库锁（`ProxyDeps.db`），采集器扫描时会持锁较久。
/// 请求处理跑在 tokio 工作线程上，在那里用 `std::sync::Mutex::lock()` 等锁会把工作线程
/// 卡住，流式请求的首字也跟着推迟。所以请求路径只排队，由一个专门的线程等锁、写库。
///
/// ## 顺序
///
/// 只有一个写入线程，按提交顺序执行。「写一行 → 稍后改判这一行」因此天然有序，
/// 改判任务执行时，前面那次写入已经完成，[`RowSlot`] 里已有行 id。
///
/// 线程起不来（极少见）时退回在当前线程执行，流水不会因此丢。
/// 进程退出时队列里还没写完的几条会丢，这与原先「锁忙就跳过」的风险相当。
pub(crate) fn submit(job: impl FnOnce() + Send + 'static) {
    use std::sync::mpsc::{channel, Sender};
    static WRITER: std::sync::OnceLock<Option<Sender<Job>>> = std::sync::OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("jolu-route-log".into())
            .spawn(move || {
                for job in rx {
                    // 一个任务出错不能让写入线程退出，否则后面的流水全丢
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                }
            })
            .map_err(|error| eprintln!("[路由流水] 写入线程启动失败，改为同步写入：{error}"))
            .ok()
            .map(|_| tx)
    });
    let job: Job = Box::new(job);
    match writer {
        Some(tx) => {
            if let Err(std::sync::mpsc::SendError(job)) = tx.send(job) {
                job();
            }
        }
        None => job(),
    }
}

/// 等写入队列里此前提交的任务全部执行完。测试在读流水之前调用。
///
/// 调用方不能持有任何排队任务要用的数据库锁，否则会互相等待。
#[cfg(test)]
pub(crate) fn flush() {
    let (tx, rx) = std::sync::mpsc::channel();
    submit(move || {
        let _ = tx.send(());
    });
    let _ = rx.recv();
}

/// 一条排队写入的流水写好后的行 id。写入失败、或还没轮到写时为空。
#[derive(Clone, Default)]
pub(crate) struct RowSlot(std::sync::Arc<std::sync::OnceLock<i64>>);

impl RowSlot {
    pub(crate) fn get(&self) -> Option<i64> {
        self.0.get().copied()
    }

    fn set(&self, id: i64) {
        let _ = self.0.set(id);
    }
}

/// 排队写一条流水（时间在调用时取好）。写好后把行 id 填进返回的 [`RowSlot`]。
pub(crate) fn queue_record(db: &Db, ts: i64, entry: OwnedEntry) -> RowSlot {
    let slot = RowSlot::default();
    let (db, filled) = (db.clone(), slot.clone());
    submit(move || {
        let conn = match db.lock() {
            Ok(conn) => conn,
            Err(error) => {
                eprintln!("[路由流水] 数据库锁不可用，记录写入失败：{error}");
                return;
            }
        };
        if let Some(id) = record_at(&conn, ts, entry.borrowed()) {
            filled.set(id);
        }
    });
    slot
}

/// [`NewEntry`] 的自有版本：排队写入时，请求里借来的字符串活不到写入那一刻。
pub(crate) struct OwnedEntry {
    pub agent: Option<String>,
    pub requested_model: String,
    pub source: Option<String>,
    pub upstream_model: Option<String>,
    pub status_code: Option<i64>,
    pub result: String,
    pub duration_ms: Option<i64>,
    pub connection_id: Option<String>,
    pub session_platform: Option<String>,
    pub session_id: Option<String>,
    pub client_key: Option<String>,
    pub request_id: Option<i64>,
    pub request_uid: Option<i64>,
    pub resolved_from: Option<&'static str>,
    pub profile: Option<String>,
    pub decision: Option<String>,
}

impl OwnedEntry {
    fn borrowed(&self) -> NewEntry<'_> {
        NewEntry {
            agent: self.agent.as_deref(),
            requested_model: &self.requested_model,
            source: self.source.as_deref(),
            upstream_model: self.upstream_model.as_deref(),
            status_code: self.status_code,
            result: &self.result,
            duration_ms: self.duration_ms,
            connection_id: self.connection_id.as_deref(),
            session_platform: self.session_platform.as_deref(),
            session_id: self.session_id.as_deref(),
            client_key: self.client_key.as_deref(),
            request_id: self.request_id,
            request_uid: self.request_uid,
            resolved_from: self.resolved_from,
            profile: self.profile.as_deref(),
            decision: self.decision.as_deref(),
        }
    }
}

/// 流式响应没有正常读完时的两种结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    /// 客户端中途断开（Ctrl-C、关终端、读超时）
    Cancelled,
    /// 上游在流中途报错
    Failed,
}

impl StreamEnd {
    fn result(self) -> &'static str {
        match self {
            StreamEnd::Cancelled => "cancelled",
            StreamEnd::Failed => "failed",
        }
    }
}

/// 流式响应的结束追踪器（P1：没读完的流不能记成 `ok`）。
///
/// ## 为什么要单独追踪
///
/// 流式请求的流水在**响应头到手**时就写下了，那时 `result` 是 `ok`——状态码 200，
/// 上游确实开始回了。之后流可能没有正常结束：客户端（Claude Code / Codex / 用户在终端
/// 按 Ctrl-C）读到一半断开，或者上游中途报错。只看状态码会把这些都记成完全成功，
/// 用量统计和「今天成功几次」都会偏高。
///
/// ## 怎么判断
///
/// | 情形 | `poll_next` 返回 | 判定 |
/// |---|---|---|
/// | 上游正常写完 | `Ready(None)` | 完成，不改 |
/// | 上游中途报错 | `Ready(Some(Err))` | **失败**（随后 body 被丢弃，不再记成取消） |
/// | 客户端断开 / 读超时 | 没等到 `None` 流就被 drop | **取消** |
///
/// 上游报错后 hyper 会丢弃 body，`Drop` 照样执行。所以收到 `Err` 时必须先把结局定为失败、
/// 收走回调，否则 `Drop` 会把上游的错误说成客户端断开。
///
/// ## 为什么改原行而不是新写一行
///
/// 同一请求已经在流水里有一行 `ok`，再写一行会让「一次请求」变成两行——
/// 正是 P1 反复要消除的「把尝试当请求」。改原行则保持一行一请求，`request_uid`
/// 关联不变。行 id 经 [`RowSlot`] 传过来。
pub(crate) struct CancelWatch {
    inner: axum::body::BodyDataStream,
    /// 流没有正常读完时执行，参数说明是哪种结局。`None` 表示已经自然结束（或已经执行过）。
    on_unfinished: Option<Box<dyn FnOnce(StreamEnd) + Send>>,
}

impl CancelWatch {
    /// 包一层响应 body：流没有正常读完时执行 `on_unfinished`。
    ///
    /// 自然读完不执行。用闭包而不是直接传数据库，是为了让判定逻辑（读完了吗？怎么没读完？）
    /// 可以脱离数据库单独测。
    pub(crate) fn wrap(
        response: axum::response::Response,
        on_unfinished: impl FnOnce(StreamEnd) + Send + 'static,
    ) -> axum::response::Response {
        let (parts, body) = response.into_parts();
        let stream = Self { inner: body.into_data_stream(), on_unfinished: Some(Box::new(on_unfinished)) };
        axum::response::Response::from_parts(parts, axum::body::Body::from_stream(stream))
    }

    /// 生产用：流没有正常读完时，把这条流水改判为 cancelled / failed。
    ///
    /// 改判同样排队交给写入线程，`Drop` 里不等数据库锁。它排在这一行的写入之后，
    /// 执行时 `row` 里已有行 id；流水没写进库时 `row` 为空，什么都不做。
    pub(crate) fn watch(response: axum::response::Response, db: &Db, row: RowSlot) -> axum::response::Response {
        let db = db.clone();
        Self::wrap(response, move |end| {
            submit(move || {
                let Some(id) = row.get() else { return };
                let Ok(conn) = db.lock() else { return };
                match mark_unfinished(&conn, id, end) {
                    Ok(true) => eprintln!("[路由流水] 流式响应没有读完，第 {id} 行改判为 {}", end.result()),
                    Ok(false) => {}
                    Err(error) => eprintln!("[路由流水] 改判失败：{error}"),
                }
            })
        })
    }

    fn finish(&mut self, end: StreamEnd) {
        if let Some(callback) = self.on_unfinished.take() {
            callback(end);
        }
    }
}

impl futures_util::Stream for CancelWatch {
    type Item = Result<axum::body::Bytes, axum::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let polled = std::pin::Pin::new(&mut self.inner).poll_next(cx);
        match &polled {
            // 自然读完：不改判，放弃回调
            std::task::Poll::Ready(None) => self.on_unfinished = None,
            // 上游中途报错：定为失败并收走回调，随后的 Drop 不再记成取消
            std::task::Poll::Ready(Some(Err(_))) => self.finish(StreamEnd::Failed),
            _ => {}
        }
        polled
    }
}

impl Drop for CancelWatch {
    fn drop(&mut self) {
        self.finish(StreamEnd::Cancelled);
    }
}

/// 把一行流水改判为流没读完的结局（P1：客户端中途断开 → cancelled；上游中途报错 → failed）。
///
/// **只改 `ok` 行**。已经写成 `failed` / `rejected` 的行不动——那些是上游或网关
/// 在响应头阶段给出的结论，比「流没读完」更早也更具体，覆盖它只会丢信息。
///
/// 返回是否真的改了一行：`false` 说明该行已被清理、或本来就不是 `ok`。
pub fn mark_unfinished(conn: &Connection, id: i64, end: StreamEnd) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE gateway_route_log SET result = ?2 WHERE id = ?1 AND result = 'ok'",
        params![id, end.result()],
    )?;
    Ok(changed > 0)
}

const COLUMNS: &str = "id, ts, agent, requested_model, source, upstream_model, status_code, result, duration_ms,
                       connection_id, session_platform, session_id, client_key, request_id, resolved_from, profile, request_uid, decision";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RouteLogEntry> {
    Ok(RouteLogEntry {
        id: r.get(0)?,
        ts: r.get(1)?,
        agent: r.get(2)?,
        requested_model: r.get(3)?,
        source: r.get(4)?,
        upstream_model: r.get(5)?,
        status_code: r.get(6)?,
        result: r.get(7)?,
        duration_ms: r.get(8)?,
        connection_id: r.get(9)?,
        session_platform: r.get(10)?,
        session_id: r.get(11)?,
        client_key: r.get(12)?,
        request_id: r.get(13)?,
        resolved_from: r.get(14)?,
        profile: r.get(15)?,
        request_uid: r.get(16)?,
        decision: r.get(17)?,
    })
}

pub fn recent(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<RouteLogEntry>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM gateway_route_log ORDER BY id DESC LIMIT ?1"))?;
    let rows = stmt.query_map([limit as i64], row)?;
    rows.collect()
}

/// 一页流水与这次浏览的查询范围。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteLogPage {
    pub rows: Vec<RouteLogEntry>,
    /// 实际返回的页码（请求页超出范围时收到最后一页；没有数据时为 1）
    pub page: i64,
    pub page_size: i64,
    /// 查询上界内、当前保留范围内的总行数
    pub total: i64,
    /// 没有数据时为 0
    pub page_count: i64,
    /// 本次浏览的查询上界（含）：翻页时带回，新写入的流水不会挤动旧页；没有数据时为 None
    pub upper_id: Option<i64>,
}

/// 分页：按 id 从大到小。`upper_id` 为 None 时以当前最大 id 建立新的查询范围。
/// 计数与列表都限定在同一上界内；保留范围清理掉的旧行会让总数变小，页码随之收拢到最后一页。
pub fn page(conn: &Connection, page: i64, page_size: i64, upper_id: Option<i64>) -> rusqlite::Result<RouteLogPage> {
    let page_size = page_size.clamp(1, 100);
    let upper = match upper_id {
        Some(id) => Some(id),
        None => conn.query_row("SELECT MAX(id) FROM gateway_route_log", [], |r| r.get::<_, Option<i64>>(0))?,
    };
    let Some(upper) = upper else {
        return Ok(RouteLogPage { rows: Vec::new(), page: 1, page_size, total: 0, page_count: 0, upper_id: None });
    };
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM gateway_route_log WHERE id <= ?1", [upper], |r| r.get(0))?;
    let page_count = (total + page_size - 1) / page_size;
    let page = page.clamp(1, page_count.max(1));
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM gateway_route_log WHERE id <= ?1 ORDER BY id DESC LIMIT ?2 OFFSET ?3"
    ))?;
    let rows = stmt
        .query_map(params![upper, page_size, (page - 1) * page_size], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(RouteLogPage { rows, page, page_size, total, page_count, upper_id: Some(upper) })
}

/// 某个会话经网关到某个连接的历史关联（保留范围内）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionLink {
    pub session_platform: String,
    pub session_id: String,
    /// 被拒或旧流水没有连接 id 时为 None
    pub connection_id: Option<String>,
    pub source: Option<String>,
    /// 最近一次的请求模型与上游模型
    pub requested_model: String,
    pub upstream_model: Option<String>,
    pub agent: Option<String>,
    pub last_ts: i64,
    pub requests: i64,
}

/// 按（会话, 连接）聚合流水里的会话关联，最新的在前。只统计带已验证会话标识的行。
/// 模型、来源与 Agent 取每组 id 最大（最近一次）的那一行。
pub fn session_links(conn: &Connection) -> rusqlite::Result<Vec<SessionLink>> {
    let mut stmt = conn.prepare(
        "SELECT l.session_platform, l.session_id, l.connection_id, l.source, l.requested_model, l.upstream_model, l.agent,
                g.last_id, g.last_ts, g.requests
         FROM (SELECT MAX(id) AS last_id, MAX(ts) AS last_ts, COUNT(*) AS requests
               FROM gateway_route_log
               WHERE session_platform IS NOT NULL AND session_id IS NOT NULL AND session_id <> ''
               GROUP BY session_platform, session_id, connection_id) g
         JOIN gateway_route_log l ON l.id = g.last_id
         ORDER BY g.last_id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SessionLink {
            session_platform: r.get(0)?,
            session_id: r.get(1)?,
            connection_id: r.get(2)?,
            source: r.get(3)?,
            requested_model: r.get(4)?,
            upstream_model: r.get(5)?,
            agent: r.get(6)?,
            last_ts: r.get(8)?,
            requests: r.get(9)?,
        })
    })?;
    rows.collect()
}

/// 某个来源（连接）经网关的使用情况（保留范围内）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceUsage {
    pub connection_id: String,
    pub last_ts: i64,
    pub requests: i64,
    /// 其中失败的次数（被拒的请求没有连接 id，不计入）
    pub failed: i64,
}

/// 按连接汇总保留范围内的流水；旧流水没有连接 id 的不计入。
pub fn source_usage(conn: &Connection) -> rusqlite::Result<Vec<SourceUsage>> {
    let mut stmt = conn.prepare(
        "SELECT connection_id, MAX(ts), COUNT(*), COALESCE(SUM(result NOT IN ('ok', 'rejected')), 0)
         FROM gateway_route_log WHERE connection_id IS NOT NULL
         GROUP BY connection_id ORDER BY MAX(ts) DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SourceUsage { connection_id: r.get(0)?, last_ts: r.get(1)?, requests: r.get(2)?, failed: r.get(3)? })
    })?;
    rows.collect()
}

/// 模型路由概览（画板 V5-3「四项指标」、V10-01 范围下拉、Agent 页「今日路由 N」）。
///
/// 一次网关请求在分组改道时会写多行（共享 `request_id`），这里按请求而不是按行计：
/// - `today` / `yesterday`：请求数；
/// - `reroutes`：本期写了不止一行的请求（失败后换下一个成员）；
/// - `errors`：本期最后一次尝试仍不是 ok 的请求（Agent 真正收到的错误）；
/// - `by_agent`：本期各 agent 的请求数（agent 未识别的不计）。
///
/// **字段名沿用 `today` / `yesterday`，含义是「本期 / 上期」**（概览页的范围下拉）：
/// 今天 → 本期今天 / 上期昨天；昨天 → 本期昨天 / 上期前天；近 7 天 → 本期 7 天 / 上期前 7 天；
/// 近 30 天同理。前端文案按范围换称呼即可，计算不分叉。
///
/// 流水受保留数量限制，旧记录缺少持久请求身份时无法准确去重。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DayOverview {
    pub today: i64,
    pub yesterday: i64,
    pub reroutes: i64,
    pub errors: i64,
    pub by_agent: std::collections::BTreeMap<String, i64>,
    /// 保留范围可能未覆盖完整的本期或上期。
    pub partial: bool,
    /// 相关时间范围内有无法可靠归并到请求的流水。
    pub identity_incomplete: bool,
    /// 本期最近一次 Agent 收到的错误（最后一次尝试那行）
    pub last_error: Option<RouteLogEntry>,
}

/// 概览页的时间范围（V10-01 右上「今天」下拉）。
/// 每个范围给出「本期起点 + 上期起点」两个时间戳：上期用于「比上期」的对比与 `partial` 判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Today,
    Yesterday,
    Last7Days,
    Last30Days,
}

impl Range {
    /// 从命令参数解析；未知值退回「今天」（不让前端一个错字把页面变空）。
    pub fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or("today").trim().to_ascii_lowercase().as_str() {
            "yesterday" => Self::Yesterday,
            "7d" | "last7" | "last7d" => Self::Last7Days,
            "30d" | "last30" | "last30d" => Self::Last30Days,
            _ => Self::Today,
        }
    }

    /// 覆盖的天数（本期）。用于算上期起点。
    fn days(self) -> i64 {
        match self {
            Self::Today | Self::Yesterday => 1,
            Self::Last7Days => 7,
            Self::Last30Days => 30,
        }
    }

    /// 本期是否截止到今天；`false` 表示本期是「昨天 / 昨天往前 N 天」这类已结束的区间。
    fn ends_today(self) -> bool {
        matches!(self, Self::Today | Self::Last7Days | Self::Last30Days)
    }

    /// 本期起点与上期起点（本地时间自然日边界，毫秒）。
    /// `today` 由调用方传入，便于测试固定日期。
    pub fn bounds(self, today: chrono::NaiveDate) -> (i64, i64) {
        let days = self.days();
        // 本期覆盖的最后一个自然日
        let end_day = if self.ends_today() { today } else { today.pred_opt().unwrap_or(today) };
        let current_start = end_day - chrono::Duration::days(days - 1);
        let prior_start = current_start - chrono::Duration::days(days);
        (midnight_ms(current_start), midnight_ms(prior_start))
    }

    /// 本期结束边界，使用本地自然日的下一次零点。
    pub fn end_exclusive(self, today: chrono::NaiveDate) -> i64 {
        let end = if self.ends_today() { today.succ_opt().unwrap_or(today) } else { today };
        midnight_ms(end)
    }
}

/// 本地时区某天 00:00 的毫秒时间戳；时区异常时退回 0（与既有实现一致）。
fn midnight_ms(day: chrono::NaiveDate) -> i64 {
    day.and_hms_opt(0, 0, 0)
        .and_then(|midnight| midnight.and_local_timezone(chrono::Local).earliest())
        .map(|midnight| midnight.timestamp_millis())
        .unwrap_or(0)
}

pub fn day_overview(conn: &Connection, today_start: i64, yesterday_start: i64, end_exclusive: i64) -> rusqlite::Result<DayOverview> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM gateway_route_log ORDER BY id ASC"))?;
    let rows = stmt.query_map([], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    let oldest: Option<i64> = conn.query_row("SELECT MIN(ts) FROM gateway_route_log", [], |r| r.get(0))?;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM gateway_route_log", [], |r| r.get(0))?;
    // 仅有持久身份的行可以确认属于同一次请求。旧行独立保留，不猜测关联。
    let mut groups: Vec<(i64, Vec<RouteLogEntry>)> = Vec::new();
    let mut index: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for entry in rows {
        if let Some(at) = entry.request_uid.and_then(|uid| index.get(&uid).copied()) {
            groups[at].0 = groups[at].0.min(entry.ts);
            groups[at].1.push(entry);
        } else {
            if let Some(uid) = entry.request_uid {
                index.insert(uid, groups.len());
            }
            groups.push((entry.ts, vec![entry]));
        }
    }
    let mut out = DayOverview {
        // 流水按 `KEEP_ROWS` 滚动删除，所以「行数正好达到上限」就说明更早的行可能已被删掉。
        // 用 `>=` 而不是 `>`：等于上限时无法排除刚删过一批，宁可标不完整也不要给假完整。
        // 第二个条件是「现存最老一行比上期起点还新」——那说明上期根本没数据留下来。
        partial: total >= KEEP_ROWS && oldest.is_some_and(|ts| ts > yesterday_start),
        ..Default::default()
    };
    for (first_ts, attempts) in groups {
        if first_ts < yesterday_start || first_ts >= end_exclusive {
            continue;
        }
        if attempts[0].request_uid.is_none() {
            out.identity_incomplete = true;
        }
        if first_ts < today_start {
            out.yesterday += 1;
            continue;
        }
        out.today += 1;
        if attempts.len() > 1 {
            out.reroutes += 1;
        }
        if let Some(agent) = attempts[0].agent.clone().filter(|a| !a.is_empty()) {
            *out.by_agent.entry(agent).or_default() += 1;
        }
        let last = attempts.last().cloned();
        if let Some(last) = last.filter(|l| l.result != "ok") {
            out.errors += 1;
            out.last_error = Some(last);
        }
    }
    Ok(out)
}

/// 某时刻之后的请求数，按结果分组：(ok, rejected, failed)。供灵动岛「今日经网关」使用。
/// 流水只保留最近 1000 行，当天超过 1000 次时这是下限，调用方据此标注。
pub fn counts_since(conn: &Connection, since_ms: i64) -> rusqlite::Result<(i64, i64, i64)> {
    conn.query_row(
        "WITH grouped AS (
             SELECT MIN(ts) AS first_ts, MAX(id) AS last_id
             FROM gateway_route_log
             GROUP BY CASE WHEN request_uid IS NULL THEN 'legacy:' || id ELSE 'request:' || request_uid END
         )
         SELECT COALESCE(SUM(l.result = 'ok'), 0), COALESCE(SUM(l.result = 'rejected'), 0),
                COALESCE(SUM(l.result NOT IN ('ok', 'rejected')), 0)
         FROM grouped g JOIN gateway_route_log l ON l.id = g.last_id WHERE g.first_ts >= ?1",
        [since_ms],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(result: &str) -> NewEntry<'_> {
        NewEntry {
            agent: None, requested_model: "a/b", source: None, upstream_model: None, status_code: None, result,
            duration_ms: None, connection_id: None, session_platform: None, session_id: None, client_key: None, request_id: None, request_uid: None, resolved_from: None, profile: None, decision: None }
    }

    fn db() -> Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    /// P1：客户端中途断开时，那条已写成 ok 的流水要能改判为 cancelled。
    ///
    /// 为什么必须能改：流式请求的流水在响应头到手时就落库了（那时状态码 200、
    /// 上游确实开始回了）。客户端读到一半断开，这次请求实际上没完成——不改的话
    /// 用量与「成功几次」都会把它算成完全成功。
    #[test]
    fn a_cancelled_stream_flips_its_ok_row_to_cancelled() {
        let conn = db();
        let id = record(&conn, entry("ok")).expect("流水应写入成功");
        assert!(mark_unfinished(&conn, id, StreamEnd::Cancelled).unwrap(), "ok 行应被改判");
        let result: String = conn
            .query_row("SELECT result FROM gateway_route_log WHERE id = ?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(result, "cancelled");
        // 行数不变：改判不能制造第二行，否则「一次请求」会变成两次尝试
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM gateway_route_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        // request_uid 等身份字段原样保留，仍能关联到同一次请求
    }

    #[test]
    fn a_failed_row_is_not_overwritten_by_cancellation() {
        // failed / rejected 是上游或网关给出的更具体结论，不能被「流没读完」盖掉
        let conn = db();
        for result in ["failed", "rejected"] {
            let id = record(&conn, entry(result)).unwrap();
            for end in [StreamEnd::Cancelled, StreamEnd::Failed] {
                assert!(!mark_unfinished(&conn, id, end).unwrap(), "{result} 行不应被改判");
            }
            let stored: String = conn
                .query_row("SELECT result FROM gateway_route_log WHERE id = ?1", [id], |r| r.get(0))
                .unwrap();
            assert_eq!(stored, result);
        }
    }

    /// 包一层 `CancelWatch`，记下回调收到的结局（`None` = 回调没跑）。
    fn watched(body: axum::body::Body) -> (axum::response::Response, std::sync::Arc<std::sync::Mutex<Option<StreamEnd>>>) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let sink = seen.clone();
        let wrapped = CancelWatch::wrap(axum::response::Response::new(body), move |end| {
            *sink.lock().unwrap() = Some(end);
        });
        (wrapped, seen)
    }

    /// 流读完 → 不改判。
    ///
    /// 这是最容易写错的一侧：`Drop` 总会在流结束时执行，如果把「drop 了」当取消，
    /// 每一次成功的流式请求都会被改成 cancelled，比不改还糟。
    #[test]
    fn a_stream_read_to_the_end_is_not_a_cancellation() {
        let (wrapped, seen) = watched(axum::body::Body::from("hello"));
        let collected = blocking_read_body(wrapped);
        assert_eq!(collected, b"hello");
        assert_eq!(*seen.lock().unwrap(), None, "读完不改判");
    }

    /// 上游在流中途报错 → 记失败，而不是取消。
    ///
    /// 报错后 body 会被丢弃、`Drop` 照样执行；修复前 `Drop` 会把它记成客户端断开。
    #[test]
    fn an_upstream_error_mid_stream_is_a_failure_not_a_cancellation() {
        let stream = futures_util::stream::iter(vec![
            Ok(axum::body::Bytes::from_static(b"hello")),
            Err(axum::Error::new(std::io::Error::other("upstream failed"))),
        ]);
        let (wrapped, seen) = watched(axum::body::Body::from_stream(stream));
        let _ = blocking_read_body(wrapped);
        assert_eq!(*seen.lock().unwrap(), Some(StreamEnd::Failed), "上游 Err 要记失败");
    }

    /// 流中途被丢弃 → 记取消。
    #[test]
    fn a_stream_dropped_before_the_end_is_a_cancellation() {
        let (wrapped, seen) = watched(axum::body::Body::from("hello"));
        // 一个字节都不读就直接丢弃
        drop(wrapped);
        assert_eq!(*seen.lock().unwrap(), Some(StreamEnd::Cancelled), "没读完就 drop 要记取消");
    }

    /// 生产路径：排队写入的流水在流中途报错后被改判为 failed，且仍只有一行。
    #[test]
    fn a_queued_row_is_flipped_to_failed_when_the_upstream_errors() {
        let db: Db = std::sync::Arc::new(std::sync::Mutex::new(db()));
        let owned = OwnedEntry {
            agent: None, requested_model: "a/b".into(), source: None, upstream_model: None,
            status_code: Some(200), result: "ok".into(), duration_ms: None, connection_id: None,
            session_platform: None, session_id: None, client_key: None, request_id: None,
            request_uid: Some(7), resolved_from: None, profile: None, decision: None,
        };
        let row = queue_record(&db, 1_000, owned);
        let stream = futures_util::stream::iter(vec![
            Ok(axum::body::Bytes::from_static(b"data: {}\n\n")),
            Err(axum::Error::new(std::io::Error::other("upstream reset"))),
        ]);
        let response = axum::response::Response::new(axum::body::Body::from_stream(stream));
        let _ = blocking_read_body(CancelWatch::watch(response, &db, row.clone()));
        flush();
        let id = row.get().expect("排队写入后应拿到行 id");
        let conn = db.lock().unwrap();
        let (result, ts, rows): (String, i64, i64) = conn
            .query_row(
                "SELECT result, ts, (SELECT COUNT(*) FROM gateway_route_log) FROM gateway_route_log WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((result.as_str(), ts, rows), ("failed", 1_000, 1), "改判原行、时间用提交时给定的值");
    }

    /// 同步读完一个 axum Body（测试用最小实现，避免引入额外依赖）。
    fn blocking_read_body(response: axum::response::Response) -> Vec<u8> {
        use std::sync::{Arc as StdArc, Mutex as StdMutex};
        let collected = StdArc::new(StdMutex::new(Vec::new()));
        let sink = collected.clone();
        let (_parts, body) = response.into_parts();
        let mut stream = Box::pin(body.into_data_stream());
        let waker = futures_util::task::noop_waker();
        let mut context = std::task::Context::from_waker(&waker);
        loop {
            // `into_data_stream` 是 `Stream`，poll_next 来自 trait 而非固有方法
            let polled = futures_util::Stream::poll_next(stream.as_mut(), &mut context);
            match polled {
                std::task::Poll::Ready(Some(Ok(chunk))) => sink.lock().unwrap().extend_from_slice(&chunk),
                std::task::Poll::Ready(Some(Err(_))) | std::task::Poll::Ready(None) => break,
                std::task::Poll::Pending => break,
            }
        }
        let out = collected.lock().unwrap().clone();
        out
    }

    #[test]
    fn cancelling_a_missing_row_reports_no_change() {
        // 行可能已被 KEEP_ROWS 清理掉：返回 false 而不是报错
        let conn = db();
        assert!(!mark_unfinished(&conn, 999, StreamEnd::Cancelled).unwrap());
    }

    #[test]
    fn record_returns_the_row_id_it_wrote() {
        // 取消改判靠这个 id 定位；返回错的值会改到别人的行
        let conn = db();
        let first = record(&conn, entry("ok")).unwrap();
        let second = record(&conn, entry("ok")).unwrap();
        assert!(second > first, "id 必须递增");
        mark_unfinished(&conn, first, StreamEnd::Cancelled).unwrap();
        let (a, b): (String, String) = conn
            .query_row(
                "SELECT (SELECT result FROM gateway_route_log WHERE id=?1),                         (SELECT result FROM gateway_route_log WHERE id=?2)",
                [first, second],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("cancelled", "ok"), "只改指定的那一行");
    }

    /// 范围下拉的各档时间窗：本期起点与上期起点必须连续、等长，且落在自然日零点。
    #[test]
    fn range_bounds_are_contiguous_equal_length_windows() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
        let day = 86_400_000i64;
        for range in [Range::Today, Range::Yesterday, Range::Last7Days, Range::Last30Days] {
            let (current, prior) = range.bounds(today);
            let span = current - prior;
            assert_eq!(span, range.days() * day, "{range:?} 本期与上期长度不等");
            assert!(prior < current, "{range:?} 上期应在本期之前");
        }
        // 具体边界：今天 → 本期今天 00:00，上期昨天 00:00
        let (cur, prior) = Range::Today.bounds(today);
        assert_eq!(cur, midnight_ms(today));
        assert_eq!(prior, midnight_ms(today.pred_opt().unwrap()));
        // 昨天 → 本期昨天 00:00，上期前天 00:00
        let (cur, prior) = Range::Yesterday.bounds(today);
        assert_eq!(cur, midnight_ms(today.pred_opt().unwrap()));
        assert_eq!(prior, midnight_ms(today - chrono::Duration::days(2)));
        // 近 7 天 → 含今天共 7 天，上期为之前 7 天
        let (cur, prior) = Range::Last7Days.bounds(today);
        assert_eq!(cur, midnight_ms(today - chrono::Duration::days(6)));
        assert_eq!(prior, midnight_ms(today - chrono::Duration::days(13)));
        // 近 30 天同理
        let (cur, prior) = Range::Last30Days.bounds(today);
        assert_eq!(cur, midnight_ms(today - chrono::Duration::days(29)));
        assert_eq!(prior, midnight_ms(today - chrono::Duration::days(59)));
    }

    #[test]
    fn range_parse_defaults_to_today_for_unknown_values() {
        assert_eq!(Range::parse(None), Range::Today);
        assert_eq!(Range::parse(Some("")), Range::Today);
        assert_eq!(Range::parse(Some("today")), Range::Today);
        assert_eq!(Range::parse(Some("YESTERDAY")), Range::Yesterday);
        assert_eq!(Range::parse(Some("7d")), Range::Last7Days);
        assert_eq!(Range::parse(Some("30d")), Range::Last30Days);
        // 拼错不能把页面变空：退回今天
        assert_eq!(Range::parse(Some("7days")), Range::Today);
        assert_eq!(Range::parse(Some("week")), Range::Today);
    }

    #[test]
    fn day_overview_counts_requests_not_rows() {
        let conn = db();
        let insert = |ts: i64, agent: &str, result: &str, request_id: Option<i64>, request_uid: Option<i64>| {
            conn.execute(
                "INSERT INTO gateway_route_log(ts, agent, requested_model, result, request_id, request_uid) VALUES(?1, ?2, 'group/x', ?3, ?4, ?5)",
                params![ts, agent, result, request_id, request_uid],
            )
            .unwrap();
        };
        // 昨天 1 次；今天：一次改道后成功、一次两次尝试都失败、一次直接成功（无 request_id）
        insert(50, "claude-code", "ok", Some(1), Some(1));
        insert(150, "claude-code", "failed", Some(2), Some(2));
        insert(151, "claude-code", "ok", Some(2), Some(2));
        insert(160, "codex", "failed", Some(3), Some(3));
        insert(161, "codex", "failed", Some(3), Some(3));
        insert(170, "claude-code", "ok", None, None);
        let day = day_overview(&conn, 100, 0, 1000).unwrap();
        assert_eq!((day.today, day.yesterday, day.reroutes, day.errors), (3, 1, 2, 1));
        assert_eq!(day.by_agent.get("claude-code"), Some(&2));
        assert_eq!(day.by_agent.get("codex"), Some(&1));
        assert_eq!(day.last_error.as_ref().map(|e| e.ts), Some(161), "取最后一次尝试那行");
        assert!(!day.partial);
        // 重启后 request_id 从头计：不能并进上一轮的同号请求
        insert(300, "codex", "ok", Some(500), Some(4));
        insert(301, "codex", "ok", Some(2), Some(5));
        assert_eq!(day_overview(&conn, 100, 0, 1000).unwrap().today, 5);
        assert_eq!(day_overview(&conn, 100, 0, 200).unwrap().today, 3);
    }

    /// P2-3：一次用户请求在分组成员间失败转移，多次尝试必须共用同一个 `request_uid`，
    /// 统计按「一次请求」计数、`reroutes` 记转移次数、`last_error` 取最后一次尝试。
    ///
    /// 这是 P2 交付要求里「重试记录和统计能正确对应」的核心：流水一行一次**尝试**，
    /// 但统计一次**请求**。区分靠 `request_uid`（P0-4 加的持久身份），
    /// 不是「时间相近」的推断。
    #[test]
    fn one_request_failing_over_many_members_counts_once_but_records_every_attempt() {
        let conn = db();
        let insert = |ts: i64, source: &str, result: &str, request_uid: i64, connection_id: &str| {
            conn.execute(
                "INSERT INTO gateway_route_log(ts, agent, requested_model, source, result, request_uid, connection_id)
                 VALUES(?1, 'claude-code', 'group/trio', ?2, ?3, ?4, ?5)",
                params![ts, source, result, request_uid, connection_id],
            )
            .unwrap();
        };
        // 同一 request_uid=7，三个成员依次尝试：a 失败 → b 失败 → c 成功
        insert(100, "a", "failed", 7, "c-a");
        insert(101, "b", "failed", 7, "c-b");
        insert(102, "c", "ok", 7, "c-c");

        let day = day_overview(&conn, 0, 0, 1000).unwrap();
        assert_eq!(day.today, 1, "三次尝试是一次请求");
        assert_eq!(day.reroutes, 1, "发生了转移");
        assert_eq!(day.errors, 0, "最终成功，不算错误请求");
        assert!(day.last_error.is_none(), "成功时不该有 last_error");

        // 流水本身保留全部三次尝试（可解释性）
        let rows = recent(&conn, 10).unwrap();
        assert_eq!(rows.len(), 3, "流水一行一次尝试，不能合并");
        assert!(rows.iter().all(|r| r.request_uid == Some(7)), "三次尝试共享身份");
        let sources: Vec<&str> = rows.iter().filter_map(|r| r.source.as_deref()).collect();
        assert_eq!(sources, ["c", "b", "a"], "最近在前");

        // 全部失败：仍是一次请求，`last_error` 取最后一次尝试
        let insert2 = |ts: i64, source: &str, result: &str| {
            conn.execute(
                "INSERT INTO gateway_route_log(ts, agent, requested_model, source, result, request_uid, connection_id)
                 VALUES(?1, 'codex', 'group/trio', ?2, ?3, 8, 'c-')",
                params![ts, source, result],
            )
            .unwrap();
        };
        insert2(200, "a", "failed");
        insert2(201, "b", "failed");
        let day = day_overview(&conn, 0, 0, 1000).unwrap();
        assert_eq!(day.today, 2, "第二次请求");
        assert_eq!(day.errors, 1, "这次请求最终失败");
        assert_eq!(day.last_error.as_ref().map(|e| e.ts), Some(201), "取最后一次尝试");
    }

    /// P2-3：失败转移时，流水能看出「每个成员各自试过几次」——这是可解释路由的前提。
    #[test]
    fn attempts_are_grouped_by_request_and_keep_per_attempt_source() {
        let conn = db();
        for (uid, source, result) in [
            (11, "zcode/zai", "failed"),
            (11, "workbuddy/wb", "ok"),
            (12, "zcode/zai", "ok"),
        ] {
            conn.execute(
                "INSERT INTO gateway_route_log(ts, requested_model, source, result, request_uid)
                 VALUES(?1, 'group/dual', ?2, ?3, ?4)",
                params![100 + uid, source, result, uid],
            )
            .unwrap();
        }
        let rows = recent(&conn, 10).unwrap();
        // 两次请求、三条尝试
        assert_eq!(rows.len(), 3);
        let uid11: Vec<_> = rows.iter().filter(|r| r.request_uid == Some(11)).collect();
        assert_eq!(uid11.len(), 2, "第一次请求两次尝试");
        assert!(uid11.iter().any(|r| r.source.as_deref() == Some("zcode/zai") && r.result == "failed"));
        assert!(uid11.iter().any(|r| r.source.as_deref() == Some("workbuddy/wb") && r.result == "ok"));
        let day = day_overview(&conn, 0, 0, 1000).unwrap();
        assert_eq!(day.today, 2, "两次请求");
        assert_eq!(day.reroutes, 1, "只有第一次转移过");
    }

    /// P2-4：路由决策的解释要能落到流水里并可读回。
    ///
    /// 缺了这一列时，用户只能看到「用了 workbuddy/wb」，看不出「为什么是它」——
    /// 是分组策略排的？额度用满被降权了？还是亲和把它提到的前面？这些恰恰是
    /// 「可解释路由」要回答的问题。
    #[test]
    fn a_group_decision_explanation_round_trips_through_the_log() {
        let conn = db();
        record(&conn, NewEntry {
            agent: Some("claude-code"),
            requested_model: "group/trio",
            source: Some("wb"),
            upstream_model: Some("deepseek-v4"),
            status_code: Some(200),
            result: "ok",
            duration_ms: Some(42),
            connection_id: Some("c-wb"),
            session_platform: Some("claude"),
            session_id: Some("s1"),
            client_key: None,
            request_id: Some(1),
            request_uid: Some(1),
            resolved_from: None,
            profile: None,
            decision: Some("分组 trio · 策略 pace · 1 个成员额度已用满，已排到候选末尾"),
        });
        let rows = recent(&conn, 1).unwrap();
        let decision = rows[0].decision.as_deref().expect("决策解释应保留");
        assert!(decision.contains("策略 pace"), "要能看出用的哪个策略：{decision}");
        assert!(decision.contains("额度已用满"), "要能看出降级原因：{decision}");
    }

    /// 普通单来源请求没有决策解释——不要为它编一句话。
    #[test]
    fn a_plain_request_has_no_decision_explanation() {
        let conn = db();
        record(&conn, NewEntry {
            agent: Some("codex"), requested_model: "kimi/kimi-k2", source: Some("kimi"),
            upstream_model: Some("kimi-k2"), status_code: Some(200), result: "ok",
            duration_ms: Some(10), connection_id: Some("c1"), session_platform: None, session_id: None,
            client_key: None, request_id: None, request_uid: None,
            resolved_from: Some("bare"), profile: None, decision: None,
        });
        assert_eq!(recent(&conn, 1).unwrap()[0].decision, None);
    }

    #[test]
    fn recent_is_newest_first_and_keeps_metadata() {
        let conn = db();
        record(&conn, NewEntry { agent: Some("codex"), requested_model: "kimi/kimi-k2", source: Some("kimi"), upstream_model: Some("kimi-k2"), status_code: Some(200), result: "ok", duration_ms: Some(42), connection_id: Some("c1"), session_platform: Some("codex"), session_id: Some("s1"), client_key: None, request_id: None, request_uid: None, resolved_from: None, profile: None, decision: None });
        record(&conn, NewEntry { agent: None, requested_model: "nope/m", source: None, upstream_model: None, status_code: Some(404), result: "rejected", duration_ms: None, connection_id: None, session_platform: None, session_id: None, client_key: None, request_id: None, request_uid: None, resolved_from: None, profile: None, decision: None });
        let rows = recent(&conn, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].result, "rejected");
        assert!(rows[0].id > rows[1].id);
        assert_eq!(rows[1].agent.as_deref(), Some("codex"));
        assert_eq!(rows[1].duration_ms, Some(42));
        assert_eq!(rows[1].session_id.as_deref(), Some("s1"));
        assert_eq!(rows[1].connection_id.as_deref(), Some("c1"));
    }

    #[test]
    fn counts_since_groups_by_result() {
        let conn = db();
        for result in ["ok", "ok", "rejected", "failed", "cancelled"] {
            record(&conn, entry(result));
        }
        // 第三项是「既非 ok 也非 rejected」：failed 与 cancelled 都进这里。
        // 灵动岛靠这一项判断「今天有多少次没成」，把 cancelled 漏掉会少报。
        assert_eq!(counts_since(&conn, 0).unwrap(), (2, 1, 2));
        assert_eq!(counts_since(&conn, i64::MAX).unwrap(), (0, 0, 0));
    }

    /// 流水被滚动删除后，`partial` 必须为真：统计只代表当前保留范围。
    ///
    /// 此前只有 `assert!(!day.partial)`（数据量小时的否定断言），从没测过「真的被截断」。
    /// 这条把流水灌到保留上限之上，再断言 `partial` 为真。
    #[test]
    fn a_truncated_log_is_reported_as_partial() {
        let conn = db();
        // 上期与本期的时间：上期在 yesterday_start 之前
        let (yesterday_start, today_start, end) = (1_000i64, 2_000i64, 3_000i64);
        let insert = |ts: i64| {
            conn.execute(
                "INSERT INTO gateway_route_log(ts, requested_model, result) VALUES(?1, 'm', 'ok')",
                params![ts],
            )
            .unwrap();
        };
        // 少量数据：不完整标记应为假
        for i in 0..10 {
            insert(today_start + i);
        }
        assert!(!day_overview(&conn, today_start, yesterday_start, end).unwrap().partial);

        // 精确灌到保留上限（含前面 10 条），且最早一行落在上期起点之后
        for i in 0..(KEEP_ROWS - 10) {
            insert(today_start + 100 + i);
        }
        let exact: i64 = conn.query_row("SELECT COUNT(*) FROM gateway_route_log", [], |r| r.get(0)).unwrap();
        assert_eq!(exact, KEEP_ROWS, "先确认行数正好等于上限");
        let day = day_overview(&conn, today_start, yesterday_start, end).unwrap();
        assert!(
            day.partial,
            "行数正好等于保留上限时无法排除更早的行刚被删掉，必须标不完整（用 > 会漏掉这一档）"
        );
        assert!(day.identity_incomplete, "这些行没有 request_uid，身份同样不完整");
    }

    #[test]
    fn request_counts_keep_restarts_and_legacy_rows_separate() {
        let conn = db();
        let insert = |ts, request_id, request_uid, result| {
            conn.execute(
                "INSERT INTO gateway_route_log(ts, requested_model, result, request_id, request_uid) VALUES (?1, 'm', ?2, ?3, ?4)",
                params![ts, result, request_id, request_uid],
            ).unwrap();
        };
        insert(90, Some(1), Some(10), "failed");
        insert(110, Some(2), Some(11), "ok");
        insert(120, Some(1), Some(10), "ok");
        insert(130, Some(1), Some(12), "failed");
        insert(140, Some(1), None::<i64>, "failed");
        insert(150, Some(1), None::<i64>, "ok");

        // 按保留流水中首次尝试时间归属。旧行缺身份，不能证明彼此属于同一次请求。
        assert_eq!(counts_since(&conn, 100).unwrap(), (2, 0, 2));
        let overview = day_overview(&conn, 100, 0, 1000).unwrap();
        assert_eq!((overview.today, overview.yesterday, overview.errors), (4, 1, 2));
        assert!(overview.identity_incomplete);
    }

    /// 局域网共享（P8.3）：命中的命名客户端 Key 随流水落库，供限额按 Key 名统计。
    #[test]
    fn client_key_is_recorded_and_readable_back() {
        let conn = db();
        record(&conn, NewEntry { client_key: Some("lab-agent"), ..entry("ok") });
        record(&conn, NewEntry { client_key: None, ..entry("ok") });
        let rows = recent(&conn, 10).unwrap();
        assert_eq!(rows[0].client_key, None, "本机请求不带 Key 名");
        assert_eq!(rows[1].client_key.as_deref(), Some("lab-agent"));
        let named = conn
            .query_row(
                "SELECT COUNT(*) FROM gateway_route_log WHERE client_key = 'lab-agent' AND result = 'ok'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(named, 1);
    }

    #[test]
    fn page_handles_empty_full_and_partial_pages() {
        let conn = db();
        let empty = page(&conn, 1, 10, None).unwrap();
        assert_eq!((empty.total, empty.page_count, empty.page, empty.rows.len()), (0, 0, 1, 0));
        assert_eq!(empty.upper_id, None);
        for count in [1usize, 9, 1] {
            for _ in 0..count {
                record(&conn, entry("ok"));
            }
            let total = conn.query_row("SELECT COUNT(*) FROM gateway_route_log", [], |r| r.get::<_, i64>(0)).unwrap();
            let first = page(&conn, 1, 10, None).unwrap();
            assert_eq!(first.total, total);
            assert_eq!(first.page_count, (total + 9) / 10);
            assert_eq!(first.rows.len() as i64, total.min(10));
        }
        // 11 条：第 2 页只有 1 条，超出范围的页收拢到最后一页
        let last = page(&conn, 2, 10, None).unwrap();
        assert_eq!((last.page, last.rows.len()), (2, 1));
        let over = page(&conn, 99, 10, None).unwrap();
        assert_eq!(over.page, 2);
    }

    #[test]
    fn upper_bound_keeps_pages_stable_while_new_rows_arrive() {
        let conn = db();
        for _ in 0..25 {
            record(&conn, entry("ok"));
        }
        let first = page(&conn, 1, 10, None).unwrap();
        let upper = first.upper_id;
        for _ in 0..7 {
            record(&conn, entry("failed"));
        }
        let second = page(&conn, 2, 10, upper).unwrap();
        let third = page(&conn, 3, 10, upper).unwrap();
        assert_eq!(second.total, 25, "新写入不进入本次浏览范围");
        let mut ids: Vec<i64> = first.rows.iter().chain(&second.rows).chain(&third.rows).map(|r| r.id).collect();
        assert_eq!(ids.len(), 25);
        ids.dedup();
        assert_eq!(ids.len(), 25, "连续翻页没有重复");
        assert!(ids.windows(2).all(|w| w[0] > w[1]), "按 id 从大到小且无遗漏");
        let refreshed = page(&conn, 1, 10, None).unwrap();
        assert_eq!(refreshed.total, 32, "显式刷新重新建立范围");
    }

    #[test]
    fn source_usage_counts_per_connection_and_skips_rows_without_one() {
        let conn = db();
        let with = |conn_id: &'static str, result: &'static str| NewEntry { connection_id: Some(conn_id), ..entry(result) };
        record(&conn, with("c1", "ok"));
        record(&conn, with("c1", "failed"));
        record(&conn, with("c2", "ok"));
        record(&conn, entry("rejected"));
        let usage = source_usage(&conn).unwrap();
        assert_eq!(usage.len(), 2);
        let c1 = usage.iter().find(|u| u.connection_id == "c1").unwrap();
        assert_eq!((c1.requests, c1.failed), (2, 1));
    }

    #[test]
    fn session_links_group_by_session_and_connection_with_latest_model() {
        let conn = db();
        let link = |model: &'static str, conn_id: &'static str, session: &'static str| NewEntry {
            agent: Some("codex"), requested_model: model, source: Some("kimi"), upstream_model: None, status_code: Some(200),
            result: "ok", duration_ms: None, connection_id: Some(conn_id), session_platform: Some("codex"), session_id: Some(session),
            client_key: None, request_id: None, request_uid: None, resolved_from: None, profile: None, decision: None };
        record(&conn, link("kimi/a", "c1", "s1"));
        record(&conn, link("kimi/b", "c1", "s1"));
        record(&conn, link("kimi/a", "c2", "s1"));
        record(&conn, link("kimi/a", "c1", "s2"));
        record(&conn, entry("ok"));
        let links = session_links(&conn).unwrap();
        assert_eq!(links.len(), 3, "没有会话标识的流水不参与关联");
        assert_eq!((links[0].session_id.as_str(), links[0].connection_id.as_deref()), ("s2", Some("c1")));
        let s1c1 = links.iter().find(|l| l.session_id == "s1" && l.connection_id.as_deref() == Some("c1")).unwrap();
        assert_eq!((s1c1.requests, s1c1.requested_model.as_str()), (2, "kimi/b"));
    }
}
