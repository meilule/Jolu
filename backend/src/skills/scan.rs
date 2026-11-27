use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::specs::{
    broken_override, broken_override_reason, default_global, homes, parse_front,
    roots, same_path, spec, tool_home, valid_name, validate_front, write_root, Config,
    Front, Homes, ProjectInfo, Root, Scope, Toggle, ToolSpec, SPECS,
};
use super::toggle::json_skill_off;
use super::{MAX_SKILL_BYTES, SKILL_FILE, TOOL_DISABLED};

/* ───────────────────────── 扫描 ───────────────────────── */

#[derive(Serialize, Clone, Debug)]
pub struct SkillItem {
    pub platform: String,
    /// frontmatter 的 name，缺失时为目录名
    pub name: String,
    pub dir_name: String,
    pub description: Option<String>,
    pub scope: Scope,
    /// 项目范围的项目目录
    pub project: Option<String>,
    /// local / git / project / imported / synced / system
    pub source: &'static str,
    pub enabled: bool,
    /// enabled / disabled / conflict / invalid
    pub status: &'static str,
    pub issue: Option<String>,
    /// 当前所在目录；停用区里的副本为停用区路径
    pub path: String,
    /// 停用区里的副本停用前所在目录
    pub original_path: Option<String>,
    /// parked：Jolu 停用区；path：Codex 按路径停用；tool：工具配置按名称停用（只读）
    pub disabled_by: Option<&'static str>,
    pub readonly: bool,
    pub symlink: bool,
    pub modified_ms: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScanError {
    pub path: String,
    pub reason: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct RootInfo {
    pub platform: String,
    pub scope: Scope,
    pub project: Option<String>,
    pub path: String,
    pub exists: bool,
    pub readonly: bool,
    /// 工具主目录，可在「扫描目录设置」里更换（D-18）
    pub overridable: bool,
    /// 已更换为其他目录
    pub overridden: bool,
    /// 已更换时的默认位置，「恢复默认」后回到这里
    pub default_path: Option<String>,
    /// 停用区里原位置在这个目录下的 Skill 数；更换目录后它们启用时仍回到这里
    pub parked_here: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct PlatformSummary {
    pub id: String,
    /// 至少有一个 Skills 根目录存在
    pub detected: bool,
    pub count: usize,
    /// 目录规则已按官方文档核实
    pub verified: bool,
    /// 新建、导入、复制到全局时的目标目录；工具不支持写入时为 None
    pub global_dir: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct Overview {
    pub platforms: Vec<PlatformSummary>,
    pub items: Vec<SkillItem>,
    pub roots: Vec<RootInfo>,
    pub projects: Vec<ProjectInfo>,
    pub errors: Vec<ScanError>,
    pub scanned_at: i64,
}

pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(crate) fn modified_ms(path: &Path) -> Option<i64> {
    let time = fs::metadata(path).ok()?.modified().ok()?;
    Some(time.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64)
}

pub(crate) fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

pub(crate) fn read_limited(path: &Path, limit: u64) -> Result<(String, bool), String> {
    let file = fs::File::open(path).map_err(|e| format!("无法读取 {}：{e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > limit;
    bytes.truncate(limit as usize);
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
}

pub(crate) struct Raw {
    pub(crate) root: Root,
    pub(crate) dir: PathBuf,
    pub(crate) dir_name: String,
    pub(crate) front: Result<Front, String>,
    pub(crate) original: Option<String>,
}

pub(crate) fn scan_root(root: &Root, out: &mut Vec<Raw>, errors: &mut Vec<ScanError>) {
    if !root.path.is_dir() {
        return;
    }
    let entries = match fs::read_dir(&root.path) {
        Ok(entries) => entries,
        Err(e) => {
            errors.push(ScanError { path: root.path.display().to_string(), reason: e.to_string() });
            return;
        }
    };
    let mut found: Vec<Raw> = Vec::new();
    for entry in entries.flatten() {
        let dir_name = entry.file_name().to_string_lossy().into_owned();
        if dir_name.starts_with('.') || root.skip.contains(&dir_name.as_str()) {
            continue;
        }
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let file = dir.join(SKILL_FILE);
        if !file.is_file() {
            continue;
        }
        let front = read_limited(&file, MAX_SKILL_BYTES).and_then(|(text, _)| parse_front(&text));
        found.push(Raw { root: root.clone(), dir, dir_name, front, original: None });
    }
    found.sort_by(|a, b| a.dir_name.cmp(&b.dir_name));
    out.extend(found);
}

/* ───────────────────────── 工具配置（只读识别） ───────────────────────── */

pub(crate) fn read_json_object(path: &Path) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(Default::default()),
        Ok(text) => match serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')) {
            Ok(serde_json::Value::Object(map)) => Ok(map),
            _ => Err(format!("{} 不是有效的 JSON 对象", path.display())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(e) => Err(format!("无法读取 {}：{e}", path.display())),
    }
}

/// 某个 settings 文件里这个名称的 skillOverrides 值
pub(crate) fn claude_override(path: &Path, name: &str) -> Option<String> {
    read_json_object(path).ok()?.get("skillOverrides")?.get(name)?.as_str().map(String::from)
}

/// 用户自己写的 `skillOverrides: "off"`：按设置优先级取第一个写了这个名称的文件
/// （项目 settings.local.json > 项目 settings.json > 用户 settings.json）
pub(crate) fn claude_tool_off(h: &Homes, spec: &ToolSpec, root: &Root, name: &str) -> bool {
    let mut files = Vec::new();
    if let (Scope::Project, Some(p)) = (root.scope, &root.project) {
        files.push(p.join(".claude").join("settings.local.json"));
        files.push(p.join(".claude").join("settings.json"));
    }
    files.push(tool_home(h, spec).join("settings.json"));
    files.iter().find_map(|f| claude_override(f, name)).as_deref() == Some("off")
}

pub(crate) fn codex_config_path(h: &Homes, spec: &ToolSpec) -> PathBuf {
    tool_home(h, spec).join("config.toml")
}

/// Codex config.toml 里 enabled = false 的覆盖：(path 选择器, name 选择器)
pub(crate) fn codex_disabled(path: &Path) -> (Vec<String>, Vec<String>) {
    let Ok(text) = fs::read_to_string(path) else { return (vec![], vec![]) };
    let Ok(value) = text.parse::<toml::Table>() else { return (vec![], vec![]) };
    let mut paths = Vec::new();
    let mut names = Vec::new();
    let entries = value.get("skills").and_then(|s| s.get("config")).and_then(|c| c.as_array()).cloned().unwrap_or_default();
    for entry in entries {
        if entry.get("enabled").and_then(|v| v.as_bool()) != Some(false) {
            continue;
        }
        if let Some(p) = entry.get("path").and_then(|v| v.as_str()) {
            paths.push(p.to_string());
        }
        if let Some(n) = entry.get("name").and_then(|v| v.as_str()) {
            names.push(n.to_string());
        }
    }
    (paths, names)
}


/* ───────────────────────── 汇总 ───────────────────────── */

pub(crate) fn source_of(raw: &Raw, imported: &[String]) -> &'static str {
    let home = raw.original.as_ref().map(PathBuf::from).unwrap_or_else(|| raw.dir.clone());
    match raw.root.scope {
        Scope::System => "system",
        Scope::Synced => "synced",
        _ if imported.iter().any(|p| same_path(p, &home)) => "imported",
        _ if raw.dir.join(".git").exists() => "git",
        Scope::Project => "project",
        Scope::Global => "local",
    }
}

pub(crate) fn collect(h: &Homes, cfg: &Config, spec: &'static ToolSpec, errors: &mut Vec<ScanError>) -> (Vec<SkillItem>, Vec<RootInfo>) {
    let platform = spec.id;
    let projects = cfg.project_paths(h);
    // 没有对应 Skills 目录的项目不进入扫描结果
    let list: Vec<Root> = roots(h, cfg, platform, &projects).into_iter().filter(|r| r.scope != Scope::Project || r.path.is_dir()).collect();
    let mut raws = Vec::new();
    for root in &list {
        scan_root(root, &mut raws, errors);
    }
    for entry in cfg.disabled.iter().filter(|e| e.platform == platform) {
        let dir = PathBuf::from(&entry.stored);
        if !dir.join(SKILL_FILE).is_file() {
            errors.push(ScanError { path: entry.original.clone(), reason: format!("停用区里找不到这份 Skill：{}", entry.stored) });
            continue;
        }
        let root = Root {
            platform,
            scope: entry.scope,
            project: entry.project.as_ref().map(PathBuf::from),
            path: dir.parent().map(Path::to_path_buf).unwrap_or_default(),
            readonly: false,
            skip: Vec::new(),
            parked: true,
            own: false,
        };
        let dir_name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let front = read_limited(&dir.join(SKILL_FILE), MAX_SKILL_BYTES).and_then(|(t, _)| parse_front(&t));
        raws.push(Raw { root, dir, dir_name, front, original: Some(entry.original.clone()) });
    }
    if let Some(path) = broken_override(cfg, spec) {
        errors.push(ScanError { path: path.to_string(), reason: broken_override_reason(path) });
    }
    let overridden = cfg.global_override.contains_key(platform);
    let infos = list
        .iter()
        .map(|r| RootInfo {
            platform: r.platform.into(),
            scope: r.scope,
            project: r.project.as_ref().map(|p| p.display().to_string()),
            path: r.path.display().to_string(),
            exists: r.path.is_dir(),
            readonly: r.readonly,
            overridable: r.own,
            overridden: overridden && r.own,
            default_path: (overridden && r.own).then(|| default_global(h, spec).display().to_string()),
            parked_here: cfg
                .disabled
                .iter()
                .filter(|e| e.platform == platform && Path::new(&e.stored).join(SKILL_FILE).is_file())
                .filter(|e| Path::new(&e.original).parent().is_some_and(|p| same_path(&p.to_string_lossy(), &r.path)))
                .count(),
        })
        .collect();
    let codex = (spec.toggle == Toggle::CodexToml).then(|| codex_disabled(&codex_config_path(h, spec)));
    let mut seen: Vec<(String, Scope)> = Vec::new();
    let mut items = Vec::new();
    for raw in &raws {
        let name = raw.front.as_ref().ok().and_then(|f| f.name.clone()).unwrap_or_else(|| raw.dir_name.clone());
        let disabled_by: Option<&'static str> = if raw.root.parked {
            Some("parked")
        } else {
            match &codex {
                Some((paths, names)) => {
                    let file = raw.dir.join(SKILL_FILE);
                    if names.contains(&name) {
                        Some("tool")
                    } else if paths.iter().any(|p| same_path(p, &file)) {
                        Some("path")
                    } else {
                        None
                    }
                }
                None if matches!(spec.toggle,Toggle::GeminiSettings|Toggle::OpenCodePermission) => match json_skill_off(h,spec,&raw.root,&name) { Ok(true)=>Some("name"),Ok(false)=>None,Err(e)=>{errors.push(ScanError{path:raw.dir.display().to_string(),reason:e});Some("tool")} },
                None => (spec.toggle == Toggle::Park && claude_tool_off(h, spec, &raw.root, &name)).then_some("tool"),
            }
        };
        let enabled = disabled_by.is_none() && spec.toggle != Toggle::None;
        let invalid = validate_front(platform, &raw.front);
        // 停用区里的副本不会被工具加载，不参与同名判断
        let conflict = if raw.root.parked {
            None
        } else {
            let earlier = seen.iter().find(|(n, _)| *n == name).map(|(_, s)| *s);
            seen.push((name.clone(), raw.root.scope));
            earlier.map(|scope| {
                if scope != Scope::Project && raw.root.scope == Scope::Project {
                    "同名条目：项目与全局同时存在".to_string()
                } else {
                    "同名条目：多个目录同时存在".to_string()
                }
            })
        };
        let (status, issue) = match (invalid, conflict) {
            (Some(reason), _) => ("invalid", Some(reason)),
            (None, Some(reason)) => ("conflict", Some(reason)),
            (None, None) if disabled_by == Some("tool") => ("disabled", Some(TOOL_DISABLED.split('；').next().unwrap_or(TOOL_DISABLED).to_string())),
            (None, None) if spec.toggle == Toggle::None => ("unknown", Some("启停由原工具管理，Jolu 仅确认文件已安装".into())),
            (None, None) => (if enabled { "enabled" } else { "disabled" }, None),
        };
        items.push(SkillItem {
            platform: platform.into(),
            name,
            dir_name: raw.dir_name.clone(),
            description: raw.front.as_ref().ok().and_then(|f| f.description.clone()),
            scope: raw.root.scope,
            project: raw.root.project.as_ref().map(|p| p.display().to_string()),
            source: source_of(raw, &cfg.imported),
            enabled,
            status,
            issue,
            path: raw.dir.display().to_string(),
            original_path: raw.original.clone(),
            disabled_by,
            readonly: raw.root.readonly || is_symlink(&raw.dir),
            symlink: is_symlink(&raw.dir),
            modified_ms: modified_ms(&raw.dir.join(SKILL_FILE)),
        });
    }
    (items, infos)
}

pub fn overview_with(h: &Homes, cfg: &Config, platform: &str) -> Result<Overview, String> {
    if platform != "all" && spec(platform).is_none() {
        return Err(format!("{platform} 暂未接入 Skills 管理"));
    }
    let mut errors = Vec::new();
    let mut platforms = Vec::new();
    let mut items = Vec::new();
    let mut roots_out = Vec::new();
    for s in SPECS {
        let (list, infos) = collect(h, cfg, s, &mut errors);
        let global_dir = write_root(h, cfg, s.id, Scope::Global, None).ok().map(|r| r.path.display().to_string());
        platforms.push(PlatformSummary { id: s.id.into(), detected: infos.iter().any(|r| r.exists), count: list.len(), verified: s.verified, global_dir });
        if platform == "all" || platform == s.id {
            items.extend(list);
            roots_out.extend(infos);
        }
    }
    Ok(Overview { platforms, items, roots: roots_out, projects: cfg.all_projects(h), errors, scanned_at: now_ms() })
}


/* ───────────────────────── 自动化选择 Skills（D-08） ───────────────────────── */

/// 自动化「Skills（可选）」的一个候选项
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SkillOption {
    pub name: String,
    pub description: Option<String>,
    /// global / project / system / synced（与 [`Scope`] 的序列化名一致）
    pub scope: String,
    /// 项目级 Skill 所在的项目目录
    pub project: Option<String>,
}

/// 目标工具会加载的 Skills：全局目录 + `cwd` 下的项目目录。只列已启用、格式有效、名称合规的项，同名只保留第一份。
/// 停用区里的副本不在扫描目录里，自然不出现；工具配置按名称 / 路径停用的项被过滤
pub fn enabled_skills(platform: &str, cwd: Option<&Path>) -> Result<Vec<SkillOption>, String> {
    enabled_skills_with(&homes()?, platform, cwd)
}

pub fn enabled_skills_with(h: &Homes, platform: &str, cwd: Option<&Path>) -> Result<Vec<SkillOption>, String> {
    if !matches!(platform,"claude"|"codex") {return Err("自动化仅支持 Claude Code 与 Codex".into())}
    if spec(platform).is_none() {
        return Err(format!("{platform} 暂未接入 Skills 管理"));
    }
    // 只扫描任务的工作目录：不读 skills.json 里的项目列表与忽略设置，原工具在该目录执行时总会加载其项目 Skills
    let cfg = Config { projects: cwd.map(|p| vec![p.display().to_string()]).unwrap_or_default(), ..Config::default() };
    let overview = overview_with(h, &cfg, platform)?;
    let mut seen = HashSet::new();
    Ok(overview
        .items
        .into_iter()
        .filter(|i| i.enabled && i.status != "invalid" && valid_name(&i.name) && seen.insert(i.name.clone()))
        .map(|i| SkillOption {
            scope: serde_json::to_value(i.scope).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
            name: i.name,
            description: i.description,
            project: i.project,
        })
        .collect())
}
