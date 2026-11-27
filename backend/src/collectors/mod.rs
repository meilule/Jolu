//! 本机各平台用量与会话增量采集器统一入口与注册表。

use std::path::{Path, PathBuf};

use crate::db::Store;
use crate::source_store;

pub(crate) mod antigravity;
pub(crate) mod claude_codex;
pub(crate) mod dsh;
pub(crate) mod kimi;
pub(crate) mod native;
pub(crate) mod opencode;
pub(crate) mod pi;
pub(crate) mod qoder;
pub(crate) mod qwen;

/// 统一采集器描述接口（供静态注册表与观测诊断枚举）。
#[allow(dead_code)]
pub(crate) trait UsageCollector: Sync {
    fn platform(&self) -> &'static str;
    fn discover_sources(&self, home: &Path, app_data: &Path) -> Vec<native::Source> {
        native::sources_at(home, app_data)
            .into_iter()
            .filter(|s| s.platform == self.platform())
            .collect()
    }
    fn watch_dirs(&self, home: &Path, app_data: &Path) -> Vec<PathBuf> {
        self.discover_sources(home, app_data)
            .into_iter()
            .map(|s| native::watch_root(&s))
            .collect()
    }
}

#[allow(dead_code)]
struct PlatformCollector(&'static str);
impl UsageCollector for PlatformCollector {
    fn platform(&self) -> &'static str {
        self.0
    }
}

pub(crate) static COLLECTORS: &[&dyn UsageCollector] = &[
    &PlatformCollector("antigravity"),
    &PlatformCollector("kimi"),
    &PlatformCollector("pi"),
    &PlatformCollector("dsh"),
    &PlatformCollector("qwen"),
    &PlatformCollector("opencode"),
    &PlatformCollector("zcode"),
    &PlatformCollector("qoder"),
    &PlatformCollector("gemini"),
    &PlatformCollector("workbuddy"),
];

/// 按平台统一分发本机来源增量扫描；返回 `None` 表示该子源由同平台主源接管（不计扫描文件数）。
pub(crate) fn dispatch_native_scan<S: Store>(
    db: &mut S,
    source: &native::Source,
    home: &Path,
    app: &Path,
) -> Option<Result<(usize, Vec<String>), String>> {
    let _ = COLLECTORS;
    match source.platform.as_str() {
        "antigravity" => {
            let root = antigravity::Root {
                source_id: source.id.clone(),
                name: source.name.clone(),
                path: source.path.clone(),
            };
            Some(antigravity::scan_root(db, &root))
        }
        "kimi" => Some(kimi::scan_usage(db, home)),
        "pi" => Some(pi::scan_usage(db, home)),
        "dsh" => Some(dsh::scan_usage(db, home)),
        "qwen" => Some(qwen::scan_usage(db, home)),
        "opencode" => {
            if source.path.is_dir() && crate::opencode::data_dir(home).join("opencode.db").exists()
            {
                return None;
            }
            Some((|| -> Result<(usize, Vec<String>), String> {
                if source.path.is_dir() {
                    let records = opencode::read_json_files(&source.path)?;
                    return db
                        .write(|conn| source_store::write(conn, &records))
                        .map(|n| (n, Vec::new()))
                        .map_err(|e| e.to_string());
                }
                let key = format!("opencode-db:{}", source.path.to_string_lossy());
                let stamps = native::db_stamps(&source.path)?;
                if db.write(|conn| native::db_unchanged(conn, &key, stamps)) {
                    return Ok((0, Vec::new()));
                }
                let input = native::readonly(&source.path)?;
                let layout = opencode::detect_layout(&input)?;
                let records = opencode::read_usage(&input, layout, 0)?;
                let since_states = chrono::Utc::now().timestamp_millis() - 24 * 60 * 60 * 1_000;
                let states = opencode::read_session_states(&input, layout, since_states);
                drop(input);
                db.write(|conn| -> Result<(usize, Vec<String>), String> {
                    let tx = conn.transaction().map_err(|e| e.to_string())?;
                    let written = source_store::write(&tx, &records).map_err(|e| e.to_string())?;
                    for s in &states {
                        if let Some(title) = s.title.as_deref() {
                            crate::db::upsert_session_title(&tx, "opencode", &s.session_id, title)
                                .map_err(|e| e.to_string())?;
                        }
                        if let Some(dir) = s.directory.as_deref() {
                            crate::db::set_session_cwd(&tx, "opencode", &s.session_id, dir)
                                .map_err(|e| e.to_string())?;
                        }
                        if s.is_subagent {
                            crate::db::set_session_subagent(&tx, "opencode", &s.session_id, true)
                                .map_err(|e| e.to_string())?;
                        }
                        crate::db::set_session_activity(&tx, "opencode", &s.session_id, s.state, s.updated_ms)
                            .map_err(|e| e.to_string())?;
                        if let Some(start_ms) = s.turn_started_ms {
                            crate::db::set_session_turn_start(&tx, "opencode", &s.session_id, start_ms)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    native::save_db_stamps(&tx, &key, stamps).map_err(|e| e.to_string())?;
                    tx.commit().map_err(|e| e.to_string())?;
                    Ok((written, Vec::new()))
                })
            })())
        }
        "zcode" => Some(native::zcode(db, source).map(|n| (n, Vec::new()))),
        "qoder"
            if source.id == "native:qoder:global" && qoder::ledger_path(home).is_file() =>
        {
            Some(native::qoder_ledger(db, home, &source.id))
        }
        _ => Some(native::scan_files(db, source, app)),
    }
}
