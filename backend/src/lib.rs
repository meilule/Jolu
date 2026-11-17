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
pub(crate) mod filelock;
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
mod commands;
use commands::*;
mod events;
mod proc;
mod settings;
mod stall;
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
    let _ = app.emit(crate::events::COLLECTION_STATUS, status);
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
        let _ = app.emit(crate::events::STARTUP_ERROR, error);
    }
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
    crate::events::settings_changed(&app);
    let _ = app.emit(crate::events::MENU_STATE_CHANGED, ());
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
    crate::events::settings_changed(&app);
    let _ = app.emit(crate::events::MENU_STATE_CHANGED, ());
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
    let _ = app.emit(crate::events::MAIN_INTENT, ());
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
                crate::events::settings_changed(&app);
                let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
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
                        crate::events::refresh_requested(&cleanup_app, None);
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
    app.state::<Cfg>().0.update(|s| s.island_visible = visible);
    crate::events::settings_changed(&app);
    refresh_tray_menu(app);
    Ok(())
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
        crate::events::settings_changed(&app);
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
    let _ = app.emit(crate::events::MENU_STATE_CHANGED, ());
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
            let _ = app.emit(crate::events::ISLAND_REFRESH, ());
            crate::events::refresh_requested(&app, None);
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
        // 4.2b：以此分身为主——与本体互换所看的连接（窗口不动），各岛随 settings-changed 重新取数
        "clone_promote" => {
            let id = island.ok_or("本体已经是主关注")?;
            app.state::<Cfg>().0.try_update(|settings| { island_clones::promote(settings, id); })?;
            crate::events::settings_changed(app);
        }
        "refresh" => {
            if TRAY_REFRESHING.swap(true, Ordering::Relaxed) { return Ok(()); }
            refresh_tray_menu(app);
            if validation::isolated() {
                // 保留合成网关与界面的刷新，只跳过本机日志采集。
                let _ = app.emit(crate::events::ISLAND_REFRESH, ());
                crate::events::refresh_requested(&app, None);
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
                let _ = app.emit(crate::events::ISLAND_REFRESH, ());
                crate::events::refresh_requested(&app, None);
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
                c.0.update(|s| s.dnd = !s.dnd);
                // 免打扰只影响展示，采集继续
                crate::events::settings_changed(&app);
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
    let mut library_lines = crate::library::restore_all_library(&conn);
    // 5.1：共享指令区块与子代理文件（被用户改过的留着）
    library_lines.extend(crate::library::shared::restore_all(&conn));
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
            library_shared_state,
            library_instructions_save,
            library_subagent_save,
            library_subagent_remove,
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
            warmup_last_outcome,
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
            zcode_set_channel,
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
            gateway_context_sessions,
            gateway_context_snapshot,
            gateway_context_items,
            gateway_context_tool_items,
            gateway_context_history,
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
            set_island_linkage,
            set_main_context,
            main_context,
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
            // 4.2b #6 按最近活跃自动选主（默认关）：每 30 秒看一次各平台最近一次请求，分身所在平台明显更活跃时与本体互换
            {
                let handle = app.handle().clone();
                std::thread::Builder::new().name("jolu-island-focus".into()).spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    let settings = handle.state::<Cfg>().0.get();
                    if !settings.island_linkage.auto_focus || settings.island_clones.is_empty() {
                        continue;
                    }
                    let Some(db) = handle.try_state::<Db>() else { continue };
                    let latest: std::collections::HashMap<String, i64> = {
                        let Ok(conn) = db.0.try_lock() else { continue };
                        let mut stmt = match conn.prepare("SELECT platform, MAX(ts) FROM requests WHERE ts >= ?1 GROUP BY platform") {
                            Ok(stmt) => stmt,
                            Err(_) => continue,
                        };
                        let since = chrono::Utc::now().timestamp_millis() - 3_600_000;
                        stmt.query_map([since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                            .map(|rows| rows.flatten().collect())
                            .unwrap_or_default()
                    };
                    let now = chrono::Utc::now().timestamp_millis();
                    if let Some(id) = island_clones::auto_focus_target(&settings, &latest, now, 120_000, 60_000) {
                        if handle.state::<Cfg>().0.try_update(|s| { island_clones::promote(s, &id); }).is_ok() {
                            crate::events::settings_changed(&handle);
                        }
                    }
                }).ok();
            }
            // 5.5：`jolu settings set` 在别的进程里改了 settings.json —— 每 2 秒看一次，读回并广播给各窗口
            {
                let handle = app.handle().clone();
                std::thread::Builder::new().name("jolu-settings-watch".into()).spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    if handle.state::<Cfg>().0.reload_external() {
                        crate::events::settings_changed(&handle);
                    }
                }).ok();
            }
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
                    let _ = window.app_handle().emit(crate::events::ISLAND_BLUR, id);
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
                                let _ = app.emit(crate::events::DOCK_HINT, DockHintEvent { island: id.clone(), hint });
                            } else if dock_state.edge.is_some()
                                && (scale_changed || !dock::monitor_exists(&webview, dock_state.monitor.as_deref())) {
                                if let Some(monitor) = dock::restore_dock(&webview, &dock_state) {
                                    store.0.update(|settings| {
                                        island_clones::set_dock(settings, id.as_deref(), dock::DockState { monitor: Some(monitor), ..dock_state.clone() });
                                    });
                                    crate::events::settings_changed(&app);
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

// 任务 2.5 结束
