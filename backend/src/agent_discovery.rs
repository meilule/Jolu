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
    // 与 wiring::zcode_dir 同源：ZCODE_HOME 覆盖 → ZCode 主目录 → 设置里改过的 dataBaseDir
    Some(crate::creds::zcode_data_dir(home))
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

// ---------- 扩展工具的配置目录 ----------
// 路径取自 magpie `internal/agent/<tool>.go` 的 `at.join("...")` / `filepath.Join`。
// 只写能从源码确认的；不确定的不加（AGENTS.md：未知值不冒充）。

fn dir_qwen(home: &Path) -> Option<PathBuf> {
    // magpie qwen.go: `at.join(".qwen")`
    Some(home.join(".qwen"))
}

fn dir_kimi(home: &Path) -> Option<PathBuf> {
    // magpie kimi.go：优先 `~/.kimi`，新版在 `~/.kimi-code`
    let code = home.join(".kimi-code");
    if code.is_dir() {
        return Some(code);
    }
    Some(home.join(".kimi"))
}

fn dir_cline(home: &Path) -> Option<PathBuf> {
    // magpie cline.go：`CLINE_DIR` 覆盖，缺省 `~/.cline`
    std::env::var_os("CLINE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(home.join(".cline")))
}

fn dir_droid(home: &Path) -> Option<PathBuf> {
    // magpie droid.go: `at.open(filepath.Join(home, ".factory"))`
    Some(home.join(".factory"))
}

fn dir_zed(home: &Path) -> Option<PathBuf> {
    // magpie zed.go：Windows 在 `%APPDATA%\Zed`，其余 `~/.config/zed`
    #[cfg(windows)]
    {
        if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
            let dir = roaming.join("Zed");
            if dir.is_dir() {
                return Some(dir);
            }
        }
    }
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    Some(xdg.join("zed"))
}

fn dir_crush(home: &Path) -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    Some(xdg.join("crush"))
}

fn dir_goose(home: &Path) -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    Some(xdg.join("goose"))
}

fn dir_continue(home: &Path) -> Option<PathBuf> {
    Some(home.join(".continue"))
}

