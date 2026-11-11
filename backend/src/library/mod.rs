//! Library（能力汇聚的分发层，批 5）：把 Jolu 的工具装进各 agent 的配置文件。
//!
//! 管什么：
//! - MCP stdio server 的安装 / 卸载 / 清单——首批只有「Jolu 搜索」（`--mcp-search`）；
//! - 按 **agent 自己的文件与格式**写（[`mcp_config_path`]、[`agent_key`]）：
//!   Claude Code 写 `~/.claude.json` 的 `mcpServers`（它只从这里读用户级 server，
//!   `~/.claude/settings.json` 里的不生效）；Gemini 写 `~/.gemini/settings.json`；
//!   Codex 写 `config.toml` 的 `mcp_servers`（toml_edit 保注释）；OpenCode 写 `mcp`
//!   （command 为数组）；ZCode 写 `~/.zcode/cli/config.json` 的嵌套键 `mcp.servers`；
//!   Qoder CLI 两版写各自 `settings.json` 的 `mcpServers`；WorkBuddy 写 `mcp.json`
//!   （装好后要在 WorkBuddy 里批准一次才会连接）；
//! - 「哪些是 Jolu 写的」落在 `library_applied` 表——卸载只删这张表里的项。
//!
//! 不管什么：
//! - **不覆盖别人写的同名条目**：目标文件里已有同名 server 且不在 applied 表里，
//!   视为用户或第三方配置，明确拒绝——静默覆盖是 magpie 特意避免的事故；
//! - Trae 不支持：它的 MCP 配置位置与格式没有可靠来源（magpie 也未接），按「不猜」原则；
//! - 不做远程（http/sse）server：Jolu 自己的工具全是本地 stdio，远程 MCP 是
//!   mcpauth 的事（后续批次）。
//!
//! 保护机制：每次写入前走 `steady` 读取与原子写（批 2），`--restore-agents`
//! 卸载钩子会把 applied 表里的项全部移除。
//!
//! 参照 magpie `internal/library/mcp.go`（键名分派 `key()`、条目形状 `entry`）与
//! `targets.go`（各 agent 的用户级 MCP 文件，克隆 `0ee424e`）。
//!
//! 历史：2026-11-10 的首版把 Claude Code 写进 `~/.claude/settings.json`、Gemini 写进
//! `~/.gemini/.env`（借用了接线的路径），两处 agent 都不会读到；卸载时会顺带清掉
//! 这两处旧位置里 Jolu 登记过的条目（`legacy_paths`）。

pub mod agents;
pub mod mcp_search;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// 可安装的库项。首批只有本地 stdio MCP server。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LibraryItem {
    McpServer(McpServerItem),
}

/// 一个本地 stdio MCP server：agent 启动它，经 stdio 说 MCP 协议。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerItem {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// 安装结果。
#[derive(Debug, serde::Serialize)]
pub struct InstallOutcome {
    pub agent: String,
    pub name: String,
    /// true = 新装；false = 已装且内容一致（幂等无操作）
    pub created: bool,
}

/// 安装错误：写不进 / 不该写的原因都单独表达。
#[derive(Debug)]
pub enum LibraryError {
    /// 目标文件里已有同名条目，且不是 Jolu 装的——拒绝覆盖
    ForeignEntry { agent: String, name: String },
    /// 该 agent 暂不支持（Trae：格式没有可靠来源）
    UnsupportedAgent(String),
    /// 文件读写 / 解析失败
    Io(String),
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LibraryError::ForeignEntry { agent, name } => write!(
                f,
                "{agent} 的配置里已有同名条目「{name}」，且不是 Jolu 装的；请改名或先手动移除，Jolu 不会覆盖它"
            ),
            LibraryError::UnsupportedAgent(agent) => {
                write!(f, "{}", unsupported_reason(agent).unwrap_or("这个 agent 暂不支持安装 MCP server"))
            }
            LibraryError::Io(reason) => write!(f, "{reason}"),
        }
    }
}

