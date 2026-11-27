use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;
use super::backup::{has_parent_component, stamp, write_atomic};
use super::scan::read_limited;
use super::{MAX_SKILL_BYTES, SESSION_TTL, SKILL_FILE};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Global,
    Project,
    /// Codex 内置 Skills，只读
    System,
    /// 从 claude.ai 同步，只读
    Synced,
}

/* ───────────────────────── 工具规则表（D-19） ───────────────────────── */

/// 启停方式
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Toggle {
    /// 把这一份移到 Jolu 停用区（Claude Code）；同时只读识别 settings.json 里用户写的 `skillOverrides`
    Park,
    /// Codex `config.toml` 的 `[[skills.config]]`，按 SKILL.md 路径停用
    CodexToml,
    /// 任务 4.2：`~/.gemini/settings.json` 的 `skills.disabled[]`
    #[allow(dead_code)]
    GeminiSettings,
    /// 任务 4.2：`opencode.json` 的 `permission.skill.<名称> = "deny"`
    #[allow(dead_code)]
    OpenCodePermission,
    /// 不提供启停
    #[allow(dead_code)]
    None,
}

/// 一个工具的 Skills 目录规则。路径用 `/` 分隔，拼接时逐段展开（不混用分隔符）。
///
/// 约定：
/// - `global[0]` 是新建、导入、复制到全局时的目标目录；
/// - `global` 的最后一项是工具自己的主目录：`home_env` 设了环境变量时为 `$ENV/<子目录>`，否则为用户目录下的默认位置，
///   默认位置去掉末尾的 `/<子目录>` 就是工具配置所在目录（如 `~/.claude/settings.json`、`~/.codex/config.toml`）；
///   「更换全局目录」（D-18）替换的也是这一项，前面的项（如 Codex 的 `~/.agents/skills`）不受影响；
/// - `readonly_subdirs` 是主目录下由工具托管的只读子目录，单独作为根目录扫描，扫描主目录时跳过。
pub struct ToolSpec {
    pub id: &'static str,
    /// 给用户看的名称（报错文案用）
    pub name: &'static str,
    /// 相对用户目录，如 ".claude/skills"
    pub global: &'static [&'static str],
    /// 相对项目根，如 ".claude/skills"
    pub project: &'static [&'static str],
    /// (环境变量, 其下子目录)，如 ("CLAUDE_CONFIG_DIR", "skills")
    pub home_env: Option<(&'static str, &'static str)>,
    pub toggle: Toggle,
    pub writable: bool,
    /// SKILL.md 必须写 name（Codex）；否则缺 name 时用目录名
    pub require_name: bool,
    /// 目录规则已按官方文档核实
    pub verified: bool,
    pub readonly_subdirs: &'static [(&'static str, Scope)],
}

/// 已接入的工具；其余供应商在前端标「待接入」（任务 4.2 继续添加）
pub const SPECS: &[ToolSpec] = &[
    ToolSpec {
        id: "claude",
        name: "Claude Code",
        global: &[".claude/skills"],
        project: &[".claude/skills"],
        home_env: Some(("CLAUDE_CONFIG_DIR", "skills")),
        toggle: Toggle::Park,
        writable: true,
        require_name: false,
        verified: true,
        readonly_subdirs: &[("synced", Scope::Synced)],
    },
    ToolSpec {
        id: "codex",
        name: "Codex",
        // `~/.agents/skills` 是首选位置，`$CODEX_HOME/skills` 是旧位置；`~/.agents/skills` 不受 CODEX_HOME 影响
        global: &[".agents/skills", ".codex/skills"],
        project: &[".agents/skills", ".codex/skills"],
        home_env: Some(("CODEX_HOME", "skills")),
        toggle: Toggle::CodexToml,
        writable: true,
        require_name: true,
        verified: true,
        readonly_subdirs: &[(".system", Scope::System)],
    },
    ToolSpec { id:"gemini", name:"Gemini", global:&[".agents/skills",".gemini/skills"], project:&[".gemini/skills"], home_env:None, toggle:Toggle::GeminiSettings, writable:true, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"opencode", name:"OpenCode", global:&[".claude/skills",".agents/skills",".config/opencode/skills"], project:&[".opencode/skills",".claude/skills",".agents/skills"], home_env:None, toggle:Toggle::OpenCodePermission, writable:true, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"qoder", name:"Qoder", global:&[".qoder-cn/skills",".qoder/skills"], project:&[".qoder/skills"], home_env:None, toggle:Toggle::None, writable:true, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"trae", name:"Trae", global:&[".trae/skills",".trae-cn/skills"], project:&[".trae/skills"], home_env:None, toggle:Toggle::None, writable:true, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"zcode", name:"ZCode", global:&[".zcode/skills"], project:&[".zcode/skills"], home_env:None, toggle:Toggle::None, writable:true, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"workbuddy", name:"WorkBuddy", global:&[".codebuddy/skills"], project:&[".codebuddy/skills"], home_env:None, toggle:Toggle::None, writable:false, require_name:false, verified:true, readonly_subdirs:&[] },
    ToolSpec { id:"custom", name:"自定义", global:&[], project:&[], home_env:None, toggle:Toggle::None, writable:false, require_name:false, verified:false, readonly_subdirs:&[] },
];

