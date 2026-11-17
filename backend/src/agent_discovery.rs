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
//! 目录证据要「有内容」（`agent_dir_has_content`，magpie `agentDir`）：只剩 `logs` / `cache`
//! 一类卸载残留、`npx skills` 建的 `skills`、登录工具写的登录文件（`Spec.sign_in`）都不算。
//!
//! 例外规则走 `Spec.detect`（`Detect` 枚举）：Gemini CLI 的 `~/.gemini` 也是 Antigravity 的，
//! 只认命令行（`CliOnly`）。magpie 的 dirShared / sign / pathOwn 在现有 13 家里没有使用者，
//! 没有加；以后新增共用目录的 agent 时再加。
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
    /// 登录工具（Jolu 的「应用到原工具」、magpie 等）会替没装过的 agent 在 dir 里写的登录文件：
    /// 目录里只有它不算装了。空串表示没有。magpie `Agent.signIn` 同款
    /// （Codex `auth.json`，Claude Code `.credentials.json`）。
    sign_in: &'static str,
    /// 判定规则（T4 例外钩子）。绝大多数是 `Standard`。
    detect: Detect,
    /// 找到的命令不是这家工具、只是同名别的程序时返回 true，当作没找到（T8，magpie `binNot`）。
    /// 只给已知会撞名的工具设，不做全量二进制扫描。
    bin_not: Option<fn(&Path) -> bool>,
}

/// Go 构建信息的魔数：Go 程序的 build info 以它开头（magpie `goBuildInfoMagic`，`debug/buildinfo` 找的就是它）
const GO_BUILD_INFO_MAGIC: &[u8] = b"\xff Go buildinf:";

/// 这个可执行文件是不是 Go 写的（magpie `agent.go::goProgram`）。用途：Block 的 Goose 是 Rust 写的，
/// PATH 上的 Go 版 `goose` 是 pressly 的数据库迁移工具。
///
/// 用 256 KB 缓冲顺序找魔数（magpie `hasGoMagic`：Block 的 goose 有 260 MB，整个解析符号表要分配
/// 200 MB），结果按「路径 + 大小 + 修改时间」缓存。magpie 找到魔数后还会用 `debug/buildinfo`
/// 再解析确认，Rust 没有对应的库，这里只认魔数；读不了的文件不算 Go 程序。
fn go_program(bin: &Path) -> bool {
    type Key = (PathBuf, u64, Option<std::time::SystemTime>);
    static SEEN: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<Key, bool>>> = std::sync::LazyLock::new(Default::default);
    let Ok(meta) = std::fs::metadata(bin) else {
        return false;
    };
    let key: Key = (bin.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(known) = SEEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&key) {
        return *known;
    }
    let is = has_go_magic(bin);
    SEEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(key, is);
    is
}

/// 顺序读文件找 Go 魔数；跨两次读取的魔数靠保留上一段末尾 `len - 1` 个字节接上。
fn has_go_magic(bin: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(bin) else {
        return false;
    };
    let keep = GO_BUILD_INFO_MAGIC.len() - 1;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut kept = 0;
    loop {
        let read = match file.read(&mut buffer[kept..]) {
            Ok(read) => read,
            Err(_) => return false,
        };
        let filled = kept + read;
        if buffer[..filled].windows(GO_BUILD_INFO_MAGIC.len()).any(|window| window == GO_BUILD_INFO_MAGIC) {
            return true;
        }
        if read == 0 {
            return false;
        }
        kept = keep.min(filled);
        buffer.copy_within(filled - kept..filled, 0);
    }
}

/// 配置目录或它的某一级上级被**文件**占着（magpie `agent.go::Taken`）：那里什么都写不进去，
/// 这家工具不可能在用这个目录。从目录本身往上找第一个存在的路径：是文件 → true，是目录 → false。
fn path_taken(path: &Path) -> bool {
    let mut current = path.to_path_buf();
    loop {
        if let Ok(meta) = std::fs::metadata(&current) {
            return !meta.is_dir();
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => return false,
        }
    }
}

/// 一家 agent 怎么算「装了」（magpie `Agent.detect` 的 Jolu 版）。
///
/// 只放有真实使用者的档位：以后真有工具需要完全接管判定时，再加
/// `Custom(fn(&Facts) -> Verdict)` 一类变体（钩子只看探测已收集的事实、不自己读盘，
/// 保持 `discover_with` 可注入测试），不要提前铺。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Detect {
    /// 配置文件 → 配置目录 → 命令，命中任一即算（`classify`）
    Standard,
    /// 只认命令行：配置目录也被别的程序用、或卸载后会留下，目录和文件都不算证据。
    /// 命令不在 → not_found（不落到「CLI 未找到」）；命令在 → installed，证据记 `bin`。
    /// 用例：Gemini CLI 的 `~/.gemini` 也是 Antigravity / Gemini Code Assist 的
    /// （magpie `internal/agent/gemini.go` 的 `detect`）。
    CliOnly,
}

/// agent 运行时写、卸载后留下的目录（magpie `agent.go::leftover`）：只剩这些不算装了。
/// 同名的**文件**照算内容。
const LEFTOVER_DIRS: [&str; 5] = ["logs", "log", "debug", "diagnostics", "cache"];

