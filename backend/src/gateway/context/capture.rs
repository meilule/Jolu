//! 采集：在调用方入口把请求体交给解析层，上游回了用量后校准，结果写进 `gateway_context_snapshot`
//! （执行文档 §6「决定」）。
//!
//! 时序：
//! 1. [`Capture::start`] 在后台线程写一行 `analyzing`，然后解析，写成 `ready` / `unsupported` / `no_body` / `failed`；
//! 2. 选路时 [`Capture::set_route`] 记下最后一次尝试的连接与模型（校准取它的用量，模型窗口按它查）；
//! 3. [`Capture::meter`] 给响应套一层，流读完（或客户端断开）时读出上游用量，再写一次。
//!
//! 2 与 3 可能早于解析完成：每一步都只改内存里的状态，再把「当前该是的样子」整行写一次，谁后到谁写最终值。
//! 数据库或解析出任何问题都只影响快照，不影响转发。

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::store::{self, Row, State, Usage};
use super::{parse_checked, Protocol, Prompt};

type Db = Arc<Mutex<Connection>>;

#[derive(Default)]
struct Shared {
    id: Option<i64>,
    state: Option<State>,
    prompt: Option<Prompt>,
    error: Option<String>,
    usage: Option<Usage>,
    route: Option<(String, String, String)>,
    window: Option<i64>,
}

/// 一次请求的采集。克隆共享同一份状态。
#[derive(Clone)]
pub(crate) struct Capture {
    shared: Arc<Mutex<Shared>>,
    db: Db,
}

impl Capture {
    /// 开始采集：后台写 `analyzing` 行并解析，不等它完成。
    pub(crate) fn start(db: Db, protocol: Protocol, body: axum::body::Bytes, platform: &str, session_id: &str, request_id: u64) -> Capture {
        let capture = Capture { shared: Arc::new(Mutex::new(Shared::default())), db };
        let worker = capture.clone();
        let (platform, session_id) = (platform.to_string(), session_id.to_string());
        let job = move || worker.run(protocol, &body, &platform, &session_id, request_id);
        // 网关在 tokio 运行时里；单元测试等没有运行时的场合直接在当前线程跑
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn_blocking(job);
            }
            Err(_) => job(),
        }
        capture
    }

    fn run(&self, protocol: Protocol, body: &[u8], platform: &str, session_id: &str, request_id: u64) {
        let new = store::NewSnapshot {
            session_platform: platform,
            session_id,
            request_id: request_id as i64,
            protocol: protocol.as_str(),
            ts: chrono::Local::now().timestamp_millis(),
        };
        let id = match self.db.lock() {
            Ok(conn) => store::insert(&conn, &new),
            Err(_) => return,
        };
        let id = match id {
            Ok(id) => id,
            Err(error) => {
                eprintln!("[上下文观察] 快照写入失败：{error}");
                return;
            }
        };
        if let Ok(mut shared) = self.shared.lock() {
            shared.id = Some(id);
        }
        let (state, prompt, error) = if body.iter().all(u8::is_ascii_whitespace) {
            (State::NoBody, None, None)
        } else {
            match parse_checked(protocol, body) {
                Ok(Some(prompt)) => (State::Ready, Some(prompt), None),
                Ok(None) => (State::Unsupported, None, None),
                Err(reason) => (State::Failed, None, Some(reason)),
            }
        };
        let Ok(mut shared) = self.shared.lock() else { return };
        shared.state = Some(state);
        shared.prompt = prompt;
        shared.error = error;
        self.flush(&shared);
    }

    /// 记下这次尝试的连接、平台与发给上游的模型（换候选时覆盖为最后一次）。
    pub(crate) fn set_route(&self, connection_id: &str, platform: &str, model: &str) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.route = Some((connection_id.to_string(), platform.to_string(), model.to_string()));
        }
    }

    /// 上游回了用量（或确定没有）。
    pub(crate) fn finish(&self, usage: Usage) {
        let Ok(mut shared) = self.shared.lock() else { return };
        if shared.usage.is_some() {
            return;
        }
        shared.usage = Some(usage);
        shared.window = shared.route.as_ref().and_then(|(_, platform, model)| self.window_of(platform, model));
        self.flush(&shared);
    }

    /// 把当前状态整行写进数据库（还没拿到行 id 时不写，等 `run` 写）。
    fn flush(&self, shared: &Shared) {
        let Some(id) = shared.id else { return };
        let usage = shared.usage.unwrap_or_default();
        let prompt = shared.prompt.as_ref().map(|p| match usage.input {
            Some(counted) if counted > 0 => p.calibrated(counted),
            _ => p.clone(),
        });
        let row = Row {
            state: shared.state.unwrap_or(State::Analyzing),
            connection_id: shared.route.as_ref().map(|(id, _, _)| id.clone()),
            usage,
            window: shared.window,
            prompt,
            error: shared.error.clone(),
        };
        let written = match self.db.lock() {
            Ok(conn) => store::update(&conn, id, &row),
            Err(_) => return,
        };
        if let Err(error) = written {
            eprintln!("[上下文观察] 快照更新失败：{error}");
        }
    }

    /// 模型窗口：models.dev 缓存（与数据库同目录）→ 内置快照，再套来源级覆盖。查不到为 `None`。
    fn window_of(&self, platform: &str, model: &str) -> Option<i64> {
        let dir = self
            .db
            .lock()
            .ok()
            .and_then(|conn| conn.path().and_then(|p| std::path::Path::new(p).parent().map(std::path::Path::to_path_buf)))
            .filter(|dir| !dir.as_os_str().is_empty());
        let from_catalog = Some(dir.map_or_else(|| crate::gateway::models_dev::builtin().clone(), |dir| catalog_in(&dir)))
            .and_then(|catalog| crate::gateway::models_dev::lookup_meta(&catalog, model))
            .map(|meta| meta.context_window)
            .filter(|w| *w > 0);
        crate::gateway::model_overrides::lookup(platform, model).and_then(|o| o.context_window).or(from_catalog)
    }

    /// 给响应套一层：透传字节，结束（或中断）时读出上游用量交给 [`Capture::finish`]。失败的响应记「没有用量」。
    pub(crate) fn meter(&self, response: axum::response::Response) -> axum::response::Response {
        let success = response.status().is_success();
        let (parts, body) = response.into_parts();
        let stream = Metered { inner: body.into_data_stream(), meter: Default::default(), capture: Some(self.clone()), success };
        axum::response::Response::from_parts(parts, axum::body::Body::from_stream(stream))
    }
}