pub fn spec(id: &str) -> Option<&'static ToolSpec> {
    SPECS.iter().find(|s| s.id == id)
}

/// 按 `/` 逐段拼到 `base` 下
pub(crate) fn rel(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').filter(|s| !s.is_empty()).fold(base.to_path_buf(), |path, seg| path.join(seg))
}

/// `home_env` 的环境变量名与未设置时的默认目录（相对用户目录）：`global` 最后一项去掉 `/<子目录>`
pub(crate) fn env_default(spec: &ToolSpec) -> Option<(&'static str, &'static str)> {
    let (name, sub) = spec.home_env?;
    let own = spec.global.last()?;
    own.strip_suffix(sub)?.strip_suffix('/').map(|dir| (name, dir))
}

#[derive(Clone, Debug)]
pub(crate) struct Root {
    pub(crate) platform: &'static str,
    pub(crate) scope: Scope,
    pub(crate) project: Option<PathBuf>,
    pub(crate) path: PathBuf,
    pub(crate) readonly: bool,
    /// 扫描时跳过的子目录（它们是单独的根目录）
    pub(crate) skip: Vec<&'static str>,
    /// 停用区里登记的副本
    pub(crate) parked: bool,
    /// 工具主目录（可在 Jolu 里更换）
    pub(crate) own: bool,
}

#[derive(Clone, Debug)]
pub struct Homes {
    pub user: PathBuf,
    /// `home_env` 的环境变量 → 工具配置所在目录（环境变量的值，或用户目录下的默认位置）
    pub env: HashMap<String, PathBuf>,
}

pub(crate) fn homes_from(user: PathBuf, var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Homes {
    let mut env = HashMap::new();
    for spec in SPECS {
        if let Some((name, default)) = env_default(spec) {
            let dir = var(name).map(PathBuf::from).unwrap_or_else(|| rel(&user, default));
            env.insert(name.to_string(), dir);
        }
    }
    Homes { user, env }
}

/// 隔离验收环境：用户目录换成 `<JOLU_TEST_DIR>/skills-home`，不读环境变量
pub(crate) fn isolated_homes(test_dir: &Path) -> Homes {
    homes_from(test_dir.join("skills-home"), |_| None)
}

pub(crate) fn homes() -> Result<Homes, String> {
    // 隔离验收环境不碰真实用户目录，也不读环境变量
    if let Some(dir) = crate::validation::data_dir() {
        return Ok(isolated_homes(&dir));
    }
    let user = crate::creds::home().ok_or("未找到用户主目录")?;
    Ok(homes_from(user, |name| std::env::var_os(name)))
}

/// 工具配置所在目录（如 `~/.claude`、`$CODEX_HOME`）；不受「更换全局目录」影响
pub(crate) fn tool_home(h: &Homes, spec: &ToolSpec) -> PathBuf {
    if let Some(dir) = spec.home_env.and_then(|(name, _)| h.env.get(name)) {
        return dir.clone();
    }
    let own = spec.global.last().copied().unwrap_or_default();
    rel(&h.user, own.rsplit_once('/').map_or(own, |(dir, _)| dir))
}

/// 工具主目录的默认位置（没有在 Jolu 里更换时）
pub(crate) fn default_global(h: &Homes, spec: &ToolSpec) -> PathBuf {
    match spec.home_env {
        Some((_, sub)) if env_default(spec).is_some() => rel(&tool_home(h, spec), sub),
        _ => rel(&h.user, spec.global.last().copied().unwrap_or_default()),
    }
}

/// 工具主目录：「更换全局目录」设置过时用它，否则用默认位置
pub(crate) fn own_global(h: &Homes, cfg: &Config, spec: &ToolSpec) -> PathBuf {
    cfg.global_override.get(spec.id).map(PathBuf::from).unwrap_or_else(|| default_global(h, spec))
}

/// 已保存的覆盖路径失效（不是绝对路径或目录已不存在）时返回它：这时主目录只读，不会被写入时静默重建
pub(crate) fn broken_override<'a>(cfg: &'a Config, spec: &ToolSpec) -> Option<&'a str> {
    let path = cfg.global_override.get(spec.id)?;
    let dir = Path::new(path);
    (!dir.is_absolute() || has_parent_component(dir) || !dir.is_dir()).then_some(path.as_str())
}

