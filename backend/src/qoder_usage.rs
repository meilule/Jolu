//! Qoder 桌面客户端的本地用量账本（`~/.qoder/shared_client/cache/db/local.db`）。
//!
//! ## 为什么单独一个读取器
//!
//! `native_parse::qoder` 解析的是 `~/.qoder/projects/**/*.jsonl`（会话日志）。那套格式在
//! 本机不存在：`~/.qoder` 下只有 `session-env` / `shared_client` / `skills`。真实用量账本
//! 在桌面客户端的 SQLite 里，`chat_message` 表逐条 assistant 记录带 `token_info`。
//! 2026-11-23 对账实测：**999 条** assistant 调用、合计 prompt 46.8M / completion 0.62M /
//! cached 38.7M，而 Jolu 侧 qoder 只有 2 行网关转发记录 —— 零交集。
//!
//! 见 `docs/verification/2026-11-23-p0-3-multi-platform-reconciliation.md` §3。
//!
//! ## 字段语义（对账报告 §3.3，均为实测结论）
//!
//! | 源端 | 映射 | 依据 |
//! |---|---|---|
//! | `chat_message.id` | `dedup_key` 的一部分 | 全局唯一；**不是** `request_id` |
//! | `request_id` | 不入库 | 它是「一轮用户对话」，一轮里最多实测 57 次调用 |
//! | `prompt_tokens` | 含缓存的总输入 | — |
//! | `cached_tokens` | `cache_read_tokens` | 999/999 满足 `cached <= prompt`，是子集 |
//! | `completion_tokens` | `output_tokens` | — |
//! | 无 | `total_tokens` = prompt + completion | 源端不提供显式 total |
//! | `model_info.model_key` | **不入库** | `ultimate` / `auto` / `efficient` 是档位不是模型名 |
//!
//! `input_tokens` 存**原始 `prompt_tokens`**（含缓存），`input_semantics = "includes_cache"`。
//! 新鲜输入由 `source_store::write` / 展示层按这个语义扣掉 `cache_read_tokens`，
//! **不能在这里先扣**——否则会被扣两遍，缓存大的请求新鲜输入算成 0。
//! 这与 `native_parse::workbuddy`（`inputTokens` + `includes_cache`）是同一套约定。
//!
//! ## 隐私
//!
//! 只读 `session_id` / `id` / `gmt_create` / `token_info` / `role` 五列。
//! `content` / `answer` / `question` / `reasoning_content` / `chat_context` 一律不碰。
use crate::source_store::Record;
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

/// 桌面客户端的用量库相对用户主目录的位置。
pub fn ledger_path(home: &Path) -> std::path::PathBuf {
    home.join(".qoder/shared_client/cache/db/local.db")
}

/// 一次读取的结果：可入库的记录，以及逐行跳过的坏行说明。
#[derive(Debug, Default)]
pub struct Read {
    pub records: Vec<Record>,
    /// 跳过的坏行（缺会话 / 消息 id、时间不可读、`token_info` 不是合法 JSON），按类计数。
    /// 不含正文；为空表示没有坏行。
    pub skipped: Vec<String>,
}

