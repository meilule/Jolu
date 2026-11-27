//! 实时监听本地会话记录（§7.7）
//!
//! CLI 每收到一次响应就往 jsonl 追加一行，因此**文件写入即用量事件**。
//! 这里用 notify 监听目录，写入发生时才增量采集并向前端推事件——
//! 前端不再定时轮询，延迟从「最多一个轮询周期」降到「文件写入后几十毫秒」。
//!
//! 去抖：一次响应可能触发多次写事件（内容 + 元数据），
//! 因此收到事件后合并 DEBOUNCE 窗口内的抖动，再做一次采集。

use crate::db::{LiveUsage, Store};
use crate::{collector, Cfg, Db, DbRead};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// 事件名：前端 listen 这个名字即可拿到实时用量
pub const EVENT_LIVE: &str = crate::events::LIVE_USAGE;

/// 写事件合并窗口。兼顾把同一次写入的内容+元数据抖动合并，同时将文件落盘到界面响应压在 ~50ms 内
const DEBOUNCE: Duration = Duration::from_millis(45);

/// 外部切号配置去抖窗口（3A）：CC Switch 等工具原子替换配置文件后快速对齐
const CONFIG_SETTLE: Duration = Duration::from_millis(180);

/// Agent Hooks 回执文件去抖窗口（4B）
const HOOK_DEBOUNCE: Duration = Duration::from_millis(35);

/// 兜底心跳：监听失效（网络盘、权限、句柄耗尽）时仍能恢复，并续期运行中会话的存活判定。
/// 未变化的文件只比对大小与修改时间、不拿写锁，常态开销很小。
const HEARTBEAT: Duration = Duration::from_secs(10);

/// 每隔这么多次心跳整体重绑一次监听，覆盖底层句柄悄悄失效的情况；
/// 其余心跳只补绑新出现或此前绑定失败的目录，不反复拆装全部递归监听。
const FULL_REBIND_EVERY: u32 = 6;

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn claude_root(home: &Path) -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"))
}

fn claude_registry() -> Option<PathBuf> {
    home().map(|h| claude_root(&h).join("sessions"))
}

/// 监听目标：Claude / Codex 会话目录、Claude 进程登记、Codex 标题索引与各本机来源目录。
fn watch_targets() -> Vec<(PathBuf, RecursiveMode)> {
    let Some(home) = home() else { return Vec::new() };
    let claude = claude_root(&home);
    let codex_home = crate::session_titles::codex_home().unwrap_or_else(|| home.join(".codex"));
    let mut targets = vec![
        (claude.join("projects"), RecursiveMode::Recursive),
        (claude.join("sessions"), RecursiveMode::NonRecursive),
        (codex_home.join("sessions"), RecursiveMode::Recursive),
        (codex_home.join("session_index.jsonl"), RecursiveMode::NonRecursive),
    ];
    targets.extend(crate::native_sources::watch_roots().into_iter().map(|path| (path, RecursiveMode::Recursive)));
    targets
}

/// 绑定监听。`full` 时拆掉重绑全部目标（覆盖底层 watcher 丢失句柄）；
/// 否则只补绑尚未成功绑定、以及已被删除需要解绑的目标。`bound` 记录当前生效的目标。
fn rebind_sources(watcher: &mut RecommendedWatcher, bound: &mut HashSet<PathBuf>, full: bool, announce: bool) {
    for (path, mode) in watch_targets() {
        let exists = path.exists();
        if !full && exists == bound.contains(&path) {
            continue;
        }
        if bound.remove(&path) {
            let _ = watcher.unwatch(&path);
        }
        if !exists {
            continue;
        }
        match watcher.watch(&path, mode) {
            Ok(()) => {
                bound.insert(path.clone());
                if announce { println!("[实时] 已监听 {}", path.display()); }
            }
            Err(error) => eprintln!("[实时] 监听 {} 失败，将由心跳重试: {error}", path.display()),
        }
    }
}

