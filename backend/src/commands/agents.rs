//! Agent：本机探测、版本、接线与自定义工具。

#[allow(unused_imports)]
use crate::*;

/// 各 agent CLI 的版本与建议更新命令（只读：不自动执行升级）。
/// 版本读不到、命令不在 PATH、安装方式认不出来都如实分开表达，不猜。
#[tauri::command]
pub(crate) async fn agent_versions() -> Result<Vec<agent_version::AgentVersion>, String> {
    tauri::async_runtime::spawn_blocking(agent_version::detect)
        .await
        .map_err(|error| format!("检测 Agent 版本失败：{error}"))
}

/// Agent 自动发现（A1/A2，只读）：探测本机装了哪些 agent。
/// 状态 installed / cli_missing / not_found 与命中的证据都如实返回，不猜。
/// 命令查找按目录戳缓存；`force`（「重新检测」）先丢掉缓存再找。
#[tauri::command]
pub(crate) async fn agent_discover(force: Option<bool>) -> Result<Vec<agent_discovery::AgentDiscovery>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if force.unwrap_or(false) {
            agent_discovery::forget_bin_lookups();
        }
        agent_discovery::discover()
    })
        .await
        .map_err(|error| format!("探测本机 Agent 失败：{error}"))
}

/// 探测 WSL 与其中各 agent 的安装情况（只读，不写 WSL 内文件）。
#[tauri::command]
pub(crate) async fn wsl_probe_agents() -> Result<wsl_probe::WslProbe, String> {
    tauri::async_runtime::spawn_blocking(wsl_probe::probe)
        .await
        .map_err(|error| format!("探测 WSL 失败：{error}"))
}

