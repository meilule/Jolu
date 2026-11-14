//! Jolu —— Tauri 应用入口
//!
//! 职责边界见需求文档 §7.7：
//!   Tauri 2 + Rust —— 窗口与托盘、只读网络查询、本地增量采集、去重、
//!                     统计计算与数据库查询；集中管理必要后台任务
//!
//! 包含双窗口、托盘、本地会话采集、SQLite 统计与连接只读查询。

mod opencode;
mod opencode_usage;
mod platforms;
mod quota_registry;
mod toast_link;
mod custom_agents;
mod provider_key;
mod source_store;
mod native_parse;
mod native_sources;
mod collector;
#[cfg(all(debug_assertions, feature = "handoff-eval"))]
pub mod handoff_eval;
mod context_menu;
mod tray_summary;
mod session_presence;
mod startup;
mod tray_icon;
mod connections;
mod creds;
mod cli_switch;
// 连接归因：把请求指到已存连接上（exact / mapped / platform 三档），见模块头注释
mod attribution;
// 连接目录自动回填：把 ZCode provider 规则的 baseUrl / 模型补进已存连接
mod catalog_backfill;
pub mod db;
mod data_files;
mod dock;
mod island_clones;
mod island_pointer;
mod main_window;
mod pricing;
mod proxy;
mod proxy_config;
pub mod gateway;
mod fx;
mod quota;
mod checkin_types;
mod checkin_stats;
mod checkin_workbuddy;
mod checkin_qoder;
mod checkin_trae;
mod daily_checkin;
mod coding_plan;
mod grok_quota;
mod zcode_quota;
mod gemini_quota;
mod gateway_balance;
mod service_balance;
mod trae_quota;
mod workbuddy_quota;
mod qoder_quota;
mod qoder_desktop;
mod provider_browser_auth;
mod workbuddy_browser_auth;
mod trae_browser_auth;
mod grok_browser_auth;
mod zcode_browser_auth;
mod cli_browser_auth;
mod agent_version;
mod agent_discovery;
mod keep_awake;
mod wsl_probe;
mod otlp;
mod warmup;
mod warmup_run;
mod settings;
mod steady;
mod zcode_version;
mod session_titles;
mod watcher;
mod validation;
mod updater;
mod cli_apply;
mod auth_snapshot;
mod claude_auth;
mod current_connection;
mod login_detect;
mod usage_stats;
mod session_threads;
pub mod library;
pub mod service;
// 模块 E · 自动化
mod automation;
// 模块 F · Skills 管理
mod skills;
// 模块 G · 设置
mod storage_stats;
// 任务 2.3 开始
mod net;
// 任务 2.3 结束
// 任务 2.5 开始
mod notify;
// 任务 2.5 结束
#[cfg(test)]
mod perf_bench;
// 任务 3.1 开始
mod handoff_baseline;
mod handoff_digest;
mod handoff_facts;
mod handoff_input;
mod session_handoff;
// 任务 3.1 结束

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{
    Emitter,
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, PhysicalPosition, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_opener::OpenerExt;

#[derive(serde::Serialize)]
struct DataInfo {
    path: String,
    size_bytes: u64,
}

#[derive(Clone, serde::Serialize)]
struct SourceInfo {
    id: String,
    platform: String,
    name: String,
    available: bool,
    capabilities: Vec<String>,
    reason: Option<String>,
}

fn local_source(platform: &str) -> Option<SourceInfo> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from);
    let path = match platform {
        "claude" => home.map(|path| path.join(".claude").join("projects")),
        "codex" => session_titles::codex_home().map(|path| path.join("sessions")),
        _ => return None,
    };
    let available = path.as_ref().is_some_and(|path| path.is_dir());
    Some(SourceInfo {
        id: format!("local:{platform}"),
        platform: platform.into(),
        name: format!("本机 {} 会话记录", platform_name(platform)),
        available,
        capabilities: vec!["token_totals".into(), "request_log".into(), "realtime_delta".into()],
        reason: (!available).then(|| format!("未检测到本机 {} 会话目录", platform_name(platform))),
    })
}

/// 全局写连接。采集、导入、连接管理等写入共用同一个后台，不为每个视图重复采集（§7.7）
pub struct Db(pub Arc<Mutex<rusqlite::Connection>>);

/// 只读查询连接池：统计、日志与实时快照走这里。WAL 允许读写并发，
/// 查询不再排在采集写入后面，采集也不必等界面查询。
pub struct DbRead(pub Arc<ReadPool>);

pub struct ReadPool {
    conns: Vec<Mutex<rusqlite::Connection>>,
    next: std::sync::atomic::AtomicUsize,
}

impl ReadPool {
    /// 主面板一次刷新会并发发出多条查询，灵动岛与托盘还各有一条；三条连接足够不排队。
    const SIZE: usize = 3;

    fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        let conns = (0..Self::SIZE)
            .map(|_| db::open_reader(path).map(Mutex::new))
            .collect::<rusqlite::Result<_>>()?;
        Ok(Self { conns, next: std::sync::atomic::AtomicUsize::new(0) })
    }

    /// 优先取空闲连接；都在忙时轮转排队，绝不退回写连接。
    pub fn with<T>(&self, work: impl FnOnce(&rusqlite::Connection) -> T) -> T {
        for conn in &self.conns {
            if let Ok(guard) = conn.try_lock() {
                return work(&guard);
            }
        }
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.conns.len();
        let guard = self.conns[index].lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        work(&guard)
    }
}

/// 在阻塞线程池上借一条只读连接执行查询。
async fn with_reader<T: Send + 'static>(
    pool: Arc<ReadPool>,
    work: impl FnOnce(&rusqlite::Connection) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || pool.with(work))
        .await
        .map_err(|e| e.to_string())?
}

/// 在阻塞线程池等待数据库锁，避免扫描期间卡住窗口或异步运行时。
async fn with_database<T: Send + 'static>(
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

#[cfg(test)]
mod responsiveness_tests {
    use super::*;
    use std::{future::Future, sync::mpsc, task::{Context, Poll, Wake, Waker}, time::{Duration, Instant}};

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
            // 即使回归为同步等待，也会超时释放锁，让测试失败而非永久挂起。
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
        let writer = Arc::new(Mutex::new(db::open(&path).unwrap()));
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
        let count: i64 = pool.with(|conn| conn.query_row("SELECT COUNT(*) FROM requests", [], |row| row.get(0))).unwrap();
        let elapsed = started.elapsed();
        let _ = release_tx.send(());
        collector.join().unwrap();
        assert_eq!(count, 0);
        assert!(elapsed < Duration::from_millis(250), "只读查询被写锁阻塞: {elapsed:?}");
        assert!(pool.with(|conn| conn.execute("DELETE FROM requests", [])).is_err(), "只读连接不得写入");
        drop(pool);
        drop(writer);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reversed_custom_ranges_are_rejected_before_database_work() {
        let range = db::CustomRange { start: 2_000, end: Some(1_000) };
        assert_eq!(
            validate_query_range(Some(range), 3_000).unwrap_err(),
            "开始时间必须早于结束时间",
        );
        assert!(validate_query_range(Some(db::CustomRange { start: 1_000, end: None }), 2_000).is_ok());
    }

    #[test]
    fn query_cache_deduplicates_concurrent_requests_and_allows_forced_success_refresh() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let cache = Arc::new(QueryCache::<usize>::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for _ in 0..4 {
            let cache = cache.clone();
            let calls = calls.clone();
            workers.push(std::thread::spawn(move || cached_query(
                &cache,
                "same".into(),
                false,
                |_| CacheOutcome::Success,
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(30));
                    7
                },
            )));
        }
        assert!(workers.into_iter().all(|worker| worker.join().unwrap() == 7));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        assert_eq!(cached_query(
            &cache,
            "same".into(),
            true,
            |_| CacheOutcome::Success,
            || { calls.fetch_add(1, Ordering::SeqCst); 8 },
        ), 8);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn periodic_refresh_reuses_a_recent_fetch_but_not_an_older_one() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::<usize>::default();
        let calls = AtomicUsize::new(0);
        let fetch = || { calls.fetch_add(1, Ordering::SeqCst); 1 };
        cached_query(&cache, "k".into(), false, |_| CacheOutcome::Success, fetch);
        // 另一个窗口的定时刷新落在同一周期内：复用刚取得的结果
        cached_query(&cache, "k".into(), refresh_mode(false, Some(60_000)), |_| CacheOutcome::Success, fetch);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // 缓存已比刷新间隔旧：即使尚未过期也重新查询
        std::thread::sleep(Duration::from_millis(20));
        cached_query(&cache, "k".into(), refresh_mode(false, Some(10)), |_| CacheOutcome::Success, fetch);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(refresh_mode(true, Some(10)), Refresh::Force);
    }

    #[test]
    fn forced_refresh_respects_rate_limit_backoff() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let cache = QueryCache::<usize>::default();
        let calls = AtomicUsize::new(0);
        let first = cached_query(
            &cache,
            "limited".into(),
            false,
            |_| CacheOutcome::RateLimited(60),
            || { calls.fetch_add(1, Ordering::SeqCst); 429 },
        );
        let forced = cached_query(
            &cache,
            "limited".into(),
            true,
            |_| CacheOutcome::Success,
            || { calls.fetch_add(1, Ordering::SeqCst); 200 },
        );
        assert_eq!((first, forced), (429, 429));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn native_quota_cache_tracks_login_changes_and_logout() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::default();
        let login = Mutex::new(Some("account-a".to_string()));
        let calls = AtomicUsize::new(0);
        let identity = || login.lock().unwrap().clone().ok_or(quota::QuotaState::Unauthorized {reason:"已退出登录".into()});
        let query = || { calls.fetch_add(1, Ordering::SeqCst); quota::QuotaState::Ok { windows:vec![],plan:login.lock().unwrap().clone(),account:None } };
        for expected in ["account-a", "account-a", "account-b"] {
            *login.lock().unwrap() = Some(expected.into());
            let state = cached_native_quota(&cache,"native-test".into(),Refresh::Cached,identity,query);
            assert!(matches!(state, quota::QuotaState::Ok{plan:Some(ref plan),..} if plan == expected));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2, "同一登录去重，换登录重新查询");
        *login.lock().unwrap() = None;
        assert!(matches!(cached_native_quota(&cache,"native-test".into(),Refresh::Cached,identity,query), quota::QuotaState::Unauthorized{..}));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn native_quota_cache_discards_login_switch_during_request() {
        let cache = QueryCache::default();
        let login = Mutex::new("before".to_string());
        let state = cached_native_quota(&cache,"native-test".into(),Refresh::Cached,||Ok(login.lock().unwrap().clone()),|| {
            *login.lock().unwrap() = "after".into();
            quota::QuotaState::Ok {windows:vec![],plan:Some("before".into()),account:None}
        });
        assert!(matches!(state,quota::QuotaState::Failed{..}), "迟到数据不能交给新登录");
        assert!(!cache.state.lock().unwrap().entries.contains_key("native-test"));
    }

    #[test]
    fn native_quota_cache_never_reuses_previous_identity_result() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::default();
        let identity_now = Mutex::new("identity-a".to_string());
        let calls = AtomicUsize::new(0);
        let identity = || Ok(identity_now.lock().unwrap().clone());
        let query = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok { windows: vec![], plan: identity_now.lock().unwrap().clone().into(), account: None }
        };
        // 本机 ZCode 登录换账号：`local-zcode` 固定 key 也必须跟着身份换结果
        let first = cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached, identity, query);
        assert!(matches!(first, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "identity-a"));
        *identity_now.lock().unwrap() = "identity-b".into();
        let second = cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached, identity, query);
        assert!(matches!(second, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "identity-b"), "换身份后不能返回上一个账号的额度");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn native_quota_cache_hits_for_the_same_identity_without_a_second_query() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::default();
        let calls = AtomicUsize::new(0);
        let identity = || Ok("identity-a".to_string());
        let query = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok { windows: vec![], plan: Some("identity-a".into()), account: None }
        };
        for _ in 0..3 {
            let state = cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached, identity, query);
            assert!(matches!(state, quota::QuotaState::Ok { .. }));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "身份相同必须命中缓存，不重复打上游");
        // 身份相同但显式刷新：仍然重查
        let forced = cached_native_quota(&cache, "local-zcode".into(), Refresh::Force, identity, query);
        assert!(matches!(forced, quota::QuotaState::Ok { .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn identity_switch_clears_cached_failure_and_backoff() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::default();
        let identity_now = Mutex::new("identity-a".to_string());
        let calls = AtomicUsize::new(0);
        let identity = || Ok(identity_now.lock().unwrap().clone());
        let failed = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Failed { reason: "网络失败".into() }
        };
        let first = cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached, identity, failed);
        assert!(matches!(first, quota::QuotaState::Failed { .. }));
        assert_eq!(cache.state.lock().unwrap().failures.get("local-zcode").copied(), Some(1));
        // 换身份后：上一个账号的失败结果与退避计数都作废，新账号立刻查询
        *identity_now.lock().unwrap() = "identity-b".into();
        let second = cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached, identity, failed);
        assert!(matches!(second, quota::QuotaState::Failed { .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 2, "换身份必须重新查询，不复用上一个账号的失败结果");
        assert_eq!(cache.state.lock().unwrap().failures.get("local-zcode").copied(), Some(1), "换身份时失败计数一并重置");
    }

    /// 展示路径（托盘）读缓存前必须校验身份：换账号后到下一次额度命令之前（≤5 分钟 TTL），
    /// 托盘不能继续显示上一个账号的数字。
    #[test]
    fn display_read_requires_the_identity_bound_at_write_time() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::<quota::QuotaState>::default();
        let calls = AtomicUsize::new(0);
        let state = cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached,
            || Ok("identity-a".into()),
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                quota::QuotaState::Ok { windows: vec![], plan: Some("identity-a".into()), account: None }
            });
        assert!(matches!(state, quota::QuotaState::Ok { .. }));

        // 身份相同：展示路径照常读到缓存，且不触发查询
        let same = current_for_identity(&cache, "local-zcode", || Ok("identity-a".into()));
        assert!(matches!(same, Some((quota::QuotaState::Ok { .. }, ..))));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // 当前身份换成 B，但额度命令还没跑：展示路径按「无缓存」返回，不能给出 A 的数字
        assert!(current_for_identity(&cache, "local-zcode", || Ok("identity-b".into())).is_none());

        // 只读：条目与身份绑定都还在，等下一次额度命令自己清理并重查
        let state = cache.state.lock().unwrap();
        assert!(state.entries.contains_key("local-zcode"), "展示路径只读，不得删条目");
        assert_eq!(state.identities.get("local-zcode").map(String::as_str), Some("identity-a"));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "展示路径不得触发网络查询");
    }

    /// 身份不可得（未登录 / 已注销）同样按无缓存处理；没有身份记录的键没有基准，沿用原口径。
    #[test]
    fn display_read_treats_unavailable_identity_as_no_cache() {
        let cache = QueryCache::<quota::QuotaState>::default();
        cached_native_quota(&cache, "local-zcode".into(), Refresh::Cached,
            || Ok("identity-a".into()),
            || quota::QuotaState::Ok { windows: vec![], plan: None, account: None });
        assert!(current_for_identity(&cache, "local-zcode",
            || Err(quota::QuotaState::Unauthorized { reason: "已退出登录".into() })).is_none());
        // 未绑定身份的键（写入走裸 `cached_query`）：沿用原口径读取，不伪造校验
        assert!(cached_value(&cache, "local-zcode").is_some());
        assert!(cached_value(&cache, "local-codex").is_none());
    }

    /* -------- 普通 zcode auth 连接（`connection_quota` 那一支）的缓存与退避 -------- */

    /// 上限：同一条路径的缓存键只能是这一个常量形态，不拼连接 id、不拼账号标识。
    #[test]
    fn zcode_connection_cache_key_is_stable_and_separate_from_the_local_command() {
        assert_eq!(ZCODE_CONNECTION_CACHE_KEY, "zcode-connection:local-login");
        assert_ne!(ZCODE_CONNECTION_CACHE_KEY, "local-zcode", "两支查询口径不同，不能共用条目");
        // 键里不能出现会随连接行或账号变化的部分：连接 id 以 `monitor:` 开头，账号标识是十六进制摘要
        assert!(!ZCODE_CONNECTION_CACHE_KEY.starts_with("monitor:"), "不拼连接 id");
        assert!(!ZCODE_CONNECTION_CACHE_KEY.starts_with("native:"), "不拼来源 id");
    }

    /// 1. 同一身份连续两次调用：第二次命中缓存，不发第二次上游。
    #[test]
    fn zcode_connection_quota_hits_cache_for_the_same_identity() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::default();
        let calls = AtomicUsize::new(0);
        let identity = || Ok("zcode-identity-a".to_string());
        let query = || {
            let nth = calls.fetch_add(1, Ordering::SeqCst) + 1;
            quota::QuotaState::Ok { windows: vec![], plan: Some(format!("nth-{nth}")), account: None }
        };
        let first = cached_zcode_connection_quota(&cache, Refresh::Cached, identity, query);
        assert!(matches!(first, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "nth-1"));
        let second = cached_zcode_connection_quota(&cache, Refresh::Cached, identity, query);
        assert!(matches!(second, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "nth-1"), "第二次必须命中缓存");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "同一身份不得发第二次上游请求");
        let state = cache.state.lock().unwrap();
        assert!(state.entries.contains_key(ZCODE_CONNECTION_CACHE_KEY), "条目落在固定键上");
        assert!(!state.entries.contains_key("local-zcode"), "不得写进本机套餐命令的键");
    }

    /// 2. `Force`（用户点「立即刷新」）仍然重新查询。
    #[test]
    fn zcode_connection_quota_force_still_refetches() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::default();
        let calls = AtomicUsize::new(0);
        let identity = || Ok("zcode-identity-a".to_string());
        let query = || {
            let nth = calls.fetch_add(1, Ordering::SeqCst) + 1;
            quota::QuotaState::Ok { windows: vec![], plan: Some(format!("nth-{nth}")), account: None }
        };
        cached_zcode_connection_quota(&cache, Refresh::Cached, identity, query);
        // 入参 force = true 在 `connection_quota` 里被 `refresh_mode` 转成 Force（显式刷新优先于 max_age）
        assert_eq!(refresh_mode(true, Some(60_000)), Refresh::Force);
        let forced = cached_zcode_connection_quota(&cache, refresh_mode(true, None), identity, query);
        assert!(matches!(forced, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "nth-2"), "Force 必须重新查询");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// 3. 失败退避期内：非 Force 命中退避（不发请求），Force 仍然查询。
    #[test]
    fn zcode_connection_quota_failure_backoff_blocks_automatic_refresh_but_not_force() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::default();
        let calls = AtomicUsize::new(0);
        let identity = || Ok("zcode-identity-a".to_string());
        let failed = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Failed { reason: "网络失败".into() }
        };
        assert!(matches!(
            cached_zcode_connection_quota(&cache, Refresh::Cached, identity, failed),
            quota::QuotaState::Failed { .. }
        ));
        assert_eq!(cache.state.lock().unwrap().failures.get(ZCODE_CONNECTION_CACHE_KEY).copied(), Some(1));
        // 自动刷新与定时刷新（MaxAge）在退避期内都复用失败结果，不再打上游
        let settled = cached_zcode_connection_quota(&cache, Refresh::Cached, identity, || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok { windows: vec![], plan: None, account: None }
        });
        assert!(matches!(settled, quota::QuotaState::Failed { .. }), "退避期内不能伪装成成功");
        let periodic = cached_zcode_connection_quota(&cache, refresh_mode(false, Some(60_000)), identity, || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok { windows: vec![], plan: None, account: None }
        });
        assert!(matches!(periodic, quota::QuotaState::Failed { .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "退避期内不得发请求");
        // Force 跳过失败退避：真的重查
        let forced = cached_zcode_connection_quota(&cache, Refresh::Force, identity, || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok { windows: vec![], plan: Some("after-force".into()), account: None }
        });
        assert!(matches!(forced, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "after-force"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// 4. 限流态：返回 RateLimited 且不发请求（连 Force 也挡住，避免连点打爆上游）。
    #[test]
    fn zcode_connection_quota_returns_rate_limited_without_sending_a_request() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::default();
        let calls = AtomicUsize::new(0);
        let identity = || Ok("zcode-identity-a".to_string());
        let limited = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::RateLimited { reason: "上游限流，请等待 45 秒后重试".into() }
        };
        assert!(matches!(
            cached_zcode_connection_quota(&cache, Refresh::Cached, identity, limited),
            quota::QuotaState::RateLimited { .. }
        ));
        let forced = cached_zcode_connection_quota(&cache, Refresh::Force, identity, || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok { windows: vec![], plan: None, account: None }
        });
        assert!(matches!(forced, quota::QuotaState::RateLimited { ref reason } if reason.contains("45")), "限流原样返回，不伪造成功");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "限流退避期内不得发请求");
    }

    /// 5. 身份变化：立即重新查询，不返回上一个账号的结果。
    #[test]
    fn zcode_connection_quota_never_reuses_the_previous_identity_result() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::default();
        let identity_now = Mutex::new("intranet-account-a".to_string());
        let calls = AtomicUsize::new(0);
        let identity = || Ok(identity_now.lock().unwrap().clone());
        let query = || {
            calls.fetch_add(1, Ordering::SeqCst);
            quota::QuotaState::Ok { windows: vec![], plan: identity_now.lock().unwrap().clone().into(), account: None }
        };
        let first = cached_zcode_connection_quota(&cache, Refresh::Cached, identity, query);
        assert!(matches!(first, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "intranet-account-a"));
        *identity_now.lock().unwrap() = "intranet-account-b".into();
        let second = cached_zcode_connection_quota(&cache, Refresh::Cached, identity, query);
        assert!(matches!(second, quota::QuotaState::Ok { plan: Some(ref plan), .. } if plan == "intranet-account-b"), "换身份后不能返回上一个账号的额度");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn forced_refresh_skips_failure_backoff() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::<usize>::default();
        let calls = AtomicUsize::new(0);
        let failed = cached_query(
            &cache,
            "flaky".into(),
            false,
            |_| CacheOutcome::Failed,
            || { calls.fetch_add(1, Ordering::SeqCst); 503 },
        );
        assert_eq!(failed, 503);
        // 失败退避期内：非显式刷新复用缓存结果
        cached_query(&cache, "flaky".into(), false, |_| CacheOutcome::Success, || { calls.fetch_add(1, Ordering::SeqCst); 200 });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // 显式刷新：跳过失败退避，真的重查
        let forced = cached_query(&cache, "flaky".into(), true, |_| CacheOutcome::Success, || { calls.fetch_add(1, Ordering::SeqCst); 200 });
        assert_eq!(forced, 200);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn forced_refresh_still_returns_rate_limited_without_a_request() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = QueryCache::<usize>::default();
        let calls = AtomicUsize::new(0);
        let limited = cached_query(
            &cache,
            "limited".into(),
            false,
            |_| CacheOutcome::RateLimited(45),
            || { calls.fetch_add(1, Ordering::SeqCst); 429 },
        );
        assert_eq!(limited, 429);
        // 限流退避对显式刷新同样生效，且原样返回限流结果，不伪造成功
        let forced = cached_query(&cache, "limited".into(), Refresh::Force, |_| CacheOutcome::Success, || { calls.fetch_add(1, Ordering::SeqCst); 200 });
        assert_eq!(forced, 429);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn single_scan_failure_does_not_mark_collection_broken_but_persistent_does() {
        let errors = vec!["rollout-x.jsonl: boom".to_string()];
        // 首次失败：可能是写入竞争的瞬时错误，不置异常
        assert_eq!(evaluate_collection_status(0, &errors), (true, 1));
        // 连续第二次失败：确认采集异常
        assert_eq!(evaluate_collection_status(1, &errors), (false, 2));
        // 期间任一轮成功：计数清零、恢复正常
        assert_eq!(evaluate_collection_status(1, &[]), (true, 0));
        // 长期失败保持异常，计数继续累计供诊断
        assert_eq!(evaluate_collection_status(5, &errors), (false, 6));
    }
}

/// 持久化设置。托盘与设置界面共享同一份值（§2.6 / §7.3）
pub struct Cfg(pub settings::Store);
static TRAY_REFRESHING: AtomicBool = AtomicBool::new(false);
#[derive(Clone, serde::Serialize)]
pub(crate) struct CollectionStatus {
    ok: bool,
    errors: Vec<String>,
    last_success_ms: Option<i64>,
    /// 本轮扫描中出错文件所属的平台。采集异常按平台定界：
    /// 只有出错平台的会话状态不可判定，其他平台照常显示真实状态。
    failed_sources: Vec<String>,
    /// 连续失败轮数。单次瞬时错误（如 CLI 正在写入时的读取竞争）在下一轮心跳
    /// 就会自愈，不足以把会话打成「状态未知」；连续达到阈值才认定采集异常。
    consecutive_failures: u32,
}

/// 连续失败达到该轮数才置「采集异常」。心跳 10 秒一轮，即约 20 秒的确认窗口：
/// 真·持续故障很快上报，单次抖动不再污染所有会话的状态展示。
const COLLECTION_FAILURE_THRESHOLD: u32 = 2;

/// 纯函数：输入上一轮的连续失败数与本轮错误，输出 (是否异常, 新的连续失败数)。
fn evaluate_collection_status(consecutive: u32, errors: &[String]) -> (bool, u32) {
    if errors.is_empty() {
        return (true, 0);
    }
    let next = consecutive + 1;
    (next < COLLECTION_FAILURE_THRESHOLD, next)
}
static COLLECTION_STATUS: LazyLock<Mutex<CollectionStatus>> = LazyLock::new(|| Mutex::new(CollectionStatus {
    ok: true,
    errors: Vec::new(),
    last_success_ms: None,
    failed_sources: Vec::new(),
    consecutive_failures: 0,
}));

#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum MainIntent {
    Providers { connection_id: String },
    Overview { settings: settings::Settings },
    Settings { section: String },
    About { version: String },
    /// 点击任务通知后打开自动化页并选中该任务。
    Automation { task_id: i64 },
    /// 灵动岛点会话（4.2）：打开会话线程并定位该会话。
    Sessions { platform: String, session_id: String },
    /// 灵动岛「查看用量」（4.2）：打开用量统计，按今天与本岛的平台 / 连接预筛。
    Usage { platform: Option<String>, connection_id: Option<String> },
}

static MAIN_INTENT: LazyLock<Mutex<Option<MainIntent>>> = LazyLock::new(|| Mutex::new(None));

/// 缓存条目当前的退避种类。显式刷新只跳过网络失败退避，不跳过上游限流退避。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CacheBackoff {
    /// 成功或稳定失败（不支持的形态等）：不构成退避，显式刷新照常重查
    None,
    /// 上游 429 / 限流：按 Retry-After 挡住所有刷新，显式刷新也一样
    RateLimited,
    /// 网络失败 / 抖动：15s×2ⁿ 的退避只挡自动与定时刷新
    Failed,
}

struct QueryCacheState<T> {
    /// 值、过期时刻、退避种类、取得时刻
    entries: HashMap<String, (T, Instant, CacheBackoff, Instant)>,
    in_flight: HashSet<String>,
    failures: HashMap<String, u32>,
    generations: HashMap<String, u64>,
    /// 原工具登录的单向摘要，仅内存使用；不包含或输出令牌。
    identities: HashMap<String, String>,
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
    state: Mutex<QueryCacheState<T>>,
    ready: Condvar,
}

