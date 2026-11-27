//! 会话源文件定位与只读 `cwd` 提取（仅依赖 `rusqlite` 与 `validation`，供 `usage_stats`、`skills`、`sessions` 共用）。

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use rusqlite::Connection;
use serde_json::Value as Json;

static CWD_CACHE: LazyLock<Mutex<HashMap<PathBuf, (u64, Option<String>)>>> =
    LazyLock::new(Default::default);

pub(crate) fn file_mtime_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// 源文件定位：会话 `(platform, session_id)` → 主记录文件（不含子代理文件）。
pub(crate) fn source_files(conn: &Connection) -> rusqlite::Result<HashMap<(String, String), PathBuf>> {
    if crate::validation::isolated() {
        return Ok(HashMap::new());
    }
    let mut stmt = conn.prepare("SELECT path, parser_session_id FROM scan_state")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    let mut out: HashMap<(String, String), PathBuf> = HashMap::new();
    for row in rows {
        let (path, parsed) = row?;
        let lower = path.replace('\\', "/").to_ascii_lowercase();
        let platform = if lower.contains("/.claude/") {
            "claude"
        } else if lower.contains("/sessions/") && lower.contains("rollout-") {
            "codex"
        } else if lower.ends_with("/transcript.jsonl") || lower.ends_with("/transcript_full.jsonl") {
            "antigravity"
        } else {
            continue;
        };
        if lower.contains("/subagents/") {
            continue;
        }
        let file = PathBuf::from(&path);
        let id = parsed.or_else(|| {
            (platform == "claude")
                .then(|| file.file_stem()?.to_str().map(String::from))
                .flatten()
        });
        if let Some(id) = id {
            out.entry((platform.into(), id)).or_insert(file);
        }
    }
    Ok(out)
}

/// 读源文件开头找 cwd：Claude 每行顶层带 cwd，Codex 在 session_meta.payload.cwd。
pub(crate) fn read_cwd(path: &Path) -> Option<String> {
    if crate::validation::isolated() {
        return None;
    }
    let stamp = file_mtime_secs(path)?;
    if let Some((cached, cwd)) = CWD_CACHE.lock().ok()?.get(path) {
        if *cached == stamp {
            return cwd.clone();
        }
    }
    let file = std::fs::File::open(path).ok()?;
    let cwd = BufReader::new(file)
        .lines()
        .take(400)
        .map_while(Result::ok)
        .find_map(|line| {
            let v: Json = serde_json::from_str(&line).ok()?;
            v.get("cwd")
                .or_else(|| v.get("payload")?.get("cwd"))?
                .as_str()
                .map(String::from)
        });
    if let Ok(mut cache) = CWD_CACHE.lock() {
        cache.insert(path.to_path_buf(), (stamp, cwd.clone()));
    }
    cwd
}
