//! 应用共享状态容器、只读连接池、异步数据库调度与网络查询去重缓存。
//!
//! 供 `commands/*`、`tray_summary`、`context_menu` 与宿主层显式导入，替代原 `lib.rs` 顶层的全局混放。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};

#[derive(serde::Serialize)]
pub(crate) struct DataInfo {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct SourceInfo {
    pub id: String,
    pub platform: String,
    pub name: String,
    pub available: bool,
    pub capabilities: Vec<String>,
    pub reason: Option<String>,
}

pub(crate) fn platform_name(id: &str) -> &str {
    crate::platforms::get(id).map(|p| p.name.as_str()).unwrap_or(id)
}

pub(crate) fn local_source(platform: &str) -> Option<SourceInfo> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from);
    let path = match platform {
        "claude" => home.map(|path| path.join(".claude").join("projects")),
        "codex" => crate::session_titles::codex_home().map(|path| path.join("sessions")),
        _ => return None,
    };
    let available = path.as_ref().is_some_and(|path| path.is_dir());
    Some(SourceInfo {
        id: format!("local:{platform}"),
        platform: platform.into(),
        name: format!("本机 {} 会话记录", platform_name(platform)),
        available,
        capabilities: vec![
            "token_totals".into(),
            "request_log".into(),
            "realtime_delta".into(),
        ],
        reason: (!available)
            .then(|| format!("未检测到本机 {} 会话目录", platform_name(platform))),
    })
}

/// 全局写连接。
pub struct Db(pub Arc<Mutex<rusqlite::Connection>>);

/// 只读查询连接池。
pub struct DbRead(pub Arc<ReadPool>);

pub struct ReadPool {
    conns: Vec<Mutex<rusqlite::Connection>>,
    next: std::sync::atomic::AtomicUsize,
}

impl ReadPool {
    const SIZE: usize = 3;

    pub(crate) fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        let conns = (0..Self::SIZE)
            .map(|_| crate::db::open_reader(path).map(Mutex::new))
            .collect::<rusqlite::Result<_>>()?;
        Ok(Self {
            conns,
            next: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    pub fn with<T>(&self, work: impl FnOnce(&rusqlite::Connection) -> T) -> T {
        for conn in &self.conns {
            if let Ok(guard) = conn.try_lock() {
                return work(&guard);
            }
        }
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.conns.len();
        let guard = self.conns[index]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        work(&guard)
    }
}

/// 在阻塞线程池上借一条只读连接执行查询。
pub(crate) async fn with_reader<T: Send + 'static>(
    pool: Arc<ReadPool>,
    work: impl FnOnce(&rusqlite::Connection) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || pool.with(work))
        .await
        .map_err(|e| e.to_string())?
}

/// 在阻塞线程池等待数据库锁，避免扫描期间卡住窗口或异步运行时。
pub(crate) async fn with_database<T: Send + 'static>(
    db: Arc<Mutex<rusqlite::Connection>>,
    work: impl FnOnce(&mut rusqlite::Connection) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db.lock().map_err(|e| e.to_string())?;
        work(&mut conn)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 持久化设置容器。
pub struct Cfg(pub crate::settings::Store);

pub(crate) const ISLAND: &str = "island";
pub(crate) const MAIN: &str = "main";
pub(crate) const STARTUP: &str = "startup";

pub(crate) static TRAY_REFRESHING: AtomicBool = AtomicBool::new(false);
pub(crate) static STARTUP_READY: AtomicBool = AtomicBool::new(false);
pub(crate) static STARTUP_INITIALIZING: AtomicBool = AtomicBool::new(false);
pub(crate) static STARTUP_REQUESTED: AtomicBool = AtomicBool::new(false);
pub(crate) static MAIN_READY: AtomicBool = AtomicBool::new(false);
pub(crate) static FIRST_INSTALL: AtomicBool = AtomicBool::new(false);
pub(crate) static FIRST_INSTALL_INTRO: AtomicBool = AtomicBool::new(false);
pub(crate) static MAIN_PLACEMENT: LazyLock<Mutex<Option<crate::settings::types::Placement>>> =
    LazyLock::new(|| Mutex::new(None));
pub(crate) static STARTUP_ERROR: LazyLock<Mutex<Option<String>>> =
    LazyLock::new(|| Mutex::new(None));