impl<T> Default for QueryCache<T> {
    fn default() -> Self {
        Self { state: Mutex::new(QueryCacheState::default()), ready: Condvar::new() }
    }
}

#[derive(Clone, Copy)]
enum CacheOutcome { Success, RateLimited(u64), Failed, StableFailure }

/// 调用方对缓存新鲜度的要求。
#[derive(Clone, Copy, Debug, PartialEq)]
enum Refresh {
    /// 有效缓存即可
    Cached,
    /// 用户显式刷新：跳过成功缓存与网络失败退避，但保留限流退避
    Force,
    /// 定时刷新：缓存取得不超过这么久就复用。多个窗口各自按刷新间隔轮询同一连接，
    /// 同一周期内只打一次上游，又不会因缓存把间隔拖长。
    MaxAge(Duration),
}

impl From<bool> for Refresh {
    fn from(force: bool) -> Self {
        if force { Refresh::Force } else { Refresh::Cached }
    }
}

/// 命令参数到刷新模式：显式 force 优先，其次是定时刷新给出的最大缓存年龄。
fn refresh_mode(force: bool, max_age_ms: Option<u64>) -> Refresh {
    match (force, max_age_ms) {
        (true, _) => Refresh::Force,
        (false, Some(ms)) => Refresh::MaxAge(Duration::from_millis(ms)),
        (false, None) => Refresh::Cached,
    }
}

