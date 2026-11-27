//! 供应商连接：增删改、凭据、模型清单与本机导入。

use tauri::{Emitter, Manager, State};

use crate::app::state::{
    data_paths, invalidate_network_cache, with_database, with_reader, Cfg, Db, DbRead,
};
use crate::{
    auth_snapshot, claude_auth, cli_apply, connections, creds, current_connection, island_clones,
    login_detect, native_sources, platforms, provider_browser_auth, provider_key, quota,
    refresh_tray_menu, update_tray_icon, validation,
};

/// 本机导入扫描：CC Switch / Claude Code / Codex 的现有上游读成草稿。
/// 键不出后端——草稿里只有掩码，落库由 `providers_import_apply` 重读原文。
#[tauri::command]
pub(crate) fn providers_import_scan() -> Result<serde_json::Value, String> {
    crate::gateway::import_sources::scan()
}

/// 按引用落库导入的连接（走既有 `connections::add`：加密 / slug / 掩码全在正路里）。
#[tauri::command]
pub(crate) async fn providers_import_apply(
    app: tauri::AppHandle,
    db: tauri::State<'_, Db>,
    references: Vec<String>,
) -> Result<Vec<String>, String> {
    let pool = db.0.clone();
    let created = with_database(pool, move |conn| {
        crate::gateway::import_sources::apply(conn, &references)
    })
    .await?;
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    Ok(created)
}

#[tauri::command]
pub(crate) async fn list_connections(db: State<'_, DbRead>) -> Result<Vec<connections::ConnectionDto>, String> {
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
pub(crate) async fn gateway_refresh_models(app: tauri::AppHandle, connection_id: String) -> Result<Vec<String>, String> {
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
    crate::events::catalog_changed(&app);
    Ok(fetched)
}

/// 「获取模型」（画布 18）：用用户填写的地址与 Key 拉取网关模型列表。
/// Key 只在本命令内使用，不落库、不打印。
#[tauri::command]
pub(crate) async fn fetch_remote_models(platform: String, base_url: String, secret: String, protocol: Option<String>) -> Result<Vec<String>, String> {
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
pub(crate) async fn verify_custom_model(base_url: String, secret: String, protocol: String, model: String) -> Result<(), String> {
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
#[tauri::command]
pub(crate) async fn enable_connection(app: tauri::AppHandle, db: State<'_, Db>, id: String) -> Result<String, String> {
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
        let _=app.emit(crate::events::CONNECTIONS_CHANGED,());
        return Ok("官方 Key 检测通过；仅保存连接，不修改外部应用登录或授予余额权限".into());
    }
    if matches!(info.platform.as_str(),"zcode"|"opencode") && info.kind=="api" {
        let secret=info.secret.clone().ok_or("该连接没有保存 API Key")?;
        tauri::async_runtime::spawn_blocking(move || quota::validate_connection(&info.platform,"api",&secret,info.base_url.as_deref(),info.model.as_deref())).await.map_err(|e|e.to_string())??;
        let key=id.clone();with_database(db.0.clone(),move|conn|connections::set_paused(conn,&key,false)).await?;
        invalidate_network_cache(&id);let _=app.emit(crate::events::CONNECTIONS_CHANGED,());crate::events::refresh_requested(&app, Some(id.as_str()));
        return Ok("已恢复连接查询，原工具配置保持原样".into());
    }
    if info.kind=="auth" && (platforms::native(&info.platform)||provider_browser_auth::is_browser(info.secret.as_deref())) {
        validate_restored_connection(&id,&info).await?;
        let key=id.clone();
        with_database(db.0.clone(),move |conn|connections::set_paused(conn,&key,false)).await?;
        invalidate_network_cache(&id);
        let _=app.emit(crate::events::CONNECTIONS_CHANGED,());
        crate::events::refresh_requested(&app, Some(id.as_str()));
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
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    Ok(note.message)
}

/// 统一编辑连接（画布 18）：完整快照式更新。
/// auth 连接只接受名称（凭证由 CLI 管理）；api 连接可改网关地址、Key 与默认参数。
/// secret 为空表示不更换 Key；地址或 Key 变化时先用组合后的凭证做一次网络验证，
/// 验证通过才落库，避免把不可用的配置存进去。
#[tauri::command]
pub(crate) async fn update_connection(
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
            crate::events::refresh_requested(&app, Some(id.as_str()));
        }
    }
    // 名称若是任一灵动岛的当前连接，同步快照（与 rename_connection 命令同一套逻辑）
    let mut renamed = false;
    cfg.0.try_update(|s| { renamed = island_clones::rename_connection(s, &id, &trimmed_name); })?;
    if renamed {
        crate::events::settings_changed(&app);
        refresh_tray_menu(&app);
    }
    invalidate_network_cache(&id);
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    Ok(())
}

#[tauri::command]
pub(crate) async fn add_connection(app: tauri::AppHandle, db: State<'_, Db>, input: connections::NewConnection) -> Result<String, String> {
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
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    Ok(id)
}

/// 移除连接：只删凭证与配置，历史统计保留（§6.3）
#[tauri::command]
pub(crate) async fn remove_connection(app: tauri::AppHandle, db: State<'_, Db>, id: String) -> Result<(), String> {
    let remove_id = id.clone();
    with_database(db.0.clone(), move |conn| {
        connections::remove(conn, &remove_id).map_err(|e| e.to_string())
    })
    .await?;
    invalidate_network_cache(&id);
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    Ok(())
}

#[tauri::command]
pub(crate) async fn set_connection_paused(app: tauri::AppHandle, db: State<'_, Db>, id: String, paused: bool) -> Result<(), String> {
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
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    // 重新连接后立刻取数，否则界面要停在「等待连接数据更新」直到下一轮轮询。
    // 断开则不必：此时界面本就该显示已断开，多发一次查询只是白跑。
    if !paused {
        crate::events::refresh_requested(&app, Some(id.as_str()));
    }
    Ok(())
}

/// 「获取」：重读本机凭证刷新已有连接。失败不破坏现有连接（§6.3）
#[tauri::command]
pub(crate) async fn fetch_credentials(app: tauri::AppHandle, db: State<'_, Db>, id: String) -> Result<connections::FetchResult, String> {
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
        let _=app.emit(crate::events::CONNECTIONS_CHANGED,());
        crate::events::refresh_requested(&app, Some(id.as_str()));
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
        let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
        // 缓存刚被清空，无论凭证是否变化都立即重查。用户按「更新」的意图是
        // 「我要最新数据」：凭证没变时同样该看到刷新，而不是毫无动静——
        // 只在 changed 时通知，会让「本机凭证未变化」这条最常见的路径彻底没反馈。
        // 携带目标连接 ID：只有这条连接被灵动岛/总览选中时才重查，避免更新一行广播刷新另一条。
        crate::events::refresh_requested(&app, Some(id.as_str()));
    }
    Ok(result)
}