/// models.dev 缓存每 10 分钟最多重读一次（文件可能有几 MB，不能每个请求读一遍）。
fn catalog_in(dir: &std::path::Path) -> crate::gateway::models_dev::CachedCatalog {
    use crate::gateway::models_dev::{cached_in_dir, CachedCatalog};
    static CACHE: Mutex<Option<(std::path::PathBuf, std::time::Instant, Arc<CachedCatalog>)>> = Mutex::new(None);
    let mut guard = match CACHE.lock() {
        Ok(g) => g,
        Err(_) => return cached_in_dir(dir),
    };
    if let Some((cached_dir, at, catalog)) = guard.as_ref() {
        if cached_dir == dir && at.elapsed() < std::time::Duration::from_secs(600) {
            return (**catalog).clone();
        }
    }
    let catalog = Arc::new(cached_in_dir(dir));
    *guard = Some((dir.to_path_buf(), std::time::Instant::now(), catalog.clone()));
    (*catalog).clone()
}

struct Metered {
    inner: axum::body::BodyDataStream,
    meter: crate::gateway::budget::UsageMeter,
    capture: Option<Capture>,
    success: bool,
}

impl Metered {
    fn settle(&mut self) {
        let Some(capture) = self.capture.take() else { return };
        if !self.success {
            capture.finish(Usage::default());
            return;
        }
        self.meter.finish();
        let (input, output) = self.meter.parts();
        capture.finish(Usage { input, output, cache_read: self.meter.cache_read() });
    }
}

