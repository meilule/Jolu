use std::path::{Path, PathBuf};
use rusqlite::Connection;
use serde::Serialize;
use super::source::{read_cwd, source_files};

/* ------------------------------ 在原工具继续 ------------------------------ */

#[derive(Debug, Serialize)]
pub struct ContinueCheck {
    /// 该来源是否有可靠的命令行续聊入口
    pub supported: bool,
    pub tool_found: bool,
    pub cwd: Option<String>,
    pub cwd_accessible: bool,
}

pub(crate) fn tool_command(platform: &str) -> Option<(&'static str, &'static [&'static str])> {
    match platform {
        "claude" => Some(("claude", &["--resume"])),
        "codex" => Some(("codex", &["resume"])),
        _ => None,
    }
}

/// 在 PATH 里找可执行文件（Windows 按 PATHEXT 补扩展名）
pub(crate) fn find_executable(name: &str) -> Option<PathBuf> {
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into()).split(';').map(|e| e.to_ascii_lowercase()).collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        exts.iter().map(|ext| dir.join(format!("{name}{ext}"))).find(|p| p.is_file())
    })
}

/// 会话 ID 会拼进新控制台的命令行，只接受 UUID 一类的安全字符
pub(crate) fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn continue_check(conn: &Connection, platform: &str, session_id: &str) -> rusqlite::Result<ContinueCheck> {
    let files = source_files(conn)?;
    let cwd = files.get(&(platform.to_string(), session_id.to_string())).filter(|p| p.exists()).and_then(|p| read_cwd(p));
    let supported = tool_command(platform).is_some() && safe_id(session_id);
    Ok(ContinueCheck {
        supported,
        tool_found: tool_command(platform).is_some_and(|(name, _)| find_executable(name).is_some()),
        cwd_accessible: cwd.as_deref().is_some_and(|d| Path::new(d).is_dir()),
        cwd,
    })
}

/// 打开新的控制台窗口，在项目目录里执行续聊命令；后续消息由原工具处理
pub fn continue_session(conn: &Connection, platform: &str, session_id: &str) -> Result<(), String> {
    let check = continue_check(conn, platform, session_id).map_err(|e| e.to_string())?;
    let (name, args) = tool_command(platform).filter(|_| check.supported).ok_or("这个来源暂不支持在原工具继续")?;
    if !check.tool_found {
        return Err(format!("未检测到 {name} 命令，请安装或配置后重试"));
    }
    let dir = check.cwd.filter(|_| check.cwd_accessible).ok_or("项目目录不可访问")?;
    let mut command = std::process::Command::new("cmd");
    command.arg("/K").arg(name).args(args).arg(session_id).current_dir(&dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        command.creation_flags(CREATE_NEW_CONSOLE);
    }
    command.spawn().map(|_| ()).map_err(|e| format!("打开 {name} 失败：{e}"))
}