fn cached_query<T: Clone>(
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
        let Ok(mut state) = cache.state.lock() else { return query() };
        if let Some((value, expires, backoff, fetched)) = state.entries.get(&key) {
            let wants_fresh = match refresh {
                Refresh::Cached => false,
                Refresh::Force => true,
                Refresh::MaxAge(age) => fetched.elapsed() >= age,
            };
            // Force 只跳过失败退避，不跳过限流退避：上游 429 或抖动时用户点「立即刷新」
            // 要能立刻重查，但真被限流时仍按 Retry-After 挡住，否则连点会打爆上游。
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
        let Ok(next) = cache.ready.wait(state) else { return query() };
        drop(next);
        waited = true;
    }

    let value = query();
    if let Ok(mut state) = cache.state.lock() {
        state.in_flight.remove(&key);
        let outcome = classify(&value);
        let failures = match outcome {
            CacheOutcome::Failed => {
                let next = state.failures.get(&key).copied().unwrap_or(0).saturating_add(1);
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
            CacheOutcome::RateLimited(seconds) => (Duration::from_secs(seconds.clamp(1, 30 * 60)), CacheBackoff::RateLimited),
            CacheOutcome::Failed => (Duration::from_secs((15u64.saturating_mul(1u64 << failures.min(4))).min(5 * 60)), CacheBackoff::Failed),
            CacheOutcome::StableFailure => (Duration::from_secs(60), CacheBackoff::None),
        };
        // 凭证在查询期间被替换时，不把旧凭证的响应重新放回缓存。
        if state.generations.get(&key).copied().unwrap_or(0) == generation {
            let now = Instant::now();
            state.entries.insert(key, (value.clone(), now + ttl, backoff, now));
        }
        cache.ready.notify_all();
    }
    value
}

fn retry_after_seconds(reason: &str) -> u64 {
    reason.split("请等待 ").nth(1)
        .and_then(|tail| tail.split(' ').next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(60)
}

fn cached_native_quota(
    cache: &QueryCache<quota::QuotaState>, key: String, refresh: Refresh,
    identity: impl Fn() -> Result<String, quota::QuotaState>, query: impl FnOnce() -> quota::QuotaState,
) -> quota::QuotaState {
    let before = match identity() {
        Ok(identity) => identity,
        Err(state) => { bind_native_identity(cache, &key, None); return state; }
    };
    bind_native_identity(cache, &key, Some(&before));
    let state = cached_query(cache, key.clone(), refresh, quota_cache_outcome, query);
    match identity() {
        Ok(after) if after == before => state,
        Ok(after) => {
            bind_native_identity(cache, &key, Some(&after));
            quota::QuotaState::Failed { reason: "原工具登录在查询期间发生变化，请刷新账号数据".into() }
        }
        Err(state) => { bind_native_identity(cache, &key, None); state }
    }
}

fn bind_native_identity(cache: &QueryCache<quota::QuotaState>, key: &str, identity: Option<&str>) {
    if let Ok(mut state) = cache.state.lock() {
        if identity.is_some() && state.identities.get(key).map(String::as_str) == identity { return; }
        match identity {
            Some(identity) => { state.identities.insert(key.into(), identity.into()); }
            None => { state.identities.remove(key); }
        }
        state.entries.remove(key);
        state.failures.remove(key);
        let next = state.generations.get(key).copied().unwrap_or(0).wrapping_add(1);
        state.generations.insert(key.into(), next);
    }
}

/// 只读的身份校验读取，供展示路径（托盘图标、托盘摘要）使用：条目记录的写入身份与当前身份
/// 不一致，或拿不到当前身份（未登录 / 已注销）时按「无缓存」返回，避免把上一个账号的数字
/// 继续显示到下一次额度命令重查之前（成功缓存 TTL 最长 5 分钟）。
///
/// 这条路径**只读**：不删条目、不写缓存、不发网络请求。清理旧身份的结果与重新查询交给下一次
/// 额度命令（`cached_native_quota`）；展示端只负责不显示错数据。
/// `identities` 里没有记录的键（写入仍走裸 `cached_query`）没有可比对的基准，沿用原口径返回。
fn current_for_identity<T: Clone>(
    cache: &QueryCache<T>,
    key: &str,
    identity: impl FnOnce() -> Result<String, quota::QuotaState>,
) -> Option<(T, Instant, CacheBackoff, Instant)> {
    // 先取身份再拿锁：读凭据 / 磁盘期间不持有缓存锁，也不和查询线程互相等待。
    let current = identity();
    let state = cache.state.lock().ok()?;
    let entry = state.entries.get(key)?.clone();
    match (state.identities.get(key), current) {
        (None, _) => Some(entry),
        (Some(bound), Ok(current)) if *bound == current => Some(entry),
        _ => None,
    }
}

/// 只读读取缓存当前值（不校验身份）：展示路径读没有身份基准的键时使用。
fn cached_value<T: Clone>(cache: &QueryCache<T>, key: &str) -> Option<(T, Instant, CacheBackoff, Instant)> {
    cache.state.lock().ok()?.entries.get(key).cloned()
}

fn quota_cache_outcome(value: &quota::QuotaState) -> CacheOutcome {
    match value {
        quota::QuotaState::Ok { .. } => CacheOutcome::Success,
        quota::QuotaState::RateLimited { reason } => CacheOutcome::RateLimited(retry_after_seconds(reason)),
        quota::QuotaState::Failed { .. } => CacheOutcome::Failed,
        _ => CacheOutcome::StableFailure,
    }
}

fn balance_cache_outcome(value: &quota::BalanceState) -> CacheOutcome {
    match value {
        quota::BalanceState::Ok { .. } => CacheOutcome::Success,
        quota::BalanceState::RateLimited { reason } => CacheOutcome::RateLimited(retry_after_seconds(reason)),
        quota::BalanceState::Failed { .. } => CacheOutcome::Failed,
        _ => CacheOutcome::StableFailure,
    }
}

fn api_usage_cache_outcome(value: &quota::ApiUsageState) -> CacheOutcome {
    match value {
        quota::ApiUsageState::Ok { .. } => CacheOutcome::Success,
        quota::ApiUsageState::RateLimited { reason } => CacheOutcome::RateLimited(retry_after_seconds(reason)),
        quota::ApiUsageState::Failed { .. } => CacheOutcome::Failed,
        _ => CacheOutcome::StableFailure,
    }
}

static QUOTA_CACHE: LazyLock<QueryCache<quota::QuotaState>> = LazyLock::new(QueryCache::default);
static BALANCE_CACHE: LazyLock<QueryCache<quota::BalanceState>> = LazyLock::new(QueryCache::default);
static API_USAGE_CACHE: LazyLock<QueryCache<quota::ApiUsageState>> = LazyLock::new(QueryCache::default);

fn invalidate_network_cache(id: &str) {
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

#[tauri::command]
fn take_main_intent() -> Option<MainIntent> {
    MAIN_INTENT.lock().ok().and_then(|mut intent| intent.take())
}

pub(crate) fn emit_collection_status(app: &tauri::AppHandle, result: &collector::ScanResult) {
    let status = if let Ok(mut current) = COLLECTION_STATUS.lock() {
        let (ok, consecutive) = evaluate_collection_status(current.consecutive_failures, &result.errors);
        if !ok {
            eprintln!("[采集] 连续 {} 轮扫描出错：{}", consecutive, result.errors.join("；"));
        }
        current.ok = ok;
        current.consecutive_failures = consecutive;
        current.errors = result.errors.clone();
        // 只在确认异常时定界到出错平台；正常轮次清空，避免历史残留影响展示
        current.failed_sources = if ok { Vec::new() } else { result.failed_sources.clone() };
        if current.ok {
            current.last_success_ms = Some(chrono::Local::now().timestamp_millis());
        }
        current.clone()
    } else {
        CollectionStatus { ok: false, errors: vec!["采集状态锁不可用".into()], last_success_ms: None, failed_sources: Vec::new(), consecutive_failures: 0 }
    };
    let _ = app.emit("collection-status", status);
}

#[tauri::command]
fn collection_status() -> CollectionStatus {
    COLLECTION_STATUS.lock().map(|status| status.clone()).unwrap_or(CollectionStatus {
        ok: false,
        errors: vec!["采集状态锁不可用".into()],
        last_success_ms: None,
        failed_sources: Vec::new(),
        consecutive_failures: 0,
    })
}

#[tauri::command]
async fn local_codex_quota(app: tauri::AppHandle, force: bool, max_age_ms: Option<u64>) -> Result<quota::QuotaState, String> {
    let force = refresh_mode(force, max_age_ms);
    let state = tauri::async_runtime::spawn_blocking(move || cached_query(
        &QUOTA_CACHE,
        "local-codex".into(),
        force,
        quota_cache_outcome,
        quota::local_codex_quota,
    ))
        .await.map_err(|e| e.to_string())?;
    let settings = app.state::<Cfg>().0.get();
    if settings.island_platform == "codex"
        && settings.island_kind == "auth"
        && settings.island_connection_id.is_none()
    {
        update_tray_icon(&app);
    }
    Ok(state)
}

/// 查本机 Grok 订阅额度（~/.grok/auth.json 的 OAuth 凭证）。
/// 灵动岛托盘图标逻辑目前只覆盖 claude/codex，Grok 额度仅在主面板展示。
#[tauri::command]
async fn local_grok_quota(force: bool, max_age_ms: Option<u64>) -> Result<quota::QuotaState, String> {
    let force = refresh_mode(force, max_age_ms);
    tauri::async_runtime::spawn_blocking(move || cached_query(
        &QUOTA_CACHE,
        "local-grok".into(),
        force,
        quota_cache_outcome,
        grok_quota::local_grok_quota,
    ))
        .await.map_err(|e| e.to_string())
}

/// 查询本机 ZCode BigModel Coding Plan API Key 的套餐额度。
///
/// 走 `cached_native_quota` 而不是裸 `cached_query`：本机 ZCode 登录可能在 5 分钟 TTL
/// 内换账号或换令牌，固定 key 会把上一个账号的额度继续显示给新账号。身份由
/// `zcode_quota::account_identity` 给出（家族 + 令牌种类 + 令牌的 SHA-256 摘要），
/// 身份变化时 `bind_native_identity` 会连同成功缓存与失败退避记录一起清掉。
/// 代价是每次查询多一次凭据读取与解密（毫秒级），缓存 key 里只有哈希，不含令牌或邮箱。
#[tauri::command]
async fn local_zcode_quota(force: bool, max_age_ms: Option<u64>) -> Result<quota::QuotaState, String> {
    let force = refresh_mode(force, max_age_ms);
    tauri::async_runtime::spawn_blocking(move || cached_native_quota(
        &QUOTA_CACHE,
        "local-zcode".into(),
        force,
        zcode_quota::account_identity,
        zcode_quota::local_zcode_quota,
    ))
        .await.map_err(|error| error.to_string())
}

/* --------------------------- Tauri 命令 --------------------------- */

#[tauri::command]
async fn scan_local_sessions(app: tauri::AppHandle, db: State<'_, Db>) -> Result<collector::ScanResult, String> {
    if validation::isolated() { return Err("隔离验收不读取用户日志".into()); }
    let result = with_database(db.0.clone(), move |conn| {
        Ok(collector::scan(conn))
    })
    .await?;
    emit_collection_status(&app, &result);
    Ok(result)
}

/// 调用方窗口必须是灵动岛（本体或分身），返回其分身 id（本体为 None）。
fn island_id_of(window: &WebviewWindow) -> Result<Option<String>, String> {
    island_clones::id_from_label(window.label()).ok_or_else(|| "只有灵动岛窗口能执行此操作".to_string())
}

fn island_dock(settings: &settings::Settings, id: Option<&str>) -> dock::DockState {
    island_clones::profile(settings, id).map(|profile| profile.dock).unwrap_or_default()
}

fn island_window(app: &tauri::AppHandle, id: Option<&str>) -> Option<WebviewWindow> {
    app.get_webview_window(&island_clones::label(id))
}

/// 当前存活的全部灵动岛窗口（本体 + 分身）。
fn island_windows(app: &tauri::AppHandle) -> Vec<WebviewWindow> {
    app.webview_windows().into_iter()
        .filter(|(label, _)| island_clones::is_island_label(label))
        .map(|(_, window)| window)
        .collect()
}

/// 本体在前、分身在后的全部岛 id。
fn island_ids(settings: &settings::Settings) -> Vec<Option<String>> {
    std::iter::once(None).chain(settings.island_clones.iter().map(|clone| Some(clone.id.clone()))).collect()
}

/// 停靠事件带上岛 id；各岛窗口只处理自己的，不会被别的岛的停靠变化重置形态。
#[derive(Clone, serde::Serialize)]
struct DockChangedEvent { island: Option<String>, dock: dock::DockState }

#[derive(Clone, serde::Serialize)]
struct DockHintEvent { island: Option<String>, hint: Option<dock::SnapHint> }

fn emit_dock_changed(app: &tauri::AppHandle, id: Option<&str>, dock: &dock::DockState) {
    let _ = app.emit("dock-changed", DockChangedEvent { island: id.map(str::to_string), dock: dock.clone() });
}

fn store_island_dock(app: &tauri::AppHandle, id: Option<&str>, dock: dock::DockState) -> settings::Settings {
    app.state::<Cfg>().0.update(|settings| { island_clones::set_dock(settings, id, dock); })
}

/// 分身的自由态位置只在拖动结束、创建与重置时记录，不在每次移动回调里落盘。
fn remember_clone_position(app: &tauri::AppHandle, id: &str, window: &WebviewWindow) {
    let Ok(position) = window.outer_position() else { return };
    let store = &app.state::<Cfg>().0;
    let changed = store.get().island_clones.iter()
        .any(|clone| clone.id == id && clone.position != Some((position.x, position.y)));
    if changed {
        store.update(|settings| { island_clones::set_position(settings, id, (position.x, position.y)); });
    }
}

/// 灵动岛窗口尺寸跟随内容。收缩 / 展开 / 停靠三态高度差很大（展开态有会话列表），
/// 写死会把内容截断——由前端测量实际内容后调用本命令。调用方窗口即目标岛。
#[tauri::command]
fn resize_island(app: tauri::AppHandle, window: WebviewWindow, width: f64, height: f64) -> Result<Option<f64>, String> {
    let id = island_id_of(&window)?;
    let w = window;
    if !width.is_finite() || !height.is_finite() || width < 8.0 || height < 8.0 {
        return Err("灵动岛尺寸无效".into());
    }
    // 用户主动展开本体后，不再按首次安装计时强制收成停靠条。
    if id.is_none() && height > 150.0 { FIRST_INSTALL_INTRO.store(false, Ordering::Release); }
    let cfg = app.state::<Cfg>().0.get();
    // 改尺寸后重新确认置顶：对应设置里默认开启的「灵动岛置顶」
    let _ = w.set_always_on_top(cfg.always_on_top);
    let dock_state = island_dock(&cfg, id.as_deref());
    if let Some(edge) = dock_state.edge {
        dock::anchor(&w, edge, dock_state.offset, width, height)?;
        // 把停靠窗口的实际工作区上限交给前端，避免动画等待不可达高度。
        return Ok(dock::work_area(&w).map(|area| area.3));
    } else {
        // 胶囊自由态：围绕水平中心缩放，展开/收回时内容不会整块往右跳
        if cfg.island_style == "capsule" {
            dock::resize_free_centered(&w, width, height)?;
        } else {
            dock::resize_free(&w, width, height)?;
        }
        if id.is_none() && FIRST_INSTALL_INTRO.load(Ordering::Acquire) {
            dock::place_free_top_center(&w)?;
        }
    }
    Ok(None)
}

/// 区间内的 Token 分项（新增输入 / 输出 / 缓存写入 / 缓存命中）、请求数与命中率。
/// 未知字段保持 null 交给界面显示「—」，不在这里补零。
#[tauri::command]
async fn source_metrics(db: State<'_, DbRead>, platform:String, period:String,
    custom:Option<db::CustomRange>, model:Option<String>, query_end_ms:i64) -> Result<source_store::Metrics,String> {
    validate_query_range(custom,query_end_ms)?;
    if !platforms::known(&platform) {return Err("未知平台".into());}
    with_reader(db.0.clone(),move |conn| {
        let (start,end)=db::resolve_range_at(conn,&platform,&period,custom,query_end_ms);
        source_store::metrics(conn,&platform,start,end,model.as_deref()).map_err(|e|e.to_string())
    }).await
}

/// 本机为该平台配置的接入方式：绿色 Auth / 蓝色 API（§6.3）
/// 只返回方式与来源说明，绝不返回凭证内容
#[tauri::command]
fn connection_kind(platform: String) -> creds::ConnectionInfo {
    creds::detect(&platform)
}

/// 灵动岛实时用量：自 `since` 游标以来的新增 Token、涉及的会话、最近活跃会话
#[tauri::command]
async fn live_usage(
    db: State<'_, DbRead>,
    platform: String,
    since: Option<i64>,
) -> Result<db::LiveUsage, String> {
    with_reader(db.0.clone(), move |conn| {
        let r = session_presence::live_usage(conn, &platform, since).map_err(|e| e.to_string())?;
        if since.is_none() || r.delta_tokens > 0 {
            println!(
                "[实时] since={:?} 新增={} token, 运行中={} 个会话, 今日={:?}",
                since, r.delta_tokens, r.active_sessions.len(), r.today_tokens
            );
        }
        Ok(r)
    })
    .await
}

/// 用量统计页（模块 C）：概览与用量分析共用的跨供应商聚合
#[tauri::command]
async fn island_source_breakdown(db: State<'_, DbRead>, platform: Option<String>) -> Result<usage_stats::SourceBreakdown, String> {
    // 本地时区的今天 [00:00, 明天 00:00)
    let today = chrono::Local::now().date_naive();
    let bound = |day: chrono::NaiveDate| {
        day.and_hms_opt(0, 0, 0)
            .and_then(|time| time.and_local_timezone(chrono::Local).earliest())
            .map(|time| time.timestamp_millis())
    };
    let (Some(start), Some(end)) = (bound(today), today.succ_opt().and_then(bound)) else {
        return Err("无法确定今天的时间范围".into());
    };
    with_reader(db.0.clone(), move |conn| usage_stats::source_breakdown(conn, platform.as_deref(), start, end).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn stats_overview(db: State<'_, DbRead>, filter: usage_stats::StatsFilter) -> Result<usage_stats::Overview, String> {
    filter.validate()?;
    with_reader(db.0.clone(), move |conn| usage_stats::overview(conn, &filter).map_err(|e| e.to_string())).await
}

/// 全年活跃日历；日期固定为一年，只按供应商与账号筛选
#[tauri::command]
async fn stats_activity(db: State<'_, DbRead>, platform: Option<String>, connections: Vec<String>, now_ms: i64) -> Result<usage_stats::Activity, String> {
    if platform.as_deref().is_some_and(|p| !platforms::known(p)) { return Err("未知平台".into()); }
    with_reader(db.0.clone(), move |conn| usage_stats::activity(conn, platform, connections, now_ms).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn stats_calls(db: State<'_, DbRead>, filter: usage_stats::StatsFilter, page: u32) -> Result<usage_stats::CallPage, String> {
    filter.validate()?;
    with_reader(db.0.clone(), move |conn| usage_stats::calls(conn, &filter, page).map_err(|e| e.to_string())).await
}

/// 会话线程页（模块 D）：本机会话列表
#[tauri::command]
async fn sessions_list(db: State<'_, DbRead>, filter: session_threads::SessionFilter) -> Result<session_threads::SessionList, String> {
    if filter.platform.as_deref().is_some_and(|p| !platforms::known(p)) { return Err("未知平台".into()); }
    with_reader(db.0.clone(), move |conn| session_threads::list(conn, &filter).map_err(|e| e.to_string())).await
}

/// 只读消息记录：`before` 为空取最后一页
#[tauri::command]
async fn session_messages(db: State<'_, DbRead>, platform: String, session_id: String, before: Option<usize>) -> Result<session_threads::MessagePage, String> {
    with_reader(db.0.clone(), move |conn| session_threads::messages(conn, &platform, &session_id, before, 40).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn session_continue_check(db: State<'_, DbRead>, platform: String, session_id: String) -> Result<session_threads::ContinueCheck, String> {
    with_reader(db.0.clone(), move |conn| session_threads::continue_check(conn, &platform, &session_id).map_err(|e| e.to_string())).await
}

/// 在新的控制台窗口里用原工具续聊（Claude Code / Codex）
#[tauri::command]
async fn session_continue(db: State<'_, DbRead>, platform: String, session_id: String) -> Result<(), String> {
    with_reader(db.0.clone(), move |conn| session_threads::continue_session(conn, &platform, &session_id)).await
}

#[tauri::command]
fn island_drag(app: tauri::AppHandle, window: WebviewWindow) -> Result<(), String> {
    let id = island_id_of(&window)?;
    if id.is_none() { FIRST_INSTALL_INTRO.store(false, Ordering::Release); }
    let w = window;
    dock::DRAGGING.store(true, std::sync::atomic::Ordering::Relaxed);
    let result = dock::drag(&w);
    dock::DRAGGING.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = app.emit("dock-hint", DockHintEvent { island: id.clone(), hint: None });
    result?;
    let snapped = dock_release_for(&app, &w, id.as_deref());
    if let (Some(id), None) = (id.as_deref(), snapped) { remember_clone_position(&app, id, &w); }
    let _ = app.emit("settings-changed", app.state::<Cfg>().0.get());
    Ok(())
}

/// 置顶是全局开关：对本体与全部分身同时生效。
#[tauri::command]
fn island_topmost(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    let windows = island_windows(&app);
    if windows.is_empty() { return Err("找不到灵动岛".into()); }
    let previous = app.state::<Cfg>().0.get().always_on_top;
    for w in &windows { w.set_always_on_top(on).map_err(|e| e.to_string())?; }
    let next = match app.state::<Cfg>().0.try_update(|s| s.always_on_top = on) {
        Ok(next) => next,
        Err(error) => { for w in &windows { let _ = w.set_always_on_top(previous); } return Err(error); }
    };
    let _ = app.emit("settings-changed", &next);
    Ok(next)
}

/// 右键菜单「显示位置」：`island` 为目标岛（None 为本体）。
#[tauri::command]
fn island_position(app: tauri::AppHandle, edge: Option<dock::Edge>, island: Option<String>) {
    let id = island.as_deref();
    if id.is_none() { FIRST_INSTALL_INTRO.store(false, Ordering::Release); }
    let Some(w) = island_window(&app, id) else { return };
    if let Some(edge) = edge {
        let Some(offset) = dock::centered_offset(&w, edge) else { return };
        dock::apply_dock(&w, edge, offset);
        // 窗口 API 会跨线程等待主事件循环；禁止在设置锁内调用，移动回调也会读取设置。
        let monitor = w.current_monitor().ok().flatten().and_then(|m| m.name().cloned());
        store_island_dock(&app, id, dock::DockState { edge: Some(edge), offset, monitor });
    } else {
        dock_undock_for(&app, &w, id);
    }
    let settings = app.state::<Cfg>().0.get();
    let _ = app.emit("settings-changed", &settings);
    emit_dock_changed(&app, id, &island_dock(&settings, id));
}

fn data_paths(app: &tauri::AppHandle) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let directory = match validation::data_dir() {
        Some(path) => path,
        None => app.path().app_data_dir().map_err(|error| error.to_string())?,
    };
    Ok((directory.clone(), directory.join("usage.db")))
}

/// 各 agent CLI 的版本与建议更新命令（只读：不自动执行升级）。
/// 版本读不到、命令不在 PATH、安装方式认不出来都如实分开表达，不猜。
#[tauri::command]
async fn agent_versions() -> Result<Vec<agent_version::AgentVersion>, String> {
    tauri::async_runtime::spawn_blocking(agent_version::detect)
        .await
        .map_err(|error| format!("检测 Agent 版本失败：{error}"))
}

/// Agent 自动发现（A1/A2，只读）：探测本机装了哪些 agent。
/// 状态 installed / cli_missing / not_found 与命中的证据都如实返回，不猜。
#[tauri::command]
async fn agent_discover() -> Result<Vec<agent_discovery::AgentDiscovery>, String> {
    tauri::async_runtime::spawn_blocking(agent_discovery::discover)
        .await
        .map_err(|error| format!("探测本机 Agent 失败：{error}"))
}

/// 探测 WSL 与其中各 agent 的安装情况（只读，不写 WSL 内文件）。
#[tauri::command]
async fn wsl_probe_agents() -> Result<wsl_probe::WslProbe, String> {
    tauri::async_runtime::spawn_blocking(wsl_probe::probe)
        .await
        .map_err(|error| format!("探测 WSL 失败：{error}"))
}

/// 导出最近 `days` 天的用量到 OTLP 端点（批次 G §3.12）。
///
/// 只导聚合值（平台 + 模型 + token 计数，以及按命名 Key 名的请求数），不含会话、连接、Key 原文或请求正文。
/// 端点与认证头由调用方传；认证头只在本函数内使用，不回显任何值。
#[tauri::command]
async fn otlp_export_usage(
    app: tauri::AppHandle, endpoint: String, authorization: Option<String>, days: u32,
) -> Result<otlp::OtlpExportResult, String> {
    let span_days = days.clamp(1, 90) as i64;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let since_ms = now_ms - span_days * 24 * 60 * 60 * 1000;
    let db = app.state::<Db>().0.clone();
    let (points, keys) = with_database(db, move |conn| {
        Ok::<_, String>((otlp::aggregate(conn, since_ms, now_ms)?, otlp::aggregate_keys(conn, since_ms, now_ms)?))
    })
    .await?;
    let mut headers = Vec::new();
    if let Some(value) = authorization.filter(|v| !v.trim().is_empty()) {
        headers.push(("authorization".to_string(), value));
    }
    otlp::export(&endpoint, &headers, &points, &keys, now_ms).await
}

#[tauri::command]
fn data_info(app: tauri::AppHandle) -> Result<DataInfo, String> {
    let (_, database) = data_paths(&app)?;
    let size_bytes = [database.clone(), database.with_extension("db-wal"), database.with_extension("db-shm")]
        .into_iter()
        .filter_map(|path| std::fs::metadata(path).ok().map(|metadata| metadata.len()))
        .sum();
    Ok(DataInfo {
        path: database.to_string_lossy().into_owned(),
        size_bytes,
    })
}

#[tauri::command]
fn open_data_directory(app: tauri::AppHandle) -> Result<(), String> {
    let (directory, _) = data_paths(&app)?;
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    app.opener()
        .open_path(directory.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| error.to_string())
}

const AUTOSTART_VALUE_NAME: &str = "Jolu";
const AUTOSTART_REGISTRY_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(target_os = "windows")]
fn windows_autostart_status() -> Result<bool, String> {
    let status = windows_command("reg.exe")
        .args(["QUERY", AUTOSTART_REGISTRY_KEY, "/v", AUTOSTART_VALUE_NAME])
        .status()
        .map_err(|error| format!("无法读取开机启动状态：{error}"))?;
    Ok(status.success())
}

#[cfg(target_os = "windows")]
fn windows_command(program: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    let mut command = std::process::Command::new(program);
    command.creation_flags(0x0800_0000);
    // 注册项不存在是“未启用”的正常状态；不要让 reg.exe 的本地代码页错误文本污染日志。
    command.stdout(Stdio::null()).stderr(Stdio::null());
    command
}

#[cfg(not(target_os = "windows"))]
fn windows_autostart_status() -> Result<bool, String> {
    Err("当前系统暂不支持开机启动配置".into())
}

#[tauri::command]
fn autostart_status() -> Result<bool, String> {
    windows_autostart_status()
}

#[tauri::command]
fn set_autostart(on: bool) -> Result<bool, String> {
    if validation::isolated() { return Err("隔离验收不修改开机启动".into()); }
    #[cfg(target_os = "windows")]
    {
        let status = if on {
            let executable = std::env::current_exe()
                .map_err(|error| format!("无法获取程序路径：{error}"))?;
            let command = startup::command(&executable);
            windows_command("reg.exe")
                .args([
                    "ADD",
                    AUTOSTART_REGISTRY_KEY,
                    "/v",
                    AUTOSTART_VALUE_NAME,
                    "/t",
                    "REG_SZ",
                    "/d",
                    &command,
                    "/f",
                ])
                .status()
        } else {
            windows_command("reg.exe")
                .args(["DELETE", AUTOSTART_REGISTRY_KEY, "/v", AUTOSTART_VALUE_NAME, "/f"])
                .status()
        }
        .map_err(|error| format!("无法更新开机启动：{error}"))?;
        if !status.success() && (on || windows_autostart_status()?) {
            return Err("系统拒绝更新开机启动注册项".into());
        }
        return windows_autostart_status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = on;
        Err("当前系统暂不支持开机启动配置".into())
    }
}

/// 每日 warm-up 设置（批次 G §3.12）：开关与触发小时。
/// 打开时清掉上次日期，让今天就能按新设置跑一次。
#[tauri::command]
fn set_warmup(app: tauri::AppHandle, enabled: bool, hour: u32) -> Result<settings::Settings, String> {
    let hour = hour.min(23);
    let next = app.state::<Cfg>().0.try_update(|settings| {
        let turning_on = enabled && !settings.warmup_enabled;
        settings.warmup_enabled = enabled;
        settings.warmup_hour = hour;
        if turning_on { settings.warmup_last_date = None; }
    })?;
    let _ = app.emit("settings-changed", &next);
    Ok(next)
}

/// OTLP 导出端点设置（批次 G §3.12）：留空即关闭导出。
/// 存之前先校验协议与格式，避免把明显写错的地址存下来。
#[tauri::command]
fn set_otlp_endpoint(app: tauri::AppHandle, endpoint: Option<String>) -> Result<settings::Settings, String> {
    let normalized = match endpoint.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => Some(otlp::metrics_url(value)?),
        None => None,
    };
    let next = app.state::<Cfg>().0.try_update(|settings| settings.otlp_endpoint = normalized)?;
    let _ = app.emit("settings-changed", &next);
    Ok(next)
}

#[tauri::command]
fn set_dock_enabled(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    if !on { FIRST_INSTALL_INTRO.store(false, Ordering::Release); }
    let next = app.state::<Cfg>().0.try_update(|settings| {
        settings.dock_enabled = on;
        if !on {
            settings.dock = Default::default();
            for clone in &mut settings.island_clones { clone.dock = Default::default(); }
        }
    })?;
    if !on {
        for window in island_windows(&app) { dock::undock(&window); }
    }
    for id in island_ids(&next) { emit_dock_changed(&app, id.as_deref(), &island_dock(&next, id.as_deref())); }
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

#[tauri::command]
fn set_silent_startup(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    // 已启用的旧版注册项也在用户保存选择时补齐启动标记。
    if !validation::isolated() && windows_autostart_status()? {
        set_autostart(true)?;
    }
    let next = app.state::<Cfg>().0.try_update(|settings| settings.silent_startup = on)?;
    let _ = app.emit("settings-changed", &next);
    Ok(next)
}

/// 保持系统唤醒（批次 G §3.12）：开启后运行期间不让系统进入睡眠。
/// 切换立即生效；不支持的系统上打开会报错，不假装成功。
#[tauri::command]
fn set_keep_awake(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    let applied = keep_awake::set(on)?;
    let next = app.state::<Cfg>().0.try_update(|settings| settings.keep_awake = applied)?;
    let _ = app.emit("settings-changed", &next);
    Ok(next)
}

#[tauri::command]
fn set_theme(app: tauri::AppHandle, theme: String) -> Result<settings::Settings, String> {
    if !matches!(theme.as_str(), "light" | "dark" | "system") {
        return Err("主题必须是 light、dark 或 system".into());
    }
    let next = app.state::<Cfg>().0.try_update(|settings| settings.theme = theme)?;
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

#[tauri::command]
fn set_display_preferences(app: tauri::AppHandle, opacity: u8, scale: u16, shrink_scale: u16, refresh_minutes: u32) -> Result<settings::Settings, String> {
    settings::validate_display_preferences(opacity, scale, shrink_scale, refresh_minutes)?;
    let next = app.state::<Cfg>().0.try_update(|settings| {
        settings.island_opacity = opacity;
        settings.island_scale = scale;
        settings.island_shrink_scale = shrink_scale;
        settings.refresh_minutes = refresh_minutes;
    })?;
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

/// 灵动岛上的「打开模型入口」「修复」：打开主窗口并切到对应页面。只接受已知页面。
#[tauri::command]
fn open_main_section(app: tauri::AppHandle, section: String) -> Result<(), String> {
    if !matches!(section.as_str(), "gateway" | "island" | "connections") {
        return Err(format!("未知的主窗口页面：{section}"));
    }
    show_main_with_intent(&app, MainIntent::Settings { section });
    Ok(())
}

/// 灵动岛点会话：主窗口打开会话线程并定位（会话已删时会话页自己回到列表）。
#[tauri::command]
fn open_main_session(app: tauri::AppHandle, platform: String, session_id: String) -> Result<(), String> {
    let (platform, session_id) = (platform.trim().to_string(), session_id.trim().to_string());
    if platform.is_empty() || session_id.is_empty() {
        return Err("缺少要打开的会话".into());
    }
    show_main_with_intent(&app, MainIntent::Sessions { platform, session_id });
    Ok(())
}

/// 灵动岛「查看用量」：主窗口打开用量统计，按今天与给定的平台 / 连接预筛（连接已删时只带平台）。
#[tauri::command]
fn open_main_usage(app: tauri::AppHandle, platform: Option<String>, connection_id: Option<String>) -> Result<(), String> {
    let clean = |value: Option<String>| value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    show_main_with_intent(&app, MainIntent::Usage { platform: clean(platform), connection_id: clean(connection_id) });
    Ok(())
}

#[tauri::command]
fn set_island_style(app: tauri::AppHandle, style: String) -> Result<settings::Settings, String> {
    settings::validate_island_style(&style)?;
    let next = app.state::<Cfg>().0.try_update(|settings| settings.island_style = style)?;
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

#[tauri::command]
fn set_balance_alert(app: tauri::AppHandle, threshold: Option<f64>, currency: String) -> Result<settings::Settings, String> {
    if threshold.is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 1_000_000_000.0)
        || currency.len() != 3 || !currency.bytes().all(|c| c.is_ascii_uppercase()) {
        return Err("请输入有效的三位大写币种代码，以及大于 0、不超过 10 亿的阈值".into());
    }
    let next = app.state::<Cfg>().0.try_update(|settings| {
        settings.balance_alert_threshold = threshold;
        settings.balance_alert_currency = currency;
    })?;
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

fn retention_cutoff(days: u32) -> Result<i64, String> {
    if !matches!(days, 30 | 90 | 180 | 365) {
        return Err("历史保留天数必须是 30、90、180 或 365".into());
    }
    Ok(chrono::Utc::now().timestamp_millis() - i64::from(days) * 86_400_000)
}

#[tauri::command]
fn set_retention_days(app: tauri::AppHandle, days: Option<u32>) -> Result<settings::Settings, String> {
    if let Some(days) = days { retention_cutoff(days)?; }
    let next = app.state::<Cfg>().0.try_update(|settings| settings.retention_days = days)?;
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

// 任务 2.1 开始
#[tauri::command]
fn set_default_page(app: tauri::AppHandle, page: String) -> Result<settings::Settings, String> {
    settings::validate_default_page(&page)?;
    let next = app.state::<Cfg>().0.try_update(|settings| settings.default_page = page)?;
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

/// 只记录上次访问的页面；切页很频繁，不广播 settings-changed，托盘打开时读快照即可。
#[tauri::command]
fn remember_page(app: tauri::AppHandle, page: String) -> Result<(), String> {
    settings::validate_last_page(&page)?;
    app.state::<Cfg>().0.try_update(|settings| settings.last_page = Some(page))?;
    Ok(())
}
// 任务 2.1 结束

#[tauri::command]
async fn cleanup_preview(db: State<'_, DbRead>, days: u32) -> Result<db::CleanupPreview, String> {
    let cutoff = retention_cutoff(days)?;
    with_reader(db.0.clone(), move |conn| {
        db::cleanup_preview(conn, cutoff).map_err(|error| error.to_string())
    }).await
}

#[tauri::command]
async fn cleanup_history(app: tauri::AppHandle, db: State<'_, Db>, days: u32) -> Result<db::CleanupPreview, String> {
    let cutoff = retention_cutoff(days)?;
    let result = with_database(db.0.clone(), move |conn| {
        db::cleanup_history(conn, cutoff).map_err(|error| error.to_string())
    }).await?;
    let _ = app.emit("refresh-requested", ());
    Ok(result)
}

/// 导出备份的核心逻辑，由 `data_files` 的文件导出命令调用；日期区间左闭右开
async fn export_data(db: State<'_, DbRead>, platform: Option<String>, start_ms: Option<i64>, end_ms: Option<i64>) -> Result<String, String> {
    data_files::valid_export_scope(platform.as_deref(), start_ms, end_ms)?;
    with_reader(db.0.clone(), move |conn| db::export_backup(conn, platform.as_deref(), start_ms, end_ms)).await
}

#[tauri::command]
fn main_drag(app: tauri::AppHandle) -> Result<(), String> {
    let window = app.get_webview_window(MAIN).ok_or("找不到主面板")?;
    window.start_dragging().map_err(|e| e.to_string())
}

#[tauri::command]
async fn island_menu(app: tauri::AppHandle, window: WebviewWindow, refreshing: bool) -> Result<(), String> {
    let _ = refreshing;
    let id = island_id_of(&window)?;
    context_menu::open(&app, true, id)
}

fn dock_release_for(app: &tauri::AppHandle, w: &WebviewWindow, id: Option<&str>) -> Option<dock::SnapResult> {
    let store = &app.state::<Cfg>().0;
    if !store.get().dock_enabled {
        store.update(|settings| { island_clones::set_dock(settings, id, Default::default()); });
        emit_dock_changed(app, id, &Default::default());
        return None;
    }
    let r = dock::snap_on_release(w);
    let monitor = w.current_monitor().ok().flatten().and_then(|m| m.name().cloned());
    let dock_state = match &r {
        Some(sr) => dock::DockState { edge: sr.edge, offset: sr.offset, monitor },
        None => Default::default(),
    };
    store.update(|settings| { island_clones::set_dock(settings, id, dock_state.clone()); });
    emit_dock_changed(app, id, &dock_state);
    r
}

/// 解除停靠，回到自由态收缩尺寸
#[tauri::command]
fn dock_undock(app: tauri::AppHandle, window: WebviewWindow) -> Result<(), String> {
    let id = island_id_of(&window)?;
    dock_undock_for(&app, &window, id.as_deref());
    Ok(())
}

fn dock_undock_for(app: &tauri::AppHandle, w: &WebviewWindow, id: Option<&str>) {
    dock::undock(w);
    let next = app.state::<Cfg>().0.update(|settings| { island_clones::set_dock(settings, id, Default::default()); });
    let _ = app.emit("settings-changed", &next);
    emit_dock_changed(app, id, &Default::default());
}

#[tauri::command]
async fn cost_estimate(
    db: State<'_, DbRead>,
    platform: String,
    period: String,
    custom: Option<db::CustomRange>,
    model: Option<String>,
    connection: Option<String>,
    query_end_ms: i64,
) -> Result<db::CostEstimate, String> {
    validate_query_range(custom, query_end_ms)?;
    with_reader(db.0.clone(), move |conn| {
        db::cost_estimate_at_scoped(conn, &platform, &period, custom, model.as_deref(), connection.as_deref(), query_end_ms)
            .map_err(|e| e.to_string())
    })
    .await
}

/* --------------------- 额度与余额查询（§2.5 / §7.4） --------------------- */

/// `connection_quota` 里「普通 ZCode 本机登录连接」这一支的缓存键。
///
/// 稳定性：这是常量，不拼连接 id、也不拼账号标识。
/// - 这条路径查的是**这台机器的 ZCode 登录**本身，不是某一行连接记录：同一个登录可能被
///   登记成 `monitor:native:zcode:main` 之类的连接，重新保存 / 迁移后连接 id 会变，拼 id
///   会让同一份数据落进不同键、各自再打一次上游；
/// - 跨账号隔离不靠键里的 id，靠写入时绑定的身份（`zcode_quota::account_identity`）：
///   换账号 / 换令牌时 `bind_native_identity` 会连同成功缓存与失败退避一起清掉条目；
/// - 与 `local-zcode` 刻意分开：那是本机 BigModel 套餐命令（`local_zcode_quota`）的条目，
///   两支入参与错误语义不同，共用键会把两边结果串起来。
const ZCODE_CONNECTION_CACHE_KEY: &str = "zcode-connection:local-login";

/// 普通 ZCode auth 连接（`platform = zcode`、`kind = auth`，且不是
/// `monitor:native:zcode:account`）的额度查询：与 `local_zcode_quota` 命令、
/// `monitor:native:zcode:account` 分支统一到同一套缓存与退避口径。
///
/// 统一的原因：这条分支原来每次调用都直连上游——供应商页、灵动岛、自动化（模块 E）与托盘
/// 各自按自己的节奏问同一份本机登录，一次界面刷新就能打出好几发请求；上游 429 时用户连点
/// 「立即刷新」更是把它打爆。走缓存后同一身份在 TTL 内只打一次，限流按 `Retry-After` 挡住。
///
/// TTL 与退避沿用 `cached_query` 里唯一那张表，不新加时长：Success 5 分钟、
/// RateLimited clamp(1s, 30min)、Failed 15s×2ⁿ（上限 5 分钟）、StableFailure 60 秒。
/// `Refresh::Force` 只跳过成功缓存与失败退避，不跳过限流退避。
fn cached_zcode_connection_quota(
    cache: &QueryCache<quota::QuotaState>,
    refresh: Refresh,
    identity: impl Fn() -> Result<String, quota::QuotaState>,
    query: impl FnOnce() -> quota::QuotaState,
) -> quota::QuotaState {
    cached_native_quota(cache, ZCODE_CONNECTION_CACHE_KEY.to_string(), refresh, identity, query)
}

/// 查某个连接的额度。
///
/// 路由规则：填了 `base_url` 时先按域名识别编程套餐供应商（智谱/Kimi/MiniMax/
/// ZenMux/OpenCode Go/火山，详见 `coding_plan`），未命中再视为 sub2api 自建网关；
/// 否则按平台走官方端点。**官方 OAuth 用量端点只认 OAuth token**，因此 API Key
/// 连接返回「不支持」而非「凭证失效」——后者会让用户白跑一趟重新授权。
#[tauri::command]
async fn connection_quota(app: tauri::AppHandle, db: State<'_, Db>, id: String, force: bool, max_age_ms: Option<u64>) -> Result<quota::QuotaState, String> {
    let force = refresh_mode(force, max_age_ms);
    let pool = db.0.clone();
    let lookup_id = id.clone();
    let Some(c) = with_database(pool.clone(), move |conn| {
        connections::query_creds(conn, &lookup_id)
    }).await? else {
        return Ok(quota::QuotaState::Failed { reason: "连接不存在".into() });
    };
    let expected = c.clone();
    if c.kind == "auth" && provider_browser_auth::is_browser(c.secret.as_deref()) {
        if validation::isolated() {
            return Ok(quota::QuotaState::Unauthorized { reason: "隔离验收不读取浏览器真实授权".into() });
        }
        let cache_id = id.clone();
        let state = tauri::async_runtime::spawn_blocking(move || {
            match provider_browser_auth::load(&cache_id, &c.platform, c.secret.as_deref().unwrap_or_default()) {
                Ok(credential) => cached_query(&QUOTA_CACHE, cache_id, force, quota_cache_outcome, || credential.query()),
                Err(reason) => quota::QuotaState::Unauthorized { reason },
            }
        }).await.map_err(|_| "浏览器账号查询异常")?;
        return finish_quota_query(app, pool, id, expected, state).await;
    }
    // 查询路线集中在 quota_registry：只有原工具登录走本机来源；自定义及各平台 API 连接按其实际地址查询。
    let has_base_url = c.base_url.as_deref().is_some_and(|base| !base.is_empty());
    let route = quota_registry::route(&c.platform, &c.kind, &id, has_base_url);
    use quota_registry::QuotaRoute;
    if matches!(route, QuotaRoute::NativeSource | QuotaRoute::QoderSource | QuotaRoute::ZcodeLocal | QuotaRoute::LocalLogin | QuotaRoute::NativeUnsupported) {
        if validation::isolated() {
            return Ok(quota::QuotaState::Unauthorized { reason: "隔离验收不读取原工具的真实登录".into() });
        }
        if matches!(route, QuotaRoute::NativeSource | QuotaRoute::QoderSource) {
            let cache_id = id.clone();
            let source_id = native_sources::monitor_id(&id).unwrap_or_default().to_string();
            let platform = c.platform.clone();
            let state = tauri::async_runtime::spawn_blocking(move || {
                // CLI 的登录由原工具管理，无法可靠取得身份摘要时不复用跨请求结果。
                if platform == "qoder" {
                    if source_id == "native:qoder:cn" {
                        return cached_native_quota(&QUOTA_CACHE,cache_id,force,
                            || qoder_desktop::source_identity(&source_id),
                            || qoder_quota::source_quota(&source_id));
                    }
                    return qoder_quota::source_quota(&source_id);
                }
                cached_native_quota(&QUOTA_CACHE, cache_id, force,
                    || match platform.as_str() {
                        "trae" => trae_quota::source_identity(&source_id),
                        "workbuddy" => workbuddy_quota::source_identity(&source_id),
                        _ => zcode_quota::account_identity(),
                    },
                    || match platform.as_str() {
                        "trae" => trae_quota::source_quota(&source_id),
                        "workbuddy" => workbuddy_quota::source_quota(&source_id),
                        _ => zcode_quota::local_zcode_account_quota(),
                    })
            }).await.map_err(|e| e.to_string())?;
            return finish_quota_query(app, pool, id, expected, state).await;
        }
        // ZCode 本机登录（`monitor:native:zcode:main` 等普通连接）：与 `local_zcode_quota`
        // 命令、上面 `monitor:native:zcode:account` 分支同一套缓存与退避，见
        // `cached_zcode_connection_quota`；`force` 原样透传，保持 `Refresh` 的语义
        // （Force 重查、MaxAge 按周期复用、限流退避对所有刷新都生效）。
        if route == QuotaRoute::ZcodeLocal {
            let state = tauri::async_runtime::spawn_blocking(move || {
                cached_zcode_connection_quota(
                    &QUOTA_CACHE,
                    force,
                    zcode_quota::account_identity,
                    zcode_quota::local_zcode_quota,
                )
            }).await.map_err(|e| e.to_string())?;
            return finish_quota_query(app, pool, id, expected, state).await;
        }
        // 本机登录的订阅账号：直接读原工具的登录状态查询（各模块只读、不写回凭证）
        if let Some(local) = quota_registry::local_login(&c.platform).filter(|_| route == QuotaRoute::LocalLogin) {
            // 这些来源没有保存到连接表的凭据快照，不能仅凭固定连接id缓存另一个本机登录。
            let state = tauri::async_runtime::spawn_blocking(local).await.map_err(|e|e.to_string())?;
            return finish_quota_query(app, pool, id, expected, state).await;
        }
        return Ok(quota::QuotaState::Unsupported {reason:platforms::limitation(&c.platform)});
    }
    let Some(secret) = c.secret.filter(|s| !s.trim().is_empty()) else {
        return Ok(quota::QuotaState::Unauthorized {
            reason: "该连接尚未保存凭证，请在 CC Switch 中配置后重新读取".into(),
        });
    };
    // 套餐查询辅助凭证（团队版组织/项目 ID、火山 AK/SK）在闭包外取出，闭包只借引用
    let local_account = if c.platform == "codex" && c.kind == "auth" {
        creds::local_codex_auth().and_then(|(local_token, account)| (local_token == secret).then_some(account).flatten())
    } else {
        None
    };
    let cache_id = id.clone();
    let state = tauri::async_runtime::spawn_blocking(move || cached_query(
        &QUOTA_CACHE,
        cache_id,
        force,
        quota_cache_outcome,
        || match route {
            QuotaRoute::ApiBaseUrl => {
                let base = c.base_url.as_deref().unwrap_or_default();
                // 套餐供应商优先于 sub2api 网关：按 base_url 域名识别（智谱/Kimi/MiniMax 等）
                if coding_plan::detect_provider(base).is_some() {
                    coding_plan::coding_plan_quota(base, &secret)
                } else {
                    // 未知网关只向用户配置的服务查询，不将它的密钥发送给其他供应商试探。
                    quota::sub2api_quota(base, &secret)
                }
            }
            QuotaRoute::ClaudeOauth => quota::claude_oauth_quota(&secret),
            QuotaRoute::CodexOauth => quota::codex_oauth_quota(&secret, local_account.as_deref()),
            QuotaRoute::ApiKeyNoPlan => quota::QuotaState::Unsupported {
                reason: "API Key 不提供订阅额度；将显示官方组织用量（需 Admin Key）或本机会话统计".into(),
            },
            _ => quota::QuotaState::Unsupported {
                reason: format!("{} 暂未接入官方额度查询", c.platform),
            },
        },
    )).await.map_err(|e| e.to_string())?;

    finish_quota_query(app, pool, id, expected, state).await
}

/// 没有官方额度时（不支持 / 查询失败），用网关转发时读到的限流头观测兜底（4.3）。
/// 官方结果、授权失效、权限不足、限流都原样返回：那些结论比一次旧观测更重要。
fn with_observation(id: &str, state: quota::QuotaState) -> quota::QuotaState {
    let reason = match &state {
        quota::QuotaState::Unsupported { reason } | quota::QuotaState::Failed { reason } => reason.clone(),
        _ => return state,
    };
    match crate::gateway::ratelimit::latest(id, chrono::Utc::now().timestamp_millis()) {
        Some(snapshot) => quota::QuotaState::Observed {
            windows: crate::gateway::ratelimit::windows(&snapshot),
            observed_at_ms: snapshot.observed_at_ms,
            reason,
        },
        None => state,
    }
}

/// 原生平台与保存凭证的查询共用收尾，在同一数据库锁中核对查询归属并记录状态。
async fn finish_quota_query(
    app: tauri::AppHandle, pool: Arc<Mutex<rusqlite::Connection>>, id: String,
    expected: connections::Creds, state: quota::QuotaState,
) -> Result<quota::QuotaState, String> {
    let state = with_observation(&id, state);
    let update_id = id.clone();
    let lookup = pool.clone();
    let success = matches!(state, quota::QuotaState::Ok { .. });
    let unauthorized = matches!(state, quota::QuotaState::Unauthorized { .. });
    let outcome = (success || unauthorized).then_some(success);
    let changed = with_database(pool, move |conn| connections::record_query_result(conn, &update_id, &expected, outcome)).await?;
    if changed { let _ = app.emit("connections-changed", ()); }
    // 额度快照（路由分组 smart / pace / cap hold 的数据源）：成功查询把最早的重置时间
    // 与「是否已用满」记入注册表。重置时间缺省的来源两样都不写，路由层如实降权/不降权。
    if let quota::QuotaState::Ok { windows, .. } = &state {
        if let Some(resets) = crate::gateway::quota_snapshot::earliest_reset_from_windows(windows) {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let exhausted = crate::gateway::quota_snapshot::exhausted_from_windows(windows, now_ms);
            crate::gateway::quota_snapshot::record(&id, resets, exhausted);
        }
    }
    if app.state::<Cfg>().0.get().island_connection_id.as_deref() == Some(id.as_str()) {
        update_tray_icon(&app);
    }
    // 额度主动提醒（P2-2）：将尽 / 登录失效 / 连续失败；暂停查询的账号不会走到这里
    let named = lookup.lock().ok().and_then(|conn| {
        conn.query_row("SELECT name, platform FROM connections WHERE id = ?1", [&id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).ok()
    });
    if let Some((name, platform)) = named {
        notify::check_quota(&app, &id, &name, &platform, &state);
    }
    Ok(state)
}

/// 自动化（模块 E）复用的额度查询：与供应商页同一口径与缓存，强制刷新时重新查询
pub(crate) async fn query_connection_quota(app: tauri::AppHandle, id: String, force: bool) -> Result<quota::QuotaState, String> {
    let handle = app.clone();
    connection_quota(app, handle.state::<Db>(), id, force, None).await
}

fn validate_query_range(custom: Option<db::CustomRange>, query_end_ms: i64) -> Result<(), String> {
    if query_end_ms <= 0 {
        return Err("查询结束时间无效".into());
    }
    if let Some(range) = custom {
        let end = range.end.unwrap_or(query_end_ms);
        if range.start < 0 || end > 253_402_300_799_999 {
            return Err("查询时间须在 1970 至 9999 年之间".into());
        }
        if range.start >= end {
            return Err("开始时间必须早于结束时间".into());
        }
    }
    Ok(())
}

/// 查直连 API Key 的官方组织用量与费用。普通调用 Key 返回权限不足，前端继续使用
/// 本机会话记录和估算费用；Admin Key 则优先显示官方聚合结果。
#[tauri::command]
async fn connection_api_usage(app: tauri::AppHandle, db: State<'_, Db>, id: String, force: bool, max_age_ms: Option<u64>) -> Result<quota::ApiUsageState, String> {
    let force = refresh_mode(force, max_age_ms);
    let pool = db.0.clone();
    let lookup_id = id.clone();
    let Some(c) = with_database(pool.clone(), move |conn| {
        connections::query_creds(conn, &lookup_id)
    }).await? else {
        return Ok(quota::ApiUsageState::Failed { reason: "连接不存在".into() });
    };
    let expected = c.clone();
    if c.kind != "api" {
        return Ok(quota::ApiUsageState::Unsupported {
            reason: "Auth 连接使用订阅额度与本机会话统计，不查询组织 API 用量".into(),
        });
    }
    if c.platform == "custom" {
        return Ok(quota::ApiUsageState::Unsupported { reason: platforms::limitation(&c.platform) });
    }
    if c.base_url.as_deref().is_some_and(|base| !base.is_empty()) {
        return Ok(quota::ApiUsageState::Unsupported {
            reason: "自建网关继续使用网关额度与余额接口".into(),
        });
    }
    if platforms::native(&c.platform) {
        return Ok(quota::ApiUsageState::Unsupported {reason:platforms::limitation(&c.platform)});
    }
    let Some(secret) = c.secret.filter(|s| !s.trim().is_empty()) else {
        return Ok(quota::ApiUsageState::Unauthorized {
            reason: "该连接尚未保存 API Key".into(),
        });
    };
    let cache_id = id.clone();
    let state = tauri::async_runtime::spawn_blocking(move || cached_query(
        &API_USAGE_CACHE,
        cache_id,
        force,
        api_usage_cache_outcome,
        || match c.platform.as_str() {
            "claude" | "codex" => quota::official_api_usage(&c.platform, &secret),
            platform => quota::ApiUsageState::Unsupported {
                reason: format!("{platform} 暂未接入官方组织用量查询"),
            },
        },
    )).await.map_err(|e| e.to_string())?;
    let valid = matches!(state, quota::ApiUsageState::Ok { .. });
    let unauthorized = matches!(state, quota::ApiUsageState::Unauthorized { .. });
    let outcome = (valid || unauthorized).then_some(valid);
    let changed = with_database(pool, move |conn| connections::record_query_result(conn, &id, &expected, outcome)).await?;
    if changed { let _ = app.emit("connections-changed", ()); }
    Ok(state)
}

/// 查余额。目前只有 sub2api 网关提供钱包余额；
/// 官方订阅没有「余额」概念，如实返回不支持。
#[tauri::command]
async fn connection_balance(app: tauri::AppHandle, db: State<'_, Db>, id: String, force: bool, max_age_ms: Option<u64>) -> Result<quota::BalanceState, String> {
    let force = refresh_mode(force, max_age_ms);
    let pool = db.0.clone();
    let lookup_id = id.clone();
    let Some(c) = with_database(pool.clone(), move |conn| {
        connections::query_creds(conn, &lookup_id)
    }).await? else {
        return Ok(quota::BalanceState::Failed { reason: "连接不存在".into() });
    };
    let expected = c.clone();
    // 自定义供应商复用同源余额适配器；本机订阅平台没有通用钱包余额。
    if platforms::uses_native_quota(&c.platform,&c.kind) {
        return Ok(quota::BalanceState::Unsupported {reason:platforms::limitation(&c.platform)});
    }
    let Some(secret) = c.secret.filter(|s| !s.trim().is_empty()) else {
        return Ok(quota::BalanceState::Unauthorized {
            reason: "该连接尚未保存凭证，请在 CC Switch 中配置后重新读取".into(),
        });
    };
    let Some(base) = c.base_url.filter(|base| !base.is_empty()) else {
        return Ok(quota::BalanceState::Unsupported {
            reason: "官方订阅与直连 API Key 不提供钱包余额；填写 sub2api 部署地址后可查询".into(),
        });
    };
    // 编程套餐供应商（智谱/Kimi/MiniMax 等）没有钱包余额接口；硬打 sub2api 端点
    // 只会得到 404，让用户误以为是配置错误。额度窗口请看订阅额度查询。
    if coding_plan::detect_provider(&base).is_some() {
        return Ok(quota::BalanceState::Unsupported {
            reason: "该编程套餐供应商未提供钱包余额接口，请看订阅额度窗口".into(),
        });
    }
    let cache_id = id.clone();
    let state = tauri::async_runtime::spawn_blocking(move || {
        cached_query(
            &BALANCE_CACHE,
            cache_id,
            force,
            balance_cache_outcome,
            || service_balance::query(&base, &secret),
        )
    })
        .await.map_err(|e| e.to_string())?;
    let success = matches!(state, quota::BalanceState::Ok { .. });
    let unauthorized = matches!(state, quota::BalanceState::Unauthorized { .. });
    let outcome = (success || unauthorized).then_some(success);
    let update_id = id.clone();
    let changed = with_database(pool, move |conn| connections::record_query_result(conn, &update_id, &expected, outcome)).await?;
    if changed { let _ = app.emit("connections-changed", ()); }
    // 仅仍属于当前凭证的余额可以触发提醒。
    if let quota::BalanceState::Ok { balance, currency, .. } = &state { notify::check_balance(&app, &id, &c.name, &c.platform, *balance, currency); }
    Ok(state)
}

/* ------------------------ 持久化设置（§2.6 / §7.3） ---------------------- */

#[tauri::command]
fn get_settings(cfg: State<Cfg>) -> settings::Settings {
    // 搜索 Key 在序列化时自动掩码（gateway::search::secret_keys），明文不出后端
    cfg.0.get()
}

/// 设置界面改灵动岛平台：与托盘子菜单是同一个值，改完要刷新托盘勾选态
#[tauri::command]
fn set_island_platform(
    app: tauri::AppHandle,
    cfg: State<Cfg>,
    platform: String,
) -> Result<settings::Settings, String> {
    if !platforms::known(&platform) {
        return Err("未知平台".into());
    }
    let next = cfg.0.try_update(|s| {
        s.island_platform = platform.clone();
        s.island_connection_id = None;
        s.island_connection_name = None;
        s.island_source_id = Some(format!("local:{platform}"));
    })?;
    let _ = app.emit("island-platform-changed", &platform);
    let _ = app.emit("settings-changed", &next);
    refresh_tray_menu(&app);
    Ok(next)
}

#[tauri::command]
fn set_island_source(
    app: tauri::AppHandle,
    cfg: State<Cfg>,
    id: Option<String>,
) -> Result<settings::Settings, String> {
    let platform = cfg.0.get().island_platform;
    let valid = local_source(&platform).map(|source| source.id);
    if id.is_some() && id != valid {
        return Err("所选统计来源不存在或不属于当前平台".into());
    }
    let next = cfg.0.try_update(|settings| settings.island_source_id = id)?;
    let _ = app.emit("settings-changed", &next);
    Ok(next)
}

/// `island` 为目标岛（None 为本体）。托盘摘要与主面板默认连接只跟随本体。
#[tauri::command]
async fn set_island_connection(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    cfg: State<'_, Cfg>,
    id: Option<String>,
    island: Option<String>,
) -> Result<settings::Settings, String> {
    let selected = match id.as_ref() {
        Some(id) => {
            let lookup_id = id.clone();
            let Some(c) = with_database(db.0.clone(), move |conn| {
                connections::selectable_creds(conn, &lookup_id)
            }).await? else {
                return Err("所选连接不存在".into());
            };
            let source_id = native_sources::monitor_id(id).map(str::to_string).unwrap_or_else(|| format!("local:{}", c.platform));
            Some(island_clones::SelectedConnection { id: id.clone(), name: c.name, platform: c.platform, kind: c.kind, source_id })
        }
        None => None,
    };
    let mut found = false;
    let next = cfg.0.try_update(|s| { found = island_clones::apply_connection(s, island.as_deref(), selected.as_ref()); })?;
    if !found { return Err("目标灵动岛不存在".into()); }
    let _ = app.emit("settings-changed", &next);
    if island.is_none() {
        let _ = app.emit("island-platform-changed", &next.island_platform);
        refresh_tray_menu(&app);
    }
    Ok(next)
}

#[tauri::command]
fn set_dnd(app: tauri::AppHandle, cfg: State<Cfg>, on: bool) -> Result<settings::Settings, String> {
    let next = cfg.0.try_update(|s| s.dnd = on)?;
    let _ = app.emit("dnd-changed", on);
    let _ = app.emit("settings-changed", &next);
    refresh_tray_menu(&app);
    Ok(next)
}

/* --------------------------- 本地代理（阶段二） --------------------------- */

/// 代理开关 / 端口 / 回退开关；切换即执行接管（或还原）并启动（或停止）监听。
/// 同步命令：接管与端口绑定都是毫秒级文件写与 bind，失败原因直接返回设置页。
#[tauri::command]
fn set_proxy(
    app: tauri::AppHandle,
    port: u16,
    enabled: bool,
    fallback_direct: bool,
) -> Result<settings::Settings, String> {
    proxy::set_proxy(&app, port, enabled, fallback_direct)
}

#[tauri::command]
fn proxy_status(app: tauri::AppHandle) -> proxy::ProxyStatus {
    proxy::status(&app)
}

/// 费用估算的汇率（USD → 目标币种）。带 24 小时缓存；失败时 fallback=true 用参考价。
#[derive(serde::Serialize)]
struct FxRateDto {
    currency: String,
    rate: f64,
    as_of: String,
    fallback: bool,
}

/// 设置费用估算的显示币种
#[tauri::command]
fn set_cost_currency(app: tauri::AppHandle, currency: String) -> Result<crate::settings::Settings, String> {
    if !matches!(currency.as_str(), "USD" | "CNY") {
        return Err("币种只支持 USD 或 CNY".into());
    }
    let next = app.state::<Cfg>().0.try_update(|settings| settings.cost_currency = currency.clone())?;
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

#[tauri::command]
fn fx_rate(currency: String) -> Result<FxRateDto, String> {
    if currency != "CNY" {
        return Ok(FxRateDto { currency, rate: 1.0, as_of: "基准币种".into(), fallback: false });
    }
    let client = crate::net::blocking_builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("创建汇率客户端失败：{e}"))?;
    let rate = crate::fx::usd_cny(&client);
    Ok(FxRateDto { currency, rate: rate.rate, as_of: rate.as_of, fallback: rate.fallback })
}

/* ------------------------- 账号汇聚网关（模块 J） ------------------------- */

/// 网关总览：类型定义在 service 层（CLI 也用同一份），这里只做别名。
type GatewayStatus = service::gateway::status::GatewayStatus;

/// 设置 / 清除「默认来源」。
///
/// 网关的模型名有两种写法：带前缀 `<来源>/<模型>` 按前缀路由，不带前缀时**只能**靠默认来源兜底。
/// 没有默认来源时无前缀请求会被直接拒绝（`route::split_model_id` 返回 None 后没有兜底）。
/// 这就是「一键配置后工具仍报未带来源前缀」的原因，所以必须有一个能单独设置它的入口。
///
/// 只接受**正在服务**（`serve_enabled`）的连接：指向一个没进模型池的来源，等于每条请求都失败。
/// 传 `None` 表示清除，清掉后无前缀请求重新变成明确报错，而不是静默乱路由。
#[tauri::command]
fn gateway_set_default_source(app: tauri::AppHandle, connection_id: Option<String>) -> Result<Option<String>, String> {
    if let Some(id) = connection_id.as_deref() {
        let state = app.state::<Db>();
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        let serving: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT name, slug FROM connections WHERE id = ?1 AND serve_enabled = 1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        match serving {
            None => return Err("该来源不在模型池中：请先在「管理来源」里让它加入，再设为默认来源".into()),
            Some((name, slug)) => {
                if slug.as_deref().unwrap_or("").is_empty() {
                    return Err(format!("来源「{name}」还没有 id，无法作为默认来源"));
                }
            }
        }
    }
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| {
            settings.gateway_default_connection = connection_id.clone();
        })
        .map_err(|error| format!("保存默认来源失败：{error}"))?;
    let _ = app.emit("settings-changed", next);
    Ok(connection_id)
}

#[tauri::command]
fn gateway_status(app: tauri::AppHandle) -> GatewayStatus {
    // 薄壳：取设置与连接 → 交给 service 组装（同一套逻辑也给 CLI 用）
    let settings = app.state::<Cfg>().0.get();
    let db = app.state::<Db>();
    let guard = db.0.lock();
    match guard {
        // 读不到锁时仍给一份可用快照（接线一律按未接线报），而不是让整个页面报错
        Ok(conn) => service::gateway::status::status(&settings, &conn),
        Err(_) => service::gateway::status::status_without_db(&settings),
    }
}

#[tauri::command]
fn gateway_access_key(app: tauri::AppHandle, refresh: bool) -> Result<String, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    crate::gateway::access::masked(&conn, refresh)
}

#[tauri::command]
fn gateway_copy_key(app: tauri::AppHandle) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    crate::gateway::access::copy(&conn)
}

/// 开关局域网共享（P8.3）。开启后网关（中转模式）在下次 `proxy::start` 时绑定
/// 0.0.0.0，并把鉴权切到严格校验（命名客户端 Key / 手动 Key / 接线记录，任意令牌
/// 不再放行）；关闭则恢复回环 + 宽松校验。观测模式不受影响——透传上游凭证且无
/// 鉴权，监听地址必须保持回环。
///
/// 照 `gateway_set_default_source` 的模式只改设置并广播：监听地址与鉴权档位在
/// `proxy::start` 时重新读取，运行中的网关要靠重启监听应用新值（设置页负责提示）。
/// 安全前提：开启共享前应至少签发一把命名 Key（或保留手动 Key），否则严格分支
/// 里「未开通过手动接入」的旧安装会对局域网放行任意令牌——界面批次要给出提醒。
#[tauri::command]
fn gateway_shared_lan_set(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| {
            settings.gateway_shared_lan = on;
        })
        .map_err(|error| format!("保存局域网共享设置失败：{error}"))?;
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

/// 本机 ZCode 客户端版本与 Jolu 适配版本的比对（C6）：本机更新时界面提示着装常量可能过期。
#[tauri::command]
async fn zcode_version_check() -> Result<zcode_version::VersionCheck, String> {
    tauri::async_runtime::spawn_blocking(|| zcode_version::check(zcode_version::installed_version()))
        .await
        .map_err(|_| "无法读取 ZCode 版本".to_string())
}

/// 网关中间件（选路前改写请求体）：列表按执行顺序返回。
#[tauri::command]
fn gateway_middleware_list(app: tauri::AppHandle) -> Result<Vec<crate::gateway::middleware::Middleware>, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    service::gateway::middleware::list(&conn)
}

/// 新增或覆盖一个中间件：配置先在样例请求上试跑，跑不通不保存。下一个请求即生效。
#[tauri::command]
fn gateway_middleware_save(
    app: tauri::AppHandle,
    name: String,
    kind: String,
    config: serde_json::Value,
    enabled: bool,
    sort: i64,
) -> Result<crate::gateway::middleware::Middleware, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法写入网关状态")?;
    service::gateway::middleware::save(&conn, &name, &kind, config, enabled, sort)
}

#[tauri::command]
fn gateway_middleware_delete(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法写入网关状态")?;
    service::gateway::middleware::delete(&conn, &name)
}

/// 列出命名客户端 Key：名称 + 掩码 + 限额 + 最近使用与当日计数。
/// Key 原文不出后端——列表里只有掩码，原文仅在签发结果里出现一次。
#[tauri::command]
fn gateway_client_keys_list(app: tauri::AppHandle) -> Result<Vec<crate::gateway::access::ClientKeySummary>, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    service::gateway::client_keys::list(&conn)
}

/// 签发一把命名客户端 Key，返回结果里带 Key 原文（只此一次，复制给客户端）。
/// `tokens_per_day` / `requests_per_day` 传 None 表示不限；请求数限额即时生效，
/// Token 限额由 `gateway::budget` 预扣 + 回冲强制（超量 429，次日零点重置）。
#[tauri::command]
fn gateway_client_key_issue(
    app: tauri::AppHandle,
    name: String,
    tokens_per_day: Option<i64>,
    requests_per_day: Option<i64>,
    expires_ms: Option<i64>,
    models: Option<Vec<String>>,
    read_only: Option<bool>,
) -> Result<crate::gateway::access::IssuedClientKey, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    service::gateway::client_keys::issue(
        &conn, &name, tokens_per_day, requests_per_day, expires_ms, models, read_only.unwrap_or(false),
    )
}

/// 撤销命名客户端 Key（删除行，立即失效；流水里的历史归属保留）。
#[tauri::command]
fn gateway_client_key_revoke(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    service::gateway::client_keys::revoke(&conn, &name)
}

/// 切换网关模式（observe / relay）。切到 relay 时按当前设置启动监听；
/// 切回 observe 前先还原全部接线，保证两套模式不交叉占用同一批配置文件。
#[tauri::command]
fn gateway_set_mode(
    app: tauri::AppHandle,
    mode: String,
    default_connection: Option<String>,
) -> Result<settings::Settings, String> {
    if !matches!(mode.as_str(), "observe" | "relay") {
        return Err("模式只能是 observe 或 relay".into());
    }
    let settings = app.state::<Cfg>().0.get();
    if mode == "observe" && settings.proxy_mode == "relay" {
        // 离开网关模式：先还原接线，再切模式
        crate::gateway::escape::manual(&app);
    }
    if mode == "relay" && settings.proxy_mode != "relay" {
        // 进入中转模式：先撤销观测接管（同一个端口，两种模式不能并存），
        // 再启动监听；端口被占用时 start() 会明确报错，不再悄悄换端口。
        crate::cli_switch::with_switch(|| {
            crate::proxy::stop();
            let _ = crate::proxy_config::restore_all_saved(&app);
            Ok::<(), String>(())
        })?;
    }
    let previous_mode = settings.proxy_mode.clone();
    let previous_enabled = settings.proxy_enabled;
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| {
            settings.proxy_mode = mode.clone();
            settings.gateway_default_connection = default_connection;
            if mode == "relay" {
                settings.proxy_enabled = true;
            }
        })
        .map_err(|error| format!("保存网关设置失败：{error}"))?;
    if mode == "relay" {
        if let Err(error) = crate::proxy::start(&app) {
            let restored = app.state::<Cfg>().0.update(|settings| {
                settings.proxy_mode = previous_mode;
                settings.proxy_enabled = previous_enabled;
            });
            let _ = app.emit("settings-changed", restored);
            // 之前本来在运行：按旧模式把它恢复起来，不留一个「开关是开的、监听不在」的状态。
            if previous_enabled {
                let _ = crate::proxy::set_proxy(&app, settings.proxy_port, true, settings.proxy_fallback_direct);
            }
            return Err(format!("启动模型入口失败：{error}"));
        }
    }
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

/// 隐私脱敏开关（P5，gateway/mask.rs）：只影响网关中转模式的请求 / 响应。
/// 进程级开关改完立即生效，不需要重启代理；前端接入口 = `settings.privacy_masking` 字段
/// + 本命令（界面待设计稿确认后再接）。
#[tauri::command]
fn privacy_masking_set(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| settings.privacy_masking = on)
        .map_err(|error| format!("保存隐私脱敏设置失败：{error}"))?;
    crate::gateway::mask::set_enabled(next.privacy_masking);
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

/// 隐私脱敏的处理方式与豁免清单（3.7）：先按网关引擎的规则校验，再保存并立即生效。
#[tauri::command]
fn privacy_policy_set(
    app: tauri::AppHandle,
    actions: std::collections::BTreeMap<String, String>,
    allow: Vec<String>,
) -> Result<settings::Settings, String> {
    let mut seen = std::collections::HashSet::new();
    let allow: Vec<String> = allow
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && v.chars().count() <= 4096 && seen.insert(v.to_lowercase()))
        .take(500)
        .collect();
    let policy = crate::gateway::mask::Policy::new(&actions, &allow)?;
    let actions: std::collections::BTreeMap<String, String> = actions.into_iter().filter(|(_, a)| a != "mask").collect();
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| {
            settings.privacy_actions = actions.clone();
            settings.privacy_allow = allow.clone();
        })
        .map_err(|error| format!("保存隐私脱敏设置失败：{error}"))?;
    crate::gateway::mask::set_policy(policy);
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

/// 最近的脱敏记录（只在内存，新的在前；不含原值）。
#[tauri::command]
fn privacy_records() -> Vec<crate::gateway::mask::MaskRecord> {
    crate::gateway::mask::records()
}

#[tauri::command]
fn privacy_records_clear() {
    crate::gateway::mask::clear_records();
}

/// 第一次真的遮住值：发一次系统通知并记下（之后不再提醒）。
fn privacy_first_notice(app: &tauri::AppHandle) {
    let cfg = &app.state::<Cfg>().0;
    if cfg.get().privacy_first_noticed {
        return;
    }
    if let Ok(next) = cfg.try_update(|settings| settings.privacy_first_noticed = true) {
        let _ = app.emit("settings-changed", next);
    }
    let _ = app.emit("privacy-first-mask", ());
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title("隐私脱敏已生效")
        .body("Jolu 第一次在请求离开本机前把敏感值换成了占位符，回复里会自动换回。可在设置 › 隐私脱敏查看记录。")
        .show();
}

/// 联网搜索：可选的后端清单（界面下拉用）。
#[tauri::command]
fn search_vendors() -> Vec<crate::gateway::search::VendorInfo> {
    crate::gateway::search::vendor_infos()
}

/// 联网搜索：整体保存后端配置。界面只回传掩码或新填的 Key，掩码在这里换回原文
/// （`gateway::search::merge_inputs`），返回的设置里 Key 仍是掩码。
#[tauri::command]
fn search_apis_set(app: tauri::AppHandle, apis: Vec<crate::gateway::search::SearchApiInput>) -> Result<settings::Settings, String> {
    let cfg = app.state::<Cfg>();
    let merged = crate::gateway::search::merge_inputs(&cfg.0.get().search_apis, apis)?;
    let next = cfg
        .0
        .try_update(|settings| settings.search_apis = merged)
        .map_err(|error| format!("保存搜索后端失败：{error}"))?;
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

/// 联网搜索：用已保存的配置试搜一次（依次尝试启用的后端），给界面核对 Key 是否可用。
#[tauri::command]
async fn search_try(app: tauri::AppHandle, query: String) -> Result<Vec<crate::gateway::search::SearchHit>, String> {
    let configs = app.state::<Cfg>().0.get().search_apis;
    crate::gateway::search::search_in_order(&configs, query.trim(), 5).await
}

/// ZCode Start Plan 优先开关（③5，gateway/adapter/zcode.rs）：开启后 ZCode 来源
/// 先试免费的 Start Plan 通道（会话 JWT），非 2xx 再落 Coding Plan Key；关闭时
/// 保持现状（Coding Plan Key 先、429/402 余额类失败落 Start Plan）。进程级开关
/// 改完立即生效，不需要重启代理；前端接入口 = `settings.zcode_free_first` 字段
/// + 本命令（界面接入口待设计稿确认后再接）。
#[tauri::command]
fn zcode_free_first_set(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| settings.zcode_free_first = on)
        .map_err(|error| format!("保存 ZCode Start Plan 优先设置失败：{error}"))?;
    crate::gateway::adapter::zcode::set_free_first(next.zcode_free_first);
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

/// 连接进出模型池。
#[tauri::command]
fn gateway_set_serve(app: tauri::AppHandle, connection_id: String, enabled: bool) -> Result<(), String> {
    let state = app.state::<Db>();
    let db = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    // 选择来源和调用就绪分开：允许预先选择订阅，实际请求仍由对应适配器核实授权。
    connections::set_serve(&db, &connection_id, enabled)
}

/// 批量加入 / 移出模型池的单项结果。
#[derive(serde::Serialize)]
struct ServeBatchItem {
    connection_id: String,
    ok: bool,
    error: Option<String>,
}

/// 批量加入 / 移出模型池（管理来源弹窗）：逐个执行，单个失败不影响其他来源，结果逐项返回。
#[tauri::command]
fn gateway_set_serve_many(app: tauri::AppHandle, connection_ids: Vec<String>, enabled: bool) -> Result<Vec<ServeBatchItem>, String> {
    let state = app.state::<Db>();
    let db = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    Ok(connection_ids
        .into_iter()
        .map(|id| match connections::set_serve(&db, &id, enabled) {
            Ok(()) => ServeBatchItem { connection_id: id, ok: true, error: None },
            Err(error) => ServeBatchItem { connection_id: id, ok: false, error: Some(error) },
        })
        .collect())
}

/// 各来源经网关的使用情况（路由流水保留范围内）：最近使用时间、请求数与失败数。
#[tauri::command]
fn gateway_source_usage(app: tauri::AppHandle) -> Result<Vec<crate::gateway::route_log::SourceUsage>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::route_log::source_usage(&conn).map_err(|error| error.to_string())
}

#[tauri::command]
fn gateway_source_capabilities(app: tauri::AppHandle) -> Result<Vec<crate::gateway::subscription::Capability>, String> {
    let state = app.state::<Db>();
    let db = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::subscription::capabilities(&db)
}

/// 接入 / 断开单个工具。
///
/// `model` 只对 Claude Code 有意义，是**主模型**：接线时写入 settings.json 顶层 `model`、
/// `ANTHROPIC_SMALL_FAST_MODEL` 与全部档位 env（opus/sonnet/haiku/fable 默认跟随主模型），
/// 并移除 `ANTHROPIC_MODEL`（它压过 /model 的选择）与各档旧的「关于」键；断开时按快照还原。
/// 不传（或传空）时只接地址和令牌，用户的模型配置原样保留。
/// Library（批 5）：把「Jolu 搜索」以 MCP server 装进指定 agent。
#[tauri::command]
fn library_install(app: tauri::AppHandle, agent: String) -> Result<library::InstallOutcome, String> {
    // 桌面端场景：current_exe 就是主程序 jolu.exe（自带 --mcp-search 分支）
    let item = library::search_item_with(&std::env::current_exe().map_err(|error| error.to_string())?);
    let outcome = crate::cli_switch::with_switch(|| {
        let db = app.state::<Db>();
        let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
        library::install(&conn, &agent, &item).map_err(|error| error.to_string())
    })
    .map_err(|error| format!("另一实例正在修改工具配置：{error}"))?;
    let _ = app.emit("library-changed", ());
    Ok(outcome)
}

/// Library：卸载指定 agent 里的一个库项（只删 Jolu 自己装的）。
#[tauri::command]
fn library_remove(app: tauri::AppHandle, agent: String, name: String) -> Result<bool, String> {
    let removed = crate::cli_switch::with_switch(|| {
        let db = app.state::<Db>();
        let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
        library::remove(&conn, &agent, &name).map_err(|error| error.to_string())
    })
    .map_err(|error| format!("另一实例正在修改工具配置：{error}"))?;
    let _ = app.emit("library-changed", ());
    Ok(removed)
}

/// Library：每个 agent 的安装状态（装了什么 + 配置里现状），供 UI 列表用。
#[derive(serde::Serialize)]
struct LibraryAgentStatus {
    agent: String,
    installed: Vec<String>,
    /// 配置文件可定位且存在；false = 从未初始化（装了会新建文件）
    config_present: bool,
    /// 能不能装；不能时 `note` 写原因
    supported: bool,
    /// 不支持的原因，或装好后用户还要做的事
    note: Option<String>,
}

#[tauri::command]
fn library_list(app: tauri::AppHandle) -> Result<Vec<LibraryAgentStatus>, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let mut statuses = Vec::new();
    for agent in library::LIBRARY_AGENTS {
        let unsupported = library::unsupported_reason(agent);
        let config_present = library::mcp_config_path(agent).is_some_and(|path| crate::steady::exists(&path));
        statuses.push(LibraryAgentStatus {
            agent: agent.to_string(),
            installed: library::applied_names(&conn, agent),
            config_present,
            supported: unsupported.is_none(),
            note: unsupported.or_else(|| library::install_note(agent)).map(str::to_string),
        });
    }
    Ok(statuses)
}

/// 命名客户端 Key 的同时请求数上限：None = 不限，1–1000。立即对之后到达的请求生效。
#[tauri::command]
fn gateway_client_key_concurrency_set(app: tauri::AppHandle, name: String, cap: Option<i64>) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::access::client_key_set_concurrency(&conn, &name, cap)
}

/// 命名客户端 Key 的 Token / 费用上限与账号范围（2.6）：上限留空即不限，账号范围为空即不限。
#[tauri::command]
fn gateway_client_key_limits_set(
    app: tauri::AppHandle,
    name: String,
    budget: crate::gateway::budget::KeyBudget,
    accounts: Option<Vec<String>>,
) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::access::client_key_set_limits(&conn, &name, &budget, accounts)
}

/// 立即刷新 models.dev 模型元数据（上下文窗口、显示名、推理档位等），返回（模型数，拉取时间）。
#[tauri::command]
async fn gateway_models_dev_refresh(app: tauri::AppHandle) -> Result<(usize, i64), String> {
    tauri::async_runtime::spawn_blocking(move || crate::gateway::models_dev::refresh_now(&app))
        .await
        .map_err(|error| format!("刷新任务失败：{error}"))?
}

/// 模型元数据现状：缓存的拉取时间（0 = 还在用内置快照）与模型数。
#[tauri::command]
fn gateway_models_dev_status(app: tauri::AppHandle) -> (i64, usize) {
    crate::gateway::models_dev::cached(&app).map(|c| (c.fetched_ms, c.models.len())).unwrap_or((0, 0))
}

/// 局域网共享的来源地址白名单（P2-9）：IP 或 CIDR，空数组 = 不限；写错的一项会整体拒存。
/// 立即生效（不用重启网关）；`JOLU_ALLOW_IPS` 环境变量设了时以它为准，这里只保存。
#[tauri::command]
fn gateway_allow_ips_set(app: tauri::AppHandle, list: Vec<String>) -> Result<settings::Settings, String> {
    let list: Vec<String> = list.iter().map(|item| item.trim().to_string()).filter(|item| !item.is_empty()).collect();
    crate::gateway::allow::parse(&list.join(","))?;
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| settings.gateway_allow_ips = list.clone())
        .map_err(|error| format!("保存白名单失败：{error}"))?;
    crate::gateway::allow::load(&next.gateway_allow_ips)?;
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

/// 白名单是否被 `JOLU_ALLOW_IPS` 环境变量接管（界面提示用）。
#[tauri::command]
fn gateway_allow_ips_from_env() -> bool {
    crate::gateway::allow::from_env()
}

/// 接线前的提醒（只读）：比如 Claude Code 同时设了顶层 model 与 env.ANTHROPIC_MODEL。
#[tauri::command]
fn gateway_wiring_preflight(agent: String) -> Vec<String> {
    crate::gateway::wiring::preflight_notices(&agent)
}

/// 接线改动预览（F6，画板 V5-1 / V5-2）：已接入时给「Jolu 改了什么」，接入前给「将要写成什么」。
/// 只读，不写文件；用户配置里的密钥在后端遮住。
#[tauri::command]
async fn gateway_wiring_preview(
    app: tauri::AppHandle,
    agent: String,
    model: Option<String>,
    subagent_model: Option<String>,
    tiers: Option<crate::gateway::wiring_store::TierModels>,
) -> Result<crate::gateway::escape::WiringPreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::gateway::escape::preview_agent(&app, &agent, model.as_deref(), subagent_model.as_deref(), tiers.as_ref())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Claude Code 长对话压缩（B5，画板 V5-1「高级」）：`""` 默认 272K、`"full"` 完整窗口、其余为自定义大小。
/// 保存后若 Claude Code 已接入，按新设置重写 `CLAUDE_CODE_AUTO_COMPACT_WINDOW`。
#[tauri::command]
fn gateway_long_context_set(app: tauri::AppHandle, value: String) -> Result<String, String> {
    let value = value.trim().to_string();
    let normalized = match value.to_ascii_lowercase().as_str() {
        "" | "default" => String::new(),
        "full" => "full".to_string(),
        other => {
            crate::gateway::escape::parse_tokens(other).ok_or("长对话大小写成 272k、1M 或 400000 这样的格式")?;
            value.clone()
        }
    };
    app.state::<Cfg>().0.try_update(|settings| settings.claude_long_context = normalized.clone())?;
    let applied = app
        .state::<Db>()
        .0
        .lock()
        .ok()
        .and_then(|conn| crate::gateway::wiring_store::load(&conn, "claude-code"))
        .filter(|record| record.state == "wired")
        .and_then(|record| record.applied());
    match applied {
        Some(applied) if applied.model.is_some() => {
            gateway_wiring_set_inner(&app, "claude-code".into(), true, applied.model.as_deref(), applied.subagent_model.as_deref(), applied.tiers.as_ref())?;
            let _ = app.emit("gateway-wiring-changed", ());
            Ok("已保存，并已更新 Claude Code 的配置".into())
        }
        _ => Ok("已保存；下次接入 Claude Code 时生效".into()),
    }
}

/// 接线记录里实际写入的模型配置（Claude Code 的主模型、子代理、档位），给接线弹窗回填。
#[derive(serde::Serialize)]
struct WiringAppliedView {
    model: Option<String>,
    subagent_model: Option<String>,
    tiers: Option<crate::gateway::wiring_store::TierModels>,
}

#[tauri::command]
fn gateway_wiring_applied(app: tauri::AppHandle, agent: String) -> Result<Option<WiringAppliedView>, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
    Ok(crate::gateway::wiring_store::load(&conn, &agent)
        .filter(|record| record.state == "wired")
        .and_then(|record| record.applied())
        .map(|applied| WiringAppliedView { model: applied.model, subagent_model: applied.subagent_model, tiers: applied.tiers }))
}

#[tauri::command]
fn gateway_wiring_set(
    app: tauri::AppHandle,
    agent: String,
    wired: bool,
    model: Option<String>,
    subagent_model: Option<String>,
    // 档位自选（批 4）：每档的最终写入值（可带 :effort）；None 的档位跟随主模型。
    tiers: Option<crate::gateway::wiring_store::TierModels>,
) -> Result<String, String> {
    if let Some(tiers) = tiers.as_ref() {
        // 档位 effort 先看 models.dev 记录的档位（2.8），没有再按内置表
        let catalog = crate::gateway::models_dev::cached(&app);
        tiers.validate_with(&|model: &str| {
            let bare = model.rsplit('/').next().unwrap_or(model).trim_end_matches("[1m]");
            catalog.as_ref().and_then(|c| crate::gateway::models_dev::lookup_meta(c, bare)).map(|meta| meta.efforts)
        })?;
    }
    // 手动改接线：之后的请求不再算「来自 Profile」
    if let Ok(conn) = app.state::<Db>().0.lock() {
        crate::gateway::profiles::set_active(&conn, None);
    }
    let result = gateway_wiring_set_inner(&app, agent, wired, model.as_deref(), subagent_model.as_deref(), tiers.as_ref());
    let _ = app.emit("gateway-wiring-changed", ());
    result
}

fn gateway_wiring_set_inner(
    app: &tauri::AppHandle,
    agent: String,
    wired: bool,
    model: Option<&str>,
    subagent_model: Option<&str>,
    tiers: Option<&crate::gateway::wiring_store::TierModels>,
) -> Result<String, String> {
    let app = app.clone();
    if wired {
        let wired_agents = crate::gateway::escape::wire_agents(&app, std::slice::from_ref(&agent), model, subagent_model, tiers)?;
        Ok(format!("已接入 {}", wired_agents.join("、")))
    } else {
        let settings = app.state::<Cfg>().0.get();
        crate::cli_switch::with_switch(|| {
            let record = app
                .state::<Db>()
                .0
                .lock()
                .ok()
                .and_then(|conn| crate::gateway::wiring_store::load(&conn, &agent));
            let restored = crate::gateway::wiring::unwire(&agent, settings.proxy_port, record.as_ref())?;
            if restored {
                if let Ok(conn) = app.state::<Db>().0.lock() {
                    let _ = crate::gateway::wiring_store::mark_idle(&conn, &agent);
                }
                let _ = app.state::<Cfg>().0.try_update(|settings| {
                    settings.gateway_wiring.remove(&agent);
                });
                Ok(format!("已断开 {agent}"))
            } else {
                Ok(format!("{agent} 的配置已被外部修改，未覆盖"))
            }
        })
    }
}

/// 逃生门：切回直连（还原全部接线 + 停网关）。
#[tauri::command]
fn gateway_escape(app: tauri::AppHandle) -> String {
    let summary = crate::gateway::escape::manual(&app).summary();
    let _ = app.emit("gateway-wiring-changed", ());
    summary
}

#[tauri::command]
fn gateway_groups(app: tauri::AppHandle) -> Result<crate::gateway::groups::GroupsDto, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::list(&conn)
}

