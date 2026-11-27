//! `db` 模块回归测试。

use super::*;
use super::backup::{import_token, request_exists};
use super::metrics::start_of_today_at;
use chrono::{Datelike, Local, TimeZone};
use rusqlite::{params, Connection};

    /// 接线记录（agent_wiring）外键指向 agents 种子表：每加一个可接线 agent 都要补种子行，
    /// 否则真机接线在「保存接线记录」一步报 FOREIGN KEY constraint failed（2026-11-17 Kimi 踩过）。
    /// 开库路径（[`open`]）会开 foreign_keys，这里照做，保证约束真的在拦。
    #[test]
    fn every_wirable_agent_has_a_seed_row_for_the_wiring_foreign_key() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        init(&conn).unwrap();
        for agent in crate::gateway::wiring::agents() {
            conn.execute(
                "INSERT INTO agent_wiring(agent_id, instance_id, owner, state) VALUES(?1, 'test', 'none', 'idle')",
                [agent],
            ).unwrap_or_else(|error| panic!("agent {agent} 的接线记录写不进去（agents 种子表缺行）：{error}"));
        }
    }

    #[test]
    fn failed_record_insert_does_not_advance_scan_checkpoint_or_titles() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_request BEFORE INSERT ON requests BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        ).unwrap();
        let record = RequestRecord {
            platform: "claude".into(),
            source: "local-session".into(),
            dedup_key: "request-1".into(),
            session_id: Some("session-1".into()),
            ts: 1,
            model: Some("claude-sonnet-4".into()),
            input_tokens: Some(1),
            output_tokens: Some(1),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            total_tokens: Some(2), effort: None,
        };

        let result = commit_scanned_file(
            &mut conn,
            &[record],
            &[("claude".into(), "session-1".into(), "标题".into())],
            None,
            "session.jsonl",
            42,
            7,
        );
        assert!(result.is_err());
        assert_eq!(get_offset(&conn, "session.jsonl"), 0);
        let titles: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0)).unwrap();
        assert_eq!(titles, 0);
    }

    #[test]
    fn proxy_timing_updates_matching_local_request_and_reports_misses() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let record = |key: &str| RequestRecord {
            platform: "claude".into(),
            source: crate::collector::SOURCE_LOCAL.into(),
            dedup_key: key.into(),
            session_id: None, ts: 1, model: None, input_tokens: Some(1), output_tokens: Some(1),
            cache_read_tokens: Some(0), cache_write_tokens: Some(0), total_tokens: Some(2), effort: None,
        };
        insert_records(&mut conn, &[record("msg_known")]).unwrap();

        // 命中：三列写入，且只影响目标行
        let updated = super::apply_proxy_timing(&conn, "msg_known", 320, 4_500, 200, None).unwrap();
        assert_eq!(updated, 1);
        let (first, duration, status): (Option<i64>, Option<i64>, Option<i64>) = conn.query_row(
            "SELECT first_token_ms, duration_ms, status_code FROM requests WHERE dedup_key = 'msg_known'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!((first, duration, status), (Some(320), Some(4_500), Some(200)));

        // 未命中（记录还没入库）：返回 0，调用方转入待关联队列
        let updated = super::apply_proxy_timing(&conn, "msg_missing", 10, 20, 200, None).unwrap();
        assert_eq!(updated, 0);
    }

    /// 收缩态那一行要说出「哪条已存连接在跑」：网关写了 `connection_id` 的行是精确档，
    /// 会话级与平台级都指向同一条连接；窗口内没有请求时退回平台档，不猜。
    #[test]
    fn live_usage_reports_exact_attribution_from_gateway_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,base_url,masked,created_ms)
             VALUES('gateway-conn','zcode','api','网关连接','synthetic-secret','https://relay.example.test','****',1)",
            [],
        ).unwrap();
        let now = Local::now().timestamp_millis();
        let record = |key: &str, session: &str| RequestRecord {
            platform: "zcode".into(),
            source: "native:zcode:main".into(),
            dedup_key: key.into(),
            session_id: Some(session.into()),
            ts: now,
            model: Some("GLM-5.3".into()),
            input_tokens: Some(10),
            output_tokens: Some(5),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            total_tokens: Some(15),
            effort: None,
        };
        insert_records(&mut conn, &[record("r1", "native:zcode:main:s1")]).unwrap();
        conn.execute("UPDATE requests SET connection_id='gateway-conn' WHERE dedup_key='r1'", []).unwrap();

        let live = live_usage(&conn, "zcode", None).unwrap();
        assert_eq!(live.attribution.connection_id.as_deref(), Some("gateway-conn"));
        assert_eq!(live.attribution.name.as_deref(), Some("网关连接"));
        assert_eq!(live.attribution.confidence, crate::attribution::Confidence::Exact);
        let session = live
            .active_sessions
            .iter()
            .find(|s| s.session_id.ends_with(":s1"))
            .expect("窗口内的会话应出现在实时快照里");
        assert_eq!(session.attribution.connection_id.as_deref(), Some("gateway-conn"));

        // 窗口内没有请求（换一个从没写过的平台）：平台档，不带任何连接
        let empty = live_usage(&conn, "codex", None).unwrap();
        assert_eq!(empty.attribution.confidence, crate::attribution::Confidence::Platform);
        assert!(empty.attribution.connection_id.is_none());
    }

    #[test]
    fn total_range_starts_at_the_selected_platform_first_record() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let record = |platform: &str, key: &str, ts: i64| RequestRecord {
            platform: platform.into(), source: "local".into(), dedup_key: key.into(),
            session_id: None, ts, model: None, input_tokens: Some(1), output_tokens: Some(0),
            cache_read_tokens: Some(0), cache_write_tokens: Some(0), total_tokens: Some(1), effort: None,
        };
        insert_records(&mut conn, &[record("claude", "c", 100), record("codex", "o", 200)]).unwrap();

        assert_eq!(period_range(&conn, "codex", "total").0, 200);
        assert_eq!(period_range(&conn, "claude", "total").0, 100);
    }

    #[test]
    fn ended_custom_daily_trend_stops_at_custom_end() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let start = Local.with_ymd_and_hms(2026, 1, 10, 0, 0, 0).single().unwrap().timestamp_millis();
        let end = Local.with_ymd_and_hms(2026, 1, 13, 0, 0, 0).single().unwrap().timestamp_millis();
        let result = trend(&conn, "codex", "custom", Some(CustomRange { start, end: Some(end) })).unwrap();

        assert_eq!(result.points.len(), 3);
        assert_eq!(result.points.first().unwrap().label, "01/10");
        assert_eq!(result.points.last().unwrap().label, "01/12");
    }

    #[test]
    fn cost_totals_and_request_log_price_each_request_at_its_own_time() {
        // DeepSeek：周六全天谷时半价，周一 07:00 UTC 峰时。两条同量请求合计应为峰时单价的 1.5 倍，
        // 请求日志里两条的费用比为 0.5——不随查看时刻变化。
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let saturday = chrono::Utc.with_ymd_and_hms(2026, 11, 7, 12, 0, 0).unwrap().timestamp_millis();
        let monday = chrono::Utc.with_ymd_and_hms(2026, 11, 9, 7, 0, 0).unwrap().timestamp_millis();
        let record = |key: &str, ts: i64| RequestRecord {
            platform: "zcode".into(), source: "local".into(), dedup_key: key.into(), session_id: None, ts,
            model: Some("deepseek-v4-flash".into()), input_tokens: Some(1_000_000), output_tokens: Some(100_000),
            cache_read_tokens: Some(0), cache_write_tokens: Some(0), total_tokens: Some(1_100_000), effort: None,
        };
        insert_records(&mut conn, &[record("off-peak", saturday), record("peak", monday)]).unwrap();
        let range = Some(CustomRange { start: saturday - 1, end: Some(monday + 1) });
        let peak_one = crate::pricing::estimate_usage_at(Some("zcode"), Some("deepseek-v4-flash"), crate::pricing::Usage {
            input: 1_000_000, output: 100_000, ..Default::default()
        }, chrono::DateTime::from_timestamp_millis(monday).unwrap()).unwrap();
        let cost = cost_estimate_at(&conn, "zcode", "custom", range, None, monday + 1).unwrap();
        assert!((cost.amount - peak_one * 1.5).abs() < 1e-9, "合计 {} 应为峰时单条 {} 的 1.5 倍", cost.amount, peak_one);
        let log = request_log_at(&conn, "zcode", "custom", 1, range, None, monday + 1).unwrap();
        let costs: Vec<f64> = log.rows.iter().filter_map(|row| row.cost_estimate).collect();
        assert_eq!(costs.len(), 2);
        let (low, high) = (costs[0].min(costs[1]), costs[0].max(costs[1]));
        assert!((low / high - 0.5).abs() < 1e-9, "日志费用比 {}", low / high);
        let trend = trend_at(&conn, "zcode", "custom", range, None, monday + 1).unwrap();
        let trend_cost: f64 = trend.points.iter().filter_map(|point| point.cost).sum();
        assert!((trend_cost - peak_one * 1.5).abs() < 1e-9, "趋势合计 {trend_cost}");
    }

    #[test]
    fn all_statistics_use_the_supplied_query_end_snapshot() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let record = |key: &str, ts: i64, total: i64| RequestRecord {
            platform: "codex".into(), source: "local".into(), dedup_key: key.into(),
            session_id: None, ts, model: None, input_tokens: Some(total), output_tokens: Some(0),
            cache_read_tokens: Some(0), cache_write_tokens: Some(0), total_tokens: Some(total), effort: None,
        };
        insert_records(&mut conn, &[record("before", 1_000, 3), record("after", 2_000, 30)]).unwrap();
        let range = Some(CustomRange { start: 0, end: None });
        let snapshot = 1_500;

        let totals = token_totals_at(&conn, "codex", range, None, snapshot).unwrap();
        let trend = trend_at(&conn, "codex", "custom", range, None, snapshot).unwrap();
        let log = request_log_at(&conn, "codex", "custom", 1, range, None, snapshot).unwrap();
        let cost = cost_estimate_at(&conn, "codex", "custom", range, None, snapshot).unwrap();

        assert_eq!(totals.custom, Some(3));
        assert_eq!(trend.points.iter().filter_map(|point| point.tokens).sum::<i64>(), 3);
        assert_eq!(log.total_count, 1);
        assert_eq!(log.rows[0].total, Some(3));
        assert_eq!(cost.uncovered_tokens, 3);
    }

    #[test]
    fn cumulative_request_updates_store_final_value_and_emit_only_the_difference() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let record = |total: i64| RequestRecord {
            platform: "codex".into(), source: "local".into(), dedup_key: "same-response".into(),
            session_id: Some("session-1".into()), ts: Local::now().timestamp_millis(),
            model: Some("unknown-test-model".into()), input_tokens: Some(total), output_tokens: Some(0),
            cache_read_tokens: Some(0), cache_write_tokens: Some(0), total_tokens: Some(total), effort: None,
        };
        assert_eq!(insert_records(&mut conn, &[record(10)]).unwrap(), 1);
        let baseline = live_usage(&conn, "codex", None).unwrap().cursor;
        assert_eq!(insert_records(&mut conn, &[record(15)]).unwrap(), 1);

        let update = live_usage(&conn, "codex", Some(baseline)).unwrap();
        assert_eq!(update.delta_tokens, 5);
        assert_eq!(update.delta_sessions[0].delta_tokens, 5);
        let stored: (i64, i64) = conn.query_row(
            "SELECT COUNT(*), SUM(total_tokens) FROM requests",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(stored, (1, 15));

        let cursor = update.cursor;
        assert_eq!(insert_records(&mut conn, &[record(15)]).unwrap(), 0);
        let duplicate = live_usage(&conn, "codex", Some(cursor)).unwrap();
        assert_eq!(duplicate.cursor, cursor);
        assert_eq!(duplicate.delta_tokens, 0);
    }

    /// P1「其它重试入口的幂等性」：所有写入路径都汇到 `insert_records_into`，
    /// 靠 `(source, dedup_key)` 去重。这条固定样本把**四个真实入口**在同一条记录上
    /// 依次跑一遍，验证累计效果仍然只有一行、总量不翻倍。
    ///
    /// 四个入口来自代码里的实际调用点：
    /// 1. 文件增量扫描（`insert_scanned_records`，collector.rs）
    /// 2. 库型来源扫描（`source_store::write`，native_sources.rs）
    /// 3. 网关订阅直接记账（`insert_gateway_record`，写 connection_id）
    /// 4. 本地代理计时回填（`apply_proxy_timing`，只 UPDATE 不 INSERT）
    ///
    /// 重试场景：扫描器重复扫同一区间、网关重试同一请求、回填滞后到记录已存在之后。
    #[test]
    fn every_retry_entry_point_is_idempotent_on_the_same_record() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis();
        let record = || RequestRecord {
            platform: "codex".into(),
            source: "local-session".into(),
            dedup_key: "resp-retry-1".into(),
            session_id: Some("s1".into()),
            ts: now,
            model: Some("gpt-5".into()),
            input_tokens: Some(100),
            output_tokens: Some(20),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            total_tokens: Some(120),
            effort: None,
        };
        let snapshot = |c: &Connection| -> (i64, i64) {
            c.query_row(
                "SELECT COUNT(*), COALESCE(SUM(CASE WHEN total_known=1 THEN total_tokens END), 0)
                 FROM requests WHERE platform='codex'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };

        // 入口 1：文件增量扫描，连跑三次（幂等重试）
        for _ in 0..3 {
            insert_scanned_records(&conn, std::slice::from_ref(&record())).unwrap();
        }
        assert_eq!(snapshot(&conn), (1, 120), "三次扫描只留一行、总量不变");

        // 入口 2：库型来源 source_store::write（同一记录再写两次）
        let stored = crate::source_store::Record {
            request: record(),
            input_semantics: "unknown",
            credits: None,
            original_credits: None,
            billable: None,
            context_ratio: None,
            reasoning: None,
            tool: None,
            duration_ms: None,
            first_token_ms: None,
        };
        for _ in 0..2 {
            crate::source_store::write(&conn, std::slice::from_ref(&stored)).unwrap();
        }
        assert_eq!(snapshot(&conn), (1, 120), "source_store 重复写不翻倍");

        // 入口 3：网关订阅记账（同一 dedup_key 再来一次，绑定 connection_id）
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug)
             VALUES('c-gw','codex','auth','网关','x','****',1,'c-gw')",
            [],
        ).unwrap();
        insert_gateway_record_with_timing(&mut conn, &record(), "c-gw", 1234, None).unwrap();
        assert_eq!(snapshot(&conn), (1, 120), "网关记账不新增行");

        // 入口 4：代理计时回填（UPDATE 语义，重复调用不产生行）
        for _ in 0..2 {
            apply_proxy_timing(&conn, "resp-retry-1", 50, 900, 200, Some("c-gw")).unwrap();
        }
        assert_eq!(snapshot(&conn), (1, 120), "回填重复调用不产生行");

        // 四个入口叠加后，那一行仍然带着最新的事实
        let row: (Option<i64>, Option<String>, Option<i64>) = conn.query_row(
            "SELECT duration_ms, connection_id, first_token_ms FROM requests WHERE dedup_key='resp-retry-1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(row, (Some(900), Some("c-gw".into()), Some(50)));
    }

    /// 同一 `dedup_key` 在不同 `source` 下是**两条**记录：去重键是 `(source, dedup_key)`
    /// 而不是只有 `dedup_key`。两个来源各自看到同一份日志时会各记一行，不能被误合并。
    #[test]
    fn dedup_key_is_scoped_to_source_not_global() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis();
        let make = |source: &str| RequestRecord {
            platform: "codex".into(), source: source.into(), dedup_key: "same-id".into(),
            session_id: None, ts: now, model: None,
            input_tokens: Some(10), output_tokens: Some(5),
            cache_read_tokens: Some(0), cache_write_tokens: Some(0),
            total_tokens: Some(15), effort: None,
        };
        insert_records(&mut conn, &[make("local-session")]).unwrap();
        insert_records(&mut conn, &[make("native:zcode:main")]).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM requests WHERE dedup_key='same-id'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2, "不同来源的同名键是两条记录，不能合并");
    }

    #[test]
    fn calendar_ranges_keep_local_midnight_and_cross_year_labels_include_year() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let end = Local.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).single().unwrap().timestamp_millis();
        let (week_start, _) = period_range_at(&conn, "codex", "week", end);
        let week_start = Local.timestamp_millis_opt(week_start).single().unwrap();
        assert_eq!(week_start.weekday().num_days_from_monday(), 0);
        assert_eq!(week_start.format("%H:%M").to_string(), "00:00");

        let start = Local.with_ymd_and_hms(2025, 12, 30, 0, 0, 0).single().unwrap().timestamp_millis();
        let end = Local.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).single().unwrap().timestamp_millis();
        let trend = trend_at(
            &conn,
            "codex",
            "custom",
            Some(CustomRange { start, end: Some(end) }),
            None,
            end,
        ).unwrap();
        assert_eq!(trend.points[0].label, "2025/12/30");
        assert_eq!(trend.points[1].label, "2025/12/31");
        assert_eq!(trend.points[2].label, "2026/01/01");
    }

    #[test]
    fn recent_tokens_without_running_lifecycle_are_not_a_running_session() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        upsert_session_title(&conn, "codex", "session-1", "已停止的会话").unwrap();
        insert_records(
            &mut conn,
            &[RequestRecord {
                platform: "codex".into(),
                source: "local".into(),
                dedup_key: "response-1".into(),
                session_id: Some("session-1".into()),
                ts: Local::now().timestamp_millis(),
                model: Some("gpt-5".into()),
                input_tokens: Some(10),
                output_tokens: Some(5),
                cache_read_tokens: Some(0),
                cache_write_tokens: Some(0),
                total_tokens: Some(15), effort: None,
            }],
        )
        .unwrap();

        let usage = live_usage(&conn, "codex", None).unwrap();
        assert!(usage.active_sessions.is_empty());
    }

    #[test]
    fn zcode_recent_sessions_carry_real_states_but_other_native_sources_stay_recent() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis();
        let record = |platform: &str, key: &str, session: &str| RequestRecord {
            platform: platform.into(), source: "native".into(), dedup_key: key.into(),
            session_id: Some(session.into()), ts: now - 1_000, model: None,
            input_tokens: Some(10), output_tokens: Some(5), cache_read_tokens: Some(0),
            cache_write_tokens: Some(0), total_tokens: Some(15), effort: None,
        };
        insert_records(&mut conn, &[
            record("zcode", "z1", "zcode-run"), record("zcode", "z2", "zcode-done"),
            record("workbuddy", "w1", "wb-done"),
        ]).unwrap();
        set_session_activity(&conn, "zcode", "zcode-run", "running", now).unwrap();
        set_session_activity(&conn, "zcode", "zcode-done", "done", now).unwrap();
        set_session_activity(&conn, "workbuddy", "wb-done", "done", now).unwrap();
        let mut zcode: Vec<(String, String)> = live_usage(&conn, "zcode", None).unwrap().active_sessions
            .into_iter().map(|s| (s.session_id, s.state)).collect();
        zcode.sort();
        assert_eq!(zcode, vec![("zcode-done".into(), "done".into()), ("zcode-run".into(), "running".into())]);
        let workbuddy = live_usage(&conn, "workbuddy", None).unwrap().active_sessions;
        assert_eq!(workbuddy.len(), 1);
        assert_eq!(workbuddy[0].state, "recent", "没有可信生命周期的来源不借用 done 状态");
    }

    #[test]
    fn running_token_snapshot_restores_only_current_turn() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis();
        let record = |key: &str, session: &str, ts, total| RequestRecord {
            platform: "codex".into(), source: "local".into(), dedup_key: key.into(),
            session_id: Some(session.into()), ts, model: None,
            input_tokens: Some(total), output_tokens: Some(0), cache_read_tokens: Some(0),
            cache_write_tokens: Some(0), total_tokens: Some(total), effort: None,
        };
        insert_records(&mut conn, &[
            record("previous-turn", "active", now - 10_000, 900),
            record("current-turn", "active", now, 120),
            record("other-session", "other", now, 500),
        ]).unwrap();
        set_session_activity(&conn, "codex", "active", "running", now).unwrap();
        conn.execute("UPDATE sessions SET turn_started_ms = ?1 WHERE session_id = 'active'", [now - 1000]).unwrap();
        let initial = live_usage(&conn, "codex", None).unwrap();
        assert_eq!(initial.delta_tokens, 0);
        assert_eq!(initial.running_tokens, Some(120));
        assert_eq!(initial.active_sessions[0].running_tokens, Some(120));
        assert_eq!(live_usage(&conn, "codex", Some(initial.cursor)).unwrap().running_tokens, Some(120));
        set_session_activity(&conn, "codex", "other", "running", now).unwrap();
        conn.execute("UPDATE sessions SET turn_started_ms = ?1 WHERE session_id = 'other'", [now - 1000]).unwrap();
        let multiple = live_usage(&conn, "codex", None).unwrap();
        assert_eq!(multiple.running_tokens, Some(620));
        assert_eq!(multiple.active_sessions.iter().find(|s| s.session_id == "active").unwrap().running_tokens, Some(120));
        assert_eq!(multiple.active_sessions.iter().find(|s| s.session_id == "other").unwrap().running_tokens, Some(500));
        conn.execute("UPDATE sessions SET turn_started_ms = ?1 WHERE session_id = 'active'", [now + 1]).unwrap();
        let next = live_usage(&conn, "codex", None).unwrap();
        assert_eq!(next.running_tokens, Some(500));
        assert_eq!(next.active_sessions.iter().find(|s| s.session_id == "active").unwrap().running_tokens, Some(0));
    }

    /// 会话按首次出现时间降序：最新的会话在最前面（2026-09-19 鼠鼠定版）。
    /// Token 更新只改活跃度、不改首次时间，因此老会话持续出 Token 也不会
    /// 跳到新会话上面；列表只在真正出现新会话时才重排。
    #[test]
    fn sessions_order_by_first_seen_not_activity() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis();
        let record = |key: &str, session: &str, ts| RequestRecord {
            platform: "claude".into(), source: "local".into(), dedup_key: key.into(),
            session_id: Some(session.into()), ts, model: None,
            input_tokens: Some(1), output_tokens: Some(0), cache_read_tokens: Some(0),
            cache_write_tokens: Some(0), total_tokens: Some(1), effort: None,
        };
        // old 会话首次出现更早，但刚刚还在出 Token（活跃度最新）；
        // new 会话刚刚创建（首次时间最新），此后静默。
        insert_records(&mut conn, &[
            record("old-first", "old", now - 60_000),
            record("old-latest", "old", now),
            record("new-first", "new", now - 30_000),
        ]).unwrap();
        set_session_activity(&conn, "claude", "old", "running", now).unwrap();
        set_session_activity(&conn, "claude", "new", "running", now - 30_000).unwrap();

        let sessions: Vec<String> = live_usage(&conn, "claude", None).unwrap()
            .active_sessions.into_iter().map(|s| s.session_id).collect();
        assert_eq!(sessions, vec!["new".to_string(), "old".to_string()],
            "首次时间最新的会话排最前，活跃度不参与排序");
    }

    #[test]
    fn codex_live_usage_excludes_cached_input_tokens() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis();
        // 查询侧「今日」窗口的上界取的是查询时刻的系统时钟，虚拟机时钟偶发回跳
        // 会把刚插入的记录挤出窗口，导致 today 断言间歇性拿到 0。
        // 记录时间戳因此留出 60 秒余量；午夜前后运行时退化为零点后 1 毫秒，保证仍落今日窗口内。
        let ts = (now - 60_000).max(start_of_today_at(now).timestamp_millis() + 1);
        set_session_activity(&conn, "codex", "cached-turn", "running", now).unwrap();
        conn.execute(
            "UPDATE sessions SET turn_started_ms = ?1 WHERE platform = 'codex' AND session_id = 'cached-turn'",
            [ts - 1_000],
        ).unwrap();
        insert_records(&mut conn, &[RequestRecord {
            platform: "codex".into(),
            source: "local".into(),
            dedup_key: "cached-response".into(),
            session_id: Some("cached-turn".into()),
            ts,
            model: Some("gpt-5".into()),
            // Codex input_tokens 已包含 cached_input_tokens：100 = 20 新鲜 + 80 缓存。
            input_tokens: Some(100),
            output_tokens: Some(20),
            cache_read_tokens: Some(80),
            cache_write_tokens: Some(0),
            total_tokens: Some(120),
            effort: None,
        }]).unwrap();

        let live = live_usage(&conn, "codex", None).unwrap();
        assert_eq!(live.running_tokens, Some(40));
        assert_eq!(live.today_tokens, Some(40));
        // 主面板的总量继续使用来源 total_tokens，保留完整消耗记录。
        assert_eq!(token_totals_at(&conn, "codex", None, None, now + 1).unwrap().today, Some(120));

        // 同一 response 后续上报累计值时，只把新鲜 Token 的差额写入事件。
        let cursor = live.cursor;
        insert_records(&mut conn, &[RequestRecord {
            platform: "codex".into(),
            source: "local".into(),
            dedup_key: "cached-response".into(),
            session_id: Some("cached-turn".into()),
            ts,
            model: Some("gpt-5".into()),
            input_tokens: Some(120),
            output_tokens: Some(30),
            cache_read_tokens: Some(100),
            cache_write_tokens: Some(0),
            total_tokens: Some(150),
            effort: None,
        }]).unwrap();
        let updated = live_usage(&conn, "codex", Some(cursor)).unwrap();
        assert_eq!(updated.delta_tokens, 10);
        assert_eq!(updated.running_tokens, Some(50));
        assert_eq!(updated.today_tokens, Some(50));
    }

    /// P4 审计 D5：原生平台的今日 Token 与灵动岛增量同为新鲜口径；拆不出缓存的行沿用总量。
    #[test]
    fn native_today_tokens_use_the_fresh_measure() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis() - 1;
        let insert = |key: &str, semantics: &str, input: i64, cache: i64, output: i64, total: i64| {
            conn.execute(
                "INSERT INTO requests(platform, source, dedup_key, ts, input_tokens, cache_read_tokens, output_tokens, total_tokens, input_semantics)
                 VALUES('qoder', 'native:qoder:global', ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![key, now, input, cache, output, total, semantics],
            )
            .unwrap();
        };
        // 输入 1000（其中缓存读 900）+ 输出 10：新鲜值 110，不是总量 1010
        insert("a", "includes_cache", 1000, 900, 10, 1010);
        assert_eq!(today_fresh_tokens(&conn, "qoder").unwrap(), Some(110));
        // 不含缓存口径：输入 + 输出
        insert("b", "excludes_cache", 20, 500, 5, 525);
        assert_eq!(today_fresh_tokens(&conn, "qoder").unwrap(), Some(135));
        // 语义未知：拆不出缓存，只能沿用来源总量
        insert("c", "unknown", 0, 0, 0, 40);
        assert_eq!(today_fresh_tokens(&conn, "qoder").unwrap(), Some(175));
    }

    /// P1-4：灵动岛入口（`live_usage`）必须把覆盖一起带出去。
    ///
    /// 只给 `today_tokens` 时，前端拿到 null 分不清「今天没请求」和「请求都没 Token」，
    /// 只能二选一猜一个——早期就是在这里把未知画成了 0。
    #[test]
    fn live_usage_carries_today_token_coverage() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis() - 1;
        let record = |key: &str, total: Option<i64>| RequestRecord {
            platform: "claude".into(), source: "local".into(), dedup_key: key.into(),
            session_id: Some("s1".into()), ts: now, model: Some("claude-sonnet-4".into()),
            input_tokens: total, output_tokens: Some(1),
            cache_read_tokens: Some(0), cache_write_tokens: Some(0),
            total_tokens: total, effort: None,
        };

        // 一条有数、一条没有：覆盖是 1 已知 / 1 未知，总量只算已知的那条
        insert_records(&mut conn, &[record("a", Some(40)), record("b", None)]).unwrap();
        let live = live_usage(&conn, "claude", None).unwrap();
        assert_eq!(live.today_token_coverage.known(), 1);
        assert_eq!(live.today_token_coverage.unknown(), 1);
        assert_eq!(live.today_token_coverage.state(), "partial");
        // total 与 today_tokens 同源（都是 `today_fresh_tokens`），不会两套口径
        assert_eq!(live.today_token_coverage.total(), live.today_tokens);
        // 灵动岛的来源元数据与覆盖同一个「今天」
        let (start, end) = period_range(&conn, "claude", "today");
        // 终点是「此刻」，两次调用差几毫秒：起点必须相同，终点不晚于后取的那次
        assert_eq!(live.today_provenance.window_start, start);
        assert!(live.today_provenance.window_end <= end);
        assert!(live.today_provenance.data_at.is_some(), "今天有记录时数据时间不能为空");
        assert_eq!(live.today_provenance.unit, crate::data_contract::Unit::Tokens);
        assert_eq!(live.today_tokens, Some(41), "40 输入 + 1 输出");
    }

    #[test]
    fn failed_sessions_stay_visible_within_window_without_counting_as_running() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis();
        let record = |key: &str, session: &str, ts, total| RequestRecord {
            platform: "claude".into(), source: "local".into(), dedup_key: key.into(),
            session_id: Some(session.into()), ts, model: None,
            input_tokens: Some(total), output_tokens: Some(0), cache_read_tokens: Some(0),
            cache_write_tokens: Some(0), total_tokens: Some(total), effort: None,
        };
        // 一轮有真实 Token 的失败会话：请求报错后状态转为 failed
        insert_records(&mut conn, &[record("failed-turn", "errored", now - 30_000, 40)]).unwrap();
        set_session_activity(&conn, "claude", "errored", "running", now - 60_000).unwrap();
        conn.execute("UPDATE sessions SET turn_started_ms = ?1 WHERE session_id = 'errored'", [now - 60_000]).unwrap();
        set_session_activity(&conn, "claude", "errored", "failed", now - 30_000).unwrap();
        // 超出 90 秒窗口的旧失败会话不应再出现
        set_session_activity(&conn, "claude", "stale-failed", "failed", now - 120_000).unwrap();

        let usage = live_usage(&conn, "claude", None).unwrap();
        let states: Vec<_> = usage.active_sessions.iter()
            .map(|s| (s.session_id.as_str(), s.state.as_str())).collect();
        assert_eq!(states, vec![("errored", "failed")], "窗口内的失败会话保留 failed 状态，超窗失败不出现");
        assert_eq!(usage.active_sessions[0].running_tokens, Some(40), "失败会话展示本轮最终 Token");
        assert_eq!(usage.running_tokens, None, "没有 running 会话时运行合计为空，失败会话不计入");
        // 有 running 会话并存时，运行合计只统计 running
        insert_records(&mut conn, &[record("live-turn", "live", now, 120)]).unwrap();
        set_session_activity(&conn, "claude", "live", "running", now).unwrap();
        conn.execute("UPDATE sessions SET turn_started_ms = ?1 WHERE session_id = 'live'", [now - 1000]).unwrap();
        let usage = live_usage(&conn, "claude", None).unwrap();
        assert_eq!(usage.running_tokens, Some(120), "失败会话的最终 Token 不得计入运行合计");
        assert_eq!(usage.active_sessions.iter().find(|s| s.session_id == "errored").unwrap().running_tokens, Some(40));
    }

    #[test]
    fn twenty_year_trend_is_bounded_and_preserves_usage_totals() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let start = Local.with_ymd_and_hms(2006, 1, 1, 0, 0, 0).single().unwrap().timestamp_millis();
        let end = Local.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap().timestamp_millis();
        let records: Vec<_> = (0..20_000).map(|index| RequestRecord {
            platform: "codex".into(), source: "local".into(), dedup_key: format!("long-{index}"),
            session_id: None, ts: start + (end - start - 1) * index / 19_999, model: None,
            input_tokens: Some(2), output_tokens: Some(1), cache_read_tokens: Some(1),
            cache_write_tokens: Some(0), total_tokens: Some(3), effort: None,
        }).collect();
        insert_records(&mut conn, &records).unwrap();
        let range = Some(CustomRange { start, end: Some(end) });
        let trend = trend_at(&conn, "codex", "custom", range, None, end).unwrap();
        assert!(trend.points.len() <= 180);
        assert_eq!(trend.points.iter().filter_map(|point| point.tokens).sum::<i64>(), 60_000);
        let page = request_log_at(&conn, "codex", "custom", 1, range, None, end).unwrap();
        assert_eq!(page.total_count, 20_000);
        assert_eq!(page.rows.len(), PAGE_SIZE as usize);
    }

    #[test]
    fn fresh_running_lifecycle_is_returned_as_running() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        upsert_session_title(&conn, "codex", "session-1", "正在处理的会话").unwrap();
        conn.execute(
            "UPDATE sessions SET is_running = 1, activity_state = 'running', activity_updated_ms = ?1
             WHERE platform = 'codex' AND session_id = 'session-1'",
            params![Local::now().timestamp_millis()],
        )
        .unwrap();

        let usage = live_usage(&conn, "codex", None).unwrap();
        assert_eq!(usage.active_sessions.len(), 1);
        assert_eq!(usage.active_sessions[0].title, "正在处理的会话");
    }

    #[test]
    fn lifecycle_completion_is_removed_immediately() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        upsert_session_title(&conn, "codex", "session-1", "刚完成的会话").unwrap();
        let now = Local::now().timestamp_millis();

        set_session_activity(&conn, "codex", "session-1", "running", now).unwrap();
        assert_eq!(live_usage(&conn, "codex", None).unwrap().active_sessions.len(), 1);

        set_session_activity(&conn, "codex", "session-1", "done", now + 1).unwrap();
        let completed = live_usage(&conn, "codex", None).unwrap();
        assert!(completed.active_sessions.is_empty());

        conn.execute(
            "UPDATE sessions SET activity_updated_ms = ?1 WHERE platform = 'codex' AND session_id = 'session-1'",
            params![now - 2_000],
        ).unwrap();
        assert!(live_usage(&conn, "codex", None).unwrap().active_sessions.is_empty());
    }

    #[test]
    fn running_session_does_not_expire_while_context_may_be_compacting() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        upsert_session_title(&conn, "codex", "session-1", "正在自动压缩上下文").unwrap();
        let before_long_compaction = Local::now().timestamp_millis() - 60_000;
        set_session_activity(
            &conn,
            "codex",
            "session-1",
            "running",
            before_long_compaction,
        )
        .unwrap();

        let usage = live_usage(&conn, "codex", None).unwrap();
        assert_eq!(usage.active_sessions.len(), 1);
        assert_eq!(usage.active_sessions[0].title, "正在自动压缩上下文");
    }

    #[test]
    fn silent_unfinished_session_keeps_running_until_explicit_end() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis();
        set_session_activity(&conn, "codex", "stale", "running", now - 301_000).unwrap();
        assert_eq!(live_usage(&conn, "codex", None).unwrap().active_sessions.len(), 1);
        set_session_activity(&conn, "codex", "stale", "running", now).unwrap();
        assert_eq!(live_usage(&conn, "codex", None).unwrap().active_sessions.len(), 1);
        set_session_activity(&conn, "codex", "stale", "failed", now).unwrap();
        let sessions = live_usage(&conn, "codex", None).unwrap().active_sessions;
        assert_eq!(sessions.len(), 1, "最终失败后在窗口内短暂展示失败状态");
        assert_eq!(sessions[0].state, "failed");
        conn.execute(
            "UPDATE sessions SET activity_updated_ms = ?1 WHERE session_id = 'stale'",
            [now - ACTIVE_WINDOW_SECONDS * 1_000 - 1],
        )
        .unwrap();
        assert!(
            live_usage(&conn, "codex", None).unwrap().active_sessions.is_empty(),
            "失败展示超过 90 秒窗口后消失"
        );
    }

    #[test]
    fn waiting_session_preserves_tokens_and_start_across_long_silence() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let started = Local::now().timestamp_millis() - 86_400_000;
        set_session_activity(&conn, "codex", "waiting", "running", started).unwrap();
        conn.execute("UPDATE sessions SET turn_started_ms = ?1 WHERE session_id = 'waiting'", [started]).unwrap();
        insert_records(&mut conn, &[RequestRecord {
            platform: "codex".into(), source: "local".into(), dedup_key: "before-wait".into(),
            session_id: Some("waiting".into()), ts: started + 1000, model: None,
            input_tokens: Some(100), output_tokens: Some(20), cache_read_tokens: Some(0),
            cache_write_tokens: Some(0), total_tokens: Some(120), effort: None,
        }]).unwrap();
        let initial = live_usage(&conn, "codex", None).unwrap();
        assert_eq!(initial.running_tokens, Some(120));
        assert_eq!(initial.active_sessions[0].started_at_ms, Some(started));
        let heartbeat = live_usage(&conn, "codex", Some(initial.cursor)).unwrap();
        assert_eq!(heartbeat.delta_tokens, 0);
        assert_eq!(heartbeat.running_tokens, Some(120));
        set_session_activity(&conn, "codex", "waiting", "done", Local::now().timestamp_millis()).unwrap();
        let ended = live_usage(&conn, "codex", Some(initial.cursor)).unwrap();
        assert!(ended.active_sessions.is_empty());
        assert_eq!(ended.running_tokens, None);
    }

    /// P1-4：灵动岛的「今日 Token」覆盖要能区分「今天没记录」与「有记录但全没给 Token」。
    ///
    /// 单一个 `today_tokens: Option<i64>` 表达不了这个差别——两者都是 None。
    /// `today_token_coverage` 用共享契约把它补上：`known` 为 0 说明全未知，
    /// 而「没记录」时 `known + unknown` 也是 0。
    #[test]
    fn today_token_coverage_separates_empty_from_all_unknown() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        // `period_range` 的右边界是「此刻」且为开区间（`ts < end`），
        // 用 `now` 当 ts 会和取边界的时刻撞在同一毫秒导致整行落在窗口外——这里退 1ms。
        let now = Local::now().timestamp_millis() - 1;
        // 输出给非零值：全零 + 有未知的行会被 `EXCLUDE_DEGENERATE_SQL` 当退化记录排除，
        // 那样测的就不是覆盖逻辑了。
        let record = |key: &str, total: Option<i64>| RequestRecord {
            platform: "claude".into(),
            source: "local".into(),
            dedup_key: key.into(),
            session_id: None,
            ts: now,
            model: Some("claude-sonnet-4".into()),
            input_tokens: total,
            output_tokens: Some(1),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            total_tokens: total,
            effort: None,
        };

        // 1) 今天完全没有记录：覆盖是「零条」，不是「未知」
        let empty = today_token_coverage(&conn, "claude", None).unwrap();
        assert_eq!((empty.known(), empty.unknown()), (0, 0), "没有记录 = 零条，不是未知");

        // 2) 有记录但全都没给 Token：这才是真正的未知
        insert_records(&mut conn, &[record("u1", None)]).unwrap();
        let unknown = today_token_coverage(&conn, "claude", None).unwrap();
        assert_eq!((unknown.known(), unknown.unknown()), (0, 1), "有 1 条但没 Token");
        assert_eq!(unknown.state(), "missing");
        assert_eq!(today_fresh_tokens(&conn, "claude").unwrap(), None, "全未知时总量是 None");

        // 3) 部分有值：已知与未知分开计
        insert_records(&mut conn, &[record("u2", Some(100))]).unwrap();
        let partial = today_token_coverage(&conn, "claude", None).unwrap();
        assert_eq!((partial.known(), partial.unknown()), (1, 1));
        assert_eq!(partial.state(), "partial");
        // Claude 的新鲜 Token 是 input + output（缓存单列），所以是 100 + 1
        assert_eq!(today_fresh_tokens(&conn, "claude").unwrap(), Some(101), "已知部分照常给出");
    }

    #[test]
    fn missing_token_fields_remain_unknown_while_real_zero_is_preserved() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis();
        let record = |key: &str, input: Option<i64>, total: Option<i64>| RequestRecord {
            platform: "claude".into(),
            source: "local".into(),
            dedup_key: key.into(),
            session_id: None,
            ts: now,
            model: Some("claude-sonnet-4".into()),
            input_tokens: input,
            output_tokens: Some(0),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            total_tokens: total, effort: None,
        };
        insert_records(&mut conn, &[
            record("missing", None, None),
            record("zero", Some(0), Some(0)),
        ]).unwrap();

        let page = request_log_at(&conn, "claude", "total", 1, None, None, now + 1).unwrap();
        let missing = page.rows.iter().find(|row| row.input.is_none()).unwrap();
        let zero = page.rows.iter().find(|row| row.input == Some(0)).unwrap();
        assert_eq!((missing.input, missing.total, missing.cost_estimate), (None, None, None));
        assert_eq!((zero.input, zero.total), (Some(0), Some(0)));
        assert_eq!(token_totals_at(&conn, "claude", None, None, now + 1).unwrap().total, None);
        assert!(!cost_estimate_at(&conn, "claude", "total", None, None, now + 1).unwrap().complete);
    }

    #[test]
    fn history_cleanup_deletes_old_data_but_keeps_scan_offsets() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let record = |key: &str, ts: i64| RequestRecord {
            platform: "codex".into(), source: "local".into(), dedup_key: key.into(),
            session_id: Some(key.into()), ts, model: None, input_tokens: Some(1),
            output_tokens: Some(1), cache_read_tokens: Some(0), cache_write_tokens: Some(0),
            total_tokens: Some(2), effort: None,
        };
        insert_records(&mut conn, &[record("old", 100), record("new", 200)]).unwrap();
        set_offset(&conn, "history.jsonl", 900, 12).unwrap();
        set_session_activity(&conn, "codex", "old", "done", 100).unwrap();

        let preview = cleanup_preview(&conn, 150).unwrap();
        assert_eq!((preview.requests, preview.usage_events, preview.sessions), (1, 1, 1));
        let deleted = cleanup_history(&mut conn, 150).unwrap();
        assert_eq!(deleted.requests, 1);
        assert_eq!(get_offset(&conn, "history.jsonl"), 900);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM requests", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn backup_round_trip_is_deduplicated_and_excludes_credentials() {
        let mut source = Connection::open_in_memory().unwrap();
        init(&source).unwrap();
        insert_records(&mut source, &[RequestRecord {
            platform: "claude".into(), source: "local".into(), dedup_key: "request-1".into(),
            session_id: Some("session-1".into()), ts: 1_000, model: Some("claude-sonnet-4".into()),
            input_tokens: Some(10), output_tokens: None, cache_read_tokens: Some(2),
            cache_write_tokens: None, total_tokens: Some(12), effort: None,
        }]).unwrap();
        upsert_session_title(&source, "claude", "session-1", "会话标题").unwrap();
        source.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES('c','claude','api','私有连接','secret-value','****',1)",
            [],
        ).unwrap();

        let json = export_backup(&source, Some("claude"), None, None).unwrap();
        assert!(!json.contains("secret-value"));
        assert!(!json.contains("私有连接"));

        let mut target = Connection::open_in_memory().unwrap();
        init(&target).unwrap();
        let first = import_backup(&mut target, &json).unwrap();
        let second = import_backup(&mut target, &json).unwrap();
        assert_eq!((first.requests_changed, first.sessions_changed), (1, 1));
        assert_eq!((second.requests_changed, second.sessions_changed), (0, 0));
        assert_eq!((first.skipped, second.skipped), (0, 1));
        let row = request_log_at(&target, "claude", "total", 1, None, None, 2_000).unwrap().rows.remove(0);
        assert_eq!((row.input, row.output, row.total), (Some(10), None, Some(12)));
    }

    fn import_record(key: &str, ts: i64, total: i64) -> RequestRecord {
        RequestRecord {
            platform: "claude".into(), source: "local".into(), dedup_key: key.into(),
            session_id: Some(format!("session-{key}")), ts, model: Some("claude-sonnet-4".into()),
            input_tokens: Some(total), output_tokens: Some(0), cache_read_tokens: Some(0),
            cache_write_tokens: Some(0), total_tokens: Some(total), effort: None,
        }
    }

    /// 文件 5 条：2 条与库里重复（Token 值不同）、2 条新记录、1 条缺 dedup_key
    fn preview_fixture() -> (Connection, String) {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        insert_records(&mut conn, &[import_record("dup-1", 1_000, 100), import_record("dup-2", 2_000, 200)]).unwrap();
        let mut missing = serde_json::to_value(import_record("broken", 5_000, 5)).unwrap();
        missing.as_object_mut().unwrap().remove("dedup_key");
        let requests = vec![
            serde_json::to_value(import_record("dup-1", 1_000, 999)).unwrap(),
            serde_json::to_value(import_record("dup-2", 2_000, 999)).unwrap(),
            serde_json::to_value(import_record("new-1", 3_000, 30)).unwrap(),
            serde_json::to_value(import_record("new-2", 4_000, 40)).unwrap(),
            missing,
        ];
        let json = serde_json::json!({
            "schema_version": 2, "exported_at_ms": 9_000, "platform": null,
            "requests": requests,
            "sessions": [{ "platform": "claude", "session_id": "session-new-1", "title": "新会话" }],
        }).to_string();
        (conn, json)
    }

    fn total_of(conn: &Connection, key: &str) -> i64 {
        conn.query_row("SELECT total_tokens FROM requests WHERE dedup_key = ?1", [key], |row| row.get(0)).unwrap()
    }

    #[test]
    fn preview_counts_match_import() {
        let (mut conn, json) = preview_fixture();
        let (preview, parsed) = preview_backup(&conn, &json).unwrap();
        assert_eq!((preview.valid, preview.duplicate, preview.invalid), (2, 2, 1));
        assert_eq!(preview.invalid_reasons.len(), 1);
        assert!(preview.invalid_reasons[0].contains("dedup_key"), "{:?}", preview.invalid_reasons);
        assert!(!preview.token.is_empty());

        let summary = import_parsed(&mut conn, &parsed).unwrap();
        assert_eq!((summary.requests_changed, summary.skipped), (2, 3));
        // 重复项跳过：库里原来那 2 条的 Token 值不变
        assert_eq!((total_of(&conn, "dup-1"), total_of(&conn, "dup-2")), (100, 200));
        assert_eq!((total_of(&conn, "new-1"), total_of(&conn, "new-2")), (30, 40));
    }

    #[test]
    fn preview_rejects_requests_with_malformed_metrics() {
        for valid_first in [false, true] {
            let mut conn = Connection::open_in_memory().unwrap();
            init(&conn).unwrap();
            let native = |key: &str| RequestRecord { platform: "workbuddy".into(), source: "native:workbuddy:main".into(), ..import_record(key, 4_000, 7) };
            let good = serde_json::json!({"source":"native:workbuddy:main", "dedup_key":"broken", "input_semantics":"includes_cache", "credits":1.5});
            let mut malformed = good.clone();
            malformed["credits"] = serde_json::json!("damaged");
            let metrics = if valid_first { vec![good, malformed] } else { vec![malformed, good] };
            let json = serde_json::json!({"schema_version":2, "requests":[native("broken"), native("good")], "sessions":[], "metrics":metrics}).to_string();
            let (preview, parsed) = preview_backup(&conn, &json).unwrap();
            assert_eq!(preview.valid, 1, "损坏指标关联的请求不得导入，顺序={valid_first}");
            assert!(!parsed.metrics.contains_key(&("native:workbuddy:main".into(), "broken".into())));
            let result = import_parsed(&mut conn, &parsed).unwrap();
            assert_eq!(result.requests_changed, 1);
            assert!(!request_exists(&conn, "native:workbuddy:main", "broken").unwrap());
            assert!(request_exists(&conn, "native:workbuddy:main", "good").unwrap());
        }
    }

    /// `native_outcome` 的值域由 [`NATIVE_OUTCOMES`] 统一描述：四值放行，拼错的值拒绝，
    /// 缺省（来源没给结论）合法。备份导入校验必须与 ZCode 的新映射同步，否则含
    /// `cancelled` 的备份会被当成"损坏指标"整条丢掉。
    #[test]
    fn backup_metrics_accept_cancelled_and_reject_unknown_values() {
        for value in ["success", "failed", "cancelled", "unknown"] {
            assert!(native_outcome_valid(Some(value)), "{value} 应在值域内");
        }
        assert!(native_outcome_valid(None), "缺少结论是合法的");
        for value in ["canceled", "cancel", "aborted", "CANCELLED", ""] {
            assert!(!native_outcome_valid(Some(value)), "{value} 不在值域内");
        }
        assert_eq!(NATIVE_OUTCOMES, ["success", "failed", "cancelled", "unknown"]);
    }

    /// 端到端：含 cancelled 指标的备份能通过预览并原样导入（旧值域会把它整条判无效）。
    #[test]
    fn backup_round_trip_preserves_cancelled_outcome() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let request = RequestRecord {
            platform: "zcode".into(), source: "native:zcode:main".into(),
            ..import_record("cancelled-1", 4_000, 7)
        };
        let json = serde_json::json!({
            "schema_version": 2, "exported_at_ms": 9_000, "platform": null,
            "requests": [request], "sessions": [],
            "metrics": [{
                "source": "native:zcode:main", "dedup_key": "cancelled-1",
                "input_semantics": "unknown", "native_outcome": "cancelled",
            }],
        }).to_string();
        let (preview, parsed) = preview_backup(&conn, &json).unwrap();
        assert_eq!(preview.valid, 1, "{:?}", preview.invalid_reasons);
        assert_eq!(parsed.metrics.len(), 1, "含 cancelled 的指标不能被判为无效");
        import_parsed(&mut conn, &parsed).unwrap();
        let outcome: Option<String> = conn
            .query_row("SELECT native_outcome FROM requests WHERE dedup_key='cancelled-1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(outcome.as_deref(), Some("cancelled"));
    }

    #[test]
    fn import_tokens_carry_random_part_and_are_unique() {
        let (a, b) = (import_token(), import_token());
        assert_ne!(a, b);
        // 最后一段是 RandomState 哈希得到的 32 位十六进制，不能由时间和序号推出
        let random = a.rsplit('-').next().unwrap();
        assert_eq!(random.len(), 32, "{a}");
        assert!(random.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(random, b.rsplit('-').next().unwrap());
    }

    #[test]
    fn preview_writes_nothing() {
        let (conn, json) = preview_fixture();
        let counts = |conn: &Connection| -> Vec<i64> {
            ["requests", "sessions", "usage_events", "request_metrics", "native_context"].iter()
                .map(|table| conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).unwrap())
                .collect()
        };
        let before = counts(&conn);
        preview_backup(&conn, &json).unwrap();
        assert_eq!(counts(&conn), before);
    }

    #[test]
    fn preview_skips_native_records_before_cleanup_cutoff() {
        let (mut conn, _) = preview_fixture();
        conn.execute("UPDATE collection_policy SET cutoff_ms = 3_500 WHERE singleton = 1", []).unwrap();
        let native = |key: &str, ts: i64| RequestRecord { platform: "workbuddy".into(), source: "native:workbuddy:main".into(), ..import_record(key, ts, 7) };
        let json = serde_json::json!({
            "schema_version": 2, "exported_at_ms": 9_000, "platform": null,
            "requests": [native("old", 3_000), native("fresh", 4_000)], "sessions": [],
        }).to_string();
        let (preview, parsed) = preview_backup(&conn, &json).unwrap();
        assert_eq!((preview.valid, preview.duplicate, preview.invalid), (1, 0, 1));
        assert!(preview.invalid_reasons[0].starts_with("第 1 条请求"), "{:?}", preview.invalid_reasons);
        let summary = import_parsed(&mut conn, &parsed).unwrap();
        assert_eq!((summary.requests_changed, summary.skipped), (1, 1));
    }

    #[test]
    fn export_range_is_half_open() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let (start, end) = (10_000, 20_000);
        insert_records(&mut conn, &[
            import_record("at-start", start, 1),
            import_record("before-end", end - 1, 2),
            import_record("at-end", end, 3),
        ]).unwrap();
        for key in ["at-start", "before-end", "at-end"] {
            upsert_session_title(&conn, "claude", &format!("session-{key}"), key).unwrap();
        }
        let json = export_backup(&conn, None, Some(start), Some(end)).unwrap();
        let file: serde_json::Value = serde_json::from_str(&json).unwrap();
        let keys: Vec<&str> = file["requests"].as_array().unwrap().iter().map(|r| r["dedup_key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["at-start", "before-end"]);
        // 会话标题只导出区间内有请求的会话
        let titles: Vec<&str> = file["sessions"].as_array().unwrap().iter().map(|s| s["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["at-start", "before-end"]);
        // 不传日期时导出全部
        let all: serde_json::Value = serde_json::from_str(&export_backup(&conn, None, None, None).unwrap()).unwrap();
        assert_eq!(all["requests"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn serve_enabled_column_migrates_and_defaults_off() {
        // 模拟旧库：没有 serve_enabled 列。init 的幂等 ALTER 补列，默认 0。
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE connections (
                id TEXT PRIMARY KEY, platform TEXT NOT NULL, kind TEXT NOT NULL,
                name TEXT NOT NULL, label TEXT NOT NULL DEFAULT '', secret TEXT,
                masked TEXT NOT NULL DEFAULT '', base_url TEXT, status TEXT NOT NULL DEFAULT 'connected',
                last_sync_ms INTEGER, created_ms INTEGER NOT NULL
            );
            INSERT INTO connections (id, platform, kind, name, created_ms) VALUES ('c1', 'claude', 'api', '旧连接', 1);",
        ).unwrap();
        init(&conn).unwrap();
        let served: i64 = conn
            .query_row("SELECT serve_enabled FROM connections WHERE id='c1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(served, 0, "旧库升级后 serve_enabled 必须默认关闭");
        // 新库直接 init 也应有该列
        let fresh = Connection::open_in_memory().unwrap();
        init(&fresh).unwrap();
        let cols: i64 = fresh
            .query_row("SELECT COUNT(*) FROM pragma_table_info('connections') WHERE name='serve_enabled'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cols, 1);
    }

    /// `request_uid` 是后加的列：旧库升级后要能补上、旧行留 NULL、新行能写。
    ///
    /// 为什么单独测：它是「一次用户请求对应多条路由尝试」的唯一持久身份。
    /// 迁移失败会让统计退回按行计数（一次重试算两次），而这在界面上看不出来——
    /// 数字只是偏大，没有任何报错。
    /// P1「旧数据库升级」：构造一个真正旧版的 `usage.db`（缺后来加的全部列），
    /// 跑真实迁移路径，验证四件事：旧记录不丢、旧值不变、新列可用、重复迁移幂等。
    ///
    /// ## 为什么不能只测「ALTER 加列能跑」
    ///
    /// `init` 里有 47 处 `ALTER TABLE`，分属 6 张表，绝大多数被 `let _ =` 吞掉错误
    /// （列已存在时报错是正常的）。这种写法很容易在某一张表上漏掉、或把 NOT NULL
    /// 默认值写错。单测若只验「列存在」，漏掉默认值时旧行的语义会悄悄变掉：
    /// `*_known` 若默认 0，历史行会被当成「Token 未知」，整库统计立刻变灰。
    ///
    /// 所以这条固定样本从最早的 schema 开始升级，逐表断言
    /// 「列存在 + 旧行默认值语义正确 + 旧数据原样」。
    #[test]
    fn an_old_schema_upgrades_without_losing_rows_or_changing_their_meaning() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE requests (
                id INTEGER PRIMARY KEY, platform TEXT NOT NULL, source TEXT NOT NULL,
                dedup_key TEXT NOT NULL, session_id TEXT, ts INTEGER NOT NULL, model TEXT,
                input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                UNIQUE(source, dedup_key)
             );
             INSERT INTO requests(platform,source,dedup_key,ts,model,input_tokens,output_tokens,total_tokens)
               VALUES('claude','local-session','m1',1000,'claude-sonnet-4',100,20,120);
             INSERT INTO requests(platform,source,dedup_key,ts,model,input_tokens,output_tokens,total_tokens)
               VALUES('codex','local-session','m2',2000,'gpt-5',50,10,60);
             CREATE TABLE connections (
                id TEXT PRIMARY KEY, platform TEXT NOT NULL, kind TEXT NOT NULL, name TEXT NOT NULL,
                secret TEXT, masked TEXT, status TEXT, created_ms INTEGER
             );
             INSERT INTO connections VALUES('c-old','claude','auth','旧账号','s','****','connected',1);
             CREATE TABLE sessions (
                platform TEXT NOT NULL, session_id TEXT NOT NULL, title TEXT,
                PRIMARY KEY(platform, session_id)
             );
             INSERT INTO sessions VALUES('claude','s1','旧标题');
             CREATE TABLE gateway_route_log (
                id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, agent TEXT,
                requested_model TEXT NOT NULL, result TEXT NOT NULL
             );
             INSERT INTO gateway_route_log(ts,requested_model,result) VALUES(1000,'group/x','ok');
             CREATE TABLE scan_state (
                path TEXT PRIMARY KEY, offset INTEGER NOT NULL, mtime INTEGER NOT NULL
             );
             INSERT INTO scan_state VALUES('/tmp/old.jsonl', 123, 456);",
        )
        .unwrap();

        init(&conn).unwrap();

        let column_exists = |table: &str, column: &str| -> bool {
            conn.query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name='{column}'"),
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
                > 0
        };

        let expected: &[(&str, &str)] = &[
            ("requests", "input_known"),
            ("requests", "output_known"),
            ("requests", "cache_read_known"),
            ("requests", "cache_write_known"),
            ("requests", "total_known"),
            ("requests", "effort"),
            ("requests", "duration_ms"),
            ("requests", "first_token_ms"),
            ("requests", "status_code"),
            ("requests", "connection_id"),
            ("requests", "provider_key"),
            ("connections", "base_url"),
            ("connections", "model"),
            ("connections", "effort"),
            ("connections", "context_1m"),
            ("connections", "protocol"),
            ("connections", "service"),
            ("connections", "account_email"),
            ("connections", "serve_enabled"),
            ("connections", "start_channel"),
            ("connections", "slug"),
            ("connections", "account_fp"),
            ("connections", "models_json"),
            ("connections", "catalog_auto"),
            ("connections", "exhausted_day"),
            ("sessions", "cwd"),
            ("sessions", "is_running"),
            ("sessions", "activity_state"),
            ("sessions", "activity_updated_ms"),
            ("sessions", "turn_started_ms"),
            ("gateway_route_log", "connection_id"),
            ("gateway_route_log", "session_platform"),
            ("gateway_route_log", "session_id"),
            ("gateway_route_log", "client_key"),
            ("gateway_route_log", "request_id"),
            ("gateway_route_log", "request_uid"),
            ("gateway_route_log", "resolved_from"),
            ("gateway_route_log", "profile"),
            ("scan_state", "parser_model"),
            ("scan_state", "parser_session_id"),
            ("scan_state", "parser_activity_state"),
            ("scan_state", "parser_updated_at_ms"),
            ("scan_state", "parser_effort"),
            ("scan_state", "parser_initialized"),
        ];
        for (table, column) in expected {
            assert!(column_exists(table, column), "{table} 缺列 {column}");
        }

        let rows: Vec<(String, i64, i64, i64)> = conn
            .prepare("SELECT dedup_key, ts, input_tokens, total_tokens FROM requests ORDER BY ts")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![("m1".to_string(), 1000, 100, 120), ("m2".to_string(), 2000, 50, 60)],
            "旧记录必须原样保留"
        );

        // 最容易漏的一处：新增的 *_known 默认必须是 1（已知）。默认成 0 会让历史
        // Token 全变「未知」，统计从彩色变灰。
        let known: (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*) FILTER (WHERE input_known=1 AND total_known=1),
                        COUNT(*) FILTER (WHERE connection_id IS NULL AND provider_key IS NULL)
                 FROM requests",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(known, (2, 2), "旧行默认按已知处理；无归因证据的列留 NULL");

        let conn_row: (String, i64) = conn
            .query_row("SELECT name, serve_enabled FROM connections WHERE id='c-old'", [],
                |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(conn_row, ("旧账号".to_string(), 0), "旧连接保留，serve_enabled 默认关闭");
        let session_row: (String, String) = conn
            .query_row("SELECT title, activity_state FROM sessions WHERE session_id='s1'", [],
                |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(session_row, ("旧标题".to_string(), "done".to_string()));
        let log_uid: Option<i64> = conn
            .query_row("SELECT request_uid FROM gateway_route_log WHERE ts=1000", [], |r| r.get(0))
            .unwrap();
        assert_eq!(log_uid, None, "旧流水没有身份证据，留 NULL 而不是编一个");
        let scan_row: (i64, i64, i64) = conn
            .query_row(
                "SELECT offset, mtime, parser_initialized FROM scan_state WHERE path='/tmp/old.jsonl'",
                [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        assert_eq!(scan_row, (123, 456, 0), "旧位点保留，解析上下文标记为未初始化");

        // 新列真的能用：写一条带 effort 的完整记录
        insert_records(&mut conn, &[RequestRecord {
            platform: "claude".into(),
            source: "local-session".into(),
            dedup_key: "m3".into(),
            session_id: Some("s1".into()),
            ts: 3000,
            model: Some("claude-sonnet-4".into()),
            input_tokens: Some(7), output_tokens: Some(3),
            cache_read_tokens: Some(0), cache_write_tokens: Some(0),
            total_tokens: Some(10), effort: Some("high".into()),
        }])
        .unwrap();
        let fresh: (i64, Option<String>) = conn
            .query_row("SELECT total_tokens, effort FROM requests WHERE dedup_key='m3'", [],
                |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(fresh, (10, Some("high".into())), "新列可写可读");

        // 重复迁移幂等：再跑两次，行数与旧值都不变
        let before: i64 = conn.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0)).unwrap();
        init(&conn).unwrap();
        init(&conn).unwrap();
        let after: i64 = conn.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0)).unwrap();
        assert_eq!(before, after, "重复迁移不能增删行");
        let still_m1: i64 = conn
            .query_row("SELECT total_tokens FROM requests WHERE dedup_key='m1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(still_m1, 120, "重复迁移不能改旧值");
    }

    /// P1「旧数据库升级」第二部分：走**真实文件路径**（`db::open` + WAL + 外键 + 调优），
    /// 而不是内存连接。
    ///
    /// 上一条测的是 `init` 的迁移逻辑本身；这条测的是「用户真的打开一个旧版
    /// `usage.db` 文件」时会不会出事。差异在：
    /// - WAL 模式下 ALTER TABLE 的表现与内存库不同；
    /// - `foreign_keys=ON` 时旧库里若有悬空引用，迁移/新写入会直接报错；
    /// - `open_reader`（只读连接池）随后能不能读到迁移后的新列。
    #[test]
    fn an_old_database_file_opens_reads_and_reopens_cleanly() {
        let root = std::env::temp_dir().join(format!(
            "jolu-old-db-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("usage.db");

        // 用一个裸连接造出旧版文件（不走 open，避免提前迁移）
        {
            let raw = Connection::open(&path).unwrap();
            raw.execute_batch(
                "CREATE TABLE requests (
                    id INTEGER PRIMARY KEY, platform TEXT NOT NULL, source TEXT NOT NULL,
                    dedup_key TEXT NOT NULL, session_id TEXT, ts INTEGER NOT NULL, model TEXT,
                    input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
                    cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                    total_tokens INTEGER NOT NULL DEFAULT 0,
                    UNIQUE(source, dedup_key)
                 );
                 INSERT INTO requests(platform,source,dedup_key,ts,total_tokens)
                   VALUES('zcode','native:zcode:main','old-1',1000000,999999);
                 CREATE TABLE connections (
                    id TEXT PRIMARY KEY, platform TEXT NOT NULL, kind TEXT NOT NULL, name TEXT NOT NULL,
                    secret TEXT, masked TEXT, status TEXT, created_ms INTEGER
                 );",
            )
            .unwrap();
            // 确保数据落到磁盘（不靠 checkpoint）
            raw.execute_batch("PRAGMA journal_mode=DELETE;").ok();
        }

        // 用户真实的打开路径
        {
            let conn = open(&path).unwrap();
            let row: (i64, i64) = conn
                .query_row(
                    "SELECT total_tokens, total_known FROM requests WHERE dedup_key='old-1'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(row, (999999, 1), "旧 Token 保留，且默认按已知处理");
            // 新列可用
            conn.execute(
                "INSERT INTO requests(platform,source,dedup_key,ts,provider_key,connection_id)
                 VALUES('zcode','native:zcode:main','new-1',2000000,'zai','c-1')",
                [],
            )
            .unwrap();
            // 只读连接池能读到
            let reader = open_reader(&path).unwrap();
            let count: i64 = reader
                .query_row("SELECT COUNT(*) FROM requests WHERE provider_key='zai'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 1, "只读连接能看到迁移后的新列");
        }

        // 关掉再开一次：迁移在已有 WAL 的库上也要幂等
        {
            let conn = open(&path).unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 2, "重开不能增删记录");
            let old: i64 = conn
                .query_row("SELECT total_tokens FROM requests WHERE dedup_key='old-1'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(old, 999999, "重开不能改旧值");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn request_uid_column_migrates_and_old_rows_stay_unknown() {
        // 模拟旧库：gateway_route_log 没有 request_uid 列，且已有一条历史记录
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE gateway_route_log (
                id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, agent TEXT,
                requested_model TEXT NOT NULL, result TEXT NOT NULL
            );
            INSERT INTO gateway_route_log (ts, requested_model, result) VALUES (100, 'group/x', 'ok');",
        ).unwrap();
        init(&conn).unwrap();

        // 1) 列补上了
        let cols: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('gateway_route_log') WHERE name='request_uid'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cols, 1, "旧库升级后必须有 request_uid 列");

        // 2) 旧行的身份保持未知，不编一个值
        let old: Option<i64> = conn
            .query_row("SELECT request_uid FROM gateway_route_log ORDER BY id LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(old, None, "历史行没有身份证据，必须留 NULL");

        // 3) 新行能写入
        conn.execute(
            "INSERT INTO gateway_route_log (ts, requested_model, result, request_uid) VALUES (200, 'group/x', 'ok', 7)",
            [],
        )
        .unwrap();
        let fresh: Option<i64> = conn
            .query_row("SELECT request_uid FROM gateway_route_log WHERE ts=200", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fresh, Some(7));

        // 4) 新库直接 init 也有该列（幂等 ALTER 不会因重复执行失败）
        let again = Connection::open_in_memory().unwrap();
        init(&again).unwrap();
        init(&again).unwrap();
        let cols: i64 = again
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('gateway_route_log') WHERE name='request_uid'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cols, 1, "重复 init 不应破坏结构");
    }

    #[test]
    fn long_running_commands_across_cli_and_typed_platforms_do_not_expire_at_90s() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        let now = Local::now().timestamp_millis();
        let five_min_ago = now - 5 * 60 * 1_000;

        // 1) CLI 生命周期平台（antigravity / kimi / opencode / pi / dsh）：
        //    即使命令已静默执行 5 分钟（远超 90 秒）且尚无 requests 行，仍稳居 running；一旦写 done 立即结束。
        for plat in ["antigravity", "kimi", "opencode", "pi", "dsh"] {
            let sid = format!("{plat}:long-cmd");
            set_session_activity(&conn, plat, &sid, "running", five_min_ago).unwrap();
            set_session_turn_start(&conn, plat, &sid, five_min_ago).unwrap();
            let live = live_usage(&conn, plat, None).unwrap();
            assert_eq!(live.active_window_seconds, 0);
            assert_eq!(live.active_sessions.len(), 1, "{plat} 5 分钟长命令期间不得误判结束");
            assert_eq!(live.active_sessions[0].state, "running");
            assert_eq!(live.running_tokens, Some(0));

            set_session_activity(&conn, plat, &sid, "done", now).unwrap();
            let finished = live_usage(&conn, plat, None).unwrap();
            assert!(finished.active_sessions.is_empty(), "{plat} 真正完成后立即退出活跃列表");
        }

        // 2) 混合状态平台（zcode / qwen / gemini）：
        //    首轮提问尚未落 requests 时即刻可见 running；命令执行 5 分钟期间同样不因 90 秒窗口断流。
        for plat in ["zcode", "qwen", "gemini"] {
            let sid = format!("{plat}:fresh-cmd");
            set_session_activity(&conn, plat, &sid, "running", five_min_ago).unwrap();
            set_session_turn_start(&conn, plat, &sid, five_min_ago).unwrap();
            let live = live_usage(&conn, plat, None).unwrap();
            assert_eq!(live.active_sessions.len(), 1, "{plat} 首轮无请求或长命令执行期间应保持 running");
            assert_eq!(live.active_sessions[0].state, "running");
        }
    }
