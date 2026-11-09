//! 桌面端数据目录的**只读**推断（供 CLI 使用）。
//!
//! 桌面端通过 Tauri 的 `app_data_dir()` 拿目录，CLI 没有 Tauri 上下文，只能按同一套
//! 约定自己算。规则必须与 `tauri.conf.json` 的 `identifier`（`dev.ningz.jolu`）保持一致：
//!
//! - Windows：`%APPDATA%\dev.ningz.jolu`（Tauri 用 Roaming）
//! - macOS：`~/Library/Application Support/dev.ningz.jolu`
//! - 其他：`$XDG_DATA_HOME/dev.ningz.jolu` 或 `~/.local/share/dev.ningz.jolu`
//!
//! 另外支持 `JOLU_DATA_DIR` 显式覆盖（验收与多实例场景），以及 debug 构建下的
//! `JOLU_TEST_DIR`（与 `validation::data_dir` 同语义）。
//!
//! 这里**不创建目录、不建库**：CLI 只读已有数据；目录不存在时如实报「找不到」，
//! 不会顺手在那里生成一个空数据库冒充「没有用量」。

use std::path::PathBuf;

/// Tauri 的 identifier，与 `tauri.conf.json` 一致。
const IDENTIFIER: &str = "dev.ningz.jolu";

/// 数据目录与数据库文件。
pub fn locate() -> Result<(PathBuf, PathBuf), String> {
    locate_with(|name| std::env::var_os(name))
}

/// 具体实现：环境变量通过参数注入，便于测试不碰进程级环境。
///
/// 用注入而不是直接读 env 的原因：Rust 测试默认并行，多个测试同时 `set_var` 会互相
/// 干扰（进程级状态），而且 `set_var` 在多线程下本身有未定义行为风险。
pub fn locate_with(lookup: impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<(PathBuf, PathBuf), String> {
    // 验收 / 隔离：与桌面端同一套覆盖变量
    #[cfg(debug_assertions)]
    if let Some(value) = lookup("JOLU_TEST_DIR") {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(format!("JOLU_TEST_DIR 必须是绝对路径：{}", path.display()));
        }
        return Ok((path.clone(), path.join("usage.db")));
    }
    if let Some(value) = lookup("JOLU_DATA_DIR") {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(format!("JOLU_DATA_DIR 必须是绝对路径：{}", path.display()));
        }
        let database = path.join("usage.db");
        // 覆盖目录同样要求库已存在：CLI 只读，不能在没有库时假装「没有用量」
        if !database.is_file() {
            return Err(format!(
                "找不到 Jolu 数据：{}。请先运行一次桌面端，或检查 JOLU_DATA_DIR。",
                database.display()
            ));
        }
        return Ok((path, database));
    }
    let base = platform_base_with(&lookup)?;
    let directory = base.join(IDENTIFIER);
    let database = directory.join("usage.db");
    if !database.is_file() {
        return Err(format!(
            "找不到 Jolu 数据：{}。请先运行一次桌面端，或用 JOLU_DATA_DIR 指定数据目录。",
            database.display()
        ));
    }
    Ok((directory, database))
}

#[cfg(windows)]
fn platform_base_with(lookup: &impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<PathBuf, String> {
    lookup("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "读不到 %APPDATA%，无法定位 Jolu 数据目录".to_string())
}

#[cfg(target_os = "macos")]
fn platform_base_with(lookup: &impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<PathBuf, String> {
    let home = lookup("HOME").map(PathBuf::from)
        .ok_or_else(|| "读不到 $HOME，无法定位 Jolu 数据目录".to_string())?;
    Ok(home.join("Library").join("Application Support"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_base_with(lookup: &impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<PathBuf, String> {
    if let Some(xdg) = lookup("XDG_DATA_HOME") {
        return Ok(PathBuf::from(xdg));
    }
    let home = lookup("HOME").map(PathBuf::from)
        .ok_or_else(|| "读不到 $HOME，无法定位 Jolu 数据目录".to_string())?;
    Ok(home.join(".local").join("share"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    /// 用注入的查找函数构造环境，不碰进程级 env（测试并行时才会稳定）。
    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| OsString::from(value))
    }

    #[test]
    fn relative_override_is_rejected_instead_of_silently_resolved() {
        // 相对路径在不同 cwd 下指向不同库，必须拒绝
        let error = locate_with(env(&[("JOLU_DATA_DIR", "relative/path")])).unwrap_err();
        assert!(error.contains("绝对路径"), "{error}");
    }

    #[test]
    fn missing_database_is_reported_not_created() {
        let missing = std::env::temp_dir().join(format!(
            "jolu-cli-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let owned = missing.to_string_lossy().to_string();
        let lookup = move |name: &str| {
            (name == "JOLU_DATA_DIR").then(|| OsString::from(owned.clone()))
        };
        let error = locate_with(lookup).unwrap_err();
        assert!(error.contains("找不到 Jolu 数据"), "{error}");
        assert!(!missing.exists(), "CLI 不能顺手创建目录或空库");
    }

    #[test]
    fn existing_database_under_override_is_accepted() {
        let directory = std::env::temp_dir().join(format!(
            "jolu-cli-ok-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("usage.db"), b"").unwrap();
        let owned = directory.to_string_lossy().to_string();
        let lookup = move |name: &str| (name == "JOLU_DATA_DIR").then(|| OsString::from(owned.clone()));
        let (found_dir, database) = locate_with(lookup).unwrap();
        assert_eq!(found_dir, directory);
        assert!(database.ends_with("usage.db"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn test_dir_override_takes_precedence_and_skips_the_existence_check() {
        // JOLU_TEST_DIR 是验收专用：桌面端自己会建库，CLI 侧不要求库已存在
        let directory = std::env::temp_dir().join("jolu-cli-testdir");
        let owned = directory.to_string_lossy().to_string();
        let lookup = move |name: &str| (name == "JOLU_TEST_DIR").then(|| OsString::from(owned.clone()));
        let (found_dir, _) = locate_with(lookup).unwrap();
        assert_eq!(found_dir, directory);
    }
}