#[tauri::command]
pub(crate) async fn replace_api_key(
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
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    // 换了 Key 必然要用新凭证重查，否则界面会一直停在旧 Key 的结果上
    // 更换 API Key 同样只刷新目标连接；全局刷新入口仍发送空 payload。
    crate::events::refresh_requested(&app, Some(id.as_str()));
    Ok(())
}

/// 本机可自动发现的连接候选，供「+ 添加连接」预填
/// 账号管理「当前使用」：本机 CLI 正在用的凭证对应哪条连接（每个平台至多一条）。
/// 只比对，不返回凭证；API Key 优先于订阅，与 CLI 同时配置两者时的生效顺序一致。
#[tauri::command]
pub(crate) async fn current_connections(db: State<'_, Db>) -> Result<Vec<String>, String> {
    if validation::isolated() { return Ok(vec![]); }
    let customs = with_database(db.0.clone(), |conn| {
        Ok(crate::custom_agents::list(conn).unwrap_or_default())
    })
    .await?;
    let local = tauri::async_runtime::spawn_blocking(move || current_connection::read_all_local(&customs))
        .await.map_err(|error| error.to_string())?;
    with_database(db.0.clone(), move |conn| {
        let mut ids: Vec<String> = Vec::new();
        for item in &local {
            if let Some((id, _, _)) = crate::local_active::find_matching_saved_connection(conn, item) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        Ok(ids)
    }).await
}

/// 「检测本机登录」：只读检测，不入库；保存仍走 read_local_connections。
#[tauri::command]
pub(crate) async fn detect_local_login(platform: String, edition: Option<String>) -> Result<login_detect::LoginDetection, String> {
    if validation::isolated() { return Err("隔离验收环境禁止读取真实凭证".into()); }
    tauri::async_runtime::spawn_blocking(move || login_detect::detect_for_edition(&platform,edition.as_deref())).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn discover_connections() -> Vec<creds::Candidate> {
    if validation::isolated() { return vec![]; }
    creds::discover()
}

/// 重命名连接：只改显示名称。若改的正是灵动岛当前连接，
/// 需同步刷新设置里的名称快照（托盘/灵动岛显示用的是快照，不实时查库）。
#[tauri::command]
pub(crate) async fn rename_connection(
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
    cfg.0.try_update(|s| { renamed = island_clones::rename_connection(s, &id, &name); })?;
    if renamed {
        crate::events::settings_changed(&app);
        refresh_tray_menu(&app);
    }
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    Ok(())
}

#[tauri::command]
pub(crate) async fn read_local_connections(app: tauri::AppHandle, db: State<'_, Db>, platform: String, kind: String, name: Option<String>, edition: Option<String>) -> Result<LocalReadSummary, String> {
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
        let _=app.emit(crate::events::CONNECTIONS_CHANGED,());
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
    let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ());
    Ok(result)
}

/// 恢复连接前先验证它保存的凭证还能用（浏览器授权、本机原生登录、API Key 三种各走各的校验）。
pub(crate) async fn validate_restored_connection(id:&str,current:&connections::Creds) -> Result<(),String> {
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

#[derive(serde::Serialize)]
pub(crate) struct LocalReadSummary { pub(crate) added: usize, pub(crate) existing: usize, pub(crate) warnings: Vec<String> }