/// 写事件路径是否值得采集：会话日志（jsonl）、Claude 进程登记，或本机来源目录里的
/// 数据文件。SQLite 的共享内存文件（-shm）在只读打开时也会变化，排除以免自我触发。
fn relevant(path: &Path, registry: Option<&Path>, native_roots: &[PathBuf]) -> bool {
    let lossy = path.to_string_lossy();
    if lossy.ends_with("-shm") || lossy.ends_with("-SHM") {
        return false;
    }
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
        || registry.is_some_and(|root| crate::native_sources::path_under(path, root))
        || native_roots
            .iter()
            .any(|root| crate::native_sources::path_under(path, root))
}

/// 自 `since` 以来有新增或更正的平台。游标是全局事件序号，删除（如 Gemini 回退）
/// 只推进序号不留事件，此时无法定位平台，按全部平台处理。
fn changed_platforms(reader: &crate::ReadPool, since: Option<i64>, inserted: bool) -> BTreeSet<String> {
    let all = || crate::platforms::ids().into_iter().collect();
    let Some(since) = since else {
        return if inserted { all() } else { BTreeSet::new() };
    };
    let found = reader.with(|conn| -> rusqlite::Result<BTreeSet<String>> {
        let mut stmt = conn.prepare_cached("SELECT DISTINCT platform FROM usage_events WHERE id > ?1")?;
        let rows = stmt.query_map([since], |row| row.get::<_, String>(0))?;
        rows.collect()
    }).unwrap_or_default();
    if found.is_empty() && inserted { all() } else { found }
}

fn stamp_recent_direct_requests(mut writer: &std::sync::Mutex<rusqlite::Connection>, strict_writing: bool) {
    if strict_writing {
        return;
    }
    let since_ms = chrono::Local::now().timestamp_millis() - 90_000;
    writer.write(|conn| {
        for (plat, conn_id) in crate::local_active::matched_connections_or_init(conn) {
            let _ = crate::db::stamp_active_config_attribution(conn, &plat, since_ms, &conn_id);
        }
    });
}

fn running_session_platforms(reader: &crate::ReadPool) -> BTreeSet<String> {
    let recent_cutoff = chrono::Local::now().timestamp_millis() - crate::db::ACTIVE_WINDOW_SECONDS * 1_000;
    reader
        .with(|conn| -> rusqlite::Result<BTreeSet<String>> {
            let mut stmt = conn.prepare_cached(
                "SELECT DISTINCT platform FROM sessions
                  WHERE activity_state = 'running' OR activity_updated_ms >= ?1",
            )?;
            let rows = stmt.query_map([recent_cutoff], |r| r.get::<_, String>(0))?;
            rows.collect()
        })
        .unwrap_or_default()
}