pub(crate) fn broken_override_reason(path: &str) -> String {
    format!("已更换的目录不存在：{path}")
}

pub(crate) fn roots(h: &Homes, cfg: &Config, platform: &str, projects: &[PathBuf]) -> Vec<Root> {
    let Some(spec) = spec(platform) else { return Vec::new() };
    let root = |scope, project: Option<&PathBuf>, path: PathBuf, readonly, skip: Vec<&'static str>, own| Root {
        platform: spec.id,
        scope,
        project: project.cloned(),
        path,
        readonly,
        skip,
        parked: false,
        own,
    };
    if platform == "custom" { return cfg.custom_dirs.iter().map(|p|root(Scope::Global,None,PathBuf::from(p),true,vec![],false)).collect(); }
    let mut out = Vec::new();
    let last = spec.global.len().saturating_sub(1);
    for (i, dir) in spec.global.iter().enumerate() {
        if i < last {
            out.push(root(Scope::Global, None, rel(&h.user, dir), !spec.writable || matches!(platform,"gemini"|"opencode"), Vec::new(), false));
            continue;
        }
        let path = own_global(h, cfg, spec);
        let skip = spec.readonly_subdirs.iter().map(|(sub, _)| *sub).collect();
        let readonly = !spec.writable || broken_override(cfg, spec).is_some();
        out.push(root(Scope::Global, None, path.clone(), readonly, skip, true));
        for (sub, scope) in spec.readonly_subdirs {
            out.push(root(*scope, None, rel(&path, sub), true, Vec::new(), false));
        }
    }
    for p in projects {
        for dir in spec.project {
            let shared = platform == "opencode" && *dir != ".opencode/skills";
            out.push(root(Scope::Project, Some(p), rel(p, dir), !spec.writable || shared, Vec::new(), false));
        }
    }
    out
}

/// 新建、导入、复制的目标目录：每个范围取首选位置（Codex 用户级用 `~/.agents/skills`，`$CODEX_HOME/skills` 已标记为旧位置）
pub(crate) fn write_root(h: &Homes, cfg: &Config, platform: &str, scope: Scope, project: Option<&Path>) -> Result<Root, String> {
    let project = match scope {
        Scope::Global => None,
        Scope::Project => Some(project.ok_or("请选择项目目录")?.to_path_buf()),
        _ => return Err("只读范围不能写入".into()),
    };
    let list = roots(h, cfg, platform, project.as_slice());
    list.into_iter().find(|r| r.scope == scope && !r.readonly).ok_or_else(|| {
        match (scope, spec(platform).and_then(|s| broken_override(cfg, s))) {
            (Scope::Global, Some(path)) => broken_override_reason(path),
            _ => format!("{platform} 暂不支持写入 Skills"),
        }
    })
}

/* ───────────────────────── 路径比较 ───────────────────────── */

/// 统一 `/` 与 `\`、去掉末尾分隔符；Windows 不区分大小写
pub fn norm_key(path: &str) -> String {
    let unified = path.replace('\\', "/");
    let trimmed = unified.trim_end_matches('/');
    if cfg!(windows) { trimmed.to_lowercase() } else { trimmed.to_string() }
}

pub(crate) fn same_path(a: &str, b: &Path) -> bool {
    norm_key(a) == norm_key(&b.to_string_lossy())
}

/* ───────────────────────── 配置（应用数据目录 skills.json） ───────────────────────── */