#[tauri::command]
fn gateway_group_create(
    app: tauri::AppHandle,
    kind: String,
    name: String,
) -> Result<crate::gateway::groups::GroupDto, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::create(&conn, &kind, &name)
}

#[tauri::command]
fn gateway_group_rename(app: tauri::AppHandle, kind: String, id: String, name: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::rename(&conn, &kind, &id, &name)
}

#[tauri::command]
fn gateway_group_delete(app: tauri::AppHandle, kind: String, id: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::delete(&conn, &kind, &id)
}

#[tauri::command]
fn gateway_group_reorder(app: tauri::AppHandle, kind: String, ids: Vec<String>) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::reorder(&conn, &kind, &ids)
}

#[tauri::command]
fn gateway_group_move(
    app: tauri::AppHandle,
    kind: String,
    member: String,
    group_id: Option<String>,
) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::move_member(&conn, &kind, &member, group_id.as_deref())
}

/// 改菜单 id。旧 id 会进入历史，之后请求旧 id 会得到 404 说明现在叫什么。
#[tauri::command]
fn gateway_set_slug(app: tauri::AppHandle, connection_id: String, slug: String) -> Result<String, String> {
    let state = app.state::<Db>();
    let (old, new, record) = {
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        let old: Option<String> = conn
            .query_row("SELECT slug FROM connections WHERE id = ?1", [&connection_id], |r| r.get(0))
            .ok()
            .flatten();
        let new = connections::set_slug(&conn, &connection_id, &slug)?;
        (old, new, crate::gateway::wiring_store::load(&conn, "claude-code"))
    };
    // 2.7：Claude Code 的接线里写着 `旧id/模型` 时一并改成新 id 重写，否则改名后工具请求全是 404
    if let (Some(old), Some(record)) = (old.filter(|o| *o != new), record.filter(|r| r.state == "wired")) {
        if let Some((model, subagent, tiers)) = record.applied().and_then(|applied| crate::gateway::wiring_store::renamed_slug(&applied, &old, &new)) {
            let model = model.map(|m| m.trim_end_matches("[1m]").trim_end_matches("[1M]").to_string());
            if let Err(error) = crate::gateway::escape::wire_agents(&app, &["claude-code".to_string()], model.as_deref(), subagent.as_deref(), tiers.as_ref()) {
                eprintln!("[接线] 来源改 id 后重写 Claude Code 接线失败：{error}");
                return Err(format!("菜单 id 已改为「{new}」，但 Claude Code 的接线没能同步更新：{error}。请在接线面板重新接线"));
            }
            let _ = app.emit("gateway-wiring-changed", ());
        }
    }
    Ok(new)
}