/// 采集一次并把最新实时用量推给前端。
/// 全平台跟踪活跃会话集合（1A）：非本体平台的会话开始与结束同样准时广播 `live-usage`。
fn collect_and_emit(
    app: &AppHandle,
    cursor: &mut Option<i64>,
    active_platforms: &mut HashSet<String>,
    last_today: &mut HashMap<String, Option<i64>>,
    changed_paths: Option<Vec<PathBuf>>,
) {
    let Some(state) = app.try_state::<Db>() else { return };
    let Some(reader) = app.try_state::<DbRead>() else { return };
    let mut writer: &std::sync::Mutex<rusqlite::Connection> = &state.0;

    let previous_cursor = *cursor;
    let scan = match changed_paths {
        Some(paths) => collector::scan_paths(&mut writer, paths),
        None => collector::scan(&mut writer),
    };
    crate::emit_collection_status(app, &scan);
    // 本地代理的计时可能在请求记录入库前到达，扫描后按去重键补写（无队列时零开销）
    writer.write(|conn| crate::proxy::flush_pending(conn));

    let settings = app
        .try_state::<Cfg>()
        .map(|c| c.0.get())
        .unwrap_or_default();
    if scan.records_inserted > 0 {
        stamp_recent_direct_requests(writer, settings.island_linkage.strict_writing);
    }
    let platform = settings.island_platform.clone();

    let primary_usage: LiveUsage =
        match reader.0.with(|conn| crate::session_presence::live_usage(conn, &platform, previous_cursor)) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("[实时] 查询失败: {e}");
                return;
            }
        };

    let first = previous_cursor.is_none();
    let advanced = primary_usage.cursor != previous_cursor.unwrap_or(-1);
    *cursor = Some(primary_usage.cursor);

    let changed = if advanced || scan.records_inserted > 0 {
        changed_platforms(&reader.0, previous_cursor, scan.records_inserted > 0)
    } else {
        BTreeSet::new()
    };

    let mut targets = BTreeSet::new();
    targets.insert(platform.clone());
    for clone in &settings.island_clones {
        if crate::platforms::known(&clone.platform) {
            targets.insert(clone.platform.clone());
        }
    }
    targets.extend(changed.iter().cloned());
    targets.extend(active_platforms.iter().cloned());
    targets.extend(running_session_platforms(&reader.0).into_iter().filter(|p| crate::platforms::known(p)));

    let mut primary_cached = Some(primary_usage);
    for target in targets {
        let mut snapshot = if target == platform {
            match primary_cached.take() {
                Some(u) => u,
                None => continue,
            }
        } else {
            match reader.0.with(|conn| crate::session_presence::live_usage(conn, &target, previous_cursor)) {
                Ok(u) => u,
                Err(_) => continue,
            }
        };
        let is_active = !snapshot.active_sessions.is_empty();
        let was_active = active_platforms.contains(&target);
        let prev_today = last_today.insert(target.clone(), snapshot.today_tokens);
        let today_changed = prev_today.is_some_and(|prev| prev != snapshot.today_tokens);
        if is_active {
            active_platforms.insert(target.clone());
        } else {
            active_platforms.remove(&target);
        }
        let should = first
            || changed.contains(&target)
            || is_active
            || was_active
            || today_changed
            || (target == platform && (advanced || scan.records_inserted > 0));
        if !should {
            continue;
        }
        snapshot.data_changed = first || today_changed || changed.contains(&target);
        if target == platform && advanced && !first && snapshot.delta_tokens > 0 {
            println!("[实时] +{} token ({target}) → 前端", snapshot.delta_tokens);
        }
        let _ = app.emit(EVENT_LIVE, &snapshot);
    }

    if scan.records_inserted > 0 && settings.island_linkage.agent_hooks {
        broadcast_agent_hooks(app);
    }
}

