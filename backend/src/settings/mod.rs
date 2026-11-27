//! 需要持久化的运行时设置存储（`Store` 与 `Snapshot`）。
//!
//! 纯配置结构与校验函数位于 [`types`]（零业务模块依赖），此处只负责原子落盘、
//! `.lastgood` 恢复、修订号递增与外部进程变更读回。

pub mod types;
pub use types::*;

use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

/// 16 字节随机数的十六进制表示；Windows 用系统加密随机数，其他平台退回时间 + 进程号。
fn new_instance_id() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::Cryptography::{
            BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        };
        let mut output = [0u8; 16];
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                output.as_mut_ptr(),
                output.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status >= 0 {
            return output.iter().map(|b| format!("{b:02x}")).collect();
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{now:032x}{:08x}", std::process::id())[..32].to_string()
}

/// 落盘用的序列化：搜索 Key 输出原文。其他任何序列化（命令返回、事件）都是掩码。
fn persisted_json(settings: &Settings) -> serde_json::Result<Vec<u8>> {
    types::secret_keys::reveal(|| serde_json::to_vec_pretty(settings))
}

/// 从已知数据目录只读解析 `settings.json`（不写回、不补 `instance_id`、不建目录）。
pub fn read_readonly(directory: &std::path::Path) -> Settings {
    std::fs::read_to_string(directory.join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
        .unwrap_or_default()
}

pub struct Store {
    path: PathBuf,
    inner: Mutex<Settings>,
    revision: std::sync::atomic::AtomicU64,
    disk: Mutex<Vec<u8>>,
}

/// 发给前端的设置快照：设置本身（字段平铺，搜索 Key 照常在序列化时掩码）加修订号。
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    #[serde(flatten)]
    pub settings: Settings,
    pub revision: u64,
}

impl Store {
    pub fn load(dir: &std::path::Path) -> Self {
        let path = dir.join("settings.json");
        let parse = |text: &str| serde_json::from_str::<Settings>(text).ok();
        let mut inner = match crate::steady::read_to_string(&path) {
            Ok(text) => match parse(&text) {
                Some(settings) => settings,
                None => {
                    let _ = std::fs::write(path.with_extension("json.corrupt"), text.as_bytes());
                    let restored =
                        crate::steady::read_to_string(path.with_extension("json.lastgood"))
                            .ok()
                            .and_then(|text| parse(&text));
                    eprintln!(
                        "[设置] settings.json 无法解析，已另存为 settings.json.corrupt；{}",
                        if restored.is_some() {
                            "已回读上一版设置"
                        } else {
                            "没有可用的上一版，使用默认设置"
                        }
                    );
                    restored.unwrap_or_default()
                }
            },
            Err(_) => Settings::default(),
        };
        let mut disk = crate::steady::read(&path).unwrap_or_default();
        if inner.instance_id.trim().is_empty() {
            inner.instance_id = new_instance_id();
            if let Ok(json) = persisted_json(&inner) {
                let _ = std::fs::create_dir_all(dir);
                if std::fs::write(&path, &json).is_ok() {
                    disk = json;
                }
            }
        }
        Self {
            path,
            inner: Mutex::new(inner),
            revision: std::sync::atomic::AtomicU64::new(1),
            disk: Mutex::new(disk),
        }
    }

    fn external(&self) -> Option<(Settings, Vec<u8>)> {
        let bytes = crate::steady::read(&self.path).ok()?;
        let known = self.disk.lock().ok()?;
        if bytes == *known {
            return None;
        }
        drop(known);
        let settings = serde_json::from_slice::<Settings>(&bytes).ok()?;
        Some((settings, bytes))
    }

    pub fn dir(&self) -> &std::path::Path {
        self.path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
    }

    pub fn reload_external(&self) -> bool {
        let Ok(_lock) =
            crate::filelock::FileLock::acquire(&self.path, std::time::Duration::from_secs(2))
        else {
            return false;
        };
        let Some((settings, bytes)) = self.external() else {
            return false;
        };
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        *guard = settings;
        if let Ok(mut known) = self.disk.lock() {
            *known = bytes;
        }
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        true
    }

