use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::backup::{has_parent_component, stamp};
use super::edit::{check_project, import_scan_with, ImportScan};
use super::specs::{write_root, Config, Homes, Scope};
use super::SKILL_FILE;

/* ───────────────────────── 从 Git 仓库导入（D-16） ───────────────────────── */
//
// 用本机 git 浅克隆到应用临时目录下的 `jolu-git-*`，再复用上面的导入扫描与冲突检查；
// 导入完成或取消后由前端调用清理命令删除临时目录。只接受不带用户名 / 密码的 `https://` 地址，
// 禁止终端与凭据管理器弹窗（`GIT_TERMINAL_PROMPT=0`、`GCM_INTERACTIVE=never`），需要登录的仓库直接报错。

pub const GIT_MISSING: &str = "未找到 git，请先安装 Git";
pub(crate) const GIT_PREFIX: &str = "jolu-git-";
pub(crate) const GIT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Serialize, Debug)]
pub struct GitImportScan {
    /// 临时克隆目录，导入完成或取消后交给 `skills_import_git_cleanup`
    pub checkout_dir: String,
    /// `scan.source` 是实际导入来源：`<checkout_dir>/<仓库名>`，或仓库不是单个 Skill 时其中的 `skills/` 子目录
    pub scan: ImportScan,
}

/// 只接受 `https://`，不能带用户名或密码，不能有空白或控制字符
pub fn validate_git_url(url: &str) -> Result<(), String> {
    let rest = url.strip_prefix("https://").ok_or("仓库地址只支持 https:// 开头")?;
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("仓库地址不能包含空格".into());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err("仓库地址不能包含用户名或密码".into());
    }
    if authority.is_empty() || authority.starts_with(':') {
        return Err("仓库地址缺少主机名".into());
    }
    Ok(())
}

/// 分支名：不能以 `-` 开头（防止被当成 git 参数），不能有空白、控制字符或 `..`
pub fn validate_git_branch(branch: &str) -> Result<(), String> {
    let bad = branch.is_empty()
        || branch.len() > 200
        || branch.starts_with('-')
        || branch.contains("..")
        || branch.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\'));
    if bad { Err("分支名无效".into()) } else { Ok(()) }
}

/// 应用临时目录 `<系统临时目录>/jolu`，不存在时创建
pub(crate) fn git_temp_root() -> PathBuf {
    let dir = std::env::temp_dir().join("jolu");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// 克隆目录里存放仓库的子目录名：URL 最后一段去掉 `.git`；只保留 `[A-Za-z0-9._-]`，其他字符换成 `_`。
/// 仓库本身就是一个 Skill 时它就是 Skill 的目录名，不符合命名规则的由导入扫描标为「目录名不符合 Skill 名称规则」
pub(crate) fn repo_dir_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default().trim_end_matches('/');
    let last = path.rsplit('/').next().unwrap_or_default();
    let last = last.strip_suffix(".git").unwrap_or(last);
    let name: String = last.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
    if name.is_empty() || name.chars().all(|c| c == '.') { "_".into() } else { name }
}

/// 删除目录树；git 会把对象文件设为只读，Windows 上要先去掉只读属性再删
pub(crate) fn remove_tree(dir: &Path) -> Result<(), String> {
    fn clear_readonly(dir: &Path) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                clear_readonly(&entry.path());
            } else if kind.is_file() {
                if let Ok(meta) = entry.metadata() {
                    let mut perms = meta.permissions();
                    if perms.readonly() {
                        #[allow(clippy::permissions_set_readonly_false)]
                        perms.set_readonly(false);
                        let _ = fs::set_permissions(entry.path(), perms);
                    }
                }
            }
        }
    }
    if fs::remove_dir_all(dir).is_ok() || !dir.exists() {
        return Ok(());
    }
    clear_readonly(dir);
    fs::remove_dir_all(dir).map_err(|e| format!("无法删除临时目录 {}：{e}", dir.display()))
}

/// 只删除 `temp_root` 下直接以 `jolu-git-` 开头的目录；已经不存在时视为成功
pub(crate) fn cleanup_checkout(temp_root: &Path, checkout_dir: &str) -> Result<(), String> {
    const REFUSE: &str = "只能清理 Jolu 拉取 Git 仓库时创建的临时目录";
    let dir = PathBuf::from(checkout_dir);
    if !dir.is_absolute() || has_parent_component(&dir) {
        return Err(REFUSE.into());
    }
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !name.starts_with(GIT_PREFIX) {
        return Err(REFUSE.into());
    }
    let parent = dir.parent().ok_or(REFUSE)?;
    let same_root = match (fs::canonicalize(parent), fs::canonicalize(temp_root)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same_root {
        return Err(REFUSE.into());
    }
    match fs::symlink_metadata(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
        // 符号链接 / junction 指向别处，不跟随删除
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => Err(REFUSE.into()),
        Ok(_) => remove_tree(&dir),
    }
}

/// 结束 git 及其子进程（git-remote-https 等），见 `crate::proc`
pub(crate) fn kill_tree(child: &mut std::process::Child) {
    crate::proc::kill_child_tree(child);
}

/// 把 git 的报错翻成用户能处理的说明，附上原文最后一行
pub(crate) fn git_error(stderr: &str) -> String {
    let last = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).next_back().unwrap_or("git 没有给出原因");
    let lower = stderr.to_lowercase();
    let hint = if lower.contains("remote branch") && lower.contains("not found") {
        "仓库里没有这个分支"
    } else if lower.contains("repository not found") || lower.contains("authentication") || lower.contains("could not read username") || lower.contains("terminal prompts disabled") {
        "无法访问仓库：地址不存在，或仓库需要登录"
    } else if lower.contains("could not resolve host") || lower.contains("failed to connect") || lower.contains("timed out") {
        "无法连接到仓库地址，请检查网络或 Git 的代理设置"
    } else {
        "拉取失败"
    };
    format!("{hint}（{last}）")
}