/// 探测规格表。顺序即界面展示顺序，与 `agent_version::AGENTS` 一致。
const SPECS: [Spec; 13] = [
    Spec { agent: "claude-code", name: "Claude Code", bin: "claude", dir: dir_claude, file: "settings.json" },
    Spec { agent: "codex", name: "Codex", bin: "codex", dir: dir_codex, file: "config.toml" },
    Spec { agent: "zcode", name: "ZCode", bin: "zcode", dir: dir_zcode, file: "v2/provider_config.json" },
    Spec { agent: "gemini", name: "Gemini CLI", bin: "gemini", dir: dir_gemini, file: ".env" },
    Spec { agent: "opencode", name: "OpenCode", bin: "opencode", dir: dir_opencode, file: "opencode.json" },
    // 以下 8 家只有探测（没有专门接线），接入走「通用接入」手填键名
    Spec { agent: "qwen", name: "Qwen Code", bin: "qwen", dir: dir_qwen, file: "settings.json" },
    Spec { agent: "kimi", name: "Kimi CLI", bin: "kimi", dir: dir_kimi, file: "config.toml" },
    Spec { agent: "cline", name: "Cline", bin: "cline", dir: dir_cline, file: "settings/providers.json" },
    Spec { agent: "droid", name: "Droid", bin: "droid", dir: dir_droid, file: "settings.json" },
    Spec { agent: "zed", name: "Zed", bin: "zed", dir: dir_zed, file: "settings.json" },
    Spec { agent: "crush", name: "Crush", bin: "crush", dir: dir_crush, file: "crush.json" },
    Spec { agent: "goose", name: "Goose", bin: "goose", dir: dir_goose, file: "config.yaml" },
    Spec { agent: "continue", name: "Continue", bin: "cn", dir: dir_continue, file: "config.json" },
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
        // 返回的字节数含结尾的 NUL：不剥掉的话最后一段是 "\0"，会被当成一个（相对的）目录
        for part in text.split(';') {
            let part = part.trim_matches(|c: char| c == '\0' || c.is_whitespace());
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

/// 各家官方安装器 / 桌面版的固定落点：装完不在 PATH 里，也不在通用兜底目录里。
/// 只按已知厂商目录逐个拼，不递归扫 `%LOCALAPPDATA%`。来源：
/// - Claude `~/.claude/local`、OpenCode `~/.opencode/bin`、Codex `~/.codex/packages/standalone`：
///   magpie `internal/agent/gone.go::cliHeld()` 的官方安装器落点
/// - Codex `%LOCALAPPDATA%\Programs\OpenAI\Codex\bin`：官方 `install.ps1` 的目录联接，指向
///   `~/.codex/packages/standalone/current/bin`（magpie `cliupdate.go` 的注释）
/// - Codex 桌面版（MSIX）：自带的 CLI 在 `%LOCALAPPDATA%\OpenAI\Codex\bin\<哈希>\codex.exe`
///   （2026-11-16 本机实测；magpie 不找这个 exe，改认 Store 包，见执行文档 T1 核对记录）
fn vendor_bin_dirs(name: &str, home: &Path, local_app_data: Option<&Path>) -> Vec<PathBuf> {
    match name {
        "claude" => vec![home.join(".claude").join("local")],
        "opencode" => vec![home.join(".opencode").join("bin")],
        "codex" => {
            let mut dirs = Vec::new();
            if let Some(local) = local_app_data {
                dirs.push(local.join("Programs").join("OpenAI").join("Codex").join("bin"));
            }
            dirs.push(home.join(".codex").join("packages").join("standalone").join("current").join("bin"));
            if let Some(local) = local_app_data {
                dirs.extend(hashed_bin_dirs(&local.join("OpenAI").join("Codex").join("bin"), name));
            }
            dirs
        }
        _ => Vec::new(),
    }
}

/// 桌面版把 CLI 放在 `<root>\<哈希>\` 子目录里：只看一层，只留含该命令的子目录
/// （同级还有只装 `rg.exe` 这类工具的哈希目录），有多个时按命令文件的修改时间新到旧。
/// `root` 不存在或读不了时为空。
fn hashed_bin_dirs(root: &Path, name: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|dir| {
            let bin = find_bin_in(name, std::slice::from_ref(&dir))?;
            let modified = std::fs::metadata(&bin).and_then(|meta| meta.modified()).unwrap_or(std::time::UNIX_EPOCH);
            Some((modified, dir))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, dir)| dir).collect()
}

/// 命令查找：进程 PATH 失败后，依次在 Windows 兜底目录、注册表 PATH、各家固定落点里找
/// `name.exe` / `name.cmd` / `name.bat`。固定落点放最后：它是兜底，PATH 上有的优先。
/// 非 Windows 只有进程 PATH（magpie 的 binAt 在非 Windows 也只补 Homebrew，这里没有）。
pub(crate) fn find_bin(name: &str, home: &Path) -> Option<PathBuf> {
    if let Some(found) = bin_on_path(name) {
        return Some(found);
    }
    let mut dirs = user_bin_dirs(home);
    #[cfg(windows)]
    dirs.extend(registry_path_dirs());
    #[cfg(windows)]
    dirs.extend(vendor_bin_dirs(name, home, std::env::var_os("LOCALAPPDATA").map(PathBuf::from).as_deref()));
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
    fn test_bins(_home: &Path) -> impl Fn(&str, &Path) -> Option<PathBuf> + '_ {
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

    /// T1：Codex 桌面版的 CLI 在 `%LOCALAPPDATA%\OpenAI\Codex\bin\<哈希>\codex.exe`，
    /// 同级还有只放 rg.exe 的哈希目录。只靠固定落点（不经 PATH）也要判 installed，
    /// 且 cli_path 指向哈希目录里的 exe。
    #[test]
    fn codex_desktop_cli_in_hashed_dir_is_found() {
        let home = temp_home("codex-desktop");
        let local = home.join("AppData").join("Local");
        let bin_root = local.join("OpenAI").join("Codex").join("bin");
        std::fs::create_dir_all(bin_root.join("a7d4c4900eafc659")).unwrap();
        std::fs::write(bin_root.join("a7d4c4900eafc659").join("rg.exe"), b"MZ").unwrap();
        std::fs::create_dir_all(bin_root.join("5449c1f0f26f1f98")).unwrap();
        std::fs::write(bin_root.join("5449c1f0f26f1f98").join("codex.exe"), b"MZ").unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(home.join(".codex").join("config.toml"), "").unwrap();
        let lookup = |name: &str, base: &Path| find_bin_in(name, &vendor_bin_dirs(name, base, Some(&local)));
        let results = discover_with(&home, &lookup);
        let found = agent_of(&results, "codex");
        assert_eq!(found.state, "installed");
        assert_eq!(found.evidence, Some("config_file"));
        let cli = found.cli_path.as_deref().expect("要找到 codex.exe");
        assert!(cli.contains("5449c1f0f26f1f98") && cli.to_lowercase().ends_with("codex.exe"), "{cli}");
        // 别家不受影响：rg 所在的哈希目录不会被当成任何工具的命令目录
        assert_eq!(agent_of(&results, "claude-code").state, "not_found");
        std::fs::remove_dir_all(&home).ok();
    }

    /// 多个哈希目录都有 codex.exe（桌面版升级后旧目录没删）→ 取命令最新的那个；
    /// 根目录不存在 → 空
    #[test]
    fn hashed_bin_dirs_prefer_newest_and_skip_missing_root() {
        let home = temp_home("hashed");
        let root = home.join("bin");
        for (dir, age) in [("old", 3600u64), ("new", 0)] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            let exe = root.join(dir).join("codex.exe");
            std::fs::write(&exe, b"MZ").unwrap();
            let when = std::time::SystemTime::now() - std::time::Duration::from_secs(age);
            std::fs::File::options().write(true).open(&exe).unwrap().set_modified(when).unwrap();
        }
        std::fs::create_dir_all(root.join("tools")).unwrap();
        std::fs::write(root.join("tools").join("rg.exe"), b"MZ").unwrap();
        let dirs = hashed_bin_dirs(&root, "codex");
        assert_eq!(dirs, vec![root.join("new"), root.join("old")]);
        assert!(hashed_bin_dirs(&home.join("missing"), "codex").is_empty());
        std::fs::remove_dir_all(&home).ok();
    }

    /// 官方安装器落点：Claude `~/.claude/local`、OpenCode `~/.opencode/bin`、
    /// Codex `install.ps1` 的 `Programs\OpenAI\Codex\bin` 与 standalone；其他家没有固定落点
    #[test]
    fn vendor_bin_dirs_cover_official_installers() {
        let home = PathBuf::from("H");
        let local = PathBuf::from("L");
        assert_eq!(vendor_bin_dirs("claude", &home, Some(&local)), vec![home.join(".claude").join("local")]);
        assert_eq!(vendor_bin_dirs("opencode", &home, None), vec![home.join(".opencode").join("bin")]);
        let codex = vendor_bin_dirs("codex", &home, Some(&local));
        assert!(codex.contains(&local.join("Programs").join("OpenAI").join("Codex").join("bin")));
        assert!(codex.contains(&home.join(".codex").join("packages").join("standalone").join("current").join("bin")));
        assert!(vendor_bin_dirs("gemini", &home, Some(&local)).is_empty());
    }

    /// T2：ZCode 在设置里改过「数据存储路径」→ 原位置 `.zcode/v2/setting.json` 记着
    /// `dataBaseDir`，provider_config.json 在新位置；探测要跟过去拿到 config_file 证据
    #[test]
    fn zcode_follows_data_base_dir() {
        let home = temp_home("zcode-moved");
        let moved = temp_home("zcode-moved-target");
        std::fs::create_dir_all(home.join(".zcode").join("v2")).unwrap();
        let value = serde_json::to_string(&moved.to_string_lossy()).unwrap();
        std::fs::write(home.join(".zcode").join("v2").join("setting.json"), format!(r#"{{"dataBaseDir":{value}}}"#)).unwrap();
        std::fs::create_dir_all(moved.join(".zcode").join("v2")).unwrap();
        std::fs::write(moved.join(".zcode").join("v2").join("provider_config.json"), "{}").unwrap();
        let found = agent_of(&discover_with(&home, &test_bins(&home)), "zcode").clone();
        assert!(found.installed);
        assert_eq!(found.evidence, Some("config_file"), "provider_config.json 在 dataBaseDir 下，要能找到");
        // 接线写的就是探测看的这个目录
        assert_eq!(crate::creds::zcode_data_dir(&home), moved.join(".zcode"));
        std::fs::remove_dir_all(&home).ok();
        std::fs::remove_dir_all(&moved).ok();
    }

    /// T2：没有 dataBaseDir 时与改动前一致：看 `~/.zcode`
    #[test]
    fn zcode_without_data_base_dir_stays_at_home() {
        let home = temp_home("zcode-home");
        std::fs::create_dir_all(home.join(".zcode").join("v2")).unwrap();
        std::fs::write(home.join(".zcode").join("v2").join("provider_config.json"), "{}").unwrap();
        let found = agent_of(&discover_with(&home, &test_bins(&home)), "zcode").clone();
        assert_eq!(found.evidence, Some("config_file"));
        assert_eq!(crate::creds::zcode_data_dir(&home), home.join(".zcode"));
        std::fs::remove_dir_all(&home).ok();
    }

    /// 真机只读核对（默认不跑）：不经 PATH 与 npm，只靠固定落点能否找到本机的 Codex 桌面版 CLI。
    /// `cargo test --manifest-path backend/Cargo.toml --lib vendor_codex_real_machine -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn vendor_codex_real_machine_readonly_smoke() {
        let home = crate::creds::home().expect("用户目录");
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let dirs = vendor_bin_dirs("codex", &home, local.as_deref());
        let found = find_bin_in("codex", &dirs);
        println!("固定落点：{dirs:#?}\n找到：{found:?}");
        println!("完整查找（含 PATH / npm）：{:?}", find_bin("codex", &home));
        assert!(found.is_some(), "本机没在固定落点找到 codex");
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
