//! 本地代理与模型入口宿主生命周期：监听绑定、路由组装、自愈与回退。
//!
//! 后台异步补查额度的动作通过 [`register_quota_refresher`] 由应用层注入，避免反向依赖 `commands/*`。

use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::DefaultBodyLimit;
use axum::Router;
use tauri::{Emitter, Manager};

use super::deps::{
    stop, ClaudeSessions, KeyIndex, ProxyDeps, ProxyRuntime, FALLBACK_REASON, LAST_ERROR, PENDING,
    PROXY, REQUEST_BODY_LIMIT,
};
use super::observe::forward;
use crate::proxy_config::{is_self_base, ProxyPlatform};

type QuotaRefresher = fn(&tauri::AppHandle, &str);

static QUOTA_REFRESHER: OnceLock<QuotaRefresher> = OnceLock::new();

/// 应用启动时注册「分组 pace 选路发现额度陈旧时的异步补查函数」。
pub(crate) fn register_quota_refresher(refresher: QuotaRefresher) {
    let _ = QUOTA_REFRESHER.set(refresher);
}

pub(crate) async fn bind_listener(
    bind_host: &'static str,
    port: u16,
    mode: crate::gateway::Mode,
) -> Result<(tokio::net::TcpListener, u16), String> {
    match tokio::net::TcpListener::bind((bind_host, port)).await {
        Ok(listener) => Ok((listener, port)),
        Err(error)
            if error.kind() == std::io::ErrorKind::AddrInUse
                && mode == crate::gateway::Mode::Observe =>
        {
            let listener = tokio::net::TcpListener::bind((bind_host, 0))
                .await
                .map_err(|fallback| format!("端口 {port} 已被占用，且无法分配随机端口：{fallback}"))?;
            let actual = listener
                .local_addr()
                .map_err(|e| format!("读取监听端口失败：{e}"))?
                .port();
            eprintln!("[代理] 端口 {port} 已被占用，改用 {actual}");
            Ok((listener, actual))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => Err(format!(
            "端口 {port} 已被占用；模型入口必须使用固定端口，请先释放该端口或换一个端口再接入工具"
        )),
        Err(error) => Err(format!("监听 {bind_host}:{port} 失败：{error}")),
    }
}

pub fn set_proxy(
    app: &tauri::AppHandle,
    port: u16,
    enabled: bool,
    fallback_direct: bool,
) -> Result<crate::settings::Settings, String> {
    crate::cli_switch::with_switch(|| set_proxy_locked(app, port, enabled, fallback_direct))
}

fn set_proxy_locked(
    app: &tauri::AppHandle,
    port: u16,
    enabled: bool,
    fallback_direct: bool,
) -> Result<crate::settings::Settings, String> {
    if !(1024..=65535).contains(&port) {
        return Err("端口须在 1024–65535 之间".into());
    }
    if let Ok(mut slot) = FALLBACK_REASON.lock() {
        *slot = None;
    }

    let (current_mode, _) = crate::gateway::mode_from(&app.state::<crate::Cfg>().0.get());
    if current_mode == crate::gateway::Mode::Relay {
        if !enabled {
            let outcome = crate::gateway::escape::restore_all(app);
            stop();
            let next = app.state::<crate::Cfg>().0.try_update(|settings| {
                settings.proxy_enabled = false;
                settings.proxy_port = port;
                settings.proxy_fallback_direct = fallback_direct;
            })?;
            crate::events::settings_changed(app);
            if !outcome.failed.is_empty() {
                eprintln!("[模型入口] 关闭时部分工具还原失败：{}", outcome.summary());
            }
            return Ok(next);
        }
        stop();
        let next = app.state::<crate::Cfg>().0.try_update(|settings| {
            settings.proxy_enabled = true;
            settings.proxy_port = port;
            settings.proxy_fallback_direct = fallback_direct;
        })?;
        if let Err(error) = start(app) {
            app.state::<crate::Cfg>()
                .0
                .update(|settings| settings.proxy_enabled = false);
            crate::events::settings_changed(app);
            return Err(error);
        }
        crate::events::settings_changed(app);
        return Ok(next);
    }

    if !enabled {
        stop();
        if let Err(error) = crate::proxy_config::restore_all_saved(app) {
            eprintln!("[代理] 还原 CLI 配置出现问题：{error}");
        }
        let next = app.state::<crate::Cfg>().0.try_update(|settings| {
            settings.proxy_enabled = false;
            settings.proxy_port = port;
            settings.proxy_fallback_direct = fallback_direct;
        })?;
        crate::events::settings_changed(app);
        return Ok(next);
    }

    stop();
    if let Err(error) = crate::proxy_config::restore_all_saved(app) {
        eprintln!("[代理] 重新接管前还原旧配置出现问题：{error}");
    }

    let mut originals = HashMap::new();
    let mut skipped = Vec::new();
    for platform in [ProxyPlatform::Claude, ProxyPlatform::Codex] {
        match crate::proxy_config::probe_original(platform) {
            Ok(original) => {
                originals.insert(platform, original);
            }
            Err(reason) => skipped.push(format!("{}：{reason}", platform.display_name())),
        }
    }
    if originals.is_empty() {
        return Err(format!("没有可接管的平台（{}）", skipped.join("；")));
    }

    app.state::<crate::Cfg>()
        .0
        .try_update(|settings| {
            settings.proxy_enabled = true;
            settings.proxy_port = port;
            settings.proxy_fallback_direct = fallback_direct;
            settings.proxy_claude_upstream = originals.get(&ProxyPlatform::Claude).cloned();
            settings.proxy_codex_upstream = originals.get(&ProxyPlatform::Codex).cloned();
        })
        .map_err(|error| format!("保存代理设置失败：{error}"))?;

    if let Err(error) = start(app) {
        rollback(app);
        return Err(format!("代理监听启动失败：{error}"));
    }
    let actual_port = PROXY
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|runtime| runtime.port))
        .unwrap_or(port);
    let proxy_base = format!("http://127.0.0.1:{actual_port}");
    if let Err(error) = crate::proxy_config::apply_takeover(app, &proxy_base) {
        rollback(app);
        return Err(error);
    }

    let next = app.state::<crate::Cfg>().0.get();
    crate::events::settings_changed(app);
    if actual_port != port {
        eprintln!(
            "[代理] 端口 {port} 已被占用，实际监听 127.0.0.1:{actual_port}，CLI 已按实际端口接管"
        );
    } else {
        println!("[代理] 已启动 127.0.0.1:{actual_port}");
    }
    Ok(next)
}