/// 可以装 MCP server 的 agent（列表展示顺序）。`trae` 列出但不支持，见 [`unsupported_reason`]。
pub const LIBRARY_AGENTS: &[&str] = &["claude-code", "codex", "gemini", "opencode", "zcode", "qoder", "qoder-cn", "workbuddy", "trae"];

/// 该 agent 配置里 MCP 段的键名；带点的是嵌套路径（ZCode 的 `mcp.servers`）。
/// 参照 magpie `mcp.go::key()`。
pub(crate) fn agent_key(agent: &str) -> Option<&'static str> {
    match agent {
        "claude-code" | "gemini" | "qoder" | "qoder-cn" | "workbuddy" => Some("mcpServers"),
        "codex" => Some("mcp_servers"),
        "opencode" => Some("mcp"),
        "zcode" => Some("mcp.servers"),
        _ => None,
    }
}

/// 不支持的原因（列表里如实显示）。
pub fn unsupported_reason(agent: &str) -> Option<&'static str> {
    match agent {
        "trae" => Some("Trae 的 MCP 配置位置与格式没有可靠来源，暂不支持"),
        _ if agent_key(agent).is_none() => Some("这个 agent 没有已知的 MCP 配置格式"),
        _ => None,
    }
}

/// 装好后用户还要做的事（列表里提示）。
pub fn install_note(agent: &str) -> Option<&'static str> {
    match agent {
        "workbuddy" => Some("WorkBuddy 只连接批准过的 server：装好后在 WorkBuddy 的 MCP 设置里打开一次"),
        "qoder" | "qoder-cn" => Some("写入的是 Qoder CLI 的配置（qodercli / qoderclicn）"),
        _ => None,
    }
}

/// ZCode 在 Windows 上读的主目录：`HOME` 已设、是绝对路径、且默认主目录就是
/// `USERPROFILE` 时用 `HOME`（与 ZCode 的 Node 运行时一致），否则用用户目录。
/// 参照 magpie `appdir/zcode.go::ZCodeHomeOn`。
fn zcode_home(home: &std::path::Path) -> std::path::PathBuf {
    let from_env = std::env::var_os("HOME").map(std::path::PathBuf::from).filter(|h| h.is_absolute());
    let is_profile = std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)
        .is_some_and(|profile| profile.as_os_str().eq_ignore_ascii_case(home.as_os_str()));
    match from_env {
        Some(h) if cfg!(windows) && is_profile => h,
        _ => home.to_path_buf(),
    }
}