/// 配置目录是不是 agent 自己建的、还有内容（magpie `agent.go::agentDir` 同款）：
/// - 目录不存在 → false；存在但读不了（权限）→ true，不误判成没装
/// - 空目录 → true（agent 可能刚建目录还没写文件）
/// - 只剩 `skills`（`npx skills` 一类会给每家都建）、`.DS_Store`、残留目录、`sign_in` 登录文件 → false
/// - 有任何别的条目 → true
fn agent_dir_has_content(path: &Path, sign_in: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return path.is_dir();
    };
    let mut empty = true;
    for entry in entries.flatten() {
        empty = false;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        let ignored = name == "skills"
            || name == ".DS_Store"
            || (is_dir && LEFTOVER_DIRS.contains(&name.to_ascii_lowercase().as_str()))
            || (!is_dir && !sign_in.is_empty() && name == sign_in);
        if !ignored {
            return true;
        }
    }
    empty
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
    // magpie kimi.go `kimiDir`：新版 Kimi Code（TypeScript 2.x）读 `~/.kimi-code`
    // （`KIMI_CODE_HOME` 覆盖）；旧版 Python kimi-cli 读 `~/.kimi`（`KIMI_SHARE_DIR` 覆盖）。
    // 新版目录存在时旧的不再被读（kimi-cli 1.50 起移交并只认新版，magpie #290），
    // 所以「新版目录存在」的优先级高于旧版的环境变量。
    if let Some(dir) = std::env::var_os("KIMI_CODE_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let code = home.join(".kimi-code");
    if code.is_dir() {
        return Some(code);
    }
    if let Some(dir) = std::env::var_os("KIMI_SHARE_DIR").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(dir));
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

/// Crush：Windows 上配置在 `%LOCALAPPDATA%\crush`（magpie crush()），其余系统 `<XDG_CONFIG_HOME 或 ~/.config>/crush`；
/// 与接线（`gateway::wiring_extra::crush_dir`）共用一处判定
fn dir_crush(home: &Path) -> Option<PathBuf> {
    Some(crate::gateway::wiring_extra::crush_dir(home))
}

fn dir_goose(home: &Path) -> Option<PathBuf> {
    // magpie agents.go::gooseIn：Windows 上 Goose 的配置在 `%APPDATA%\Block\goose\config`，
    // 其余系统在 `<XDG_CONFIG_HOME 或 ~/.config>/goose`
    #[cfg(windows)]
    {
        if let Some(roaming) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
            return Some(PathBuf::from(roaming).join("Block").join("goose").join("config"));
        }
    }
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    Some(xdg.join("goose"))
}

fn dir_continue(home: &Path) -> Option<PathBuf> {
    // magpie 里没有 Continue；依据官方文档（docs.continue.dev/cli/configuration，2026-11-16 经 Context7 核对）：
    // CLI 命令是 `cn`，默认读 `~/.continue/config.yaml`（Windows 为 `%USERPROFILE%\.continue`），
    // 目录与 VS Code / JetBrains 插件共用。旧版的 `config.json` 已迁移到 YAML，两者都在时读 YAML；
    // 只有 config.json 的旧用户目录有内容，照样算目录证据。
    Some(home.join(".continue"))
}

// ---------- 2026-11-16 探测对齐 magpie · 批 1：本机实测存在但 Jolu 缺的 4 家 ----------

/// Pencil（pen.dev）：自定义 provider 在 `~/.pencil/models.json`（Pi 格式）。
/// magpie 另有 Mac 应用包判定；Windows 上只看目录。无 CLI（pencil/pen 是常见名，不认）。
fn dir_pencil(home: &Path) -> Option<PathBuf> {
    Some(home.join(".pencil"))
}

/// DeepSeek Harness：`DSH_HOME` 覆盖，缺省 `~/.dsh`。新版按 profile 存
/// `profiles/<name>/cordis.patch.yml`，旧版 `config.yaml`——目录存在即算。
fn dir_dsh(home: &Path) -> Option<PathBuf> {
    std::env::var_os("DSH_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(home.join(".dsh")))
}

/// Copilot CLI：`~/.copilot/settings.json`。JetBrains / VS Code 的 Copilot 插件也写
/// 这个目录（agents、skills、hooks），CLI 不在时仍可能是插件在用。
fn dir_copilot(home: &Path) -> Option<PathBuf> {
    Some(home.join(".copilot"))
}

/// VS Code 的 User 目录：Windows 为 `<home>\AppData\Roaming\Code\User`（即 %APPDATA%），
/// 用 home 推导而非读环境变量，测试注入临时 home 时不会命中真实系统。
/// 内置 Copilot Chat 的自定义 provider 在 `chatLanguageModels.json`，探测用 `settings.json`。
fn dir_vscode(home: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let dir = home.join("AppData").join("Roaming").join("Code").join("User");
        if dir.is_dir() {
            return Some(dir);
        }
    }
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    Some(xdg.join("Code").join("User"))
}

// ---------- 常用 20 家补充（2026-11-16 收敛）：只保留高频工具的探测 ----------

fn env_or_home(env: &str, home: &Path, dot: &str) -> Option<PathBuf> {
    Some(
        std::env::var_os(env)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(dot)),
    )
}

fn dir_qoder(home: &Path) -> Option<PathBuf> { env_or_home("QODER_CONFIG_DIR", home, ".qoder") }
fn dir_qoder_cn(home: &Path) -> Option<PathBuf> { env_or_home("QODERCN_CONFIG_DIR", home, ".qoder-cn") }
fn dir_workbuddy(home: &Path) -> Option<PathBuf> { env_or_home("WORKBUDDY_CONFIG_DIR", home, ".workbuddy") }

