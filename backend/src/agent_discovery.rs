//! Agent 自动发现（A1/A2，magpie 对齐文档 §8）：探测本机装了哪些 agent。
//!
//! 探测顺序照 magpie 的 `Detected()`（internal/agent/agent.go 370–394 行），命中任一即算已装：
//! 配置文件存在 → 配置目录存在 → CLI 命令找到。证据按这个顺序记录（文件最强）。
//!
//! Windows 的命令查找带兜底（照 magpie `binAt`/`FindTool`，396–410 行 + `proc/find.go`）：
//! 当前进程 PATH 之外，还查注册表里最新的用户/系统 PATH，以及 npm、`~/.local/bin`、
//! bun、Volta、pnpm、mise 的固定目录——否则「装完 agent 才启动 Jolu」的会话里 PATH
//! 还是旧的，永远探测不到（magpie #839 / #1482）。
//!
//! 状态分层（A2）：
//! - `installed`：有任一证据，且 CLI 在——可正常接线
//! - `cli_missing`：有配置证据但 CLI 不在——配置残留（agent 卸载了），接线按钮禁用
//! - `not_found`：全无证据——本机没装
//!
//! 现有 5 家的配置目录都是独占的，不存在 magpie 的 dirShared 情形（Cursor CLI 与
//! Cursor 编辑器共用 `~/.cursor` 那种），所以没有 dirShared / sign 字段；以后新增
//! 共用目录的 agent 时再加。
//!
//! 只读：不创建目录、不写文件、不运行 agent 的命令（版本探测在 `agent_version.rs`）。

use serde::Serialize;
use std::path::{Path, PathBuf};

/// 一个 agent 的探测规格。`agent` 与 `gateway::wiring::AGENTS` 同一套 id。
struct Spec {
    agent: &'static str,
    name: &'static str,
    /// PATH 里的命令名
    bin: &'static str,
    /// 配置目录：home 之上解析（含各自的环境变量覆盖，与接线用的路径同源）
    dir: fn(home: &Path) -> Option<PathBuf>,
    /// 配置文件相对 dir 的片段；空串表示没有单独文件，只看目录
    file: &'static str,
}

fn dir_claude(home: &Path) -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(home.join(".claude")))
}

fn dir_codex(home: &Path) -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(home.join(".codex")))
}

