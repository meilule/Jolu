use std::fs;
use std::path::{Path, PathBuf};

use super::backup::{backup_dir_tree, backup_file, copy_dir, ensure_writable, stamp, write_atomic};
use super::scan::{claude_tool_off, codex_config_path, codex_disabled, now_ms};
use super::specs::{same_path, save_config, spec, tool_home, Config, DisabledEntry, Homes, Root, Toggle, ToolSpec};
use super::{SKILL_FILE, TOOL_DISABLED};

/* ───────────────────────── 启停 ───────────────────────── */

pub(crate) fn has_symlink_inside(dir: &Path) -> bool {
    fs::read_dir(dir).map(|entries| {
        entries.flatten().any(|e| match e.file_type() {
            Ok(t) if t.is_symlink() => true,
            Ok(t) if t.is_dir() => has_symlink_inside(&e.path()),
            _ => false,
        })
    }).unwrap_or(false)
}

/// 移动目录：同盘直接改名；跨盘先完整复制并校验再删原目录。目录里有符号链接时拒绝跨盘移动，避免丢失链接
pub(crate) fn move_dir(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        return Err(format!("目标已存在：{}", to.display()));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建目录：{e}"))?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if has_symlink_inside(from) {
        return Err("目录里有符号链接，不能跨磁盘移动；未做任何修改".into());
    }
    if let Err(e) = copy_dir(from, to) {
        let _ = fs::remove_dir_all(to);
        return Err(format!("移动失败，原目录保持不变：{e}"));
    }
    if !to.join(SKILL_FILE).is_file() {
        let _ = fs::remove_dir_all(to);
        return Err("移动后校验失败，原目录保持不变".into());
    }
    fs::remove_dir_all(from).map_err(|e| format!("已复制到 {}，但原目录删除失败：{e}", to.display()))
}

pub(crate) fn remove_empty(dir: Option<&Path>) {
    if let Some(dir) = dir {
        let _ = fs::remove_dir(dir);
    }
}

/// Claude 停用：把这一份移到停用区，记录原路径
pub(crate) fn park(cfg: &mut Config, root: &Root, dir: &Path, name: &str, data: &Path) -> Result<String, String> {
    ensure_writable(root, dir)?;
    let dir_name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    backup_dir_tree(dir, root.platform, &data.join("skill-backups").join("disabled"))?;
    let stored = data.join("skills-disabled").join(format!("{}-{}", stamp(), root.platform)).join(&dir_name);
    move_dir(dir, &stored)?;
    cfg.disabled.push(DisabledEntry {
        platform: root.platform.into(),
        scope: root.scope,
        project: root.project.as_ref().map(|p| p.display().to_string()),
        name: name.into(),
        original: dir.display().to_string(),
        stored: stored.display().to_string(),
        disabled_at: now_ms(),
    });
    if let Err(e) = save_config(data, cfg) {
        cfg.disabled.pop();
        let _ = move_dir(&stored, dir);
        remove_empty(stored.parent());
        return Err(format!("记录停用状态失败，已恢复原位置：{e}"));
    }
    Ok(stored.display().to_string())
}

/// Claude 启用：从停用区移回原位置；原位置已有目录时报冲突
pub(crate) fn unpark(cfg: &mut Config, dir: &Path, data: &Path) -> Result<String, String> {
    let index = cfg.disabled.iter().position(|e| same_path(&e.stored, dir)).ok_or("停用区里没有这份 Skill 的登记")?;
    let entry = cfg.disabled[index].clone();
    let original = PathBuf::from(&entry.original);
    if original.exists() {
        return Err(format!("原位置已有同名目录，未启用：{}", entry.original));
    }
    move_dir(dir, &original)?;
    cfg.disabled.remove(index);
    if let Err(e) = save_config(data, cfg) {
        cfg.disabled.insert(index, entry);
        let _ = move_dir(&original, dir);
        return Err(format!("记录启用状态失败，已放回停用区：{e}"));
    }
    remove_empty(dir.parent());
    Ok(entry.original)
}

