//! Agent CLI 版本检测与更新命令提示。
//!
//! 只读：跑一次 `<cli> --version` 读出版本号，按「版本能读到 → 给出该安装方式对应的
//! 更新命令」组织结果。**不自动执行更新**——各 CLI 的升级方式与副作用不同，
//! 由用户自己决定什么时候升、升哪个。
//!
//! 语义边界（与 AGENTS.md 的「未知值不冒充」一致）：
//! - `found: false` 表示 PATH 里没有这个命令，不是「版本未知」；
//! - `version: None` 且 `found: true` 表示命令在，但 `--version` 没输出可解析的版本号；
//! - 更新命令只在能确定安装方式时给出，认定不准时给 `None` 并写明原因，不猜。

use serde::Serialize;
use std::process::{Command, Stdio};
use std::time::Duration;

/// 一个 agent 的可执行文件名与「它是谁」。顺序即界面展示顺序。
pub(crate) const AGENTS: [(&str, &str, &str); 5] = [
    ("claude", "Claude Code", "claude"),
    ("codex", "Codex", "codex"),
    ("gemini", "Gemini CLI", "gemini"),
    ("opencode", "OpenCode", "opencode"),
    ("zcode", "ZCode", "zcode"),
];

#[derive(Debug, Clone, Serialize)]
pub struct AgentVersion {
    /// 与 `gateway::wiring::AGENTS` 同一套 id，方便前端把版本贴到接线卡片上
    pub agent: String,
    pub name: String,
    /// PATH 里是否存在该命令
    pub found: bool,
    /// 解析出的版本号（如 `2.0.31`）；命令在但读不出来时为 None
    pub version: Option<String>,
    /// `--version` 的原始首行，排错时对照用；不含任何凭据（这些 CLI 的版本输出只有版本号）
    pub raw: Option<String>,
    /// 建议的更新命令；无法确定安装方式时为 None
    pub update_command: Option<String>,
    /// 更新命令为空时的说明，或已给出命令时的安装方式标注
    pub update_note: String,
}

/// 从 `--version` 的输出里取版本号：取第一个形如 `1.2` 或 `1.2.3` 的片段。
///
/// 各家输出格式不一（`2.0.31 (Claude Code)`、`codex-cli 0.42.0`、`v1.6.0`），
/// 统一按「数字.数字[.数字...]」提取，避免为每个 CLI 写一套解析。
pub(crate) fn parse_version(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        // 找到一段「数字(.数字)+」，且前面不是数字（避免从 2.0.31 里切出 0.31）
        let start = index;
        let mut cursor = index;
        let mut dots = 0;
        loop {
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() { cursor += 1; }
            if cursor < bytes.len() && bytes[cursor] == b'.' && cursor + 1 < bytes.len() && bytes[cursor + 1].is_ascii_digit() {
                dots += 1;
                cursor += 1;
            } else {
                break;
            }
        }
        if dots > 0 {
            return Some(raw[start..cursor].to_string());
        }
        index = cursor.max(index + 1);
    }
    None
}

/// 跑一次 `--version`。超时或启动失败都按「没读到版本」处理，不阻塞界面。
fn read_version(executable: &std::path::Path) -> Option<String> {
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 不弹控制台窗口
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let child = command.spawn().ok()?;
    // 版本输出很小；等一会儿没结果就当读不到，避免某个 CLI 卡住把界面拖死
    let output = wait_with_timeout(child, Duration::from_secs(5))?;
    let text = String::from_utf8_lossy(&output).to_string();
    let first = text.lines().find(|line| !line.trim().is_empty())?.trim().to_string();
    Some(first)
}

/// 等子进程结束并取 stdout；超时则杀掉子进程返回 None。
fn wait_with_timeout(mut child: std::process::Child, timeout: Duration) -> Option<Vec<u8>> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
    let mut stdout = Vec::new();
    use std::io::Read;
    child.stdout.take()?.read_to_end(&mut stdout).ok()?;
    Some(stdout)
}