fn rollback(app: &tauri::AppHandle) {
    if let Err(error) = crate::proxy_config::restore_all_saved(app) {
        eprintln!("[代理] 回滚还原 CLI 配置失败：{error}");
    }
    let _ = app.state::<crate::Cfg>().0.try_update(|settings| {
        settings.proxy_enabled = false;
        settings.proxy_claude_upstream = None;
        settings.proxy_codex_upstream = None;
    });
    crate::events::settings_changed(app);
}

fn build_proxy_deps(
    app: &tauri::AppHandle,
    settings: &crate::settings::Settings,
    mode: crate::gateway::Mode,
    lan_shared: bool,
    upstreams: HashMap<ProxyPlatform, String>,
    shutdown: Arc<tokio::sync::watch::Sender<bool>>,
) -> Result<Arc<ProxyDeps>, String> {
    let client = crate::net::async_builder()
        .connect_timeout(Duration::from_secs(15))
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(16)
        .tcp_nodelay(true)
        .build()
        .map_err(|error| format!("构建上游客户端失败：{error}"))?;

    let app_for_fallback = app.clone();
    let on_fallback: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |reason: &str| {
        eprintln!("[代理] 上游连续不可达，自动回退直连：{reason}");
        if let Ok(mut slot) = FALLBACK_REASON.lock() {
            *slot = Some(reason.to_string());
        }
        if let Err(error) = crate::proxy_config::restore_all_saved(&app_for_fallback) {
            eprintln!("[代理] 回退还原 CLI 配置失败：{error}");
        }
        app_for_fallback
            .state::<crate::Cfg>()
            .0
            .update(|settings| {
                settings.proxy_enabled = false;
            });
        crate::events::settings_changed(&app_for_fallback);
        if let Ok(mut guard) = PROXY.lock() {
            *guard = None;
        }
    });

    let deps = Arc::new(ProxyDeps {
        client,
        db: app.state::<crate::Db>().0.clone(),
        upstreams,
        pending: PENDING.clone(),
        consecutive_failures: Arc::new(AtomicU32::new(0)),
        fallback_enabled: settings.proxy_fallback_direct,
        key_index: Mutex::new(KeyIndex::default()),
        shutdown,
        on_fallback,
        pool_index: if mode == crate::gateway::Mode::Relay {
            Some(Mutex::new(crate::gateway::route::PoolIndex::new()))
        } else {
            None
        },
        gateway_default_connection: if mode == crate::gateway::Mode::Relay {
            let app = app.clone();
            Some(Arc::new(move || {
                app.state::<crate::Cfg>().0.get().gateway_default_connection
            }))
        } else {
            None
        },
        affinity: if mode == crate::gateway::Mode::Relay {
            Some(Mutex::new(crate::gateway::route::Affinity::new()))
        } else {
            None
        },
        cooldowns: if mode == crate::gateway::Mode::Relay {
            Some(Mutex::new(crate::gateway::route::Cooldowns::new()))
        } else {
            None
        },
        claude_sessions: if mode == crate::gateway::Mode::Relay {
            Some(Mutex::new(ClaudeSessions::new()))
        } else {
            None
        },
        circuit_threshold: settings.gateway_circuit_threshold.max(1),
        live_circuit_threshold: {
            let app = app.clone();
            Some(Arc::new(move || {
                app.state::<crate::Cfg>()
                    .0
                    .get()
                    .gateway_circuit_threshold
                    .max(1)
            }))
        },
        on_circuit: {
            let app = app.clone();
            Arc::new(move |reason: &str| {
                if !app.state::<crate::Cfg>().0.get().gateway_auto_escape {
                    eprintln!("[网关] 连续失败达阈值，但自动逃生已关闭：{reason}");
                    return;
                }
                crate::gateway::escape::circuit_break(&app, reason);
            })
        },
        on_activity: {
            let app = app.clone();
            Arc::new(move |name: &'static str, payload: serde_json::Value| {
                let _ = app.emit(name, payload);
            })
        },
        on_quota_stale: {
            let app = app.clone();
            Arc::new(move |connection_id: &str| {
                if let Some(refresher) = QUOTA_REFRESHER.get() {
                    refresher(&app, connection_id);
                }
            })
        },
        gateway_auth_relaxed: mode == crate::gateway::Mode::Relay && !lan_shared,
        gateway_lan_shared: lan_shared,
        request_uids: Default::default(),
    });
    if mode == crate::gateway::Mode::Relay {
        deps.request_uids.prefetch(&deps.db);
    }
    Ok(deps)
}