fn dir_zcode(home: &Path) -> Option<PathBuf> {
    // 与 wiring::zcode_dir 同款：ZCODE_HOME 覆盖，缺省 ~/.zcode
    std::env::var_os("ZCODE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(home.join(".zcode")))
}

fn dir_gemini(home: &Path) -> Option<PathBuf> {
    Some(home.join(".gemini"))
}

fn dir_opencode(home: &Path) -> Option<PathBuf> {
    Some(
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("opencode"),
    )
}

/// 探测规格表。顺序即界面展示顺序，与 `agent_version::AGENTS` 一致。
const SPECS: [Spec; 5] = [
    Spec { agent: "claude-code", name: "Claude Code", bin: "claude", dir: dir_claude, file: "settings.json" },
    Spec { agent: "codex", name: "Codex", bin: "codex", dir: dir_codex, file: "config.toml" },
    Spec { agent: "zcode", name: "ZCode", bin: "zcode", dir: dir_zcode, file: "v2/provider_config.json" },
    Spec { agent: "gemini", name: "Gemini CLI", bin: "gemini", dir: dir_gemini, file: ".env" },
    Spec { agent: "opencode", name: "OpenCode", bin: "opencode", dir: dir_opencode, file: "opencode.json" },
];

/// 一家 agent 的发现结果。
#[derive(Debug, Clone, Serialize)]
pub struct AgentDiscovery {
    /// 与 `gateway::wiring::AGENTS` 同一套 id
    pub agent: String,
    pub name: String,
    /// 任一证据命中即算已装
    pub installed: bool,
    /// installed / cli_missing / not_found
    pub state: &'static str,
    /// 最强证据：config_file / config_dir / bin。not_found 时为 None
    pub evidence: Option<&'static str>,
    /// CLI 命令是否找到（含 Windows 兜底目录）
    pub cli_found: bool,
    /// CLI 路径（诊断用；探测不到时 None）
    pub cli_path: Option<String>,
}

/// 决策核心（纯函数，测试直接覆盖）：
/// 文件 → 目录 → 命令，命中任一即已装；已装但命令不在归 `cli_missing`。
/// 返回 (installed, state, evidence)。
fn classify(file_found: bool, dir_found: bool, bin_found: bool) -> (bool, &'static str, Option<&'static str>) {
    let (installed, evidence) = if file_found {
        (true, Some("config_file"))
    } else if dir_found {
        (true, Some("config_dir"))
    } else if bin_found {
        (true, Some("bin"))
    } else {
        (false, None)
    };
    let state = if installed { if bin_found { "installed" } else { "cli_missing" } } else { "not_found" };
    (installed, state, evidence)
}

/// 当前进程 PATH 里找命令（接线/续聊用的既有实现）。
fn bin_on_path(name: &str) -> Option<PathBuf> {
    crate::session_threads::find_executable(name)
}

/// Windows 注册表里的 PATH（HKLM 系统 + HKCU 用户），展开 REG_EXPAND_SZ。
/// magpie 的 `LoginPath` 同款（proc/path_windows.go 63–91 行）。
#[cfg(windows)]
fn registry_path_dirs() -> Vec<PathBuf> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};
    const PLACES: [(&str, &str, HKEY); 2] = [
        // 键、值名、根。系统在前（登录 PATH 的组成），用户在后（覆盖在前更合理，这里只做查找不排优先级）
        ("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment", "Path", HKEY_LOCAL_MACHINE),
        ("Environment", "Path", HKEY_CURRENT_USER),
    ];
    let mut dirs = Vec::new();
    for (key_path, value_name, root) in PLACES {
        let key: Vec<u16> = key_path.encode_utf16().chain(std::iter::once(0)).collect();
        let value: Vec<u16> = value_name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut size: u32 = 0;
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
        // 第一次带空缓冲拿字节数；REG_EXPAND_SZ 会自动展开成 REG_SZ 返回
        let status = unsafe {
            RegGetValueW(root, key.as_ptr(), value.as_ptr(), flags, std::ptr::null_mut(), std::ptr::null_mut(), &mut size)
        };
        if status != 0 || size == 0 {
            continue;
        }
        let mut buffer = vec![0u16; (size as usize) / 2];
        let mut written = size;
        let status = unsafe {
            RegGetValueW(root, key.as_ptr(), value.as_ptr(), flags, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut written)
        };
        if status != 0 {
            continue;
        }
        let text = String::from_utf16_lossy(&buffer[..(written as usize) / 2]);
        // RegGetValueW 对 REG_EXPAND_SZ 的展开用的是当前进程环境，SystemRoot 一般在；
        // 展开不了的（极少见的受限环境）至少把 %SystemRoot% 补上，别的变量不猜
        let text = text.replace("%SystemRoot%", &std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()));
        for part in text.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            dirs.push(PathBuf::from(part));
        }
    }
    dirs
}

/// 用户级 CLI 安装目录（Windows）：npm 全局、官方安装脚本、bun、Volta、pnpm、mise。
/// magpie 的 `UserBinDirs` 同款（proc/path_windows.go 29–47 行）。
#[cfg(windows)]
fn user_bin_dirs(home: &Path) -> Vec<PathBuf> {
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let mise = local.as_ref().map(|base| base.join("mise"));
    let mut dirs = vec![
        appdata.map(|base| base.join("npm")),
        Some(home.join(".local").join("bin")),
        Some(home.join(".bun").join("bin")),
        local.as_ref().map(|base| base.join("Volta").join("bin")),
        local.as_ref().map(|base| base.join("pnpm")),
        mise.map(|base| base.join("shims")),
    ];
    dirs.retain(Option::is_some);
    dirs.into_iter().flatten().collect()
}