/// 5.10 成本调优建议（只读，不改任何工具配置；无画板，待确认）
#[tauri::command]
pub(crate) async fn tune_advice(app: tauri::AppHandle, days: i64) -> Result<Vec<crate::tune::AgentAdvice>, String> {
    let db = app.state::<Db>().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = db.lock().map_err(|_| "数据库不可用".to_string())?;
        crate::service::tune::advise(&conn, days, chrono::Utc::now().timestamp_millis())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 5.6 WSL 接线现状：发行版的网络模式与已接的工具（无画板，待确认）
#[derive(serde::Serialize)]
pub(crate) struct WslWiringState {
    net: Option<crate::gateway::wiring_wsl::DistroNet>,
    /// 读不到网络情况的原因
    net_error: Option<String>,
    rows: Vec<crate::gateway::wiring_wsl::WslWiringRow>,
    agents: Vec<String>,
    lan_shared: bool,
}

#[tauri::command]
pub(crate) async fn wsl_wiring_state(app: tauri::AppHandle, distro: String) -> Result<WslWiringState, String> {
    let probe_distro = distro.clone();
    let net = tauri::async_runtime::spawn_blocking(move || crate::gateway::wiring_wsl::probe_net(&probe_distro)).await.map_err(|e| e.to_string())?;
    let rows = {
        let state = app.state::<Db>();
        let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
        crate::gateway::wiring_wsl::list(&conn)
    };
    let (net, net_error) = match net { Ok(net) => (Some(net), None), Err(error) => (None, Some(error)) };
    Ok(WslWiringState {
        net,
        net_error,
        rows: rows.into_iter().filter(|row| row.distro == distro).collect(),
        agents: crate::gateway::wiring_wsl::WSL_AGENTS.iter().map(|a| a.to_string()).collect(),
        lan_shared: app.state::<Cfg>().0.get().gateway_shared_lan,
    })
}

/// 把 WSL 发行版里的一个工具接到模型路由。NAT 网络下只在用户已开局域网共享时接，并签一把专用客户端 Key。
#[tauri::command]
pub(crate) async fn wsl_wire(app: tauri::AppHandle, distro: String, agent: String) -> Result<String, String> {
    use crate::gateway::wiring_wsl::{self, Route};
    let rel = wiring_wsl::relative_config(&agent).ok_or_else(|| format!("WSL 里暂不支持接入 {agent}"))?;
    let settings = app.state::<Cfg>().0.get();
    let (mode, _) = crate::gateway::mode_from(&settings);
    if !settings.proxy_enabled || mode != crate::gateway::Mode::Relay {
        return Err("模型路由没有运行：请先开启模型路由，再接入工具".into());
    }
    let probe_distro = distro.clone();
    let net = tauri::async_runtime::spawn_blocking(move || wiring_wsl::probe_net(&probe_distro)).await.map_err(|e| e.to_string())??;
    let route = wiring_wsl::route_for(&net, &agent, settings.proxy_port, settings.gateway_shared_lan)?;
    let path = wiring_wsl::unc(&distro, &format!("{}/{rel}", net.home.trim_end_matches('/')));
    let dir_exists = path.parent().is_some_and(|parent| crate::steady::exists(parent));
    if !dir_exists {
        return Err(format!("WSL「{distro}」里没找到 {agent} 的配置目录（{}）：先在 WSL 里装好并运行一次", path.parent().map(|p| p.display().to_string()).unwrap_or_default()));
    }
    let backup = custom_backup_dir(&app)?;
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let (base, token, key_name) = match &route {
        Route::Loopback { base } => (base.clone(), format!("jolu-{agent}.{}", settings.instance_id.chars().take(8).collect::<String>()), None),
        Route::Host { base, key_name } => {
            // 重接时换新 Key：旧 Key 原文只在签发时出现过一次，取不回来
            let _ = crate::gateway::access::client_key_revoke(&conn, key_name);
            let issued = crate::gateway::access::client_key_issue(&conn, key_name, None, None, None, None, false)?;
            (base.clone(), issued.key, Some(key_name.clone()))
        }
    };
    let result = crate::cli_switch::with_switch(|| {
        wiring_wsl::wire_at(&conn, &distro, &agent, &path, &base, &token, key_name.as_deref(), &backup, chrono::Utc::now().timestamp_millis())
    });
    if let Err(error) = result {
        if let Some(name) = key_name.as_deref() {
            let _ = crate::gateway::access::client_key_revoke(&conn, name);
        }
        return Err(error);
    }
    drop(conn);
    let _ = app.emit(crate::events::GATEWAY_WIRING_CHANGED, ());
    Ok(match route {
        Route::Loopback { .. } => format!("已接入 WSL「{distro}」里的 {agent}（经 127.0.0.1）"),
        Route::Host { key_name, .. } => format!("已接入 WSL「{distro}」里的 {agent}（经 {base}，专用 Key「{key_name}」；Windows 防火墙需放行 WSL）"),
    })
}

#[tauri::command]
pub(crate) fn wsl_unwire(app: tauri::AppHandle, distro: String, agent: String) -> Result<String, String> {
    let backup = custom_backup_dir(&app)?;
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let (changed, key) = crate::cli_switch::with_switch(|| crate::gateway::wiring_wsl::unwire(&conn, &distro, &agent, &backup))?;
    if let Some(key) = key {
        let _ = crate::gateway::access::client_key_revoke(&conn, &key);
    }
    drop(conn);
    let _ = app.emit(crate::events::GATEWAY_WIRING_CHANGED, ());
    Ok(if changed { format!("已断开并还原 WSL「{distro}」里的 {agent}") } else { format!("WSL「{distro}」里的 {agent} 已不是 Jolu 写的地址，只清掉了接线记录") })
}

/// 接线前的提醒（只读）：比如 Claude Code 同时设了顶层 model 与 env.ANTHROPIC_MODEL。
#[tauri::command]
pub(crate) fn gateway_wiring_preflight(agent: String) -> Vec<String> {
    crate::gateway::wiring::preflight_notices(&agent)
}

/// 接线改动预览（F6，画板 V5-1 / V5-2）：已接入时给「Jolu 改了什么」，接入前给「将要写成什么」。
/// 只读，不写文件；用户配置里的密钥在后端遮住。
#[tauri::command]
pub(crate) async fn gateway_wiring_preview(
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
pub(crate) fn gateway_long_context_set(app: tauri::AppHandle, value: String) -> Result<String, String> {
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
            let _ = app.emit(crate::events::GATEWAY_WIRING_CHANGED, ());
            Ok("已保存，并已更新 Claude Code 的配置".into())
        }
        _ => Ok("已保存；下次接入 Claude Code 时生效".into()),
    }
}

/// 接线记录里实际写入的模型配置（Claude Code 的主模型、子代理、档位），给接线弹窗回填。
#[derive(serde::Serialize)]
pub(crate) struct WiringAppliedView {
    model: Option<String>,
    subagent_model: Option<String>,
    tiers: Option<crate::gateway::wiring_store::TierModels>,
}

#[tauri::command]
pub(crate) fn gateway_wiring_applied(app: tauri::AppHandle, agent: String) -> Result<Option<WiringAppliedView>, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
    Ok(crate::gateway::wiring_store::load(&conn, &agent)
        .filter(|record| record.state == "wired")
        .and_then(|record| record.applied())
        .map(|applied| WiringAppliedView { model: applied.model, subagent_model: applied.subagent_model, tiers: applied.tiers }))
}