/// 模型清单：只读数据库缓存，不打上游。
///
/// 薄壳：models.dev 元数据的「刷新触发 + 取缓存」依赖 `AppHandle`，留在这一侧；
/// 目录组装与合并交给 service（CLI 用同一份，只是不触发刷新）。
#[tauri::command]
fn gateway_catalog(app: tauri::AppHandle) -> Result<Vec<crate::gateway::catalog::CatalogModel>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    // models.dev 元数据并入（P4.2）：只读缓存，过期时后台刷新——目录命令不阻塞 20 秒
    crate::gateway::models_dev::refresh_if_stale(&app);
    let meta_catalog = crate::gateway::models_dev::cached(&app);
    service::gateway::catalog::list(&conn, &|model: &str| {
        meta_catalog
            .as_ref()
            .and_then(|catalog| crate::gateway::models_dev::lookup_meta(catalog, model))
    })
}

/// Profile 列表。
#[tauri::command]
fn gateway_profiles_list(app: tauri::AppHandle) -> Result<Vec<crate::gateway::profiles::Profile>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::profiles::list(&conn).map_err(|error| error.to_string())
}

/// 保存当前已接线 agent 的主模型为一个 Profile。
#[tauri::command]
fn gateway_profiles_save(app: tauri::AppHandle, name: String) -> Result<crate::gateway::profiles::Profile, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let agents = crate::gateway::profiles::capture_current(&conn)?;
    let default_connection = app.state::<Cfg>().0.get().gateway_default_connection;
    let extras = crate::gateway::profiles::capture_extras(&conn, default_connection, chrono::Utc::now().timestamp_millis());
    crate::gateway::profiles::save_with(&conn, &name, &agents, &extras)
}

/// 删除 Profile。
#[tauri::command]
fn gateway_profiles_delete(app: tauri::AppHandle, name: String) -> Result<bool, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::profiles::delete(&conn, &name)
}

/// 应用 Profile：对「当前已接线且在 Profile 里」的 agent 逐个重放接线。
///
/// 走 [`gateway::escape::wire_agents`] 的完整路径（备份 / 原子写 / 快照 / 1M 标记）；
/// 未接线与配置漂移的 agent 跳过并说明——Profile 不静默覆盖用户的手动改动。
#[tauri::command]
fn gateway_profiles_apply(app: tauri::AppHandle, name: String) -> Result<crate::gateway::profiles::ApplyResult, String> {
    let state = app.state::<Db>();
    let profile = {
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        crate::gateway::profiles::get(&conn, &name)?.ok_or_else(|| format!("没有名为「{name}」的 Profile"))?
    };
    // 当前已接线且有主模型决策的 agent 才参与切换
    let wired = {
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        crate::gateway::profiles::capture_current(&conn)?
    };
    let mut switched = Vec::new();
    let mut skipped = Vec::new();
    for (agent, model) in &profile.agents {
        if !wired.contains_key(agent) {
            skipped.push(agent.clone());
            continue;
        }
        // Profile 存了档位与子代理就用它的；旧 Profile 没存时保留该 agent 现有的（tier_own）——
        // 以前这里传空子代理，每次切换都会把用户的子代理模型删掉
        let current = {
            let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
            crate::gateway::wiring_store::load(&conn, agent).and_then(|record| record.applied())
        };
        let saved = profile.extras.agents.get(agent);
        let tiers = saved.and_then(|s| s.tiers.clone()).or_else(|| current.as_ref().and_then(|a| a.tiers.clone()));
        let subagent = saved.map(|s| s.subagent_model.clone()).unwrap_or_else(|| current.as_ref().and_then(|a| a.subagent_model.clone()));
        match crate::gateway::escape::wire_agents(&app, std::slice::from_ref(agent), Some(model), subagent.as_deref(), tiers.as_ref()) {
            Ok(_) => switched.push(format!("{agent} → {model}")),
            Err(error) => skipped.push(format!("{agent}（{error}）")),
        }
    }
    if switched.is_empty() {
        return Err(format!(
            "没有可切换的 agent：{}",
            if skipped.is_empty() { "Profile 里没有已接线的成员".into() } else { format!("跳过：{}", skipped.join("、")) }
        ));
    }
    // 默认来源：Profile 存了就切过去（裸名兜底跟着 Profile 走）
    if let Some(default_connection) = profile.extras.default_connection.clone() {
        let _ = app.state::<Cfg>().0.try_update(|settings| settings.gateway_default_connection = Some(default_connection));
    }
    {
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        crate::gateway::profiles::set_active(&conn, Some(&profile.name));
    }
    let mut message = format!("已切换 {}", switched.join("、"));
    if !skipped.is_empty() {
        message.push_str(&format!("；跳过 {}", skipped.join("、")));
    }
    Ok(crate::gateway::profiles::ApplyResult { switched, skipped, message })
}

/// 本机导入扫描：CC Switch / Claude Code / Codex 的现有上游读成草稿。
/// 键不出后端——草稿里只有掩码，落库由 `providers_import_apply` 重读原文。
#[tauri::command]
fn providers_import_scan() -> Result<serde_json::Value, String> {
    crate::gateway::import_sources::scan()
}

/// 按引用落库导入的连接（走既有 `connections::add`：加密 / slug / 掩码全在正路里）。
#[tauri::command]
async fn providers_import_apply(
    app: tauri::AppHandle,
    db: tauri::State<'_, Db>,
    references: Vec<String>,
) -> Result<Vec<String>, String> {
    let pool = db.0.clone();
    let created = with_database(pool, move |conn| {
        crate::gateway::import_sources::apply(conn, &references)
    })
    .await?;
    let _ = app.emit("connections-changed", ());
    Ok(created)
}

/// 路由分组列表。
#[tauri::command]
fn gateway_routing_group_list(app: tauri::AppHandle) -> Result<Vec<crate::gateway::routing_groups::RoutingGroup>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::routing_groups::list(&conn).map_err(|error| error.to_string())
}

/// 创建路由分组。成员是 `[{slug, model}]`；slug 为来源 id，模型可带或不带 `[1m]` 标记。
#[tauri::command]
fn gateway_routing_group_create(
    app: tauri::AppHandle,
    slug: String,
    name: String,
    strategy: String,
    members: Vec<crate::gateway::routing_groups::GroupMember>,
    intent: Option<crate::gateway::routing_groups::IntentConfig>,
) -> Result<crate::gateway::routing_groups::RoutingGroup, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let group = crate::gateway::routing_groups::create(&conn, &slug, &name, &strategy, &members, intent.as_ref())?;
    let _ = app.emit("gateway-catalog-changed", ());
    Ok(group)
}

/// 更新路由分组（名称 / 策略 / 成员 / 意图配置，均可选）。
#[tauri::command]
fn gateway_routing_group_update(
    app: tauri::AppHandle,
    slug: String,
    name: Option<String>,
    strategy: Option<String>,
    members: Option<Vec<crate::gateway::routing_groups::GroupMember>>,
    intent: Option<crate::gateway::routing_groups::IntentConfig>,
) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::routing_groups::update(&conn, &slug, name.as_deref(), strategy.as_deref(), members.as_deref(), intent.as_ref())?;
    let _ = app.emit("gateway-catalog-changed", ());
    Ok(())
}

/// 删除路由分组。
#[tauri::command]
fn gateway_routing_group_delete(app: tauri::AppHandle, slug: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::routing_groups::delete(&conn, &slug)?;
    let _ = app.emit("gateway-catalog-changed", ());
    Ok(())
}

/// 路由流水：最近 20 条网关请求（不含提示词与凭证）。
#[tauri::command]
fn gateway_route_log(app: tauri::AppHandle) -> Result<Vec<crate::gateway::route_log::RouteLogEntry>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::route_log::recent(&conn, 20).map_err(|error| error.to_string())
}

/// 路由流水分页（统一网关）：`upper_id` 为空时以当前最新一条建立查询范围，翻页时带回同一上界。
#[tauri::command]
fn gateway_route_log_page(
    app: tauri::AppHandle,
    page: i64,
    page_size: i64,
    upper_id: Option<i64>,
) -> Result<crate::gateway::route_log::RouteLogPage, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::route_log::page(&conn, page, page_size, upper_id).map_err(|error| error.to_string())
}

/// 统一网关「模型来源 → 最近会话」：在途快照（带序号）+ 经网关的会话及其来源关联、权威 Token。
#[tauri::command]
fn gateway_sessions_overview(app: tauri::AppHandle) -> Result<crate::gateway::sessions::SessionsOverview, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::sessions::overview(&conn).map_err(|error| error.to_string())
}

/// 今日（本地零点起）经网关的请求数：(ok, rejected, failed)。
#[derive(serde::Serialize)]
struct GatewayTodayCounts { since: i64, at: i64, ok: i64, rejected: i64, failed: i64 }

#[tauri::command]
fn gateway_today_counts(app: tauri::AppHandle) -> Result<GatewayTodayCounts, String> {
    let since = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|midnight| midnight.and_local_timezone(chrono::Local).earliest())
        .map(|midnight| midnight.timestamp_millis())
        .unwrap_or(0);
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let (ok, rejected, failed) = crate::gateway::route_log::counts_since(&conn, since).map_err(|error| error.to_string())?;
    // 计数时刻：前端据此只补加之后才结束的实时请求
    let at = chrono::Local::now().timestamp_millis();
    Ok(GatewayTodayCounts { since, at, ok, rejected, failed })
}

/* ─────────────────────────── 通用接入（P1-1） ─────────────────────────── */

#[tauri::command]
fn custom_agents_list(app: tauri::AppHandle) -> Result<Vec<crate::custom_agents::CustomAgentView>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    Ok(crate::custom_agents::list(&conn).map_err(|e| e.to_string())?.iter().map(crate::custom_agents::view).collect())
}

#[tauri::command]
fn custom_agents_add(app: tauri::AppHandle, input: crate::custom_agents::CustomAgentInput) -> Result<crate::custom_agents::CustomAgentView, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let agent = crate::custom_agents::add(&conn, &input, chrono::Local::now().timestamp_millis())?;
    Ok(crate::custom_agents::view(&agent))
}

#[tauri::command]
fn custom_agents_remove(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::custom_agents::remove(&conn, &id)
}

fn custom_backup_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path().app_data_dir().map(|dir| dir.join("gateway-backups")).map_err(|e| format!("无法定位应用数据目录：{e}"))
}

#[tauri::command]
fn custom_agents_wire(app: tauri::AppHandle, id: String, model: Option<String>) -> Result<crate::custom_agents::CustomAgentView, String> {
    let settings = app.state::<Cfg>().0.get();
    let (mode, _) = crate::gateway::mode_from(&settings);
    if !settings.proxy_enabled || mode != crate::gateway::Mode::Relay {
        return Err("模型路由没有运行：请先开启模型路由，再接入工具".into());
    }
    let backup = custom_backup_dir(&app)?;
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let view = crate::custom_agents::wire(&conn, &id, settings.proxy_port, &settings.instance_id, model.as_deref(), &backup)?;
    drop(conn);
    let _ = app.emit("gateway-wiring-changed", ());
    Ok(view)
}

#[tauri::command]
fn custom_agents_unwire(app: tauri::AppHandle, id: String) -> Result<bool, String> {
    let backup = custom_backup_dir(&app)?;
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let changed = crate::custom_agents::unwire(&conn, &id, &backup)?;
    drop(conn);
    let _ = app.emit("gateway-wiring-changed", ());
    Ok(changed)
}

#[tauri::command]
fn custom_agents_preview(app: tauri::AppHandle, id: String, model: Option<String>) -> Result<crate::gateway::escape::WiringPreview, String> {
    let settings = app.state::<Cfg>().0.get();
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::custom_agents::preview(&conn, &id, settings.proxy_port, &settings.instance_id, model.as_deref())
}

/// 本机的局域网地址（画板 V5-4「地址」行）：按路由表选出的出口 IPv4。UDP connect 不发任何包，
/// 只让系统选一个出口地址；拿不到（没有网络）返回 None。
#[tauri::command]
fn gateway_lan_address() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then(|| ip.to_string())
}

/// 模型路由概览（画板 V5-3 四项指标、Agent 页「今日路由」）：按请求而不是按流水行计。
#[tauri::command]
fn gateway_day_overview(app: tauri::AppHandle) -> Result<crate::gateway::route_log::DayOverview, String> {
    let today = chrono::Local::now().date_naive();
    let start = |day: chrono::NaiveDate| {
        day.and_hms_opt(0, 0, 0)
            .and_then(|midnight| midnight.and_local_timezone(chrono::Local).earliest())
            .map(|midnight| midnight.timestamp_millis())
            .unwrap_or(0)
    };
    let yesterday = today.pred_opt().unwrap_or(today);
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::route_log::day_overview(&conn, start(today), start(yesterday)).map_err(|error| error.to_string())
}

/// 接受接线漂移：只清接线记录，不改用户的配置文件。
#[tauri::command]
fn gateway_wiring_accept(app: tauri::AppHandle, agent: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::wiring_store::mark_idle(&conn, &agent).map_err(|error| error.to_string())?;
    drop(conn);
    let _ = app.state::<Cfg>().0.try_update(|settings| {
        settings.gateway_wiring.remove(&agent);
    });
    let _ = app.emit("gateway-wiring-changed", ());
    Ok(())
}

/* --------------------------- 连接管理（§6.3） --------------------------- */

#[tauri::command]
async fn list_connections(db: State<'_, DbRead>) -> Result<Vec<connections::ConnectionDto>, String> {
    with_reader(db.0.clone(), move |conn| {
        connections::list(conn).map_err(|e| e.to_string())
    })
    .await
}

/// 刷新某个来源的模型清单并落库（写 `connections.models_json`）。
///
/// 为什么需要单独一条命令：界面读的 `catalog::list` 只读缓存、从不打上游，所以缓存为空时
/// 永远显示 0 模型。两类来源走两条路：
///
/// - **API 连接**（`kind = api`）：直接在原进程按保存的地址与协议打 `<base>/v1/models`，
///   与「获取模型」同一个 `fetch_models`。从本机导入的连接、以及手填地址但当时模型接口
///   不通的连接，就靠这条补上清单。
/// - **订阅连接**（`kind = auth`）：交给各平台适配器（zcode / grok / gemini / qoder /
///   trae / workbuddy）。适配器的模型路径只碰 db 与 HTTP 客户端，网关没在监听时用
///   [`proxy::detached_deps`] 兜住，账号卡随时能拉。
///
/// 只从已存连接取凭证，不接受前端传入的 Key；凭据不出后端。
#[tauri::command]
async fn gateway_refresh_models(app: tauri::AppHandle, connection_id: String) -> Result<Vec<String>, String> {
    // 订阅适配器要按 PoolEntry 读凭证（qoder / trae），API 连接只要地址与密钥
    let (platform, kind, slug, name, base_url, secret, protocol, account_fp, model, status) = {
        let state = app.state::<Db>();
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        conn.query_row(
            "SELECT platform, kind, slug, name, base_url, secret, protocol, account_fp, model, status
             FROM connections WHERE id = ?1",
            [&connection_id],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, String>(9)?,
            )),
        )
        .map_err(|_| "来源不存在".to_string())?
    };
    let fetched = if kind == "api" {
        let base = base_url.unwrap_or_default();
        let secret = secret.unwrap_or_default();
        if base.trim().is_empty() || secret.trim().is_empty() {
            return Err("这个连接没有可用的地址或密钥，请先编辑连接补全".into());
        }
        let platform_for_fetch = platform.clone();
        tauri::async_runtime::spawn_blocking(move || {
            connections::fetch_models(&platform_for_fetch, protocol.as_deref(), &base, &secret)
        })
        .await
        .map_err(|error| error.to_string())??
    } else {
        // 订阅连接：状态不是可用时先让用户重新登录，拿旧凭证打上游只会得到 401
        if !matches!(status.as_str(), "connected" | "ok" | "valid") {
            return Err("该来源当前不可用：请先重新登录或检查凭证".into());
        }
        // 订阅适配器的模型路径只用 db 与 HTTP 客户端（缓存读写、凭证读取、快照比对都
        // 打在真实连接上，不查池索引），网关没在监听时给一份不接监听的 deps，账号卡的
        // 「拉取清单」就不必先开「允许接入」；转发路径仍只能用运行中的 deps（running_deps）。
        let deps = crate::proxy::running_deps().unwrap_or_else(|| {
            let db = app.state::<Db>().0.clone();
            crate::proxy::detached_deps(db)
        });
        // qoder / trae 的适配器按 PoolEntry 取凭证与站点配置；拼法与 load_pool 一致
        let entry = crate::gateway::route::PoolEntry {
            connection_id: connection_id.clone(),
            slug: slug.unwrap_or_default(),
            name,
            platform: platform.clone(),
            kind: kind.clone(),
            protocol: connections::auth_protocol(&platform, protocol.as_deref()).to_string(),
            base_url: base_url.clone().map(|b| b.trim().trim_end_matches('/').to_string()),
            secret,
            account_fp,
            model,
            status: status.clone(),
        };
        match platform.as_str() {
            "zcode" => crate::gateway::adapter::zcode::models(&deps, &connection_id).await,
            "grok" => crate::gateway::adapter::grok::models(&deps, &connection_id).await,
            "gemini" => crate::gateway::adapter::gemini_subscription::models(&deps, &connection_id).await,
            "qoder" => crate::gateway::adapter::qoder::models(&deps, &entry).await,
            "trae" => crate::gateway::adapter::trae_models::models(&deps, &entry).await,
            "workbuddy" => crate::gateway::adapter::workbuddy::models(&deps, &connection_id).await,
            _ => Err(format!("该平台没有可实时拉取的模型清单（{platform}）")),
        }?
    };
    if fetched.is_empty() {
        return Err("上游返回了空清单：请确认该账号已开通可用的模型".into());
    }
    // qoder / trae / workbuddy 的适配器在拉取时已经把**富缓存**写进 models_json
    // （凭证指纹守卫 + 带官方 display_name 的条目），这里再用纯 key 数组覆盖一次
    // 会把 display_name 砸掉、指纹校验也会失败——账号卡将退回内置清单。
    // 只有适配器不自管缓存的平台（zcode / grok / gemini 与 API 连接）才在这里落库。
    if !matches!(platform.as_str(), "qoder" | "trae" | "workbuddy") {
        let state = app.state::<Db>();
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        let json = serde_json::to_string(&fetched).map_err(|error| format!("清单序列化失败：{error}"))?;
        conn.execute(
            "UPDATE connections SET models_json = ?2 WHERE id = ?1",
            rusqlite::params![connection_id, json],
        )
        .map_err(|error| format!("保存模型清单失败：{error}"))?;
    }
    let _ = app.emit("gateway-catalog-changed", ());
    Ok(fetched)
}