/// Jolu 停用区里的一份副本
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DisabledEntry {
    pub platform: String,
    pub scope: Scope,
    pub project: Option<String>,
    pub name: String,
    /// 停用前所在的目录，启用时移回这里
    pub original: String,
    /// 停用区里的目录
    pub stored: String,
    pub disabled_at: i64,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// 「目录设置」里手动添加的项目目录
    #[serde(default)]
    pub projects: Vec<String>,
    /// 用户忽略的、从会话记录自动带出的项目目录
    #[serde(default)]
    pub ignored_projects: Vec<String>,
    /// 通过 Jolu 导入的安装项，来源列显示「导入」
    #[serde(default)]
    pub imported: Vec<String>,
    /// 停用区登记
    #[serde(default)]
    pub disabled: Vec<DisabledEntry>,
    /// 会话记录带出的项目目录（运行时填入，不落盘）
    #[serde(skip)]
    pub session_projects: Vec<String>,
    /// 「更换全局目录」（D-18）：工具 id → Jolu 扫描与写入的全局目录，替换工具主目录；不改工具本身的环境变量
    #[serde(default)]
    pub global_override: HashMap<String, String>,
    /// 任务 4.2：「自定义」扫描目录（只扫描，不提供启停）
    #[serde(default)]
    pub custom_dirs: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProjectInfo {
    pub path: String,
    /// manual：手动添加；session：来自会话记录
    pub source: &'static str,
    pub ignored: bool,
    /// 含有项目级 Skills 目录的工具 id（按 `SPECS` 顺序）
    pub tools: Vec<&'static str>,
}

impl Config {
    pub(crate) fn ignored(&self, path: &str) -> bool {
        self.ignored_projects.iter().any(|p| norm_key(p) == norm_key(path))
    }

    /// 手动 + 会话记录的项目，合并去重；跳过不存在的目录和用户主目录（其 Skills 目录就是全局目录）
    pub fn all_projects(&self, h: &Homes) -> Vec<ProjectInfo> {
        let home = norm_key(&h.user.to_string_lossy());
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let candidates = self.projects.iter().map(|p| (p, "manual")).chain(self.session_projects.iter().map(|p| (p, "session")));
        for (path, source) in candidates {
            let key = norm_key(path);
            let dir = Path::new(path);
            if key.is_empty() || key == home || !dir.is_absolute() || !dir.is_dir() || !seen.insert(key) {
                continue;
            }
            out.push(ProjectInfo {
                path: path.clone(),
                source,
                ignored: source == "session" && self.ignored(path),
                tools: SPECS.iter().filter(|s| s.project.iter().any(|d| rel(dir, d).is_dir())).map(|s| s.id).collect(),
            });
        }
        out
    }

    /// 参与扫描与作为写入目标的项目（不含已忽略的）
    pub(crate) fn project_paths(&self, h: &Homes) -> Vec<PathBuf> {
        self.all_projects(h).into_iter().filter(|p| !p.ignored).map(|p| PathBuf::from(p.path)).collect()
    }
}

pub(crate) fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    crate::app::state::data_paths(app).map(|(dir, _)| dir)
}

pub(crate) type FileSignature = (u64, Option<std::time::SystemTime>);

/// 读不懂的 skills.json：配置目录 → (文件大小与修改时间, 拒绝保存时的说明)。
/// 读取时登记并备份一次；文件修好后下次读取即清除。登记期间所有保存都被拒绝，避免用空配置覆盖掉停用登记
pub(crate) static BROKEN_CONFIG: LazyLock<Mutex<HashMap<String, (FileSignature, String)>>> = LazyLock::new(Default::default);

pub(crate) fn file_signature(path: &Path) -> FileSignature {
    fs::metadata(path).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None))
}

pub(crate) fn parse_config(path: &Path) -> Result<Config, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e.to_string()),
    };
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok(Config::default());
    }
    serde_json::from_str(text).map_err(|e| e.to_string())
}

/// 损坏时先把原文件复制为 `skills.json.broken-<时间戳>`（同一份损坏内容只备份一次），本次按空配置继续读取扫描
pub(crate) fn mark_broken(key: String, path: &Path) {
    let signature = file_signature(path);
    let Ok(mut broken) = BROKEN_CONFIG.lock() else { return };
    if broken.get(&key).is_some_and(|(known, _)| *known == signature) {
        return;
    }
    let backup = path.with_file_name(format!("skills.json.broken-{}", stamp()));
    let message = match fs::copy(path, &backup) {
        Ok(_) => format!("Skills 配置文件损坏，已备份为 {}，请检查后重试", backup.display()),
        Err(e) => format!("Skills 配置文件损坏且无法备份（{e}），请检查 {} 后重试", path.display()),
    };
    broken.insert(key, (signature, message));
}

pub(crate) fn load_config(dir: &Path) -> Config {
    let path = dir.join("skills.json");
    let key = norm_key(&dir.to_string_lossy());
    match parse_config(&path) {
        Ok(cfg) => {
            if let Ok(mut broken) = BROKEN_CONFIG.lock() {
                broken.remove(&key);
            }
            cfg
        }
        Err(error) => {
            if let Err(io) = fs::read(&path) {
                if let Ok(mut broken) = BROKEN_CONFIG.lock() { broken.insert(key, (file_signature(&path), format!("无法读取 Skills 配置：{io}（{error}）；未修改原文件"))); }
                return Config::default();
            }
            mark_broken(key, &path);
            Config::default()
        }
    }
}