    pub fn get(&self) -> Settings {
        self.inner.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn snapshot(&self) -> Snapshot {
        match self.inner.lock() {
            Ok(guard) => Snapshot {
                settings: guard.clone(),
                revision: self.revision.load(std::sync::atomic::Ordering::Acquire),
            },
            Err(_) => Snapshot {
                settings: Settings::default(),
                revision: 0,
            },
        }
    }

    pub fn update(&self, f: impl FnOnce(&mut Settings)) -> Settings {
        match self.try_update(f) {
            Ok(next) => next,
            Err(error) => {
                eprintln!("[设置] {error}");
                self.get()
            }
        }
    }

    pub fn try_update(&self, f: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
        use std::io::Write;
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| format!("设置锁获取失败：{e}"))?;
        let _lock =
            crate::filelock::FileLock::acquire(&self.path, std::time::Duration::from_secs(5))?;
        let mut next = match self.external() {
            Some((settings, _)) => settings,
            None => guard.clone(),
        };
        f(&mut next);
        let json = persisted_json(&next).map_err(|e| format!("设置序列化失败：{e}"))?;
        let temporary = self.path.with_extension("json.tmp");
        let write = || -> std::io::Result<()> {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::File::create(&temporary)?;
            file.write_all(&json)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, &self.path)
        };
        if let Err(error) = write() {
            let _ = std::fs::remove_file(&temporary);
            return Err(format!("设置保存失败：{error}"));
        }
        let _ = std::fs::write(self.path.with_extension("json.lastgood"), &json);
        if let Ok(mut known) = self.disk.lock() {
            *known = json;
        }
        *guard = next.clone();
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_search_keys() -> Settings {
        let mut settings = Settings::default();
        settings.search_apis = vec![SearchApiConfig {
            vendor: "tavily".into(),
            keys: vec!["tvly-abcdefgh12345678".into(), "short".into()],
            url: None,
            off: false,
        }];
        settings
    }

    #[test]
    fn any_serialization_outside_the_store_masks_search_keys() {
        let json = serde_json::to_string(&with_search_keys()).unwrap();
        assert!(!json.contains("tvly-abcdefgh12345678"), "{json}");
        assert!(!json.contains("short"), "{json}");
        let value = serde_json::to_value(with_search_keys()).unwrap();
        assert_eq!(value["search_apis"][0]["keys"][0], "tvly••••5678");
        assert_eq!(value["search_apis"][0]["keys"][1], "••••••••");
        assert_eq!(value["search_apis"][0]["vendor"], "tavily");
    }