/// 在后台线程启动监听。调用后立即返回。
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        // 容量 1 即可表达“需要扫描”；事件洪峰只合并信号，不线性堆积内存。
        let (tx, rx) = mpsc::sync_channel(1);
        // 路径集合与容量 1 的唤醒信号分离：信号被合并时路径仍会保留，
        // 去抖窗口结束后一次取走，避免事件洪峰漏掉第二个文件。
        let pending_paths = Arc::new(Mutex::new(HashSet::<PathBuf>::new()));
        let callback_paths = pending_paths.clone();
        let registry = claude_registry();
        let native_roots = Arc::new(Mutex::new(crate::native_sources::watch_roots()));
        let callback_roots = native_roots.clone();

        // notify 的 watcher 必须在整个监听期间存活，故绑定到本线程局部变量
        let mut watcher: Option<RecommendedWatcher> =
            match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    let relevant_kind = matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_));
                    if !relevant_kind {
                        return;
                    }
                    let roots = callback_roots
                        .lock()
                        .map(|r| r.clone())
                        .unwrap_or_default();
                    let relevant_path = event
                        .paths
                        .iter()
                        .any(|path| relevant(path, registry.as_deref(), &roots));
                    if relevant_path {
                        if let Ok(mut pending) = callback_paths.lock() {
                            // 进程登记只触发一次实时快照重算，不作为待采集文件
                            pending.extend(event.paths.into_iter().filter(|path| {
                                relevant(path, None, &roots)
                            }));
                        }
                        let _ = tx.try_send(());
                    }
                }
            }) {
                Ok(w) => Some(w),
                Err(e) => {
                    eprintln!("[实时] 无法创建监听器，回落到心跳采集: {e}");
                    None
                }
            };

        let mut bound = HashSet::new();
        if let Some(watcher) = watcher.as_mut() {
            rebind_sources(watcher, &mut bound, true, true);
        }
        let mut heartbeats = 0u32;
        let mut last_rebind = Instant::now();

        let mut cursor: Option<i64> = None;
        let mut active_platforms = HashSet::new();
        let mut last_today = HashMap::new();
        // 先推一次初值，灵动岛打开就有数据，不用等第一次写入
        collect_and_emit(&app, &mut cursor, &mut active_platforms, &mut last_today, None);

        loop {
            // 无锁 CLI（kimi / opencode / qwen 等）处于活跃期时每 2 秒巡检一次进程退出即停表；空闲时保持 10 秒低频心跳
            let wait_for = if active_platforms
                .iter()
                .any(|p| crate::usage::cli_liveness::is_lockless_cli_platform(p))
            {
                Duration::from_secs(2)
            } else {
                HEARTBEAT
            };
            let got = match rx.recv_timeout(wait_for) {
                Ok(_) => true,
                Err(mpsc::RecvTimeoutError::Timeout) => false,
                // 监听器创建失败或运行中断开时继续按心跳补采，不能退出后台采集。
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    std::thread::sleep(HEARTBEAT);
                    false
                }
            };

            if got {
                // 去抖：把紧随其后的抖动一起吃掉，只采集一次
                let deadline = Instant::now() + DEBOUNCE;
                while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                    if rx.recv_timeout(left).is_err() {
                        break;
                    }
                }
                // 持续有活跃文件写入时，每 3 秒顺手补绑一次刚创建的新工具会话目录（仅 stat 根目录，<0.1ms）
                if last_rebind.elapsed() >= Duration::from_secs(3) {
                    if let Ok(mut guard) = native_roots.lock() {
                        *guard = crate::native_sources::watch_roots();
                    }
                    if let Some(watcher) = watcher.as_mut() {
                        rebind_sources(watcher, &mut bound, false, false);
                    }
                    last_rebind = Instant::now();
                }
            } else if let Some(watcher) = watcher.as_mut() {
                heartbeats = heartbeats.wrapping_add(1);
                if let Ok(mut guard) = native_roots.lock() {
                    *guard = crate::native_sources::watch_roots();
                }
                rebind_sources(watcher, &mut bound, heartbeats % FULL_REBIND_EVERY == 0, false);
                last_rebind = Instant::now();
            }

            let changed_paths = if got {
                Some(
                    pending_paths
                        .lock()
                        .map(|mut pending| pending.drain().collect())
                        .unwrap_or_default(),
                )
            } else {
                None
            };
            collect_and_emit(
                &app,
                &mut cursor,
                &mut active_platforms,
                &mut last_today,
                changed_paths,
            );
        }
    });
}