#[tauri::command]
pub(crate) fn gateway_wiring_set(
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
    let _ = app.emit(crate::events::GATEWAY_WIRING_CHANGED, ());
    result
}

pub(crate) fn gateway_wiring_set_inner(
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
pub(crate) fn gateway_escape(app: tauri::AppHandle) -> String {
    let summary = crate::gateway::escape::manual(&app).summary();
    let _ = app.emit(crate::events::GATEWAY_WIRING_CHANGED, ());
    summary
}

#[tauri::command]
pub(crate) fn custom_agents_list(app: tauri::AppHandle) -> Result<Vec<crate::custom_agents::CustomAgentView>, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    Ok(crate::custom_agents::list(&conn).map_err(|e| e.to_string())?.iter().map(crate::custom_agents::view).collect())
}

#[tauri::command]
pub(crate) fn custom_agents_add(app: tauri::AppHandle, input: crate::custom_agents::CustomAgentInput) -> Result<crate::custom_agents::CustomAgentView, String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let agent = crate::custom_agents::add(&conn, &input, chrono::Local::now().timestamp_millis())?;
    Ok(crate::custom_agents::view(&agent))
}

#[tauri::command]
pub(crate) fn custom_agents_remove(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::custom_agents::remove(&conn, &id)
}

#[tauri::command]
pub(crate) fn custom_agents_wire(app: tauri::AppHandle, id: String, model: Option<String>) -> Result<crate::custom_agents::CustomAgentView, String> {
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
    let _ = app.emit(crate::events::GATEWAY_WIRING_CHANGED, ());
    Ok(view)
}

#[tauri::command]
pub(crate) fn custom_agents_unwire(app: tauri::AppHandle, id: String) -> Result<bool, String> {
    let backup = custom_backup_dir(&app)?;
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let changed = crate::custom_agents::unwire(&conn, &id, &backup)?;
    drop(conn);
    let _ = app.emit(crate::events::GATEWAY_WIRING_CHANGED, ());
    Ok(changed)
}

#[tauri::command]
pub(crate) fn custom_agents_preview(app: tauri::AppHandle, id: String, model: Option<String>) -> Result<crate::gateway::escape::WiringPreview, String> {
    let settings = app.state::<Cfg>().0.get();
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::custom_agents::preview(&conn, &id, settings.proxy_port, &settings.instance_id, model.as_deref())
}

/// 接受接线漂移：只清接线记录，不改用户的配置文件。
#[tauri::command]
pub(crate) fn gateway_wiring_accept(app: tauri::AppHandle, agent: String) -> Result<(), String> {
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    crate::gateway::wiring_store::mark_idle(&conn, &agent).map_err(|error| error.to_string())?;
    drop(conn);
    let _ = app.state::<Cfg>().0.try_update(|settings| {
        settings.gateway_wiring.remove(&agent);
    });
    let _ = app.emit(crate::events::GATEWAY_WIRING_CHANGED, ());
    Ok(())
}

pub(crate) fn custom_backup_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path().app_data_dir().map(|dir| dir.join("gateway-backups")).map_err(|e| format!("无法定位应用数据目录：{e}"))
}

/// 打开本机的 ZCode 客户端（签到页「去 ZCode 领取」用）。
///
/// 为什么需要：ZCode 的礼包领取接口要求阿里云人机验证（`code 3007`，2026-11-19 实测），
/// 验证 token 只能在浏览器环境里生成，Jolu 不代领。所以签到卡片给一个直达客户端的入口，
/// 让用户在官方界面里完成领取。
///
/// 只尝试 ZCode 已安装的固定位置（与探测 `dir_zcode` 同源），不猜、不搜 PATH：
/// 找不到就如实报「未安装」，用户自己去开始菜单开。
#[tauri::command]
pub(crate) fn open_zcode_client() -> Result<(), String> {
    let home = crate::creds::home().ok_or("未找到用户目录")?;
    let candidates = [
        // 官方安装器：%LOCALAPPDATA%\Programs\ZCode\ZCode.exe
        std::env::var_os("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .map(|local| local.join("Programs").join("ZCode").join("ZCode.exe")),
        // 便携版 / 绿色版：<ZCode 数据目录同级>
        Some(crate::creds::zcode_data_dir(&home).parent().map(|parent| parent.join("ZCode.exe")).unwrap_or_default()),
    ];
    for candidate in candidates.into_iter().flatten() {
        if candidate.is_file() {
            std::process::Command::new(&candidate)
                .spawn()
                .map_err(|error| format!("启动 ZCode 失败：{error}"))?;
            return Ok(());
        }
    }
    Err("未找到 ZCode 客户端，请在开始菜单中打开".into())
}
