    #![allow(unused_imports)]
    use super::*;
    use std::collections::{BTreeMap, HashMap, HashSet};
    use chrono::{Local, NaiveDate, TimeZone};
    use rusqlite::Connection;
    use crate::db::{insert_records, RequestRecord};

    #[test]
    fn unknown_outcomes_are_separate_from_success_and_exported() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        for (key, outcome, status) in [("pending", Some("unknown"), None), ("old", None, None),
            ("error", Some("unknown"), Some(500)), ("cancel", Some("cancelled"), Some(500))] {
            conn.execute("INSERT INTO requests(platform,source,dedup_key,ts,total_tokens,total_known,native_outcome,status_code) VALUES('zcode','test',?1,10,5,1,?2,?3)",
                rusqlite::params![key, outcome, status]).unwrap();
        }
        let mut f = StatsFilter { start: 0, end: 100, ..Default::default() };
        let overview = overview(&conn, &f).unwrap();
        assert_eq!((overview.requests, overview.failed, overview.cancelled, overview.unknown), (4, 1, 1, 1));
        for (status, key) in [("success", "old"), ("error", "error"), ("cancelled", "cancel"), ("unknown", "pending")] {
            f.status = Some(status.into());
            assert!(f.validate().is_ok());
            let page = calls(&conn, &f, 1).unwrap();
            assert_eq!(page.rows.len(), 1);
            assert_eq!(page.rows[0].call_id, key);
        }
        let (csv, _) = calls_csv(&conn, &f).unwrap();
        assert!(csv.contains("状态未知"));
    }

    #[test]
    fn model_rows_carry_price_source_and_check_date() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        for (key, model) in [("1", "claude-sonnet-4-5"), ("2", "made-up-model")] {
            conn.execute(
                "INSERT INTO requests(platform,source,dedup_key,ts,model,total_tokens) VALUES('claude','test',?1,10,?2,5)",
                rusqlite::params![key, model],
            )
            .unwrap();
        }
        let filter: StatsFilter = serde_json::from_value(serde_json::json!({"start": 0, "end": 100})).unwrap();
        let rows = overview(&conn, &filter).unwrap().models;
        let known = rows.iter().find(|row| row.model.as_deref() == Some("claude-sonnet-4-5")).unwrap();
        assert!(known.price_source.as_deref().is_some_and(|url| url.starts_with("https://")));
        assert_eq!(known.price_checked.as_deref(), Some("2026-11-03"));
        let unknown = rows.iter().find(|row| row.model.as_deref() == Some("made-up-model")).unwrap();
        assert_eq!((unknown.price_source.as_deref(), unknown.price_checked.as_deref()), (None, None), "未收录不编出处");
    }

    #[test]
    fn source_breakdown_groups_by_connection_and_counts_unattributed_apart() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,status,created_ms) VALUES('c-a','codex','auth','工作号','connected',1)", []).unwrap();
        let insert = |key: &str, platform: &str, ts: i64, total: i64, known: i64, connection: Option<&str>| {
            conn.execute(
                "INSERT INTO requests(platform,source,dedup_key,ts,input_tokens,total_tokens,total_known,connection_id) VALUES(?1,'test',?2,?3,?4,?4,?5,?6)",
                rusqlite::params![platform, key, ts, total, known, connection],
            )
            .unwrap();
        };
        insert("1", "codex", 100, 50, 1, Some("c-a"));
        insert("2", "codex", 110, 30, 1, Some("c-a"));
        insert("3", "codex", 120, 0, 0, Some("c-gone"));
        insert("4", "codex", 130, 70, 1, None);
        insert("5", "claude", 140, 90, 1, Some("c-a"));
        insert("6", "codex", 999, 10, 1, Some("c-a"));
        let b = source_breakdown(&conn, Some("codex"), 0, 500).unwrap();
        assert_eq!(b.total_requests, 4);
        assert_eq!(b.unattributed_requests, 1, "没有归属的不并进任何一行");
        assert_eq!(b.rows.len(), 2);
        assert_eq!(b.rows[0], SourceRow { connection_id: "c-a".into(), name: Some("工作号".into()), requests: 2, tokens: Some(80) });
        assert_eq!(b.rows[1], SourceRow { connection_id: "c-gone".into(), name: None, requests: 1, tokens: None }, "全未知不当 0；已删连接没有名字");
        assert_eq!(source_breakdown(&conn, None, 0, 500).unwrap().total_requests, 5);
    }

    /// P4 审计 D6：「今日按来源」与「本地今日 Token」同为新鲜口径，缓存重读不计。
    #[test]
    fn source_breakdown_uses_the_same_fresh_measure_as_todays_tokens() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,status,created_ms) VALUES('c1','claude','api','主号','connected',1)", []).unwrap();
        let now = chrono::Local::now().timestamp_millis() - 1;
        conn.execute(
            "INSERT INTO requests(platform,source,dedup_key,ts,input_tokens,output_tokens,cache_read_tokens,total_tokens,connection_id)
             VALUES('claude','test','k',?1,10,5,1000,1015,'c1')",
            [now],
        )
        .unwrap();
        let (start, end) = crate::db::period_range(&conn, "claude", "today");
        let row = &source_breakdown(&conn, Some("claude"), start, end).unwrap().rows[0];
        assert_eq!(row.tokens, Some(15), "不能把 1000 缓存重读算进来源行");
        assert_eq!(row.tokens, crate::db::today_fresh_tokens(&conn, "claude").unwrap(), "与底部合计同口径");
    }

    fn rec(platform: &str, key: &str, ts: i64, model: &str, tokens: [Option<i64>; 5], session: Option<&str>) -> RequestRecord {
        RequestRecord {
            platform: platform.into(),
            source: format!("test-{platform}"),
            dedup_key: key.into(),
            session_id: session.map(String::from),
            ts,
            model: Some(model.into()),
            input_tokens: tokens[0],
            output_tokens: tokens[1],
            cache_read_tokens: tokens[2],
            cache_write_tokens: tokens[3],
            total_tokens: tokens[4],
            effort: None,
        }
    }

    fn seeded(records: &[RequestRecord]) -> (Connection, std::path::PathBuf) {
        // 并行测试的记录条数可能相同，按序号区分临时库，避免互相删除或写入
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("jolu-usage-stats-{}-{seq}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut conn = crate::db::open(&path).unwrap();
        insert_records(&mut conn, records).unwrap();
        (conn, path)
    }

    fn filter(start: i64, end: i64) -> StatsFilter {
        StatsFilter { start, end, ..Default::default() }
    }

    #[test]
    fn call_cost_is_priced_at_request_time_like_its_price_snapshot() {
        // DeepSeek 周末全天谷时（半价），工作日 07:00 UTC 是峰时。按请求时间计价，
        // 两条同量请求的费用比应为 0.5，且与单价拆解的「谷时」标记一致——不能随查看时刻变化。
        let saturday = chrono::Utc.with_ymd_and_hms(2026, 11, 7, 12, 0, 0).unwrap().timestamp_millis();
        let monday = chrono::Utc.with_ymd_and_hms(2026, 11, 9, 7, 0, 0).unwrap().timestamp_millis();
        let tokens = [Some(1_000_000), Some(100_000), Some(0), Some(0), Some(1_100_000)];
        let (conn, path) = seeded(&[
            rec("zcode", "off-peak", saturday, "deepseek-v4-flash", tokens, None),
            rec("zcode", "peak", monday, "deepseek-v4-flash", tokens, None),
        ]);
        let page = calls(&conn, &filter(saturday - 1, monday + 1), 1).unwrap();
        let row = |key: &str| page.rows.iter().find(|row| row.call_id == key).unwrap();
        let (off, peak) = (row("off-peak"), row("peak"));
        assert_eq!(off.price.as_ref().map(|p| p.off_peak), Some(true));
        assert_eq!(peak.price.as_ref().map(|p| p.off_peak), Some(false));
        let ratio = off.cost.unwrap() / peak.cost.unwrap();
        assert!((ratio - 0.5).abs() < 1e-9, "谷时应半价，实际比值 {ratio}");
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    /// 模型行在部分请求没有 Token 时仍要给已知小计。
    ///
    /// 现场（2026-11-22 本机）：ZCode 的 `deepseek-v4.1-flash-1017` 有 4987 条记录，
    /// 其中 29 条是取消/失败、没有 Token。`sum_known` 遇到一条未知就整组返回 None，
    /// 于是整行显示破折号，真实的 23 亿 Token 完全看不见。
    #[test]
    fn a_model_row_keeps_its_known_subtotal_when_some_requests_have_none() {
        let base = chrono::Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap().timestamp_millis();
        let done = [Some(1_000), Some(500), Some(0), Some(0), Some(1_500)];
        let unknown = [Some(0), Some(0), Some(0), Some(0), None];
        let (conn, path) = seeded(&[
            rec("zcode", "m-ok-1", base + 1_000, "deepseek-v4.1-flash-1017", done, None),
            rec("zcode", "m-ok-2", base + 2_000, "deepseek-v4.1-flash-1017", done, None),
            // 取消/失败：Token 全零且标记为未知，正是「没跑完所以没有用量」
            rec("zcode", "m-cancel", base + 3_000, "deepseek-v4.1-flash-1017", unknown, None),
            // 另一个 model id：两个中转商给同一个上游模型起了不同名字（:1017 vs -1017），
            // 它们是真实的两个来源，不该被合并
            rec("zcode", "m-colon", base + 4_000, "deepseek-v4.1-flash:1017", done, None),
        ]);
        let view = overview(&conn, &filter(base, base + 10_000)).unwrap();
        let row = |model: &str| view.models.iter().find(|m| m.model.as_deref() == Some(model)).unwrap();

        let dashed = row("deepseek-v4.1-flash-1017");
        assert_eq!(dashed.requests, 3);
        assert_eq!(dashed.tokens, None, "有一条未知，整组合计不能当成完整值");
        assert_eq!(dashed.known_tokens, 3_000, "已知小计必须是真实值，不能因一条未知就归零");
        assert_eq!(dashed.known_requests, 2, "覆盖 3 条里的 2 条");

        let colon = row("deepseek-v4.1-flash:1017");
        assert_eq!(colon.requests, 1);
        assert_eq!(colon.tokens, Some(1_500), "全覆盖的家保持原有语义");
        assert_eq!((colon.known_tokens, colon.known_requests), (1_500, 1));

        // 排序按已知小计：部分未知的行（3000）不能因为 tokens 是 None 就落到全覆盖行（1500）后面
        let order: Vec<_> = view.models.iter().map(|m| m.model.clone().unwrap()).collect();
        assert_eq!(
            order,
            vec!["deepseek-v4.1-flash-1017", "deepseek-v4.1-flash:1017"],
            "排序用已知小计，不是可能为 None 的合计：{order:?}"
        );
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unpriced_reasons_separate_absent_usage_from_partial_fields() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("claude", "missing-price", base + 1, "unknown-model", [Some(1); 5], None),
            rec("claude", "absent", base + 2, "claude-sonnet-4-5", [None; 5], None),
            rec("codex", "partial", base + 3, "gpt-5", [Some(100), None, None, None, None], None),
            rec("claude", "total-only", base + 4, "claude-sonnet-4-5", [None, None, None, None, Some(100)], None),
            rec("claude", "zero", base + 5, "claude-sonnet-4-5", [Some(0); 5], None),
        ]);
        let summary = serde_json::to_value(overview(&conn, &filter(base, base + 100)).unwrap().cost).unwrap();
        assert_eq!(summary["unpriced_no_usage"], 1);
        assert_eq!(summary["unpriced_partial"], 2);
        assert_eq!(summary["unpriced_no_price"], serde_json::json!(["claude:unknown-model"]));
        assert_eq!(summary["priced_requests"], 1, "真实零仍应计价");
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn overview_normalizes_input_and_keeps_unknown_fields_unknown() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("codex", "c1", base + 1_000, "gpt-5", [Some(1_000), Some(200), Some(800), Some(0), Some(1_200)], Some("s1")),
            rec("claude", "a1", base + 2_000, "claude-sonnet-4-5", [Some(100), Some(50), Some(300), Some(0), Some(450)], Some("s2")),
            rec("claude", "a0", base - 86_400_000 + 1_000, "claude-sonnet-4-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
        ]);
        let o = overview(&conn, &filter(base, base + 86_400_000)).unwrap();
        assert_eq!(o.requests, 2);
        assert_eq!(o.tokens.fresh_input, Some(200 + 100), "Codex 扣掉缓存读取，Claude 原样");
        assert_eq!(o.tokens.cache_read, Some(1_100));
        assert_eq!(o.tokens.total, Some(1_650));
        assert_eq!(o.previous_total, Some(15));
        assert_eq!(o.cost.requests, 2);
        assert_eq!(o.providers.len(), 2);
        assert_eq!(o.sessions[0].session_id, "s1");
        assert_eq!(o.trend.bucket, "1h");
        assert_eq!(o.trend.points.len(), 24);
        // 来源元数据与统计同一批行：数据时间是范围内最后一条，最早可用记录不受范围限制
        assert_eq!(o.provenance.data_at, Some(base + 2_000));
        assert_eq!(o.provenance.earliest_available, Some(base - 86_400_000 + 1_000));
        assert_eq!((o.provenance.window_start, o.provenance.window_end), (base, base + 86_400_000));
        assert_eq!(o.provenance.unit, crate::data_contract::Unit::Tokens);
        assert_eq!(
            o.provenance.completeness,
            crate::data_contract::Completeness::AsRecorded,
            "本机采集无法对照源端证明完整，只能是「已采到的部分」"
        );
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    /// P4 审计 D8：今天上午 10 点对比的应是昨天 0–10 点，而不是昨天最后 10 小时。
    #[test]
    fn todays_comparison_uses_the_same_hours_of_yesterday() {
        let today = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let yesterday = local_ms(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap());
        let hour = 3_600_000;
        let (conn, path) = seeded(&[
            rec("claude", "y-morning", yesterday + 5 * hour, "claude-sonnet-4-5", [Some(60), Some(40), Some(0), Some(0), Some(100)], None),
            rec("claude", "y-evening", yesterday + 20 * hour, "claude-sonnet-4-5", [Some(5), Some(2), Some(0), Some(0), Some(7)], None),
            rec("claude", "t-morning", today + 2 * hour, "claude-sonnet-4-5", [Some(1), Some(1), Some(0), Some(0), Some(2)], None),
        ]);
        let o = overview(&conn, &filter(today, today + 10 * hour)).unwrap();
        assert_eq!(o.previous_total, Some(100), "应对比昨天 0–10 点（100），不是昨天 14–24 点（7）");
        // 整天区间平移结果不变
        assert_eq!(previous_window(today, today + 24 * hour), (yesterday, today));
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn overview_keeps_known_token_subtotal_when_native_requests_have_no_tokens() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("codex", "coverage-c", base + 1_000, "gpt-5", [Some(100), Some(20), Some(60), Some(0), Some(120)], None),
            rec("claude", "coverage-a", base + 2_000, "claude-sonnet-4-5", [Some(20), Some(10), Some(0), Some(0), Some(30)], None),
            rec("qoder", "coverage-q", base + 3_000, "qfmodel", [None; 5], None),
        ]);
        let all = overview(&conn, &filter(base, base + 3_600_000)).unwrap();
        assert_eq!(all.tokens.total, None, "未知请求不能伪装成完整总量");
        let json = serde_json::to_value(&all).unwrap();
        assert_eq!(json["token_coverage"]["known_total"], 150, "未知原生记录不能抹掉已知 Token 合计");
        assert_eq!(json["token_coverage"]["known_requests"], 2);
        assert_eq!(json["token_coverage"]["cache_hit_rate"], 0.5, "缓存分子与分母必须来自同一批可计算请求");
        assert_eq!(json["token_coverage"]["cache_requests"], 2);
        let qoder = overview(&conn, &StatsFilter { platform: Some("qoder".into()), ..filter(base, base + 3_600_000) }).unwrap();
        let json = serde_json::to_value(qoder).unwrap();
        assert!(json["token_coverage"]["known_total"].is_null(), "全部未知时不能显示零");
        assert_eq!(json["token_coverage"]["known_requests"], 0);
        assert!(json["token_coverage"]["cache_hit_rate"].is_null());
        assert_eq!(json["token_coverage"]["cache_requests"], 0);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_cache_field_makes_that_split_unknown_and_platform_filter_applies() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("claude", "a1", base + 1_000, "claude-sonnet-4-5", [Some(100), Some(50), None, Some(0), Some(150)], None),
            rec("codex", "c1", base + 2_000, "gpt-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
        ]);
        let all = overview(&conn, &filter(base, base + 3_600_000)).unwrap();
        assert_eq!(all.tokens.cache_read, None);
        assert_eq!(all.cache_hit_rate, None);
        assert_eq!(all.trend.bucket, "1h");
        let codex = overview(&conn, &StatsFilter { platform: Some("codex".into()), ..filter(base, base + 3_600_000) }).unwrap();
        assert_eq!(codex.tokens.cache_read, Some(0));
        assert_eq!(codex.requests, 1);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn calls_paginate_newest_first_and_search_by_session() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let records: Vec<_> = (0..22)
            .map(|i| rec("claude", &format!("k{i}"), base + i * 1_000, "claude-sonnet-4-5", [Some(1), Some(1), Some(0), Some(0), Some(2)], Some(if i == 3 { "needle" } else { "other" })))
            .collect();
        let (conn, path) = seeded(&records);
        let page = calls(&conn, &filter(base, base + 60_000), 1).unwrap();
        assert_eq!(page.total_count, 22);
        assert_eq!(page.page_count, 2);
        assert_eq!(page.rows.len(), 20);
        assert_eq!(page.rows[0].call_id, "k21");
        assert_eq!(page.models, vec!["claude-sonnet-4-5".to_string()]);
        let found = calls(&conn, &StatsFilter { search: Some("needle".into()), ..filter(base, base + 60_000) }, 1).unwrap();
        assert_eq!(found.total_count, 1);
        assert_eq!(found.rows[0].call_id, "k3");
        let (csv, count) = calls_csv(&conn, &filter(base, base + 60_000)).unwrap();
        assert_eq!(count, 22);
        assert!(csv.starts_with('\u{feff}'));
        assert_eq!(csv.lines().count(), 23);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn day_buckets_follow_local_midnight() {
        let start = local_ms(NaiveDate::from_ymd_opt(2026, 10, 15).unwrap());
        let end = local_ms(NaiveDate::from_ymd_opt(2026, 10, 22).unwrap());
        let (kind, ranges) = buckets(start, end);
        assert_eq!(kind, "day");
        assert_eq!(ranges.len(), 7);
        assert_eq!(ranges[0].2, "10/15");
        let (kind, ranges) = buckets(start, start + 12 * 3_600_000 + 1);
        assert_eq!(kind, "1h");
        assert_eq!(ranges.len(), 13);
        assert_eq!(ranges[12].2, "12:00");
        let (kind, ranges) = buckets(start, start + 2 * 86_400_000);
        assert_eq!(kind, "2h");
        assert_eq!(ranges.len(), 24);
    }

    #[test]
    fn csv_cells_are_quoted_when_needed() {
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("plain"), "plain");
    }

    /// P4 审计 D11：会话里有记录没给 Token 时，榜单给已知部分并带上未提供的次数，不把未知当 0。
    #[test]
    fn top_sessions_keep_unknown_requests_apart() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("claude", "k1", base + 1_000, "claude-sonnet-4-5", [Some(60), Some(40), Some(0), Some(0), Some(100)], Some("s")),
            rec("claude", "k2", base + 2_000, "claude-sonnet-4-5", [None; 5], Some("s")),
            rec("claude", "k3", base + 3_000, "claude-sonnet-4-5", [None; 5], Some("only-unknown")),
        ]);
        let o = overview(&conn, &filter(base, base + 3_600_000)).unwrap();
        assert_eq!(o.sessions.len(), 1, "一条已知都没有的会话无从排名，不进榜");
        assert_eq!((o.sessions[0].tokens, o.sessions[0].unknown_requests), (100, 1));
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn top_sessions_fills_cwd_from_source() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("claude", "a1", base + 1_000, "claude-sonnet-4-5", [Some(100), Some(50), Some(0), Some(0), Some(150)], Some("with-cwd")),
            rec("claude", "a2", base + 2_000, "claude-sonnet-4-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], Some("gone")),
            rec("claude", "a3", base + 3_000, "claude-sonnet-4-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], Some("with-cwd")),
            rec("codex", "c1", base + 4_000, "gpt-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], Some("unregistered")),
        ]);
        let dir = std::env::temp_dir().join(format!("jolu-top-sessions-cwd-{}", std::process::id()));
        let projects = dir.join(".claude").join("projects").join("demo");
        std::fs::create_dir_all(&projects).unwrap();
        let file = projects.join("with-cwd.jsonl");
        std::fs::write(&file, "{\"type\":\"user\",\"cwd\":\"D:\\\\work\\\\demo-app\",\"sessionId\":\"with-cwd\"}\n").unwrap();
        let missing = projects.join("gone.jsonl");
        for p in [&file, &missing] {
            conn.execute("INSERT INTO scan_state(path, offset, mtime) VALUES(?1, 0, 0)", [p.to_string_lossy()]).unwrap();
        }
        let o = overview(&conn, &filter(base, base + 86_400_000)).unwrap();
        let with = o.sessions.iter().find(|s| s.session_id == "with-cwd").unwrap();
        assert_eq!(with.cwd.as_deref(), Some("D:\\work\\demo-app"));
        let gone = o.sessions.iter().find(|s| s.session_id == "gone").unwrap();
        assert_eq!(gone.cwd, None, "来源文件不存在时不猜项目");
        let unregistered = o.sessions.iter().find(|s| s.session_id == "unregistered").unwrap();
        assert_eq!(unregistered.cwd, None, "没有登记来源文件时不猜项目");
        drop(conn);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 失败口径：error 计失败，cancelled 不算失败也不算成功，三种状态筛选互斥且合起来
    /// 覆盖全部有结论的行。这条测试锁定「取消不算失败」是有意为之，改动口径时必须同步改它。
    #[test]
    fn cancelled_is_neither_failure_nor_success() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 11, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("zcode", "ok", base + 1_000, "glm-4.7", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
            rec("zcode", "err", base + 2_000, "glm-4.7", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
            rec("zcode", "cancel", base + 3_000, "glm-4.7", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
            // 没有原生结论、也没有状态码的本机记录按成功处理
            rec("claude", "native-ok", base + 4_000, "claude-sonnet-4-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
            // 取消时上游可能留下非 2xx：cancelled 优先，不能既算取消又算失败
            rec("zcode", "cancel-500", base + 5_000, "glm-4.7", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
        ]);
        for (key, outcome) in [("ok", "success"), ("err", "failed"), ("cancel", "cancelled"), ("cancel-500", "cancelled")] {
            conn.execute(
                "UPDATE requests SET native_outcome = ?2 WHERE platform = 'zcode' AND dedup_key = ?1",
                rusqlite::params![key, outcome],
            ).unwrap();
        }
        conn.execute(
            "UPDATE requests SET status_code = 502 WHERE platform = 'zcode' AND dedup_key = 'cancel-500'",
            [],
        ).unwrap();

        let range = filter(base, base + 86_400_000);
        let o = overview(&conn, &range).unwrap();
        assert_eq!((o.requests, o.failed, o.cancelled), (5, 1, 2), "取消不并进失败数（即便带了 502）");
        assert_eq!(
            (o.requests - o.failed - o.cancelled) as f64 / o.requests as f64, 0.4,
            "成功率 = 成功 / 全部：2 条成功（含无状态码的本机记录）、1 失败、2 取消",
        );

        // 状态筛选：三档互斥，合起来正好是全部行
        let keys = |status: &str| -> Vec<String> {
            let mut f = filter(base, base + 86_400_000);
            f.status = Some(status.into());
            let mut ids: Vec<String> = calls(&conn, &f, 1).unwrap().rows.into_iter().map(|r| r.call_id).collect();
            ids.sort();
            ids
        };
        assert_eq!(keys("success"), vec!["native-ok", "ok"], "取消不能落进成功筛选");
        assert_eq!(keys("error"), vec!["err"], "取消不能落进失败筛选");
        assert_eq!(keys("cancelled"), vec!["cancel", "cancel-500"]);
        assert_eq!(
            keys("success").len() + keys("error").len() + keys("cancelled").len(), 5,
            "三档必须正好划分全部行：重复会让成功率被扣两次，遗漏会让某行哪档都不在",
        );

        // 明细行与 CSV 也带出独立状态
        let row = |key: &str| {
            calls(&conn, &range, 1).unwrap().rows.into_iter().find(|r| r.call_id == key).unwrap()
        };
        assert_eq!((row("cancel").failed, row("cancel").cancelled), (false, true));
        assert_eq!((row("err").failed, row("err").cancelled), (true, false));
        assert_eq!((row("cancel-500").failed, row("cancel-500").cancelled), (false, true), "cancel 优先于 502");
        let (csv, _) = calls_csv(&conn, &range).unwrap();
        assert!(csv.contains("已取消"), "CSV 状态列不能把取消写成失败");
        assert!(csv.contains("失败") && !csv.contains("失败 已取消"));

        // 非法状态值仍要被校验拦住；cancelled 必须放行
        let mut bad = filter(base, base + 1);
        bad.status = Some("canceled".into());
        assert!(bad.validate().is_err());
        let mut good = filter(base, base + 1);
        good.status = Some("cancelled".into());
        assert!(good.validate().is_ok());
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn billing_scope_splits_api_and_subscription_and_gemini_prices_without_cache_write() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap());
        let (conn, path) = seeded(&[
            // Gemini 缺 cache_write_tokens（cache_write_known = 0），仍应计价
            rec("gemini", "g1", base + 1_000, "gemini-2.5-pro", [Some(100_000), Some(20_000), Some(0), None, Some(120_000)], None),
            // API 连接（倍率 0.5）
            rec("claude", "c-api", base + 2_000, "claude-sonnet-4-5", [Some(1_000_000), Some(0), Some(0), Some(0), Some(1_000_000)], None),
            // 订阅连接
            rec("claude", "c-sub", base + 3_000, "claude-sonnet-4-5", [Some(1_000_000), Some(0), Some(0), Some(0), Some(1_000_000)], None),
            // 同模型一条取消无用量，模型行仍应保留已知费用小计
            rec("claude", "c-cancel", base + 4_000, "claude-sonnet-4-5", [None; 5], None),
        ]);
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,status,created_ms,billing_mode,price_multiplier)
             VALUES('conn-api','claude','api','半价中转','connected',1,'api',0.5),
                   ('conn-sub','claude','auth','官方订阅','connected',1,'auto',1.0)",
            [],
        ).unwrap();
        conn.execute("UPDATE requests SET connection_id='conn-api' WHERE dedup_key='c-api'", []).unwrap();
        conn.execute("UPDATE requests SET connection_id='conn-sub' WHERE dedup_key='c-sub'", []).unwrap();

        let all = overview(&conn, &filter(base, base + 86_400_000)).unwrap();
        assert_eq!(all.cost.priced_requests, 3, "Gemini 缺 cache_write 也应成功计价");
        assert!((all.cost.subscription_amount - 3.0).abs() < 1e-6);
        assert!(all.cost.api_amount > 1.5, "包含半价 API ($1.5) + 未归属 Gemini API");

        let claude_row = all.models.iter().find(|m| m.model.as_deref() == Some("claude-sonnet-4-5")).unwrap();
        assert_eq!(claude_row.priced_requests, 2);
        assert!((claude_row.cost.unwrap() - 4.5).abs() < 1e-6, "部分取消不应抹掉模型行已知费用（$1.5 + $3.0 = $4.5）");

        let mut only_api = filter(base, base + 86_400_000);
        only_api.billing_scope = Some("api".into());
        let api_view = overview(&conn, &only_api).unwrap();
        assert_eq!(api_view.cost.subscription_amount, 0.0);
        assert_eq!(api_view.cost.priced_requests, 2);

        let mut only_sub = filter(base, base + 86_400_000);
        only_sub.billing_scope = Some("subscription".into());
        let sub_view = overview(&conn, &only_sub).unwrap();
        assert_eq!(sub_view.requests, 1);
        assert!((sub_view.cost.amount - 3.0).abs() < 1e-6);

        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    /// P0 回归：
    /// 1) excludes_cache 缺 cache_write 时仍可算命中率（如 Antigravity / DSH）；
    /// 2) 趋势分桶混入单条 unknown 语义记录时，不把整根柱子抹零，并回报 unknown_requests 与 cache_hit_rate；
    /// 3) ModelRow.cache_hit_rate 不因单条取消/未知行归零；
    /// 4) 存量 Claude `<synthetic>` 零 Token 伪行自动从统计中排除。
    #[test]
    fn p0_excludes_cache_hit_rate_trend_partial_bucket_and_synthetic_exclusion() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 11, 21).unwrap());
        let (conn, path) = seeded(&[
            // 正常 Claude 行：有缓存读 800、新增输入 200 → 命中率 80%
            rec("claude", "c-ok", base + 1_000, "claude-sonnet-4-5", [Some(200), Some(50), Some(800), Some(0), Some(1_050)], None),
            // 同模型一条取消行（无 Token）
            rec("claude", "c-cancel", base + 2_000, "claude-sonnet-4-5", [None; 5], None),
            // Claude `<synthetic>` 零 Token 伪行（应被排除）
            rec("claude", "c-syn", base + 3_000, "<synthetic>", [Some(0), Some(0), Some(0), Some(0), Some(0)], None),
        ]);
        // 一条 excludes_cache 但 cache_write_known=0 的原生记录（如 Antigravity）
        crate::source_store::write(&conn, &[crate::source_store::Record {
            request: RequestRecord {
                platform: "antigravity".into(),
                source: "native:antigravity:default".into(),
                dedup_key: "ag-1".into(),
                session_id: Some("native:antigravity:default:s1".into()),
                ts: base + 4_000,
                model: Some("gemini-4-neon-xhigh".into()),
                input_tokens: Some(3_000),
                output_tokens: Some(800),
                cache_read_tokens: Some(177_000),
                cache_write_tokens: None,
                total_tokens: Some(180_800),
                effort: None,
            },
            input_semantics: "excludes_cache",
            credits: None,
            original_credits: None,
            billable: None,
            context_ratio: None,
            reasoning: None,
            tool: None,
            duration_ms: Some(15_000),
            first_token_ms: Some(10_000),
        }]).unwrap();
        // 一条 input_semantics='unknown' 的 workbuddy 记录（曾导致整日趋势桶归零）
        crate::source_store::write(&conn, &[crate::source_store::Record {
            request: RequestRecord {
                platform: "workbuddy".into(),
                source: "native:workbuddy:main".into(),
                dedup_key: "wb-1".into(),
                session_id: None,
                ts: base + 5_000,
                model: Some("glm-5".into()),
                input_tokens: Some(100),
                output_tokens: Some(20),
                cache_read_tokens: None,
                cache_write_tokens: None,
                total_tokens: Some(120),
                effort: None,
            },
            input_semantics: "unknown",
            credits: None,
            original_credits: None,
            billable: None,
            context_ratio: None,
            reasoning: None,
            tool: None,
            duration_ms: None,
            first_token_ms: None,
        }]).unwrap();

        let o = overview(&conn, &filter(base, base + 86_400_000)).unwrap();
        // `<synthetic>` 应被排除：共 4 条有效请求（c-ok, c-cancel, ag-1, wb-1）
        assert_eq!(o.requests, 4);
        assert!(!o.cost.unpriced_no_price.iter().any(|m| m.contains("<synthetic>")));

        // 趋势图第 0 个 1h 桶不能被 wb-1 / c-cancel 吞零
        let b0 = &o.trend.points[0];
        assert_eq!(b0.fresh_input, Some(3_200), "已知新鲜输入 = 200 + 3000");
        assert_eq!(b0.cache_read, Some(177_800));
        assert_eq!(b0.unknown_requests, 2, "c-cancel 和 wb-1 缺分项或语义未知");
        assert!(b0.cache_hit_rate.unwrap() > 0.98);

        // Antigravity excludes_cache 缺 cache_write 仍可计算准确命中率：177_000 / 180_000 = 98.333%
        let ag = overview(&conn, &StatsFilter { platform: Some("antigravity".into()), ..filter(base, base + 86_400_000) }).unwrap();
        let rate = ag.cache_hit_rate.expect("excludes_cache 缺 cache_write 不应导致命中率为空");
        assert!((rate - (177_000.0 / 180_000.0)).abs() < 1e-6);

        // Claude 模型行虽含 1 条取消行，命中率仍保留 80%
        let claude_m = o.models.iter().find(|m| m.model.as_deref() == Some("claude-sonnet-4-5")).unwrap();
        assert!((claude_m.cache_hit_rate.unwrap() - 0.8).abs() < 1e-6);

        // Antigravity 写入的 first_token_ms 应完整落库并透传到调用明细
        let ag_calls = calls(&conn, &StatsFilter { platform: Some("antigravity".into()), ..filter(base, base + 86_400_000) }, 1).unwrap();
        assert_eq!(ag_calls.rows[0].first_token_ms, Some(10_000));
        assert_eq!(ag_calls.rows[0].duration_ms, Some(15_000));

        drop(conn);
        let _ = std::fs::remove_file(path);
    }