const SPECS: [Spec; 20] = [
    Spec { agent: "claude-code", name: "Claude Code", bin: "claude", dir: dir_claude, file: "settings.json", sign_in: ".credentials.json", detect: Detect::Standard, bin_not: None },
    Spec { agent: "codex", name: "Codex", bin: "codex", dir: dir_codex, file: "config.toml", sign_in: "auth.json", detect: Detect::Standard, bin_not: None },
    Spec { agent: "zcode", name: "ZCode", bin: "zcode", dir: dir_zcode, file: "v2/provider_config.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "gemini", name: "Gemini CLI", bin: "gemini", dir: dir_gemini, file: ".env", sign_in: "", detect: Detect::CliOnly, bin_not: None },
    Spec { agent: "opencode", name: "OpenCode", bin: "opencode", dir: dir_opencode, file: "opencode.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "qwen", name: "Qwen Code", bin: "qwen", dir: dir_qwen, file: "settings.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "kimi", name: "Kimi CLI", bin: "kimi", dir: dir_kimi, file: "config.toml", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "cline", name: "Cline", bin: "cline", dir: dir_cline, file: "settings/providers.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "droid", name: "Droid", bin: "droid", dir: dir_droid, file: "settings.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "zed", name: "Zed", bin: "zed", dir: dir_zed, file: "settings.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "crush", name: "Crush", bin: "crush", dir: dir_crush, file: "crush.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "goose", name: "Goose", bin: "goose", dir: dir_goose, file: "config.yaml", sign_in: "", detect: Detect::Standard, bin_not: Some(go_program) },
    Spec { agent: "continue", name: "Continue", bin: "cn", dir: dir_continue, file: "config.yaml", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "pencil", name: "Pencil", bin: "", dir: dir_pencil, file: "models.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "dsh", name: "DeepSeek Harness", bin: "dsh", dir: dir_dsh, file: "config.yaml", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "copilot", name: "Copilot CLI", bin: "copilot", dir: dir_copilot, file: "settings.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "vscode", name: "VS Code", bin: "", dir: dir_vscode, file: "settings.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "qoder", name: "Qoder", bin: "qodercli", dir: dir_qoder, file: "settings.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "qoder-cn", name: "Qoder CN", bin: "qoderclicn", dir: dir_qoder_cn, file: "settings.json", sign_in: "", detect: Detect::Standard, bin_not: None },
    Spec { agent: "workbuddy", name: "WorkBuddy", bin: "", dir: dir_workbuddy, file: "models.json", sign_in: "", detect: Detect::Standard, bin_not: None },
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

// ---------- T5：命令查找缓存（magpie `internal/proc/look.go` 同款） ----------
//
// 贵的是「找不到」：每个命令要在 PATH 的几十个目录 × PATHEXT 的每个扩展名里各 stat 一次
// （2026-11-16 本机 release 实测约 10–12ms 一个，13 家里 9 家没装，一次探测约 100ms）。
// 便宜的是「目录戳」：每个查找目录 stat 一次。答案按戳缓存：戳不变就直接给；装上或卸掉一个
// 工具会改所在目录的修改时间，戳跟着变，答案自动作废。2 秒窗口只是「多久重新取一次戳」。
// 注册表 PATH 只在取戳时读一次（0.09ms），未命中的查找复用这份目录清单，不再各读一遍（T6）。

/// 多久重新取一次目录戳（magpie `lookFresh`）
const LOOK_FRESH: std::time::Duration = std::time::Duration::from_secs(2);
/// 目录在这段时间内刚变过：时钟粒度粗，同一刻的下一次变化可能不改时间，戳不可信、答案不留
/// （magpie `lookSettle`）
const LOOK_SETTLE: std::time::Duration = std::time::Duration::from_millis(100);

/// 一次取戳：戳文本、顺手算好的兜底目录（通用兜底 + 注册表 PATH，供未命中的查找复用）、是否稳定
struct Stamp {
    text: String,
    fallback: Vec<PathBuf>,
    settled: bool,
}

#[derive(Default)]
struct LookCache {
    /// 取戳时的环境（PATH、PATHEXT、几个目录变量、home）：变了立即重取
    env: String,
    /// 空串 = 当前没有可信的戳，什么都不留
    stamp: String,
    fallback: Vec<PathBuf>,
    checked: Option<std::time::Instant>,
    /// 每次 forget 加一：跨过 forget 问到的答案不留
    generation: u64,
    answers: std::collections::HashMap<String, Option<PathBuf>>,
}

impl LookCache {
    fn forget(&mut self) {
        self.answers.clear();
        self.stamp.clear();
        self.fallback.clear();
        self.checked = None;
        self.generation += 1;
    }
}

static LOOKS: std::sync::LazyLock<std::sync::Mutex<LookCache>> = std::sync::LazyLock::new(Default::default);

fn lock_looks(cache: &std::sync::Mutex<LookCache>) -> std::sync::MutexGuard<'_, LookCache> {
    cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 缓存核心（纯逻辑，时间、取戳、查找都由调用方给，测试直接覆盖）。
/// 照 magpie `looked`：取戳和查找都不在锁里做；跨过 forget、戳变了或戳不稳时问到的答案不留。
fn looked(
    cache: &std::sync::Mutex<LookCache>,
    key: &str,
    env: &str,
    now: std::time::Instant,
    take_stamp: impl FnOnce() -> Stamp,
    ask: impl FnOnce(&[PathBuf]) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let mut state = lock_looks(cache);
    let stale = state.env != env || state.checked.is_none_or(|checked| now.saturating_duration_since(checked) >= LOOK_FRESH);
    if stale {
        drop(state);
        let stamp = take_stamp();
        state = lock_looks(cache);
        let text = if stamp.settled { stamp.text } else { String::new() };
        // 环境变了（PATHEXT 不在戳文本里）、戳变了、戳不稳：之前的答案都不算数
        if text != state.stamp || text.is_empty() || state.env != env {
            state.answers.clear();
            state.stamp = text;
        }
        state.fallback = stamp.fallback;
        state.env = env.to_string();
        state.checked = stamp.settled.then_some(now);
    }
    if let Some(answer) = state.answers.get(key) {
        return answer.clone();
    }
    let (generation, stamp, fallback) = (state.generation, state.stamp.clone(), state.fallback.clone());
    drop(state);
    let answer = ask(&fallback);
    let mut state = lock_looks(cache);
    if state.generation == generation && state.stamp == stamp && !stamp.is_empty() {
        state.answers.insert(key.to_string(), answer.clone());
    }
    answer
}

/// 目录戳：每个目录「路径@修改时间」拼起来；不存在的只记路径（出现即变）。
/// 有目录在 `LOOK_SETTLE` 内刚改过 → 不稳定。修改时间在将来的照算稳定（下一次变化会把它拨回现在）。
fn stamp_of(dirs: &[PathBuf], now: std::time::SystemTime) -> (String, bool) {
    let mut text = String::new();
    let mut settled = true;
    for dir in dirs {
        text.push('\0');
        text.push_str(&dir.to_string_lossy());
        if let Ok(modified) = std::fs::metadata(dir).and_then(|meta| meta.modified()) {
            let nanos = modified.duration_since(std::time::UNIX_EPOCH).map(|age| age.as_nanos()).unwrap_or_default();
            text.push('@');
            text.push_str(&nanos.to_string());
            if now.duration_since(modified).is_ok_and(|age| age < LOOK_SETTLE) {
                settled = false;
            }
        }
    }
    (text, settled)
}

/// 取戳时的环境：这些变了，查找目录就变了
fn look_env(home: &Path) -> String {
    ["PATH", "PATHEXT", "APPDATA", "LOCALAPPDATA"]
        .iter()
        .map(|name| std::env::var_os(name).map(|value| value.to_string_lossy().into_owned()).unwrap_or_default())
        .chain(std::iter::once(home.to_string_lossy().into_owned()))
        .collect::<Vec<_>>()
        .join("\0")
}

fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

/// 真机取戳：进程 PATH、通用兜底、注册表 PATH（只读这一次）、各家固定落点及桌面版哈希目录的根
fn take_stamp(home: &Path) -> Stamp {
    let mut fallback = user_bin_dirs(home);
    #[cfg(windows)]
    fallback.extend(registry_path_dirs());
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default();
    dirs.extend(fallback.iter().cloned());
    let local = local_app_data();
    for spec in &SPECS {
        dirs.extend(vendor_bin_dirs(spec.bin, home, local.as_deref()));
    }
    if let Some(local) = &local {
        // 桌面版升级会在这里新建哈希子目录，根目录的修改时间跟着变
        dirs.push(local.join("OpenAI").join("Codex").join("bin"));
    }
    let (text, settled) = stamp_of(&dirs, std::time::SystemTime::now());
    Stamp { text, fallback, settled }
}

/// 丢掉全部缓存的查找结果，下一次重新找（「重新检测」用，magpie `Forget`）。
pub(crate) fn forget_bin_lookups() {
    lock_looks(&LOOKS).forget();
}

/// 命令查找：进程 PATH 失败后，依次在 Windows 兜底目录、注册表 PATH、各家固定落点里找
/// `name.exe` / `name.cmd` / `name.bat`。固定落点放最后：它是兜底，PATH 上有的优先。
/// 结果按目录戳缓存（见上方 T5 说明）；「找不到」同样缓存。
/// 非 Windows 只有进程 PATH（magpie 的 binAt 在非 Windows 也只补 Homebrew，这里没有）。
pub(crate) fn find_bin(name: &str, home: &Path) -> Option<PathBuf> {
    let key = format!("{name}\0{}", home.to_string_lossy());
    looked(&LOOKS, &key, &look_env(home), std::time::Instant::now(), || take_stamp(home), |fallback| find_bin_fresh(name, home, fallback))
}

/// 不经缓存的一次查找；`fallback` 是取戳时算好的通用兜底 + 注册表 PATH。
fn find_bin_fresh(name: &str, home: &Path, fallback: &[PathBuf]) -> Option<PathBuf> {
    if let Some(found) = bin_on_path(name) {
        return Some(found);
    }
    let mut dirs = fallback.to_vec();
    dirs.extend(vendor_bin_dirs(name, home, local_app_data().as_deref()));
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
            // 只有卸载残留 / 别人建的 skills / 登录文件的目录不算证据（T3）
            let dir_found = dir.as_deref().is_some_and(|path| agent_dir_has_content(path, spec.sign_in));
            let file_found = !spec.file.is_empty()
                && dir.as_deref()
                    .map(|base| base.join(spec.file))
                    .is_some_and(|path| path.is_file());
            // 同名的别的程序（例如 Go 写的 goose）当作没找到（T8）
            // 无 CLI 的桌面应用（Pencil / VS Code）不找命令，避免空名扫描
            let bin_path = if spec.bin.is_empty() {
                None
            } else {
                bin_lookup(spec.bin, home).filter(|path| !spec.bin_not.is_some_and(|not_ours| not_ours(path)))
            };
            let bin_found = bin_path.is_some();
            // 配置目录被文件占着：这家工具不可能在用它，直接判没装（T9，magpie `Detected` 第一步）
            let taken = dir.as_deref().is_some_and(path_taken);
            let (installed, state, evidence) = match spec.detect {
                _ if taken => (false, "not_found", None),
                Detect::Standard => classify(file_found, dir_found, bin_found),
                // 目录和文件都不算证据：等于只把命令交给 classify
                Detect::CliOnly => classify(false, false, bin_found),
            };
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

    /// 批 1（2026-11-16）：Pencil / dsh / Copilot / VS Code 按配置目录探测；
    /// Pencil / VS Code 无 CLI 属正常，配置在即已装。
    #[test]
    fn batch1_pencil_dsh_copilot_vscode_found_by_config_dirs() {
        let home = temp_home("batch1");
        std::fs::create_dir_all(home.join(".pencil")).unwrap();
        std::fs::write(home.join(".pencil").join("models.json"), "{}").unwrap();
        std::fs::create_dir_all(home.join(".dsh").join("profiles").join("default")).unwrap();
        std::fs::create_dir_all(home.join(".copilot")).unwrap();
        std::fs::write(home.join(".copilot").join("settings.json"), "{}").unwrap();
        let vs_user = home.join("AppData").join("Roaming").join("Code").join("User");
        std::fs::create_dir_all(&vs_user).unwrap();
        std::fs::write(vs_user.join("settings.json"), "{}").unwrap();
        let lookup = test_bins(&home);
        let found = discover_with(&home, &lookup);
        for agent in ["pencil", "dsh", "copilot", "vscode"] {
            let d = agent_of(&found, agent);
            assert!(d.installed, "{agent} 应判为已装");
        }
        // 无 CLI 的两家：证据是配置，不是命令
        assert!(!agent_of(&found, "pencil").cli_found);
        assert!(!agent_of(&found, "vscode").cli_found);
        let _ = std::fs::remove_dir_all(&home);
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

    // ---------- T5 / T6：命令查找缓存 ----------

    use std::cell::Cell;
    use std::time::{Duration, Instant, SystemTime};

    fn stamp(text: &str) -> Stamp {
        Stamp { text: text.into(), fallback: vec![PathBuf::from("fallback")], settled: true }
    }

    /// 13 家一轮查找：戳只取一次（注册表 PATH 跟着只读一次），找不到也缓存；
    /// 过了窗口重取戳、戳没变 → 一次查找都不重做
    #[test]
    fn lookups_share_one_stamp_and_cache_misses() {
        let cache = std::sync::Mutex::new(LookCache::default());
        let (stamps, asks) = (Cell::new(0), Cell::new(0));
        let start = Instant::now();
        // 无 CLI 的桌面应用（bin 为空）在生产里不查找，测试 likewise 跳过
        let wirable: Vec<&Spec> = SPECS.iter().filter(|s| !s.bin.is_empty()).collect();
        let round = |now: Instant| {
            for spec in &wirable {
                let found = looked(&cache, spec.bin, "env", now, || { stamps.set(stamps.get() + 1); stamp("s1") }, |fallback| {
                    asks.set(asks.get() + 1);
                    assert_eq!(fallback, [PathBuf::from("fallback")], "未命中的查找复用取戳时的兜底目录");
                    (spec.bin == "claude").then(|| PathBuf::from("claude.exe"))
                });
                assert_eq!(found.is_some(), spec.bin == "claude");
            }
        };
        round(start);
        assert_eq!((stamps.get(), asks.get()), (1, wirable.len()), "第一轮：取戳 1 次，有 CLI 的家各找 1 次");
        round(start + Duration::from_millis(500));
        assert_eq!((stamps.get(), asks.get()), (1, wirable.len()), "窗口内：不取戳、不查找");
        round(start + LOOK_FRESH);
        assert_eq!((stamps.get(), asks.get()), (2, wirable.len()), "过了窗口：重取一次戳，戳没变就不重找");
    }

    /// 目录变了（戳变）、PATH 等环境变了、手动 forget → 答案作废，重新查找
    #[test]
    fn stamp_env_change_and_forget_invalidate_answers() {
        let cache = std::sync::Mutex::new(LookCache::default());
        let asks = Cell::new(0);
        let start = Instant::now();
        let ask = |_: &[PathBuf]| { asks.set(asks.get() + 1); None };
        looked(&cache, "codex", "env", start, || stamp("s1"), ask);
        looked(&cache, "codex", "env", start + LOOK_FRESH, || stamp("s2"), ask);
        assert_eq!(asks.get(), 2, "装了 / 卸了工具（目录修改时间变了）要重找");
        looked(&cache, "codex", "env-with-new-path", start + LOOK_FRESH, || stamp("s2"), ask);
        assert_eq!(asks.get(), 3, "PATH 变了立即重找，不等窗口");
        lock_looks(&cache).forget();
        looked(&cache, "codex", "env-with-new-path", start + LOOK_FRESH, || stamp("s2"), ask);
        assert_eq!(asks.get(), 4, "「重新检测」forget 后立即重找");
        looked(&cache, "codex", "env-with-new-path", start + LOOK_FRESH, || stamp("s2"), ask);
        assert_eq!(asks.get(), 4, "forget 之后的答案照常缓存");
    }

    /// 戳不稳（目录刚改过）→ 什么都不留，下一次立即重取
    #[test]
    fn unsettled_stamp_keeps_nothing() {
        let cache = std::sync::Mutex::new(LookCache::default());
        let (stamps, asks) = (Cell::new(0), Cell::new(0));
        let now = Instant::now();
        for _ in 0..2 {
            looked(&cache, "codex", "env", now, || {
                stamps.set(stamps.get() + 1);
                Stamp { text: "s1".into(), fallback: Vec::new(), settled: false }
            }, |_| { asks.set(asks.get() + 1); None });
        }
        assert_eq!((stamps.get(), asks.get()), (2, 2));
    }

    /// 目录戳：刚建的目录不稳；过了 LOOK_SETTLE 稳定；不存在的目录只记路径，建出来后戳会变
    #[test]
    fn stamp_of_marks_recent_changes_unsettled() {
        let home = temp_home("stamp");
        let dir = home.join("bin");
        let missing = home.join("later");
        std::fs::create_dir_all(&dir).unwrap();
        let (_, settled_now) = stamp_of(&[dir.clone()], SystemTime::now());
        assert!(!settled_now, "刚改过的目录戳不可信");
        let (before, settled_later) = stamp_of(&[dir.clone(), missing.clone()], SystemTime::now() + Duration::from_secs(1));
        assert!(settled_later);
        std::fs::create_dir_all(&missing).unwrap();
        let (after, _) = stamp_of(&[dir.clone(), missing.clone()], SystemTime::now() + Duration::from_secs(1));
        assert_ne!(before, after, "目录出现后戳要变");
        std::fs::remove_dir_all(&home).ok();
    }

    // ---------- T8：同名程序甄别 ----------

    /// Go 魔数在开头、横跨 256 KB 读缓冲的边界、完全没有、文件不存在
    #[test]
    fn go_magic_is_found_across_buffer_boundary() {
        let home = temp_home("go-magic");
        let write = |name: &str, bytes: Vec<u8>| {
            let path = home.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        let mut head = b"MZ".to_vec();
        head.extend_from_slice(GO_BUILD_INFO_MAGIC);
        assert!(has_go_magic(&write("head.exe", head)));
        let mut straddle = vec![0u8; 256 * 1024 - 5];
        straddle.extend_from_slice(GO_BUILD_INFO_MAGIC);
        straddle.extend_from_slice(&[0u8; 64]);
        assert!(has_go_magic(&write("straddle.exe", straddle)), "魔数横跨两次读取也要找到");
        assert!(!has_go_magic(&write("rust.exe", vec![0x4d; 600 * 1024])), "没有魔数的大文件不是 Go 程序");
        assert!(!has_go_magic(&home.join("missing.exe")));
        std::fs::remove_dir_all(&home).ok();
    }

    /// Goose：PATH 上的 goose 是 Go 写的（pressly 的迁移工具）→ 当作没找到命令；
    /// 不是 Go 写的 → 照常找到。其他家不受 bin_not 影响
    #[test]
    fn go_goose_on_path_is_not_block_goose() {
        let home = temp_home("go-goose");
        std::fs::create_dir_all(home.join("bins")).unwrap();
        let mut go = b"MZ".to_vec();
        go.extend_from_slice(GO_BUILD_INFO_MAGIC);
        std::fs::write(home.join("bins").join("goose.exe"), &go).unwrap();
        std::fs::write(home.join("bins").join("qwen.exe"), &go).unwrap();
        let results = discover_with(&home, &test_bins(&home));
        let goose = agent_of(&results, "goose");
        assert!(!goose.cli_found, "Go 写的 goose 不是 Block 的 Goose");
        assert_eq!(goose.cli_path, None);
        assert!(agent_of(&results, "qwen").cli_found, "没设 bin_not 的工具不做甄别");
        std::fs::write(home.join("bins").join("goose.exe"), b"MZ rust goose").unwrap();
        assert!(agent_of(&discover_with(&home, &test_bins(&home)), "goose").cli_found);
        std::fs::remove_dir_all(&home).ok();
    }

    /// 真机只读核对（默认不跑）：用本机真实二进制核对 Go 魔数判定，并打印耗时。
    /// Go 样本：GitHub CLI 的 gh.exe；Rust 样本：Codex 桌面版自带的 rg.exe（ripgrep）。
    /// `cargo test --manifest-path backend/Cargo.toml --release --lib go_magic_real_binaries -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn go_magic_real_binaries_readonly_smoke() {
        let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from).expect("ProgramFiles");
        let local = local_app_data().expect("LOCALAPPDATA");
        let samples = [
            (program_files.join("GitHub CLI").join("gh.exe"), true),
            (local.join("OpenAI").join("Codex").join("bin").join("a7d4c4900eafc659").join("rg.exe"), false),
        ];
        for (path, expect_go) in samples {
            if !path.is_file() {
                println!("跳过（本机没有）：{}", path.file_name().unwrap().to_string_lossy());
                continue;
            }
            let size = std::fs::metadata(&path).unwrap().len();
            let started = std::time::Instant::now();
            let is_go = go_program(&path);
            let first = started.elapsed();
            let started = std::time::Instant::now();
            go_program(&path);
            println!("{} {} MB：go={is_go} 首次 {first:?}，缓存 {:?}", path.file_name().unwrap().to_string_lossy(), size / 1024 / 1024, started.elapsed());
            assert_eq!(is_go, expect_go);
        }
    }

    /// 13 行里只有 Goose 设了 bin_not
    #[test]
    fn only_goose_has_bin_not() {
        for spec in &SPECS {
            assert_eq!(spec.bin_not.is_some(), spec.agent == "goose", "{}", spec.agent);
        }
    }

    /// Goose 在 Windows 上的配置目录是 `%APPDATA%\Block\goose\config`（magpie gooseIn）
    #[cfg(windows)]
    #[test]
    fn goose_dir_on_windows_is_under_appdata_block() {
        if let Some(roaming) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
            assert_eq!(dir_goose(Path::new("H")), Some(PathBuf::from(roaming).join("Block").join("goose").join("config")));
        }
    }

    /// Continue：现行配置 `~/.continue/config.yaml` 算配置文件证据；只有旧版 `config.json`
    /// 的用户目录有内容，算目录证据，不会漏掉
    #[test]
    fn continue_uses_config_yaml_and_keeps_legacy_json_users() {
        let home = temp_home("continue");
        std::fs::create_dir_all(home.join(".continue")).unwrap();
        std::fs::write(home.join(".continue").join("config.json"), b"{}").unwrap();
        let legacy = agent_of(&discover_with(&home, &test_bins(&home)), "continue").clone();
        assert_eq!((legacy.state, legacy.evidence), ("cli_missing", Some("config_dir")));
        std::fs::write(home.join(".continue").join("config.yaml"), b"name: local\n").unwrap();
        std::fs::create_dir_all(home.join("bins")).unwrap();
        std::fs::write(home.join("bins").join("cn.exe"), b"MZ").unwrap();
        let current = agent_of(&discover_with(&home, &test_bins(&home)), "continue").clone();
        assert_eq!((current.state, current.evidence), ("installed", Some("config_file")));
        std::fs::remove_dir_all(&home).ok();
    }

    // ---------- T9：路径占用 ----------

    /// 目录本身或某一级上级是文件 → 被占；目录存在或一路都不存在（直到一个真实目录）→ 没被占
    #[test]
    fn path_taken_checks_self_and_ancestors() {
        let home = temp_home("taken");
        std::fs::write(home.join(".qwen"), b"").unwrap();
        assert!(path_taken(&home.join(".qwen")), "配置目录的位置是个文件");
        std::fs::write(home.join(".config"), b"").unwrap();
        assert!(path_taken(&home.join(".config").join("crush")), "上级是文件，下面建不了目录");
        std::fs::create_dir_all(home.join(".kimi")).unwrap();
        assert!(!path_taken(&home.join(".kimi")));
        assert!(!path_taken(&home.join("nothing").join("here")), "一路都不存在，上到 home 是目录");
        std::fs::remove_dir_all(&home).ok();
    }

    /// 配置目录被同名文件挡住 → not_found，命令在也一样（Standard 与 CliOnly 都适用）；
    /// cli_found 仍如实反映命令找没找到
    #[test]
    fn taken_config_dir_is_not_found() {
        let home = temp_home("taken-discover");
        std::fs::write(home.join(".qwen"), b"").unwrap();
        std::fs::write(home.join(".gemini"), b"").unwrap();
        std::fs::create_dir_all(home.join("bins")).unwrap();
        std::fs::write(home.join("bins").join("qwen.exe"), b"MZ").unwrap();
        std::fs::write(home.join("bins").join("gemini.exe"), b"MZ").unwrap();
        let results = discover_with(&home, &test_bins(&home));
        for agent in ["qwen", "gemini"] {
            let found = agent_of(&results, agent);
            assert_eq!(found.state, "not_found", "{agent} 的配置目录被文件占着");
            assert!(!found.installed);
            assert!(found.cli_found, "{agent} 的命令照实报告");
        }
        std::fs::remove_dir_all(&home).ok();
    }

    /// T4：Gemini 只认命令行。`~/.gemini` 有 Antigravity 的东西和 settings.json、
    /// 甚至有 `.env`，但 `gemini` 不在 → not_found（原先会判 cli_missing）；命令在 → installed，证据记 bin
    #[test]
    fn gemini_is_detected_by_cli_only() {
        let home = temp_home("gemini-cli-only");
        let gemini = home.join(".gemini");
        std::fs::create_dir_all(gemini.join("antigravity")).unwrap();
        std::fs::write(gemini.join("settings.json"), b"{}").unwrap();
        std::fs::write(gemini.join(".env"), b"").unwrap();
        let without = agent_of(&discover_with(&home, &test_bins(&home)), "gemini").clone();
        assert_eq!(without.state, "not_found");
        assert!(!without.installed);
        assert_eq!(without.evidence, None);
        std::fs::create_dir_all(home.join("bins")).unwrap();
        std::fs::write(home.join("bins").join("gemini.exe"), b"MZ").unwrap();
        let with = agent_of(&discover_with(&home, &test_bins(&home)), "gemini").clone();
        assert_eq!(with.state, "installed");
        assert_eq!(with.evidence, Some("bin"));
        std::fs::remove_dir_all(&home).ok();
    }

    /// T4：只有 Gemini 是 CliOnly，其他 12 家都按 Standard（行为不变）
    #[test]
    fn only_gemini_uses_cli_only() {
        for spec in &SPECS {
            let expected = if spec.agent == "gemini" { Detect::CliOnly } else { Detect::Standard };
            assert_eq!(spec.detect, expected, "{}", spec.agent);
        }
        // 其他家：配置残留没有命令照旧 cli_missing
        let home = temp_home("standard-unchanged");
        std::fs::create_dir_all(home.join(".qwen")).unwrap();
        std::fs::write(home.join(".qwen").join("settings.json"), b"{}").unwrap();
        assert_eq!(agent_of(&discover_with(&home, &test_bins(&home)), "qwen").state, "cli_missing");
        std::fs::remove_dir_all(&home).ok();
    }

    /// T3：目录内容甄别（magpie `agentDir` 同款）
    #[test]
    fn agent_dir_content_ignores_leftovers_skills_and_sign_in() {
        let home = temp_home("dir-content");
        let case = |tag: &str| {
            let dir = home.join(tag);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        };
        // 不存在 → 没装
        assert!(!agent_dir_has_content(&home.join("missing"), ""));
        // 空目录 → 算装了（刚建目录还没写文件）
        assert!(agent_dir_has_content(&case("empty"), ""));
        // 只有 skills（npx skills 给每家都建）→ 不算
        let skills = case("skills-only");
        std::fs::create_dir_all(skills.join("skills")).unwrap();
        assert!(!agent_dir_has_content(&skills, ""));
        // 只有运行时残留目录（大小写不敏感）→ 不算
        let leftover = case("leftover");
        for name in ["logs", "Cache", "debug", "diagnostics", "log"] {
            std::fs::create_dir_all(leftover.join(name)).unwrap();
        }
        std::fs::write(leftover.join(".DS_Store"), b"").unwrap();
        assert!(!agent_dir_has_content(&leftover, ""));
        // 同名的是文件而不是目录 → 照算内容
        let file_named = case("file-named-logs");
        std::fs::write(file_named.join("logs"), b"").unwrap();
        assert!(agent_dir_has_content(&file_named, ""));
        // 只有登录工具写的登录文件 → 不算；换个 agent（没有 sign_in）就算
        let signed = case("sign-in-only");
        std::fs::write(signed.join("auth.json"), b"{}").unwrap();
        assert!(!agent_dir_has_content(&signed, "auth.json"));
        assert!(agent_dir_has_content(&signed, ""));
        // 有配置文件或任何未知条目 → 算
        let real = case("real");
        std::fs::create_dir_all(real.join("logs")).unwrap();
        std::fs::write(real.join("settings.json"), b"{}").unwrap();
        assert!(agent_dir_has_content(&real, ""));
        let unknown = case("unknown");
        std::fs::write(unknown.join("history.jsonl"), b"").unwrap();
        assert!(agent_dir_has_content(&unknown, ""));
        std::fs::remove_dir_all(&home).ok();
    }

    /// T3 接进探测：卸载残留、登录文件、skills 只会让工具消失（not_found），
    /// 不再落到「暂不能接入」；真有配置时照旧 cli_missing / installed
    #[test]
    fn residue_only_dirs_are_not_found() {
        let home = temp_home("residue");
        // Qwen 卸载后只剩 logs + cache
        std::fs::create_dir_all(home.join(".qwen").join("logs")).unwrap();
        std::fs::create_dir_all(home.join(".qwen").join("cache")).unwrap();
        // 没装过 Codex，只是被「应用到原工具」写了 auth.json
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(home.join(".codex").join("auth.json"), b"{}").unwrap();
        // Droid 只有 npx skills 建的 skills
        std::fs::create_dir_all(home.join(".factory").join("skills")).unwrap();
        // Kimi 有真实的历史文件
        std::fs::create_dir_all(home.join(".kimi")).unwrap();
        std::fs::write(home.join(".kimi").join("history.jsonl"), b"").unwrap();
        let results = discover_with(&home, &test_bins(&home));
        for agent in ["qwen", "codex", "droid"] {
            assert_eq!(agent_of(&results, agent).state, "not_found", "{agent} 只有残留不该算装了");
        }
        let kimi = agent_of(&results, "kimi");
        assert_eq!(kimi.state, "cli_missing");
        assert_eq!(kimi.evidence, Some("config_dir"));
        std::fs::remove_dir_all(&home).ok();
    }

    /// 真机只读探测（默认不跑）：打印 13 家的状态与证据，不打印路径。
    /// `cargo test --manifest-path backend/Cargo.toml --lib discover_real_machine -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn discover_real_machine_readonly_smoke() {
        for round in 1..=3 {
            let started = std::time::Instant::now();
            let results = discover();
            println!("第 {round} 次 discover：{:?}", started.elapsed());
            if round == 1 {
                for found in &results {
                    println!("{:<12} {:<12} evidence={:?} cli={}", found.agent, found.state, found.evidence, found.cli_found);
                }
            }
        }
        // 过了取戳窗口：重取一次戳（stat 各目录 + 读一次注册表），戳没变就不重找
        std::thread::sleep(LOOK_FRESH + std::time::Duration::from_millis(100));
        let started = std::time::Instant::now();
        discover();
        println!("过窗口后 discover：{:?}", started.elapsed());
        let started = std::time::Instant::now();
        forget_bin_lookups();
        discover();
        println!("forget 后 discover：{:?}", started.elapsed());
        let home = crate::creds::home().unwrap();
        for spec in &SPECS {
            let started = std::time::Instant::now();
            let found = find_bin(spec.bin, &home).is_some();
            println!("find_bin {:<9} {:>10?} found={found}", spec.bin, started.elapsed());
        }
        #[cfg(windows)]
        {
            let started = std::time::Instant::now();
            let dirs = registry_path_dirs();
            println!("registry_path_dirs：{:?}，{} 个目录", started.elapsed(), dirs.len());
        }
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