/// skills.json 损坏时拒绝一切写入
pub(crate) fn config_guard(dir: &Path) -> Result<(), String> {
    let key = norm_key(&dir.to_string_lossy());
    match BROKEN_CONFIG.lock() {
        Ok(broken) => broken.get(&key).map_or(Ok(()), |(_, message)| Err(message.clone())),
        Err(_) => Ok(()),
    }
}

pub(crate) fn save_config(dir: &Path, cfg: &Config) -> Result<(), String> {
    config_guard(dir)?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    write_atomic(&dir.join("skills.json"), text.as_bytes())
}

/* ───────────────────────── 会话记录里的项目目录 ───────────────────────── */

pub(crate) static SESSION_PROJECTS: LazyLock<Mutex<Option<(Instant, Vec<String>)>>> = LazyLock::new(Default::default);

/// 复用会话线程的源文件定位与 cwd 读取（自带按修改时间失效的缓存）
pub(crate) fn read_session_projects(conn: &rusqlite::Connection) -> Vec<String> {
    let files = crate::sessions::source::source_files(conn).unwrap_or_default();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in files.values() {
        if let Some(cwd) = crate::sessions::source::read_cwd(path) {
            if seen.insert(norm_key(&cwd)) {
                out.push(cwd);
            }
        }
    }
    out
}

pub(crate) async fn session_projects(pool: Arc<crate::ReadPool>, force: bool) -> Result<Vec<String>, String> {
    if !force {
        if let Ok(guard) = SESSION_PROJECTS.lock() {
            if let Some((at, list)) = guard.as_ref() {
                if at.elapsed() < SESSION_TTL {
                    return Ok(list.clone());
                }
            }
        }
    }
    let list = crate::with_reader(pool, |conn| Ok(read_session_projects(conn))).await?;
    if let Ok(mut guard) = SESSION_PROJECTS.lock() {
        *guard = Some((Instant::now(), list.clone()));
    }
    Ok(list)
}

/* ───────────────────────── SKILL.md frontmatter ───────────────────────── */

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Front {
    pub name: Option<String>,
    pub description: Option<String>,
}

pub(crate) fn unquote(value: &str) -> String {
    let v = value.trim();
    if v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\''))) {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

/// 只解析顶层 `key: value`（含 `>` / `|` 多行写法），够判断 name 与 description；其余字段原样保留在文件里
pub fn parse_front(text: &str) -> Result<Front, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err("SKILL.md 缺少 frontmatter".into());
    }
    let mut body = Vec::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        body.push(line);
    }
    if !closed {
        return Err("SKILL.md 的 frontmatter 没有结束标记".into());
    }
    let mut front = Front::default();
    let mut i = 0;
    while i < body.len() {
        let line = body[i];
        i += 1;
        if line.starts_with([' ', '\t', '#']) || line.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let key = key.trim();
        let raw = value.trim();
        let value = if matches!(raw, ">" | "|" | ">-" | "|-" | ">+" | "|+") {
            let mut parts = Vec::new();
            while i < body.len() && (body[i].starts_with([' ', '\t']) || body[i].trim().is_empty()) {
                parts.push(body[i].trim());
                i += 1;
            }
            let sep = if raw.starts_with('>') { " " } else { "\n" };
            parts.join(sep).trim().to_string()
        } else {
            unquote(raw)
        };
        let value = (!value.is_empty()).then_some(value);
        match key {
            "name" => front.name = value,
            "description" => front.description = value,
            _ => {}
        }
    }
    Ok(front)
}

/// 格式问题；None 表示有效。都要求 description；`require_name` 的工具（Codex）还要求 name，其余缺 name 时用目录名
pub(crate) fn validate_front(platform: &str, front: &Result<Front, String>) -> Option<String> {
    match front {
        Err(reason) => Some(reason.clone()),
        Ok(f) if f.description.is_none() => Some("SKILL.md 缺少 description".into()),
        Ok(f) if f.name.is_none() && spec(platform).is_some_and(|s| s.require_name) => Some("SKILL.md 缺少 name".into()),
        Ok(_) => None,
    }
}

/// 名称规则：小写字母、数字与连字符，1–64 位，不以连字符开头或结尾
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub(crate) fn skill_name(dir: &Path) -> String {
    read_limited(&dir.join(SKILL_FILE), MAX_SKILL_BYTES)
        .and_then(|(t, _)| parse_front(&t))
        .ok()
        .and_then(|f| f.name)
        .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}
