use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::digest::{DigestRow, LEVEL_ROLLUP};
use super::facts::Facts;
use super::pipeline::{covered_range, Draft, PreviousHandoff};
use super::PROMPT_VERSION;

/* ─────────────────────────── 存储 ─────────────────────────── */

pub fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS handoff_digests (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            platform       TEXT    NOT NULL,
            session_id     TEXT    NOT NULL,
            level          INTEGER NOT NULL,
            from_idx       INTEGER NOT NULL,
            to_idx         INTEGER NOT NULL,
            range_hash     TEXT    NOT NULL,
            digest_md      TEXT    NOT NULL,
            tokens         INTEGER NOT NULL,
            source         TEXT    NOT NULL DEFAULT 'raw',
            rolled_into    INTEGER,
            prompt_version TEXT    NOT NULL,
            created_ms     INTEGER NOT NULL,
            UNIQUE(platform, session_id, level, from_idx)
        );
        CREATE TABLE IF NOT EXISTS session_handoffs (
            id                     INTEGER PRIMARY KEY AUTOINCREMENT,
            platform               TEXT    NOT NULL,
            session_id             TEXT    NOT NULL,
            version                INTEGER NOT NULL,
            covered_to             INTEGER NOT NULL,
            narrative_md           TEXT    NOT NULL,
            facts_json             TEXT    NOT NULL,
            focus                  TEXT    NOT NULL DEFAULT '',
            target                 TEXT    NOT NULL DEFAULT '',
            draft_rounds           INTEGER NOT NULL DEFAULT 1,
            path_warnings          TEXT    NOT NULL DEFAULT '[]',
            executor_platform      TEXT,
            executor_connection_id TEXT,
            model                  TEXT,
            input_tokens           INTEGER,
            output_tokens          INTEGER,
            calls                  INTEGER,
            prompt_version         TEXT    NOT NULL,
            created_ms             INTEGER NOT NULL,
            snapshot_hash          TEXT,
            UNIQUE(platform, session_id, version)
        );",
    )?;
    // 早期开发库里的表没有 snapshot_hash：幂等补列
    let has_snapshot: bool = conn.prepare("SELECT 1 FROM pragma_table_info('session_handoffs') WHERE name='snapshot_hash'")?.exists([])?;
    if !has_snapshot {
        conn.execute("ALTER TABLE session_handoffs ADD COLUMN snapshot_hash TEXT", [])?;
    }
    Ok(())
}

/// 落库一份纪要（同一起点的同级纪要被重做时覆盖），返回库里的 id；早期纪要会把它覆盖范围内尚未归并的纪要指向自己
pub fn save_digest(conn: &Connection, row: &DigestRow) -> rusqlite::Result<i64> {
    let id: i64 = conn.query_row(
        "INSERT INTO handoff_digests (platform, session_id, level, from_idx, to_idx, range_hash, digest_md, tokens, source, rolled_into, prompt_version, created_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11)
         ON CONFLICT(platform, session_id, level, from_idx) DO UPDATE SET
            to_idx=excluded.to_idx, range_hash=excluded.range_hash, digest_md=excluded.digest_md, tokens=excluded.tokens,
            source=excluded.source, rolled_into=NULL, prompt_version=excluded.prompt_version, created_ms=excluded.created_ms
         RETURNING id",
        params![row.platform, row.session_id, row.level, row.from_idx as i64, row.to_idx as i64, row.range_hash, row.digest_md, row.tokens as i64, row.source, row.prompt_version, row.created_ms],
        |r| r.get(0),
    )?;
    if row.level == LEVEL_ROLLUP {
        conn.execute(
            "UPDATE handoff_digests SET rolled_into=?1 WHERE platform=?2 AND session_id=?3 AND id<>?1 AND from_idx>=?4 AND to_idx<=?5 AND rolled_into IS NULL",
            params![id, row.platform, row.session_id, row.from_idx as i64, row.to_idx as i64],
        )?;
    }
    Ok(id)
}

pub fn load_digests(conn: &Connection, platform: &str, session_id: &str) -> rusqlite::Result<Vec<DigestRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, platform, session_id, level, from_idx, to_idx, range_hash, digest_md, tokens, source, rolled_into, prompt_version, created_ms
         FROM handoff_digests WHERE platform=?1 AND session_id=?2 ORDER BY from_idx, level",
    )?;
    let rows = stmt.query_map(params![platform, session_id], |r| {
        Ok(DigestRow {
            id: r.get(0)?,
            platform: r.get(1)?,
            session_id: r.get(2)?,
            level: r.get(3)?,
            from_idx: r.get::<_, i64>(4)?.max(0) as usize,
            to_idx: r.get::<_, i64>(5)?.max(0) as usize,
            range_hash: r.get(6)?,
            digest_md: r.get(7)?,
            tokens: r.get::<_, i64>(8)?.max(0) as usize,
            source: r.get(9)?,
            rolled_into: r.get(10)?,
            prompt_version: r.get(11)?,
            created_ms: r.get(12)?,
        })
    })?;
    rows.collect()
}