/// 「获取模型」（画布 18）：用用户填写的地址与 Key 拉取网关模型列表。
/// Key 只在本命令内使用，不落库、不打印。
#[tauri::command]
async fn fetch_remote_models(platform: String, base_url: String, secret: String, protocol: Option<String>) -> Result<Vec<String>, String> {
    // Gemini / Grok 走各自官方 OpenAI 兼容的 /models（Bearer 鉴权）；自定义供应商按所选协议鉴权
    if !matches!(platform.as_str(), "claude" | "codex" | "gemini" | "grok" | "custom" | "zcode" | "opencode") {
        return Err("不支持的平台".into());
    }
    // 协议只对自定义供应商有效，其他平台忽略传入值（任务 4.3 再按需放开）
    let protocol = if matches!(platform.as_str(),"custom"|"zcode"|"opencode") { protocol } else { None };
    tauri::async_runtime::spawn_blocking(move || connections::fetch_models(&platform, protocol.as_deref(), &base_url, &secret))
        .await
        .map_err(|e| e.to_string())?
}

/// 「验证模型」（任务 3.4，画板 27）：自定义供应商的模型接口不可用、手填模型 ID 时，由用户确认后发送一次最小请求。
/// 可能产生少量 API 用量；Key 只在本命令内使用，不落库、不打印。
#[tauri::command]
async fn verify_custom_model(base_url: String, secret: String, protocol: String, model: String) -> Result<(), String> {
    if !matches!(protocol.as_str(), "openai" | "anthropic") {
        return Err("协议只能是 OpenAI 兼容或 Anthropic 兼容".into());
    }
    tauri::async_runtime::spawn_blocking(move || connections::verify_model(&protocol, &base_url, &secret, &model))
        .await
        .map_err(|e| e.to_string())?
}

/// 启用连接（画布 18 后续，对齐 cc-switch）：把连接保存的凭证与地址写入
/// 对应 CLI 的配置文件（Claude settings.json / .credentials.json、Codex auth.json
/// / config.toml），写前自动备份；同时在本应用内恢复该连接的额度查询。
/// 返回写入说明文案（供 Toast 展示），不含任何凭证内容。
async fn validate_restored_connection(id:&str,current:&connections::Creds) -> Result<(),String> {
    let (id,current)=(id.to_string(),current.clone());
    tauri::async_runtime::spawn_blocking(move || {
        if current.kind=="auth" && provider_browser_auth::is_browser(current.secret.as_deref()) {
            return provider_browser_auth::validate(&id,&current.platform,current.secret.as_deref().unwrap_or_default());
        }
        if platforms::native(&current.platform) && current.kind=="auth" {
            return native_sources::check(&id,&current.platform).map(|_|());
        }
        let secret=current.secret.ok_or("该连接没有保存凭证，请重新授权或读取本地登录")?;
        quota::validate_connection(&current.platform,&current.kind,&secret,current.base_url.as_deref(),current.model.as_deref())
    }).await.map_err(|_|"连接验证异常")?
}

#[tauri::command]
async fn enable_connection(app: tauri::AppHandle, db: State<'_, Db>, id: String) -> Result<String, String> {
    let info = with_database(db.0.clone(), {
        let id = id.clone();
        move |conn| connections::creds_of(conn, &id).map_err(|e| e.to_string())
    })
    .await?
    .ok_or("连接不存在")?;
    if provider_key::supported(&info.platform) && info.kind == "api" {
        let secret=info.secret.clone().ok_or("该连接没有保存 API Key")?;
        tauri::async_runtime::spawn_blocking(move || provider_key::validate(&info.platform,&secret,info.base_url.as_deref()))
            .await.map_err(|e|e.to_string())??;
        let key=id.clone();
        with_database(db.0.clone(),move |conn|connections::set_paused(conn,&key,false)).await?;
        invalidate_network_cache(&id);
        let _=app.emit("connections-changed",());
        return Ok("官方 Key 检测通过；仅保存连接，不修改外部应用登录或授予余额权限".into());
    }
    if matches!(info.platform.as_str(),"zcode"|"opencode") && info.kind=="api" {
        let secret=info.secret.clone().ok_or("该连接没有保存 API Key")?;
        tauri::async_runtime::spawn_blocking(move || quota::validate_connection(&info.platform,"api",&secret,info.base_url.as_deref(),info.model.as_deref())).await.map_err(|e|e.to_string())??;
        let key=id.clone();with_database(db.0.clone(),move|conn|connections::set_paused(conn,&key,false)).await?;
        invalidate_network_cache(&id);let _=app.emit("connections-changed",());let _=app.emit("refresh-requested",id);
        return Ok("已恢复连接查询，原工具配置保持原样".into());
    }
    if info.kind=="auth" && (platforms::native(&info.platform)||provider_browser_auth::is_browser(info.secret.as_deref())) {
        validate_restored_connection(&id,&info).await?;
        let key=id.clone();
        with_database(db.0.clone(),move |conn|connections::set_paused(conn,&key,false)).await?;
        invalidate_network_cache(&id);
        let _=app.emit("connections-changed",());
        let _=app.emit("refresh-requested",id);
        return Ok("已验证并恢复账号查询".into());
    }
    let secret = info.secret.clone().ok_or("该连接没有保存凭证，无法写入 CLI 配置")?;
    let claude_prepared = if info.platform == "claude" && info.kind == "auth" {
        let saved = with_database(db.0.clone(), move |conn| auth_snapshot::claude_saved(conn)).await?;
        Some(tauri::async_runtime::spawn_blocking(move || claude_auth::prepare_with_saved(saved)).await.map_err(|e| e.to_string())??)
    } else { None };
    let backup_dir = data_paths(&app).map_err(|e| e.to_string())?.0.join("cli-backups");
    let pool = db.0.clone();
    let snapshot_id = id.clone();
    let note = tauri::async_runtime::spawn_blocking(move || cli_apply::serialized_switch(|| {
        // 保持写数据库锁到 CLI 写入和连接状态更新结束，其他窗口不能删除或替换目标。
        let conn = pool.lock().map_err(|_| "连接数据库锁不可用")?;
        let current = connections::creds_of(&conn, &snapshot_id).map_err(|e| e.to_string())?.ok_or("连接不存在")?;
        if current.secret.as_deref() != Some(secret.as_str()) || current.platform != info.platform || current.kind != info.kind || current.base_url != info.base_url {
            return Err("连接在准备期间已变化，请重试".into());
        }
        auth_snapshot::preserve_current(&conn, &info.platform)?;
        if let Some(prepared) = &claude_prepared { prepared.preserve(&conn)?; }
        // 保全时可能已安全同步同账号的新 token，必须使用最新的完整授权。
        let current = connections::creds_of(&conn, &snapshot_id).map_err(|e| e.to_string())?.ok_or("连接不存在")?;
        let secret = current.secret.as_deref().ok_or("该连接没有保存凭证")?;
        let snapshot = if matches!(current.platform.as_str(), "codex" | "claude") && current.kind == "auth" {
            if current.platform == "codex" { auth_snapshot::capture(&conn, &snapshot_id, &current.platform, secret)?; }
            auth_snapshot::load(&conn, &snapshot_id, secret)?
        } else { None };
        let note = if let Some(prepared) = &claude_prepared {
            cli_apply::apply_claude_verified(secret, &backup_dir, snapshot.as_ref().ok_or("该 Claude 账号缺少完整身份快照，请切回原账号重新同步")?, prepared)?
        } else {
            cli_apply::apply_to_cli(&current.platform, &current.kind, secret, current.base_url.as_deref(), &backup_dir, snapshot.as_ref())?
        };
        connections::set_paused(&conn, &snapshot_id, false)?;
        Ok(note)
    })).await.map_err(|e| e.to_string())??;
    invalidate_network_cache(&id);
    let _ = app.emit("connections-changed", ());
    Ok(note.message)
}

/// 统一编辑连接（画布 18）：完整快照式更新。
/// auth 连接只接受名称（凭证由 CLI 管理）；api 连接可改网关地址、Key 与默认参数。
/// secret 为空表示不更换 Key；地址或 Key 变化时先用组合后的凭证做一次网络验证，
/// 验证通过才落库，避免把不可用的配置存进去。
#[tauri::command]
async fn update_connection(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    cfg: State<'_, Cfg>,
    id: String,
    name: String,
    base_url: Option<String>,
    secret: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    context_1m: Option<bool>,
) -> Result<(), String> {
    let lookup_id = id.clone();
    let Some(current) = with_database(db.0.clone(), move |conn| {
        connections::creds_of(conn, &lookup_id).map_err(|e| e.to_string())
    })
    .await?
    else {
        return Err("连接不存在".into());
    };
    let trimmed_name = name.trim().to_string();

    if current.kind != "api" {
        // 官方订阅：凭证由 CLI 登录管理，编辑仅限名称
        let rename_id = id.clone();
        let rename_name = trimmed_name.clone();
        with_database(db.0.clone(), move |conn| {
            connections::rename_connection(conn, &rename_id, &rename_name)
        })
        .await?;
    } else {
        let final_base = base_url
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty());
        let final_base = connections::normalized_base_for(&current.platform, final_base);
        if provider_key::supported(&current.platform) {
            provider_key::check_base(&current.platform, final_base.as_deref())?;
        }
        let new_secret = secret
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let base_changed = final_base != current.base_url;
        let key_changed = new_secret.as_deref().is_some_and(|s| Some(s) != current.secret.as_deref());
        // 自定义供应商换 Key / 地址须同时提供模型；可达性由前端获取或验证模型保证（任务 3.4）
        if base_changed || key_changed {
            connections::check_custom_key_change(&current.platform, model.as_deref())?;
        }
        if base_changed || key_changed {
            let platform = current.platform.clone();
            let probe_secret = new_secret.clone().or_else(|| current.secret.clone()).ok_or("该连接没有保存凭证，请填写 API Key")?;
            let probe_base = final_base.clone();
            let probe_model = model.clone();
            tauri::async_runtime::spawn_blocking(move || {
                quota::validate_connection(&platform, "api", &probe_secret, probe_base.as_deref(), probe_model.as_deref())
            })
            .await
            .map_err(|e| e.to_string())??;
        }
        let update_id = id.clone();
        let rename_name = trimmed_name.clone();
        with_database(db.0.clone(), move |conn| {
            connections::rename_connection(conn, &update_id, &rename_name)?;
            if let Some(new_secret) = new_secret {
                connections::replace_api_key(conn, &update_id, &new_secret)?;
            }
            connections::update_params(
                conn,
                &update_id,
                final_base.as_deref(),
                model.as_deref().map(str::trim).filter(|m| !m.is_empty()),
                effort.as_deref().map(str::trim).filter(|e| !e.is_empty()),
                context_1m,
            )
        })
        .await?;
        if key_changed {
            let _ = app.emit("refresh-requested", id.clone());
        }
    }
    // 名称若是任一灵动岛的当前连接，同步快照（与 rename_connection 命令同一套逻辑）
    let mut renamed = false;
    let next = cfg.0.try_update(|s| { renamed = island_clones::rename_connection(s, &id, &trimmed_name); })?;
    if renamed {
        let _ = app.emit("settings-changed", &next);
        refresh_tray_menu(&app);
    }
    invalidate_network_cache(&id);
    let _ = app.emit("connections-changed", ());
    Ok(())
}

#[tauri::command]
async fn add_connection(app: tauri::AppHandle, db: State<'_, Db>, input: connections::NewConnection) -> Result<String, String> {
    if validation::isolated() && input.secret.as_deref().map_or(true, |s| s.trim().is_empty()) {
        return Err("隔离验收必须显式提供测试凭证".into());
    }
    let input = connections::prepare_new(input)?;
    let platform = input.platform.clone();
    let kind = input.kind.clone();
    let secret = input.secret.clone().ok_or("连接尚未准备好凭证")?;
    let base_url = input.base_url.clone();
    let model = input.model.clone();
    let claude_prepared = if platform == "claude" && kind == "auth" {
        let expected = secret.clone();
        Some(tauri::async_runtime::spawn_blocking(move || {
            let prepared = claude_auth::prepare_current()?;
            prepared.require(Some(&expected))?;
            Ok::<_, String>(prepared)
        }).await.map_err(|e| e.to_string())??)
    } else { None };
    tauri::async_runtime::spawn_blocking(move || {
        quota::validate_connection(&platform, &kind, &secret, base_url.as_deref(), model.as_deref())
    }).await.map_err(|e| e.to_string())??;
    let id = with_database(db.0.clone(), move |conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let id = connections::add(&tx, input)?;
        if let Some(prepared) = &claude_prepared { prepared.sync(&tx, &id)?; }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id)
    })
    .await?;
    invalidate_network_cache(&id);
    let _ = app.emit("connections-changed", ());
    Ok(id)
}

/// 移除连接：只删凭证与配置，历史统计保留（§6.3）
#[tauri::command]
async fn remove_connection(app: tauri::AppHandle, db: State<'_, Db>, id: String) -> Result<(), String> {
    let remove_id = id.clone();
    with_database(db.0.clone(), move |conn| {
        connections::remove(conn, &remove_id).map_err(|e| e.to_string())
    })
    .await?;
    invalidate_network_cache(&id);
    let _ = app.emit("connections-changed", ());
    Ok(())
}

#[tauri::command]
async fn set_connection_paused(app: tauri::AppHandle, db: State<'_, Db>, id: String, paused: bool) -> Result<(), String> {
    if !paused {
        let lookup_id = id.clone();
        let current = with_database(db.0.clone(), move |conn| {
            connections::creds_of(conn, &lookup_id).map_err(|e| e.to_string())
        }).await?.ok_or("连接不存在")?;
        validate_restored_connection(&id,&current).await?;
    }
    let update_id = id.clone();
    with_database(db.0.clone(), move |conn| connections::set_paused(conn, &update_id, paused)).await?;
    invalidate_network_cache(&id);
    if app.state::<Cfg>().0.get().island_connection_id.as_deref() == Some(id.as_str()) {
        update_tray_icon(&app);
    }
    let _ = app.emit("connections-changed", ());
    // 重新连接后立刻取数，否则界面要停在「等待连接数据更新」直到下一轮轮询。
    // 断开则不必：此时界面本就该显示已断开，多发一次查询只是白跑。
    if !paused {
        let _ = app.emit("refresh-requested", id);
    }
    Ok(())
}

/// 「获取」：重读本机凭证刷新已有连接。失败不破坏现有连接（§6.3）
#[tauri::command]
async fn fetch_credentials(app: tauri::AppHandle, db: State<'_, Db>, id: String) -> Result<connections::FetchResult, String> {
    if validation::isolated() { return Err("隔离验收不读取本机凭证".into()); }
    let lookup = id.clone();
    let info = with_database(db.0.clone(), move |conn| connections::creds_of(conn, &lookup).map_err(|e| e.to_string())).await?.ok_or("连接不存在")?;
    // 本机/浏览器来源验证可能涉及外部进程与网络；不得持数据库锁等待。
    if native_sources::monitor_id(&id).is_some() || provider_browser_auth::is_browser(info.secret.as_deref()) {
        if info.paused { return Err("连接已断开，请先恢复查询".into()); }
        validate_restored_connection(&id,&info).await?;
        let key=id.clone();
        with_database(db.0.clone(),move |conn| {
            if connections::creds_of(conn,&key).map_err(|e|e.to_string())?.as_ref()!=Some(&info) { return Err("连接在验证期间发生变化，请重新查询".into()); }
            Ok(())
        }).await?;
        invalidate_network_cache(&id);
        let _=app.emit("connections-changed",());
        let _=app.emit("refresh-requested",id);
        return Ok(connections::FetchResult {ok:true,masked:None,message:"已核实所选账号的查询授权".into(),scanned:vec![],changed:false});
    }
    let claude_prepared = if info.platform == "claude" && info.kind == "auth" {
        if info.paused { return Err("连接已断开，请先重新连接".into()); }
        Some(tauri::async_runtime::spawn_blocking(claude_auth::prepare_current).await.map_err(|e| e.to_string())??)
    } else { None };
    let fetch_id = id.clone();
    let result = with_database(db.0.clone(), move |conn| {
        if let Some(prepared) = claude_prepared {
            let current = connections::creds_of(conn, &fetch_id).map_err(|e| e.to_string())?.ok_or("连接不存在")?;
            if current.secret != info.secret || current.paused { return Err("连接在准备期间已变化，请重试".into()); }
            let changed = prepared.sync(conn, &fetch_id)?;
            let token = prepared.require(None)?["claudeAiOauth"]["accessToken"].as_str().ok_or("Claude 授权不完整")?;
            return Ok(connections::FetchResult { ok: true, masked: Some(connections::mask(token)), message: "已核实 Claude 身份并保存完整授权".into(), scanned: vec![], changed });
        }
        let result = connections::fetch_local(conn, &fetch_id).map_err(|e| e.to_string())?;
        if result.ok {
            if let Some(info) = connections::creds_of(conn, &fetch_id).map_err(|e| e.to_string())? {
                if info.kind == "auth" { auth_snapshot::capture(conn, &fetch_id, &info.platform, info.secret.as_deref().unwrap_or_default())?; }
            }
        }
        Ok(result)
    })
    .await?;
    if result.ok {
        invalidate_network_cache(&id);
        let _ = app.emit("connections-changed", ());
        // 缓存刚被清空，无论凭证是否变化都立即重查。用户按「更新」的意图是
        // 「我要最新数据」：凭证没变时同样该看到刷新，而不是毫无动静——
        // 只在 changed 时通知，会让「本机凭证未变化」这条最常见的路径彻底没反馈。
        // 携带目标连接 ID：只有这条连接被灵动岛/总览选中时才重查，避免更新一行广播刷新另一条。
        let _ = app.emit("refresh-requested", id);
    }
    Ok(result)
}

#[tauri::command]
async fn replace_api_key(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    id: String,
    secret: String,
) -> Result<(), String> {
    let secret = secret.trim().to_string();
    if secret.is_empty() { return Err("API Key 不能为空".into()); }
    let lookup_id = id.clone();
    let Some(current) = with_database(db.0.clone(), move |conn| {
        connections::creds_of(conn, &lookup_id).map_err(|e| e.to_string())
    }).await? else { return Err("连接不存在".into()); };
    if current.paused { return Err("连接已断开，请先重新连接".into()); }
    if current.kind != "api" { return Err("该连接不是 API Key 连接".into()); }
    // 自定义供应商只能通过「编辑」带上重新获取或验证过的模型更换 Key（update_connection，任务 3.4）
    connections::check_custom_key_change(&current.platform, None)?;
    let platform = current.platform;
    let base_url = current.base_url;
    let model = current.model;
    let probe_secret = secret.clone();
    tauri::async_runtime::spawn_blocking(move || {
        quota::validate_connection(&platform, "api", &probe_secret, base_url.as_deref(), model.as_deref())
    }).await.map_err(|e| e.to_string())??;
    let replace_id = id.clone();
    with_database(db.0.clone(), move |conn| connections::replace_api_key(conn, &replace_id, &secret)).await?;
    invalidate_network_cache(&id);
    let _ = app.emit("connections-changed", ());
    // 换了 Key 必然要用新凭证重查，否则界面会一直停在旧 Key 的结果上
    // 更换 API Key 同样只刷新目标连接；全局刷新入口仍发送空 payload。
    let _ = app.emit("refresh-requested", id);
    Ok(())
}

/// 本机可自动发现的连接候选，供「+ 添加连接」预填
/// 账号管理「当前使用」：本机 CLI 正在用的凭证对应哪条连接（每个平台至多一条）。
/// 只比对，不返回凭证；API Key 优先于订阅，与 CLI 同时配置两者时的生效顺序一致。
#[tauri::command]
async fn current_connections(db: State<'_, Db>) -> Result<Vec<String>, String> {
    if validation::isolated() { return Ok(vec![]); }
    let local = tauri::async_runtime::spawn_blocking(current_connection::read_local)
        .await.map_err(|error| error.to_string())?;
    with_database(db.0.clone(), move |conn| {
        local.iter().filter_map(|local| connections::find_current_local(conn, local).transpose()).collect()
    }).await
}

/// 「检测本机登录」：只读检测，不入库；保存仍走 read_local_connections。
#[tauri::command]
async fn detect_local_login(platform: String, edition: Option<String>) -> Result<login_detect::LoginDetection, String> {
    if validation::isolated() { return Err("隔离验收环境禁止读取真实凭证".into()); }
    tauri::async_runtime::spawn_blocking(move || login_detect::detect_for_edition(&platform,edition.as_deref())).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn discover_connections() -> Vec<creds::Candidate> {
    if validation::isolated() { return vec![]; }
    creds::discover()
}

/// 重命名连接：只改显示名称。若改的正是灵动岛当前连接，
/// 需同步刷新设置里的名称快照（托盘/灵动岛显示用的是快照，不实时查库）。
#[tauri::command]
async fn rename_connection(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    cfg: State<'_, Cfg>,
    id: String,
    name: String,
) -> Result<(), String> {
    let rename_id = id.clone();
    let rename_name = name.clone();
    with_database(db.0.clone(), move |conn| {
        connections::rename_connection(conn, &rename_id, &rename_name)
    }).await?;
    let mut renamed = false;
    let next = cfg.0.try_update(|s| { renamed = island_clones::rename_connection(s, &id, &name); })?;
    if renamed {
        let _ = app.emit("settings-changed", &next);
        refresh_tray_menu(&app);
    }
    let _ = app.emit("connections-changed", ());
    Ok(())
}

#[derive(serde::Serialize)]
struct LocalReadSummary { added: usize, existing: usize, warnings: Vec<String> }

#[tauri::command]
async fn read_local_connections(app: tauri::AppHandle, db: State<'_, Db>, platform: String, kind: String, name: Option<String>, edition: Option<String>) -> Result<LocalReadSummary, String> {
    if validation::isolated() { return Err("隔离验收环境禁止读取真实凭证".into()); }
    if platforms::native(&platform) {
        if kind != "auth" { return Err("该平台请使用本机来源获取".into()); }
        let accounts=if matches!(platform.as_str(),"qoder"|"workbuddy"|"trae") {
            let (selected,region)=(platform.clone(),edition.clone());
            Some(tauri::async_runtime::spawn_blocking(move || login_detect::native_accounts_for(&selected,region.as_deref())).await.map_err(|_|"本地登录检测异常")??)
        } else {None};
        let result=with_database(db.0.clone(), move |conn| {
            let result=match accounts {
                Some(accounts)=>native_sources::register_accounts(conn,&platform,name.as_deref(),accounts),
                None=>native_sources::register(conn,&platform,name.as_deref()),
            };
            result.map(|(added,existing,warnings)|LocalReadSummary {added,existing,warnings})
        }).await?;
        let _=app.emit("connections-changed",());
        return Ok(result);
    }
    let (inputs, claude_prepared) = tauri::async_runtime::spawn_blocking(move || {
        let inputs = creds::local_connections(&platform, &kind)?;
        let prepared = if platform == "claude" && kind == "auth" {
            let prepared = claude_auth::prepare_current()?;
            for input in &inputs { prepared.require(input.secret.as_deref())?; }
            Some(prepared)
        } else { None };
        Ok::<_, String>((inputs, prepared))
    }).await.map_err(|e| e.to_string())??;
    let result = with_database(db.0.clone(), move |conn| {
        // 订阅自动识别账号；API Key 可用自定义服务名，校验失败则不写库。
        let inputs = connections::with_custom_name(inputs, name)?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut summary = LocalReadSummary { added: 0, existing: 0, warnings: vec![] };
        for input in inputs {
            let (id, added) = connections::register_local(&tx, input)?;
            if let Some(prepared) = &claude_prepared { prepared.sync(&tx, &id)?; }
            if added { summary.added += 1; } else { summary.existing += 1; }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(summary)
    }).await?;
    let _ = app.emit("connections-changed", ());
    Ok(result)
}

const ISLAND: &str = "island";
const MAIN: &str = "main";
const STARTUP: &str = "startup";

static STARTUP_READY: AtomicBool = AtomicBool::new(false);
static STARTUP_INITIALIZING: AtomicBool = AtomicBool::new(false);
static STARTUP_REQUESTED: AtomicBool = AtomicBool::new(false);
static MAIN_READY: AtomicBool = AtomicBool::new(false);
static FIRST_INSTALL: AtomicBool = AtomicBool::new(false);
static FIRST_INSTALL_INTRO: AtomicBool = AtomicBool::new(false);
static MAIN_PLACEMENT: LazyLock<Mutex<Option<main_window::Placement>>> = LazyLock::new(|| Mutex::new(None));
static STARTUP_ERROR: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));

fn persist_main_placement(app: &tauri::AppHandle, placement: main_window::Placement) {
    if !placement.valid() { return; }
    app.state::<Cfg>().0.update(|settings| settings.main_window = Some(placement));
}

