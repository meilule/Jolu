//! 统计数据导出备份、导入前预览与去重导入。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use super::requests::{insert_records_into, RequestRecord};

#[derive(Debug, Serialize)]
pub struct ImportSummary {
    /// 新导入的请求条数（重复项不再合并更新，见 D-26）
    pub requests_changed: usize,
    pub sessions_changed: usize,
    /// 跳过的条数：重复项 + 无效项
    pub skipped: usize,
}

/// 导入前预览（画板 100 · 05）：有效、重复、无效三个数字；原因最多 20 条
#[derive(Debug, Clone, Serialize)]
pub struct ImportPreview {
    pub token: String,
    pub file_name: String,
    pub valid: usize,
    pub duplicate: usize,
    pub invalid: usize,
    pub invalid_reasons: Vec<String>,
}

/// 通过逐条校验的备份内容。是否重复在导入时按库里的现状再判断一次，
/// 预览之后采集写入的新记录同样会被跳过。
#[derive(Debug, Default)]
pub struct ParsedBackup {
    requests: Vec<RequestRecord>,
    pub(super) metrics: std::collections::HashMap<(String, String), BackupMetric>,
    attributions: std::collections::HashMap<(String, String), String>,
    contexts: Vec<crate::source_store::ContextSnapshot>,
    sessions: Vec<BackupSession>,
    invalid: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupSession {
    platform: String,
    session_id: String,
    title: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct BackupMetric {
    source: String, dedup_key: String, input_semantics: String,
    #[serde(default)] native_outcome: Option<String>,
    credits: Option<f64>, original_credits: Option<f64>, billable: Option<bool>,
    context_ratio: Option<f64>, reasoning: Option<i64>,
    #[serde(default)] tool: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupAttribution { source: String, dedup_key: String, connection_id: String }

#[derive(Debug, Serialize, Deserialize)]
struct BackupFile {
    #[serde(default)]
    metrics: Vec<BackupMetric>,
    /// 经本地代理归账到连接的记录；旧备份没有该段，导入时保持无归属。
    #[serde(default)]
    attributions: Vec<BackupAttribution>,
    #[serde(default)]
    contexts: Vec<crate::source_store::ContextSnapshot>,
    schema_version: u32,
    exported_at_ms: i64,
    platform: Option<String>,
    requests: Vec<RequestRecord>,
    sessions: Vec<BackupSession>,
}

/// 导入时的外层结构：各列表先按 JSON 值读入，再逐条转换和校验，单条出错只跳过该条
#[derive(Deserialize)]
struct BackupInput {
    schema_version: u32,
    requests: Vec<serde_json::Value>,
    sessions: Vec<serde_json::Value>,
    #[serde(default)]
    metrics: Vec<serde_json::Value>,
    #[serde(default)]
    attributions: Vec<serde_json::Value>,
    #[serde(default)]
    contexts: Vec<serde_json::Value>,
}


/// 导出备份。`start_ms` / `end_ms` 为左闭右开区间，不传表示全部日期；
/// 会话标题只导出区间内有请求的会话。
pub fn export_backup(conn: &Connection, platform: Option<&str>, start_ms: Option<i64>, end_ms: Option<i64>) -> Result<String, String> {
    let mut request_stmt = conn.prepare(
        "SELECT platform, source, dedup_key, session_id, ts, model,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, total_tokens,
                input_known, output_known, cache_read_known, cache_write_known, total_known,
                effort
         FROM requests
         WHERE (?1 IS NULL OR platform = ?1) AND (?2 IS NULL OR ts >= ?2) AND (?3 IS NULL OR ts < ?3)
         ORDER BY ts ASC",
    ).map_err(|error| error.to_string())?;
    let requests = request_stmt.query_map(params![platform, start_ms, end_ms], |row| {
        let known = [
            row.get::<_, i32>(11)? != 0,
            row.get::<_, i32>(12)? != 0,
            row.get::<_, i32>(13)? != 0,
            row.get::<_, i32>(14)? != 0,
            row.get::<_, i32>(15)? != 0,
        ];
        let values = [row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?];
        Ok(RequestRecord {
            platform: row.get(0)?,
            source: row.get(1)?,
            dedup_key: row.get(2)?,
            session_id: row.get(3)?,
            ts: row.get(4)?,
            model: row.get(5)?,
            input_tokens: known[0].then_some(values[0]),
            output_tokens: known[1].then_some(values[1]),
            cache_read_tokens: known[2].then_some(values[2]),
            cache_write_tokens: known[3].then_some(values[3]),
            total_tokens: known[4].then_some(values[4]),
            effort: row.get(16)?,
        })
    }).map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(|error| error.to_string())?;

    let mut session_stmt = conn.prepare(
        "SELECT s.platform, s.session_id, s.title FROM sessions s
         WHERE s.title IS NOT NULL AND (?1 IS NULL OR s.platform = ?1)
           AND ((?2 IS NULL AND ?3 IS NULL) OR EXISTS (
             SELECT 1 FROM requests r
             WHERE r.platform = s.platform AND r.session_id = s.session_id
               AND (?2 IS NULL OR r.ts >= ?2) AND (?3 IS NULL OR r.ts < ?3)))
         ORDER BY s.platform, s.session_id",
    ).map_err(|error| error.to_string())?;
    let sessions = session_stmt.query_map(params![platform, start_ms, end_ms], |row| Ok(BackupSession {
        platform: row.get(0)?,
        session_id: row.get(1)?,
        title: row.get(2)?,
    })).map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(|error| error.to_string())?;

    let mut stmt=conn.prepare("SELECT r.source,r.dedup_key,r.input_semantics,m.credits,m.original_credits,m.billable,m.context_ratio,m.reasoning,r.native_outcome,m.tool
        FROM requests r LEFT JOIN request_metrics m ON m.request_id=r.id
        WHERE (?1 IS NULL OR r.platform=?1) AND (?2 IS NULL OR r.ts>=?2) AND (?3 IS NULL OR r.ts<?3)").map_err(|e|e.to_string())?;
    let metrics=stmt.query_map(params![platform, start_ms, end_ms],|r|Ok(BackupMetric { source:r.get(0)?,dedup_key:r.get(1)?,input_semantics:r.get(2)?,
        credits:r.get(3)?,original_credits:r.get(4)?,billable:r.get(5)?,context_ratio:r.get(6)?,reasoning:r.get(7)?,native_outcome:r.get(8)?,tool:r.get(9)? }))
        .map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let mut attribution_stmt = conn.prepare("SELECT source, dedup_key, connection_id FROM requests
        WHERE connection_id IS NOT NULL AND (?1 IS NULL OR platform = ?1)
          AND (?2 IS NULL OR ts >= ?2) AND (?3 IS NULL OR ts < ?3)").map_err(|e| e.to_string())?;
    let attributions = attribution_stmt.query_map(params![platform, start_ms, end_ms], |r| Ok(BackupAttribution {
        source: r.get(0)?, dedup_key: r.get(1)?, connection_id: r.get(2)?,
    })).map_err(|e| e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&BackupFile {
        metrics,
        attributions,
        contexts: crate::source_store::contexts(conn, platform, start_ms, end_ms).map_err(|e|e.to_string())?,
        schema_version: 2,
        exported_at_ms: chrono::Utc::now().timestamp_millis(),
        platform: platform.map(str::to_string),
        requests,
        sessions,
    }).map_err(|error| error.to_string())
}

const MAX_INVALID_REASONS: usize = 20;

/// 无效项计数与原因（最多保留 20 条）
#[derive(Default)]
struct InvalidLog { count: usize, reasons: Vec<String> }

impl InvalidLog {
    fn push(&mut self, kind: &str, index: usize, reason: impl Into<String>) {
        self.count += 1;
        if self.reasons.len() < MAX_INVALID_REASONS {
            self.reasons.push(format!("第 {} 条{kind}：{}", index + 1, reason.into()));
        }
    }
}

/// 把 serde 的转换错误改成可读原因；缺字段时写「缺少 字段名」
fn shape_problem(error: &serde_json::Error) -> String {
    let text = error.to_string();
    match text.strip_prefix("missing field `").and_then(|rest| rest.split('`').next()) {
        Some(field) => format!("缺少 {field}"),
        None => format!("格式无效（{text}）"),
    }
}

fn clip(value: &str) -> String {
    let mut text: String = value.chars().take(32).collect();
    if text.len() < value.len() { text.push('…'); }
    text
}

fn request_problem(record: &RequestRecord) -> Option<String> {
    if !crate::platforms::known(&record.platform) { return Some(format!("未知平台 {}", clip(&record.platform))); }
    if record.source.trim().is_empty() { return Some("缺少 source".into()); }
    if record.dedup_key.trim().is_empty() { return Some("缺少 dedup_key".into()); }
    if record.source.len() > 512 { return Some("source 超过 512 字节".into()); }
    if record.dedup_key.len() > 1_024 { return Some("dedup_key 超过 1024 字节".into()); }
    if record.model.as_ref().is_some_and(|model| model.len() > 512) { return Some("模型名超过 512 字节".into()); }
    if record.ts <= 0 { return Some("时间无效".into()); }
    if [record.input_tokens, record.output_tokens, record.cache_read_tokens, record.cache_write_tokens, record.total_tokens]
        .into_iter().flatten().any(|value| value < 0) {
        return Some("Token 数值为负".into());
    }
    None
}

/// `requests.native_outcome` 的合法值域：原生来源对一次请求给出的最终结论。
///
/// - `success` / `failed` / `unknown` 是原有的三值，其他原生来源（Gemini、Qoder、
///   OpenBuddy 等）只用这三个，含义未变。
/// - `cancelled` 是 2026-11-21 为 ZCode 扩的一档：ZCode 的 `model_usage.status`
///   原生区分 `completed` / `error` / `cancelled`，把取消并进失败会让「失败次数」
///   和「成功率」虚高。新增这一档只增加可选值，不改任何已有值的含义，所以其他
///   来源的既有记录与统计口径都不受影响。
pub const NATIVE_OUTCOMES: [&str; 4] = ["success", "failed", "cancelled", "unknown"];

/// `native_outcome` 是否属于当前值域；`None` 表示来源没有给结论，合法。
pub fn native_outcome_valid(value: Option<&str>) -> bool {
    value.is_none_or(|s| NATIVE_OUTCOMES.contains(&s))
}

fn metric_valid(m: &BackupMetric) -> bool {
    matches!(m.input_semantics.as_str(), "includes_cache" | "excludes_cache" | "unknown")
        && [m.credits, m.original_credits, m.context_ratio].into_iter().flatten().all(|n| n.is_finite() && n >= 0.0)
        && native_outcome_valid(m.native_outcome.as_deref())
        && !m.context_ratio.is_some_and(|n| n > 1.0) && !m.reasoning.is_some_and(|n| n < 0) && !m.tool.is_some_and(|n| n < 0)
}

pub(super) fn request_exists(conn: &Connection, source: &str, dedup_key: &str) -> rusqlite::Result<bool> {
    conn.prepare_cached("SELECT 1 FROM requests WHERE source = ?1 AND dedup_key = ?2")?
        .exists(params![source, dedup_key])
}

/// 预览 token：时间 + 序号保证唯一，再拼两段 `RandomState`（每进程随机密钥）哈希，
/// 共 128 位，不能由时间推出；不新增依赖。
pub(super) fn import_token() -> String {
    use std::hash::BuildHasher;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let random = |salt: u8| std::collections::hash_map::RandomState::new().hash_one((nanos, seq, salt));
    format!("{nanos:x}-{seq:x}-{:016x}{:016x}", random(0), random(1))
}

/// 导入前预览：逐条校验并统计有效、重复、无效，只读不写库。
/// 单条无效只跳过该条；文件本身不是合法备份（JSON 错误、版本不支持、超限）时整体报错。
pub fn preview_backup(conn: &Connection, json: &str) -> Result<(ImportPreview, ParsedBackup), String> {
    if json.len() > 100 * 1024 * 1024 {
        return Err("导入文件超过 100 MB 限制".into());
    }
    let backup: BackupInput = serde_json::from_str(json)
        .map_err(|error| format!("备份 JSON 格式无效：{error}"))?;
    if !matches!(backup.schema_version, 1 | 2) {
        return Err(format!("不支持的备份版本：{}", backup.schema_version));
    }
    if backup.requests.len() > 1_000_000 {
        return Err("导入记录超过 100 万条限制".into());
    }
    let mut invalid = InvalidLog::default();
    let mut parsed = ParsedBackup::default();

    // 来源指标无效或同一请求出现多份时，对应请求整条按无效跳过，不导入残缺数据
    let mut bad_metrics = std::collections::HashSet::new();
    for (index, value) in backup.metrics.into_iter().enumerate() {
        // 即使某个指标字段类型损坏，仍可凭完整标识拒绝其关联请求，避免导入残缺数据。
        let raw_key = value.get("source").and_then(serde_json::Value::as_str)
            .zip(value.get("dedup_key").and_then(serde_json::Value::as_str))
            .map(|(source, key)| (source.to_owned(), key.to_owned()));
        match serde_json::from_value::<BackupMetric>(value) {
            Err(error) => {
                if let Some(key) = raw_key {
                    parsed.metrics.remove(&key);
                    bad_metrics.insert(key);
                }
                invalid.push("来源指标", index, shape_problem(&error));
            }
            Ok(metric) => {
                let key = (metric.source.clone(), metric.dedup_key.clone());
                if !metric_valid(&metric) || bad_metrics.contains(&key) || parsed.metrics.contains_key(&key) {
                    parsed.metrics.remove(&key);
                    bad_metrics.insert(key);
                } else {
                    parsed.metrics.insert(key, metric);
                }
            }
        }
    }

    let cutoff = crate::source_store::cutoff(conn).map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashSet::new();
    let (mut valid, mut duplicate) = (0, 0);
    for (index, value) in backup.requests.into_iter().enumerate() {
        let record = match serde_json::from_value::<RequestRecord>(value) {
            Ok(record) => record,
            Err(error) => { invalid.push("请求", index, shape_problem(&error)); continue; }
        };
        if let Some(problem) = request_problem(&record) { invalid.push("请求", index, problem); continue; }
        let key = (record.source.clone(), record.dedup_key.clone());
        if bad_metrics.contains(&key) { invalid.push("请求", index, "来源指标无效或重复"); continue; }
        // 原生来源按清理规则不再收录截止日期之前的记录，导入时同样会被跳过
        if crate::platforms::native(&record.platform) && record.ts < cutoff {
            invalid.push("请求", index, "早于历史清理截止日期，按清理规则不再收录");
            continue;
        }
        if !seen.insert(key) || request_exists(conn, &record.source, &record.dedup_key).map_err(|e| e.to_string())? {
            duplicate += 1;
        } else {
            valid += 1;
        }
        parsed.requests.push(record);
    }

    for (index, value) in backup.sessions.into_iter().enumerate() {
        match serde_json::from_value::<BackupSession>(value) {
            Err(error) => invalid.push("会话标题", index, shape_problem(&error)),
            Ok(session) if !crate::platforms::known(&session.platform) => invalid.push("会话标题", index, format!("未知平台 {}", clip(&session.platform))),
            Ok(session) if session.session_id.trim().is_empty() || session.session_id.len() > 512 => invalid.push("会话标题", index, "会话标识为空或过长"),
            Ok(session) if session.title.len() > 4_096 => invalid.push("会话标题", index, "标题超过 4096 字节"),
            // 空标题没有可导入的内容，与旧版一致直接忽略
            Ok(session) if session.title.trim().is_empty() => {}
            Ok(session) => parsed.sessions.push(session),
        }
    }

    for (index, value) in backup.contexts.into_iter().enumerate() {
        match serde_json::from_value::<crate::source_store::ContextSnapshot>(value) {
            Err(error) => invalid.push("上下文快照", index, shape_problem(&error)),
            Ok(c) if !crate::platforms::known(&c.platform) || c.source.len() > 512 || c.session_id.len() > 1_024
                || c.ts <= 0 || !c.ratio.is_finite() || !(0.0..=1.0).contains(&c.ratio) => {
                invalid.push("上下文快照", index, "平台、标识、时间或占用比例无效");
            }
            Ok(c) => parsed.contexts.push(c),
        }
    }

    for (index, value) in backup.attributions.into_iter().enumerate() {
        match serde_json::from_value::<BackupAttribution>(value) {
            Err(error) => invalid.push("连接归属", index, shape_problem(&error)),
            Ok(a) if a.source.len() > 512 || a.dedup_key.len() > 1_024 || a.connection_id.trim().is_empty() || a.connection_id.len() > 128 => {
                invalid.push("连接归属", index, "来源、标识或连接 ID 无效");
            }
            Ok(a) => { parsed.attributions.insert((a.source, a.dedup_key), a.connection_id); }
        }
    }

    parsed.invalid = invalid.count;
    let preview = ImportPreview {
        token: import_token(),
        file_name: String::new(),
        valid,
        duplicate,
        invalid: invalid.count,
        invalid_reasons: invalid.reasons,
    };
    Ok((preview, parsed))
}

/// 按预览结果导入：只插入有效且不重复的记录，库里已有的记录保持原值（D-26）。
/// 连接归属、来源结果只写到本次新插入的请求上；上下文快照只补库里没有的；
/// 会话标题只补库里还没有标题的会话，已有标题不覆盖。
pub fn import_parsed(conn: &mut Connection, parsed: &ParsedBackup) -> Result<ImportSummary, String> {
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    let mut requests_changed = 0;
    for r in &parsed.requests {
        if request_exists(&tx, &r.source, &r.dedup_key).map_err(|e| e.to_string())? { continue; }
        let key = (r.source.clone(), r.dedup_key.clone());
        let native = crate::platforms::native(&r.platform);
        let metric = parsed.metrics.get(&key);
        let added = if native {
            let input_semantics = match metric.map(|m| m.input_semantics.as_str()) {
                Some("includes_cache") => "includes_cache", Some("excludes_cache") => "excludes_cache", _ => "unknown",
            };
            crate::source_store::write(&tx, &[crate::source_store::Record { request: r.clone(), input_semantics,
                credits: metric.and_then(|m| m.credits), original_credits: metric.and_then(|m| m.original_credits),
                billable: metric.and_then(|m| m.billable), context_ratio: metric.and_then(|m| m.context_ratio),
                reasoning: metric.and_then(|m| m.reasoning), tool: metric.and_then(|m| m.tool), duration_ms: None, first_token_ms: None }]).map_err(|e| e.to_string())?
        } else {
            insert_records_into(&tx, std::slice::from_ref(r)).map_err(|e| e.to_string())?
        };
        if added == 0 { continue; }
        requests_changed += 1;
        if let Some(m) = metric.filter(|_| native) {
            tx.execute("UPDATE requests SET native_outcome=?1 WHERE source=?2 AND dedup_key=?3",
                params![m.native_outcome, r.source, r.dedup_key]).map_err(|e| e.to_string())?;
        }
        if let Some(connection_id) = parsed.attributions.get(&key) {
            tx.execute("UPDATE requests SET connection_id = ?1 WHERE source = ?2 AND dedup_key = ?3",
                params![connection_id, r.source, r.dedup_key]).map_err(|e| e.to_string())?;
        }
    }
    for c in &parsed.contexts {
        let exists = tx.prepare_cached("SELECT 1 FROM native_context WHERE source = ?1 AND session_id = ?2")
            .and_then(|mut q| q.exists(params![c.source, c.session_id])).map_err(|e| e.to_string())?;
        if !exists { crate::source_store::context(&tx, c).map_err(|e| e.to_string())?; }
    }
    let mut sessions_changed = 0;
    for session in &parsed.sessions {
        sessions_changed += tx.execute(
            "INSERT INTO sessions(platform, session_id, title) VALUES(?1,?2,?3)
             ON CONFLICT(platform, session_id) DO UPDATE SET title = excluded.title
             WHERE sessions.title IS NULL",
            params![session.platform, session.session_id, session.title],
        ).map_err(|error| error.to_string())?;
    }
    tx.commit().map_err(|error| error.to_string())?;
    Ok(ImportSummary { requests_changed, sessions_changed, skipped: parsed.invalid + parsed.requests.len() - requests_changed })
}

/// 测试用：预览后立即导入
#[cfg(test)]
pub fn import_backup(conn: &mut Connection, json: &str) -> Result<ImportSummary, String> {
    let (_, parsed) = preview_backup(conn, json)?;
    import_parsed(conn, &parsed)
}

