    use super::*;
    use std::io::Write;

    fn temp_jsonl(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "jolu-{name}-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    #[test]
    fn incremental_reader_keeps_incomplete_tail_and_counts_crlf_bytes_exactly() {
        let path = temp_jsonl("partial-lines");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"first\r\nsecond\npartial").unwrap();
        drop(file);
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE scan_state(path TEXT PRIMARY KEY, offset INTEGER NOT NULL, mtime INTEGER NOT NULL);",
        ).unwrap();

        let (lines, offset, _) = read_new_lines(&conn, &path).unwrap();
        assert_eq!(lines, vec!["first", "second"]);
        assert_eq!(offset, b"first\r\nsecond\n".len() as i64);

        db::set_offset(&conn, &path.to_string_lossy(), offset, 0).unwrap();
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"\n").unwrap();
        drop(file);
        let (lines, next_offset, _) = read_new_lines(&conn, &path).unwrap();
        assert_eq!(lines, vec!["partial"]);
        assert_eq!(next_offset, std::fs::metadata(&path).unwrap().len() as i64);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn equal_length_rewrite_is_rescanned_when_mtime_changes() {
        let path = temp_jsonl("equal-rewrite");
        std::fs::write(&path, b"first\n").unwrap();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE scan_state(path TEXT PRIMARY KEY, offset INTEGER NOT NULL, mtime INTEGER NOT NULL);",
        ).unwrap();
        let (_, offset, mtime) = read_new_lines(&conn, &path).unwrap();
        db::set_offset(&conn, &path.to_string_lossy(), offset, mtime - 1).unwrap();
        std::fs::write(&path, b"other\n").unwrap();

        let (lines, next_offset, _) = read_new_lines(&conn, &path).unwrap();
        assert_eq!(lines, vec!["other"]);
        assert_eq!(next_offset, offset);
        let _ = std::fs::remove_file(path);
    }

    /// P1「扫描期间源端被并发改写」形态一：**边扫边追加**。
    ///
    /// ## 真实场景
    ///
    /// Claude Code / Codex 正在写日志，Jolu 同时在扫。`scan_file` 先 `metadata()` 拿到
    /// `size`，再 `file.take(size - offset)` 只读到那个长度——扫描期间新增的字节**不在**
    /// 本次读取范围内。正确行为是：本次把已知部分读完并推进位点到 `offset + consumed`，
    /// 新增的那部分留给下一次扫描，且下一次不会重复写已采过的行。
    ///
    /// 已有测试覆盖的缺口：`equal_length_rewrite_is_rescanned_when_mtime_changes` 只测
    /// `read_new_lines` 的纯判定，没走 `scan_file` 的事务路径（记录写入 + 位点提交 +
    /// `insert_records_into` 的合并语义）。这条补上完整链路。
    #[test]
    fn a_file_growing_during_the_scan_is_picked_up_next_round_without_duplicates() {
        let path = temp_jsonl("concurrent-append");
        let mut conn = db::open(Path::new(":memory:")).unwrap();
        let line = |id: &str, ts: &str| {
            format!(
                "{{\"type\":\"assistant\",\"timestamp\":\"{ts}\",\"sessionId\":\"s\",\
                  \"message\":{{\"id\":\"{id}\",\"usage\":{{\"input_tokens\":10,\"output_tokens\":5,\
                  \"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}}}}\n"
            )
        };
        // 初始：两行
        std::fs::write(&path, format!(
            "{}{}",
            line("m1", "2026-09-16T00:00:00Z"),
            line("m2", "2026-09-16T00:00:01Z"),
        ))
        .unwrap();
        assert_eq!(scan_file(&mut conn, &path, "claude").unwrap(), 2);

        // 模拟「扫描过程中源端还在写」：scan_file 拿到的 size 是此刻的，
        // 读完这两行后源端又追加了一行。下一轮扫描必须补上它。
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            write!(file, "{}", line("m3", "2026-09-16T00:00:02Z")).unwrap();
            file.flush().unwrap();
        }
        let count = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM requests WHERE platform='claude'", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(count(&conn), 2, "上一轮只采到当时看见的两行");

        // 下一轮：补上第三行，不重复前两行
        assert_eq!(scan_file(&mut conn, &path, "claude").unwrap(), 1, "只新增 1 行");
        assert_eq!(count(&conn), 3, "三行都在，一条不多");
        let dup: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM (SELECT dedup_key FROM requests GROUP BY dedup_key HAVING COUNT(*)>1)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dup, 0, "不能有重复行");

        // 第三轮：无新增
        assert_eq!(scan_file(&mut conn, &path, "claude").unwrap(), 0);
        let _ = std::fs::remove_file(&path);
    }

    /// P-4「缺 ID」固定样本：源端记录没有稳定 ID（`message.id` 缺失或不是字符串）时，
    /// 不能编一个键入库——编出来的键每次扫描都可能不同，重扫就会翻倍；也不能让它挡住同文件的好行。
    ///
    /// 判据：只有带 ID 的两行入库；重扫一行都不新增；位点照常推进（坏行不卡住后面的行）。
    #[test]
    fn records_without_a_stable_id_are_not_given_a_made_up_key() {
        let path = temp_jsonl("missing-id");
        let mut conn = db::open(Path::new(":memory:")).unwrap();
        let with_id = |id: &str, ts: &str| {
            format!(
                "{{\"type\":\"assistant\",\"timestamp\":\"{ts}\",\"sessionId\":\"s\",\
                  \"message\":{{\"id\":\"{id}\",\"usage\":{{\"input_tokens\":10,\"output_tokens\":5,\
                  \"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}}}}\n"
            )
        };
        // 缺 message.id
        let no_id = "{\"type\":\"assistant\",\"timestamp\":\"2026-09-16T00:00:01Z\",\"sessionId\":\"s\",\
            \"message\":{\"usage\":{\"input_tokens\":99,\"output_tokens\":1}}}\n";
        // message.id 不是字符串
        let numeric_id = "{\"type\":\"assistant\",\"timestamp\":\"2026-09-16T00:00:02Z\",\"sessionId\":\"s\",\
            \"message\":{\"id\":42,\"usage\":{\"input_tokens\":99,\"output_tokens\":1}}}\n";
        std::fs::write(&path, format!(
            "{}{no_id}{numeric_id}{}",
            with_id("m1", "2026-09-16T00:00:00Z"),
            with_id("m2", "2026-09-16T00:00:03Z"),
        ))
        .unwrap();
        assert_eq!(scan_file(&mut conn, &path, "claude").unwrap(), 2, "只有带 ID 的两行入库");
        let keys: Vec<String> = conn
            .prepare("SELECT dedup_key FROM requests WHERE platform='claude' ORDER BY dedup_key")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(keys, ["m1", "m2"], "缺 ID 的行不编键");
        let total: i64 = conn
            .query_row("SELECT SUM(input_tokens) FROM requests WHERE platform='claude'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 20, "缺 ID 那两行的 99 不能混进来");
        assert_eq!(scan_file(&mut conn, &path, "claude").unwrap(), 0, "重扫不新增");
        let _ = std::fs::remove_file(&path);
    }

    /// P-2「源端删除历史」固定样本：源端把旧会话文件删掉之后，Jolu 已采的历史保留，
    /// 新文件照常采集，统计不倒退。
    ///
    /// 这是对账里「只在 Jolu」那部分记录（Claude Code 692 条、ZCode 5,872 条）的可重复版本：
    /// 真实对账只能观察一次，这条每次都能跑。
    #[test]
    fn history_survives_the_source_deleting_its_files() {
        let old = temp_jsonl("deleted-history-old");
        let new = temp_jsonl("deleted-history-new");
        let mut conn = db::open(Path::new(":memory:")).unwrap();
        let line = |id: &str, ts: &str| {
            format!(
                "{{\"type\":\"assistant\",\"timestamp\":\"{ts}\",\"sessionId\":\"s\",\
                  \"message\":{{\"id\":\"{id}\",\"usage\":{{\"input_tokens\":10,\"output_tokens\":5,\
                  \"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}}}}\n"
            )
        };
        std::fs::write(&old, format!("{}{}", line("old-1", "2026-09-01T00:00:00Z"), line("old-2", "2026-09-01T00:00:01Z"))).unwrap();
        assert_eq!(scan_file(&mut conn, &old, "claude").unwrap(), 2);

        // 源端清理：旧文件被删掉，随后出现一个新文件
        std::fs::remove_file(&old).unwrap();
        std::fs::write(&new, line("new-1", "2026-09-30T00:00:00Z")).unwrap();
        assert!(scan_file(&mut conn, &old, "claude").is_err(), "文件已删，读不到是正常的错误，不是清空历史");
        assert_eq!(scan_file(&mut conn, &new, "claude").unwrap(), 1);

        let keys: Vec<String> = conn
            .prepare("SELECT dedup_key FROM requests WHERE platform='claude' ORDER BY ts")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(keys, ["old-1", "old-2", "new-1"], "源端删掉的历史仍在 Jolu，且新记录照常补上");
        let _ = std::fs::remove_file(&new);
    }

    #[test]
    fn claude_and_codex_logs_estimate_duration_without_overwriting_proxy_timing() {
        let claude_path = temp_jsonl("duration-claude");
        let codex_path = temp_jsonl("duration-codex");
        let mut conn = db::open(Path::new(":memory:")).unwrap();

        let claude_lines = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":\"2026-11-25T10:00:00.000Z\",\"sessionId\":\"s1\",\"message\":{\"content\":\"hi\"}}\n",
            "{\"type\":\"attachment\",\"uuid\":\"a1\",\"parentUuid\":\"u1\",\"timestamp\":\"2026-11-25T10:00:01.000Z\",\"sessionId\":\"s1\"}\n",
            "{\"type\":\"assistant\",\"uuid\":\"m1a\",\"parentUuid\":\"a1\",\"apiBlockIndex\":0,\"timestamp\":\"2026-11-25T10:00:02.000Z\",\"sessionId\":\"s1\",\"message\":{\"id\":\"msg_dur_1\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"m1b\",\"parentUuid\":\"m1a\",\"apiBlockIndex\":1,\"timestamp\":\"2026-11-25T10:00:03.500Z\",\"sessionId\":\"s1\",\"message\":{\"id\":\"msg_dur_1\",\"stop_reason\":\"end_turn\",\"usage\":{\"input_tokens\":10,\"output_tokens\":80,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n"
        );
        std::fs::write(&claude_path, claude_lines).unwrap();
        assert_eq!(scan_file(&mut conn, &claude_path, "claude").unwrap(), 2);
        let dur: Option<i64> = conn
            .query_row("SELECT duration_ms FROM requests WHERE dedup_key = 'msg_dur_1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(dur, Some(3500), "应跳过 attachment 沿父链算 u1 -> m1b = 3500ms");

        // 若已有代理实测耗时，重扫绝不覆盖
        db::apply_proxy_timing(&conn, "msg_dur_1", 420, 3120, 200, None).unwrap();
        conn.execute("UPDATE scan_state SET parser_initialized = 0", []).unwrap();
        let _ = scan_file(&mut conn, &claude_path, "claude").unwrap();
        let kept: (Option<i64>, Option<i64>) = conn
            .query_row("SELECT first_token_ms, duration_ms FROM requests WHERE dedup_key = 'msg_dur_1'", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(kept, (Some(420), Some(3120)), "绝不能覆盖已有的代理实测耗时");

        let codex_lines = concat!(
            "{\"type\":\"turn_context\",\"timestamp\":\"2026-11-25T10:01:00.000Z\",\"payload\":{\"model\":\"gpt-5\"}}\n",
            "{\"type\":\"token_usage_record\",\"timestamp\":\"2026-11-25T10:01:02.250Z\",\"payload\":{\"response_id\":\"resp_dur_1\",\"session_id\":\"cs1\",\"usage\":{\"input_tokens\":100,\"output_tokens\":60,\"cached_input_tokens\":0,\"total_tokens\":160}}}\n"
        );
        std::fs::write(&codex_path, codex_lines).unwrap();
        assert_eq!(scan_file(&mut conn, &codex_path, "codex").unwrap(), 1);
        let codex_dur: Option<i64> = conn
            .query_row("SELECT duration_ms FROM requests WHERE dedup_key = 'resp_dur_1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(codex_dur, Some(2250));

        let _ = std::fs::remove_file(&claude_path);
        let _ = std::fs::remove_file(&codex_path);
    }

    /// P1「扫描期间源端被并发改写」形态二：**同名文件被截断 / 重写**。
    ///
    /// 两种子形态，判定逻辑不同（见 `scan_file` 的 `rewritten`）：
    ///
    /// | 情形 | 判据 | 期望 |
    /// |---|---|---|
    /// | 文件变短（截断后重写） | `saved.offset > size` | 从头重扫；已采行靠去重键兜底，不翻倍 |
    /// | 等长内容替换 | `saved.offset == size && mtime != saved.mtime` | 同样从头重扫 |
    ///
    /// 真实场景：CLI 做 compaction 后把会话文件重写一遍，行数变少、内容重排。
    /// 这是最容易丢记录的地方——若只看 offset 不检查 size/mtime，重写后的新行永远采不到。
    #[test]
    fn a_truncated_or_rewritten_file_is_rescanned_from_the_start() {
        let path = temp_jsonl("concurrent-rewrite");
        let mut conn = db::open(Path::new(":memory:")).unwrap();
        let line = |id: &str, input: i64, ts: &str| {
            format!(
                "{{\"type\":\"assistant\",\"timestamp\":\"{ts}\",\"sessionId\":\"s\",\
                  \"message\":{{\"id\":\"{id}\",\"usage\":{{\"input_tokens\":{input},\"output_tokens\":5,\
                  \"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}}}}\n"
            )
        };
        let count = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM requests WHERE platform='claude'", [], |r| r.get(0)).unwrap()
        };
        let tokens = |c: &Connection, key: &str| -> Option<i64> {
            c.query_row(
                "SELECT CASE WHEN total_known=1 THEN total_tokens END FROM requests WHERE dedup_key=?1",
                [key], |r| r.get(0),
            ).unwrap()
        };

        // 初始三行
        std::fs::write(&path, format!(
            "{}{}{}",
            line("m1", 10, "2026-09-16T00:00:00Z"),
            line("m2", 20, "2026-09-16T00:00:01Z"),
            line("m3", 30, "2026-09-16T00:00:02Z"),
        ))
        .unwrap();
        assert_eq!(scan_file(&mut conn, &path, "claude").unwrap(), 3);
        assert_eq!(count(&conn), 3);

        // 子形态 A：文件被截断重写——只剩一行，且 ID 不同（compaction 后的新会话内容）
        std::fs::write(&path, line("m4", 40, "2026-09-16T01:00:00Z")).unwrap();
        assert_eq!(scan_file(&mut conn, &path, "claude").unwrap(), 1, "截断后要能读到新行");
        assert_eq!(count(&conn), 4, "旧的三行保留（历史账本不删），新的第四行补上");
        assert_eq!(tokens(&conn, "m4"), Some(45), "m4 = 40 输入 + 5 输出");

        // 子形态 B：等长内容替换——同样的字节数，内容不同
        let before = std::fs::read_to_string(&path).unwrap();
        let replacement = line("m5", 50, "2026-09-16T02:00:00Z");
        assert_eq!(replacement.len(), before.len(), "构造等长替换是这条子形态的前提");
        // 改 mtime：等长替换只能靠 mtime 认出来
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, &replacement).unwrap();
        assert_eq!(scan_file(&mut conn, &path, "claude").unwrap(), 1, "等长替换也要重扫");
        assert_eq!(count(&conn), 5);
        assert_eq!(tokens(&conn, "m5"), Some(55));

        // 再扫一次：位点已追平且 mtime 未变，不该再插入
        assert_eq!(scan_file(&mut conn, &path, "claude").unwrap(), 0);
        assert_eq!(count(&conn), 5, "重复扫描不能增加记录");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn codex_task_started_marks_session_running_and_tracks_latest_write() {
        let state = parse_codex_file_state([
            r#"{"timestamp":"2026-09-16T00:00:00Z","type":"session_meta","payload":{"id":"session-1"}}"#,
            r#"{"timestamp":"2026-09-16T00:00:01Z","type":"event_msg","payload":{"type":"task_started"}}"#,
            r#"{"timestamp":"2026-09-16T00:00:02Z","type":"token_usage_record","payload":{}}"#,
        ]);

        assert_eq!(state.session_id.as_deref(), Some("session-1"));
        assert_eq!(state.activity_state.as_deref(), Some("running"));
        assert_eq!(state.updated_at_ms, iso_to_millis("2026-09-16T00:00:02Z"));
        assert_eq!(state.turn_started_ms, iso_to_millis("2026-09-16T00:00:01Z"));
    }

    #[test]
    fn turn_start_survives_incremental_scan_and_resets_on_next_turn() {
        use std::io::Write;
        let path = temp_jsonl("turn-start");
        let mut conn = db::open(Path::new(":memory:")).unwrap();
        std::fs::write(&path, concat!(
            "{\"timestamp\":\"2026-09-16T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"elapsed\"}}\n",
            "{\"timestamp\":\"2026-09-16T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
        )).unwrap();
        scan_file(&mut conn, &path, "codex").unwrap();
        let start = |conn: &Connection| conn.query_row(
            "SELECT turn_started_ms FROM sessions WHERE session_id = 'elapsed'",
            [], |row| row.get::<_, i64>(0),
        ).unwrap();
        assert_eq!(start(&conn), iso_to_millis("2026-09-16T00:00:01Z").unwrap());
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{{\"timestamp\":\"2026-09-16T00:01:00Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\"}}}}").unwrap();
        scan_file(&mut conn, &path, "codex").unwrap();
        assert_eq!(start(&conn), iso_to_millis("2026-09-16T00:00:01Z").unwrap());
        writeln!(file, "{{\"timestamp\":\"2026-09-16T00:02:00Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\"}}}}").unwrap();
        scan_file(&mut conn, &path, "codex").unwrap();
        assert_eq!(start(&conn), iso_to_millis("2026-09-16T00:02:00Z").unwrap());
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn codex_new_turn_resets_elapsed_start() {
        let state = parse_codex_file_state([
            r#"{"timestamp":"2026-09-16T00:00:01Z","type":"event_msg","payload":{"type":"task_started"}}"#,
            r#"{"timestamp":"2026-09-16T00:00:02Z","type":"event_msg","payload":{"type":"task_complete"}}"#,
            r#"{"timestamp":"2026-09-16T00:01:00Z","type":"event_msg","payload":{"type":"task_started"}}"#,
        ]);
        assert_eq!(state.turn_started_ms, iso_to_millis("2026-09-16T00:01:00Z"));
    }

    #[test]
    fn claude_terminal_and_metadata_events_keep_lifecycle_consistent() {
        let start = r#"{"type":"user","sessionId":"s","timestamp":"2026-09-16T00:00:00Z","message":{"content":"prompt"}}"#;
        for end in [
            r#"{"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
            r#"{"type":"system","subtype":"turn_duration"}"#,
            r#"{"type":"assistant","message":{"stop_reason":"end_turn"}}"#,
        ] {
            let mut state = SessionFileState::default();
            apply_claude_file_state(&mut state, start);
            let mut event: Value = serde_json::from_str(end).unwrap();
            event["sessionId"] = Value::String("s".into());
            event["timestamp"] = Value::String("2026-09-16T00:00:01Z".into());
            apply_claude_file_state(&mut state, &event.to_string());
            assert_ne!(state.activity_state.as_deref(), Some("running"));
            for extra in [
                r#"{"type":"user","isMeta":true,"message":{"content":"metadata"}}"#,
                r#"{"type":"user","isSidechain":true,"message":{"content":"side task"}}"#,
                r#"{"type":"user","message":{"content":[{"type":"tool_result"}]}}"#,
            ] {
                let mut event: Value = serde_json::from_str(extra).unwrap();
                event["sessionId"] = Value::String("s".into());
                event["timestamp"] = Value::String("2026-09-16T00:00:02Z".into());
                apply_claude_file_state(&mut state, &event.to_string());
                assert_ne!(state.activity_state.as_deref(), Some("running"));
            }
        }
    }

    #[test]
    fn claude_api_error_message_ends_the_turn_but_retries_do_not() {
        let start = r#"{"type":"user","sessionId":"s","timestamp":"2026-09-16T00:00:00Z","message":{"content":"prompt"}}"#;
        // 重试中的 api_error 记录：CLI 仍在自动重试，本轮没有结束
        for retry in [
            r#"{"type":"system","subtype":"api_error","level":"error","source":"request_retry","retryAttempt":1,"maxRetries":10}"#,
            r#"{"type":"system","subtype":"api_error","level":"error","source":"connection_retry","retryAttempt":2,"maxRetries":10}"#,
        ] {
            let mut state = SessionFileState::default();
            apply_claude_file_state(&mut state, start);
            let mut event: Value = serde_json::from_str(retry).unwrap();
            event["sessionId"] = Value::String("s".into());
            event["timestamp"] = Value::String("2026-09-16T00:00:01Z".into());
            apply_claude_file_state(&mut state, &event.to_string());
            assert_eq!(state.activity_state.as_deref(), Some("running"), "重试尚未放弃，本轮必须仍是运行中：{retry}");
        }
        // 最终失败：isApiErrorMessage 合成 assistant 记录（真实样本字段）→ failed
        let mut state = SessionFileState::default();
        apply_claude_file_state(&mut state, start);
        let mut error: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"model":"<synthetic>","role":"assistant","stop_reason":"stop_sequence","usage":{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0},"content":[{"type":"text","text":"API Error: 403 rejected"}]},"error":"authentication_failed","isApiErrorMessage":true,"apiErrorStatus":403}"#,
        ).unwrap();
        error["sessionId"] = Value::String("s".into());
        error["timestamp"] = Value::String("2026-09-16T00:01:00Z".into());
        apply_claude_file_state(&mut state, &error.to_string());
        assert_eq!(state.activity_state.as_deref(), Some("failed"), "最终失败的请求必须结束本轮");
        // 失败后用户再次发消息：开启新一轮 running，轮次起点刷新
        apply_claude_file_state(&mut state, r#"{"type":"user","sessionId":"s","timestamp":"2026-09-16T00:02:00Z","message":{"content":"again"}}"#);
        assert_eq!(state.activity_state.as_deref(), Some("running"));
        assert_eq!(state.turn_started_ms, iso_to_millis("2026-09-16T00:02:00Z"));
    }

    #[test]
    fn claude_lifecycle_preserves_waiting_tokens_and_ends_without_recent_window() {
        use std::io::Write;
        let path = temp_jsonl("claude-lifecycle");
        let mut conn = db::open(Path::new(":memory:")).unwrap();
        std::fs::write(&path, concat!(
            "{\"type\":\"user\",\"sessionId\":\"claude-turn\",\"timestamp\":\"2026-09-16T00:00:00Z\",\"message\":{\"role\":\"user\",\"content\":\"test prompt\"}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"claude-turn\",\"timestamp\":\"2026-09-16T00:00:02Z\",\"message\":{\"id\":\"response-test\",\"model\":\"claude-sonnet-4\",\"stop_reason\":\"tool_use\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0}}}\n",
        )).unwrap();
        scan_file(&mut conn, &path, "claude").unwrap();
        let live = db::live_usage(&conn, "claude", None).unwrap();
        assert_eq!(live.active_sessions.len(), 1, "a silent unfinished turn must not expire after 90 seconds");
        assert_eq!(live.active_window_seconds, 0);
        assert_eq!(live.running_tokens, Some(15));
        let key = path.to_string_lossy().to_string();
        let mut old_checkpoint = db::get_scan_checkpoint(&conn, &key).unwrap();
        old_checkpoint.parser_initialized = false;
        db::set_scan_checkpoint(&conn, &key, &old_checkpoint).unwrap();
        assert_eq!(scan_file(&mut conn, &path, "claude").unwrap(), 0);
        assert_eq!(db::live_usage(&conn, "claude", None).unwrap().running_tokens, Some(15));
        let start = live.active_sessions[0].started_at_ms;
        assert_eq!(start, iso_to_millis("2026-09-16T00:00:00Z"));
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{{\"type\":\"user\",\"sessionId\":\"claude-turn\",\"timestamp\":\"2026-09-16T00:20:00Z\",\"message\":{{\"content\":[{{\"type\":\"tool_result\"}}]}}}}").unwrap();
        writeln!(file, "{{\"type\":\"system\",\"subtype\":\"compact_boundary\",\"sessionId\":\"claude-turn\",\"timestamp\":\"2026-09-16T00:20:01Z\"}}").unwrap();
        scan_file(&mut conn, &path, "claude").unwrap();
        assert_eq!(db::live_usage(&conn, "claude", None).unwrap().active_sessions[0].started_at_ms, start);
        writeln!(file, "{{\"type\":\"assistant\",\"sessionId\":\"claude-turn\",\"timestamp\":\"2026-09-16T00:20:02Z\",\"message\":{{\"stop_reason\":\"end_turn\"}}}}").unwrap();
        scan_file(&mut conn, &path, "claude").unwrap();
        assert!(db::live_usage(&conn, "claude", None).unwrap().active_sessions.is_empty());
        writeln!(file, "{{\"type\":\"user\",\"sessionId\":\"claude-turn\",\"timestamp\":\"2026-09-16T00:21:00Z\",\"message\":{{\"content\":\"next prompt\"}}}}").unwrap();
        scan_file(&mut conn, &path, "claude").unwrap();
        let next = db::live_usage(&conn, "claude", None).unwrap();
        assert_eq!(next.active_sessions[0].started_at_ms, iso_to_millis("2026-09-16T00:21:00Z"));
        assert_eq!(next.running_tokens, Some(0));
        drop(file); std::fs::remove_file(path).unwrap();
    }

    #[test]
        fn dbg_today_probe() {
            use chrono::Utc;
            let path = temp_jsonl("claude-dbg");
            let mut conn = db::open(Path::new(":memory:")).unwrap();
            let base = Utc::now().timestamp_millis();
            let iso = |offset_ms: i64| {
                chrono::DateTime::from_timestamp((base + offset_ms) / 1000, 0).unwrap()
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            };
            std::fs::write(&path, format!(
                concat!(
                    "{{\"type\":\"user\",\"sessionId\":\"dbg\",\"timestamp\":\"{t0}\",\"message\":{{\"role\":\"user\",\"content\":\"p\"}}}}\n",
                    "{{\"type\":\"assistant\",\"sessionId\":\"dbg\",\"timestamp\":\"{t1}\",\"message\":{{\"id\":\"r1\",\"model\":\"m\",\"stop_reason\":\"tool_use\",\"usage\":{{\"input_tokens\":10,\"output_tokens\":5,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":1000}}}}}}\n"
                ),
                t0 = iso(0), t1 = iso(2_000),
            )).unwrap();
            scan_file(&mut conn, &path, "claude").unwrap();
            let rows: Vec<(i64, i64, i64, i64, String)> = conn.prepare(
                "SELECT input_tokens, output_tokens, total_tokens, total_known, datetime(ts/1000,'unixepoch') FROM requests")
                .unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).unwrap()
                .collect::<Result<_,_>>().unwrap();
            for row in &rows { println!("ROW {:?}", row); }
            let now_local = chrono::Local::now().timestamp_millis();
            println!("local_midnight_ms = {}", now_local - (now_local % 86_400_000));
            let range = db::period_range(&conn, "claude", "today");
            println!("today range = {} .. {}", range.0, range.1);
            let live = db::live_usage(&conn, "claude", None).unwrap();
            println!("today_tokens = {:?}", live.today_tokens);
            std::fs::remove_file(path).unwrap();
        }

    #[test]
    fn running_tokens_count_fresh_input_output_not_cache_rereads() {
        // 实时口径对齐 Claude Code 终端：缓存重读（cache_read/write）不计入运行中增量。
        // 同一轮里每次请求都重读全部缓存上下文，若计入会把 +34k 显示成 +1.5M。
        use chrono::Utc;
        let path = temp_jsonl("claude-fresh-tokens");
        let mut conn = db::open(Path::new(":memory:")).unwrap();
        // 时间戳取「当前时刻之前」，保证落在今日统计窗口内（窗口右边界是当下）
        let base = Utc::now().timestamp_millis() - 60_000;
        let iso = |offset_ms: i64| {
            chrono::DateTime::from_timestamp((base + offset_ms) / 1000, 0).unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        };
        std::fs::write(&path, format!(
            concat!(
                "{{\"type\":\"user\",\"sessionId\":\"fresh-tokens\",\"timestamp\":\"{t0}\",\"message\":{{\"role\":\"user\",\"content\":\"test prompt\"}}}}\n",
                "{{\"type\":\"assistant\",\"sessionId\":\"fresh-tokens\",\"timestamp\":\"{t1}\",\"message\":{{\"id\":\"r1\",\"model\":\"claude-sonnet-4\",\"stop_reason\":\"tool_use\",\"usage\":{{\"input_tokens\":10,\"output_tokens\":5,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":1000}}}}}}\n",
                "{{\"type\":\"assistant\",\"sessionId\":\"fresh-tokens\",\"timestamp\":\"{t2}\",\"message\":{{\"id\":\"r2\",\"model\":\"claude-sonnet-4\",\"stop_reason\":\"tool_use\",\"usage\":{{\"input_tokens\":20,\"output_tokens\":8,\"cache_creation_input_tokens\":30,\"cache_read_input_tokens\":2000}}}}}}\n"
            ),
            t0 = iso(0), t1 = iso(2_000), t2 = iso(4_000),
        )).unwrap();
        scan_file(&mut conn, &path, "claude").unwrap();
        let live = db::live_usage(&conn, "claude", None).unwrap();
        // 新鲜 Token = (10+5) + (20+8) = 43；缓存读 3000 与缓存写 30 不计
        assert_eq!(live.running_tokens, Some(43));
        // 灵动岛「本机今日 Token」同口径：新鲜 Token
        assert_eq!(live.today_tokens, Some(43));
        // 主面板统计口径不变：token_totals 仍含缓存（43 + 3000 + 30 = 3073）
        let totals = db::token_totals_at(&conn, "claude", None, None, chrono::Utc::now().timestamp_millis()).unwrap();
        assert_eq!(totals.today, Some(3073));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn codex_thinking_waiting_and_compacting_do_not_end_or_restart_turn() {
        let start = r#"{"timestamp":"2026-09-16T00:00:01Z","type":"event_msg","payload":{"type":"task_started"}}"#;
        for line in [
            r#"{"timestamp":"2026-09-16T00:10:01Z","type":"event_msg","payload":{"type":"agent_reasoning","text":"thinking"}}"#,
            r#"{"timestamp":"2026-09-16T00:10:01Z","type":"response_item","payload":{"type":"function_call","name":"request_user_input"}}"#,
            r#"{"timestamp":"2026-09-16T00:10:01Z","type":"event_msg","payload":{"type":"context_compacted"}}"#,
            r#"{"timestamp":"2026-09-16T00:10:01Z","type":"compacted","payload":{}}"#,
        ] {
            let state = parse_codex_file_state([start, line]);
            assert_eq!(state.activity_state.as_deref(), Some("running"));
            assert_eq!(state.turn_started_ms, iso_to_millis("2026-09-16T00:00:01Z"));
        }
    }

    #[test]
    fn codex_completion_abort_and_final_answer_all_stop_the_session() {
        for end in [
            r#"{"timestamp":"2026-09-16T00:00:02Z","type":"event_msg","payload":{"type":"task_complete"}}"#,
            r#"{"timestamp":"2026-09-16T00:00:02Z","type":"event_msg","payload":{"type":"turn_aborted"}}"#,
            r#"{"timestamp":"2026-09-16T00:00:02Z","type":"event_msg","payload":{"type":"agent_message","phase":"final_answer"}}"#,
        ] {
            let state = parse_codex_file_state([
                r#"{"timestamp":"2026-09-16T00:00:00Z","type":"session_meta","payload":{"id":"session-1"}}"#,
                r#"{"timestamp":"2026-09-16T00:00:01Z","type":"event_msg","payload":{"type":"task_started"}}"#,
                end,
            ]);
            let expected = if end.contains("turn_aborted") { "failed" } else { "done" };
            assert_eq!(state.activity_state.as_deref(), Some(expected), "结束事件状态错误：{end}");
        }
    }

    #[test]
    fn heartbeat_prefilter_skips_only_files_that_are_fully_caught_up() {
        let path = temp_jsonl("prefilter");
        std::fs::write(&path, b"line\n").unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let (size, mtime) = (meta.len() as i64, modified_ms(&meta));
        let stamp = |offset, mtime, parser_initialized| db::ScanStamp { offset, mtime, parser_initialized };
        assert!(needs_scan(&path, None), "没有位点的新文件必须扫描");
        assert!(!needs_scan(&path, Some(&stamp(size, mtime, true))), "已追平且未改写的文件跳过");
        assert!(needs_scan(&path, Some(&stamp(size - 1, mtime, true))), "有新字节");
        assert!(needs_scan(&path, Some(&stamp(size + 1, mtime, true))), "文件被截断");
        assert!(needs_scan(&path, Some(&stamp(size, mtime - 1, true))), "等长改写");
        assert!(needs_scan(&path, Some(&stamp(size, mtime, false))), "旧版位点需重建解析上下文");
        let _ = std::fs::remove_file(&path);
        assert!(needs_scan(&path, Some(&stamp(size, mtime, true))), "读不到文件时交给采集报告错误");
    }

    #[test]
    fn targeted_scan_only_reads_the_supplied_file() {
        let first = temp_jsonl("targeted-first");
        let second = temp_jsonl("targeted-second");
        std::fs::write(
            &first,
            b"{\"timestamp\":\"2026-09-16T00:00:00Z\",\"type\":\"assistant\",\"sessionId\":\"s1\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}}\n",
        )
        .unwrap();
        std::fs::write(
            &second,
            b"{\"timestamp\":\"2026-09-16T00:00:00Z\",\"type\":\"assistant\",\"sessionId\":\"s2\",\"message\":{\"id\":\"m2\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20}}}\n",
        )
        .unwrap();
        let database = temp_jsonl("targeted-db").with_extension("db");
        let mut conn = crate::db::open(&database).unwrap();

        let result = scan_files(&mut conn, vec![(first.clone(), "claude")]);

        assert_eq!(result.files_scanned, 1);
        assert_eq!(result.records_inserted, 1);
        assert_eq!(crate::db::get_offset(&conn, &second.to_string_lossy()), 0);
        let stored: (i64, i64, i64) = conn
            .query_row("SELECT input_tokens, output_tokens, total_known FROM requests", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        assert_eq!(stored, (1, 2, 0));
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
        drop(conn);
        let _ = std::fs::remove_file(database);
    }

    #[test]
    fn scan_skips_invalid_utf8_line_and_keeps_following_records() {
        let path = temp_jsonl("invalid-utf8-line");
        std::fs::write(
            &path,
            b"\xff\n{\"timestamp\":\"2026-09-16T00:00:00Z\",\"type\":\"assistant\",\"sessionId\":\"s1\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}}\n",
        )
        .unwrap();
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();

        let result = scan_files(&mut conn, vec![(path.clone(), "claude")]);

        assert!(result.errors.is_empty(), "坏行不应阻断整份日志：{:?}", result.errors);
        assert_eq!(result.records_inserted, 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn scan_failures_report_their_platform_so_other_sources_stay_trustworthy() {
        // 目录不存在 → open 失败 → 采集错误必须带上所属平台，供状态展示按平台定界
        let missing = temp_jsonl("missing-codex-dir");
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();

        let result = scan_files(&mut conn, vec![(missing.clone(), "codex")]);

        assert_eq!(result.errors.len(), 1, "读不到文件应报错：{:?}", result.errors);
        assert_eq!(result.failed_sources, vec!["codex".to_string()], "错误必须定界到出错平台");
        // 成功的文件不产生任何平台污染
        let ok_path = temp_jsonl("ok-claude");
        std::fs::write(
            &ok_path,
            "{\"timestamp\":\"2026-09-16T00:00:00Z\",\"type\":\"assistant\",\"sessionId\":\"s1\",\"message\":{\"id\":\"m-ok\",\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}}\n",
        )
        .unwrap();
        let clean = scan_files(&mut conn, vec![(ok_path.clone(), "claude")]);
        assert!(clean.errors.is_empty());
        assert!(clean.failed_sources.is_empty(), "成功扫描不得报告失败平台");
        let _ = std::fs::remove_file(missing);
        let _ = std::fs::remove_file(ok_path);
    }

    #[test]
    fn codex_streaming_scan_batches_records_and_reuses_persisted_turn_context() {
        let path = temp_jsonl("codex-streaming");
        let database = temp_jsonl("codex-streaming-db").with_extension("db");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, r#"{{"timestamp":"2026-09-16T00:00:00Z","type":"session_meta","payload":{{"id":"session-stream"}}}}"#).unwrap();
        writeln!(file, r#"{{"timestamp":"2026-09-16T00:00:01Z","type":"turn_context","payload":{{"model":"gpt-5.6","thread_settings":{{"reasoning_effort":"high"}}}}}}"#).unwrap();
        for index in 0..(RECORD_BATCH_SIZE + 10) {
            writeln!(file, "{{\"timestamp\":\"2026-09-16T00:00:02Z\",\"type\":\"token_usage_record\",\"payload\":{{\"response_id\":\"response-{index}\",\"session_id\":\"session-stream\",\"usage\":{{\"input_tokens\":1,\"output_tokens\":2,\"total_tokens\":3}}}}}}").unwrap();
        }
        drop(file);
        let mut conn = crate::db::open(&database).unwrap();

        let first = scan_files(&mut conn, vec![(path.clone(), "codex")]);
        assert_eq!(first.records_inserted, RECORD_BATCH_SIZE + 10);

        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, r#"{{"timestamp":"2026-09-16T00:00:03Z","type":"token_usage_record","payload":{{"response_id":"response-next","session_id":"session-stream","usage":{{"input_tokens":2,"output_tokens":3,"total_tokens":5}}}}}}"#).unwrap();
        drop(file);
        let second = scan_files(&mut conn, vec![(path.clone(), "codex")]);
        assert_eq!(second.records_inserted, 1);
        // model 与 effort 都只在首次扫描读到的 turn_context 上出现。续扫的这条记录
        // 要拿到它们，只能靠位点里持久化的解析上下文——否则会平白缺值。
        let (model, effort): (String, Option<String>) = conn.query_row(
            "SELECT model, effort FROM requests WHERE dedup_key = 'response-next'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(model, "gpt-5.6");
        assert_eq!(effort.as_deref(), Some("high"));

        let _ = std::fs::remove_file(path);
        drop(conn);
        let _ = std::fs::remove_file(database);
    }

    #[test]
    fn claude_api_request_and_gateway_tool_use_stay_running_across_long_commands() {
        let mut state = SessionFileState::default();
        // 1) 队列出队或恢复对话时，api-request 一落盘即立刻进入 running
        apply_claude_file_state(
            &mut state,
            r#"{"type":"api-request","sessionId":"s-long","timestamp":"2026-11-27T10:00:00Z"}"#,
        );
        assert_eq!(state.activity_state.as_deref(), Some("running"));
        assert_eq!(state.turn_started_ms, iso_to_millis("2026-11-27T10:00:00Z"));

        // 2) 即使第三方兼容网关将带 tool_use 块的响应写成了 stop_reason="end_turn"，长命令执行期也绝不能误判结束
        apply_claude_file_state(
            &mut state,
            r#"{"type":"assistant","sessionId":"s-long","timestamp":"2026-11-27T10:00:05Z","message":{"stop_reason":"end_turn","content":[{"type":"tool_use","id":"tu_1","name":"Bash"}]}}"#,
        );
        assert_eq!(state.activity_state.as_deref(), Some("running"));

        // 3) 命令跑了 6 分钟后返回 tool_result，轮次起点与 running 状态完整保留
        apply_claude_file_state(
            &mut state,
            r#"{"type":"user","sessionId":"s-long","timestamp":"2026-11-27T10:06:05Z","message":{"content":[{"type":"tool_result","tool_use_id":"tu_1","content":"ok"}]}}"#,
        );
        assert_eq!(state.activity_state.as_deref(), Some("running"));
        assert_eq!(state.turn_started_ms, iso_to_millis("2026-11-27T10:00:00Z"));

        // 4) 若用户在长命令执行途中按 Ctrl+C 中断（tool_result 带中断标记），立即转为 failed
        apply_claude_file_state(
            &mut state,
            r#"{"type":"user","sessionId":"s-long","timestamp":"2026-11-27T10:07:00Z","toolUseResult":true,"message":{"content":[{"type":"tool_result","content":"[Request interrupted by user for tool use]"}]}}"#,
        );
        assert_eq!(state.activity_state.as_deref(), Some("failed"));
    }

    #[test]
    fn codex_user_message_and_long_shell_command_keep_session_running() {
        let state = parse_codex_file_state([
            r#"{"timestamp":"2026-11-27T10:00:00Z","type":"session_meta","payload":{"id":"codex-long"}}"#,
            r#"{"timestamp":"2026-11-27T10:00:01Z","type":"event_msg","payload":{"type":"user_message"}}"#,
            r#"{"timestamp":"2026-11-27T10:00:04Z","type":"response_item","payload":{"type":"function_call","name":"exec_command"}}"#,
            r#"{"timestamp":"2026-11-27T10:08:00Z","type":"response_item","payload":{"type":"function_call_output"}}"#,
        ]);
        assert_eq!(state.activity_state.as_deref(), Some("running"));
        assert_eq!(state.turn_started_ms, iso_to_millis("2026-11-27T10:00:01Z"));
    }

    #[test]
    #[ignore = "读取本机 Claude/Codex 日志；仅显式执行且不输出会话内容"]
    fn real_local_logs_read_only_smoke() {
        let database = temp_jsonl("real-log-smoke").with_extension("db");
        let mut conn = crate::db::open(&database).unwrap();
        let result = scan(&mut conn);
        assert!(result.files_scanned > 0, "未发现可验证的本机会话日志");
        assert!(result.errors.is_empty(), "真实日志扫描错误：{:?}", result.errors);
        let records: i64 = conn.query_row("SELECT COUNT(*) FROM requests", [], |row| row.get(0)).unwrap();
        assert!(records > 0, "真实日志没有解析出任何请求记录");
        let claude: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform = 'claude'", [], |row| row.get(0)).unwrap();
        let codex: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform = 'codex'", [], |row| row.get(0)).unwrap();
        assert!(claude > 0, "Claude 真实日志没有解析出请求记录");
        assert!(codex > 0, "Codex 真实日志没有解析出请求记录");
        println!("validated_files={} parsed_records={records} claude_records={claude} codex_records={codex}", result.files_scanned);
        drop(conn);
        for path in [database.clone(), database.with_extension("db-wal"), database.with_extension("db-shm")] {
            let _ = std::fs::remove_file(path);
        }
    }

    /// 诊断工具：复制真实应用数据库（含各文件扫描位点与解析上下文）到临时目录，
    /// 用与应用完全相同的增量路径跑一次全量扫描并输出真实错误文本。
    /// 与 [`real_local_logs_read_only_smoke`] 的差异：那个从全新库从头扫，
    /// 复现不了「应用位点 + 增量读取」路径上的问题。只输出错误与计数，不输出会话内容。
    #[test]
    #[ignore = "复制应用真实库诊断采集错误；仅显式执行且不输出会话内容"]
    fn diagnose_app_scan_state_errors() {
        let Ok(app_data) = std::env::var("APPDATA") else {
            panic!("缺少 APPDATA，非 Windows 环境无法诊断");
        };
        let dir = std::path::Path::new(&app_data).join("dev.ningz.jolu");
        let workspace = temp_jsonl("app-db-diagnose");
        let database = workspace.with_extension("db");
        for (from, to) in [
            ("usage.db", "diagnosis.db"),
            ("usage.db-wal", "diagnosis.db-wal"),
            ("usage.db-shm", "diagnosis.db-shm"),
        ] {
            let source = dir.join(from);
            if source.exists() {
                std::fs::copy(&source, workspace.with_file_name(to))
                    .unwrap_or_else(|error| panic!("复制 {from} 失败：{error}"));
            }
        }
        let mut conn = crate::db::open(&database).unwrap();
        let result = scan(&mut conn);
        println!(
            "files={} inserted={} errors={}",
            result.files_scanned,
            result.records_inserted,
            result.errors.len()
        );
        for error in &result.errors {
            println!("ERROR: {error}");
        }
        assert!(result.errors.is_empty(), "应用库位点下仍存在采集错误（见上方 ERROR）");
        drop(conn);
        let _ = std::fs::remove_file(&database);
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(workspace.with_file_name(&format!("diagnosis.db{suffix}")));
        }
    }