impl futures_util::Stream for Metered {
    type Item = Result<axum::body::Bytes, axum::Error>;

    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        let this = &mut *self;
        let polled = std::pin::Pin::new(&mut this.inner).poll_next(cx);
        match &polled {
            std::task::Poll::Ready(Some(Ok(bytes))) => this.meter.feed(bytes),
            std::task::Poll::Ready(None) | std::task::Poll::Ready(Some(Err(_))) => this.settle(),
            std::task::Poll::Pending => {}
        }
        polled
    }
}

impl Drop for Metered {
    /// 客户端中途断开：按已读到的用量记。
    fn drop(&mut self) {
        self.settle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::context::{Kind, Protocol};

    const CLAUDE: &str = include_str!("testdata/claude-code-2.1.326-3.anthropic.json");

    fn fresh_db() -> Db {
        Arc::new(Mutex::new(crate::db::open(std::path::Path::new(":memory:")).unwrap()))
    }

    fn idle(db: &Db) -> Capture {
        Capture { shared: Arc::new(Mutex::new(Shared::default())), db: db.clone() }
    }

    fn row(db: &Db, id: i64) -> store::Stored {
        store::get(&db.lock().unwrap(), id).unwrap().expect("快照行")
    }

    fn only_id(db: &Db) -> i64 {
        store::id_of_request(&db.lock().unwrap(), 7).unwrap().expect("request 7 的快照")
    }

    /// 解析先完成、用量后到：先是 ready 的估算，用量到了按实测校准；估算值保留，缓存命中与连接写上。
    #[test]
    fn parse_then_usage_calibrates() {
        let db = fresh_db();
        let capture = Capture::start(db.clone(), Protocol::Anthropic, CLAUDE.as_bytes().to_vec().into(), "claude-code", "ses-1", 7);
        let id = only_id(&db);
        let first = row(&db, id);
        assert_eq!((first.state.as_str(), first.counted, first.input_tokens, first.cache_read), ("ready", false, None, None));
        let estimated = first.estimated.unwrap();
        let sum: i64 = first.parts.as_ref().unwrap().iter().map(|p| p.tokens).sum();
        assert_eq!(sum, estimated);
        capture.set_route("conn-1", "claude", "claude-sonnet-4-5");
        capture.finish(Usage { input: Some(30_000), output: Some(12), cache_read: Some(20_000) });
        let done = row(&db, id);
        assert_eq!((done.state.as_str(), done.counted), ("ready", true));
        assert_eq!((done.input_tokens, done.output_tokens, done.cache_read), (Some(30_000), Some(12), Some(20_000)));
        assert_eq!(done.estimated, Some(estimated), "估算值保留，用来提示偏差");
        assert_eq!(done.parts.as_ref().unwrap().iter().map(|p| p.tokens).sum::<i64>(), 30_000);
        assert_eq!(done.connection_id.as_deref(), Some("conn-1"));
        let expected_window = crate::gateway::models_dev::lookup_meta(crate::gateway::models_dev::builtin(), "claude-sonnet-4-5")
            .map(|m| m.context_window)
            .filter(|w| *w > 0);
        assert_eq!(done.window_tokens, expected_window);
        assert_eq!((done.session_platform.as_str(), done.session_id.as_str(), done.protocol.as_str()), ("claude-code", "ses-1", "anthropic"));
        // 逐条工具结果：Read 读 README、Bash cat src/cart.ts，不含正文
        let calls = done.calls.unwrap();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert_eq!((calls[0].tool.as_str(), calls[0].kind), ("Read", Kind::Files));
        assert_eq!(calls[0].target.as_deref(), Some(r"D:\work\demo-shop\README.md"));
        assert_eq!((calls[1].tool.as_str(), calls[1].target.as_deref()), ("Bash", Some("cat src/cart.ts")));
        assert!(calls.iter().all(|c| !c.captured && c.lines > 0 && c.summary.starts_with("输出")), "{calls:?}");
        // 同一次请求的用量只认第一次
        capture.finish(Usage { input: Some(1), output: None, cache_read: None });
        assert_eq!(row(&db, id).input_tokens, Some(30_000));
    }

    /// 用量先到、解析后完成：最终一样是校准后的 ready。
    #[test]
    fn usage_before_parse_still_ends_calibrated() {
        let db = fresh_db();
        let capture = idle(&db);
        capture.finish(Usage { input: Some(50_000), output: Some(3), cache_read: None });
        capture.run(Protocol::Anthropic, CLAUDE.as_bytes(), "claude-code", "ses-2", 7);
        let done = row(&db, only_id(&db));
        assert_eq!((done.state.as_str(), done.counted, done.input_tokens, done.cache_read), ("ready", true, Some(50_000), None));
        assert_eq!(done.parts.unwrap().iter().map(|p| p.tokens).sum::<i64>(), 50_000);
    }

    /// 请求体不是 JSON → unsupported；空请求体 → no_body；上游失败没有用量 → 各项 NULL，不是 0。
    #[test]
    fn states_and_unknowns() {
        let db = fresh_db();
        let capture = Capture::start(db.clone(), Protocol::Chat, b"not json".to_vec().into(), "codex", "s", 7);
        capture.finish(Usage::default());
        let r = row(&db, only_id(&db));
        assert_eq!((r.state.as_str(), r.parts.is_none(), r.estimated, r.input_tokens, r.cache_read, r.window_tokens, r.counted),
                   ("unsupported", true, None, None, None, None, false));
        let db = fresh_db();
        Capture::start(db.clone(), Protocol::Chat, axum::body::Bytes::new(), "codex", "s", 7);
        assert_eq!(row(&db, only_id(&db)).state, "no_body");
    }

    /// 计量层：读出 Anthropic 流里的输入（含缓存）与缓存命中，失败的响应记没有用量。
    #[tokio::test]
    async fn meter_reads_upstream_usage() {
        let db = fresh_db();
        let capture = idle(&db);
        capture.run(Protocol::Anthropic, CLAUDE.as_bytes(), "claude-code", "ses-3", 7);
        let sse = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":900,\"cache_creation_input_tokens\":0,\"output_tokens\":1}}}\n\n\
                   event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":42}}\n\n";
        let response = axum::response::Response::builder().status(200).body(axum::body::Body::from(sse)).unwrap();
        let body = axum::body::to_bytes(capture.meter(response).into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.len(), sse.len(), "透传不改字节");
        let r = row(&db, only_id(&db));
        assert_eq!((r.input_tokens, r.output_tokens, r.cache_read, r.counted), (Some(1000), Some(42), Some(900), true));

