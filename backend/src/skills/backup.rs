use super::SKILL_FILE;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use super::scan::is_symlink;
use super::specs::{roots, same_path, spec, Config, Homes, Root, SPECS};

/* ───────────────────────── 路径定位 ───────────────────────── */

pub(crate) fn has_parent_component(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::ParentDir))
}

/// 校验 `path` 是某个已知根目录的直接子目录，或停用区里登记过的副本，返回所属根目录
pub(crate) fn locate(h: &Homes, cfg: &Config, path: &str) -> Result<(Root, PathBuf), String> {
    let dir = PathBuf::from(path);
    if !dir.is_absolute() || has_parent_component(&dir) {
        return Err("路径无效".into());
    }
    let parent = dir.parent().ok_or("路径无效")?;
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.is_empty() || name.starts_with('.') {
        return Err("路径无效".into());
    }
    if let Some(entry) = cfg.disabled.iter().find(|e| same_path(&e.stored, &dir)) {
        if !dir.join(SKILL_FILE).is_file() {
            return Err("停用区里找不到这份 Skill".into());
        }
        let root = Root {
            platform: spec(&entry.platform).ok_or("路径无效")?.id,
            scope: entry.scope,
            project: entry.project.as_ref().map(PathBuf::from),
            path: parent.to_path_buf(),
            readonly: false,
            skip: Vec::new(),
            parked: true,
            own: false,
        };
        return Ok((root, dir));
    }
    let parent_c = fs::canonicalize(parent).map_err(|_| "Skill 所在目录不存在")?;
    let projects = cfg.project_paths(h);
    for s in SPECS {
        for root in roots(h, cfg, s.id, &projects) {
            let Ok(root_c) = fs::canonicalize(&root.path) else { continue };
            if root_c == parent_c && !root.skip.contains(&name.as_str()) {
                if !dir.join(SKILL_FILE).is_file() {
                    return Err("目录里没有 SKILL.md".into());
                }
                return Ok((root, dir));
            }
        }
    }
    Err("不在已配置的 Skills 目录中".into())
}

pub(crate) fn ensure_writable(root: &Root, dir: &Path) -> Result<(), String> {
    if root.readonly {
        return Err("这是工具托管的只读安装项，可复制为自定义后再修改".into());
    }
    if is_symlink(dir) {
        return Err("符号链接安装项只读，请到原位置修改".into());
    }
    Ok(())
}

pub(crate) fn safe_rel(rel: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(rel);
    if rel.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("文件路径无效".into());
    }
    Ok(p)
}

/* ───────────────────────── 文件工具 ───────────────────────── */

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.jolu-{nonce}.tmp"));
    let result = (|| {
        let mut file = fs::OpenOptions::new().create_new(true).write(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}

pub(crate) fn stamp() -> String {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() % 1000).unwrap_or(0);
    format!("{}{nonce:03}", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f"))
}

pub(crate) fn backup_file(path: &Path, backup_dir: &Path) -> Result<Option<PathBuf>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    fs::create_dir_all(backup_dir).map_err(|e| e.to_string())?;
    let target = backup_dir.join(format!("{}-{name}", stamp()));
    fs::copy(path, &target).map_err(|e| format!("备份失败，未修改：{e}"))?;
    Ok(Some(target))
}

/// 递归复制，不跟随符号链接
pub(crate) fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| format!("读取 {} 失败：{e}", from.display()))?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target).map_err(|e| format!("复制 {} 失败：{e}", entry.path().display()))?;
        }
    }
    Ok(())
}

pub(crate) fn backup_dir_tree(dir: &Path, platform: &str, backup_root: &Path) -> Result<PathBuf, String> {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let target = backup_root.join(format!("{}-{platform}-{name}", stamp()));
    copy_dir(dir, &target).map_err(|e| format!("备份失败，未修改：{e}"))?;
    Ok(target)
}

/// 先完整复制来源，再交换安装目录；复制或交换失败时保留原安装项。
pub(crate) fn overwrite_dir(source: &Path, dest: &Path, platform: &str, backup_root: &Path) -> Result<(), String> {
    let source_c = fs::canonicalize(source).map_err(|e| e.to_string())?;
    let dest_c = fs::canonicalize(dest).map_err(|e| e.to_string())?;
    if source_c.starts_with(&dest_c) || dest_c.starts_with(&source_c) {
        return Err("来源与目标目录相同或互相包含，不能覆盖导入".into());
    }
    // 放在目标同一目录，两个 rename 都不会跨磁盘；create_dir 避免复用已有暂存区。
    let staging = dest.parent().ok_or("目标目录无效")?.join(format!(".jolu-import-{}", stamp()));
    fs::create_dir(&staging).map_err(|e| format!("无法创建导入暂存目录：{e}"))?;
    let incoming = staging.join("incoming");
    let previous = staging.join("previous");
    let prepared = copy_dir(source, &incoming)
        .and_then(|_| backup_dir_tree(dest, platform, backup_root));
    let backup = match prepared {
        Ok(backup) => backup,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("导入准备失败，原目录保持不变：{error}"));
        }
    };
    if let Err(error) = fs::rename(dest, &previous) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("无法替换目标，原目录保持不变：{error}"));
    }
    if let Err(error) = fs::rename(&incoming, dest) {
        if let Err(rollback) = fs::rename(&previous, dest) {
            return Err(format!("覆盖失败：{error}；恢复失败：{rollback}；原目录保留在 {}，备份在 {}", previous.display(), backup.display()));
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("覆盖失败，已恢复原目录：{error}"));
    }
    fs::remove_dir_all(&staging).map_err(|e| format!("已覆盖导入，但清理暂存目录 {} 失败：{e}；原内容备份在 {}", staging.display(), backup.display()))
}
