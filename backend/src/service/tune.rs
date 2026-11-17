//! 成本调优建议的取数与汇总（5.10 / N9）：桌面端命令 `tune_advice` 与 `jolu tune` 共用。

use std::collections::BTreeMap;

/// 读最近 `days` 天两家的调用与当前压缩设置，给出建议
pub fn advise(conn: &rusqlite::Connection, days: i64, now_ms: i64) -> Result<Vec<crate::tune::AgentAdvice>, String> {
    let since = now_ms - days.clamp(1, 365) * 86_400_000;
    let mut by = BTreeMap::new();
    for platform in ["claude", "codex"] {
        by.insert(platform.to_string(), crate::tune::calls_from_db(conn, platform, since).map_err(|e| e.to_string())?);
    }
    let claude = crate::creds::claude_dir()
        .and_then(|dir| crate::steady::read_to_string(dir.join("settings.json")).ok())
        .and_then(|text| crate::tune::claude_compact_setting(&text));
    let codex = crate::creds::codex_dir()
        .and_then(|dir| crate::steady::read_to_string(dir.join("config.toml")).ok())
        .and_then(|text| crate::tune::codex_compact_setting(&text));
    // 模型窗口取 models.dev 元数据缓存（桌面端维护的文件）；`来源/模型` 先按全名查，再按裸名查
    let meta = crate::service::settings::models_meta();
    let window_of = |model: &str| -> Option<i64> {
        let meta = meta.as_ref()?;
        let bare = model.rsplit('/').next().unwrap_or(model);
        meta(model).or_else(|| meta(bare)).map(|m| m.context_window).filter(|w| *w > 0)
    };
    Ok(crate::tune::advise(&by, claude, codex, &window_of))
}
