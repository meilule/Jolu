use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::State;

use super::backup::{
    backup_dir_tree, backup_file, copy_dir, ensure_writable, has_parent_component, locate,
    overwrite_dir, safe_rel, write_atomic,
};
use super::scan::{collect, is_symlink, modified_ms, read_limited, ScanError, SkillItem};
use super::specs::{
    config_guard, data_dir, default_global, load_config, norm_key, parse_front, roots, same_path,
    save_config, session_projects, skill_name, spec, valid_name, validate_front, write_root,
    Config, Homes, Scope, SPECS,
};
use super::toggle::{remove_empty, set_enabled_impl};
use super::{EXTERNAL_CHANGE, MAX_FILES, MAX_PREVIEW_BYTES, MAX_SKILL_BYTES, SKILL_FILE};

/* ───────────────────────── 详情 ───────────────────────── */

#[derive(Serialize, Debug)]
pub struct FileEntry {
    pub rel: String,
    pub dir: bool,
    pub depth: u8,
    pub size: Option<u64>,
}

#[derive(Serialize, Debug)]
pub struct Detail {
    pub item: SkillItem,
    pub files: Vec<FileEntry>,
    pub files_truncated: bool,
    pub content: String,
    pub truncated: bool,
}

pub(crate) fn list_files(dir: &Path, base: &Path, depth: u8, out: &mut Vec<FileEntry>) -> bool {
    let Ok(entries) = fs::read_dir(dir) else { return false };
    let mut entries: Vec<_> = entries.flatten().filter(|e| e.file_type().map(|t| !t.is_symlink()).unwrap_or(false)).collect();
    entries.sort_by_key(|e| (!e.path().is_dir(), e.file_name()));
    for entry in entries {
        if out.len() >= MAX_FILES {
            return true;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        let is_dir = path.is_dir();
        out.push(FileEntry { rel, dir: is_dir, depth, size: if is_dir { None } else { fs::metadata(&path).ok().map(|m| m.len()) } });
        if is_dir && depth < 3 && list_files(&path, base, depth + 1, out) {
            return true;
        }
    }
    false
}

pub fn detail_with(h: &Homes, cfg: &Config, path: &str) -> Result<Detail, String> {
    let (root, dir) = locate(h, cfg, path)?;
    let mut errors = Vec::new();
    let (items, _) = collect(h, cfg, spec(root.platform).ok_or("路径无效")?, &mut errors);
    let item = items.into_iter().find(|i| same_path(&i.path, &dir)).ok_or("没有找到这个 Skill，可能已被移除")?;
    let (content, truncated) = read_limited(&dir.join(SKILL_FILE), MAX_PREVIEW_BYTES)?;
    let mut files = Vec::new();
    let files_truncated = list_files(&dir, &dir, 0, &mut files);
    Ok(Detail { item, files, files_truncated, content, truncated })
}

#[derive(Serialize, Debug)]
pub struct FileContent {
    pub text: Option<String>,
    pub binary: bool,
    pub size: u64,
    pub truncated: bool,
}

pub fn file_with(h: &Homes, cfg: &Config, path: &str, rel: &str) -> Result<FileContent, String> {
    let (_, dir) = locate(h, cfg, path)?;
    let full = dir.join(safe_rel(rel)?);
    let full_c = fs::canonicalize(&full).map_err(|_| "文件不存在")?;
    let dir_c = fs::canonicalize(&dir).map_err(|e| e.to_string())?;
    if !full_c.starts_with(&dir_c) || is_symlink(&full) || !full_c.is_file() {
        return Err("文件不在此 Skill 目录内".into());
    }
    let size = fs::metadata(&full_c).map(|m| m.len()).unwrap_or(0);
    let file = fs::File::open(&full_c).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_PREVIEW_BYTES + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > MAX_PREVIEW_BYTES;
    bytes.truncate(MAX_PREVIEW_BYTES as usize);
    if bytes.contains(&0) {
        return Ok(FileContent { text: None, binary: true, size, truncated: false });
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(FileContent { text: Some(text), binary: false, size, truncated }),
        Err(e) if truncated => Ok(FileContent { text: Some(String::from_utf8_lossy(&e.into_bytes()).into_owned()), binary: false, size, truncated }),
        Err(_) => Ok(FileContent { text: None, binary: true, size, truncated: false }),
    }
}

/* ───────────────────────── 新建 / 编辑 ───────────────────────── */

#[derive(Deserialize, Debug)]
pub struct SaveInput {
    pub platform: String,
    pub scope: Scope,
    pub project: Option<String>,
    pub name: String,
    pub content: String,
    /// 编辑时为原目录；新建为 None
    pub original_path: Option<String>,
    /// 打开编辑器时 SKILL.md 的修改时间，用于发现外部修改
    pub expected_modified_ms: Option<i64>,
    pub enable: bool,
}

#[derive(Serialize, Debug)]
pub struct SaveResult {
    pub path: String,
    pub modified_ms: Option<i64>,
    pub backup: Option<String>,
}

pub(crate) fn check_project(h: &Homes, cfg: &Config, project: Option<&str>) -> Result<Option<PathBuf>, String> {
    match project {
        None => Ok(None),
        Some(p) => cfg
            .project_paths(h)
            .into_iter()
            .find(|known| same_path(p, known))
            .map(Some)
            .ok_or_else(|| "项目目录不在会话记录或「目录设置」里".into()),
    }
}

pub fn save_with(h: &Homes, cfg: &mut Config, input: SaveInput, data: &Path) -> Result<SaveResult, String> {
    if !spec(&input.platform).is_some_and(|s| s.writable) {
        return Err(format!("{} 暂不支持写入 Skills", input.platform));
    }
    if !valid_name(&input.name) {
        return Err("名称只能使用小写字母、数字和连字符，最长 64 位".into());
    }
    if input.content.len() as u64 > MAX_SKILL_BYTES {
        return Err("SKILL.md 超过 1 MB".into());
    }
    let front = parse_front(&input.content);
    if let Some(reason) = validate_front(&input.platform, &front) {
        return Err(reason);
    }
    if front.as_ref().ok().and_then(|f| f.name.as_deref()).is_some_and(|n| n != input.name) {
        return Err("SKILL.md 里的 name 与名称不一致".into());
    }
    if let Some(original) = &input.original_path {
        let (root, dir) = locate(h, cfg, original)?;
        ensure_writable(&root, &dir)?;
        if root.platform != input.platform || dir.file_name().map(|n| n.to_string_lossy() != input.name).unwrap_or(true) {
            return Err("编辑时不能修改名称、供应商或位置；请用「复制到供应商」".into());
        }
        let file = dir.join(SKILL_FILE);
        if let (Some(expected), Some(actual)) = (input.expected_modified_ms, modified_ms(&file)) {
            if (expected - actual).abs() > 1 {
                return Err(EXTERNAL_CHANGE.into());
            }
        }
        let backup = backup_file(&file, &data.join("skill-backups").join("files"))?;
        write_atomic(&file, input.content.as_bytes())?;
        return Ok(SaveResult { path: dir.display().to_string(), modified_ms: modified_ms(&file), backup: backup.map(|p| p.display().to_string()) });
    }
    let project = check_project(h, cfg, input.project.as_deref())?;
    let root = write_root(h, cfg, &input.platform, input.scope, project.as_deref())?;
    let dir = root.path.join(&input.name);
    if dir.exists() {
        return Err(format!("目标已有同名 Skill：{}", dir.display()));
    }
    fs::create_dir_all(&root.path).map_err(|e| format!("无法创建目录：{e}"))?;
    fs::create_dir(&dir).map_err(|e| format!("无法创建目录：{e}"))?;
    let file = dir.join(SKILL_FILE);
    write_atomic(&file, input.content.as_bytes())?;
    let mut path = dir.display().to_string();
    if !input.enable {
        path = set_enabled_impl(h, cfg, &root, &dir, &input.name, false, data)?;
    }
    Ok(SaveResult { modified_ms: modified_ms(&Path::new(&path).join(SKILL_FILE)), path, backup: None })
}

/* ───────────────────────── 复制 / 移除 ───────────────────────── */

#[derive(Serialize, Debug)]
pub struct CopyResult {
    pub target: String,
    /// 仅检查时为 false
    pub copied: bool,
    pub conflict: bool,
    pub issue: Option<String>,
}

pub fn copy_with(h: &Homes, cfg: &Config, path: &str, platform: &str, scope: Scope, project: Option<&str>, dry_run: bool) -> Result<CopyResult, String> {
    let (_, dir) = locate(h, cfg, path)?;
    let project = check_project(h, cfg, project)?;
    let root = write_root(h, cfg, platform, scope, project.as_deref())?;
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let target = root.path.join(&name);
    let front = read_limited(&dir.join(SKILL_FILE), MAX_SKILL_BYTES).and_then(|(t, _)| parse_front(&t));
    let issue = validate_front(platform, &front).map(|r| format!("目标工具不接受：{r}"));
    let conflict = target.exists();
    if same_path(&target.to_string_lossy(), &dir) {
        return Err("目标与源目录相同".into());
    }
    if dry_run || conflict || issue.is_some() {
        return Ok(CopyResult { target: target.display().to_string(), copied: false, conflict, issue });
    }
    copy_dir(&dir, &target)?;
    Ok(CopyResult { target: target.display().to_string(), copied: true, conflict: false, issue: None })
}

pub fn remove_with(h: &Homes, cfg: &mut Config, path: &str, data: &Path) -> Result<String, String> {
    let (root, dir) = locate(h, cfg, path)?;
    ensure_writable(&root, &dir)?;
    let backup = backup_dir_tree(&dir, root.platform, &data.join("skill-backups").join("removed"))?;
    fs::remove_dir_all(&dir).map_err(|e| format!("移除失败（备份已保存在 {}）：{e}", backup.display()))?;
    if root.parked {
        cfg.disabled.retain(|e| !same_path(&e.stored, &dir));
        save_config(data, cfg)?;
        remove_empty(dir.parent());
    }
    Ok(backup.display().to_string())
}

/* ───────────────────────── 导入 ───────────────────────── */

#[derive(Serialize, Debug, Clone)]
pub struct ImportItem {
    pub name: String,
    pub path: String,
    pub description: Option<String>,
    /// ok / conflict / invalid
    pub status: &'static str,
    pub issue: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct ImportScan {
    pub source: String,
    pub target: String,
    pub items: Vec<ImportItem>,
}

pub fn import_scan_with(h: &Homes, cfg: &Config, source: &str, platform: &str, scope: Scope, project: Option<&str>) -> Result<ImportScan, String> {
    let src = PathBuf::from(source);
    if !src.is_absolute() || has_parent_component(&src) || !src.is_dir() {
        return Err("来源目录不存在".into());
    }
    let project = check_project(h, cfg, project)?;
    let root = write_root(h, cfg, platform, scope, project.as_deref())?;
    let mut candidates = Vec::new();
    if src.join(SKILL_FILE).is_file() {
        candidates.push(src.clone());
    } else {
        let mut dirs: Vec<PathBuf> = fs::read_dir(&src)
            .map_err(|e| format!("无法读取来源目录：{e}"))?
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .filter(|p| p.join(SKILL_FILE).is_file())
            .collect();
        dirs.sort();
        candidates.extend(dirs.into_iter().take(MAX_FILES));
    }
    let items = candidates
        .into_iter()
        .map(|dir| {
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let front = read_limited(&dir.join(SKILL_FILE), MAX_SKILL_BYTES).and_then(|(t, _)| parse_front(&t));
            let invalid = validate_front(platform, &front).or_else(|| (!valid_name(&name)).then(|| "目录名不符合 Skill 名称规则".to_string()));
            let (status, issue) = match invalid {
                Some(reason) => ("invalid", Some(format!("{reason}，需先修复"))),
                None if root.path.join(&name).exists() => ("conflict", Some("目标已有同名 Skill".to_string())),
                None if parked_copy(cfg, platform, &root.path.join(&name)).is_some() => ("conflict", Some("目标已有同名 Skill（已停用）".to_string())),
                None => ("ok", None),
            };
            ImportItem { name, path: dir.display().to_string(), description: front.ok().and_then(|f| f.description), status, issue }
        })
        .collect();
    Ok(ImportScan { source: src.display().to_string(), target: root.path.display().to_string(), items })
}

#[derive(Deserialize, Debug)]
pub struct ImportChoice {
    pub name: String,
    /// import / overwrite / skip
    pub action: String,
}

#[derive(Serialize, Debug, Default)]
pub struct ImportResult {
    pub imported: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<ScanError>,
}

/// 停用区里原位置是 `target` 的那一份（有多份时取最近停用的）
pub(crate) fn parked_copy(cfg: &Config, platform: &str, target: &Path) -> Option<PathBuf> {
    cfg.disabled
        .iter()
        .rev()
        .filter(|e| e.platform == platform && same_path(&e.original, target))
        .map(|e| PathBuf::from(&e.stored))
        .find(|stored| stored.join(SKILL_FILE).is_file())
}

#[allow(clippy::too_many_arguments)]
pub fn import_with(h: &Homes, cfg: &mut Config, source: &str, platform: &str, scope: Scope, project: Option<&str>, choices: Vec<ImportChoice>, enable: bool, data: &Path) -> Result<ImportResult, String> {
    let scan = import_scan_with(h, cfg, source, platform, scope, project)?;
    let project_path = check_project(h, cfg, project)?;
    let root = write_root(h, cfg, platform, scope, project_path.as_deref())?;
    let mut result = ImportResult::default();
    for choice in choices {
        let Some(item) = scan.items.iter().find(|i| i.name == choice.name) else {
            result.failed.push(ScanError { path: choice.name, reason: "来源中没有这个 Skill".into() });
            continue;
        };
        let target = root.path.join(&item.name);
        let outcome = match (choice.action.as_str(), item.status) {
            ("skip", _) => {
                result.skipped.push(item.name.clone());
                continue;
            }
            (_, "invalid") => Err(item.issue.clone().unwrap_or_else(|| "格式异常".into())),
            ("import", "ok") => fs::create_dir_all(&root.path)
                .map_err(|e| e.to_string())
                .and_then(|_| copy_dir(Path::new(&item.path), &target))
                .map(|_| (target.clone(), true)),
            // 覆盖写到同名 Skill 当前所在的位置（原目录，或 Claude 停用区里的那一份），启停状态保持不变
            ("overwrite", "conflict") => (|| {
                let dest = if target.exists() { target.clone() } else { parked_copy(cfg, platform, &target).ok_or("目标已不存在，请重新扫描")? };
                ensure_writable(&root, &dest)?;
                overwrite_dir(Path::new(&item.path), &dest, platform, &data.join("skill-backups").join("overwritten"))?;
                Ok((dest, false))
            })(),
            ("import", "conflict") => Err("目标已有同名 Skill，请选择跳过或覆盖".into()),
            _ => Err("不支持的导入方式".into()),
        };
        match outcome {
            Ok((dest, is_new)) => {
                // 来源按原位置登记：停用区里的副本也按原位置判断来源
                if !cfg.imported.iter().any(|p| same_path(p, &target)) {
                    cfg.imported.push(target.display().to_string());
                }
                let mut final_path = dest.display().to_string();
                // 「导入后启用」只作用于新导入的 Skill
                if is_new && !enable {
                    match set_enabled_impl(h, cfg, &root, &target, &skill_name(&target), false, data) {
                        Ok(p) => final_path = p,
                        Err(reason) => result.failed.push(ScanError { path: item.name.clone(), reason: format!("已导入，但未能设为停用：{reason}") }),
                    }
                }
                result.imported.push(final_path);
            }
            Err(reason) => result.failed.push(ScanError { path: item.name.clone(), reason }),
        }
    }
    Ok(result)
}


/* ───────────────────────── 项目目录设置 ───────────────────────── */

pub fn set_projects_with(cfg: &mut Config, projects: Vec<String>) -> Result<Vec<String>, String> {
    let mut list: Vec<String> = Vec::new();
    for p in projects {
        let path = PathBuf::from(p.trim());
        if !path.is_absolute() || has_parent_component(&path) || !path.is_dir() {
            return Err(format!("项目目录不存在：{}", path.display()));
        }
        if !list.iter().any(|known| same_path(known, &path)) {
            list.push(path.display().to_string());
        }
    }
    if list.len() > 50 {
        return Err("最多添加 50 个项目目录".into());
    }
    cfg.projects = list.clone();
    Ok(list)
}

/// 「更换全局目录」（D-18）：替换工具主目录（`ToolSpec::global` 的最后一项），只改 Jolu 扫描和写入的位置，
/// 不改工具本身的环境变量；`path` 为 None 或与默认位置相同时恢复默认。目录必须已存在
pub fn set_global_dir_with(h: &Homes, cfg: &mut Config, platform: &str, path: Option<&str>) -> Result<(), String> {
    let spec = spec(platform).ok_or_else(|| format!("{platform} 暂未接入 Skills 管理"))?;
    let Some(raw) = path.map(str::trim).filter(|p| !p.is_empty()) else {
        cfg.global_override.remove(spec.id);
        return Ok(());
    };
    let dir = PathBuf::from(raw);
    if !dir.is_absolute() || has_parent_component(&dir) || !dir.is_dir() {
        return Err(format!("目录不存在：{raw}"));
    }
    if same_path(raw, &default_global(h, spec)) {
        cfg.global_override.remove(spec.id);
        return Ok(());
    }
    // 已经是任一工具（含自己）的其他扫描目录、只读子目录或项目 Skills 目录时拒绝：同一目录被扫描两次会标成同名冲突，
    // 被两个工具扫描时 `locate` 先命中排在前面的工具，启停就会走错方式（如在 Codex 列表里停用却按 Claude 移进停用区）
    let projects = cfg.project_paths(h);
    for other in SPECS {
        let taken = roots(h, cfg, other.id, &projects).into_iter().filter(|r| !(other.id == spec.id && r.own)).find(|r| same_path(raw, &r.path));
        if let Some(root) = taken {
            return Err(format!("这个目录已经是 {} 的扫描目录：{}", other.name, root.path.display()));
        }
    }
    cfg.global_override.insert(spec.id.into(), dir.display().to_string());
    Ok(())
}

#[derive(Deserialize, Debug)]
pub struct IgnoreChange {
    pub path: String,
    pub ignored: bool,
}

#[derive(Deserialize, Debug)]
pub struct GlobalChange {
    pub platform: String,
    /// None：恢复默认
    pub path: Option<String>,
}

/// 「扫描目录设置」弹窗的草稿，点「保存并扫描」时一次提交
#[derive(Deserialize, Debug, Default)]
pub struct DirSettings {
    #[serde(default)]
    pub custom_dirs: Option<Vec<String>>,
    /// 手动添加的项目目录（完整列表）
    pub projects: Vec<String>,
    /// 会话记录带出的项目里改了忽略状态的
    #[serde(default)]
    pub ignore: Vec<IgnoreChange>,
    /// 改了全局目录的工具
    #[serde(default)]
    pub globals: Vec<GlobalChange>,
}

/// 在副本上依次应用并校验，全部通过才替换 `cfg`；任何一项失败都不改动配置
pub fn save_dir_settings_with(h: &Homes, cfg: &mut Config, input: DirSettings) -> Result<(), String> {
    let mut next = cfg.clone();
    if let Some(dirs)=input.custom_dirs {
        if dirs.len()>50 {return Err("最多添加 50 个自定义目录".into())}
        let mut valid=Vec::new();for dir in dirs {let p=PathBuf::from(dir.trim());if !p.is_absolute()||has_parent_component(&p)||!p.is_dir(){return Err(format!("自定义目录不存在：{}",p.display()))}if !valid.iter().any(|v:&String|same_path(v,&p)){valid.push(p.display().to_string());}}
        next.custom_dirs=valid;
    }
    set_projects_with(&mut next, input.projects)?;
    for change in input.ignore {
        ignore_project_with(&mut next, &change.path, change.ignored);
    }
    for change in input.globals {
        set_global_dir_with(h, &mut next, &change.platform, change.path.as_deref())?;
    }
    // 再按最终草稿检查，覆盖“先更换全局目录、后加项目”和恢复默认的反向冲突。
    let projects=next.project_paths(h);
    for (platform,path) in &next.global_override {
        for other in SPECS {
            if roots(h,&next,other.id,&projects).iter().any(|r| !(r.platform==platform&&r.own) && same_path(path,&r.path)) {
                return Err(format!("更换的全局目录与 {} 扫描目录重复：{path}",other.name));
            }
        }
    }
    *cfg = next;
    Ok(())
}

/// 忽略 / 恢复一个从会话记录带出的项目；只改 skills.json，不碰会话数据
pub fn ignore_project_with(cfg: &mut Config, path: &str, ignored: bool) {
    cfg.ignored_projects.retain(|p| norm_key(p) != norm_key(path));
    if ignored {
        cfg.ignored_projects.push(path.to_string());
    }
}

/* ───────────────────────── Tauri 命令 ───────────────────────── */

pub(crate) async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())?
}

/// 应用数据目录 + 合并了会话项目的配置
pub(crate) async fn context(app: &tauri::AppHandle, db: &State<'_, crate::DbRead>, force: bool) -> Result<(PathBuf, Config), String> {
    let dir = data_dir(app)?;
    let sessions = session_projects(db.0.clone(), force).await?;
    let mut cfg = load_config(&dir);
    cfg.session_projects = sessions;
    Ok((dir, cfg))
}

/// 会改动文件或配置的命令：skills.json 损坏时在动手之前就拒绝，避免改了文件却记不下停用登记
pub(crate) async fn context_mut(app: &tauri::AppHandle, db: &State<'_, crate::DbRead>) -> Result<(PathBuf, Config), String> {
    let (dir, cfg) = context(app, db, false).await?;
    config_guard(&dir)?;
    Ok((dir, cfg))
}
