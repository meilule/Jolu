//! 能力汇聚：隐私脱敏、联网搜索与 Library。

use tauri::{Emitter, Manager};

use crate::app::state::{Cfg, Db};
use crate::{library, settings};

/// 隐私脱敏开关（P5，gateway/mask.rs）：只影响网关中转模式的请求 / 响应。
/// 进程级开关改完立即生效，不需要重启代理；前端接入口 = `settings.privacy_masking` 字段
/// + 本命令（界面待设计稿确认后再接）。
#[tauri::command]
pub(crate) fn privacy_masking_set(app: tauri::AppHandle, on: bool) -> Result<settings::Settings, String> {
    let next = app
        .state::<Cfg>()
        .0
        .try_update(|settings| settings.privacy_masking = on)
        .map_err(|error| format!("保存隐私脱敏设置失败：{error}"))?;
    crate::gateway::mask::set_enabled(next.privacy_masking);
    crate::events::settings_changed(&app);
    Ok(next)
}

/// 隐私脱敏的处理方式与豁免清单（3.7）：先按网关引擎的规则校验，再保存并立即生效。
#[tauri::command]
pub(crate) fn privacy_policy_set(
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
    crate::events::settings_changed(&app);
    Ok(next)
}

/// 最近的脱敏记录（只在内存，新的在前；不含原值）。
#[tauri::command]
pub(crate) fn privacy_records() -> Vec<crate::gateway::mask::MaskRecord> {
    crate::gateway::mask::records()
}

#[tauri::command]
pub(crate) fn privacy_records_clear() {
    crate::gateway::mask::clear_records();
}

/// 联网搜索：可选的后端清单（界面下拉用）。
#[tauri::command]
pub(crate) fn search_vendors() -> Vec<crate::gateway::search::VendorInfo> {
    crate::gateway::search::vendor_infos()
}

/// 联网搜索：整体保存后端配置。界面只回传掩码或新填的 Key，掩码在这里换回原文
/// （`gateway::search::merge_inputs`），返回的设置里 Key 仍是掩码。
#[tauri::command]
pub(crate) fn search_apis_set(app: tauri::AppHandle, apis: Vec<crate::gateway::search::SearchApiInput>) -> Result<settings::Settings, String> {
    let cfg = app.state::<Cfg>();
    let merged = crate::gateway::search::merge_inputs(&cfg.0.get().search_apis, apis)?;
    let next = cfg
        .0
        .try_update(|settings| settings.search_apis = merged)
        .map_err(|error| format!("保存搜索后端失败：{error}"))?;
    crate::events::settings_changed(&app);
    Ok(next)
}

/// 联网搜索：用已保存的配置试搜一次（依次尝试启用的后端），给界面核对 Key 是否可用。
#[tauri::command]
pub(crate) async fn search_try(app: tauri::AppHandle, query: String) -> Result<Vec<crate::gateway::search::SearchHit>, String> {
    let configs = app.state::<Cfg>().0.get().search_apis;
    crate::gateway::search::search_in_order(&configs, query.trim(), 5).await
}

/// 接入 / 断开单个工具。
///
/// `model` 只对 Claude Code 有意义，是**主模型**：接线时写入 settings.json 顶层 `model`、
/// `ANTHROPIC_SMALL_FAST_MODEL` 与全部档位 env（opus/sonnet/haiku/fable 默认跟随主模型），
/// 并移除 `ANTHROPIC_MODEL`（它压过 /model 的选择）与各档旧的「关于」键；断开时按快照还原。
/// 不传（或传空）时只接地址和令牌，用户的模型配置原样保留。
/// Library（批 5）：把「Jolu 搜索」以 MCP server 装进指定 agent。
#[tauri::command]
pub(crate) fn library_install(app: tauri::AppHandle, agent: String) -> Result<library::InstallOutcome, String> {
    // 桌面端场景：current_exe 就是主程序 jolu.exe（自带 --mcp-search 分支）
    let item = library::search_item_with(&std::env::current_exe().map_err(|error| error.to_string())?);
    let outcome = crate::cli_switch::with_switch(|| {
        backup_library_config(&app, &agent)?;
        let db = app.state::<Db>();
        let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
        library::install(&conn, &agent, &item).map_err(|error| error.to_string())
    })
    .map_err(|error| format!("另一实例正在修改工具配置：{error}"))?;
    let _ = app.emit(crate::events::LIBRARY_CHANGED, ());
    Ok(outcome)
}

/// 改 agent 的 MCP 配置前先备份到 `gateway-backups/`（与接线同一目录、同一纪律：备份失败就不写）。
/// 文件还不存在时无需备份。
fn backup_library_config(app: &tauri::AppHandle, agent: &str) -> Result<(), String> {
    let Some(path) = library::mcp_config_path(agent) else { return Ok(()) };
    let dir = crate::commands::agents::custom_backup_dir(app)?;
    crate::gateway::wiring::backup(&path, &dir)
}