/// 各 agent 的用户级 MCP 配置文件。与接线（`wiring::config_path_for`）是两回事：
/// 接线改的是模型地址，MCP server 各家放在别的文件里。
pub fn mcp_config_path(agent: &str) -> Option<std::path::PathBuf> {
    let home = crate::creds::home()?;
    let env_dir = |name: &str| std::env::var_os(name).map(std::path::PathBuf::from).filter(|p| !p.as_os_str().is_empty());
    Some(match agent {
        "claude-code" => env_dir("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.clone()).join(".claude.json"),
        "codex" => crate::creds::codex_dir()?.join("config.toml"),
        "gemini" => home.join(".gemini").join("settings.json"),
        "opencode" => env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")).join("opencode").join("opencode.json"),
        "zcode" => zcode_home(&home).join(".zcode").join("cli").join("config.json"),
        "qoder" => env_dir("QODER_CONFIG_DIR").unwrap_or_else(|| home.join(".qoder")).join("settings.json"),
        "qoder-cn" => env_dir("QODERCN_CONFIG_DIR").unwrap_or_else(|| home.join(".qoder-cn")).join("settings.json"),
        "workbuddy" => env_dir("WORKBUDDY_CONFIG_DIR").unwrap_or_else(|| home.join(".workbuddy")).join("mcp.json"),
        _ => return None,
    })
}

/// 首版写错的旧位置（见模块头「历史」）。只在卸载时用来清掉 Jolu 登记过的条目。
fn legacy_paths(agent: &str) -> Vec<std::path::PathBuf> {
    let Some(home) = crate::creds::home() else { return Vec::new() };
    match agent {
        "claude-code" => crate::creds::claude_dir().map(|dir| vec![dir.join("settings.json")]).unwrap_or_default(),
        "gemini" => vec![home.join(".gemini").join(".env")],
        _ => Vec::new(),
    }
}

/// 清掉旧位置里 Jolu 的条目（旧版写法都是顶层 `mcpServers`）；旧位置是 `.env`
/// 且只剩空对象时删除整个文件（那是首版凭空建出的 JSON，`.env` 本不该是 JSON）。
/// 读不懂的文件不动。
fn clean_legacy(agent: &str, _key: &str, name: &str) {
    for path in legacy_paths(agent) {
        clean_legacy_at(&path, name);
    }
}

fn clean_legacy_at(path: &std::path::Path, name: &str) {
    let Ok(true) = agents::remove_server(path, "claude-code", "mcpServers", name) else { return };
    if path.file_name().is_some_and(|file| file == ".env") {
        let empty = crate::steady::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|value| value.as_object().is_some_and(|object| object.is_empty()));
        if empty {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// library 表里这个 agent 已装的名字。
pub fn applied_names(conn: &Connection, agent: &str) -> Vec<String> {
    let mut stmt = match conn.prepare("SELECT name FROM library_applied WHERE agent = ?1 ORDER BY installed_ms") {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([agent], |row| row.get::<_, String>(0))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// 这个名字是不是 Jolu 装的（决定同名冲突时允许更新还是拒绝）。
fn is_ours(conn: &Connection, agent: &str, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM library_applied WHERE agent = ?1 AND name = ?2",
        [agent, name],
        |row| row.get::<_, i64>(0).map(|_| ()),
    )
    .is_ok()
}

/// 安装一个库项：定位该 agent 的真实配置路径后写入。
///
/// **测试不要用这个**——它会写真实的 `~/.claude` 等用户文件；测试走 [`install_at`]。
pub fn install(conn: &Connection, agent: &str, item: &LibraryItem) -> Result<InstallOutcome, LibraryError> {
    // 支持性先于路径定位：Trae 这类没有受支持格式的 agent，
    // 报「不支持」而不是「路径不可定位」——前者是事实，后者只是副作用
    if agent_key(agent).is_none() {
        return Err(LibraryError::UnsupportedAgent(agent.to_string()));
    }
    let path = mcp_config_path(agent).ok_or_else(|| {
        LibraryError::Io(format!("{agent} 的配置文件路径不可定位（未找到用户目录）"))
    })?;
    install_at(&path, conn, agent, item)
}

/// [`install`] 的路径显式版本：测试与多目标场景用。
///
/// 幂等：同名的 Jolu 项再装一次按最新内容更新（created=false）；
/// 同名的第三方条目拒绝（[`LibraryError::ForeignEntry`]）。
pub fn install_at(path: &std::path::Path, conn: &Connection, agent: &str, item: &LibraryItem) -> Result<InstallOutcome, LibraryError> {
    let LibraryItem::McpServer(server) = item;
    let Some(key) = agent_key(agent) else {
        return Err(LibraryError::UnsupportedAgent(agent.to_string()));
    };
    let name = server.name.trim();
    if name.is_empty() {
        return Err(LibraryError::Io("MCP server 名称为空".into()));
    }
    let ours = is_ours(conn, agent, name);
    let existing = agents::read_server_names(path, agent, key).map_err(LibraryError::Io)?;
    if existing.iter().any(|n| n == name) && !ours {
        return Err(LibraryError::ForeignEntry { agent: agent.to_string(), name: name.to_string() });
    }
    // 先落库再写文件，文件写失败回滚记录——与 wiring 的 mark_pending→写→mark_wired
    // 同一半事务语义：绝不允许「配置已写、记录却没有」的失忆状态（那样同名重装
    // 会被当第三方拒绝，用户只能手改配置解锁）
    let now = chrono::Utc::now().timestamp_millis();
    let payload = serde_json::to_string(item).map_err(|error| LibraryError::Io(error.to_string()))?;
    conn.execute(
        "INSERT INTO library_applied(agent, name, kind, payload_json, installed_ms)
         VALUES(?1, ?2, 'mcp_server', ?3, ?4)
         ON CONFLICT(agent, name) DO UPDATE SET payload_json = excluded.payload_json, installed_ms = excluded.installed_ms",
        [agent, name, payload.as_str(), now.to_string().as_str()],
    )
    .map_err(|error| LibraryError::Io(format!("记录 library_applied 失败：{error}")))?;
    if let Err(error) = agents::write_server(path, agent, key, server) {
        let _ = conn.execute("DELETE FROM library_applied WHERE agent = ?1 AND name = ?2", [agent, name]);
        return Err(LibraryError::Io(error));
    }
    Ok(InstallOutcome { agent: agent.to_string(), name: name.to_string(), created: !existing.contains(&name.to_string()) })
}

/// 卸载：只删 `library_applied` 表里登记过的项。返回是否真的删了文件里的条目。
/// 测试走 [`remove_at`]（不碰真实用户文件）。
pub fn remove(conn: &Connection, agent: &str, name: &str) -> Result<bool, LibraryError> {
    let path = mcp_config_path(agent).ok_or_else(|| LibraryError::Io(format!("{agent} 的配置文件路径不可定位")))?;
    let ours = is_ours(conn, agent, name);
    let removed = remove_at(&path, conn, agent, name)?;
    if ours {
        if let Some(key) = agent_key(agent) {
            clean_legacy(agent, key, name);
        }
    }
    Ok(removed)
}

/// [`remove`] 的路径显式版本。
pub fn remove_at(path: &std::path::Path, conn: &Connection, agent: &str, name: &str) -> Result<bool, LibraryError> {
    if !is_ours(conn, agent, name) {
        // 不是 Jolu 装的：不动文件，如实说明
        return Ok(false);
    }
    let Some(key) = agent_key(agent) else {
        return Err(LibraryError::UnsupportedAgent(agent.to_string()));
    };
    let removed = agents::remove_server(path, agent, key, name).map_err(LibraryError::Io)?;
    conn.execute("DELETE FROM library_applied WHERE agent = ?1 AND name = ?2", [agent, name])
        .map_err(|error| LibraryError::Io(format!("清理 library_applied 失败：{error}")))?;
    Ok(removed)
}

/// 卸载钩子（`--restore-agents`）：移除所有 agent 的全部 Jolu 库项，返回每项结果行。
pub fn restore_all_library(conn: &Connection) -> Vec<String> {
    for agent in LIBRARY_AGENTS {
        if let Some(key) = agent_key(agent) {
            for name in applied_names(conn, agent) {
                clean_legacy(agent, key, &name);
            }
        }
    }
    restore_all_library_at(mcp_config_path, conn)
}

/// [`restore_all_library`] 的路径解析可注入版本（测试用临时目录）。
pub fn restore_all_library_at(
    path_of: impl Fn(&str) -> Option<std::path::PathBuf>,
    conn: &Connection,
) -> Vec<String> {
    let mut lines = Vec::new();
    for agent in LIBRARY_AGENTS {
        for name in applied_names(conn, agent) {
            let result = match path_of(agent) {
                Some(path) => remove_at(&path, conn, agent, &name),
                None => Err(LibraryError::Io(format!("{agent} 的配置文件路径不可定位"))),
            };
            match result {
                Ok(true) => lines.push(format!("已移除 {agent} 的「{name}」")),
                Ok(false) => lines.push(format!("跳过 {agent} 的「{name}」（配置里已不在）")),
                Err(error) => lines.push(format!("失败 {agent} 的「{name}」：{error}")),
            }
        }
    }
    lines
}

/// 搜索 MCP server 的启动项：**当前可执行文件** + `--mcp-search`。
///
/// 为什么用当前 exe：安装包里只有主程序一个 exe（tauri bundle 不含额外 bin），
/// agent 配置里写它就能在 agent 启动时被拉起；桌面端安装时 `current_exe` 指向
/// `%LOCALAPPDATA%\Jolu\jolu.exe`，开发环境指向 target\debug。
pub fn search_entry_command() -> Result<(String, Vec<String>), LibraryError> {
    let exe = std::env::current_exe().map_err(|error| LibraryError::Io(format!("无法定位当前可执行文件：{error}")))?;
    Ok((exe.to_string_lossy().to_string(), vec!["--mcp-search".into()]))
}

/// CLI 场景的启动项：CLI 进程（jolu_cli.exe）**没有** `--mcp-search` 分支，
/// agent 配置里必须写主程序——取同目录的 `jolu.exe`（安装包与 target 目录里都同目录）。
pub fn search_entry_command_from_cli() -> Result<(String, Vec<String>), LibraryError> {
    let current = std::env::current_exe().map_err(|error| LibraryError::Io(format!("无法定位当前可执行文件：{error}")))?;
    let main = current
        .parent()
        .map(|dir| dir.join("jolu.exe"))
        .ok_or_else(|| LibraryError::Io("无法定位主程序目录".into()))?;
    if !main.is_file() {
        return Err(LibraryError::Io(format!(
            "同目录找不到主程序 jolu.exe（{}）：请在桌面端里安装，或把本 CLI 与 jolu.exe 放在一起",
            main.display()
        )));
    }
    Ok((main.to_string_lossy().to_string(), vec!["--mcp-search".into()]))
}

/// 预构好的「Jolu 搜索」库项。`exe` = agent 配置里要写的可执行文件路径。
pub fn search_item_with(exe: &std::path::Path) -> LibraryItem {
    LibraryItem::McpServer(McpServerItem {
        name: "jolu-search".into(),
        command: exe.to_string_lossy().to_string(),
        args: vec!["--mcp-search".into()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        crate::db::open(&std::env::temp_dir().join(format!(
            "jolu-library-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        )))
        .unwrap()
    }

    fn item(name: &str) -> LibraryItem {
        LibraryItem::McpServer(McpServerItem { name: name.into(), command: r"C:\jolu\jolu.exe".into(), args: vec!["--mcp-search".into()] })
    }

    /// 临时目录路径解析：测试绝不写真实的 ~/.claude / ~/.codex
    fn temp_path_of(agent: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "jolu-library-home-{}-{}-{agent}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path.join(match agent {
            "claude-code" => "settings.json",
            "codex" => "config.toml",
            _ => "config.json",
        })
    }

    #[test]
    fn unsupported_agent_is_reported_before_any_write() {
        let conn = database();
        assert!(matches!(
            install(&conn, "trae", &item("x")),
            Err(LibraryError::UnsupportedAgent(a)) if a == "trae"
        ));
        assert!(matches!(
            install(&conn, "custom", &item("x")),
            Err(LibraryError::UnsupportedAgent(_))
        ));
    }


    #[test]
    fn mcp_files_are_where_each_agent_reads_them_not_the_wiring_files() {
        // 首版借用接线路径：Claude Code 写进 settings.json、Gemini 写进 .env，两处都不生效
        let claude = mcp_config_path("claude-code").unwrap();
        assert_eq!(claude.file_name().unwrap(), ".claude.json");
        let gemini = mcp_config_path("gemini").unwrap();
        assert!(gemini.ends_with(std::path::Path::new(".gemini").join("settings.json")));
        assert!(mcp_config_path("zcode").unwrap().ends_with(std::path::Path::new(".zcode").join("cli").join("config.json")));
        assert!(mcp_config_path("workbuddy").unwrap().ends_with("mcp.json"));
        assert!(mcp_config_path("trae").is_none());
        for agent in LIBRARY_AGENTS {
            assert_eq!(agent_key(agent).is_some(), unsupported_reason(agent).is_none(), "{agent}");
        }
    }

    #[test]
    fn legacy_entries_are_cleaned_and_a_json_only_env_file_is_deleted() {
        let dir = std::env::temp_dir().join(format!("jolu-library-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 旧版在不存在的 .env 里凭空建了 JSON
        let env = dir.join(".env");
        std::fs::write(&env, r#"{"mcpServers":{"jolu-search":{"command":"x","args":[]}}}"#).unwrap();
        clean_legacy_at(&env, "jolu-search");
        assert!(!env.exists(), "只剩空对象的 .env 整个删掉");
        // settings.json 里还有用户自己的内容：只删 Jolu 的条目
        let settings = dir.join("settings.json");
        std::fs::write(&settings, r#"{"theme":"dark","mcpServers":{"jolu-search":{"command":"x"},"mine":{"command":"y"}}}"#).unwrap();
        clean_legacy_at(&settings, "jolu-search");
        let left: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(left["theme"], "dark");
        assert!(left["mcpServers"].get("jolu-search").is_none() && left["mcpServers"].get("mine").is_some());
        // 真正的 .env（不是 JSON）读不懂就不动
        std::fs::write(&env, "GEMINI_API_KEY=abc\n").unwrap();
        clean_legacy_at(&env, "jolu-search");
        assert_eq!(std::fs::read_to_string(&env).unwrap(), "GEMINI_API_KEY=abc\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn install_is_idempotent_and_updates_in_place() {
        let conn = database();
        let path = temp_path_of("claude-code");
        let first = install_at(&path, &conn, "claude-code", &item("jolu-search")).unwrap();
        assert!(first.created);
        let second = install_at(&path, &conn, "claude-code", &item("jolu-search")).unwrap();
        assert!(!second.created, "同名重装是更新不是新装");
        assert_eq!(applied_names(&conn, "claude-code"), vec!["jolu-search".to_string()]);
    }

    #[test]
    fn foreign_same_name_entry_is_refused_not_overwritten() {
        let conn = database();
        let path = temp_path_of("claude-code");
        // 用户（或第三方）先写了一个同名条目——Jolu 必须拒绝覆盖
        std::fs::write(&path, r#"{"mcpServers":{"jolu-search":{"command":"C:/theirs/tool.exe"}}}"#).unwrap();
        let error = install_at(&path, &conn, "claude-code", &item("jolu-search")).unwrap_err();
        assert!(matches!(error, LibraryError::ForeignEntry { .. }), "{error}");
        // 文件原样：没被覆盖
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("theirs/tool.exe"), "第三方内容必须原样保留");
    }

    #[test]
    fn remove_only_touches_our_own_entries() {
        let conn = database();
        let path = temp_path_of("claude-code");
        install_at(&path, &conn, "claude-code", &item("jolu-search")).unwrap();
        // 别人的名字：remove 返回 false 且不动文件
        assert!(!remove_at(&path, &conn, "claude-code", "user-own").unwrap());
        assert!(remove_at(&path, &conn, "claude-code", "jolu-search").unwrap());
        assert!(applied_names(&conn, "claude-code").is_empty());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("jolu-search"), "卸载后配置里不应再有该条目：{text}");
    }

    #[test]
    fn restore_removes_everything_across_agents() {
        let conn = database();
        let claude_path = temp_path_of("claude-code");
        let opencode_path = temp_path_of("opencode");
        install_at(&claude_path, &conn, "claude-code", &item("jolu-search")).unwrap();
        install_at(&opencode_path, &conn, "opencode", &item("jolu-search")).unwrap();
        let paths = |agent: &str| match agent {
            "claude-code" => Some(claude_path.clone()),
            "opencode" => Some(opencode_path.clone()),
            _ => None,
        };
        let lines = restore_all_library_at(paths, &conn);
        assert_eq!(lines.len(), 2, "{lines:?}");
        for agent in ["claude-code", "opencode"] {
            assert!(applied_names(&conn, agent).is_empty(), "{agent} 应被清空");
        }
    }
}
