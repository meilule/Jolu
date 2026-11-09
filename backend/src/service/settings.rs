//! 设置与本地缓存的只读访问（供 CLI 使用）。
//!
//! 桌面端的 `settings::Store` 带读写与实例标识落盘逻辑；CLI 只需要「读出来是什么」，
//! 因此这里只做只读解析：**不写回、不补 instance_id、不创建目录**。
//! 缺字段仍然按 `Settings` 的 serde 默认值补齐（与桌面端同一套结构体，不会出现两份真相）。

use std::path::Path;

use crate::connections::Creds;
use crate::service::paths;

pub use crate::settings::Settings;

/// 读桌面端的 `settings.json`。文件不存在时返回默认值——调用方据此知道「还没配置过」。
pub fn load() -> Result<Settings, String> {
    let (directory, _) = paths::locate()?;
    Ok(read_settings(&directory))
}

/// 从一个已知数据目录读设置（测试与调用方已定位目录时用）。
pub fn read_settings(directory: &Path) -> Settings {
    std::fs::read_to_string(directory.join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
        .unwrap_or_default()
}

/// models.dev 元数据缓存（桌面端维护的文件）。
///
/// 返回一个查询闭包：给模型名返回元数据。缓存不存在时返回 `None`——CLI 不触发网络刷新，
/// 目录命令只并入已有缓存，行为和桌面端「缓存过期时后台刷新」不同，但都不会阻塞。
pub fn models_meta() -> Option<impl Fn(&str) -> Option<crate::gateway::models_dev::ModelMeta>> {
    let (directory, _) = paths::locate().ok()?;
    let raw = std::fs::read_to_string(directory.join("models-dev.json")).ok()?;
    let catalog: crate::gateway::models_dev::CachedCatalog = serde_json::from_str(&raw).ok()?;
    Some(move |model: &str| crate::gateway::models_dev::lookup_meta(&catalog, model))
}

/// 读一个连接的凭据（CLI 的 `quota` 只用它做路由判断，不打印密钥）。
pub fn creds(conn: &rusqlite::Connection, id: &str) -> Result<Option<Creds>, String> {
    crate::connections::creds_of(conn, id).map_err(|error| format!("读取连接失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_settings_file_yields_defaults_not_an_error() {
        let directory = std::env::temp_dir().join(format!(
            "jolu-svc-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let settings = read_settings(&directory);
        assert_eq!(settings.proxy_port, Settings::default().proxy_port);
        assert!(!directory.exists(), "只读访问不能顺手创建目录");
    }

    #[test]
    fn broken_settings_file_falls_back_to_defaults() {
        let directory = std::env::temp_dir().join(format!(
            "jolu-svc-settings-bad-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("settings.json"), b"{ not json").unwrap();
        let settings = read_settings(&directory);
        assert_eq!(settings.proxy_port, Settings::default().proxy_port);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn settings_file_values_are_honoured() {
        let directory = std::env::temp_dir().join(format!(
            "jolu-svc-settings-ok-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("settings.json"),
            br#"{"proxy_port": 12345, "proxy_mode": "relay"}"#,
        )
        .unwrap();
        let settings = read_settings(&directory);
        assert_eq!(settings.proxy_port, 12345);
        assert_eq!(settings.proxy_mode, "relay");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn missing_models_cache_is_none_not_a_panic() {
        // 没有缓存文件时不 panic、不联网；这里只验证函数在无文件时不炸
        let _ = models_meta();
    }
}
