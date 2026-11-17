//! CLI 写设置（5.5，`jolu settings set`）：只开放「用到时才读」的简单键——改了以后桌面端
//! 2 秒内读回（`settings::Store::reload_external`）就能生效，不需要重启监听、改系统设置。
//! 端口、开机自启、保持唤醒这类要做副作用的键不在白名单里，仍然在桌面端改。
//! 网关的默认来源、熔断阈值、自动逃生原先在模型路由启动时取快照，改了要重启才生效；2026-11-17 改成
//! 每次读当前设置后，阈值与自动逃生重新开放。默认来源要校验「正在对外服务」，走 [`set_default_source`]。
//!
//! 写法：拿文件锁（与桌面端同一把，`filelock`）→ 读 JSON → 只改这一个键 → 整份按 `Settings`
//! 反序列化校验 → 临时文件 + 改名写回，并留 `.lastgood`。只改这一个键，别的内容（含搜索 Key 原文）原样保留。

use std::path::Path;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    /// 闭区间
    Int(u32, u32),
    /// 可以清空的数
    OptNumber,
    /// 主窗口默认打开的页面
    Page,
}

pub struct Settable {
    pub key: &'static str,
    pub kind: Kind,
    pub about: &'static str,
}

pub const SETTABLE: &[Settable] = &[
    Settable { key: "dnd", kind: Kind::Bool, about: "免打扰（灵动岛与系统通知不打扰）" },
    Settable { key: "reduce_motion", kind: Kind::Bool, about: "减少动效" },
    Settable { key: "notify_task_resumed", kind: Kind::Bool, about: "自动化任务续跑时通知" },
    Settable { key: "notify_task_finished", kind: Kind::Bool, about: "自动化任务完成时通知" },
    Settable { key: "notify_quota_alerts", kind: Kind::Bool, about: "额度提醒通知" },
    Settable { key: "privacy_masking", kind: Kind::Bool, about: "经网关的请求做隐私脱敏" },
    Settable { key: "zcode_free_first", kind: Kind::Bool, about: "ZCode 先试免费 Start Plan 通道" },
    Settable { key: "gateway_auto_escape", kind: Kind::Bool, about: "网关连续失败时自动把工具切回直连" },
    Settable { key: "gateway_circuit_threshold", kind: Kind::Int(1, 100), about: "连续失败几次后熔断" },
    Settable { key: "warmup_hour", kind: Kind::Int(0, 23), about: "每日 warm-up 的触发小时" },
    Settable { key: "balance_alert_threshold", kind: Kind::OptNumber, about: "余额提醒阈值（none = 关闭）" },
    Settable { key: "default_page", kind: Kind::Page, about: "主窗口默认打开的页面（last = 上次访问的）" },
];

pub fn settable(key: &str) -> Option<&'static Settable> {
    SETTABLE.iter().find(|s| s.key == key)
}

fn parse_value(item: &Settable, raw: &str) -> Result<Value, String> {
    let raw = raw.trim();
    let clear = matches!(raw.to_ascii_lowercase().as_str(), "none" | "null" | "");
    match item.kind {
        Kind::Bool => match raw.to_ascii_lowercase().as_str() {
            "true" | "on" | "1" | "yes" => Ok(Value::Bool(true)),
            "false" | "off" | "0" | "no" => Ok(Value::Bool(false)),
            _ => Err(format!("{} 要写 on / off", item.key)),
        },
        Kind::Int(min, max) => match raw.parse::<u32>() {
            Ok(n) if (min..=max).contains(&n) => Ok(Value::from(n)),
            _ => Err(format!("{} 要写 {min}–{max} 的整数", item.key)),
        },
        Kind::OptNumber if clear => Ok(Value::Null),
        Kind::OptNumber => match raw.parse::<f64>() {
            Ok(n) if n.is_finite() && n >= 0.0 => Ok(Value::from(n)),
            _ => Err(format!("{} 要写非负数，或 none", item.key)),
        },
        Kind::Page => {
            crate::settings::validate_default_page(raw)?;
            Ok(Value::String(raw.to_string()))
        }
    }
}