/// `git clone --depth 1 [--branch B] -- <url> <dest>`，隐藏窗口、禁止凭据弹窗，超时后结束进程树
pub(crate) fn clone_repo(program: &str, url: &str, branch: Option<&str>, dest: &Path, timeout: Duration) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(program);
    // 只允许 https 传输；符号链接按普通文件检出（导入复制时本来也不跟随符号链接）
    command.args(["-c", "protocol.allow=never", "-c", "protocol.https.allow=always", "-c", "core.symlinks=false", "clone", "--depth", "1", "--quiet"]);
    if let Some(branch) = branch {
        command.args(["--branch", branch]);
    }
    command
        .arg("--")
        .arg(url)
        .arg(dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(GIT_MISSING.into()),
        Err(e) => return Err(format!("无法启动 git：{e}")),
    };
    let stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = stderr {
            let _ = pipe.read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                kill_tree(&mut child);
                return Err(format!("拉取超时（{} 秒），已取消", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                kill_tree(&mut child);
                return Err(format!("等待 git 结束失败：{e}"));
            }
        }
    };
    let stderr = reader.join().unwrap_or_default();
    if status.success() { Ok(()) } else { Err(git_error(&stderr)) }
}

pub(crate) fn has_import_candidates(dir: &Path) -> bool {
    dir.join(SKILL_FILE).is_file()
        || fs::read_dir(dir).map(|entries| entries.flatten().any(|e| e.path().join(SKILL_FILE).is_file())).unwrap_or(false)
}

/// 导入来源：仓库本身就是一个 Skill 时用根目录；否则仓库约定的合集目录 `skills/` 里有 Skill 就用它
/// （如 anthropics/skills 根目录还有一个 `template/` 模板，不应混进导入列表）；都没有时用根目录
pub(crate) fn git_import_source(checkout: &Path) -> PathBuf {
    let nested = checkout.join("skills");
    if !checkout.join(SKILL_FILE).is_file() && nested.is_dir() && has_import_candidates(&nested) {
        nested
    } else {
        checkout.to_path_buf()
    }
}

/// 克隆完成后：先删掉 `.git`（不随导入复制进 Skills 目录），再选来源并扫描
pub(crate) fn scan_clone(h: &Homes, cfg: &Config, repo: &Path, platform: &str, scope: Scope, project: Option<&str>) -> Result<ImportScan, String> {
    let git = repo.join(".git");
    match fs::symlink_metadata(&git) {
        Ok(meta) if meta.is_dir() => remove_tree(&git)?,
        Ok(_) => fs::remove_file(&git).map_err(|e| format!("无法删除 {}：{e}", git.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // 拒绝访问等情况无法确认 .git 已不在，中止导入（调用方会删除临时目录）
        Err(e) => return Err(format!("无法确认 .git 已删除：{e}")),
    }
    import_scan_with(h, cfg, &git_import_source(repo).display().to_string(), platform, scope, project)
}

#[allow(clippy::too_many_arguments)]
pub fn import_git_with(h: &Homes, cfg: &Config, temp_root: &Path, program: &str, url: &str, branch: Option<&str>, platform: &str, scope: Scope, project: Option<&str>) -> Result<GitImportScan, String> {
    let url = url.trim();
    validate_git_url(url)?;
    let branch = branch.map(str::trim).filter(|b| !b.is_empty());
    if let Some(b) = branch {
        validate_git_branch(b)?;
    }
    // 先确认目标范围可写，避免白拉一次
    let project_path = check_project(h, cfg, project)?;
    write_root(h, cfg, platform, scope, project_path.as_deref())?;
    fs::create_dir_all(temp_root).map_err(|e| format!("无法创建临时目录：{e}"))?;
    // 克隆到 `jolu-git-<时间戳>/<仓库名>`：仓库本身是 Skill 时目录名就是仓库名，重复导入能命中冲突检查
    let dest = temp_root.join(format!("{GIT_PREFIX}{}", stamp()));
    let repo = dest.join(repo_dir_name(url));
    let result = fs::create_dir_all(&dest)
        .map_err(|e| format!("无法创建临时目录：{e}"))
        .and_then(|_| clone_repo(program, url, branch, &repo, GIT_TIMEOUT))
        .and_then(|_| scan_clone(h, cfg, &repo, platform, scope, project));
    match result {
        Ok(scan) => Ok(GitImportScan { checkout_dir: dest.display().to_string(), scan }),
        Err(reason) => {
            let _ = remove_tree(&dest);
            Err(reason)
        }
    }
}
