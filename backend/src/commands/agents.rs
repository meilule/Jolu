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
            let _ = app.emit("gateway-wiring-changed", ());
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
    let _ = app.emit("gateway-wiring-changed", ());
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
    let _ = app.emit("gateway-wiring-changed", ());
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
    let _ = app.emit("gateway-wiring-changed", ());
    Ok(view)
}

#[tauri::command]
pub(crate) fn custom_agents_unwire(app: tauri::AppHandle, id: String) -> Result<bool, String> {
    let backup = custom_backup_dir(&app)?;
    let state = app.state::<Db>();
    let conn = state.0.lock().map_err(|_| "数据库不可用".to_string())?;
    let changed = crate::custom_agents::unwire(&conn, &id, &backup)?;
    drop(conn);
    let _ = app.emit("gateway-wiring-changed", ());
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
    let _ = app.emit("gateway-wiring-changed", ());
    Ok(())
}

pub(crate) fn custom_backup_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path().app_data_dir().map(|dir| dir.join("gateway-backups")).map_err(|e| format!("无法定位应用数据目录：{e}"))
}
