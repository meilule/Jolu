//! 额度、余额与 API 用量查询。

#[allow(unused_imports)]
use crate::*;

#[tauri::command]
pub(crate) async fn local_codex_quota(app: tauri::AppHandle, force: bool, max_age_ms: Option<u64>) -> Result<quota::QuotaState, String> {
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
pub(crate) async fn local_grok_quota(force: bool, max_age_ms: Option<u64>) -> Result<quota::QuotaState, String> {
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
pub(crate) async fn local_zcode_quota(force: bool, max_age_ms: Option<u64>) -> Result<quota::QuotaState, String> {
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
pub(crate) const ZCODE_CONNECTION_CACHE_KEY: &str = "zcode-connection:local-login";

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
pub(crate) fn cached_zcode_connection_quota(
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
pub(crate) async fn connection_quota(app: tauri::AppHandle, db: State<'_, Db>, id: String, force: bool, max_age_ms: Option<u64>) -> Result<quota::QuotaState, String> {
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
pub(crate) fn with_observation(id: &str, state: quota::QuotaState) -> quota::QuotaState {
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
pub(crate) async fn finish_quota_query(
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
    if changed { let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ()); }
    // 额度快照（路由分组 smart / pace / cap hold 的数据源）：按查询结果处置。
    //
    // 以前只在「能解析出重置时间」时写，于是两个坑：
    // ① 只给剩余量不给重置时间的来源，数据被整个丢掉；
    // ② 查询变成结构性失败（授权失效 / 平台不支持）时旧快照继续生效，
    //    已失效的账号还按「已用满 + 旧重置时间」参与选路。
    // 现在按 `SnapshotAction` 的映射处理：有事实就写、结构性结论就清、临时失败保留
    // （新鲜度会兜住），`Observed`（网关限流头推断）不动快照。
    {
        use crate::gateway::quota_snapshot::SnapshotAction;
        let action = match &state {
            quota::QuotaState::Ok { windows, .. } => {
                crate::gateway::quota_snapshot::action_for_ok(
                    windows,
                    chrono::Utc::now().timestamp_millis(),
                )
            }
            // 结构性结论：这个连接不再有额度可查，旧快照不再代表现实
            quota::QuotaState::Unauthorized { .. }
            | quota::QuotaState::Unsupported { .. }
            | quota::QuotaState::Forbidden { .. } => SnapshotAction::Clear,
            // 临时失败与网关限流头推断：不动快照，交给新鲜度判断
            quota::QuotaState::Failed { .. }
            | quota::QuotaState::RateLimited { .. }
            | quota::QuotaState::Observed { .. } => SnapshotAction::Keep,
        };
        match action {
            SnapshotAction::Write { resets_at_ms, remaining_percent, exhausted } => {
                crate::gateway::quota_snapshot::record(&id, resets_at_ms, remaining_percent, exhausted);
            }
            SnapshotAction::Clear => crate::gateway::quota_snapshot::clear(&id),
            SnapshotAction::Keep => {}
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

pub(crate) fn validate_query_range(custom: Option<db::CustomRange>, query_end_ms: i64) -> Result<(), String> {
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
pub(crate) async fn connection_api_usage(app: tauri::AppHandle, db: State<'_, Db>, id: String, force: bool, max_age_ms: Option<u64>) -> Result<quota::ApiUsageState, String> {
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
    if changed { let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ()); }
    Ok(state)
}

/// 查余额。目前只有 sub2api 网关提供钱包余额；
/// 官方订阅没有「余额」概念，如实返回不支持。
#[tauri::command]
pub(crate) async fn connection_balance(app: tauri::AppHandle, db: State<'_, Db>, id: String, force: bool, max_age_ms: Option<u64>) -> Result<quota::BalanceState, String> {
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
    if changed { let _ = app.emit(crate::events::CONNECTIONS_CHANGED, ()); }
    // 仅仍属于当前凭证的余额可以触发提醒。
    if let quota::BalanceState::Ok { balance, currency, .. } = &state { notify::check_balance(&app, &id, &c.name, &c.platform, *balance, currency); }
    Ok(state)
}
