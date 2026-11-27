#![allow(unused_imports)]
//! 模块 F · Skills 管理（画板 74–85）：按供应商扫描本机 Skill 目录，查看、新建 / 编辑、导入、复制、启停与移除。

use std::time::Duration;

pub(crate) mod backup;
pub(crate) mod edit;
pub(crate) mod git_import;
pub(crate) mod scan;
pub(crate) mod specs;
#[cfg(test)]
mod tests;
pub(crate) mod toggle;

pub use edit::{
    copy_with, detail_with, file_with, ignore_project_with, import_scan_with, import_with,
    remove_with, save_dir_settings_with, save_with, set_global_dir_with, set_projects_with,
    CopyResult, Detail, DirSettings, FileContent, FileEntry, GlobalChange, IgnoreChange,
    ImportChoice, ImportItem, ImportResult, ImportScan, SaveInput, SaveResult,
};
pub(crate) use edit::{blocking, context, context_mut};
pub use git_import::{
    import_git_with, validate_git_branch, validate_git_url, GitImportScan, GIT_MISSING,
};
pub(crate) use git_import::{cleanup_checkout, git_temp_root};
pub use scan::{
    enabled_skills, enabled_skills_with, overview_with, Overview, PlatformSummary, RootInfo,
    ScanError, SkillItem, SkillOption,
};
pub use specs::{
    norm_key, parse_front, spec, valid_name, Config, DisabledEntry, Front, Homes, ProjectInfo,
    Scope, Toggle, ToolSpec, SPECS,
};
pub(crate) use backup::locate;
pub(crate) use specs::{config_guard, data_dir, homes, load_config, save_config, skill_name};
pub(crate) use toggle::set_enabled_impl;

#[cfg(test)]
pub(crate) use edit::check_project;
#[cfg(test)]
pub(crate) use backup::{backup_dir_tree, backup_file, copy_dir, ensure_writable, has_parent_component, overwrite_dir, safe_rel, stamp, write_atomic};
#[cfg(test)]
pub(crate) use git_import::{clone_repo, git_error, git_import_source, has_import_candidates, remove_tree, repo_dir_name, scan_clone, GIT_PREFIX, GIT_TIMEOUT};
#[cfg(test)]
pub(crate) use scan::{claude_override, claude_tool_off, codex_config_path, codex_disabled, collect, is_symlink, modified_ms, now_ms, read_json_object, read_limited, scan_root, source_of, Raw};
#[cfg(test)]
pub(crate) use specs::{broken_override, broken_override_reason, default_global, env_default, homes_from, isolated_homes, own_global, read_session_projects, rel, roots, same_path, session_projects, tool_home, validate_front, write_root, Root};
#[cfg(test)]
pub(crate) use toggle::{has_symlink_inside, json_skill_off, json_skill_path, move_dir, park, read_skill_json, remove_empty, set_json_skill, unpark};

pub(crate) const SKILL_FILE: &str = "SKILL.md";
pub(crate) const MAX_SKILL_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_PREVIEW_BYTES: u64 = 512 * 1024;
pub(crate) const MAX_FILES: usize = 200;
pub const EXTERNAL_CHANGE: &str = "external_change";
pub(crate) const TOOL_DISABLED: &str =
    "由工具配置停用：这条配置按名称生效，会影响所有同名副本；Jolu 不改动你的配置，请在配置文件中修改";
pub(crate) const SESSION_TTL: Duration = Duration::from_secs(600);