/// 写盘节流：拖动与缩放会连发事件，每个都写 JSON 会让磁盘和设置锁跟着抖。
/// 只记最近一次的时间戳，静默 800ms 后才写；退出时 `RunEvent::Exit` 会补最后一次。
static PENDING_PLACEMENT_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// 位置/尺寸变化后延迟落盘。
///
/// 为什么不能只在 `CloseRequested` 写：主面板点关闭是**隐藏**（`api.prevent_close` + `hide`），
/// 进程并不退出，那条分支之后不会再有 `CloseRequested`；用户拖过窗口就关掉再打开时，
/// 记下的仍是上一次**应用退出**时的值——表现就是「窗口像被固定死，没有上次调整过的记忆」。
/// 拖拽与缩放本身就要记住，所以改为「变化即安排落盘」。
fn schedule_persist_placement(window: &tauri::Window, placement: main_window::Placement) {
    let now = chrono::Utc::now().timestamp_millis();
    PENDING_PLACEMENT_MS.store(now, Ordering::Release);
    let handle = window.app_handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(800));
        // 期间又有新事件：让最后一次的安排去写
        if PENDING_PLACEMENT_MS.load(Ordering::Acquire) != now { return; }
        // 用事件里累积的值，不在后台线程重新 capture：
        // 后台线程与主线程的窗口状态不同步，实测会读到缩放动画中间尺寸
        // （窗口已是 1100×700，后台读到 1357×865），把错误值写进设置。
        // 拖动/缩放的最终值由事件序列累积保证——最后一个 Resized 即终值。
        persist_main_placement(&handle, placement);
    });
}

fn record_main_window_event(window: &tauri::Window, event: &WindowEvent) {
    if !MAIN_READY.load(Ordering::Acquire) { return; }
    let placement = {
        let Ok(mut current) = MAIN_PLACEMENT.lock() else { return };
        let Some(placement) = current.as_mut() else { return };
        let mut next = placement.clone();
        match event {
            WindowEvent::Moved(position) => { next.x = position.x; next.y = position.y; }
            WindowEvent::Resized(size) => { next.width = size.width; next.height = size.height; }
            WindowEvent::CloseRequested { .. } => {},
            _ => return,
        }
        if !next.valid() { return; }
        *placement = next.clone();
        next
    };
    // 拖拽 / 缩放：安排一次延迟落盘（节流见 schedule_persist_placement）
    if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) {
        schedule_persist_placement(window, placement.clone());
    }
    // 关闭（隐藏）：立刻落盘，这是用户「用这个尺寸」的最后意图
    if matches!(event, WindowEvent::CloseRequested { .. }) {
        persist_main_placement(window.app_handle(), placement);
    }
}

fn show_startup(app: &tauri::AppHandle) -> tauri::Result<()> {
    let window = if let Some(window) = app.get_webview_window(STARTUP) {
        window
    } else {
        // 画板 109 / 110：启动页随应用主题；设置尚未加载时跟随系统
        let theme = app.try_state::<Cfg>().map(|cfg| cfg.0.get().theme).filter(|t| matches!(t.as_str(), "light" | "dark" | "system"));
        let url = format!("startup.html?theme={}", theme.as_deref().unwrap_or("system"));
        let (width, height) = startup_size(app);
        WebviewWindowBuilder::new(app, STARTUP, WebviewUrl::App(url.into()))
            .title("Jolu · 启动中")
            // 1280×720，页面自绘 12 圆角底色；屏幕放不下时等比缩小
            .inner_size(width, height)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .skip_taskbar(true)
            .center()
            .build()?
    };
    window.show()?;
    let _ = window.set_focus();
    Ok(())
}

/// 启动窗口按画板 1280×720，屏幕可用区域不足时等比缩到 90% 以内。
fn startup_size(app: &tauri::AppHandle) -> (f64, f64) {
    let scale = app.primary_monitor().ok().flatten().map(|monitor| {
        let factor = monitor.scale_factor();
        let area = monitor.work_area().size;
        fit_scale(area.width as f64 / factor, area.height as f64 / factor)
    }).unwrap_or(1.0);
    (1280.0 * scale, 720.0 * scale)
}

fn fit_scale(width: f64, height: f64) -> f64 {
    (width * 0.9 / 1280.0).min(height * 0.9 / 720.0).clamp(0.4, 1.0)
}

#[cfg(test)]
mod observation_tests {
    use crate::quota::QuotaState;

    /// 4.3：只有「不支持 / 失败」会被限流头观测兜底；官方结果与授权、权限、限流结论原样返回。
    #[test]
    fn only_missing_official_quota_falls_back_to_the_last_observation() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-ratelimit-limit-requests", "10".parse().unwrap());
        headers.insert("x-ratelimit-remaining-requests", "4".parse().unwrap());
        crate::gateway::ratelimit::observe("obs-conn", &headers, chrono::Utc::now().timestamp_millis());
        let unsupported = super::with_observation("obs-conn", QuotaState::Unsupported { reason: "API Key 不提供订阅额度".into() });
        match unsupported {
            QuotaState::Observed { windows, reason, .. } => {
                assert_eq!(reason, "API Key 不提供订阅额度");
                assert_eq!(windows[0].used_percent, Some(60.0));
            }
            other => panic!("应该用观测兜底：{other:?}"),
        }
        assert!(matches!(super::with_observation("obs-conn", QuotaState::Failed { reason: "x".into() }), QuotaState::Observed { .. }));
        assert!(matches!(super::with_observation("obs-conn", QuotaState::Unauthorized { reason: "x".into() }), QuotaState::Unauthorized { .. }));
        assert!(matches!(super::with_observation("obs-conn", QuotaState::Ok { windows: vec![], plan: None, account: None }), QuotaState::Ok { .. }));
        assert!(matches!(super::with_observation("no-observation", QuotaState::Unsupported { reason: "x".into() }), QuotaState::Unsupported { .. }));
    }
}

#[cfg(test)]
mod startup_size_tests {
    #[test]
    fn keeps_board_size_on_large_screens_and_shrinks_on_small_ones() {
        assert_eq!(super::fit_scale(1920.0, 1040.0), 1.0);
        let scale = super::fit_scale(1366.0, 728.0);
        assert!(scale < 1.0 && 720.0 * scale <= 728.0 * 0.9 + 0.01);
    }
}

fn report_startup_error(app: &tauri::AppHandle, error: String) {
    eprintln!("[启动] {error}");
    if let Ok(mut current) = STARTUP_ERROR.lock() { *current = Some(error.clone()); }
    if STARTUP_REQUESTED.load(Ordering::Acquire) {
        let _ = show_startup(app);
        let _ = app.emit("startup-error", error);
    }
}

#[tauri::command]
fn startup_error() -> Option<String> {
    STARTUP_ERROR.lock().ok().and_then(|current| current.clone())
}

#[tauri::command]
fn quit_startup(app: tauri::AppHandle) { app.exit(1); }

#[tauri::command]
fn retry_startup(app: tauri::AppHandle) -> Result<&'static str, String> {
    if STARTUP_INITIALIZING.load(Ordering::Acquire) {
        return Ok("initializing");
    }
    if let Ok(mut current) = STARTUP_ERROR.lock() { *current = None; }
    if STARTUP_READY.load(Ordering::Acquire) {
        if !MAIN_READY.load(Ordering::Acquire) {
            if let Some(window) = app.get_webview_window(MAIN) {
                window.destroy().map_err(|error| error.to_string())?;
            }
            show_main(&app);
        } else {
            show_main(&app);
        }
    } else {
        let (data_dir, _) = data_paths(&app)?;
        start_initialization(app, data_dir);
    }
    Ok("restarting")
}

#[tauri::command]
fn main_panel_ready(app: tauri::AppHandle) -> Result<(), String> {
    let window = app.get_webview_window(MAIN).ok_or("主面板窗口不存在")?;
    window.show().map_err(|error| error.to_string())?;
    window.unminimize().map_err(|error| error.to_string())?;
    let captured = main_window::capture(&window);
    if let Ok(mut placement) = MAIN_PLACEMENT.lock() {
        *placement = captured;
    }
    MAIN_READY.store(true, Ordering::Release);
    let _ = window.set_focus();
    if let Some(splash) = app.get_webview_window(STARTUP) { let _ = splash.close(); }
    Ok(())
}

/// 通知链接 → 主窗口跳转意图（4.6）。只打开主窗口的链接按「默认打开页面」处理。
fn link_intent(app: &tauri::AppHandle, target: toast_link::LinkTarget) -> MainIntent {
    match target {
        toast_link::LinkTarget::Task(task_id) => MainIntent::Automation { task_id },
        toast_link::LinkTarget::Connection(connection_id) => MainIntent::Providers { connection_id },
        toast_link::LinkTarget::Main => MainIntent::Overview { settings: app.try_state::<Cfg>().map(|cfg| cfg.0.get()).unwrap_or_default() },
    }
}

/// 第二个实例带着通知链接启动（Jolu 已在运行时点了协议激活的提示框）：交给已运行的实例定位。
fn activate_with_link(app: &tauri::AppHandle, target: toast_link::LinkTarget) {
    if STARTUP_READY.load(Ordering::Acquire) {
        let intent = link_intent(app, target);
        show_main_with_intent(app, intent);
        return;
    }
    // 还在初始化：先记下跳转，主窗口挂载后自己取走
    let intent = link_intent(app, target);
    if let Ok(mut pending) = MAIN_INTENT.lock() { *pending = Some(intent); }
    STARTUP_REQUESTED.store(true, Ordering::Release);
    if let Err(error) = show_startup(app) {
        eprintln!("[启动] 无法显示启动窗口: {error}");
    }
}

fn activate_existing_instance(app: &tauri::AppHandle) {
    STARTUP_REQUESTED.store(true, Ordering::Release);
    if STARTUP_READY.load(Ordering::Acquire) {
        let settings = app.try_state::<Cfg>().map(|cfg| cfg.0.get()).unwrap_or_default();
        show_main_with_intent(app, MainIntent::Overview { settings });
    } else if let Err(error) = show_startup(app) {
        eprintln!("[启动] 无法显示启动窗口: {error}");
    }
}

/// 本体由 tauri.conf 创建、分身在启动与「开启分身」时创建；WebView 被外部关闭或崩溃后，
/// 托盘仍能按同一配置重建。`id` 为 None 表示本体。
fn ensure_island_window_for(app: &tauri::AppHandle, id: Option<&str>) -> Result<WebviewWindow, String> {
    let label = island_clones::label(id);
    if let Some(window) = app.get_webview_window(&label) {
        return Ok(window);
    }
    let settings = app.state::<Cfg>().0.get();
    let profile = island_clones::profile(&settings, id).ok_or("灵动岛配置不存在")?;
    let url = match id {
        Some(id) => format!("index.html?window=island&clone={id}"),
        None => "index.html?window=island".to_string(),
    };
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title("Jolu")
        .inner_size(428.0, 124.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(settings.always_on_top)
        .skip_taskbar(true)
        .visible(false)
        .center()
        .build().map_err(|e| e.to_string())?;
    if settings.dock_enabled && profile.dock.edge.is_some() {
        let _ = dock::restore_dock(&window, &profile.dock);
    } else if let Some((x, y)) = id
        .and_then(|id| settings.island_clones.iter().find(|clone| clone.id == id))
        .and_then(|clone| clone.position)
    {
        // 分身自由态：先挪到记录的位置，再夹回当前工作区，显示器变化后不会落到屏幕外。
        let _ = window.set_position(PhysicalPosition::new(x, y));
        let _ = dock::resize_free(&window, dock::FREE_W, dock::FREE_H);
    }
    Ok(window)
}

/// 「开启分身」：以来源岛的连接为初始值，新窗口在来源岛右下 24px 以自由态出现。
fn create_island_clone(app: &tauri::AppHandle, source: Option<&str>) -> Result<(), String> {
    let store = &app.state::<Cfg>().0;
    // 先在副本上校验上限与来源，失败不落盘。
    island_clones::create_clone(&mut store.get(), source)?;
    let mut created = None;
    let next = store.try_update(|settings| { created = island_clones::create_clone(settings, source).ok(); })?;
    let clone = created.ok_or("创建分身失败")?;
    let window = ensure_island_window_for(app, Some(&clone.id))?;
    if let Some(origin) = island_window(app, source) {
        if let (Ok(position), Ok(scale)) = (origin.outer_position(), origin.scale_factor()) {
            let offset = (24.0 * scale).round() as i32;
            let _ = window.set_position(PhysicalPosition::new(position.x + offset, position.y + offset));
            let _ = dock::resize_free(&window, dock::FREE_W, dock::FREE_H);
        }
    }
    remember_clone_position(app, &clone.id, &window);
    let _ = window.set_always_on_top(next.always_on_top);
    if next.island_visible {
        window.show().map_err(|e| e.to_string())?;
        let _ = window.set_focus();
    }
    let _ = app.emit("settings-changed", store.get());
    let _ = app.emit("menu-state-changed", ());
    Ok(())
}

/// 「销毁分身」：销毁右键所在的岛；本体被销毁时第一个分身接任，本体窗口挪到它的位置。
fn destroy_island(app: &tauri::AppHandle, target: Option<&str>) -> Result<(), String> {
    let store = &app.state::<Cfg>().0;
    island_clones::destroy(&mut store.get(), target)?;
    let mut plan = None;
    store.try_update(|settings| { plan = island_clones::destroy(settings, target).ok(); })?;
    match plan.ok_or("销毁分身失败")? {
        island_clones::DestroyPlan::CloseClone(id) => {
            // 灵动岛的关闭请求会被拦成隐藏，这里必须直接销毁。
            if let Some(window) = island_window(app, Some(&id)) { window.destroy().map_err(|e| e.to_string())?; }
        }
        island_clones::DestroyPlan::PromoteClone { closed } => {
            let clone_window = island_window(app, Some(&closed.id));
            if let Some(primary) = app.get_webview_window(ISLAND) {
                if let Some(window) = &clone_window {
                    if let (Ok(position), Ok(size)) = (window.outer_position(), window.outer_size()) {
                        let _ = primary.set_size(size);
                        let _ = primary.set_position(position);
                    }
                }
                if closed.dock.edge.is_some() { let _ = dock::restore_dock(&primary, &closed.dock); }
                emit_dock_changed(app, None, &closed.dock);
            }
            if let Some(window) = clone_window { window.destroy().map_err(|e| e.to_string())?; }
            refresh_tray_menu(app);
        }
    }
    let _ = app.emit("settings-changed", store.get());
    let _ = app.emit("menu-state-changed", ());
    Ok(())
}

/// 主面板首次使用时才创建；关闭只隐藏窗口并保留 WebView，后台采集与灵动岛继续运行。
fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        if MAIN_READY.load(Ordering::Acquire) {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        } else { let _ = show_startup(app); }
        return;
    }
    MAIN_READY.store(false, Ordering::Release);
    if let Err(error) = show_startup(app) { eprintln!("[启动] 无法显示启动窗口: {error}"); }
    let window = {
        WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
            .title("Jolu")
            .inner_size(1280.0, 900.0)
            .min_inner_size(900.0, 600.0)
            .decorations(false)
            .visible(false)
            .center()
            .build()
    };
    match window {
        Ok(window) => {
            let _ = window.set_icon(tray_icon::taskbar(window.scale_factor().unwrap_or(1.0)));
            if let Some(cfg) = app.try_state::<Cfg>() { let _ = apply_main_zoom(&window, settings::normalize_zoom(cfg.0.get().main_zoom)); } // 任务 2.2
            if let Some(saved) = app.try_state::<Cfg>().and_then(|cfg| cfg.0.get().main_window) {
                if let Err(error) = main_window::restore(&window, &saved) {
                    eprintln!("[窗口] 恢复主面板位置失败: {error}");
                }
            }
        },
        Err(error) => report_startup_error(app, format!("创建主面板失败：{error}")),
    }
}

fn show_main_with_intent(app: &tauri::AppHandle, intent: MainIntent) {
    if let Ok(mut pending) = MAIN_INTENT.lock() {
        *pending = Some(intent);
    }
    show_main(app);
    // 已存在窗口会立即消费；新窗口若此时尚未加载，会在挂载后主动 take。
    let _ = app.emit("main-intent", ());
}

fn start_initialization(app: tauri::AppHandle, data_dir: std::path::PathBuf) {
    if STARTUP_INITIALIZING.swap(true, Ordering::AcqRel) { return; }
    tauri::async_runtime::spawn_blocking(move || {
        let result = initialize_app(&app, &data_dir);
        STARTUP_INITIALIZING.store(false, Ordering::Release);
        match result {
            Ok(()) => {
                STARTUP_READY.store(true, Ordering::Release);
                // 灵动岛 WebView 可能早于后台数据库初始化完成，通知它重读已保存的选择和连接。
                let _ = app.emit("settings-changed", app.state::<Cfg>().0.get());
                let _ = app.emit("connections-changed", ());
                if STARTUP_REQUESTED.load(Ordering::Acquire) { show_main(&app); }
            }
            Err(error) => report_startup_error(&app, error),
        }
    });
}

fn initialize_app(app: &tauri::AppHandle, data_dir: &std::path::Path) -> Result<(), String> {
    if app.try_state::<Db>().is_none() {
        let conn = db::open(&data_dir.join("usage.db")).map_err(|error| error.to_string())?;
        // 路由流水标「来自 Profile」用：读回上次生效的 Profile
        crate::gateway::profiles::load_active(&conn);
        app.manage(Db(Arc::new(Mutex::new(conn))));
    }
    if app.try_state::<DbRead>().is_none() {
        // 写连接已完成建表与迁移，只读连接此后打开才能看到完整结构。
        let pool = ReadPool::open(&data_dir.join("usage.db")).map_err(|error| error.to_string())?;
        app.manage(DbRead(Arc::new(pool)));
        // 自动化调度（模块 E）：建表、恢复中断的运行并开始按时检查
        automation::start(app.clone());
        // 保持系统唤醒（批次 G）：按上次保存的选择恢复；失败只记录，不影响启动
        keep_awake::restore(app.state::<Cfg>().0.get().keep_awake);
    }

    // 恢复已有用户的停靠状态；首次安装先展示自由态灵动岛，再自动贴上边缘。
    {
        let cfg = app.state::<Cfg>().0.get();
        let first_intro = FIRST_INSTALL.swap(false, Ordering::AcqRel)
            && cfg.island_visible && cfg.dock_enabled && cfg.dock.edge.is_none();
        if let Some(w) = app.get_webview_window(ISLAND) {
            let _ = w.set_always_on_top(cfg.always_on_top);
            if first_intro {
                FIRST_INSTALL_INTRO.store(true, Ordering::Release);
                if let Err(error) = dock::place_free_top_center(&w) {
                    eprintln!("[启动] 灵动岛首次居中失败: {error}");
                    let _ = w.center();
                }
            }
        }
        if let (true, Some(edge), Some(w)) = (cfg.dock_enabled, cfg.dock.edge, app.get_webview_window(ISLAND)) {
            let _ = edge;
            if let Some(monitor) = dock::restore_dock(&w, &cfg.dock) {
                if cfg.dock.monitor.as_deref() != Some(&monitor) {
                    app.state::<Cfg>().0.update(|settings| settings.dock.monitor = Some(monitor));
                }
            }
        }
        if let Some(w) = app.get_webview_window(ISLAND) {
            if cfg.island_visible { let _ = w.show(); }
            else { let _ = w.hide(); }
        }
        // 重建上次运行时开启的分身；各自按保存的停靠或自由态位置落位。
        for clone in &cfg.island_clones {
            match ensure_island_window_for(app, Some(&clone.id)) {
                Ok(w) => { if cfg.island_visible { let _ = w.show(); } }
                Err(error) => eprintln!("[启动] 重建灵动岛分身 {} 失败: {error}", clone.id),
            }
        }
        if first_intro {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if !FIRST_INSTALL_INTRO.swap(false, Ordering::AcqRel) { return; }
                let settings = app.state::<Cfg>().0.get();
                if settings.island_visible && settings.dock_enabled && settings.dock.edge.is_none() {
                    island_position(app, Some(dock::Edge::Top), None);
                }
            });
        }
    }

    if app.tray_by_id("main-tray").is_none() {
        TrayIconBuilder::with_id("main-tray")
            .icon(tray_icon_image(app))
            .show_menu_on_left_click(false)
            .on_tray_icon_event(|tray, event| {
                match &event {
                    TrayIconEvent::Enter { position, .. } => tray_summary::enter(tray.app_handle(), position.x, position.y),
                    TrayIconEvent::Leave { .. } | TrayIconEvent::Click { .. } | TrayIconEvent::DoubleClick { .. } => tray_summary::close(tray.app_handle()),
                    _ => {}
                }
                if matches!(event, TrayIconEvent::Click { button: MouseButton::Right, button_state: MouseButtonState::Up, .. }) {
                    if let Err(error) = context_menu::open(tray.app_handle(), false, None) { eprintln!("[菜单] {error}"); }
                    return;
                }
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event {
                    let app = tray.app_handle();
                    show_main_with_intent(
                        app,
                        MainIntent::Overview { settings: app.state::<Cfg>().0.get() },
                    );
                }
            })
            .build(app)
            .map_err(|error| error.to_string())?;
    }

    // 后台采集与代理在数据库和托盘就位后启动，不阻塞启动窗口绘制。
    if !validation::isolated() { watcher::start(app.clone()); }
    if !validation::isolated() { watcher::start_config_sync(app.clone()); }
    if !validation::isolated() { proxy::startup(app); }
    // 预建右键菜单窗口：把首次右键的 WebView 构建耗时挪到启动后的空闲期
    context_menu::prewarm(app);
    // 胶囊灵动岛的眼睛跟随窗口外的鼠标；只在胶囊样式且灵动岛可见时采样
    island_pointer::start(app.clone());

    if let Some(days) = app.state::<Cfg>().0.get().retention_days {
        if let Ok(cutoff) = retention_cutoff(days) {
            let database = app.state::<Db>().0.clone();
            let cleanup_app = app.clone();
            tauri::async_runtime::spawn(async move {
                match with_database(database, move |conn| {
                    db::cleanup_history(conn, cutoff).map_err(|error| error.to_string())
                }).await {
                    Ok(result) if result.requests > 0 => {
                        println!("[数据] 自动清理 {} 条过期请求", result.requests);
                        let _ = cleanup_app.emit("refresh-requested", ());
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("[数据] 自动清理失败：{error}"),
                }
            });
        }
    }

    Ok(())
}

/// 窗口操作成功后再同步设置、前端和托盘，避免显示状态与实际窗口脱节。
/// 显示 / 隐藏是全局开关：本体与全部分身一起动。
fn set_island_visible(app: &tauri::AppHandle, visible: bool) -> Result<(), String> {
    if !visible { FIRST_INSTALL_INTRO.store(false, Ordering::Release); }
    if visible {
        for id in island_ids(&app.state::<Cfg>().0.get()) {
            let w = ensure_island_window_for(app, id.as_deref())?;
            // show 不会解除 Windows 的最小化状态；灵动岛也没有任务栏恢复入口。
            w.unminimize().map_err(|e| e.to_string())?;
            w.show().map_err(|e| e.to_string())?;
            let _ = w.set_focus();
        }
    } else {
        for w in island_windows(app) { w.hide().map_err(|e| e.to_string())?; }
    }
    let next = app.state::<Cfg>().0.update(|s| s.island_visible = visible);
    let _ = app.emit("settings-changed", next);
    refresh_tray_menu(app);
    Ok(())
}

/// 主面板「显示灵动岛」开关与右上角按钮；与托盘的显示 / 隐藏是同一个值。
/// 显示时可能要重建窗口，WebView2 不允许在同步 IPC 回调里建窗口，所以是 async。
#[tauri::command]
async fn set_island_visibility(app: tauri::AppHandle, visible: bool) -> Result<settings::Settings, String> {
    set_island_visible(&app, visible)?;
    Ok(app.state::<Cfg>().0.get())
}

/// 主面板「灵动岛」页里的操作，复用托盘 / 右键菜单的同一套实现。
/// `island` 是目标岛：开启分身时为来源窗口，销毁时为被销毁的窗口，None 为本体。
#[tauri::command]
async fn island_action(app: tauri::AppHandle, id: String, island: Option<String>) -> Result<(), String> {
    if !matches!(id.as_str(), "refresh" | "reset_layout" | "clone_create" | "clone_destroy") {
        return Err("不支持的灵动岛操作".into());
    }
    handle_tray_menu(&app, &id, island.as_deref())
}

fn toggle_island(app: &tauri::AppHandle) -> Result<(), String> {
    // 窗口缺失时视为隐藏并重建；最小化窗口也恢复，避免用户没有任务栏入口。
    let visible = match app.get_webview_window(ISLAND) {
        Some(window) => !window.is_visible().map_err(|e| e.to_string())? || window.is_minimized().map_err(|e| e.to_string())?,
        None => true,
    };
    set_island_visible(app, visible)
}

/// 重置窗口位置（§2.6）：把灵动岛拉回主显示器、解除停靠，
/// 主面板恢复默认尺寸与位置，同时恢复灵动岛的可见状态。
fn reset_window_layout(app: &tauri::AppHandle) {
    FIRST_INSTALL_INTRO.store(false, Ordering::Release);
    app.state::<Cfg>().0.update(|settings| settings.main_window = None);
    if let Ok(mut placement) = MAIN_PLACEMENT.lock() { *placement = None; }
    if let Some(w) = app.get_webview_window(ISLAND) {
        let next = app.state::<Cfg>().0.update(|s| {
            s.dock = Default::default();
            for clone in &mut s.island_clones { clone.dock = Default::default(); clone.position = None; }
        });
        let _ = app.emit("settings-changed", &next);
        for id in island_ids(&next) { emit_dock_changed(app, id.as_deref(), &Default::default()); }
        let _ = w.set_size(tauri::LogicalSize::new(428.0, 124.0));
        let _ = w.center();
        // 分身解除停靠后从本体位置依次向右下错开 24px，避免叠在一起看不见。
        let origin = w.outer_position().ok();
        let scale = w.scale_factor().unwrap_or(1.0);
        for (index, clone) in next.island_clones.iter().enumerate() {
            let Some(cw) = island_window(app, Some(&clone.id)) else { continue };
            let _ = cw.set_size(tauri::LogicalSize::new(428.0, 124.0));
            if let Some(position) = origin {
                let offset = ((index as f64 + 1.0) * 24.0 * scale).round() as i32;
                let _ = cw.set_position(PhysicalPosition::new(position.x + offset, position.y + offset));
                let _ = dock::resize_free(&cw, dock::FREE_W, dock::FREE_H);
            }
            remember_clone_position(app, &clone.id, &cw);
        }
        if let Err(e) = set_island_visible(app, true) {
            eprintln!("[窗口] 恢复灵动岛失败: {e}");
        }
    }
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.set_size(tauri::LogicalSize::new(1280.0, 900.0));
        let _ = w.center();
        // 原生查询会等待 UI 线程；移动/缩放回调也要取此锁，必须先查询再加锁。
        let captured = main_window::capture(&w);
        if let Ok(mut placement) = MAIN_PLACEMENT.lock() {
            *placement = captured;
        }
    }
}

/// 平台是否已接入（未接入的在菜单里禁用并标「待接入」）
fn platform_name(id: &str) -> &str {
    platforms::get(id).map(|p|p.name.as_str()).unwrap_or(id)
}

/// 系统托盘右键菜单 —— 结构与文案见需求文档 §2.6
///
/// 勾选项用 CheckMenuItem，状态取自持久化设置，与设置界面是同一个值。
fn refresh_tray_menu(app: &tauri::AppHandle) {
    update_tray_icon(app);
    let _ = app.emit("menu-state-changed", ());
}

