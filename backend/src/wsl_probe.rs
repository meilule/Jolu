//! WSL 内 agent 安装的探测（批次 G §3.12）。
//!
//! 背景：magpie 的 `appdir` 语义允许配置指向 WSL 里的安装。Jolu 目前只给主安装接线
//! （接线面板已写明「同一工具的第二份安装（WSL、便携版）暂不支持」）。本模块只做
//! **只读探测**：查 WSL 是否存在、里面有没有这些 CLI、配置文件在哪，作为后续接线的
//! 数据基础；**本模块不写任何 WSL 内的文件**。
//!
//! 边界：
//! - 没装 WSL 时 `available: false`，`reason` 说明原因，不去猜；
//! - 发行版列表读不出来同样如实报错；
//! - 探测超时按「查不到」处理，不阻塞界面。

use serde::Serialize;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 探测超时：`wsl.exe -l` 在 WSL 服务未启动时要等一会儿，给足 8 秒。
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Serialize)]
pub struct WslAgent {
    /// 与主安装同一套 agent id
    pub agent: String,
    /// WSL 内该命令是否在 PATH 里
    pub found: bool,
    /// WSL 内该命令的绝对路径（`command -v` 的结果）
    pub path: Option<String>,
    /// 该 agent 的配置文件在 WSL 内的绝对路径（不检查是否存在，只按约定给出）
    pub config_path: String,
    /// 配置文件在 WSL 内是否确实存在
    pub config_exists: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WslProbe {
    /// `wsl.exe` 是否可用（找不到或执行失败都为 false）
    pub available: bool,
    /// 不可用时的原因；可用时为 None
    pub reason: Option<String>,
    /// 已安装的发行版名（不含 `*` 默认标记）
    pub distros: Vec<String>,
    /// 默认发行版名（`wsl.exe -l` 里带 `*` 的那一行）
    pub default_distro: Option<String>,
    /// 每个 agent 在默认发行版里的探测结果
    pub agents: Vec<WslAgent>,
}

/// agent id → WSL 内的命令名与配置文件约定路径（相对 `$HOME`）。
const WSL_AGENTS: [(&str, &str, &str); 4] = [
    ("claude-code", "claude", ".claude/settings.json"),
    ("codex", "codex", ".codex/config.toml"),
    ("gemini", "gemini", ".gemini/.env"),
    ("opencode", "opencode", ".config/opencode/opencode.json"),
];

/// 解析 `wsl.exe -l -q` 的输出。
///
/// WSL 的列表输出是 **UTF-16LE**（Windows 上直接按 UTF-8 读会得到夹着 `\0` 的乱码），
/// 这里同时容忍两种编码；行首 `*` 表示默认发行版。
///
/// 我们调的是 `-l -q`（每行一个名字），但宽表格式的 `-l` 输出也可能被喂进来，
/// 那种行是「NAME<空格>FRIENDLY NAME」两列，所以**取每行第一个词**作发行版名，
/// 并跳过表头、中文提示行（结尾是冒号的那些）。
pub(crate) fn parse_distro_list(raw: &[u8]) -> (Vec<String>, Option<String>) {
    let text = decode_wsl_output(raw);
    let mut distros = Vec::new();
    let mut default = None;
    for line in text.lines() {
        let trimmed = line.trim().trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() { continue; }
        // 表头与状态提示（「适用于 Linux 的 Windows 子系统分发:」「正在安装…」）
        if trimmed.ends_with(':') || trimmed.contains("正在") { continue; }
        let (is_default, rest) = match trimmed.strip_prefix('*') {
            Some(rest) => (true, rest.trim()),
            None => (false, trimmed),
        };
        // 宽表行有两列，发行版名是第一列
        let name = rest.split_whitespace().next().unwrap_or("");
        if name.is_empty() || name.eq_ignore_ascii_case("NAME") { continue; }
        if is_default { default = Some(name.to_string()); }
        distros.push(name.to_string());
    }
    (distros, default)
}

/// WSL 输出可能是 UTF-16LE（带或不带 BOM），也可能是 UTF-8。按字节特征判断。
fn decode_wsl_output(raw: &[u8]) -> String {
    if raw.len() >= 2 && ((raw[0] == 0xff && raw[1] == 0xfe) || looks_utf16le(raw)) {
        let start = if raw.len() >= 2 && raw[0] == 0xff && raw[1] == 0xfe { 2 } else { 0 };
        let units: Vec<u16> = raw[start..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        return String::from_utf16_lossy(&units).replace('\0', "");
    }
    String::from_utf8_lossy(raw).replace('\0', "")
}

/// UTF-16LE 的 ASCII 文本特征是「每隔一个字节就是 0」。
fn looks_utf16le(raw: &[u8]) -> bool {
    if raw.len() < 4 { return false; }
    let alternating = raw.iter().step_by(2).filter(|b| **b == 0).count() * 2;
    alternating >= raw.len() / 2
}

#[cfg(windows)]
fn wsl_exe() -> Option<std::path::PathBuf> {
    crate::session_threads::find_executable("wsl")
}

#[cfg(not(windows))]
fn wsl_exe() -> Option<std::path::PathBuf> {
    None
}

/// 跑一条 wsl 命令并取 stdout；超时或失败返回错误说明。
fn run_wsl(exe: &std::path::Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new(exe);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let child = command.spawn().map_err(|error| format!("无法执行 wsl.exe：{error}"))?;
    wait_with_timeout(child, PROBE_TIMEOUT).ok_or_else(|| "wsl.exe 执行超时".to_string())
}

fn wait_with_timeout(mut child: std::process::Child, timeout: Duration) -> Option<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            Err(_) => return None,
        }
    }
    let mut stdout = Vec::new();
    use std::io::Read;
    child.stdout.take()?.read_to_end(&mut stdout).ok()?;
    Some(stdout)
}