pub fn start(app: &tauri::AppHandle) -> Result<(), String> {
    let settings = app.state::<crate::Cfg>().0.get();
    let port = settings.proxy_port;
    let (mode, mode_warn) = crate::gateway::mode_from(&settings);
    if let Some(warn) = mode_warn {
        eprintln!("[代理] {warn}");
    }
    let mut upstreams = HashMap::new();
    if mode == crate::gateway::Mode::Observe {
        if let Some(base) = settings
            .proxy_claude_upstream
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            upstreams.insert(
                ProxyPlatform::Claude,
                base.trim().trim_end_matches('/').to_string(),
            );
        }
        if let Some(base) = settings
            .proxy_codex_upstream
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            upstreams.insert(
                ProxyPlatform::Codex,
                base.trim().trim_end_matches('/').to_string(),
            );
        }
    }
    if mode == crate::gateway::Mode::Observe && upstreams.is_empty() {
        return Err("没有已接管的平台上游，无法启动".into());
    }

    let lan_shared = mode == crate::gateway::Mode::Relay && settings.gateway_shared_lan;
    let bind_host: &'static str = if lan_shared { "0.0.0.0" } else { "127.0.0.1" };

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let shutdown = Arc::new(shutdown_tx);
    let deps = build_proxy_deps(app, &settings, mode, lan_shared, upstreams, shutdown.clone())?;
    let router = build_router(mode, deps.clone());
    match crate::gateway::allow::load(&settings.gateway_allow_ips) {
        Ok((count, source)) if count > 0 => {
            eprintln!("[网关] 来源地址白名单生效：{count} 条（来自{source}）")
        }
        Ok(_) => {}
        Err(error) => {
            eprintln!("[网关] 来源地址白名单无效，只放行本机：{error}");
            let _ = crate::gateway::allow::load(&["127.0.0.1/32".to_string()]);
        }
    }

    if lan_shared {
        eprintln!(
            "[网关] 局域网共享已开启：监听 0.0.0.0:{port}，凭据走严格校验（命名客户端 Key / 手动 Key）"
        );
    }

    let (listener, port) = tauri::async_runtime::block_on(bind_listener(bind_host, port, mode))?;

    let signal = async move {
        let mut receiver = shutdown_rx;
        loop {
            if *receiver.borrow_and_update() {
                break;
            }
            if receiver.changed().await.is_err() {
                break;
            }
        }
    };
    tauri::async_runtime::spawn(async move {
        if let Err(error) = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(signal)
        .await
        {
            eprintln!("[代理] 服务异常退出：{error}");
        }
    });

    *PROXY.lock().expect("代理状态锁") = Some(Arc::new(ProxyRuntime {
        port,
        mode,
        shutdown,
        deps,
    }));
    Ok(())
}