    #[test]
    fn snapshot_revision_tracks_successful_saves_and_keeps_masking() {
        let dir = std::env::temp_dir().join(format!("jolu-settings-revision-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::load(&dir);
        let first = store.snapshot().revision;
        store.try_update(|s| s.search_apis = with_search_keys().search_apis).unwrap();
        let after_one = store.snapshot();
        assert_eq!(after_one.revision, first + 1);
        store.try_update(|s| s.dnd = !s.dnd).unwrap();
        assert_eq!(store.snapshot().revision, first + 2);
        let json = serde_json::to_value(store.snapshot()).unwrap();
        assert_eq!(json["revision"], first + 2);
        assert!(json.get("dnd").is_some(), "{json}");
        assert_eq!(json["search_apis"][0]["keys"][0], "tvly••••5678");
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::create_dir_all(dir.join("settings.json")).unwrap();
        assert!(store.try_update(|s| s.dnd = !s.dnd).is_err());
        assert_eq!(store.snapshot().revision, first + 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_store_persists_search_keys_in_plain_text_and_reads_them_back() {
        let dir = std::env::temp_dir().join(format!("jolu-settings-search-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::load(&dir);
        let returned = store.try_update(|s| s.search_apis = with_search_keys().search_apis).unwrap();
        assert_eq!(returned.search_apis[0].keys[0], "tvly-abcdefgh12345678");
        let disk = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(disk.contains("tvly-abcdefgh12345678"));
        let last_good = std::fs::read_to_string(dir.join("settings.json.lastgood")).unwrap();
        assert!(last_good.contains("tvly-abcdefgh12345678"));
        assert_eq!(Store::load(&dir).get().search_apis[0].keys[1], "short");
        assert!(!serde_json::to_string(&store.get()).unwrap().contains("tvly-abcdefgh12345678"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gateway_fields_default_when_missing_from_old_settings() {
        let old: Settings = serde_json::from_str(
            r#"{"island_platform":"claude","dnd":false,"island_visible":true,"proxy_enabled":false}"#,
        )
        .unwrap();
        assert_eq!(old.proxy_mode, "observe");
        assert_eq!(old.gateway_default_connection, None);
        assert_eq!(old.gateway_circuit_threshold, 5);
        assert!(old.gateway_auto_escape);
        assert!(!old.gateway_shared_lan);
        assert!(old.gateway_wiring.is_empty());
    }

    #[test]
    fn gateway_settings_roundtrip() {
        let mut s = Settings::default();
        s.proxy_mode = "relay".into();
        s.gateway_default_connection = Some("c1".into());
        s.gateway_circuit_threshold = 3;
        s.gateway_auto_escape = false;
        s.gateway_shared_lan = true;
        s.gateway_wiring.insert("claude-code".into(), r#"{"base":"https://x"}"#.into());
        let v = serde_json::to_value(&s).unwrap();
        let back: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(back.proxy_mode, "relay");
        assert_eq!(back.gateway_default_connection.as_deref(), Some("c1"));
        assert_eq!(back.gateway_circuit_threshold, 3);
        assert!(!back.gateway_auto_escape);
        assert!(back.gateway_shared_lan);
        assert_eq!(back.gateway_wiring.get("claude-code").map(String::as_str), Some(r#"{"base":"https://x"}"#));
    }

    #[test]
    fn unreadable_settings_fall_back_to_last_good_copy_and_keep_the_broken_file() {
        let dir = std::env::temp_dir().join(format!("jolu-settings-lastgood-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::load(&dir);
        store.try_update(|s| { s.theme = "dark".into(); s.island_platform = "codex".into(); }).unwrap();
        assert!(dir.join("settings.json.lastgood").exists());
        std::fs::write(dir.join("settings.json"), "{\"theme\": \"da").unwrap();
        let restored = Store::load(&dir).get();
        assert_eq!(restored.theme, "dark");
        assert_eq!(restored.island_platform, "codex");
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.corrupt")).unwrap(), "{\"theme\": \"da");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unreadable_settings_without_last_good_copy_still_keep_the_broken_file() {
        let dir = std::env::temp_dir().join(format!("jolu-settings-no-lastgood-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), "not json").unwrap();
        let store = Store::load(&dir);
        assert_eq!(store.get().theme, "light");
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.corrupt")).unwrap(), "not json");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_save_keeps_previous_settings() {
        let dir = std::env::temp_dir().join(format!("island-settings-failure-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("settings.json")).unwrap();
        let store = Store::load(&dir);
        assert!(store.try_update(|s| s.theme = "dark".into()).is_err());
        assert_eq!(store.get().theme, "light");
        std::fs::remove_dir(dir.join("settings.json")).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_roundtrip_preserves_all_preferences() {
        let dir = std::env::temp_dir().join(format!("island-settings-roundtrip-{}", std::process::id()));
        let store = Store::load(&dir);
        store.try_update(|s| {
            s.theme = "dark".into(); s.dnd = true; s.dock_enabled = false;
            s.always_on_top = false; s.silent_startup = false;
            s.island_opacity = 70; s.island_scale = 115; s.island_shrink_scale = 125; s.refresh_minutes = 15;
            s.retention_days = Some(90); s.balance_alert_threshold = Some(12.5);
            s.balance_alert_currency = "CNY".into();
            s.island_connection_id = Some("test-connection".into());
            s.island_source_id = Some("local:codex".into()); s.island_platform = "codex".into();
        }).unwrap();
        store.try_update(|s| s.theme = "system".into()).unwrap();
        assert_eq!(serde_json::to_value(store.get()).unwrap(), serde_json::to_value(Store::load(&dir).get()).unwrap());
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn upgrade_keeps_saved_island_connection_when_other_fields_are_missing() {
        let dir = std::env::temp_dir().join(format!("island-upgrade-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            r#"{"island_platform":"codex","island_kind":"auth","island_connection_id":"saved-account","island_connection_name":"上次使用的连接"}"#,
        ).unwrap();
        let store = Store::load(&dir);
        assert_eq!(store.get().island_connection_id.as_deref(), Some("saved-account"));
        assert!(store.get().island_visible);
        store.try_update(|s| s.theme = "dark".into()).unwrap();
        let restored = Store::load(&dir).get();
        assert_eq!(restored.island_connection_id.as_deref(), Some("saved-account"));
        assert_eq!(restored.island_connection_name.as_deref(), Some("上次使用的连接"));
        assert_eq!(restored.island_platform, "codex");
        assert_eq!(restored.island_kind, "auth");
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_updates_keep_disk_and_memory_in_order() {
        let dir = std::env::temp_dir().join(format!("island-settings-concurrent-{}", std::process::id()));
        let store = std::sync::Arc::new(Store::load(&dir));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for _ in 0..8 {
                        store.try_update(|s| s.dock.offset += 1.0).unwrap();
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(store.get().dock.offset, 32.0);
        assert_eq!(Store::load(&dir).get().dock.offset, 32.0);
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn display_preferences_validate_and_old_settings_keep_defaults() {
        assert!(validate_display_preferences(60, 85, 100, 1).is_ok());
        assert!(validate_display_preferences(100, 115, 125, 30).is_ok());
        for (opacity, scale, shrink, refresh) in [
            (0, 100, 100, 5),
            (101, 100, 100, 5),
            (100, 0, 100, 5),
            (100, 100, 90, 5),
            (100, 100, 100, 0),
        ] {
            assert!(validate_display_preferences(opacity, scale, shrink, refresh).is_err());
        }
        let old = r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#;
        let settings: Settings = serde_json::from_str(old).unwrap();
        assert!(settings.silent_startup);
        assert_eq!(
            (settings.island_opacity, settings.island_scale, settings.island_shrink_scale, settings.refresh_minutes),
            (100, 100, 100, 5)
        );
    }

    #[test]
    fn island_style_defaults_to_classic_and_only_accepts_known_values() {
        let old: Settings = serde_json::from_str(r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#).unwrap();
        assert_eq!(old.island_style, "classic");
        assert!(validate_island_style("classic").is_ok());
        assert!(validate_island_style("capsule").is_ok());
        for bad in ["", "Capsule", "mini", "classic "] {
            assert!(validate_island_style(bad).is_err(), "{bad:?}");
        }
        let dir = std::env::temp_dir().join(format!("island-style-roundtrip-{}", std::process::id()));
        let store = Store::load(&dir);
        store.try_update(|s| s.island_style = "capsule".into()).unwrap();
        assert_eq!(Store::load(&dir).get().island_style, "capsule");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn older_settings_keep_docking_enabled_and_optional_connection_name_empty() {
        let settings: Settings = serde_json::from_str(
            r#"{"island_platform":"codex","island_kind":"auth","island_connection_id":null,"island_source_id":null,"always_on_top":true,"dnd":false,"island_visible":true,"dock":{"edge":null,"offset":0.0,"monitor":null}}"#,
        )
        .expect("旧设置应可迁移");
        assert!(settings.dock_enabled);
        assert!(settings.island_horizontal_dock_full);
        assert_eq!(settings.island_linkage, IslandLinkage::default());
        assert_eq!(settings.island_linkage.quota_warn_percent, 80);
        assert!(settings.island_linkage.poke_reaction);
        assert!(settings.island_linkage.props_theater);
        assert!(settings.island_linkage.daily_recap);
        assert_eq!(normalize_island_linkage(IslandLinkage { quota_warn_percent: 30, ..Default::default() }).quota_warn_percent, 60);
        assert_eq!(normalize_island_linkage(IslandLinkage { quota_warn_percent: 99, ..Default::default() }).quota_warn_percent, 95);
        assert_eq!(settings.theme, "light");
        assert_eq!(settings.island_connection_name, None);
    }

    #[test]
    fn auto_update_preference_defaults_on_and_survives_reload() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert!(old.auto_check_updates);
        let dir = std::env::temp_dir().join(format!("jolu-auto-update-settings-{}", std::process::id()));
        let store = Store::load(&dir);
        for enabled in [false, true] {
            store.try_update(|settings| settings.auto_check_updates = enabled).unwrap();
            assert_eq!(Store::load(&dir).get().auto_check_updates, enabled);
        }
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn silent_startup_choice_survives_reload_without_changing_island_visibility() {
        let dir = std::env::temp_dir().join(format!("island-startup-settings-{}", std::process::id()));
        let store = Store::load(&dir);
        for on in [false, true] {
            store.update(|settings| settings.silent_startup = on);
            let restored = Store::load(&dir).get();
            assert_eq!(restored.silent_startup, on);
            assert!(restored.island_visible);
        }
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn default_page_rejects_unknown() {
        assert!(validate_default_page("bogus").is_err());
        assert!(validate_default_page("").is_err());
        for page in ["last", "island", "providers", "checkin", "usage", "sessions", "automation", "skills", "agents", "gateway", "settings"] {
            assert!(validate_default_page(page).is_ok(), "{page} 应当可选");
        }
        assert!(validate_last_page("last").is_err());
        assert!(validate_last_page("skills").is_ok());
    }

    #[test]
    fn old_settings_default_to_last_visited_page() {
        let settings: Settings = serde_json::from_str(r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#).unwrap();
        assert_eq!(settings.default_page, "last");
        assert_eq!(settings.last_page, None);
        assert_eq!(Settings::default().default_page, "last");
    }

    #[test]
    fn appearance_validation() {
        for zoom in [80, 90, 100, 110, 125, 150] {
            assert!(validate_appearance(zoom, "comfortable").is_ok());
        }
        for zoom in [0, 75, 99, 115, 175, 200] {
            assert!(validate_appearance(zoom, "comfortable").is_err());
        }
        assert!(validate_appearance(100, "compact").is_ok());
        for density in ["", "Compact", "dense"] {
            assert!(validate_appearance(100, density).is_err());
        }
    }

    #[test]
    fn saved_zoom_outside_steps_falls_back_to_standard() {
        for zoom in [80, 90, 100, 110, 125, 150] {
            assert_eq!(normalize_zoom(zoom), zoom);
        }
        for zoom in [0, 75, 115, 300, u16::MAX] {
            assert_eq!(normalize_zoom(zoom), 100);
        }
    }

    #[test]
    fn old_settings_default_to_standard_appearance() {
        let settings: Settings = serde_json::from_str(r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#).unwrap();
        assert_eq!((settings.main_zoom, settings.list_density.as_str(), settings.reduce_motion), (100, "comfortable", false));
        let defaults = Settings::default();
        assert_eq!((defaults.main_zoom, defaults.list_density.as_str(), defaults.reduce_motion), (100, "comfortable", false));
    }

    #[test]
    fn old_settings_default_task_notices_on() {
        let settings: Settings = serde_json::from_str(r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#).unwrap();
        assert!(settings.notify_task_resumed && settings.notify_task_finished);
        let defaults = Settings::default();
        assert!(defaults.notify_task_resumed && defaults.notify_task_finished);
    }

    #[test]
    fn privacy_masking_defaults_off_and_survives_reload() {
        let old: Settings = serde_json::from_str(r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#).unwrap();
        assert!(!old.privacy_masking);
        assert!(!Settings::default().privacy_masking);
        let dir = std::env::temp_dir().join(format!("jolu-privacy-masking-settings-{}", std::process::id()));
        let store = Store::load(&dir);
        store.try_update(|settings| settings.privacy_masking = true).unwrap();
        assert!(Store::load(&dir).get().privacy_masking);
        let back: Settings = serde_json::from_value(serde_json::to_value(store.get()).unwrap()).unwrap();
        assert!(back.privacy_masking);
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn zcode_free_first_defaults_off_and_survives_reload() {
        let old: Settings = serde_json::from_str(r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#).unwrap();
        assert!(!old.zcode_free_first);
        assert!(!Settings::default().zcode_free_first);
        let dir = std::env::temp_dir().join(format!("jolu-zcode-free-first-settings-{}", std::process::id()));
        let store = Store::load(&dir);
        store.try_update(|settings| settings.zcode_free_first = true).unwrap();
        assert!(Store::load(&dir).get().zcode_free_first);
        let back: Settings = serde_json::from_value(serde_json::to_value(store.get()).unwrap()).unwrap();
        assert!(back.zcode_free_first);
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