fn display(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "（未设置）".into(),
        Some(Value::Bool(true)) => "on".into(),
        Some(Value::Bool(false)) => "off".into(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// 读一个可写键的当前值（文件里没有时按默认值）
pub fn get(directory: &Path, key: &str) -> Result<String, String> {
    let item = settable(key).ok_or_else(|| format!("「{key}」不能用命令行改；`jolu settings list` 看可改的键"))?;
    let settings = crate::service::settings::read_settings(directory);
    let value = serde_json::to_value(&settings).map_err(|e| e.to_string())?;
    Ok(display(value.get(item.key)))
}

/// 改一个键；返回（旧值，新值）的显示文字
pub fn set(directory: &Path, key: &str, raw: &str) -> Result<(String, String), String> {
    let item = settable(key).ok_or_else(|| format!("「{key}」不能用命令行改；`jolu settings list` 看可改的键"))?;
    let value = parse_value(item, raw)?;
    write_key(directory, item.key, value)
}

/// 设默认来源（同桌面端「设为默认来源」的校验：只接受正在对外服务、有前缀的连接）；`None` 清除
pub fn set_default_source(directory: &Path, conn: &rusqlite::Connection, id: Option<&str>) -> Result<(String, String), String> {
    if let Some(id) = id {
        let serving: Option<(String, Option<String>)> = conn
            .query_row("SELECT name, slug FROM connections WHERE id = ?1 AND serve_enabled = 1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .ok();
        match serving {
            None => return Err("该来源不在模型池中：先 `jolu provider serve <id> on`，再设为默认来源".into()),
            Some((name, slug)) if slug.as_deref().unwrap_or("").is_empty() => return Err(format!("来源「{name}」还没有 id，无法作为默认来源")),
            Some(_) => {}
        }
    }
    write_key(directory, "gateway_default_connection", id.map(|id| Value::String(id.into())).unwrap_or(Value::Null))
}

fn write_key(directory: &Path, key: &str, value: Value) -> Result<(String, String), String> {
    let path = directory.join("settings.json");
    let _lock = crate::filelock::FileLock::acquire(&path, std::time::Duration::from_secs(5))?;
    let text = crate::steady::read_optional(&path)
        .map_err(|e| format!("读取 {} 失败：{e}", path.display()))?
        .ok_or("还没有设置文件：先启动一次 Jolu 桌面端")?;
    let mut root: Value = serde_json::from_str(&text).map_err(|e| format!("settings.json 不是有效 JSON：{e}"))?;
    let object = root.as_object_mut().ok_or("settings.json 顶层不是对象")?;
    let before = display(object.get(key));
    object.insert(key.to_string(), value.clone());
    // 整份按桌面端的结构体校验一遍：类型不对的值不会写进去让桌面端读不出
    serde_json::from_value::<crate::settings::Settings>(root.clone()).map_err(|e| format!("写入后设置读不出来，已放弃：{e}"))?;
    let bytes = serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("json.cli.tmp");
    std::fs::write(&temporary, &bytes).and_then(|_| std::fs::rename(&temporary, &path)).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        format!("写入 {} 失败：{e}", path.display())
    })?;
    let _ = std::fs::write(path.with_extension("json.lastgood"), &bytes);
    Ok((before, display(Some(&value))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jolu-settings-write-{}-{name}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn set_changes_one_key_and_keeps_everything_else() {
        let dir = dir("one");
        let original = r#"{"dnd":false,"search_apis":[{"vendor":"tavily","keys":["tvly-secret-1234567890"]}],"unknown_future_key":7}"#;
        std::fs::write(dir.join("settings.json"), original).unwrap();
        assert_eq!(set(&dir, "dnd", "on").unwrap(), ("off".into(), "on".into()));
        let after: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(after["dnd"], true);
        assert_eq!(after["search_apis"][0]["keys"][0], "tvly-secret-1234567890", "Key 原文不被掩码覆盖");
        assert_eq!(after["unknown_future_key"], 7);
        assert_eq!(get(&dir, "dnd").unwrap(), "on");
        assert!(!dir.join("settings.json.jolu-lock").exists(), "锁已释放");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_unknown_keys_bad_values_and_missing_file() {
        let dir = dir("bad");
        assert!(set(&dir, "dnd", "on").unwrap_err().contains("先启动一次"));
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        assert!(set(&dir, "proxy_port", "1").unwrap_err().contains("不能用命令行改"));
        assert!(set(&dir, "dnd", "maybe").is_err());
        assert!(set(&dir, "warmup_hour", "24").is_err());
        assert!(set(&dir, "default_page", "nowhere").is_err());
        assert!(set(&dir, "gateway_default_connection", "c1").unwrap_err().contains("不能用命令行改"), "网关启动时快照的键不开放");
        assert_eq!(set(&dir, "balance_alert_threshold", "12.5").unwrap().1, "12.5");
        assert_eq!(set(&dir, "default_page", "last").unwrap().1, "last");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn default_source_must_be_serving_and_have_a_prefix() {
        let dir = dir("default");
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE connections (id TEXT, name TEXT, slug TEXT, serve_enabled INTEGER);
            INSERT INTO connections VALUES ('a','A','a-slug',1),('b','B','b-slug',0),('c','C','',1);").unwrap();
        assert_eq!(set_default_source(&dir, &conn, Some("a")).unwrap().1, "a");
        assert!(set_default_source(&dir, &conn, Some("b")).unwrap_err().contains("不在模型池"));
        assert!(set_default_source(&dir, &conn, Some("c")).unwrap_err().contains("还没有 id"));
        assert_eq!(set_default_source(&dir, &conn, None).unwrap(), ("a".into(), "（未设置）".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_settable_key_exists_on_settings() {
        let value = serde_json::to_value(crate::settings::Settings::default()).unwrap();
        for item in SETTABLE {
            assert!(value.get(item.key).is_some(), "{} 不是设置字段", item.key);
        }
    }

    #[test]
    fn desktop_store_picks_up_cli_writes_and_does_not_overwrite_them() {
        let dir = dir("store");
        let store = crate::settings::Store::load(&dir);
        store.try_update(|s| s.dnd = false).unwrap();
        assert!(!store.reload_external(), "自己写的不算外部改动");
        set(&dir, "warmup_hour", "7").unwrap();
        // 桌面端在读回之前又改了别的键：以磁盘为底，CLI 的改动不丢
        let next = store.try_update(|s| s.dnd = true).unwrap();
        assert_eq!((next.warmup_hour, next.dnd), (7, true));
        set(&dir, "reduce_motion", "on").unwrap();
        assert!(store.reload_external());
        assert!(store.get().reduce_motion);
        assert!(!store.reload_external(), "读回一次后不再报变化");
        let _ = std::fs::remove_dir_all(dir);
    }
}