        let db = fresh_db();
        let capture = idle(&db);
        capture.run(Protocol::Anthropic, CLAUDE.as_bytes(), "claude-code", "ses-4", 7);
        let failed = axum::response::Response::builder().status(429).body(axum::body::Body::from("{\"usage\":{\"input_tokens\":5}}")).unwrap();
        let _ = axum::body::to_bytes(capture.meter(failed).into_body(), usize::MAX).await.unwrap();
        let r = row(&db, only_id(&db));
        assert_eq!((r.input_tokens, r.counted), (None, false));
    }

    /// 清理：每个会话留最近 20 条。
    #[test]
    fn prune_keeps_recent_per_session() {
        let db = fresh_db();
        let conn = db.lock().unwrap();
        for i in 0..25 {
            store::insert(&conn, &store::NewSnapshot { session_platform: "codex", session_id: "a", request_id: i, protocol: "responses", ts: i }).unwrap();
        }
        for i in 0..3 {
            store::insert(&conn, &store::NewSnapshot { session_platform: "codex", session_id: "b", request_id: 100 + i, protocol: "responses", ts: i }).unwrap();
        }
        store::prune(&conn);
        let count = |sid: &str| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM gateway_context_snapshot WHERE session_id = ?1", [sid], |r| r.get(0)).unwrap()
        };
        assert_eq!((count("a"), count("b")), (store::KEEP_PER_SESSION, 3));
        let oldest: i64 = conn.query_row("SELECT MIN(request_id) FROM gateway_context_snapshot WHERE session_id = 'a'", [], |r| r.get(0)).unwrap();
        assert_eq!(oldest, 5, "删的是最早的");
    }
}
