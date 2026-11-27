    use super::*;

    fn encode_varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
    }

    fn push_varint_field(field: u32, val: u64, out: &mut Vec<u8>) {
        encode_varint(((field as u64) << 3) | 0, out);
        encode_varint(val, out);
    }

    fn push_bytes_field(field: u32, data: &[u8], out: &mut Vec<u8>) {
        encode_varint(((field as u64) << 3) | 2, out);
        encode_varint(data.len() as u64, out);
        out.extend_from_slice(data);
    }

    fn encode_timestamp(secs: u64, nanos: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        push_varint_field(1, secs, &mut buf);
        if nanos > 0 {
            push_varint_field(2, nanos, &mut buf);
        }
        buf
    }

    fn encode_step_metadata(
        created: (u64, u64),
        first_token: (u64, u64),
        completed: (u64, u64),
        model_enum: u64,
        input: u64,
        output: u64,
        cache_read: u64,
        response_id: &str,
    ) -> Vec<u8> {
        let mut usage = Vec::new();
        push_varint_field(1, model_enum, &mut usage);
        if input > 0 {
            push_varint_field(2, input, &mut usage);
        }
        if output > 0 {
            push_varint_field(3, output, &mut usage);
        }
        if cache_read > 0 {
            push_varint_field(5, cache_read, &mut usage);
        }
        push_bytes_field(7, response_id.as_bytes(), &mut usage);

        let mut meta = Vec::new();
        push_bytes_field(1, &encode_timestamp(created.0, created.1), &mut meta);
        push_bytes_field(6, &encode_timestamp(first_token.0, first_token.1), &mut meta);
        push_bytes_field(7, &encode_timestamp(completed.0, completed.1), &mut meta);
        push_bytes_field(9, &usage, &mut meta);
        meta
    }

    #[test]
    fn parses_protobuf_metadata_joins_transcript_and_syncs_summaries() {
        let dir = std::env::temp_dir().join(format!(
            "jolu-antigravity-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let root_dir = dir.join(".cc-switch").join("agy").join("prof-12345678").join("antigravity-cli");
        let conv_dir = root_dir.join("conversations");
        let log_dir = root_dir.join("brain").join("conv-1").join(".system_generated").join("logs");
        std::fs::create_dir_all(&conv_dir).unwrap();
        std::fs::create_dir_all(&log_dir).unwrap();

        // 构造模拟 conversations/conv-1.db（含首轮冷启动 cache_read=0 与二轮缓存命中 181_000）
        let conv_db = Connection::open(conv_dir.join("conv-1.db")).unwrap();
        conv_db
            .execute_batch("CREATE TABLE steps (idx INTEGER PRIMARY KEY, step_type INTEGER, status INTEGER, metadata BLOB);")
            .unwrap();
        let meta0 = encode_step_metadata(
            (1_795_620_000, 0),
            (1_795_620_005, 500_000_000),
            (1_795_620_010, 0),
            1468,
            20_000,
            500,
            0,
            "resp-cold",
        );
        let meta1 = encode_step_metadata(
            (1_795_625_546, 0),
            (1_795_625_556, 400_000_000),
            (1_795_625_562, 900_000_000),
            1468,
            2_883,
            795,
            181_000,
            "resp-warm",
        );
        conv_db
            .execute("INSERT INTO steps VALUES (0, 15, 3, ?1), (1, 15, 3, ?2), (2, 10, 3, ?2)", params![meta0, meta1])
            .unwrap();
        drop(conv_db);

        // 构造配套 transcript.jsonl（只给 step 1 写模型名，验证 step 0 通过同 enum 1468 继承）
        std::fs::write(
            log_dir.join("transcript.jsonl"),
            "{\"type\":\"PLANNER_RESPONSE\",\"step_index\":1,\"model\":\"gemini-4-neon-xhigh\",\"status\":\"DONE\"}\n",
        )
        .unwrap();

        // 构造 conversation_summaries.db
        let sum_db = Connection::open(root_dir.join("conversation_summaries.db")).unwrap();
        sum_db
            .execute_batch(
                "CREATE TABLE conversation_summaries (conversation_id TEXT PRIMARY KEY, title TEXT, workspace_uris TEXT, last_modified_time TEXT);",
            )
            .unwrap();
        sum_db
            .execute(
                "INSERT INTO conversation_summaries VALUES ('conv-1', '测试会话标题', '[\"file:///D:/develop/Jolu\"]', '2026-11-26T00:52:26+08:00')",
                [],
            )
            .unwrap();
        drop(sum_db);

        let roots = discover_roots(&dir);
        let ccs = roots.iter().find(|r| r.source_id == "native:antigravity:ccs-prof-12345678").expect("应自动发现 CCS 托管目录");

        let mut dest = crate::db::open(Path::new(":memory:")).unwrap();
        let (n, errs) = scan_root(&mut dest, ccs).unwrap();
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(n, 2, "两条 PLANNER_RESPONSE 应全部入库，非 step_type=15 被忽略");

        // 二次扫描幂等（戳记命中直接跳过）
        let (n2, _) = scan_root(&mut dest, ccs).unwrap();
        assert_eq!(n2, 0);

        let filter = crate::usage_stats::StatsFilter {
            start: 1_795_600_000_000,
            end: 1_795_700_000_000,
            platform: Some("antigravity".into()),
            ..Default::default()
        };
        let ov = crate::usage_stats::overview(&dest, &filter).unwrap();
        assert_eq!(ov.requests, 2);
        assert_eq!(ov.tokens.fresh_input, Some(22_883));
        assert_eq!(ov.tokens.output, Some(1_295));
        assert_eq!(ov.tokens.cache_read, Some(181_000));
        assert_eq!(ov.tokens.total, Some(205_178));
        let expected_hit = 181_000.0 / (22_883.0 + 181_000.0);
        assert!((ov.cache_hit_rate.unwrap() - expected_hit).abs() < 1e-6);

        let page = crate::usage_stats::calls(&dest, &filter, 1).unwrap();
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].call_id, "resp-warm");
        assert_eq!(page.rows[0].model.as_deref(), Some("gemini-4-neon-xhigh"));
        assert_eq!(page.rows[0].first_token_ms, Some(10_400));
        assert_eq!(page.rows[0].duration_ms, Some(16_900));
        assert_eq!(page.rows[1].model.as_deref(), Some("gemini-4-neon-xhigh"), "同 enum 应关联模型名");
        assert_eq!(page.rows[1].first_token_ms, Some(5_500));

        // 核对 conversation_summaries.db 同步的标题与 cwd
        let (title, cwd): (Option<String>, Option<String>) = dest
            .query_row(
                "SELECT title, cwd FROM sessions WHERE platform = 'antigravity'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("测试会话标题"));
        assert!(cwd.as_deref().is_some_and(|p| p.ends_with("Jolu")));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn transcript_fallback_works_when_db_absent() {
        let dir = std::env::temp_dir().join(format!(
            "jolu-antigravity-fallback-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let root_dir = dir.join(".gemini").join("antigravity");
        let log_dir = root_dir.join("brain").join("conv-only-tr").join(".system_generated").join("logs");
        std::fs::create_dir_all(&log_dir).unwrap();
        std::fs::write(
            log_dir.join("transcript.jsonl"),
            "{\"type\":\"PLANNER_RESPONSE\",\"step_index\":4,\"model\":\"gemini-4-neon-xhigh\",\"status\":\"DONE\",\"created_at\":\"2026-11-26T00:52:26+08:00\",\"input_tokens\":1000,\"output_tokens\":200,\"cache_read_tokens\":8000}\n",
        )
        .unwrap();

        let roots = discover_roots(&dir);
        let def = roots.iter().find(|r| r.source_id == "native:antigravity:default").unwrap();
        let mut dest = crate::db::open(Path::new(":memory:")).unwrap();
        let (n, errs) = scan_root(&mut dest, def).unwrap();
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(n, 1);

        let total: i64 = dest.query_row("SELECT total_tokens FROM requests WHERE platform = 'antigravity'", [], |r| r.get(0)).unwrap();
        assert_eq!(total, 9200);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn title_backfill_space_timestamp_subagent_and_transcript_lookup() {
        let dir = std::env::temp_dir().join(format!(
            "jolu-antigravity-meta-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let root_dir = dir.join(".cc-switch").join("agy").join("prof-abcd").join("antigravity-cli");
        let log_dir = root_dir.join("brain").join("conv-main").join(".system_generated").join("logs");
        std::fs::create_dir_all(&log_dir).unwrap();
        let tr = log_dir.join("transcript.jsonl");
        std::fs::write(
            &tr,
            concat!(
                "{\"type\":\"USER_INPUT\",\"step_index\":0,\"created_at\":\"2026-11-26T00:50:00+08:00\",\"content\":\"<USER_REQUEST>\\n修复 Antigravity 会话读取\\n第二行说明\\n</USER_REQUEST>\"}\n",
                "{\"type\":\"PLANNER_RESPONSE\",\"step_index\":1,\"model\":\"gemini-4-neon-xhigh\",\"status\":\"DONE\",\"created_at\":\"2026-11-26T00:52:26+08:00\",\"input_tokens\":500,\"output_tokens\":100,\"cache_read_tokens\":1000}\n",
            ),
        )
        .unwrap();

        let sum_db = Connection::open(root_dir.join("conversation_summaries.db")).unwrap();
        sum_db
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    preview TEXT NOT NULL DEFAULT '',
                    workspace_uris TEXT NOT NULL DEFAULT '',
                    last_modified_time TEXT NOT NULL DEFAULT '',
                    status TEXT NOT NULL DEFAULT '',
                    parent_conversation_id TEXT NOT NULL DEFAULT ''
                );
                INSERT INTO conversation_summaries VALUES
                  ('conv-main', '', '', '[\"file:///D:/develop/Jolu\"]', '2026-11-26 12:50:21.455+00:00', 'CASCADE_RUN_STATUS_RUNNING', ''),
                  ('conv-sub', '子代理任务', '', '[\"file:///D:/develop/Jolu\"]', '2026-11-26 12:50:21+00:00', 'CASCADE_RUN_STATUS_IDLE', 'conv-main');",
            )
            .unwrap();
        drop(sum_db);

        let roots = discover_roots(&dir);
        let ccs = roots.iter().find(|r| r.source_id == "native:antigravity:ccs-prof-abcd").unwrap();
        let found = find_transcript_in(&roots, "native:antigravity:ccs-prof-abcd:conv-main");
        assert_eq!(found.as_deref(), Some(tr.as_path()));
        // 路径穿越拒绝
        assert!(find_transcript_in(&roots, "native:antigravity:ccs-prof-abcd:../evil").is_none());

        let mut dest = crate::db::open(Path::new(":memory:")).unwrap();
        let (n, errs) = scan_root(&mut dest, ccs).unwrap();
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(n, 1);

        let (title, state, sub): (Option<String>, Option<String>, i64) = dest
            .query_row(
                "SELECT title, activity_state, is_subagent FROM sessions WHERE platform='antigravity' AND session_id='native:antigravity:ccs-prof-abcd:conv-main'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("修复 Antigravity 会话读取"));
        assert_eq!(state.as_deref(), Some("running"));
        assert_eq!(sub, 0);

        let child_sub: i64 = dest
            .query_row(
                "SELECT is_subagent FROM sessions WHERE platform='antigravity' AND session_id='native:antigravity:ccs-prof-abcd:conv-sub'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(child_sub, 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// 真机只读对账测试（手动 `--ignored --nocapture` 执行，绝不修改真实数据库）：
    /// 1. 在内存库扫描本机 Antigravity 根目录并打印今日指标与调用明细，核对 CCS 口径；
    /// 2. 将真实 `usage.db` 复制到临时目录，验证增量迁移、2026-11-21 趋势非零修复、
    ///    Kimi / Pi / DSH 采集入库及 `session_threads::list` 无重复且隐藏 DSH subagent。
    #[test]
    #[ignore]
    fn real_machine_readonly_reconciliation() {
        use chrono::{Local, TimeZone};
        use std::collections::HashSet;

        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .expect("home_dir");
        let roots = discover_roots(&home);
        println!("[AGY] discovered roots ({}):", roots.len());
        for r in &roots {
            println!("  - {} => {}", r.source_id, r.path.display());
        }
        assert!(!roots.is_empty(), "应发现至少一个 Antigravity 根目录");

        let mut mem = crate::db::open(Path::new(":memory:")).unwrap();
        let mut total_written = 0usize;
        for r in &roots {
            let (n, errs) = scan_root(&mut mem, r).unwrap();
            println!("  scanned {}: written={}, errs={:?}", r.source_id, n, errs);
            total_written += n;
        }
        let today = Local::now().date_naive();
        let start = Local
            .from_local_datetime(&today.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .unwrap()
            .timestamp_millis();
        let end = Local::now().timestamp_millis() + 60_000;
        let agy_filter = crate::usage_stats::StatsFilter {
            start,
            end,
            platform: Some("antigravity".into()),
            ..Default::default()
        };
        let ov = crate::usage_stats::overview(&mem, &agy_filter).unwrap();
        println!(
            "[AGY TODAY] total_written={total_written}, requests={}, tokens={:?}, fresh={:?}, output={:?}, cache_read={:?}, hit_rate={:?}, cost=${:.4}, bucket={}, points={}",
            ov.requests,
            ov.tokens.total,
            ov.tokens.fresh_input,
            ov.tokens.output,
            ov.tokens.cache_read,
            ov.cache_hit_rate,
            ov.cost.amount,
            ov.trend.bucket,
            ov.trend.points.len()
        );
        let page = crate::usage_stats::calls(&mem, &agy_filter, 1).unwrap();
        println!("[AGY CALLS] total_count={}, page_size={}, showing first 5:", page.total_count, page.page_size);
        for row in page.rows.iter().take(5) {
            println!(
                "  ts={} model={:?} fresh={:?} out={:?} cache_read={:?} ttft={:?}ms dur={:?}ms cost={:?}",
                row.ts, row.model, row.fresh_input, row.output, row.cache_read, row.first_token_ms, row.duration_ms, row.cost
            );
        }
        assert!(ov.requests >= 22, "今日 Antigravity 请求应 >= 22，实际 {}", ov.requests);
        assert_eq!(ov.trend.bucket, "1h");
        assert!(ov.cache_hit_rate.is_some_and(|r| r > 0.7), "缓存命中率应 > 70%，实际 {:?}", ov.cache_hit_rate);
        assert!(page.rows.iter().any(|r| r.first_token_ms.is_some_and(|ms| ms > 0)));

        // 2. 复制真实 usage.db 到临时目录，验证迁移、2026-11-21 趋势与 P2 来源采集
        let real_db = std::env::var_os("APPDATA")
            .map(|d| PathBuf::from(d).join("dev.ningz.jolu").join("usage.db"));
        if let Some(src_db) = real_db.filter(|p| p.is_file()) {
            let tmp = std::env::temp_dir().join(format!(
                "jolu-reconcile-{}-{}.db",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
            ));
            std::fs::copy(&src_db, &tmp).unwrap();
            let mut conn = crate::db::open(&tmp).unwrap();

            // 核对 2026-11-21 趋势柱（此前因 1 条未知行被整柱清零）
            let d1121 = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
            let s21 = Local.from_local_datetime(&d1121.and_hms_opt(0, 0, 0).unwrap()).single().unwrap().timestamp_millis();
            let e21 = Local.from_local_datetime(&d1121.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap()).single().unwrap().timestamp_millis();
            let ov21 = crate::usage_stats::overview(&conn, &crate::usage_stats::StatsFilter { start: s21, end: e21, ..Default::default() }).unwrap();
            let trend_sum: i64 = ov21.trend.points.iter().filter_map(|p| p.total).sum();
            let unknown_sum: i64 = ov21.trend.points.iter().map(|p| p.unknown_requests).sum();
            println!(
                "[2026-11-21] requests={}, known_total={:?}, trend_sum={}, unknown_in_buckets={}, hit_rate={:?}",
                ov21.requests, ov21.token_coverage.known_total, trend_sum, unknown_sum, ov21.cache_hit_rate
            );
            if ov21.token_coverage.known_total.unwrap_or(0) > 0 {
                assert!(trend_sum > 0, "2026-11-21 趋势合计不应再被未知行清零");
            }

            let agy_before: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='antigravity'", [], |r| r.get(0)).unwrap();
            let kimi_before: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='kimi'", [], |r| r.get(0)).unwrap();
            let pi_before: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='pi'", [], |r| r.get(0)).unwrap();
            let dsh_before: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='dsh'", [], |r| r.get(0)).unwrap();
            println!("[BEFORE INGEST ON COPY] agy={agy_before}, kimi={kimi_before}, pi={pi_before}, dsh={dsh_before}");

            // 扫描 P1 + P2 来源到临时库副本（注意：scan_usage 接收的是 home 目录，与 native_sources::scan_sources 一致）
            let mut agy_n = 0usize;
            for r in &roots {
                agy_n += scan_root(&mut conn, r).unwrap().0;
            }
            let (kimi_n, kimi_e) = crate::kimi_sessions::scan_usage(&mut conn, &home).unwrap();
            let (pi_n, pi_e) = crate::pi_sessions::scan_usage(&mut conn, &home).unwrap();
            let (dsh_n, dsh_e) = crate::dsh_sessions::scan_usage(&mut conn, &home).unwrap();
            println!(
                "[P1/P2 INGEST ON COPY] agy={agy_n}, kimi={kimi_n} (errs={}), pi={pi_n} (errs={}), dsh={dsh_n} (errs={})",
                kimi_e.len(),
                pi_e.len(),
                dsh_e.len()
            );

            for plat in ["antigravity", "kimi", "pi", "dsh"] {
                let ov_all = crate::usage_stats::overview(
                    &conn,
                    &crate::usage_stats::StatsFilter {
                        start: 0,
                        end,
                        platform: Some(plat.into()),
                        ..Default::default()
                    },
                )
                .unwrap();
                println!(
                    "  platform={plat}: requests={}, known_total={:?}, strict_hit={:?}, cov_hit={:?} ({}/{}), cost=${:.4}",
                    ov_all.requests,
                    ov_all.token_coverage.known_total,
                    ov_all.cache_hit_rate,
                    ov_all.token_coverage.cache_hit_rate,
                    ov_all.token_coverage.cache_requests,
                    ov_all.requests,
                    ov_all.cost.amount
                );
            }

            // 验证 session_threads::list 无重复且隐藏 DSH subagent
            let list = crate::session_threads::list(&conn, &crate::session_threads::SessionFilter::default()).unwrap();
            let mut seen = HashSet::new();
            for item in &list.sessions {
                let key = (item.platform.clone(), item.session_id.clone());
                assert!(seen.insert(key.clone()), "session_threads 发现重复会话: {:?}", key);
            }
            let subagent_ids: HashSet<String> = conn
                .prepare("SELECT session_id FROM sessions WHERE platform = 'dsh' AND is_subagent = 1")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .flatten()
                .collect();
            let leaked = list
                .sessions
                .iter()
                .filter(|s| s.platform == "dsh" && subagent_ids.contains(&s.session_id))
                .count();
            println!(
                "[SESSIONS] total_listed={}, unique={}, dsh_subagents_in_db={}, leaked_subagents={}",
                list.sessions.len(),
                seen.len(),
                subagent_ids.len(),
                leaked
            );
            assert_eq!(leaked, 0, "DSH subagent 会话不得出现在 session_threads 列表");

            drop(conn);
            let _ = std::fs::remove_file(&tmp);
            let _ = std::fs::remove_file(format!("{}-wal", tmp.display()));
            let _ = std::fs::remove_file(format!("{}-shm", tmp.display()));
        }
    }