/// 命令查找：进程 PATH 失败后，在 Windows 兜底目录里找 `name.exe` / `name.cmd` / `name.bat`。
/// 非 Windows 只有进程 PATH（magpie 的 binAt 在非 Windows 也只补 Homebrew，这里没有）。
pub(crate) fn find_bin(name: &str, home: &Path) -> Option<PathBuf> {
    if let Some(found) = bin_on_path(name) {
        return Some(found);
    }
    let mut dirs = user_bin_dirs(home);
    #[cfg(windows)]
    dirs.extend(registry_path_dirs());
    find_bin_in(name, &dirs)
}

/// 在给定目录清单里按 PATHEXT 找命令（测试注入目录用）。
fn find_bin_in(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(|ext| ext.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    dirs.iter().find_map(|dir| {
        exts.iter()
            .map(|ext| dir.join(format!("{name}{ext}")))
            .find(|candidate| candidate.is_file())
    })
}

/// 发现全部 agent（生产入口）。
pub(crate) fn discover() -> Vec<AgentDiscovery> {
    let home = crate::creds::home().unwrap_or_else(|| PathBuf::from("."));
    discover_with(&home, &find_bin)
}

/// 可注入版（测试用）：home 与命令查找都由调用方给。
pub(crate) fn discover_with(home: &Path, bin_lookup: &dyn Fn(&str, &Path) -> Option<PathBuf>) -> Vec<AgentDiscovery> {
    SPECS
        .iter()
        .map(|spec| {
            let dir = (spec.dir)(home);
            let dir_found = dir.as_deref().is_some_and(|path| path.is_dir());
            let file_found = !spec.file.is_empty()
                && dir.as_deref()
                    .map(|base| base.join(spec.file))
                    .is_some_and(|path| path.is_file());
            let bin_path = bin_lookup(spec.bin, home);
            let bin_found = bin_path.is_some();
            let (installed, state, evidence) = classify(file_found, dir_found, bin_found);
            AgentDiscovery {
                agent: spec.agent.to_string(),
                name: spec.name.to_string(),
                installed,
                state,
                evidence,
                cli_found: bin_found,
                cli_path: bin_path.map(|path| path.to_string_lossy().to_string()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 临时 home：`.zcode/v2` 这类多级目录按需建，测完即删。
    fn temp_home(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("jolu-discovery-{tag}-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 命令查找只认测试放进去的文件：注入版查找，指向 home 里的 `bins/<name>.exe`
    fn test_bins(home: &Path) -> impl Fn(&str, &Path) -> Option<PathBuf> + '_ {
        move |name: &str, base: &Path| {
            let candidate = base.join("bins").join(format!("{name}.exe"));
            candidate.is_file().then(|| candidate)
        }
    }

    fn agent_of<'a>(results: &'a [AgentDiscovery], agent: &str) -> &'a AgentDiscovery {
        results.iter().find(|d| d.agent == agent).unwrap_or_else(|| panic!("缺少 {agent}"))
    }

    #[test]
    fn classify_orders_file_over_dir_over_bin() {
        // 文件最强：文件在即 config_file
        assert_eq!(classify(true, true, true), (true, "installed", Some("config_file")));
        // 命令不在就是 cli_missing，哪怕文件、目录都在（配置残留 = agent 卸载了）
        assert_eq!(classify(true, true, false), (true, "cli_missing", Some("config_file")));
        // 次之目录
        assert_eq!(classify(false, true, true), (true, "installed", Some("config_dir")));
        // 只有命令也算装了
        assert_eq!(classify(false, false, true), (true, "installed", Some("bin")));
        // 全无
        assert_eq!(classify(false, false, false), (false, "not_found", None));
    }

    #[test]
    fn classify_marks_installed_without_bin_as_cli_missing() {
        // 配置残留（agent 卸载了）：已装但没有命令 → cli_missing
        assert_eq!(classify(true, true, false), (true, "cli_missing", Some("config_file")));
        assert_eq!(classify(false, true, false), (true, "cli_missing", Some("config_dir")));
    }

    /// §8.5 场景 4：删掉 CLI 但保留配置目录 → cli_missing
    #[test]
    fn config_left_behind_without_cli_is_cli_missing() {
        let home = temp_home("cli-missing");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(home.join(".claude").join("settings.json"), "{}").unwrap();
        let results = discover_with(&home, &test_bins(&home));
        let found = agent_of(&results, "claude-code");
        assert!(found.installed);
        assert_eq!(found.state, "cli_missing");
        assert_eq!(found.evidence, Some("config_file"));
        assert!(!found.cli_found);
        std::fs::remove_dir_all(&home).ok();
    }

    /// §8.5 场景 1 的查找部分：命令不在 PATH，但放在兜底目录里 → 找得到。
    /// 直接给目录清单（不依赖真机 PATH 与注册表，真机 PATH 命中会短路兜底）。
    #[cfg(windows)]
    #[test]
    fn find_bin_falls_back_to_user_dirs() {
        let home = temp_home("bin-fallback");
        // 官方安装脚本目录：~/.local/bin/claude.exe（PATH 里没有它）
        let bin = home.join(".local").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("claude.exe"), b"MZ").unwrap();
        let found = find_bin_in("claude", &[bin.clone()]);
        assert!(found.is_some(), "兜底目录里的命令要能找到");
        assert!(found.unwrap().to_string_lossy().contains(".local"));
        // 没有的目录 → 找不到
        assert_eq!(find_bin_in("codex", &[bin]), None);
        std::fs::remove_dir_all(&home).ok();
    }

    /// 兜底目录清单包含 npm 与 mise（npm 全局是这些 CLI 最常见的安装方式）
    #[cfg(windows)]
    #[test]
    fn user_bin_dirs_cover_npm_and_mise() {
        let home = temp_home("bin-dirs");
        let dirs = user_bin_dirs(&home);
        let text = dirs.iter().map(|d| d.to_string_lossy().to_lowercase()).collect::<Vec<_>>().join(";");
        assert!(text.contains("npm"), "npm 全局目录要在清单里：{text}");
        assert!(text.contains("mise"), "mise shims 要在清单里：{text}");
        std::fs::remove_dir_all(&home).ok();
    }

    /// 注册表读取不炸、结果都是绝对路径（内容因机器而异，不断言具体值）
    #[cfg(windows)]
    #[test]
    fn registry_path_dirs_returns_absolute_paths() {
        let dirs = registry_path_dirs();
        for dir in dirs {
            assert!(dir.is_absolute(), "注册表 PATH 展开后应是绝对路径：{dir:?}");
        }
    }

    /// 全无证据 → not_found；结果覆盖全部 5 家且不重不漏
    #[test]
    fn empty_home_reports_not_found_for_all() {
        let home = temp_home("empty");
        let results = discover_with(&home, &test_bins(&home));
        assert_eq!(results.len(), SPECS.len());
        for found in &results {
            assert!(!found.installed, "{} 不该被误判", found.agent);
            assert_eq!(found.state, "not_found");
            assert_eq!(found.evidence, None);
        }
        std::fs::remove_dir_all(&home).ok();
    }

    /// 有配置文件与命令 → installed（完整正向用例）
    #[test]
    fn config_and_bin_together_is_installed() {
        let home = temp_home("full");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(home.join(".codex").join("config.toml"), "[model]\n").unwrap();
        std::fs::create_dir_all(home.join("bins")).unwrap();
        std::fs::write(home.join("bins").join("codex.exe"), b"MZ").unwrap();
        let results = discover_with(&home, &test_bins(&home));
        let found = agent_of(&results, "codex");
        assert!(found.installed);
        assert_eq!(found.state, "installed");
        assert_eq!(found.evidence, Some("config_file"));
        assert!(found.cli_found);
        std::fs::remove_dir_all(&home).ok();
    }
}