// 名称级开关会作用于同名副本；遇到无法解释的配置拒绝改写。
pub(crate) fn json_skill_path(h: &Homes, spec: &ToolSpec, root: &Root) -> PathBuf {
    match (spec.toggle, root.project.as_ref()) {
        (Toggle::GeminiSettings, Some(p)) => p.join(".gemini/settings.json"),
        (Toggle::OpenCodePermission, Some(p)) => p.join("opencode.json"),
        (Toggle::GeminiSettings, None) => tool_home(h,spec).join("settings.json"),
        _ => tool_home(h,spec).join("opencode.json"),
    }
}
pub(crate) fn read_skill_json(path: &Path) -> Result<serde_json::Value,String> {
    // 不把 JSONC 当作空配置覆盖；保留原文件让用户在原工具处理。
    if path.with_extension("jsonc").exists() { return Err("检测到 JSONC 配置，请在原工具中管理 Skills 启停".into()); }
    match fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(s.trim_start_matches('\u{feff}')).map_err(|_|format!("{} 格式无法解析，未修改",path.display())),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(e) => Err(format!("无法读取 {}：{e}",path.display())),
    }
}
pub(crate) fn json_skill_off(h:&Homes, spec:&ToolSpec, root:&Root, name:&str) -> Result<bool,String> {
    let path=json_skill_path(h,spec,root);
    let v=read_skill_json(&path)?;
    let local=match spec.toggle {
        Toggle::GeminiSettings => v.pointer("/skills/enabled")==Some(&serde_json::json!(false)) || v.pointer("/skills/disabled").and_then(|x|x.as_array()).is_some_and(|a|a.iter().any(|n|n.as_str()==Some(name))),
        Toggle::OpenCodePermission => {
            let p=v.pointer("/permission/skill");
            if p.and_then(|p|p.as_object()).is_some_and(|m|m.keys().any(|k|k!="*"&&(k.contains('*')||k.contains('?')))){return Err("存在无法静态判定的 Skills 通配符规则，请在原工具核对".into())}
            let rule=p.and_then(|p|p.get(name)).or_else(||p.and_then(|p|p.get("*"))).or(p.filter(|p|p.is_string())).and_then(|p|p.as_str());
            if rule.is_none() && root.project.is_some(){let mut global=root.clone();global.project=None;return json_skill_off(h,spec,&global,name)}
            rule.is_some_and(|p|p!="allow")
        }, _=>false
    };
    if !local && root.project.is_some() && spec.toggle==Toggle::GeminiSettings {
        let mut global=root.clone();global.project=None; return json_skill_off(h,spec,&global,name);
    }
    Ok(local)
}
pub(crate) fn set_json_skill(h:&Homes,spec:&ToolSpec,root:&Root,name:&str,enabled:bool,data:&Path)->Result<(),String>{
    let path=json_skill_path(h,spec,root); let mut v=read_skill_json(&path)?;
    let object=v.as_object_mut().ok_or("配置根节点必须是对象，未修改")?;
    if spec.toggle==Toggle::GeminiSettings {
        let skills=object.entry("skills").or_insert_with(||serde_json::json!({})).as_object_mut().ok_or("skills 格式无效")?;
        if enabled && skills.get("enabled")==Some(&serde_json::json!(false)){return Err("Skills 总开关已关闭，请在原工具中启用".into())}
        let disabled=skills.entry("disabled").or_insert_with(||serde_json::json!([])).as_array_mut().ok_or("skills.disabled 必须是数组")?;
        disabled.retain(|v|v.as_str()!=Some(name)); if !enabled {disabled.push(serde_json::json!(name));}
        if enabled && root.project.is_some(){let mut global=root.clone();global.project=None;if json_skill_off(h,spec,&global,name)?{return Err("全局配置已停用此 Skill，请先在全局范围启用".into())}}
    }else{
        let permission=object.entry("permission").or_insert_with(||serde_json::json!({})).as_object_mut().ok_or("permission 为全局规则，请在原工具中修改")?;
        let skills=permission.entry("skill").or_insert_with(||serde_json::json!({})).as_object_mut().ok_or("permission.skill 为全局规则，请在原工具中修改")?;
        if skills.keys().any(|k|k!=name&&(k.contains('*')||k.contains('?'))) { return Err("存在通配符权限规则，请在原工具中管理，避免改变规则优先级".into()); }
        if enabled {
            if root.project.is_some(){let mut global=root.clone();global.project=None;if json_skill_off(h,spec,&global,name)?{return Err("全局权限仍限制此 Skill，请在原工具中调整".into())}}
            skills.remove(name);
        }else{skills.insert(name.into(),serde_json::json!("deny"));}
    }
    backup_file(&path,&data.join("skill-backups/config"))?;
    if let Some(parent)=path.parent(){fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
    write_atomic(&path,serde_json::to_string_pretty(&v).map_err(|e|e.to_string())?.as_bytes())
}

/// 启停这一份副本，返回启停后的目录
pub(crate) fn set_enabled_impl(h: &Homes, cfg: &mut Config, root: &Root, dir: &Path, name: &str, enabled: bool, data: &Path) -> Result<String, String> {
    let spec = spec(root.platform).ok_or_else(|| format!("{} 暂不支持启停 Skills", root.platform))?;
    match spec.toggle {
        Toggle::Park => match (enabled, root.parked) {
            (true, true) => unpark(cfg, dir, data),
            (false, false) => park(cfg, root, dir, name, data),
            (true, false) if claude_tool_off(h, spec, root, name) => Err(TOOL_DISABLED.into()),
            _ => Ok(dir.display().to_string()),
        },
        Toggle::CodexToml => {
            let path = codex_config_path(h, spec);
            let (_, names) = codex_disabled(&path);
            if enabled && names.iter().any(|n| n == name) {
                return Err(TOOL_DISABLED.into());
            }
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(format!("无法读取 config.toml：{e}")),
            };
            let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| "config.toml 格式无效，未修改")?;
            let skill_file = dir.join(SKILL_FILE);
            let skills = doc
                .entry("skills")
                .or_insert_with(|| {
                    let mut t = toml_edit::Table::new();
                    t.set_implicit(true);
                    toml_edit::Item::Table(t)
                })
                .as_table_like_mut()
                .ok_or("config.toml 的 skills 不是表，未修改")?;
            let config = skills.entry("config").or_insert(toml_edit::Item::ArrayOfTables(Default::default()));
            let array = config.as_array_of_tables_mut().ok_or("config.toml 的 skills.config 写法不受支持，请手动修改")?;
            // 只动指向这一份 SKILL.md 的 path 条目；name 条目是用户写的，保持原样
            let mut keep = toml_edit::ArrayOfTables::new();
            for table in array.iter() {
                if table.get("path").and_then(|v| v.as_str()).is_some_and(|p| same_path(p, &skill_file)) {
                    continue;
                }
                keep.push(table.clone());
            }
            if !enabled {
                let mut entry = toml_edit::Table::new();
                entry["path"] = toml_edit::value(skill_file.to_string_lossy().into_owned());
                entry["enabled"] = toml_edit::value(false);
                keep.push(entry);
            }
            *array = keep;
            backup_file(&path, &data.join("skill-backups").join("config"))?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            write_atomic(&path, doc.to_string().as_bytes())?;
            Ok(dir.display().to_string())
        }
        Toggle::GeminiSettings | Toggle::OpenCodePermission => { set_json_skill(h,spec,root,name,enabled,data)?; Ok(dir.display().to_string()) },
        Toggle::None => Err(format!("{} 的启停由原工具管理", spec.name)),
    }
}