/// Library：卸载指定 agent 里的一个库项（只删 Jolu 自己装的）。
#[tauri::command]
pub(crate) fn library_remove(app: tauri::AppHandle, agent: String, name: String) -> Result<bool, String> {
    let removed = crate::cli_switch::with_switch(|| {
        backup_library_config(&app, &agent)?;
        let db = app.state::<Db>();
        let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
        library::remove(&conn, &agent, &name).map_err(|error| error.to_string())
    })
    .map_err(|error| format!("另一实例正在修改工具配置：{error}"))?;
    let _ = app.emit(crate::events::LIBRARY_CHANGED, ());
    Ok(removed)
}

/// Library 5.1：共享指令与子代理的定义和各 agent 的写入现状（无画板，待确认）。
#[derive(serde::Serialize)]
pub(crate) struct LibrarySharedState {
    instructions: library::shared::InstructionsDef,
    instruction_targets: Vec<library::shared::TargetStatus>,
    subagents: Vec<LibrarySubagentView>,
    subagent_agents: Vec<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct LibrarySubagentView {
    #[serde(flatten)]
    def: library::shared::SubagentDef,
    targets: Vec<library::shared::TargetStatus>,
}

fn shared_state(conn: &rusqlite::Connection) -> LibrarySharedState {
    use library::shared;
    LibrarySharedState {
        instructions: shared::load_instructions(conn),
        instruction_targets: shared::INSTRUCTION_AGENTS.iter().map(|agent| shared::instruction_status(conn, agent, shared::instruction_path(agent).as_deref())).collect(),
        subagents: shared::list_subagents(conn)
            .into_iter()
            .map(|def| LibrarySubagentView { targets: shared::SUBAGENT_AGENTS.iter().map(|agent| shared::subagent_status(conn, &def.name, agent)).collect(), def })
            .collect(),
        subagent_agents: shared::SUBAGENT_AGENTS.iter().map(|a| a.to_string()).collect(),
    }
}

#[tauri::command]
pub(crate) fn library_shared_state(app: tauri::AppHandle) -> Result<LibrarySharedState, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
    Ok(shared_state(&conn))
}

/// 保存共享指令并同步到勾选的工具（没勾选但写过的移除）；返回每处的结果。
#[tauri::command]
pub(crate) fn library_instructions_save(app: tauri::AppHandle, text: String, agents: Vec<String>) -> Result<Vec<library::shared::FileOutcome>, String> {
    let def = library::shared::InstructionsDef { text, agents };
    let out = crate::cli_switch::with_switch(|| {
        let db = app.state::<Db>();
        let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
        library::shared::save_instructions(&conn, &def, chrono::Utc::now().timestamp_millis(), &library::shared::instruction_path)
    })
    .map_err(|error| format!("另一实例正在修改工具配置：{error}"))?;
    let _ = app.emit(crate::events::LIBRARY_CHANGED, ());
    Ok(out)
}

#[tauri::command]
pub(crate) fn library_subagent_save(app: tauri::AppHandle, def: library::shared::SubagentDef) -> Result<Vec<library::shared::FileOutcome>, String> {
    let out = crate::cli_switch::with_switch(|| {
        let db = app.state::<Db>();
        let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
        library::shared::save_subagent(&conn, &def, chrono::Utc::now().timestamp_millis(), &library::shared::subagent_path)
    })
    .map_err(|error| format!("另一实例正在修改工具配置：{error}"))?;
    let _ = app.emit(crate::events::LIBRARY_CHANGED, ());
    Ok(out)
}

#[tauri::command]
pub(crate) fn library_subagent_remove(app: tauri::AppHandle, name: String) -> Result<Vec<library::shared::FileOutcome>, String> {
    let out = crate::cli_switch::with_switch(|| {
        let db = app.state::<Db>();
        let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
        Ok::<_, String>(library::shared::remove_subagent(&conn, &name, chrono::Utc::now().timestamp_millis(), &library::shared::subagent_path))
    })
    .map_err(|error| format!("另一实例正在修改工具配置：{error}"))?;
    let _ = app.emit(crate::events::LIBRARY_CHANGED, ());
    Ok(out)
}

/// Library：每个 agent 的安装状态（装了什么 + 配置里现状），供 UI 列表用。
#[derive(serde::Serialize)]
pub(crate) struct LibraryAgentStatus {
    agent: String,
    installed: Vec<String>,
    /// 配置文件可定位且存在；false = 从未初始化（装了会新建文件）
    config_present: bool,
    /// 能不能装；不能时 `note` 写原因
    supported: bool,
    /// 不支持的原因，或装好后用户还要做的事
    note: Option<String>,
}

/// MCP 导入预览（MCP 配置管理第 1 期，只读）：列出各 agent 用户级配置里已有的条目、归属与同名冲突。
/// 不改任何文件；环境变量与请求头只给名字。界面无画板，待确认。
#[tauri::command]
pub(crate) fn library_import_preview(app: tauri::AppHandle) -> Result<library::ImportPreview, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().map_err(|_| "数据库不可用".to_string())?;
    Ok(library::import_preview(&conn))
}

#[tauri::command]
pub(crate) fn library_list(app: tauri::AppHandle) -> Result<Vec<LibraryAgentStatus>, String> {
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
