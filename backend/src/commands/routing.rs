//! 模型路由：网关状态、对外访问、中间件、分组、预设与流水。

#[allow(unused_imports)]
use crate::*;

/// 网关总览：类型定义在 service 层（CLI 也用同一份），这里只做别名。
pub(crate) type GatewayStatus = service::gateway::status::GatewayStatus;

/// 设置 / 清除「默认来源」。
///
/// 网关的模型名有两种写法：带前缀 `<来源>/<模型>` 按前缀路由，不带前缀时**只能**靠默认来源兜底。
/// 没有默认来源时无前缀请求会被直接拒绝（`route::split_model_id` 返回 None 后没有兜底）。
/// 这就是「一键配置后工具仍报未带来源前缀」的原因，所以必须有一个能单独设置它的入口。
///
/// 只接受**正在服务**（`serve_enabled`）的连接：指向一个没进模型池的来源，等于每条请求都失败。
/// 传 `None` 表示清除，清掉后无前缀请求重新变成明确报错，而不是静默乱路由。
#[tauri::command]
pub(crate) fn gateway_set_default_source(app: tauri::AppHandle, connection_id: Option<String>) -> Result<Option<String>, String> {
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
pub(crate) fn gateway_status(app: tauri::AppHandle) -> GatewayStatus {
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
pub(crate) fn gateway_access_key(app: tauri::AppHandle, refresh: bool) -> Result<String, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    crate::gateway::access::masked(&conn, refresh)
}

#[tauri::command]
pub(crate) fn gateway_copy_key(app: tauri::AppHandle) -> Result<(), String> {
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
pub(crate) fn gateway_shared_lan_set(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
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
pub(crate) async fn zcode_version_check() -> Result<zcode_version::VersionCheck, String> {
    tauri::async_runtime::spawn_blocking(|| zcode_version::check(zcode_version::installed_version()))
        .await
        .map_err(|_| "无法读取 ZCode 版本".to_string())
}

/// 网关中间件（选路前改写请求体）：列表按执行顺序返回。
#[tauri::command]
pub(crate) fn gateway_middleware_list(app: tauri::AppHandle) -> Result<Vec<crate::gateway::middleware::Middleware>, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    service::gateway::middleware::list(&conn)
}

/// 新增或覆盖一个中间件：配置先在样例请求上试跑，跑不通不保存。下一个请求即生效。
#[tauri::command]
pub(crate) fn gateway_middleware_save(
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
pub(crate) fn gateway_middleware_delete(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法写入网关状态")?;
    service::gateway::middleware::delete(&conn, &name)
}

/// 列出命名客户端 Key：名称 + 掩码 + 限额 + 最近使用与当日计数。
/// Key 原文不出后端——列表里只有掩码，原文仅在签发结果里出现一次。
#[tauri::command]
pub(crate) fn gateway_client_keys_list(app: tauri::AppHandle) -> Result<Vec<crate::gateway::access::ClientKeySummary>, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    service::gateway::client_keys::list(&conn)
}

/// 签发一把命名客户端 Key，返回结果里带 Key 原文（只此一次，复制给客户端）。
/// `tokens_per_day` / `requests_per_day` 传 None 表示不限；请求数限额即时生效，
/// Token 限额由 `gateway::budget` 预扣 + 回冲强制（超量 429，次日零点重置）。
#[tauri::command]
pub(crate) fn gateway_client_key_issue(
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
pub(crate) fn gateway_client_key_revoke(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "无法读取网关状态")?;
    service::gateway::client_keys::revoke(&conn, &name)
}

/// 切换网关模式（observe / relay）。切到 relay 时按当前设置启动监听；
/// 切回 observe 前先还原全部接线，保证两套模式不交叉占用同一批配置文件。
#[tauri::command]
pub(crate) fn gateway_set_mode(
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

/// ZCode Start Plan 优先开关（③5，gateway/adapter/zcode.rs）：开启后 ZCode 来源
/// 先试免费的 Start Plan 通道（会话 JWT），非 2xx 再落 Coding Plan Key；关闭时
/// 保持现状（Coding Plan Key 先、429/402 余额类失败落 Start Plan）。进程级开关
/// 改完立即生效，不需要重启代理；前端接入口 = `settings.zcode_free_first` 字段
/// + 本命令（界面接入口待设计稿确认后再接）。
#[tauri::command]
pub(crate) fn zcode_free_first_set(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
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
pub(crate) fn gateway_set_serve(app: tauri::AppHandle, connection_id: String, enabled: bool) -> Result<(), String> {
    let state = app.state::<Db>();
    let db = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    // 选择来源和调用就绪分开：允许预先选择订阅，实际请求仍由对应适配器核实授权。
    connections::set_serve(&db, &connection_id, enabled)
}

/// 批量加入 / 移出模型池的单项结果。
#[derive(serde::Serialize)]
pub(crate) struct ServeBatchItem {
    connection_id: String,
    ok: bool,
    error: Option<String>,
}

/// 批量加入 / 移出模型池（管理来源弹窗）：逐个执行，单个失败不影响其他来源，结果逐项返回。
#[tauri::command]
pub(crate) fn gateway_set_serve_many(app: tauri::AppHandle, connection_ids: Vec<String>, enabled: bool) -> Result<Vec<ServeBatchItem>, String> {
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
pub(crate) fn gateway_source_usage(app: tauri::AppHandle) -> Result<Vec<crate::gateway::route_log::SourceUsage>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::route_log::source_usage(&conn).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn gateway_source_capabilities(app: tauri::AppHandle) -> Result<Vec<crate::gateway::subscription::Capability>, String> {
    let state = app.state::<Db>();
    let db = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::subscription::capabilities(&db)
}

/// 命名客户端 Key 的同时请求数上限：None = 不限，1–1000。立即对之后到达的请求生效。
#[tauri::command]
pub(crate) fn gateway_client_key_concurrency_set(app: tauri::AppHandle, name: String, cap: Option<i64>) -> Result<(), String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::access::client_key_set_concurrency(&conn, &name, cap)
}

/// 命名客户端 Key 的 Token / 费用上限与账号范围（2.6）：上限留空即不限，账号范围为空即不限。
#[tauri::command]
pub(crate) fn gateway_client_key_limits_set(
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
pub(crate) async fn gateway_models_dev_refresh(app: tauri::AppHandle) -> Result<(usize, i64), String> {
    tauri::async_runtime::spawn_blocking(move || crate::gateway::models_dev::refresh_now(&app))
        .await
        .map_err(|error| format!("刷新任务失败：{error}"))?
}

/// 模型元数据现状：缓存的拉取时间（0 = 还在用内置快照）与模型数。
#[tauri::command]
pub(crate) fn gateway_models_dev_status(app: tauri::AppHandle) -> (i64, usize) {
    crate::gateway::models_dev::cached(&app).map(|c| (c.fetched_ms, c.models.len())).unwrap_or((0, 0))
}

/// 局域网共享的来源地址白名单（P2-9）：IP 或 CIDR，空数组 = 不限；写错的一项会整体拒存。
/// 立即生效（不用重启网关）；`JOLU_ALLOW_IPS` 环境变量设了时以它为准，这里只保存。
#[tauri::command]
pub(crate) fn gateway_allow_ips_set(app: tauri::AppHandle, list: Vec<String>) -> Result<settings::Settings, String> {
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
pub(crate) fn gateway_allow_ips_from_env() -> bool {
    crate::gateway::allow::from_env()
}

#[tauri::command]
pub(crate) fn gateway_groups(app: tauri::AppHandle) -> Result<crate::gateway::groups::GroupsDto, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::list(&conn)
}

#[tauri::command]
pub(crate) fn gateway_group_create(
    app: tauri::AppHandle,
    kind: String,
    name: String,
) -> Result<crate::gateway::groups::GroupDto, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::create(&conn, &kind, &name)
}

#[tauri::command]
pub(crate) fn gateway_group_rename(app: tauri::AppHandle, kind: String, id: String, name: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::rename(&conn, &kind, &id, &name)
}

#[tauri::command]
pub(crate) fn gateway_group_delete(app: tauri::AppHandle, kind: String, id: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::delete(&conn, &kind, &id)
}

#[tauri::command]
pub(crate) fn gateway_group_reorder(app: tauri::AppHandle, kind: String, ids: Vec<String>) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    service::gateway::groups::reorder(&conn, &kind, &ids)
}

#[tauri::command]
pub(crate) fn gateway_group_move(
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
pub(crate) fn gateway_set_slug(app: tauri::AppHandle, connection_id: String, slug: String) -> Result<String, String> {
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
pub(crate) fn gateway_catalog(app: tauri::AppHandle) -> Result<Vec<crate::gateway::catalog::CatalogModel>, String> {
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
pub(crate) fn gateway_profiles_list(app: tauri::AppHandle) -> Result<Vec<crate::gateway::profiles::Profile>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::profiles::list(&conn).map_err(|error| error.to_string())
}

/// 保存当前已接线 agent 的主模型为一个 Profile。
#[tauri::command]
pub(crate) fn gateway_profiles_save(app: tauri::AppHandle, name: String) -> Result<crate::gateway::profiles::Profile, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let agents = crate::gateway::profiles::capture_current(&conn)?;
    let default_connection = app.state::<Cfg>().0.get().gateway_default_connection;
    let extras = crate::gateway::profiles::capture_extras(&conn, default_connection, chrono::Utc::now().timestamp_millis());
    crate::gateway::profiles::save_with(&conn, &name, &agents, &extras)
}

/// 删除 Profile。
#[tauri::command]
pub(crate) fn gateway_profiles_delete(app: tauri::AppHandle, name: String) -> Result<bool, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::profiles::delete(&conn, &name)
}

/// 应用 Profile：对「当前已接线且在 Profile 里」的 agent 逐个重放接线。
///
/// 走 [`gateway::escape::wire_agents`] 的完整路径（备份 / 原子写 / 快照 / 1M 标记）；
/// 未接线与配置漂移的 agent 跳过并说明——Profile 不静默覆盖用户的手动改动。
#[tauri::command]
pub(crate) fn gateway_profiles_apply(app: tauri::AppHandle, name: String) -> Result<crate::gateway::profiles::ApplyResult, String> {
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

/// 路由分组列表。
#[tauri::command]
pub(crate) fn gateway_routing_group_list(app: tauri::AppHandle) -> Result<Vec<crate::gateway::routing_groups::RoutingGroup>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::routing_groups::list(&conn).map_err(|error| error.to_string())
}

/// 创建路由分组。成员是 `[{slug, model}]`；slug 为来源 id，模型可带或不带 `[1m]` 标记。
#[tauri::command]
pub(crate) fn gateway_routing_group_create(
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
pub(crate) fn gateway_routing_group_update(
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
pub(crate) fn gateway_routing_group_delete(app: tauri::AppHandle, slug: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::routing_groups::delete(&conn, &slug)?;
    let _ = app.emit("gateway-catalog-changed", ());
    Ok(())
}

/// 路由流水：最近 20 条网关请求（不含提示词与凭证）。
#[tauri::command]
pub(crate) fn gateway_route_log(app: tauri::AppHandle) -> Result<Vec<crate::gateway::route_log::RouteLogEntry>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::route_log::recent(&conn, 20).map_err(|error| error.to_string())
}

/// 路由流水分页（统一网关）：`upper_id` 为空时以当前最新一条建立查询范围，翻页时带回同一上界。
#[tauri::command]
pub(crate) fn gateway_route_log_page(
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
pub(crate) fn gateway_sessions_overview(app: tauri::AppHandle) -> Result<crate::gateway::sessions::SessionsOverview, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::sessions::overview(&conn).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn gateway_today_counts(app: tauri::AppHandle) -> Result<GatewayTodayCounts, String> {
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

/// 本机的局域网地址（画板 V5-4「地址」行）：按路由表选出的出口 IPv4。UDP connect 不发任何包，
/// 只让系统选一个出口地址；拿不到（没有网络）返回 None。
#[tauri::command]
pub(crate) fn gateway_lan_address() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then(|| ip.to_string())
}

/// 模型路由概览（画板 V5-3 四项指标、Agent 页「今日路由」）：按请求而不是按流水行计。
#[tauri::command]
pub(crate) fn gateway_day_overview(app: tauri::AppHandle) -> Result<crate::gateway::route_log::DayOverview, String> {
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

/// 锁定 ZCode 连接的订阅通道（账号卡「订阅通道」切换）。
/// `channel`：None/"auto" = 自动双通道失败转移；"start-plan" / "coding-plan" = 只走指定通道。
/// 锁定会清掉自动通道的记忆，下一次请求立即按新设置走。
#[tauri::command]
pub(crate) fn zcode_set_channel(
    app: tauri::AppHandle,
    connection_id: String,
    channel: Option<String>,
) -> Result<(), String> {
    {
        let state = app.state::<Db>();
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        connections::set_start_channel(&conn, &connection_id, channel.as_deref())?;
    }
    crate::gateway::adapter::zcode::clear_channel_memory(&connection_id);
    Ok(())
}

/// 今日（本地零点起）经网关的请求数：(ok, rejected, failed)。
#[derive(serde::Serialize)]
pub(crate) struct GatewayTodayCounts { pub(crate) since: i64, pub(crate) at: i64, pub(crate) ok: i64, pub(crate) rejected: i64, pub(crate) failed: i64 }