/// 从只读连接读全部用量。`since_ms` 为 0 表示全量重建。
///
/// 返回的 `dedup_key` 形如 `{session_id}:{message_id}`。注意它与旧的 projects 日志解析器
/// （`native_parse::qoder`）**不一致**：那边优先用 `request_id` 当 id，这边用消息 id。
/// 两套布局目前不会同时扫描（账本存在时只扫账本），但不能假设两者的键可以互相去重。
///
/// 坏行逐行跳过并计数，不让一行拖垮整个来源，也不静默吞掉。表结构不对才整体报错。
pub fn read_usage(db: &Connection, source: &str, since_ms: i64) -> Result<Read, String> {
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='chat_message')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("Qoder 用量库结构不受支持（缺 chat_message 表）".into());
    }
    let mut stmt = db
        .prepare(
            "SELECT session_id, id, gmt_create, token_info
             FROM chat_message
             WHERE role = 'assistant'
               AND token_info IS NOT NULL AND token_info NOT IN ('', '{}')
               AND gmt_create >= ?1
             ORDER BY gmt_create",
        )
        .map_err(|_| "Qoder 用量库结构不受支持".to_string())?;
    // 每列都按可空读：某一行缺值或类型不对只跳过这一行
    let rows = stmt
        .query_map([since_ms], |r| {
            Ok((
                r.get::<_, Option<String>>(0).ok().flatten(),
                r.get::<_, Option<String>>(1).ok().flatten(),
                r.get::<_, Option<i64>>(2).ok().flatten(),
                r.get::<_, Option<String>>(3).ok().flatten(),
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Read::default();
    let (mut missing_id, mut bad_time, mut bad_json) = (0usize, 0usize, 0usize);
    for row in rows {
        let (session, id, ts, token_info) = row.map_err(|e| e.to_string())?;
        let (Some(session), Some(id)) = (session, id) else {
            missing_id += 1;
            continue;
        };
        let Some(ts) = ts else {
            bad_time += 1;
            continue;
        };
        let Some(Ok(usage)) = token_info.map(|text| serde_json::from_str::<Value>(&text)) else {
            bad_json += 1;
            continue;
        };
        if let Some(record) = parse(&usage, source, &session, &id, ts) {
            out.records.push(record);
        }
    }
    for (count, what) in [
        (missing_id, "缺会话或消息 id"),
        (bad_time, "时间不可读"),
        (bad_json, "Token 信息不是合法 JSON"),
    ] {
        if count > 0 {
            out.skipped.push(format!("跳过 {count} 条{what}的用量记录"));
        }
    }
    Ok(out)
}

/// 单条 `token_info` → 用量记录。字段全缺时返回 `None`（不是零）。
pub fn parse(usage: &Value, source: &str, session: &str, id: &str, ts: i64) -> Option<Record> {
    let n = |key: &str| usage.get(key).and_then(Value::as_i64).filter(|v| *v >= 0);
    let prompt = n("prompt_tokens");
    let completion = n("completion_tokens");
    let cached = n("cached_tokens");
    // 三个字段都缺：这一行没有可用量，跳过而不是记一条全零
    if prompt.is_none() && completion.is_none() && cached.is_none() {
        return None;
    }
    // **存原始 prompt，不是扣减后的新鲜输入。** `source_store::write` 的约定是
    // `input_tokens` 保存来源给的原始输入，新鲜 Token 在算的时候按 `input_semantics`
    // 扣掉 `cache_read_tokens`（见那条 SQL 的 `max(input_tokens-cache_read_tokens,0)`）。
    // 这里若先扣一次再标 `includes_cache`，会被扣两遍——缓存大的请求新鲜输入会算成 0。
    let input = prompt;
    // total 只在两项都有时才算：缺一项时拿另一项充当总量，入库后会被当成已知总量（未知不是零）
    let total = prompt.zip(completion).map(|(p, c)| p + c);
    Some(Record {
        request: crate::db::RequestRecord {
            platform: "qoder".into(),
            source: source.into(),
            dedup_key: format!("{session}:{id}"),
            session_id: Some(format!("{source}:{session}")),
            ts,
            // model_key 是档位（ultimate / auto / efficient…），不是模型名，不能当 model 存
            model: None,
            input_tokens: input,
            output_tokens: completion,
            cache_read_tokens: cached,
            cache_write_tokens: None,
            total_tokens: total,
            effort: None,
        },
        input_semantics: "includes_cache",
        credits: None,
        original_credits: None,
        billable: None,
        context_ratio: None,
        reasoning: None,
        tool: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE chat_message(
                id TEXT, session_id TEXT, request_id TEXT, role TEXT,
                content TEXT, token_info TEXT, model_info TEXT, gmt_create INTEGER
             );",
        )
        .unwrap();
        db
    }

    fn insert(db: &Connection, session: &str, id: &str, role: &str, tokens: &str, ts: i64) {
        db.execute(
            "INSERT INTO chat_message VALUES(?1,?2,'req',?3,'正文不读',?4,NULL,?5)",
            rusqlite::params![id, session, role, tokens, ts],
        )
        .unwrap();
    }

    /// 存的必须是**原始 prompt**，缓存单列；扣减交给按 `input_semantics` 算的那一侧。
    ///
    /// 若这里先扣（`prompt - cached`）再标 `includes_cache`，`source_store` 会再扣一次，
    /// 缓存大的请求新鲜输入会算成 0。这条测试锁住「不提前扣」。
    #[test]
    fn raw_prompt_is_stored_and_cache_stays_separate() {
        let db = ledger();
        insert(&db, "s1", "m1", "assistant",
               r#"{"prompt_tokens":1000,"completion_tokens":50,"cached_tokens":800,"max_input_tokens":180000}"#, 100);
        let records = read_usage(&db, "native:qoder:global", 0).unwrap().records;
        assert_eq!(records.len(), 1);
        let r = &records[0].request;
        assert_eq!(r.input_tokens, Some(1000), "存原始 prompt，不是 200");
        assert_eq!(r.cache_read_tokens, Some(800), "缓存单列");
        assert_eq!(r.output_tokens, Some(50));
        assert_eq!(r.total_tokens, Some(1050), "total = prompt + completion");
        assert_eq!(records[0].input_semantics, "includes_cache");
        assert_eq!(r.dedup_key, "s1:m1");
    }

    #[test]
    fn request_id_is_not_the_dedup_key() {
        // 一轮对话里多次调用共享同一个 request_id，按它合并会丢掉 56/57 的用量
        let db = ledger();
        for (i, id) in ["m1", "m2", "m3"].iter().enumerate() {
            insert(&db, "s1", id, "assistant",
                   r#"{"prompt_tokens":100,"completion_tokens":10,"cached_tokens":0}"#, 100 + i as i64);
        }
        let records = read_usage(&db, "native:qoder:global", 0).unwrap().records;
        assert_eq!(records.len(), 3, "三次调用是三条记录");
        let keys: std::collections::HashSet<_> =
            records.iter().map(|r| r.request.dedup_key.clone()).collect();
        assert_eq!(keys.len(), 3, "去重键必须两两不同");
    }

    #[test]
    fn non_assistant_rows_and_empty_tokens_are_skipped() {
        let db = ledger();
        insert(&db, "s1", "u1", "user", r#"{"prompt_tokens":999,"completion_tokens":0,"cached_tokens":0}"#, 100);
        insert(&db, "s1", "t1", "tool", r#"{"prompt_tokens":999,"completion_tokens":0,"cached_tokens":0}"#, 101);
        insert(&db, "s1", "m1", "assistant", r#"{"max_input_tokens":180000}"#, 102);
        insert(&db, "s1", "m2", "assistant",
               r#"{"prompt_tokens":10,"completion_tokens":5,"cached_tokens":0}"#, 103);
        let records = read_usage(&db, "native:qoder:global", 0).unwrap().records;
        assert_eq!(records.len(), 1, "只留那条真正有 token 的 assistant");
        assert_eq!(records[0].request.dedup_key, "s1:m2");
    }

    #[test]
    fn model_key_is_a_tier_not_a_model_name() {
        let db = ledger();
        insert(&db, "s1", "m1", "assistant",
               r#"{"prompt_tokens":10,"completion_tokens":5,"cached_tokens":0}"#, 100);
        let records = read_usage(&db, "native:qoder:global", 0).unwrap().records;
        assert_eq!(records[0].request.model, None, "档位不能冒充模型名");
    }

    #[test]
    fn since_ms_filters_by_gmt_create() {
        let db = ledger();
        insert(&db, "s1", "old", "assistant", r#"{"prompt_tokens":1,"completion_tokens":1,"cached_tokens":0}"#, 100);
        insert(&db, "s1", "new", "assistant", r#"{"prompt_tokens":1,"completion_tokens":1,"cached_tokens":0}"#, 500);
        let records = read_usage(&db, "native:qoder:global", 200).unwrap().records;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].request.dedup_key, "s1:new");
    }

    /// 真机只读验证（`#[ignore]`，只在显式指定时跑）：读用户真实的 Qoder 用量库。
    ///
    /// 2026-11-23 实测基准：999 条 assistant 调用、缓存读 38,745,227、输出 619,650。
    /// 断言只锁「能读出量、字段形状合理」，不锁具体条数——用户继续用 Qoder 会增长。
    #[test]
    #[ignore = "真实账号只读验证：jolu 真实 usage.db 与 ~/.qoder 账本"]
    fn real_qoder_ledger_is_readable() {
        let Some(home) = crate::creds::home() else { return };
        let path = ledger_path(&home);
        if !path.is_file() {
            eprintln!("跳过：{} 不存在", path.display());
            return;
        }
        let conn = Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        let records = read_usage(&conn, "native:qoder:global", 0).unwrap().records;
        assert!(!records.is_empty(), "真实账本应能读出记录");
        let cache: i64 = records.iter().filter_map(|r| r.request.cache_read_tokens).sum();
        let out: i64 = records.iter().filter_map(|r| r.request.output_tokens).sum();
        eprintln!("Qoder 真实账本：{} 条，缓存读 {}，输出 {}", records.len(), cache, out);
        assert!(cache > 0, "缓存读应为正（实测 3,800 万量级）");
        assert!(out > 0);
        // 每条都必须有去重键，且不能有重复
        let keys: std::collections::HashSet<_> =
            records.iter().map(|r| r.request.dedup_key.clone()).collect();
        assert_eq!(keys.len(), records.len(), "去重键不能重复");
    }

    /// C-4：两项缺一项时总量未知，不能拿另一项充当总量。
    #[test]
    fn total_is_unknown_when_either_part_is_missing() {
        let db = ledger();
        insert(&db, "s1", "only-out", "assistant", r#"{"completion_tokens":50}"#, 100);
        insert(&db, "s1", "only-in", "assistant", r#"{"prompt_tokens":1000,"cached_tokens":800}"#, 101);
        let records = read_usage(&db, "native:qoder:global", 0).unwrap().records;
        assert_eq!(records.len(), 2, "有部分用量的行照样入库");
        for record in &records {
            assert_eq!(record.request.total_tokens, None, "{}：缺一项时总量未知", record.request.dedup_key);
        }
        assert_eq!(records[0].request.output_tokens, Some(50), "已知的那一项保留");
        assert_eq!(records[1].request.input_tokens, Some(1000));
    }

    /// C-5：坏行逐行跳过并计数；一行缺 id 不能拖垮整个来源，坏 JSON 也不能静默吞掉。
    #[test]
    fn bad_rows_are_skipped_one_by_one_and_reported() {
        let db = ledger();
        insert(&db, "s1", "good", "assistant", r#"{"prompt_tokens":10,"completion_tokens":5,"cached_tokens":0}"#, 100);
        insert(&db, "s1", "broken", "assistant", "{not json", 101);
        db.execute(
            "INSERT INTO chat_message VALUES(NULL,'s1','req','assistant','',?1,NULL,102)",
            [r#"{"prompt_tokens":1,"completion_tokens":1}"#],
        ).unwrap();
        db.execute(
            "INSERT INTO chat_message VALUES('m-no-session',NULL,'req','assistant','',?1,NULL,103)",
            [r#"{"prompt_tokens":1,"completion_tokens":1}"#],
        ).unwrap();
        let read = read_usage(&db, "native:qoder:global", 0).expect("坏行不能让整个来源失败");
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.records[0].request.dedup_key, "s1:good");
        assert_eq!(read.skipped.len(), 2, "两类坏行各一条说明：{:?}", read.skipped);
        assert!(read.skipped.iter().any(|s| s.contains("2 条") && s.contains("id")), "{:?}", read.skipped);
        assert!(read.skipped.iter().any(|s| s.contains("1 条") && s.contains("JSON")), "{:?}", read.skipped);
    }

    #[test]
    fn missing_table_is_reported_not_silently_empty() {
        let db = Connection::open_in_memory().unwrap();
        let error = read_usage(&db, "native:qoder:global", 0).unwrap_err();
        assert!(error.contains("chat_message"), "结构不支持要说清楚：{error}");
    }
}