fn normalize_watch_key(path: &Path) -> String {
    if cfg!(windows) {
        path.to_string_lossy().replace('/', "\\").to_ascii_lowercase()
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn invalidate_local_quota_caches(platforms: &[String]) {
    for plat in platforms {
        match plat.as_str() {
            "zcode" => {
                crate::invalidate_network_cache("local-zcode");
                crate::invalidate_network_cache("zcode-connection:local-login");
                crate::invalidate_network_cache("monitor:native:zcode:account");
                crate::invalidate_network_cache("monitor:native:zcode:main");
            }
            "grok" => {
                crate::invalidate_network_cache("local-grok");
                crate::invalidate_network_cache("monitor:native:grok:cli");
            }
            "gemini" => {
                crate::invalidate_network_cache("monitor:native:gemini:cli");
            }
            "opencode" => {
                crate::invalidate_network_cache("monitor:native:opencode:openai");
            }
            "codex" => {
                crate::invalidate_network_cache("local-codex");
            }
            _ => {}
        }
    }
}

/// 同步一次连接、刷新本机生效账号快照（3A + 本地 OAuth 切号缓存失效）并检测全量 Agent 接线漂移；返回最新应监听的文件清单。
fn sync_connections_and_emit(app: &AppHandle) -> Vec<PathBuf> {
    static LAST_WIRING_FP: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();
    static ACTIVE_INITIALIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    let port = app
        .try_state::<Cfg>()
        .map(|c| {
            let p = c.0.get().proxy_port;
            if p > 0 { p } else { 12731 }
        })
        .unwrap_or(12731);

    let Some(state) = app.try_state::<Db>() else { return Vec::new() };
    let Ok(mut conn) = state.0.lock() else { return Vec::new() };
    let synced = crate::connections::sync_from_local(&mut conn);
    let (active_accounts, changed_platforms) = crate::local_active::refresh_and_diff_platforms(&conn);
    let wiring_fp = crate::agent_configs::wiring_fingerprint(&conn, port);
    let customs = crate::custom_agents::list(&conn).unwrap_or_default();
    let targets = home()
        .map(|h| crate::agent_configs::watch_files(&h, &customs))
        .unwrap_or_default();
    // 代理跟随配置时可能要写库（proxy::on_config_changed），先释放写锁
    drop(conn);

    let was_initialized = ACTIVE_INITIALIZED.swap(true, std::sync::atomic::Ordering::AcqRel);
    let mut already_refreshed = false;
    match synced {
        Ok(true) => {
            println!("[连接] 本机配置有变化，已自动更新连接并触发重查");
            let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
            crate::events::refresh_requested(app, None);
            already_refreshed = true;
        }
        Ok(false) => {}
        Err(error) => eprintln!("[连接] 本机配置同步失败: {error}"),
    }
    if !changed_platforms.is_empty() {
        let _ = app.emit(crate::events::ACTIVE_ACCOUNT_CHANGED, &active_accounts);
        if was_initialized {
            invalidate_local_quota_caches(&changed_platforms);
            if !already_refreshed {
                crate::events::refresh_requested(app, None);
            }
        }
    }

    let fp_cell = LAST_WIRING_FP.get_or_init(|| Mutex::new(None));
    let mut fp_guard = fp_cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let wiring_changed = fp_guard.as_ref().is_some_and(|prev| prev != &wiring_fp);
    *fp_guard = Some(wiring_fp);
    drop(fp_guard);
    if wiring_changed {
        let _ = app.emit(crate::events::GATEWAY_WIRING_CHANGED, ());
    }

    // 代理开着时跟随外部改动：外部换网关 → 更新保存的上游并重写代理地址；切回官方 → 放弃接管
    crate::proxy::on_config_changed(app);
    targets
}

/// 监听全量可接线工具与自定义工具的本机配置文件（写入即检测接线漂移 + 外部切号同步）。
/// 监听父目录（NonRecursive）并按目标文件名精确过滤，兼顾原子改名替换（tmp → rename）与新文件首次落盘。
pub fn start_config_sync(app: AppHandle) {
    std::thread::spawn(move || {
        let initial_targets = sync_connections_and_emit(&app);
        let target_keys = Arc::new(Mutex::new(
            initial_targets.iter().map(|p| normalize_watch_key(p)).collect::<HashSet<String>>(),
        ));
        let callback_keys = target_keys.clone();

        let (tx, rx) = mpsc::sync_channel::<()>(1);
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let relevant_kind = matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_));
                if !relevant_kind {
                    return;
                }
                let hit = callback_keys
                    .lock()
                    .map(|keys| event.paths.iter().any(|p| keys.contains(&normalize_watch_key(p))))
                    .unwrap_or(false);
                if hit {
                    let _ = tx.try_send(());
                }
            }
        }) {
            Ok(w) => w,
            Err(error) => {
                eprintln!("[连接] 无法创建配置监听器，自动同步退化为仅启动时对齐: {error}");
                return;
            }
        };

        let mut bound_dirs: HashSet<PathBuf> = HashSet::new();
        let bind_dirs = |watcher: &mut RecommendedWatcher, bound: &mut HashSet<PathBuf>, files: &[PathBuf]| {
            let wanted_dirs: BTreeSet<PathBuf> = files
                .iter()
                .filter_map(|p| p.parent().map(Path::to_path_buf))
                .filter(|dir| dir.is_dir())
                .collect();
            let stale: Vec<PathBuf> = bound.iter().filter(|d| !wanted_dirs.contains(*d)).cloned().collect();
            for dir in stale {
                let _ = watcher.unwatch(&dir);
                bound.remove(&dir);
            }
            for dir in wanted_dirs {
                if bound.contains(&dir) {
                    continue;
                }
                match watcher.watch(&dir, RecursiveMode::NonRecursive) {
                    Ok(()) => {
                        bound.insert(dir);
                    }
                    Err(error) => eprintln!("[连接] 监听目录 {} 失败: {error}", dir.display()),
                }
            }
        };
        bind_dirs(&mut watcher, &mut bound_dirs, &initial_targets);

        loop {
            // 15 秒兜底重绑一次新出现的工具配置目录（如用户刚装好某个 CLI 首次建目录）
            let woke = match rx.recv_timeout(Duration::from_secs(15)) {
                Ok(()) => true,
                Err(mpsc::RecvTimeoutError::Timeout) => false,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };
            if woke {
                // 去抖 500ms：CC Switch 等切换时可能连续改写多个文件，等落定后再读
                let deadline = Instant::now() + CONFIG_SETTLE;
                while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                    if rx.recv_timeout(left).is_err() {
                        break;
                    }
                }
            }
            let next_targets = sync_connections_and_emit(&app);
            if let Ok(mut guard) = target_keys.lock() {
                *guard = next_targets.iter().map(|p| normalize_watch_key(p)).collect();
            }
            bind_dirs(&mut watcher, &mut bound_dirs, &next_targets);
        }
    });
}