/// 托盘图标按系统实际索取的尺寸原生渲染（SM_CXSMICON = 16 × DPI/96）。
/// 缩放倍数从任一存活窗口读取；窗口全关时按 100% 兜底。
fn tray_scale_factor(app: &tauri::AppHandle) -> f64 {
    app.get_webview_window(MAIN)
        .or_else(|| app.get_webview_window(ISLAND))
        .and_then(|window| window.scale_factor().ok())
        .unwrap_or(1.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayQuotaTone {
    Normal,
    Warning,
    Critical,
    Unknown,
}

fn tray_quota_summary(state: &quota::QuotaState) -> (TrayQuotaTone, Option<f64>) {
    let quota::QuotaState::Ok { windows, .. } = state else {
        return (TrayQuotaTone::Unknown, None);
    };
    let worst = windows
        .iter()
        .filter_map(|window| window.used_percent)
        .filter(|value| value.is_finite())
        .fold(None::<f64>, |current, value| Some(current.map_or(value, |old| old.max(value))))
        .map(|value| value.clamp(0.0, 100.0));
    match worst {
        Some(value) if value >= 90.0 => (TrayQuotaTone::Critical, Some(value)),
        Some(value) if value >= 75.0 => (TrayQuotaTone::Warning, Some(value)),
        Some(value) => (TrayQuotaTone::Normal, Some(value)),
        None => (TrayQuotaTone::Unknown, None),
    }
}

/// 画板 108 的托盘状态色，取自 pn-island-success / warn / danger（浅色值）；未知为灰。
fn tray_quota_color(state: &quota::QuotaState, is_auth: bool) -> [u8; 3] {
    let (tone, _) = tray_quota_summary(state);
    // API Key 没有订阅额度窗口，图标只表达“已配置/可用”的绿色状态；
    // 黄、红、灰等额度状态只对 Auth 连接有意义。
    let tone = if is_auth { tone } else { TrayQuotaTone::Normal };
    match tone {
        TrayQuotaTone::Normal => [6, 193, 103],
        TrayQuotaTone::Warning => [217, 133, 0],
        TrayQuotaTone::Critical => [220, 38, 38],
        TrayQuotaTone::Unknown => [154, 160, 170],
    }
}

/// 托盘当前应显示的 (状态色, 像素尺寸)
fn tray_look(app: &tauri::AppHandle) -> ([u8; 3], usize) {
    let state = tray_summary::current_state(app);
    let is_auth = app.state::<Cfg>().0.get().island_kind == "auth";
    (tray_quota_color(&state, is_auth), tray_icon::output_size(tray_scale_factor(app)))
}

/// 画板 108：托盘为人物 + 右下角额度状态圆点，按系统索取的尺寸原生出图。
fn tray_icon_image(app: &tauri::AppHandle) -> Image<'static> {
    let (color, size) = tray_look(app);
    tray_icon::badge(&tray_icon::base(size), color)
}

/// 上次设置到托盘的 (状态色, 像素尺寸)。内容没变时不重绘、不重复调用系统托盘接口。
static TRAY_ICON_SHOWN: Mutex<Option<([u8; 3], usize)>> = Mutex::new(None);

fn update_tray_icon(app: &tauri::AppHandle) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        let look = tray_look(app);
        let Ok(mut shown) = TRAY_ICON_SHOWN.lock() else { return };
        if *shown == Some(look) {
            return;
        }
        if tray.set_icon(Some(tray_icon::badge(&tray_icon::base(look.1), look.0))).is_ok() {
            *shown = Some(look);
        }
    }
}

#[cfg(test)]
mod tray_icon_tests {
    use super::*;

    fn quota(percentages: &[Option<f64>]) -> quota::QuotaState {
        quota::QuotaState::Ok {
            windows: percentages.iter().enumerate().map(|(index, used_percent)| quota::QuotaWindow {
                key: format!("window-{index}"),
                window_name: format!("窗口 {index}"),
                used_percent: *used_percent,
                amount_text: None,
                remaining_text: None,
                resets_at: None,
            }).collect(),
            plan: None,
            account: None,
        }
    }

    #[test]
    fn tray_uses_the_worst_available_quota_window() {
        assert_eq!(tray_quota_summary(&quota(&[Some(24.0), Some(78.0)])), (TrayQuotaTone::Warning, Some(78.0)));
        assert_eq!(tray_quota_summary(&quota(&[Some(91.0), Some(12.0)])), (TrayQuotaTone::Critical, Some(91.0)));
    }

    #[test]
    fn tray_keeps_unknown_distinct_from_zero_usage() {
        assert_eq!(tray_quota_summary(&quota(&[None])), (TrayQuotaTone::Unknown, None));
        assert_eq!(tray_quota_summary(&quota(&[Some(0.0)])), (TrayQuotaTone::Normal, Some(0.0)));
    }

    #[test]
    fn api_key_tray_icon_stays_green_for_all_quota_tones() {
        let warning = quota(&[Some(78.0)]);
        let critical = quota(&[Some(95.0)]);
        assert_eq!(tray_quota_color(&warning, false), [6, 193, 103]);
        assert_eq!(tray_quota_color(&critical, false), [6, 193, 103]);
    }
}

fn finish_tray_refresh_after_cooldown(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        // 同一轮前端网络查询尚未返回时不允许连续触发，也避免贴着平台限流重试。
        std::thread::sleep(std::time::Duration::from_secs(5));
        TRAY_REFRESHING.store(false, Ordering::Relaxed);
        refresh_tray_menu(&app);
    });
}

/// 托盘菜单事件。灵动岛可见性与平台选择都写回持久化设置，
/// 并通过事件通知前端，保证托盘与设置界面显示同一个值。
/// `island` 是右键菜单所属的岛（None 为本体或托盘）；位置与分身操作都作用于它。
fn handle_tray_menu(app: &tauri::AppHandle, id: &str, island: Option<&str>) -> Result<(), String> {
    match id {
        "open_main" => show_main_with_intent(
            app,
            MainIntent::Overview { settings: app.state::<Cfg>().0.get() },
        ),
        "island_refresh" => {
            if TRAY_REFRESHING.swap(true, Ordering::Relaxed) { return Ok(()); }
            refresh_tray_menu(app);
            let _ = app.emit("island-refresh", ());
            let _ = app.emit("refresh-requested", ());
            finish_tray_refresh_after_cooldown(app.clone());
        }
        "topmost" => { let _ = island_topmost(app.clone(), !app.state::<Cfg>().0.get().always_on_top); }
        "pos_free" => island_position(app.clone(), None, island.map(str::to_string)),
        "pos_top" => island_position(app.clone(), Some(dock::Edge::Top), island.map(str::to_string)),
        "pos_bottom" => island_position(app.clone(), Some(dock::Edge::Bottom), island.map(str::to_string)),
        "pos_left" => island_position(app.clone(), Some(dock::Edge::Left), island.map(str::to_string)),
        "pos_right" => island_position(app.clone(), Some(dock::Edge::Right), island.map(str::to_string)),
        "clone_create" => create_island_clone(app, island)?,
        "clone_destroy" => destroy_island(app, island)?,
        "refresh" => {
            if TRAY_REFRESHING.swap(true, Ordering::Relaxed) { return Ok(()); }
            refresh_tray_menu(app);
            if validation::isolated() {
                // 保留合成网关与界面的刷新，只跳过本机日志采集。
                let _ = app.emit("island-refresh", ());
                let _ = app.emit("refresh-requested", ());
                finish_tray_refresh_after_cooldown(app.clone());
                return Ok(());
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let result = with_database(app.state::<Db>().0.clone(), |conn| {
                    Ok(collector::scan(conn))
                }).await;
                match result {
                    Ok(r) => println!("[托盘] 立即刷新：新增 {} 条", r.records_inserted),
                    Err(e) => eprintln!("[托盘] 刷新失败: {e}"),
                }
                let _ = app.emit("island-refresh", ());
                let _ = app.emit("refresh-requested", ());
                finish_tray_refresh_after_cooldown(app);
            });
        }
        "toggle_island" => {
            if let Err(e) = toggle_island(app) {
                eprintln!("[托盘] 切换灵动岛失败: {e}");
            }
        }
        "dnd" => {
            if let Some(c) = app.try_state::<Cfg>() {
                let next = c.0.update(|s| s.dnd = !s.dnd);
                // 免打扰只影响展示，采集继续
                let _ = app.emit("dnd-changed", next.dnd);
                let _ = app.emit("settings-changed", &next);
            }
            refresh_tray_menu(app);
        }
        "reset_layout" => reset_window_layout(app),
        "about" => {
            show_main_with_intent(app, MainIntent::About {
                version: env!("CARGO_PKG_VERSION").into(),
            });
        }
        "open_source_settings" => {
            show_main_with_intent(app, MainIntent::Settings { section: "island".into() });
        }
        "quit" => app.exit(0),
        other => {
            if let Some(p) = other.strip_prefix("plat_") {
                if let Err(error) = set_island_platform(app.clone(), app.state::<Cfg>(), p.to_string()) {
                    eprintln!("[托盘] 平台切换失败：{error}");
                }
            }
        }
    }
    Ok(())
}

/// 卸载钩子入口（`jolu.exe --restore-agents`）：按 wiring 快照还原全部 agent 的
/// 配置文件，返回 0（全成功或无事可做）或 1（有失败）。
///
/// 为什么不用 Tauri 上下文：NSIS 在删除主程序**之前**调用本入口，此时桌面端已被
/// 关闭，没有窗口可挂；数据库直接按路径打开。设置读不出时端口按默认值处理
/// ——`unwire` 只用它判断「配置当前是否指向网关」，判断错了至多多走一次无害还原。
pub fn restore_agents_headless() -> i32 {
    let (directory, database) = match crate::service::paths::locate() {
        Ok(paths) => paths,
        Err(reason) => {
            eprintln!("[卸载] {reason}");
            // 没有数据目录 = 从没接过线，无事可做不算失败
            return 0;
        }
    };
    let settings = crate::service::settings::read_settings(&directory);
    let port = if settings.proxy_port > 0 { settings.proxy_port } else { 12731 };
    let mut conn = match crate::db::open(&database) {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("[卸载] 打开数据库失败：{error}");
            return 1;
        }
    };
    let report = crate::service::gateway::wiring_restore::restore_all(&mut conn, port);
    for agent in &report.restored {
        eprintln!("[卸载] 已还原 {agent}");
    }
    for line in &report.failed {
        eprintln!("[卸载] 失败：{line}");
    }
    // Library（批 5）：Jolu 写进各 agent 的 MCP server 一并移除
    let library_lines = crate::library::restore_all_library(&conn);
    for line in &library_lines {
        eprintln!("[卸载] {line}");
    }
    let failed = report.failed.len() + library_lines.iter().filter(|l| l.starts_with("失败")).count();
    if failed == 0 { 0 } else { 1 }
}

/// 搜索 MCP server 的 stdio 循环（`jolu.exe --mcp-search`，批 5）。
pub fn run_mcp_search() {
    use std::io::{BufRead, Write};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("搜索 MCP 需要一个小异步运行时");
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Some(response) = runtime.block_on(library::mcp_search::handle_line(&line)) else { continue };
        let mut stdout = std::io::stdout();
        if writeln!(stdout, "{response}").and_then(|_| stdout.flush()).is_err() {
            break;
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            match toast_link::from_args(&args) {
                Some(target) => activate_with_link(app, target),
                None => activate_existing_instance(app),
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init()) // 任务 2.5：只在 Rust 端发通知
        .invoke_handler({
            let handler: fn(tauri::ipc::Invoke) -> bool = tauri::generate_handler![
            tray_summary::tray_summary_get, tray_summary::tray_summary_fit,
            scan_local_sessions,
            collection_status,
            agent_versions,
            agent_discover,
            wsl_probe_agents,
            otlp_export_usage,
            data_info,
            open_data_directory,
            autostart_status,
            set_autostart,
            set_silent_startup,
            library_install,
            library_remove,
            library_list,
            gateway_wiring_applied,
            gateway_wiring_preflight,
            gateway_wiring_preview,
            gateway_long_context_set,
            gateway_client_key_limits_set,
            gateway_models_dev_refresh,
            gateway_models_dev_status,
            gateway_allow_ips_set,
            gateway_allow_ips_from_env,
            gateway_client_key_concurrency_set,
            search_vendors,
            search_apis_set,
            search_try,
            set_keep_awake,
            set_warmup,
            set_otlp_endpoint,
            resize_island,
            live_usage,
            connection_kind,
            source_metrics,
            list_connections,
            daily_checkin::daily_checkin_overview,
            daily_checkin::daily_checkin_status,
            daily_checkin::daily_checkin_claim,
            add_connection,
            update_connection,
            enable_connection,
            remove_connection,
            set_connection_paused,
            fetch_credentials,
            replace_api_key,
            rename_connection,
            discover_connections,
            detect_local_login,
            current_connections,
            read_local_connections,
            data_files::export_data_file,
            data_files::export_calls_csv,
            stats_overview,
            island_source_breakdown,
            stats_activity,
            stats_calls,
            sessions_list,
            session_messages,
            session_continue_check,
            session_continue,
            // 模块 E · 自动化（命令登记在此行下方）
            automation::automation_overview,
            automation::automation_waiting_connections,
            automation::automation_task,
            automation::automation_create,
            automation::automation_update,
            automation::automation_action,
            automation::automation_runs,
            automation::automation_run,
            automation::automation_accounts,
            automation::automation_models,
            automation::automation_quota,
            automation::automation_tools,
            automation::automation_pick_dir,
            // 任务 2.9 开始
            automation::automation_delete,
            // 任务 2.9 结束
            // 模块 F · Skills 管理（命令登记在此行下方）
            skills::skills_overview, skills::skill_detail, skills::skill_file, skills::skill_set_enabled, skills::skill_save, skills::skill_copy, skills::skill_remove, skills::skills_import_scan, skills::skills_import, skills::skills_set_projects, skills::skills_ignore_project, skills::skills_pick_folder, skills::skill_open_dir,
            // 模块 G · 设置（命令登记在此行下方）
            storage_stats::storage_counts,
            get_settings,
            set_island_platform,
            set_island_source,
            set_island_connection,
            set_dnd,
            set_proxy,
            proxy_status,
            gateway_status,
            gateway_access_key,
            gateway_copy_key,
            gateway_shared_lan_set,
            gateway_client_keys_list,
            gateway_client_key_issue,
            gateway_client_key_revoke,
            gateway_middleware_list,
            zcode_version_check,
            gateway_middleware_save,
            gateway_middleware_delete,
            fx_rate,
            set_cost_currency,
            gateway_set_mode,
            gateway_set_default_source,
            gateway_refresh_models,
            gateway_routing_group_list,
            gateway_routing_group_create,
            gateway_group_create,
            gateway_routing_group_update,
            gateway_routing_group_delete,
            providers_import_scan,
            providers_import_apply,
            gateway_profiles_list,
            gateway_profiles_save,
            gateway_profiles_delete,
            gateway_profiles_apply,
            gateway_group_delete,
            gateway_set_serve,
            gateway_set_serve_many,
            gateway_source_usage,
            gateway_source_capabilities,
            gateway_wiring_set,
            gateway_escape,
            privacy_masking_set,
            privacy_policy_set,
            privacy_records,
            privacy_records_clear,
            zcode_free_first_set,
            gateway_groups,
            gateway_group_create,
            gateway_group_rename,
            gateway_group_delete,
            gateway_group_reorder,
            gateway_group_move,
            gateway_set_slug,
            gateway_wiring_accept,
            gateway_catalog,
            gateway_route_log,
            gateway_route_log_page,
            gateway_sessions_overview,
            gateway_today_counts,
            gateway_day_overview,
            gateway_lan_address,
            custom_agents_list,
            custom_agents_add,
            custom_agents_remove,
            custom_agents_wire,
            custom_agents_unwire,
            custom_agents_preview,
            dock_undock,
            cost_estimate,
            connection_quota,
            provider_browser_auth::start_browser_authorization,
            provider_browser_auth::browser_authorization_status,
            provider_browser_auth::cancel_browser_authorization,
            provider_browser_auth::save_browser_connection,
            connection_api_usage,
            local_codex_quota,
            local_grok_quota,
            local_zcode_quota,
            connection_balance,
            island_menu,
            context_menu::menu_action,
            context_menu::menu_fit,
            context_menu::menu_show,
            context_menu::menu_close,
            context_menu::menu_refreshing,
            island_drag,
            main_drag,
            island_position,
            set_island_visibility,
            island_action,
            island_topmost,
            set_dock_enabled,
            set_theme,
            set_display_preferences,
            set_island_style,
            open_main_section,
            open_main_session,
            open_main_usage,
            set_balance_alert,
            notify::take_app_notices,
            set_retention_days,
            cleanup_preview,
            cleanup_history,
            main_panel_ready,
            startup_error,
            retry_startup,
            quit_startup,
            take_main_intent,
            fetch_remote_models,
            // 任务 2.4 开始
            data_files::preview_import_file,
            data_files::confirm_import,
            data_files::discard_import,
            // 任务 2.4 结束
            updater::set_auto_check_updates,
            updater::set_update_interval,
            updater::check_app_update_available,
            updater::download_app_update,
            updater::cancel_app_update_download,
            updater::install_downloaded_update_and_restart,
            // 任务 2.1 开始
            set_default_page,
            remember_page,
            // 任务 2.1 结束
            // 任务 2.2 开始
            set_appearance_preferences,
            // 任务 2.2 结束
            // 任务 2.7 开始
            skills::skills_import_git,
            skills::skills_import_git_cleanup,
            // 任务 2.7 结束
            // 任务 3.4 开始
            verify_custom_model,
            // 任务 3.4 结束
            // 任务 2.3 开始
            net::set_network,
            net::diagnose_network,
            // 任务 2.3 结束
            // 任务 3.3 开始
            automation::automation_skills,
            // 任务 3.3 结束
            // 任务 4.1 开始
            skills::skills_set_global_dir,
            skills::skills_save_dir_settings,
            // 任务 4.1 结束
            // 任务 2.5 开始
            set_task_notify,
            // 任务 2.5 结束
            // 任务 3.1 开始
            session_handoff::handoff_state,
            session_handoff::handoff_start,
            session_handoff::handoff_refine,
            session_handoff::handoff_estimate_refine,
            session_handoff::handoff_finish,
            session_handoff::handoff_cancel,
            session_handoff::handoff_render,
            session_handoff::handoff_local_export,
            session_handoff::handoff_save,
            session_handoff::validate_file_name,
            // 任务 3.1 结束
            ];
            move |invoke: tauri::ipc::Invoke| {
                if invoke.message.webview().label().starts_with("provider-auth-") {
                    invoke.resolver.reject("官方授权页面不能访问 Jolu 应用命令");
                    true
                } else { handler(invoke) }
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            notify::register_click_handler(&handle)?;
            // 系统通知冷启动（4.6）：登记 jolu: 协议；本次若是点通知启动的，记下要跳转的页面，主窗口挂载后取走
            toast_link::register_protocol();
            if let Some(target) = toast_link::from_args(std::env::args()).filter(|target| *target != toast_link::LinkTarget::Main) {
                let intent = link_intent(&handle, target);
                if let Ok(mut pending) = MAIN_INTENT.lock() { *pending = Some(intent); }
            }
            app.manage(updater::UpdateDownloadState::default());
            let automatic = std::env::args_os().any(|arg| arg == "--autostart");
            if !automatic {
                STARTUP_REQUESTED.store(true, Ordering::Release);
                let _ = show_startup(&handle);
            }

            let data_dir = match data_paths(&handle) {
                Ok((path, _)) => path,
                Err(error) => {
                    report_startup_error(&handle, error);
                    return Ok(());
                }
            };
            let first_install = !data_dir.join("settings.json").exists()
                && !data_dir.join("usage.db").exists();
            FIRST_INSTALL.store(first_install, Ordering::Release);
            app.manage(Cfg(settings::Store::load(&data_dir)));
            net::init(&app.state::<Cfg>().0.get()); // 任务 2.3：出站请求按已保存的联网方式发出
            // P5 隐私脱敏：按设置初始化进程级开关（privacy_masking_set 运行时改）
            crate::gateway::mask::set_enabled(app.state::<Cfg>().0.get().privacy_masking);
            {
                let saved = app.state::<Cfg>().0.get();
                match crate::gateway::mask::Policy::new(&saved.privacy_actions, &saved.privacy_allow) {
                    Ok(policy) => crate::gateway::mask::set_policy(policy),
                    Err(reason) => eprintln!("[隐私脱敏] 保存的处理方式无效，按默认全部遮住：{reason}"),
                }
                let handle = app.handle().clone();
                crate::gateway::mask::on_first_mask(move || privacy_first_notice(&handle));
            }
            // ③5 ZCode Start Plan 优先：按设置初始化进程级开关（zcode_free_first_set 运行时改）。
            // 字段默认 false，与静态默认一致；用户开启过则重启后仍要生效，必须回读一次。
            crate::gateway::adapter::zcode::set_free_first(app.state::<Cfg>().0.get().zcode_free_first);
            // C6：本机 ZCode 比 Jolu 适配的版本新时记一条日志（界面提示走 zcode_version_check）。
            // 读注册表放后台线程，不拖慢启动。
            std::thread::spawn(|| {
                let check = zcode_version::check(zcode_version::installed_version());
                if check.newer {
                    eprintln!(
                        "[ZCode] 本机 ZCode {} 比 Jolu 适配的 {} 新：Start Plan 着装常量可能需要更新（见 docs/todo/2026-11-09-zcode-version-follow.md）",
                        check.installed.as_deref().unwrap_or("?"), check.expected
                    );
                }
            });
            let startup_plan = startup::plan(automatic, app.state::<Cfg>().0.get().silent_startup, first_install);
            STARTUP_REQUESTED.store(startup_plan.show_main, Ordering::Release);
            if startup_plan.show_main { let _ = show_startup(&handle); }
            start_initialization(handle, data_dir);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == MAIN {
                record_main_window_event(window, event);
                if let WindowEvent::ScaleFactorChanged { scale_factor, .. } = event {
                    let _ = window.set_icon(tray_icon::taskbar(*scale_factor));
                }
                // 关闭只隐藏、保留 WebView，托盘再次打开时秒开，不再重建主面板并走启动窗。
                // 未就绪的主面板（加载失败等）照常关闭，下次打开会重新创建。
                if let WindowEvent::CloseRequested { api, .. } = event {
                    if MAIN_READY.load(Ordering::Acquire) {
                        api.prevent_close();
                        if let Err(error) = window.hide() {
                            eprintln!("[窗口] 隐藏主面板失败: {error}");
                        }
                        return;
                    }
                }
            }
            if island_clones::is_island_label(window.label()) {
                // 点到别的窗口：WebView 不一定给页面派发 blur，由原生窗口失焦通知胶囊收起
                if let WindowEvent::Focused(false) = event {
                    // 按岛 id 广播：前端 listen 注册的是「任意来源」，emit_to(标签) 匹配不上
                    let id = crate::island_clones::id_from_label(window.label()).flatten();
                    let _ = window.app_handle().emit("island-blur", id);
                }
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    // 无边框灵动岛没有用户可点的关闭入口；系统关闭请求多见于应用退出或开发热重载。
                    // 只隐藏当前原生窗口，不把用户的“显示灵动岛”偏好错误持久化为 false。
                    if let Err(error) = window.hide() {
                        eprintln!("[窗口] 隐藏灵动岛失败: {error}");
                    }
                    return;
                }
            }
            if island_clones::is_island_label(window.label())
                && matches!(event, WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. })
            {
                // Windows 回调中不能同步等待窗口查询；交给后台线程并按窗口合并密集移动事件。
                static PENDING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
                let label = window.label().to_string();
                let queued = PENDING.lock().map(|mut set| set.insert(label.clone())).unwrap_or(false);
                if queued {
                    let app = window.app_handle().clone();
                    let scale_changed = matches!(event, WindowEvent::ScaleFactorChanged { .. });
                    tauri::async_runtime::spawn_blocking(move || {
                        if let (Some(webview), Some(store)) = (app.get_webview_window(&label), app.try_state::<Cfg>()) {
                            let id = island_clones::id_from_label(&label).flatten();
                            let cfg = store.0.get();
                            let dock_state = island_dock(&cfg, id.as_deref());
                            if dock::DRAGGING.load(Ordering::Relaxed) {
                                let hint = if cfg.dock_enabled { dock::hover_hint(&webview) } else { None };
                                let _ = app.emit("dock-hint", DockHintEvent { island: id.clone(), hint });
                            } else if dock_state.edge.is_some()
                                && (scale_changed || !dock::monitor_exists(&webview, dock_state.monitor.as_deref())) {
                                if let Some(monitor) = dock::restore_dock(&webview, &dock_state) {
                                    let next = store.0.update(|settings| {
                                        island_clones::set_dock(settings, id.as_deref(), dock::DockState { monitor: Some(monitor), ..dock_state.clone() });
                                    });
                                    let _ = app.emit("settings-changed", next);
                                }
                            }
                        }
                        if let Ok(mut set) = PENDING.lock() { set.remove(&label); }
                    });
                }
            }
            // 主面板使用默认关闭行为销毁 WebView，释放图表、订阅与网络定时器。
        })
        .build(tauri::generate_context!())
        .expect("error while building Jolu");
    app.run(|app, event| {
        // 正常退出前还原 CLI 直连配置，避免 CLI 指向已失效的本机代理端口；
        // 崩溃 / 强杀的残留由下次启动的 proxy::startup 自愈。
        if matches!(event, tauri::RunEvent::Exit { .. }) {
            if let Ok(placement) = MAIN_PLACEMENT.lock() {
                if let Some(placement) = placement.clone() {
                    persist_main_placement(app, placement);
                }
            }
            proxy::shutdown_and_restore(app);
            // 保持唤醒按线程生效，进程退出前显式交还电源策略
            keep_awake::shutdown();
            // 任务 3.1 开始
            session_handoff::shutdown();
            // 任务 3.1 结束
        }
    });
}

// 任务 2.2 开始
/// 界面缩放只作用于 `main` 窗口（D-21）；灵动岛、菜单等窗口各自独立，保持原大小。
fn apply_main_zoom(window: &WebviewWindow, zoom: u16) -> Result<(), String> {
    window.set_zoom(f64::from(zoom) / 100.0).map_err(|error| {
        eprintln!("[窗口] 设置主面板缩放失败: {error}");
        error.to_string()
    })
}

/// 外观（画板 88）：界面缩放、列表密度、减少动效一起保存；主窗口已打开时立即应用缩放。
#[tauri::command]
fn set_appearance_preferences(app: tauri::AppHandle, zoom: u16, density: String, reduce_motion: bool) -> Result<settings::Settings, String> {
    settings::validate_appearance(zoom, &density)?;
    let next = app.state::<Cfg>().0.try_update(|settings| {
        settings.main_zoom = zoom;
        settings.list_density = density;
        settings.reduce_motion = reduce_motion;
    })?;
    let _ = app.emit("settings-changed", next.clone());
    if let Some(window) = app.get_webview_window(MAIN) {
        apply_main_zoom(&window, zoom).map_err(|error| format!("界面缩放已保存，但应用到主窗口失败：{error}"))?;
    }
    Ok(next)
}
// 任务 2.2 结束
// 任务 2.5 开始
/// 任务提醒（画板 94）：两个开关分别保存，传 None 的保持不变
#[tauri::command]
fn set_task_notify(app: tauri::AppHandle, resumed: Option<bool>, finished: Option<bool>, quota: Option<bool>) -> Result<settings::Settings, String> {
    let next = app.state::<Cfg>().0.try_update(|settings| {
        if let Some(on) = resumed { settings.notify_task_resumed = on; }
        if let Some(on) = finished { settings.notify_task_finished = on; }
        if let Some(on) = quota { settings.notify_quota_alerts = on; }
    })?;
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}
// 任务 2.5 结束