#[derive(Clone, serde::Serialize)]
pub(crate) struct CollectionStatus {
    pub ok: bool,
    pub errors: Vec<String>,
    pub last_success_ms: Option<i64>,
    pub failed_sources: Vec<String>,
    pub consecutive_failures: u32,
}

const COLLECTION_FAILURE_THRESHOLD: u32 = 2;

pub(crate) fn evaluate_collection_status(consecutive: u32, errors: &[String]) -> (bool, u32) {
    if errors.is_empty() {
        return (true, 0);
    }
    let next = consecutive + 1;
    (next < COLLECTION_FAILURE_THRESHOLD, next)
}

pub(crate) static COLLECTION_STATUS: LazyLock<Mutex<CollectionStatus>> = LazyLock::new(|| {
    Mutex::new(CollectionStatus {
        ok: true,
        errors: Vec::new(),
        last_success_ms: None,
        failed_sources: Vec::new(),
        consecutive_failures: 0,
    })
});

pub(crate) fn emit_collection_status(app: &tauri::AppHandle, result: &crate::collector::ScanResult) {
    let status = if let Ok(mut current) = COLLECTION_STATUS.lock() {
        let (ok, consecutive) =
            evaluate_collection_status(current.consecutive_failures, &result.errors);
        if !ok {
            eprintln!(
                "[采集] 连续 {} 轮扫描出错：{}",
                consecutive,
                result.errors.join("；")
            );
        }
        current.ok = ok;
        current.consecutive_failures = consecutive;
        current.errors = result.errors.clone();
        current.failed_sources = if ok {
            Vec::new()
        } else {
            result.failed_sources.clone()
        };
        if current.ok {
            current.last_success_ms = Some(chrono::Local::now().timestamp_millis());
        }
        current.clone()
    } else {
        CollectionStatus {
            ok: false,
            errors: vec!["采集状态锁不可用".into()],
            last_success_ms: None,
            failed_sources: Vec::new(),
            consecutive_failures: 0,
        }
    };
    let _ = app.emit(crate::events::COLLECTION_STATUS, status);
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum MainIntent {
    Providers { connection_id: String },
    Overview { settings: crate::settings::Settings },
    Settings { section: String },
    About { version: String },
    Automation { task_id: i64 },
    Sessions { platform: String, session_id: String },
    Usage { platform: Option<String>, connection_id: Option<String> },
    AutomationNew { platform: Option<String>, session_id: Option<String> },
}

pub(crate) static MAIN_INTENT: LazyLock<Mutex<Option<MainIntent>>> =
    LazyLock::new(|| Mutex::new(None));

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CacheBackoff {
    None,
    RateLimited,
    Failed,
}

pub(crate) struct QueryCacheState<T> {
    pub(crate) entries: HashMap<String, (T, Instant, CacheBackoff, Instant)>,
    pub(crate) in_flight: HashSet<String>,
    pub(crate) failures: HashMap<String, u32>,
    pub(crate) generations: HashMap<String, u64>,
    pub(crate) identities: HashMap<String, String>,
}

impl<T> Default for QueryCacheState<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            in_flight: HashSet::new(),
            failures: HashMap::new(),
            generations: HashMap::new(),
            identities: HashMap::new(),
        }
    }
}

pub(crate) struct QueryCache<T> {
    pub(crate) state: Mutex<QueryCacheState<T>>,
    pub(crate) ready: Condvar,
}