pub(crate) fn build_router(mode: crate::gateway::Mode, deps: Arc<ProxyDeps>) -> Router {
    match mode {
        crate::gateway::Mode::Observe => Router::new()
            .fallback(forward)
            .layer(DefaultBodyLimit::max(REQUEST_BODY_LIMIT))
            .with_state(deps),
        crate::gateway::Mode::Relay => crate::gateway::server::router(deps),
    }
}

pub fn startup(app: &tauri::AppHandle) {
    crate::gateway::escape::startup(app);
    let settings = app.state::<crate::Cfg>().0.get();
    if settings.proxy_enabled {
        if let Err(error) = set_proxy(
            app,
            settings.proxy_port,
            true,
            settings.proxy_fallback_direct,
        ) {
            eprintln!("[代理] 启动恢复失败，保持直连：{error}");
        } else {
            println!("[代理] 已按上次设置恢复接管（端口 {}）", settings.proxy_port);
        }
    }
    heal_orphans(app);
}

pub(crate) fn heal_orphans(app: &tauri::AppHandle) {
    let _ = crate::cli_switch::with_switch(|| {
        heal_orphans_locked(app);
        Ok::<(), String>(())
    });
}

fn heal_orphans_locked(app: &tauri::AppHandle) {
    const MIN_HEAL_INTERVAL: Duration = Duration::from_secs(10);
    static LAST_HEAL: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(|| Mutex::new(None));
    if let Ok(mut last) = LAST_HEAL.lock() {
        if last.is_some_and(|t| t.elapsed() < MIN_HEAL_INTERVAL) {
            return;
        }
        *last = Some(Instant::now());
    }

    let settings = app.state::<crate::Cfg>().0.get();
    let runtime = PROXY.lock().ok().and_then(|guard| guard.clone());
    heal_observed_orphans(&settings, runtime.as_deref());
}

pub(crate) fn heal_observed_orphans(
    settings: &crate::settings::Settings,
    runtime: Option<&ProxyRuntime>,
) {
    let mode = runtime
        .map(|rt| rt.mode)
        .unwrap_or_else(|| crate::gateway::mode_from(settings).0);
    if mode == crate::gateway::Mode::Relay {
        return;
    }
    let port = settings.proxy_port;
    for (platform, saved) in [
        (ProxyPlatform::Claude, settings.proxy_claude_upstream.clone()),
        (ProxyPlatform::Codex, settings.proxy_codex_upstream.clone()),
    ] {
        if runtime.is_some_and(|rt| rt.deps.upstreams.contains_key(&platform)) {
            continue;
        }
        let Ok(Some(current)) = crate::proxy_config::read_current_base(platform) else {
            continue;
        };
        if !is_self_base(&current, port) {
            continue;
        }
        match saved.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(original) => {
                if let Err(error) =
                    crate::proxy_config::restore_platform(platform, Some(original), port)
                {
                    eprintln!("[代理] 清理 {} 残留失败：{error}", platform.display_name());
                } else {
                    println!(
                        "[代理] 检测到 {} 配置指向已停止的代理（疑似外部工具回写），已自动还原直连",
                        platform.display_name()
                    );
                }
            }
            None => {
                eprintln!(
                    "[代理] {} 配置指向本机代理但无已保存上游，无法自动还原；请手动改回网关地址或重启 CC Switch",
                    platform.display_name()
                );
            }
        }
    }
}

pub fn on_config_changed(app: &tauri::AppHandle) {
    let _ = crate::cli_switch::with_switch(|| {
        on_config_changed_locked(app);
        Ok::<(), String>(())
    });
}

fn on_config_changed_locked(app: &tauri::AppHandle) {
    heal_orphans_locked(app);
    let Some(runtime) = PROXY.lock().expect("代理状态锁").clone() else {
        return;
    };
    if follow_external_configs(&app.state::<crate::Cfg>().0, &runtime) {
        crate::events::settings_changed(app);
    }
}