/// 按可执行文件路径判断安装方式，给出对应的更新命令。
///
/// 只认能确定的情况：npm 全局、Homebrew、官方安装脚本目录。其余一律不给命令，
/// 让用户自己去官方文档升级——给错命令比不给更糟。
pub(crate) fn update_hint(executable: &std::path::Path, name: &str) -> (Option<String>, String) {
    let path = executable.to_string_lossy().to_lowercase();
    if path.contains("node_modules") || path.contains("/npm/") || path.contains("\\npm\\") {
        return (Some(format!("npm install -g {}", npm_package(name))), "通过 npm 全局安装".into());
    }
    if path.contains("homebrew") || path.contains("/opt/homebrew/") || path.contains("/usr/local/cellar/") {
        return (Some(format!("brew upgrade {}", brew_formula(name))), "通过 Homebrew 安装".into());
    }
    if path.contains("\\.local\\bin") || path.contains("/.local/bin") || path.contains("\\.claude\\local") {
        return (None, "通过官方安装脚本安装：按官方文档的升级方式更新".into());
    }
    (None, "安装方式未能识别：请按官方文档升级，避免用错包管理器".into())
}

fn npm_package(name: &str) -> &str {
    match name {
        "Claude Code" => "@anthropic-ai/claude-code",
        "Codex" => "@openai/codex",
        "Gemini CLI" => "@google/gemini-cli",
        "OpenCode" => "opencode-ai",
        "ZCode" => "zcode",
        other => other,
    }
}

fn brew_formula(name: &str) -> &str {
    match name {
        "Claude Code" => "claude-code",
        "Codex" => "codex",
        "Gemini CLI" => "gemini-cli",
        "OpenCode" => "opencode",
        "ZCode" => "zcode",
        other => other,
    }
}

/// 检测全部 agent 的版本。找不到的命令如实标记 `found: false`。
pub fn detect() -> Vec<AgentVersion> {
    AGENTS
        .iter()
        .map(|(agent, label, executable)| {
            let found_path = crate::session_threads::find_executable(executable);
            let found = found_path.is_some();
            let raw = found_path.as_deref().and_then(read_version);
            let version = raw.as_deref().and_then(parse_version);
            let (update_command, update_note) = match found_path.as_deref() {
                Some(path) => update_hint(path, label),
                None => (None, "未检测到命令：先安装并确保在 PATH 中".into()),
            };
            AgentVersion {
                agent: (*agent).to_string(),
                name: (*label).to_string(),
                found,
                version,
                raw: raw.clone(),
                update_command,
                update_note,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_from_common_output_shapes() {
        assert_eq!(parse_version("2.0.31 (Claude Code)"), Some("2.0.31".into()));
        assert_eq!(parse_version("codex-cli 0.42.0"), Some("0.42.0".into()));
        assert_eq!(parse_version("v1.6"), Some("1.6".into()));
        assert_eq!(parse_version("1.2.3.4"), Some("1.2.3.4".into()));
        assert_eq!(parse_version("no digits here"), None);
        assert_eq!(parse_version("build 12"), None, "单个整数不算版本号");
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn version_parse_does_not_start_inside_a_number() {
        // 「2026.1」是完整片段，不能从中间切出「026.1」
        assert_eq!(parse_version("2026.1"), Some("2026.1".into()));
    }

    #[test]
    fn npm_install_gives_package_specific_upgrade_command() {
        let path = std::path::Path::new("C:/Users/x/AppData/Roaming/npm/node_modules/@anthropic-ai/claude-code/cli.js");
        let (command, note) = update_hint(path, "Claude Code");
        assert_eq!(command.as_deref(), Some("npm install -g @anthropic-ai/claude-code"));
        assert!(note.contains("npm"));
    }

    #[test]
    fn unknown_install_method_does_not_invent_a_command() {
        let path = std::path::Path::new("C:/Program Files/WeirdTool/thing.exe");
        let (command, note) = update_hint(path, "ZCode");
        assert!(command.is_none(), "识别不出安装方式时不能瞎给命令");
        assert!(!note.is_empty());
    }

    #[test]
    fn detected_agents_keep_stable_ids() {
        let detected = detect();
        assert_eq!(detected.len(), AGENTS.len());
        assert_eq!(
            detected.iter().map(|d| d.agent.as_str()).collect::<Vec<_>>(),
            vec!["claude", "codex", "gemini", "opencode", "zcode"]
        );
        // 版本号与原始输出要么同时为空，要么版本号能解析出来
        for entry in &detected {
            if entry.raw.is_some() { assert_eq!(entry.raw.as_deref().and_then(parse_version), entry.version); }
        }
    }
}