impl<T> Default for QueryCache<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(QueryCacheState::default()),
            ready: Condvar::new(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum CacheOutcome {
    Success,
    RateLimited(u64),
    Failed,
    StableFailure,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Refresh {
    Cached,
    Force,
    MaxAge(Duration),
}

impl From<bool> for Refresh {
    fn from(force: bool) -> Self {
        if force {
            Refresh::Force
        } else {
            Refresh::Cached
        }
    }
}

pub(crate) fn refresh_mode(force: bool, max_age_ms: Option<u64>) -> Refresh {
    match (force, max_age_ms) {
        (true, _) => Refresh::Force,
        (false, Some(ms)) => Refresh::MaxAge(Duration::from_millis(ms)),
        (false, None) => Refresh::Cached,
    }
}

pub(crate) fn cached_query<T: Clone>(
    cache: &QueryCache<T>,
    key: String,
    refresh: impl Into<Refresh>,
    classify: impl Fn(&T) -> CacheOutcome,
    query: impl FnOnce() -> T,
) -> T {
    let refresh = refresh.into();
    let mut waited = false;
    let generation;
    loop {
        let Ok(mut state) = cache.state.lock() else {
            return query();
        };
        if let Some((value, expires, backoff, fetched)) = state.entries.get(&key) {
            let wants_fresh = match refresh {
                Refresh::Cached => false,
                Refresh::Force => true,
                Refresh::MaxAge(age) => fetched.elapsed() >= age,
            };
            let backoff_applies = match backoff {
                CacheBackoff::Failed => refresh != Refresh::Force,
                CacheBackoff::RateLimited => true,
                CacheBackoff::None => false,
            };
            if waited || (*expires > Instant::now() && (!wants_fresh || backoff_applies)) {
                return value.clone();
            }
        }
        if state.in_flight.insert(key.clone()) {
            generation = state.generations.get(&key).copied().unwrap_or(0);
            break;
        }
        let Ok(next) = cache.ready.wait(state) else {
            return query();
        };
        drop(next);
        waited = true;
    }

    let value = query();
    if let Ok(mut state) = cache.state.lock() {
        state.in_flight.remove(&key);
        let outcome = classify(&value);
        let failures = match outcome {
            CacheOutcome::Failed => {
                let next = state
                    .failures
                    .get(&key)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(1);
                state.failures.insert(key.clone(), next);
                next
            }
            _ => {
                state.failures.remove(&key);
                0
            }
        };
        let (ttl, backoff) = match outcome {
            CacheOutcome::Success => (Duration::from_secs(5 * 60), CacheBackoff::None),
            CacheOutcome::RateLimited(seconds) => {
                (Duration::from_secs(seconds.clamp(1, 30 * 60)), CacheBackoff::RateLimited)
            }
            CacheOutcome::Failed => (
                Duration::from_secs((15u64.saturating_mul(1u64 << failures.min(4))).min(5 * 60)),
                CacheBackoff::Failed,
            ),
            CacheOutcome::StableFailure => (Duration::from_secs(60), CacheBackoff::None),
        };
        if state.generations.get(&key).copied().unwrap_or(0) == generation {
            let now = Instant::now();
            state
                .entries
                .insert(key, (value.clone(), now + ttl, backoff, now));
        }
        cache.ready.notify_all();
    }
    value
}

fn retry_after_seconds(reason: &str) -> u64 {
    reason
        .split("请等待 ")
        .nth(1)
        .and_then(|tail| tail.split(' ').next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(60)
}

pub(crate) fn cached_native_quota(
    cache: &QueryCache<crate::quota::QuotaState>,
    key: String,
    refresh: Refresh,
    identity: impl Fn() -> Result<String, crate::quota::QuotaState>,
    query: impl FnOnce() -> crate::quota::QuotaState,
) -> crate::quota::QuotaState {
    let before = match identity() {
        Ok(identity) => identity,
        Err(state) => {
            bind_native_identity(cache, &key, None);
            return state;
        }
    };
    bind_native_identity(cache, &key, Some(&before));
    let state = cached_query(cache, key.clone(), refresh, quota_cache_outcome, query);
    match identity() {
        Ok(after) if after == before => state,
        Ok(after) => {
            bind_native_identity(cache, &key, Some(&after));
            crate::quota::QuotaState::Failed {
                reason: "原工具登录在查询期间发生变化，请刷新账号数据".into(),
            }
        }
        Err(state) => {
            bind_native_identity(cache, &key, None);
            state
        }
    }
}

pub(crate) fn bind_native_identity(
    cache: &QueryCache<crate::quota::QuotaState>,
    key: &str,
    identity: Option<&str>,
) {
    if let Ok(mut state) = cache.state.lock() {
        if identity.is_some() && state.identities.get(key).map(String::as_str) == identity {
            return;
        }
        match identity {
            Some(identity) => {
                state.identities.insert(key.into(), identity.into());
            }
            None => {
                state.identities.remove(key);
            }
        }
        state.entries.remove(key);
        state.failures.remove(key);
        let next = state.generations.get(key).copied().unwrap_or(0).wrapping_add(1);
        state.generations.insert(key.into(), next);
    }
}

pub(crate) fn current_for_identity<T: Clone>(
    cache: &QueryCache<T>,
    key: &str,
    identity: impl FnOnce() -> Result<String, crate::quota::QuotaState>,
) -> Option<(T, Instant, CacheBackoff, Instant)> {
    let current = identity();
    let state = cache.state.lock().ok()?;
    let entry = state.entries.get(key)?.clone();
    match (state.identities.get(key), current) {
        (None, _) => Some(entry),
        (Some(bound), Ok(current)) if *bound == current => Some(entry),
        _ => None,
    }
}

pub(crate) fn cached_value<T: Clone>(
    cache: &QueryCache<T>,
    key: &str,
) -> Option<(T, Instant, CacheBackoff, Instant)> {
    cache.state.lock().ok()?.entries.get(key).cloned()
}

pub(crate) fn quota_cache_outcome(value: &crate::quota::QuotaState) -> CacheOutcome {
    match value {
        crate::quota::QuotaState::Ok { .. } => CacheOutcome::Success,
        crate::quota::QuotaState::RateLimited { reason } => {
            CacheOutcome::RateLimited(retry_after_seconds(reason))
        }
        crate::quota::QuotaState::Failed { .. } => CacheOutcome::Failed,
        _ => CacheOutcome::StableFailure,
    }
}

pub(crate) fn balance_cache_outcome(value: &crate::quota::BalanceState) -> CacheOutcome {
    match value {
        crate::quota::BalanceState::Ok { .. } => CacheOutcome::Success,
        crate::quota::BalanceState::RateLimited { reason } => {
            CacheOutcome::RateLimited(retry_after_seconds(reason))
        }
        crate::quota::BalanceState::Failed { .. } => CacheOutcome::Failed,
        _ => CacheOutcome::StableFailure,
    }
}

pub(crate) fn api_usage_cache_outcome(value: &crate::quota::ApiUsageState) -> CacheOutcome {
    match value {
        crate::quota::ApiUsageState::Ok { .. } => CacheOutcome::Success,
        crate::quota::ApiUsageState::RateLimited { reason } => {
            CacheOutcome::RateLimited(retry_after_seconds(reason))
        }
        crate::quota::ApiUsageState::Failed { .. } => CacheOutcome::Failed,
        _ => CacheOutcome::StableFailure,
    }
}

pub(crate) static QUOTA_CACHE: LazyLock<QueryCache<crate::quota::QuotaState>> =
    LazyLock::new(QueryCache::default);
pub(crate) static BALANCE_CACHE: LazyLock<QueryCache<crate::quota::BalanceState>> =
    LazyLock::new(QueryCache::default);
pub(crate) static API_USAGE_CACHE: LazyLock<QueryCache<crate::quota::ApiUsageState>> =
    LazyLock::new(QueryCache::default);

pub(crate) fn invalidate_network_cache(id: &str) {
    fn invalidate<T>(cache: &QueryCache<T>, id: &str) {
        if let Ok(mut state) = cache.state.lock() {
            state.entries.remove(id);
            state.failures.remove(id);
            state.identities.remove(id);
            let next = state.generations.get(id).copied().unwrap_or(0).wrapping_add(1);
            state.generations.insert(id.to_string(), next);
        }
    }
    invalidate(&QUOTA_CACHE, id);
    invalidate(&BALANCE_CACHE, id);
    invalidate(&API_USAGE_CACHE, id);
}

pub(crate) fn data_paths(app: &tauri::AppHandle) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let directory = match crate::validation::data_dir() {
        Some(path) => path,
        None => app.path().app_data_dir().map_err(|error| error.to_string())?,
    };
    Ok((directory.clone(), directory.join("usage.db")))
}

pub(crate) fn retention_cutoff(days: u32) -> Result<i64, String> {
    if !matches!(days, 30 | 90 | 180 | 365) {
        return Err("历史保留天数必须是 30、90、180 或 365".into());
    }
    Ok(chrono::Utc::now().timestamp_millis() - i64::from(days) * 86_400_000)
}

/// 第一次真的遮住值：发一次系统通知并记下（之后不再提醒）。
pub(crate) fn privacy_first_notice(app: &tauri::AppHandle) {
    let cfg = &app.state::<Cfg>().0;
    if cfg.get().privacy_first_noticed {
        return;
    }
    if cfg.try_update(|settings| settings.privacy_first_noticed = true).is_ok() {
        crate::events::settings_changed(app);
    }
    let _ = app.emit(crate::events::PRIVACY_FIRST_MASK, ());
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title("隐私脱敏已生效")
        .body("Jolu 第一次在请求离开本机前把敏感值换成了占位符，回复里会自动换回。可在设置 › 隐私脱敏查看记录。")
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::quotas::{
        cached_zcode_connection_quota, validate_query_range, ZCODE_CONNECTION_CACHE_KEY,
    };
    use crate::quota;
    use std::{
        future::Future,
        sync::{atomic::AtomicUsize, mpsc},
        task::{Context, Poll, Wake, Waker},
    };

    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    #[test]
    fn database_contention_yields_without_blocking_the_caller() {
        let db = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let busy_db = db.clone();
        let collector = std::thread::spawn(move || {
            let _guard = busy_db.lock().unwrap();
            ready_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
        });
        ready_rx.recv().unwrap();

        let caller = std::thread::current().id();
        let mut query = Box::pin(with_database(db, move |conn| {
            assert_ne!(std::thread::current().id(), caller);
            conn.query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
                .map_err(|e| e.to_string())
        }));
        let started = Instant::now();
        let waker = Waker::from(Arc::new(NoopWake));
        let first_poll = query.as_mut().poll(&mut Context::from_waker(&waker));
        let elapsed = started.elapsed();
        let _ = release_tx.send(());
        collector.join().unwrap();

        assert!(matches!(first_poll, Poll::Pending));
        assert!(elapsed < Duration::from_millis(250), "查询阻塞调用线程: {elapsed:?}");
        assert_eq!(tauri::async_runtime::block_on(query).unwrap(), 42);
    }

    #[test]
    fn read_queries_do_not_wait_for_the_collector_write_lock() {
        let dir = std::env::temp_dir().join(format!("jolu-read-pool-{}", std::process::id()));
        let path = dir.join("usage.db");
        let writer = Arc::new(Mutex::new(crate::db::open(&path).unwrap()));
        let pool = ReadPool::open(&path).unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let busy = writer.clone();
        let collector = std::thread::spawn(move || {
            let _guard = busy.lock().unwrap();
            ready_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
        });
        ready_rx.recv().unwrap();
        let started = Instant::now();
        let count: i64 = pool
            .with(|conn| conn.query_row("SELECT COUNT(*) FROM requests", [], |row| row.get(0)))
            .unwrap();
        let elapsed = started.elapsed();
        let _ = release_tx.send(());
        collector.join().unwrap();
        assert_eq!(count, 0);
        assert!(elapsed < Duration::from_millis(250), "只读查询被写锁阻塞: {elapsed:?}");
        assert!(pool.with(|conn| conn.execute("DELETE FROM requests", [])).is_err());
        drop(pool);
        drop(writer);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reversed_custom_ranges_are_rejected_before_database_work() {
        let range = crate::db::CustomRange {
            start: 2_000,
            end: Some(1_000),
        };
        assert_eq!(
            validate_query_range(Some(range), 3_000).unwrap_err(),
            "开始时间必须早于结束时间",
        );
        assert!(validate_query_range(
            Some(crate::db::CustomRange {
                start: 1_000,
                end: None
            }),
            2_000
        )
        .is_ok());
    }

    #[test]
    fn query_cache_deduplicates_concurrent_requests_and_allows_forced_success_refresh() {
        let cache = Arc::new(QueryCache::<usize>::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for _ in 0..4 {
            let cache = cache.clone();
            let calls = calls.clone();
            workers.push(std::thread::spawn(move || {
                cached_query(
                    &cache,
                    "same".into(),
                    false,
                    |_| CacheOutcome::Success,
                    || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(30));
                        7
                    },
                )
            }));
        }
        assert!(workers.into_iter().all(|worker| worker.join().unwrap() == 7));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        assert_eq!(
            cached_query(
                &cache,
                "same".into(),
                true,
                |_| CacheOutcome::Success,
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    8
                },
            ),
            8
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn periodic_refresh_reuses_a_recent_fetch_but_not_an_older_one() {
        let cache = QueryCache::<usize>::default();
        let calls = AtomicUsize::new(0);
        let fetch = || {
            calls.fetch_add(1, Ordering::SeqCst);
            1
        };
        cached_query(&cache, "k".into(), false, |_| CacheOutcome::Success, fetch);
        cached_query(
            &cache,
            "k".into(),
            refresh_mode(false, Some(60_000)),
            |_| CacheOutcome::Success,
            fetch,
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        std::thread::sleep(Duration::from_millis(20));
        cached_query(
            &cache,
            "k".into(),
            refresh_mode(false, Some(10)),
            |_| CacheOutcome::Success,
            fetch,
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(refresh_mode(true, Some(10)), Refresh::Force);
    }

    #[test]
    fn forced_refresh_respects_rate_limit_backoff() {
        let cache = QueryCache::<usize>::default();
        let calls = AtomicUsize::new(0);
        let first = cached_query(
            &cache,
            "limited".into(),
            false,
            |_| CacheOutcome::RateLimited(60),
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                429
            },
        );
        let forced = cached_query(
            &cache,
            "limited".into(),
            true,
            |_| CacheOutcome::Success,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                200
            },
        );
        assert_eq!((first, forced), (429, 429));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn native_quota_cache_tracks_login_changes_and_logout() {
        let cache = QueryCache::default();
        let login = Mutex::new(Some("account-a".to_string()));
        let calls = AtomicUsize::new(0);
        let identity = || {
            login
                .lock()
                .unwrap()
                .clone()
                .ok_or(quota::QuotaState::Unauthorized {
                    reason: "已退出登录".into(),
                })
        };
        let query = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok {
                windows: vec![],
                plan: login.lock().unwrap().clone(),
                account: None,
            }
        };
        for expected in ["account-a", "account-a", "account-b"] {
            *login.lock().unwrap() = Some(expected.into());
            let state =
                cached_native_quota(&cache, "native-test".into(), Refresh::Cached, identity, query);
            assert!(matches!(state, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == expected));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        *login.lock().unwrap() = None;
        assert!(matches!(
            cached_native_quota(&cache, "native-test".into(), Refresh::Cached, identity, query),
            quota::QuotaState::Unauthorized { .. }
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn native_quota_cache_discards_login_switch_during_request() {
        let cache = QueryCache::default();
        let login = Mutex::new("before".to_string());
        let state = cached_native_quota(
            &cache,
            "native-test".into(),
            Refresh::Cached,
            || Ok(login.lock().unwrap().clone()),
            || {
                *login.lock().unwrap() = "after".into();
                quota::QuotaState::Ok {
                    windows: vec![],
                    plan: Some("before".into()),
                    account: None,
                }
            },
        );
        assert!(matches!(state, quota::QuotaState::Failed { .. }));
        assert!(!cache.state.lock().unwrap().entries.contains_key("native-test"));
    }

    #[test]
    fn native_quota_cache_never_reuses_previous_identity_result() {
        let cache = QueryCache::default();
        let identity_now = Mutex::new("identity-a".to_string());
        let calls = AtomicUsize::new(0);
        let identity = || Ok(identity_now.lock().unwrap().clone());
        let query = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok {
                windows: vec![],
                plan: identity_now.lock().unwrap().clone().into(),
                account: None,
            }
        };
        let first =
            cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached, identity, query);
        assert!(matches!(first, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "identity-a"));
        *identity_now.lock().unwrap() = "identity-b".into();
        let second =
            cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached, identity, query);
        assert!(matches!(second, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "identity-b"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn native_quota_cache_hits_for_the_same_identity_without_a_second_query() {
        let cache = QueryCache::default();
        let calls = AtomicUsize::new(0);
        let identity = || Ok("identity-a".to_string());
        let query = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok {
                windows: vec![],
                plan: Some("identity-a".into()),
                account: None,
            }
        };
        for _ in 0..3 {
            let state =
                cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached, identity, query);
            assert!(matches!(state, quota::QuotaState::Ok { .. }));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let forced =
            cached_native_quota(&cache, "local-zcode".into(), Refresh::Force, identity, query);
        assert!(matches!(forced, quota::QuotaState::Ok { .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn identity_switch_clears_cached_failure_and_backoff() {
        let cache = QueryCache::default();
        let identity_now = Mutex::new("identity-a".to_string());
        let calls = AtomicUsize::new(0);
        let identity = || Ok(identity_now.lock().unwrap().clone());
        let failed = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Failed {
                reason: "网络失败".into(),
            }
        };
        let first =
            cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached, identity, failed);
        assert!(matches!(first, quota::QuotaState::Failed { .. }));
        assert_eq!(cache.state.lock().unwrap().failures.get("local-zcode").copied(), Some(1));
        *identity_now.lock().unwrap() = "identity-b".into();
        let second =
            cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached, identity, failed);
        assert!(matches!(second, quota::QuotaState::Failed { .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(cache.state.lock().unwrap().failures.get("local-zcode").copied(), Some(1));
    }

    #[test]
    fn display_read_requires_the_identity_bound_at_write_time() {
        let cache = QueryCache::<quota::QuotaState>::default();
        let calls = AtomicUsize::new(0);
        let state = cached_native_quota(
            &cache,
            "local-zcode".into(),
            Refresh::Cached,
            || Ok("identity-a".into()),
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                quota::QuotaState::Ok {
                    windows: vec![],
                    plan: Some("identity-a".into()),
                    account: None,
                }
            },
        );
        assert!(matches!(state, quota::QuotaState::Ok { .. }));
        let same = current_for_identity(&cache, "local-zcode", || Ok("identity-a".into()));
        assert!(matches!(same, Some((quota::QuotaState::Ok { .. }, ..))));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(current_for_identity(&cache, "local-zcode", || Ok("identity-b".into())).is_none());
        let state = cache.state.lock().unwrap();
        assert!(state.entries.contains_key("local-zcode"));
        assert_eq!(state.identities.get("local-zcode").map(String::as_str), Some("identity-a"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn display_read_treats_unavailable_identity_as_no_cache() {
        let cache = QueryCache::<quota::QuotaState>::default();
        cached_native_quota(
            &cache,
            "local-zcode".into(),
            Refresh::Cached,
            || Ok("identity-a".into()),
            || quota::QuotaState::Ok {
                windows: vec![],
                plan: None,
                account: None,
            },
        );
        assert!(current_for_identity(&cache, "local-zcode", || {
            Err(quota::QuotaState::Unauthorized {
                reason: "已退出登录".into(),
            })
        })
        .is_none());
        assert!(cached_value(&cache, "local-zcode").is_some());
        assert!(cached_value(&cache, "local-codex").is_none());
    }

    #[test]
    fn zcode_connection_cache_key_is_stable_and_separate_from_the_local_command() {
        assert_eq!(ZCODE_CONNECTION_CACHE_KEY, "zcode-connection:local-login");
        assert_ne!(ZCODE_CONNECTION_CACHE_KEY, "local-zcode");
        assert!(!ZCODE_CONNECTION_CACHE_KEY.starts_with("monitor:"));
        assert!(!ZCODE_CONNECTION_CACHE_KEY.starts_with("native:"));
    }

    #[test]
    fn zcode_connection_quota_hits_cache_for_the_same_identity() {
        let cache = QueryCache::default();
        let calls = AtomicUsize::new(0);
        let identity = || Ok("zcode-identity-a".to_string());
        let query = || {
            let nth = calls.fetch_add(1, Ordering::SeqCst) + 1;
            quota::QuotaState::Ok {
                windows: vec![],
                plan: Some(format!("nth-{nth}")),
                account: None,
            }
        };
        let first = cached_zcode_connection_quota(&cache, Refresh::Cached, identity, query);
        assert!(matches!(first, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "nth-1"));
        let second = cached_zcode_connection_quota(&cache, Refresh::Cached, identity, query);
        assert!(matches!(second, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "nth-1"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let state = cache.state.lock().unwrap();
        assert!(state.entries.contains_key(ZCODE_CONNECTION_CACHE_KEY));
        assert!(!state.entries.contains_key("local-zcode"));
    }

    #[test]
    fn zcode_connection_quota_force_still_refetches() {
        let cache = QueryCache::default();
        let calls = AtomicUsize::new(0);
        let identity = || Ok("zcode-identity-a".to_string());
        let query = || {
            let nth = calls.fetch_add(1, Ordering::SeqCst) + 1;
            quota::QuotaState::Ok {
                windows: vec![],
                plan: Some(format!("nth-{nth}")),
                account: None,
            }
        };
        cached_zcode_connection_quota(&cache, Refresh::Cached, identity, query);
        assert_eq!(refresh_mode(true, Some(60_000)), Refresh::Force);
        let forced =
            cached_zcode_connection_quota(&cache, refresh_mode(true, None), identity, query);
        assert!(matches!(forced, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "nth-2"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn zcode_connection_quota_failure_backoff_blocks_automatic_refresh_but_not_force() {
        let cache = QueryCache::default();
        let calls = AtomicUsize::new(0);
        let identity = || Ok("zcode-identity-a".to_string());
        let failed = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Failed {
                reason: "网络失败".into(),
            }
        };
        assert!(matches!(
            cached_zcode_connection_quota(&cache, Refresh::Cached, identity, failed),
            quota::QuotaState::Failed { .. }
        ));
        assert_eq!(
            cache.state.lock().unwrap().failures.get(ZCODE_CONNECTION_CACHE_KEY).copied(),
            Some(1)
        );
        let settled = cached_zcode_connection_quota(&cache, Refresh::Cached, identity, || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok {
                windows: vec![],
                plan: None,
                account: None,
            }
        });
        assert!(matches!(settled, quota::QuotaState::Failed { .. }));
        let periodic =
            cached_zcode_connection_quota(&cache, refresh_mode(false, Some(60_000)), identity, || {
                calls.fetch_add(1, Ordering::SeqCst);
                quota::QuotaState::Ok {
                    windows: vec![],
                    plan: None,
                    account: None,
                }
            });
        assert!(matches!(periodic, quota::QuotaState::Failed { .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let forced = cached_zcode_connection_quota(&cache, Refresh::Force, identity, || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok {
                windows: vec![],
                plan: Some("after-force".into()),
                account: None,
            }
        });
        assert!(matches!(forced, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "after-force"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn zcode_connection_quota_returns_rate_limited_without_sending_a_request() {
        let cache = QueryCache::default();
        let calls = AtomicUsize::new(0);
        let identity = || Ok("zcode-identity-a".to_string());
        let limited = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::RateLimited {
                reason: "上游限流，请等待 45 秒后重试".into(),
            }
        };
        assert!(matches!(
            cached_zcode_connection_quota(&cache, Refresh::Cached, identity, limited),
            quota::QuotaState::RateLimited { .. }
        ));
        let forced = cached_zcode_connection_quota(&cache, Refresh::Force, identity, || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok {
                windows: vec![],
                plan: None,
                account: None,
            }
        });
        assert!(matches!(forced, quota::QuotaState::RateLimited { ref reason } if reason.contains("45")));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn zcode_connection_quota_never_reuses_the_previous_identity_result() {
        let cache = QueryCache::default();
        let identity_now = Mutex::new("intranet-account-a".to_string());
        let calls = AtomicUsize::new(0);
        let identity = || Ok(identity_now.lock().unwrap().clone());
        let query = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok {
                windows: vec![],
                plan: identity_now.lock().unwrap().clone().into(),
                account: None,
            }
        };
        let first = cached_zcode_connection_quota(&cache, Refresh::Cached, identity, query);
        assert!(matches!(first, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "intranet-account-a"));
        *identity_now.lock().unwrap() = "intranet-account-b".into();
        let second = cached_zcode_connection_quota(&cache, Refresh::Cached, identity, query);
        assert!(matches!(second, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "intranet-account-b"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn forced_refresh_skips_failure_backoff() {
        let cache = QueryCache::<usize>::default();
        let calls = AtomicUsize::new(0);
        let failed = cached_query(
            &cache,
            "flaky".into(),
            false,
            |_| CacheOutcome::Failed,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                503
            },
        );
        assert_eq!(failed, 503);
        cached_query(&cache, "flaky".into(), false, |_| CacheOutcome::Success, || {
            calls.fetch_add(1, Ordering::SeqCst);
            200
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let forced = cached_query(&cache, "flaky".into(), true, |_| CacheOutcome::Success, || {
            calls.fetch_add(1, Ordering::SeqCst);
            200
        });
        assert_eq!(forced, 200);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn forced_refresh_still_returns_rate_limited_without_a_request() {
        let cache = QueryCache::<usize>::default();
        let calls = AtomicUsize::new(0);
        let limited = cached_query(
            &cache,
            "limited".into(),
            false,
            |_| CacheOutcome::RateLimited(45),
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                429
            },
        );
        assert_eq!(limited, 429);
        let forced = cached_query(
            &cache,
            "limited".into(),
            Refresh::Force,
            |_| CacheOutcome::Success,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                200
            },
        );
        assert_eq!(forced, 429);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn single_scan_failure_does_not_mark_collection_broken_but_persistent_does() {
        let errors = vec!["rollout-x.jsonl: boom".to_string()];
        assert_eq!(evaluate_collection_status(0, &errors), (true, 1));
        assert_eq!(evaluate_collection_status(1, &errors), (false, 2));
        assert_eq!(evaluate_collection_status(1, &[]), (true, 0));
        assert_eq!(evaluate_collection_status(5, &errors), (false, 6));
    }
}