pub(crate) fn follow_external_configs(
    store: &crate::settings::Store,
    runtime: &ProxyRuntime,
) -> bool {
    if runtime.mode == crate::gateway::Mode::Relay {
        return false;
    }
    let proxy_base = format!("http://127.0.0.1:{}", runtime.port);
    let mut changed = false;
    for platform in [ProxyPlatform::Claude, ProxyPlatform::Codex] {
        if !runtime.deps.upstreams.contains_key(&platform) {
            continue;
        }
        let Ok(current) = crate::proxy_config::read_current_base(platform) else {
            continue;
        };
        match current.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(value) if is_self_base(value, runtime.port) => {}
            Some(external) => {
                let normalized = external.trim().trim_end_matches('/').to_string();
                if let Err(error) = crate::proxy_config::write_proxy_base(platform, &proxy_base) {
                    eprintln!(
                        "[代理] 跟随外部切换失败（{}）：{error}",
                        platform.display_name()
                    );
                    continue;
                }
                if store
                    .try_update(|settings| set_upstream(settings, platform, Some(normalized)))
                    .is_ok()
                {
                    changed = true;
                    println!(
                        "[代理] {} 上游已跟随外部配置更新并保持接管",
                        platform.display_name()
                    );
                }
            }
            None => {
                if let Err(error) =
                    crate::proxy_config::restore_platform(platform, None, runtime.port)
                {
                    eprintln!(
                        "[代理] 放弃 {} 接管时清理失败：{error}",
                        platform.display_name()
                    );
                    continue;
                }
                if store
                    .try_update(|settings| set_upstream(settings, platform, None))
                    .is_ok()
                {
                    changed = true;
                    println!("[代理] {} 已切回官方直连，放弃接管", platform.display_name());
                }
            }
        }
    }
    changed
}

fn set_upstream(
    settings: &mut crate::settings::Settings,
    platform: ProxyPlatform,
    value: Option<String>,
) {
    match platform {
        ProxyPlatform::Claude => settings.proxy_claude_upstream = value,
        ProxyPlatform::Codex => settings.proxy_codex_upstream = value,
    }
}

pub fn shutdown_and_restore(app: &tauri::AppHandle) {
    let Some(store) = app.try_state::<crate::Cfg>() else {
        stop();
        return;
    };
    let settings = store.0.get();
    let mode = PROXY
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|rt| rt.mode))
        .unwrap_or_else(|| crate::gateway::mode_from(&settings).0);
    if mode == crate::gateway::Mode::Relay {
        let result = crate::cli_switch::with_switch(|| {
            let outcome = crate::gateway::escape::restore_all(app);
            if outcome.failed.is_empty() {
                Ok(())
            } else {
                Err(outcome.summary())
            }
        });
        if let Err(error) = result {
            eprintln!("[模型入口] 退出还原失败：{error}");
        }
        stop();
        return;
    }
    stop();
    if settings.proxy_claude_upstream.is_some() || settings.proxy_codex_upstream.is_some() {
        if let Err(error) = crate::proxy_config::restore_all_saved(app) {
            eprintln!("[代理] 退出还原失败：{error}");
        }
    }
}

#[derive(serde::Serialize)]
pub struct ProxyStatus {
    pub running: bool,
    pub port: u16,
    pub fallback_reason: Option<String>,
    pub last_error: Option<String>,
    pub claude_upstream: Option<String>,
    pub codex_upstream: Option<String>,
    pub claude_taken_over: bool,
    pub codex_taken_over: bool,
}

pub fn status(app: &tauri::AppHandle) -> ProxyStatus {
    let settings = app.state::<crate::Cfg>().0.get();
    let runtime = PROXY.lock().ok().and_then(|guard| guard.clone());
    let observing = runtime
        .as_ref()
        .is_some_and(|rt| rt.mode == crate::gateway::Mode::Observe);
    ProxyStatus {
        running: runtime.is_some(),
        port: settings.proxy_port,
        fallback_reason: FALLBACK_REASON.lock().ok().and_then(|slot| slot.clone()),
        last_error: LAST_ERROR.lock().ok().and_then(|slot| slot.clone()),
        claude_upstream: settings.proxy_claude_upstream.clone(),
        codex_upstream: settings.proxy_codex_upstream.clone(),
        claude_taken_over: observing && settings.proxy_claude_upstream.is_some(),
        codex_taken_over: observing && settings.proxy_codex_upstream.is_some(),
    }
}