/// 「本次压缩记录」与对话弹窗入口需要的版本信息（不含叙述正文）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HandoffMeta {
    pub version: i64,
    /// 覆盖的消息条数（内部为不含的结束下标）
    pub covered_to: usize,
    /// 给界面显示的「覆盖第 a–b 条」（1 起，闭区间）
    pub covered_range: Option<[usize; 2]>,
    pub focus: String,
    pub target: String,
    pub draft_rounds: i64,
    pub path_warnings: Vec<String>,
    pub executor_platform: Option<String>,
    pub executor_connection_id: Option<String>,
    pub model: Option<String>,
    /// 压缩消耗的 token（D-04：不并入用量统计）；执行器没报告时为 None
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub calls: Option<i64>,
    pub prompt_version: String,
    pub created_ms: i64,
}

#[derive(Debug, Clone)]
pub struct HandoffRecord {
    pub meta: HandoffMeta,
    pub narrative_md: String,
    pub facts: Facts,
    /// 写入时快照的校验值（判断「上次之后没有新消息」）
    pub snapshot_hash: Option<String>,
}

impl HandoffRecord {
    pub(crate) fn previous(self) -> PreviousHandoff {
        PreviousHandoff { prompt_version: self.meta.prompt_version, covered_to: self.meta.covered_to, narrative_md: self.narrative_md, focus: self.meta.focus, snapshot_hash: self.snapshot_hash }
    }
}

pub fn latest_handoff(conn: &Connection, platform: &str, session_id: &str) -> rusqlite::Result<Option<HandoffRecord>> {
    conn.query_row(
        "SELECT version, covered_to, focus, target, draft_rounds, path_warnings, executor_platform, executor_connection_id, model,
                input_tokens, output_tokens, calls, prompt_version, created_ms, narrative_md, facts_json, snapshot_hash
         FROM session_handoffs WHERE platform=?1 AND session_id=?2 ORDER BY version DESC LIMIT 1",
        params![platform, session_id],
        |r| {
            let warnings: String = r.get(5)?;
            let facts: String = r.get(15)?;
            Ok(HandoffRecord {
                meta: HandoffMeta {
                    version: r.get(0)?,
                    covered_to: r.get::<_, i64>(1)?.max(0) as usize,
                    covered_range: covered_range(r.get::<_, i64>(1)?.max(0) as usize),
                    focus: r.get(2)?,
                    target: r.get(3)?,
                    draft_rounds: r.get(4)?,
                    path_warnings: serde_json::from_str(&warnings).unwrap_or_default(),
                    executor_platform: r.get(6)?,
                    executor_connection_id: r.get(7)?,
                    model: r.get(8)?,
                    input_tokens: r.get(9)?,
                    output_tokens: r.get(10)?,
                    calls: r.get(11)?,
                    prompt_version: r.get(12)?,
                    created_ms: r.get(13)?,
                },
                narrative_md: r.get(14)?,
                facts: serde_json::from_str(&facts).unwrap_or_default(),
                snapshot_hash: r.get(16)?,
            })
        },
    )
    .optional()
}

/// 写入新的交接版本（一个事务内取下一个版本号并插入），返回版本号
pub(crate) struct NewHandoff<'a> {
    pub platform: &'a str,
    pub session_id: &'a str,
    pub covered_to: usize,
    pub draft: &'a Draft,
    pub facts: &'a Facts,
    pub focus: &'a str,
    pub target: &'a str,
    pub draft_rounds: usize,
    pub executor_platform: &'a str,
    pub connection_id: &'a str,
    pub model: Option<&'a str>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub calls: u64,
    pub now_ms: i64,
    pub snapshot_hash: &'a str,
}

pub(crate) fn insert_handoff(conn: &Connection, h: &NewHandoff) -> rusqlite::Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let version: i64 = tx.query_row("SELECT COALESCE(MAX(version), 0) + 1 FROM session_handoffs WHERE platform=?1 AND session_id=?2", params![h.platform, h.session_id], |r| r.get(0))?;
    tx.execute(
        "INSERT INTO session_handoffs (platform, session_id, version, covered_to, narrative_md, facts_json, focus, target, draft_rounds, path_warnings,
                                       executor_platform, executor_connection_id, model, input_tokens, output_tokens, calls, prompt_version, created_ms, snapshot_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            h.platform,
            h.session_id,
            version,
            h.covered_to as i64,
            h.draft.narrative_md,
            serde_json::to_string(h.facts).unwrap_or_else(|_| "{}".into()),
            h.focus,
            h.target,
            h.draft_rounds as i64,
            serde_json::to_string(&h.draft.path_warnings).unwrap_or_else(|_| "[]".into()),
            h.executor_platform,
            h.connection_id,
            h.model,
            h.input_tokens.map(|n| n as i64),
            h.output_tokens.map(|n| n as i64),
            h.calls as i64,
            PROMPT_VERSION,
            h.now_ms,
            h.snapshot_hash,
        ],
    )?;
    tx.commit()?;
    Ok(version)
}