/// 在 WSL 默认发行版里跑一段 sh 脚本，返回 stdout 文本。
fn sh(exe: &std::path::Path, script: &str) -> Result<String, String> {
    let raw = run_wsl(exe, &["sh", "-lc", script])?;
    Ok(decode_wsl_output(&raw))
}

/// 探测 WSL 与其中的 agent 安装。只读，不写任何文件。
pub fn probe() -> WslProbe {
    let unavailable = |reason: String| WslProbe {
        available: false,
        reason: Some(reason),
        distros: Vec::new(),
        default_distro: None,
        agents: Vec::new(),
    };
    let Some(exe) = wsl_exe() else {
        return unavailable("未检测到 wsl.exe：这台机器没有安装 WSL".into());
    };
    let list = match run_wsl(&exe, &["-l", "-q"]) { Ok(raw) => raw, Err(error) => return unavailable(error) };
    let (distros, default_distro) = parse_distro_list(&list);
    if distros.is_empty() {
        return unavailable("WSL 里没有已安装的发行版".into());
    }
    let agents = WSL_AGENTS
        .iter()
        .map(|(agent, command, config)| {
            // 一次 sh 调用查命令与配置文件，避免逐 agent 反复起 wsl.exe
            let script = format!(
                "printf '%s\\n' \"$(command -v {command} 2>/dev/null)\"; \
                 [ -f \"$HOME/{config}\" ] && printf 'yes\\n' || printf 'no\\n'"
            );
            match sh(&exe, &script) {
                Ok(text) => {
                    let mut lines = text.lines();
                    let path = lines.next().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string);
                    let exists = lines.next().map(str::trim) == Some("yes");
                    WslAgent {
                        agent: (*agent).to_string(),
                        found: path.is_some(),
                        path,
                        config_path: format!("~/{config}"),
                        config_exists: exists,
                    }
                }
                // 单个 agent 查不动不影响其他 agent 的结论，如实标记「没找到」
                Err(_) => WslAgent {
                    agent: (*agent).to_string(),
                    found: false,
                    path: None,
                    config_path: format!("~/{config}"),
                    config_exists: false,
                },
            }
        })
        .collect();
    WslProbe { available: true, reason: None, distros, default_distro, agents }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utf8_quiet_list() {
        let raw = b"Ubuntu\nDebian\n";
        let (distros, default) = parse_distro_list(raw);
        assert_eq!(distros, vec!["Ubuntu", "Debian"]);
        assert_eq!(default, None, "-l -q 不带默认标记");
    }

    #[test]
    fn parses_utf16le_list_with_default_marker() {
        // UTF-16LE 编码的 "Ubuntu\n* Debian\n"
        let text = "Ubuntu\n* Debian\n";
        let raw: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let (distros, default) = parse_distro_list(&raw);
        assert_eq!(distros, vec!["Ubuntu", "Debian"]);
        assert_eq!(default.as_deref(), Some("Debian"));
    }

    #[test]
    fn real_wide_table_from_chinese_windows_keeps_only_names() {
        // 真机 `wsl.exe -l` 在中文 Windows 上的典型输出（制表符分隔两列）
        let raw = "适用于 Linux 的 Windows 子系统分发:\r\n\r\nNAME\t\t\tFRIENDLY NAME\r\nUbuntu-22.04\t\tUbuntu 22.04.3 LTS\r\n* Debian\t\tDebian GNU/Linux\r\n";
        let (distros, default) = parse_distro_list(raw.as_bytes());
        assert_eq!(distros, vec!["Ubuntu-22.04", "Debian"]);
        assert_eq!(default.as_deref(), Some("Debian"), "带 * 的是默认发行版");
    }

    #[test]
    fn parses_utf16le_with_bom() {
        let text = "* Ubuntu\r\n";
        let mut raw: Vec<u8> = vec![0xff, 0xfe];
        raw.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
        let (distros, default) = parse_distro_list(&raw);
        assert_eq!(distros, vec!["Ubuntu"]);
        assert_eq!(default.as_deref(), Some("Ubuntu"));
    }

    #[test]
    fn skips_header_and_prompt_lines() {
        let raw = "适用于 Linux 的 Windows 子系统分发:\r\nNAME            FRIENDLY NAME\r\nUbuntu          Ubuntu 22.04\r\n正在安装，这可能需要几分钟…\r\n".as_bytes();
        let (distros, _) = parse_distro_list(raw);
        assert_eq!(distros, vec!["Ubuntu"], "宽表行只取第一列，表头与提示行都跳过");
    }

    #[test]
    fn empty_output_is_not_an_error_but_yields_nothing() {
        let (distros, default) = parse_distro_list(b"");
        assert!(distros.is_empty());
        assert_eq!(default, None);
    }

    #[test]
    fn probe_reports_reason_when_wsl_missing() {
        if wsl_exe().is_some() { return; }
        let probe = probe();
        assert!(!probe.available);
        assert!(probe.reason.is_some(), "不可用必须给原因");
        assert!(probe.agents.is_empty());
    }
}