/// 广播最新的受控 Agent Hooks 状态（4B）；无变化时不重发。
pub(crate) fn broadcast_agent_hooks(app: &AppHandle) {
    static LAST_EMITTED: std::sync::OnceLock<Mutex<Option<Vec<crate::agent_hooks::AgentHookAlertDto>>>> =
        std::sync::OnceLock::new();
    let Some(cfg) = app.try_state::<Cfg>() else { return };
    let settings = cfg.0.get();
    let alerts = if settings.island_linkage.agent_hooks {
        let data_dir = cfg.0.dir().to_path_buf();
        let now_ms = chrono::Local::now().timestamp_millis();
        match app.try_state::<DbRead>() {
            Some(reader) => reader
                .0
                .with(|conn| -> rusqlite::Result<_> {
                    Ok(crate::agent_hooks::active_alerts(&data_dir, Some(conn), now_ms))
                })
                .unwrap_or_default(),
            None => crate::agent_hooks::active_alerts(&data_dir, None, now_ms),
        }
    } else {
        Vec::new()
    };
    let cell = LAST_EMITTED.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.as_ref() == Some(&alerts) {
        return;
    }
    *guard = Some(alerts.clone());
    drop(guard);
    let _ = app.emit(crate::events::AGENT_HOOK_CHANGED, &alerts);
}

/// 监听 `<data_dir>/agent-hooks/` 目录（4B），状态回执落地后即时通知灵动岛。
pub fn start_agent_hooks_sync(app: AppHandle) {
    std::thread::spawn(move || {
        let Some(cfg) = app.try_state::<Cfg>() else { return };
        let hooks_dir = crate::agent_hooks::hooks_state_dir(cfg.0.dir());
        let _ = std::fs::create_dir_all(&hooks_dir);
        broadcast_agent_hooks(&app);

        let (tx, rx) = mpsc::sync_channel::<()>(1);
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)) {
                    let _ = tx.try_send(());
                }
            }
        }) {
            Ok(w) => w,
            Err(error) => {
                eprintln!("[Agent Hooks] 无法监听回执目录，退化为心跳检查: {error}");
                loop {
                    std::thread::sleep(Duration::from_secs(3));
                    broadcast_agent_hooks(&app);
                }
            }
        };

        let mut watching = false;
        if hooks_dir.exists() && watcher.watch(&hooks_dir, RecursiveMode::NonRecursive).is_ok() {
            watching = true;
        }

        loop {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(()) => {
                    let deadline = Instant::now() + HOOK_DEBOUNCE;
                    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                        if rx.recv_timeout(left).is_err() {
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !watching && hooks_dir.exists() {
                        watching = watcher.watch(&hooks_dir, RecursiveMode::NonRecursive).is_ok();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            broadcast_agent_hooks(&app);
        }
    });
}
