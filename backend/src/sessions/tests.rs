use std::collections::HashSet;
use std::path::Path;
use rusqlite::Connection;

    #[test]
    fn external_session_platforms_can_be_filtered() {
        for platform in ["kimi", "pi", "dsh", "claude", "zcode"] {
            assert!(super::session_platform_allowed(platform), "{platform}");
        }
        assert!(!super::session_platform_allowed("nope"));
    }

    use super::*;

    #[test]
    fn native_database_activity_requires_evidence_and_expires() {
        let conn = crate::db::open(Path::new(":memory:")).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        for (platform, source, id) in [
            ("zcode", "native:zcode:main", "fresh"),
            ("zcode", "native:zcode:main", "stale"),
            ("zcode", "native:zcode:main", "legacy"),
            ("gemini", "native:gemini:main", "other"),
        ] {
            conn.execute(
                "INSERT INTO requests(platform, source, dedup_key, session_id, ts) VALUES(?1, ?2, ?3, ?3, ?4)",
                rusqlite::params![platform, source, id, now],
            ).unwrap();
        }
        crate::db::set_session_activity(&conn, "zcode", "fresh", "running", now).unwrap();
        crate::db::set_session_activity(&conn, "zcode", "stale", "running", now - crate::native_sources::ZCODE_RUNNING_GRACE_MS - 1).unwrap();
        conn.execute("INSERT INTO sessions(platform, session_id) VALUES('zcode', 'legacy')", []).unwrap();
        crate::db::set_session_activity(&conn, "gemini", "other", "done", now).unwrap();
        let result = list(&conn, &SessionFilter::default()).unwrap();
        let status = |id: &str| result.sessions.iter().find(|s| s.session_id == id).unwrap().status.as_str();
        assert_eq!(status("fresh"), "running");
        assert_eq!(status("stale"), "recent");
        assert_eq!(status("legacy"), "recent");
        assert_eq!(status("other"), "recent");
    }

    #[test]
    fn message_read_failure_is_not_reported_as_partial_success() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("jolu-message-read-{}-{nonce}", std::process::id()));
        let folder = root.join(".claude");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("broken-session.jsonl");
        let mut bytes = b"{\"type\":\"user\",\"message\":{\"content\":\"valid first line\"}}\n".to_vec();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read_lines(&path).unwrap().len(), 1);
        bytes.extend_from_slice(&[0xff, b'\n']);
        std::fs::write(&path, bytes).unwrap();
        let conn = crate::db::open(Path::new(":memory:")).unwrap();
        conn.execute("INSERT INTO scan_state(path, offset, mtime, parser_session_id) VALUES(?1, 0, 0, 'broken-session')", [path.to_string_lossy().as_ref()]).unwrap();
        let result = messages(&conn, "claude", "broken-session", None, 40);
        std::fs::remove_file(&path).unwrap();
        assert!(read_lines(&path).is_err(), "missing files must not become an empty successful read");
        std::fs::remove_dir(&folder).unwrap();
        std::fs::remove_dir(&root).unwrap();
        assert!(result.is_err(), "a file read failure must not return a successful partial conversation");
    }

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn claude_messages_merge_blocks_and_attach_tool_output() {
        let msgs = parse_claude(lines(&[
            r#"{"type":"user","timestamp":"2026-10-21T04:32:00Z","message":{"role":"user","content":"把列表和详情放在同一页"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"assistant","timestamp":"2026-10-21T04:33:00Z","message":{"content":[{"type":"thinking","thinking":"x"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"已整理为左右分栏"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"SessionList.tsx"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"已读取 128 行"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"子代理"}]}}"#,
        ]));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].text, "已整理为左右分栏");
        assert_eq!(msgs[1].tools[0].summary, "SessionList.tsx");
        assert_eq!(msgs[1].tools[0].output.as_deref(), Some("已读取 128 行"));
    }

    #[test]
    fn codex_messages_skip_injected_context_and_pair_calls() {
        let msgs = parse_codex(lines(&[
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-10-21T04:32:00Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"优化会话界面"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"rg\",\"Session\"]}","call_id":"c1"}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"3 matches"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"完成"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"规则"}]}}"#,
        ]));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text, "优化会话界面");
        assert_eq!(msgs[1].tools[0].summary, "rg Session");
        assert_eq!(msgs[1].tools[0].output.as_deref(), Some("3 matches"));
        assert_eq!(msgs[1].text, "完成");
    }

    const CLAUDE_FIXTURE: &str = include_str!("../../tests/fixtures/handoff/claude-compacted.jsonl");
    const CODEX_FIXTURE: &str = include_str!("../../tests/fixtures/handoff/codex-commands.jsonl");

    fn fixture(text: &str) -> Vec<String> {
        text.lines().map(String::from).collect()
    }

    #[test]
    fn claude_tool_failure_exit_code_and_uuid_dedupe() {
        let msgs = parse_claude(fixture(CLAUDE_FIXTURE));
        // 压缩后按原 uuid 重写的 u1 被去重；两条摘要记录不进入消息
        assert_eq!(msgs.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(), ["user", "assistant", "user", "assistant", "user"]);
        assert_eq!(msgs.iter().filter(|m| m.text == "占位：用户第一条要求").count(), 1);
        let bash = &msgs[1].tools[0];
        assert_eq!((bash.failed, bash.exit_code), (Some(true), Some(1)));
        let edit = &msgs[1].tools[1];
        assert_eq!(edit.paths, vec!["D:/placeholder/project/src/a.ts".to_string()]);
        assert_eq!((edit.failed, edit.exit_code), (None, None), "没有 is_error 时不推断");
        let ls = &msgs[3].tools[0];
        assert_eq!((ls.failed, ls.exit_code), (Some(false), None));
        // 详细解析与会话页序号一致
        assert_eq!(parse_detail("claude", &fixture(CLAUDE_FIXTURE)).len(), msgs.len());
    }

    #[test]
    fn codex_detail_reads_command_events() {
        let view = parse_codex(fixture(CODEX_FIXTURE));
        let detail = parse_detail("codex", &fixture(CODEX_FIXTURE));
        assert_eq!(view.len(), 2);
        assert_eq!(detail.len(), view.len(), "命令事件不新增消息");
        // 会话页显示不变：只有 exec 与 exec_command 两个调用
        assert_eq!(view[1].tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["exec", "exec_command"]);
        let tools = &detail[1].tools;
        assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["exec", "command_execution", "apply_patch", "exec_command"]);
        assert_eq!((tools[0].failed, tools[0].exit_code), (Some(true), None), "Script failed");
        assert_eq!(tools[0].summary, "占位脚本第一行\n占位脚本第二行");
        assert_eq!(view[1].tools[0].summary, "占位脚本第一行");
        assert_eq!((tools[1].summary.as_str(), tools[1].failed, tools[1].exit_code), ("pnpm test", Some(true), Some(1)));
        assert_eq!(tools[2].paths, vec!["src/a.ts".to_string()]);
        assert_eq!((tools[3].summary.as_str(), tools[3].failed, tools[3].exit_code), ("cargo check", Some(false), Some(0)));
    }

    #[test]
    fn detail_output_keeps_head_and_tail() {
        let long: String = (0..10_000).map(|i| format!("行{i}\n")).collect();
        let clipped = DETAIL.output(&long);
        assert!(clipped.starts_with("行0\n") && clipped.trim_end().ends_with("行9999"));
        assert!(clipped.contains("中间省略"));
        assert!(VIEW.output(&long).ends_with('…'));
    }

    #[test]
    fn unsafe_session_ids_are_rejected() {
        assert!(safe_id("0f8e2c5a-1b2c-4d5e-8f90-123456789abc"));
        assert!(!safe_id("x & calc"));
        assert!(!safe_id(""));
    }

    /* ------------------------------ ZCode 库型来源 ------------------------------ */

    const ZSESSION: &str = "session_fixture";

    /// 内存里的小 sqlite：只建 ZCode 的 session / message / part 三张表和两条索引，
    /// 断言不依赖本机那份 477 MB 的库。
    fn zcode_fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, title TEXT NOT NULL DEFAULT '', directory TEXT NOT NULL DEFAULT '');
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL, sequence INTEGER);
             CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL, sequence INTEGER);
             CREATE INDEX message_session_sequence_idx ON message(session_id, sequence, time_created, id);
             CREATE INDEX part_message_sequence_idx ON part(message_id, sequence, time_created, id);",
        ).unwrap();
        conn
    }

    fn zcode_message_row(conn: &Connection, id: &str, sequence: i64, ts: i64, data: &str) {
        conn.execute(
            "INSERT INTO message(id, session_id, time_created, time_updated, data, sequence) VALUES(?1, ?2, ?3, ?3, ?4, ?5)",
            rusqlite::params![id, ZSESSION, ts, data, sequence],
        ).unwrap();
    }

    fn zcode_part_row(conn: &Connection, id: &str, message: &str, sequence: i64, data: &str) {
        conn.execute(
            "INSERT INTO part(id, message_id, session_id, time_created, time_updated, data, sequence) VALUES(?1, ?2, ?3, 0, 0, ?4, ?5)",
            rusqlite::params![id, message, ZSESSION, data, sequence],
        ).unwrap();
    }

    fn jolu_store() -> Connection {
        crate::db::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn zcode_stream_keeps_text_reasoning_tools_and_hides_injected_messages() {
        let zdb = zcode_fixture();
        zcode_message_row(&zdb, "m1", 0, 1_000, r#"{"role":"user","time":{"created":1000},"semantics":{"uiVisibility":"visible","transcriptVisibility":"visible"}}"#);
        zcode_part_row(&zdb, "m1p1", "m1", 0, r#"{"type":"text","text":"把列表和详情放在同一页"}"#);
        // 系统提醒：模型可见、记录里不可见，不显示
        zcode_message_row(&zdb, "m2", 1, 1_100, r#"{"role":"user","time":{"created":1100},"synthetic":true,"metadata":{"visibility":"model-only"},"semantics":{"uiVisibility":"hidden","transcriptVisibility":"hidden"}}"#);
        zcode_part_row(&zdb, "m2p1", "m2", 0, r#"{"type":"text","text":"agent_listing_delta"}"#);
        zcode_message_row(&zdb, "m3", 2, 1_200, r#"{"role":"assistant","time":{"created":1200,"completed":1500},"modelId":"GLM-5.3","providerId":"account:zai-start-plan","cost":0.5,"tokens":{"total":120,"input":100,"output":20,"reasoning":5,"cache":{"read":30,"write":0}}}"#);
        zcode_part_row(&zdb, "m3p1", "m3", 0, r#"{"type":"step-start"}"#);
        zcode_part_row(&zdb, "m3p2", "m3", 1, r#"{"type":"reasoning","text":"先看现状再改"}"#);
        zcode_part_row(&zdb, "m3p3", "m3", 2, r#"{"type":"text","text":"已整理为左右分栏"}"#);
        zcode_part_row(&zdb, "m3p4", "m3", 3, r#"{"type":"tool","callID":"call_1","tool":"Bash","state":{"status":"completed","input":{"command":"rg SessionList"},"output":"3 matches"}}"#);
        zcode_part_row(&zdb, "m3p5", "m3", 4, r#"{"type":"tool","callID":"call_2","tool":"Read","state":{"status":"error","input":{"file_path":"src/a.ts"},"output":"ENOENT"}}"#);

        let stream = zcode_load(&zdb, ZSESSION, VIEW, ZCODE_PART_LIMIT).unwrap();
        assert!(!stream.truncated);
        assert_eq!(stream.skipped, 0);
        assert_eq!(stream.messages.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(), ["user", "assistant"]);
        assert_eq!(stream.messages[0].text, "把列表和详情放在同一页");
        let assistant = &stream.messages[1];
        assert_eq!(assistant.ts, Some(1_200));
        assert_eq!(assistant.text, "已整理为左右分栏");
        assert_eq!(assistant.reasoning.as_deref(), Some("先看现状再改"));
        assert_eq!(assistant.model.as_deref(), Some("GLM-5.3"));
        assert_eq!(assistant.provider.as_deref(), Some("account:zai-start-plan"));
        assert_eq!(assistant.cost, Some(0.5));
        let tokens = assistant.tokens.as_ref().unwrap();
        assert_eq!((tokens.total, tokens.input, tokens.cache_read), (Some(120), Some(100), Some(30)));
        let tools = &assistant.tools;
        assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Bash", "Read"]);
        assert_eq!((tools[0].summary.as_str(), tools[0].output.as_deref(), tools[0].failed), ("rg SessionList", Some("3 matches"), Some(false)));
        assert_eq!((tools[1].failed, tools[1].output.as_deref()), (Some(true), Some("ENOENT")));
        assert_eq!(tools[1].paths, vec!["src/a.ts".to_string()]);
    }

    #[test]
    fn zcode_tokens_fall_back_to_step_finish_parts() {
        let zdb = zcode_fixture();
        // 消息级没有 tokens / cost 时用 step-finish 结算节点累加，仍缺失就是 None
        zcode_message_row(&zdb, "m1", 0, 1_000, r#"{"role":"assistant","modelId":"GLM-5.3"}"#);
        zcode_part_row(&zdb, "m1p1", "m1", 0, r#"{"type":"text","text":"第一段"}"#);
        zcode_part_row(&zdb, "m1p2", "m1", 1, r#"{"type":"step-finish","cost":0.25,"tokens":{"total":10,"input":8,"output":2,"cache":{"read":4,"write":0}}}"#);
        zcode_part_row(&zdb, "m1p3", "m1", 2, r#"{"type":"step-finish","cost":0.75,"tokens":{"total":20,"input":16,"output":4,"cache":{"read":0,"write":0}}}"#);
        let stream = zcode_load(&zdb, ZSESSION, VIEW, ZCODE_PART_LIMIT).unwrap();
        let message = &stream.messages[0];
        let tokens = message.tokens.as_ref().expect("step-finish 应能累加 tokens");
        assert_eq!((tokens.total, tokens.input, tokens.output, tokens.cache_read), (Some(30), Some(24), Some(6), Some(4)));
        assert_eq!(message.cost, Some(1.0));
    }

    #[test]
    fn zcode_part_budget_keeps_the_newest_messages_and_flags_truncation() {
        let zdb = zcode_fixture();
        for i in 0..3i64 {
            let id = format!("m{i}");
            zcode_message_row(&zdb, &id, i, 1_000 + i, r#"{"role":"user"}"#);
            zcode_part_row(&zdb, &format!("{id}p0"), &id, 0, &format!(r#"{{"type":"text","text":"消息 {i} 前半"}}"#));
            zcode_part_row(&zdb, &format!("{id}p1"), &id, 1, &format!(r#"{{"type":"text","text":"消息 {i} 后半"}}"#));
        }
        let stream = zcode_load(&zdb, ZSESSION, VIEW, 2).unwrap();
        assert!(stream.truncated, "超过 part 上限要标记截断");
        assert_eq!(stream.messages.len(), 1, "截断时保留最近的会话尾巴");
        assert_eq!(stream.messages[0].text, "消息 2 前半\n\n消息 2 后半");

        let jolu = jolu_store();
        let page = zcode_page_with(&jolu, Some(&zdb), ZSESSION, None, 40, 2).unwrap();
        assert!(page.available);
        assert_eq!(page.messages.len(), 1);
        assert!(page.note.as_deref().is_some_and(|note| note.contains("只解析了本会话最近")), "截断必须在页面上标注");
    }

    #[test]
    fn zcode_single_oversized_message_still_shows_its_latest_parts() {
        let zdb = zcode_fixture();
        // 一条消息自己就超过上限：留下它最近的 part，而不是让最新一次对话整段消失
        zcode_message_row(&zdb, "m1", 0, 1_000, r#"{"role":"user"}"#);
        for i in 0..4i64 {
            zcode_part_row(&zdb, &format!("p{i}"), "m1", i, &format!(r#"{{"type":"text","text":"段 {i}"}}"#));
        }
        let stream = zcode_load(&zdb, ZSESSION, VIEW, 2).unwrap();
        assert!(stream.truncated);
        assert_eq!(stream.messages.len(), 1);
        assert_eq!(stream.messages[0].text, "段 2\n\n段 3");
    }

    /// 条数上限挡不住「几千条大 part」：字节预算先到就先停，且从最旧的开始丢。
    #[test]
    fn zcode_part_budget_also_caps_bytes() {
        let mut rows: Vec<String> = (0..10).map(|_| "x".repeat(1024 * 1024)).collect();
        assert!(keep_recent_parts(&mut rows, ZCODE_PART_LIMIT, 4 * 1024 * 1024));
        assert_eq!(rows.len(), 4, "字节预算先到：只留最近的 4 MB");

        let mut rows: Vec<String> = (0..10).map(|i| format!("段{i}")).collect();
        assert!(!keep_recent_parts(&mut rows, ZCODE_PART_LIMIT, 1024), "没超预算不动内容");
        assert_eq!(rows.len(), 10);

        let mut rows = vec!["超长".repeat(1024), "最新".to_string()];
        assert!(keep_recent_parts(&mut rows, ZCODE_PART_LIMIT, 16));
        assert_eq!(rows, vec!["最新".to_string()], "单条超预算时保留最新一条");
    }

    /// 2000 条以内但字节很大：解析到预算就停，不再把后面的消息也读进来。
    #[test]
    fn zcode_byte_budget_stops_before_the_next_message() {
        let zdb = zcode_fixture();
        let part = |index: i64, half: &str| {
            format!(r#"{{"type":"text","text":"消息 {index} {half}，占位字节"}}"#)
        };
        for i in 0..3i64 {
            let id = format!("m{i}");
            zcode_message_row(&zdb, &id, i, 1_000 + i, r#"{"role":"user"}"#);
            zcode_part_row(&zdb, &format!("{id}p0"), &id, 0, &part(i, "前半"));
            zcode_part_row(&zdb, &format!("{id}p1"), &id, 1, &part(i, "后半"));
        }
        // 刚好放得下一条消息、放不下第二条
        let budget = part(0, "前半").len() + part(0, "后半").len();
        let stream = zcode_load_with(&zdb, ZSESSION, VIEW, ZCODE_PART_LIMIT, budget).unwrap();
        assert!(stream.truncated, "超过字节预算要标记截断");
        assert_eq!(stream.messages.len(), 1, "只解析最近一条消息，后面的不再读");
        assert_eq!(stream.messages[0].text, "消息 2 前半，占位字节\n\n消息 2 后半，占位字节");
    }

    /// 会话页逐行流式解析：结果与整份读入一致；文件缺失仍然是错误，不是空会话。
    #[test]
    fn session_file_parsing_streams_and_reports_missing_files() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("jolu-session-stream-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("stream-session.jsonl");
        let body = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":\"第一句\"}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"第二句\"}]}}\n",
        );
        std::fs::write(&path, body).unwrap();
        let streamed = parse_file(&path, "claude").unwrap();
        let buffered = parse_claude(read_lines(&path).unwrap());
        assert_eq!(streamed.len(), buffered.len());
        assert_eq!(streamed[0].text, buffered[0].text);
        assert_eq!(streamed[1].text, "第二句");
        std::fs::remove_file(&path).unwrap();
        assert!(parse_file(&path, "claude").is_err(), "文件缺失不能变成空会话");
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn zcode_malformed_rows_degrade_instead_of_failing_the_page() {
        let zdb = zcode_fixture();
        zcode_message_row(&zdb, "bad", 0, 1_000, "{not json");
        zcode_message_row(&zdb, "m1", 1, 1_100, r#"{"role":"user"}"#);
        zcode_part_row(&zdb, "m1p1", "m1", 0, r#"{"type":"text","text":"还能打开"}"#);
        zcode_message_row(&zdb, "m2", 2, 1_200, r#"{"role":"user"}"#);
        // 单条 part 坏掉只丢这一条，不影响同一条消息的其他内容和整页
        zcode_part_row(&zdb, "m2p1", "m2", 0, "not json either");
        zcode_part_row(&zdb, "m2p2", "m2", 1, r#"{"type":"text","text":"好的"}"#);

        let jolu = jolu_store();
        let page = zcode_page_with(&jolu, Some(&zdb), ZSESSION, None, 40, ZCODE_PART_LIMIT).unwrap();
        assert!(page.available);
        assert_eq!(page.total, 2);
        assert_eq!(page.messages[0].text, "还能打开");
        assert_eq!(page.messages[1].text, "好的");
        assert!(page.note.as_deref().is_some_and(|note| note.contains("格式无法解析")));
    }

    #[test]
    fn zcode_without_a_readable_database_reports_usage_only() {
        let jolu = jolu_store();
        // 库不存在（未安装 / 已删除）：降级为「仅用量」，不是错误页
        let page = zcode_page(&jolu, None, "session_missing", None, 40).unwrap();
        assert!(!page.available);
        assert_eq!(page.total, 0);
        assert!(page.messages.is_empty());
        assert!(page.note.is_none());
        // 结构漂移（旧版本库缺少列）：同样降级，不 panic、不猜列名
        let odd = Connection::open_in_memory().unwrap();
        odd.execute_batch("CREATE TABLE message (id TEXT PRIMARY KEY)").unwrap();
        let page = zcode_page(&jolu, Some(&odd), "session_missing", None, 40).unwrap();
        assert!(!page.available);
    }

    /// 真实库的只读冒烟：只在显式执行时跑，断言不依赖具体对话内容，也不写任何文件
    #[test]
    #[ignore = "需要本机 ZCode 会话库；仅显式只读执行"]
    fn zcode_local_database_readonly_smoke() {
        assert!(zcode_db_path().is_some_and(|path| path.is_file()), "未找到 ZCode 会话库");
        let zdb = open_zcode_db().expect("ZCode 会话库应能以只读方式打开");
        assert!(zcode_schema_ok(&zdb), "本机库结构应与探测一致");
        let raw: String = zdb
            .query_row("SELECT id FROM session WHERE parent_id IS NULL ORDER BY time_updated DESC LIMIT 1", [], |r| r.get(0))
            .expect("本机应有根会话");
        // 走真实入口：Jolu 的 session_id 带 `native:zcode:main:` 前缀，必须能还原成原库的裸 id
        let jolu = jolu_store();
        let session_id = format!("native:zcode:main:{raw}");
        let started = std::time::Instant::now();
        let page = messages(&jolu, "zcode", &session_id, None, 40).unwrap();
        let elapsed = started.elapsed().as_millis();
        println!("zcode read-only: available={} total={} page={} note={} elapsed_ms={elapsed}",
            page.available, page.total, page.messages.len(), page.note.is_some());
        assert!(page.available, "本机库应可只读解析");
        assert!(!page.messages.is_empty(), "最近的根会话应有可读正文");
        // 477 MB 的库单会话最多 14,404 条 part；按会话 id 走索引解析必须远低于一次全量扫描
        assert!(elapsed < 5_000, "单会话解析耗时过长：{elapsed} ms");
    }

    #[test]
    fn zcode_session_ids_keep_the_collector_prefix_but_query_by_raw_id() {
        assert_eq!(zcode_raw_session_id("native:zcode:main:sess_0a15"), "sess_0a15");
        assert_eq!(zcode_raw_session_id("native:zcode:ai:session_6b90"), "session_6b90");
        // 已经是裸 id 时不改动
        assert_eq!(zcode_raw_session_id("sess_0a15"), "sess_0a15");
        let children: HashSet<String> = ["sess_child".to_string()].into_iter().collect();
        assert!(zcode_child_session(Some(&children), "zcode", "native:zcode:main:sess_child"));
    }

    #[test]
    fn zcode_child_sessions_are_not_listed_as_main_sessions() {
        let children: HashSet<String> = ["session_child".to_string()].into_iter().collect();
        assert!(zcode_child_session(Some(&children), "zcode", "session_child"));
        assert!(!zcode_child_session(Some(&children), "zcode", "session_root"));
        // 只按平台过滤，其他来源的同名 id 不受影响
        assert!(!zcode_child_session(Some(&children), "claude", "session_child"));
        // 库不可读时不过滤，宁可多显示也不要吞掉真实会话
        assert!(!zcode_child_session(None, "zcode", "session_child"));
    }

    #[test]
    fn antigravity_transcript_parses_user_requests_reasoning_and_tool_outputs() {
        let msgs = parse_antigravity(lines(&[
            r#"{"type":"USER_INPUT","step_index":0,"created_at":"2026-11-26T00:50:00+08:00","content":"<system-instruction>rules</system-instruction><USER_REQUEST>\n检查会话线程为何读不到 Antigravity\n</USER_REQUEST><ADDITIONAL_METADATA>meta</ADDITIONAL_METADATA>","media":[{"mime_type":"image/png","uri":"file:///tmp/a.png"}]}"#,
            r#"{"type":"PLANNER_RESPONSE","step_index":1,"created_at":"2026-11-26T00:50:05+08:00","model":"gemini-4-neon-xhigh","thinking":"先读取 session_threads.rs","tool_calls":[{"name":"read_file","args":{"Path":"D:/develop/Jolu/backend/src/session_threads.rs"}},{"name":"run_command","args":{"CommandLine":"cargo test","toolSummary":"Run unit tests"}}],"input_tokens":1000,"output_tokens":120,"cache_read_tokens":5000}"#,
            r#"{"type":"GENERIC","step_index":2,"status":"DONE","content":"Showing lines 1-50 of 1774"}"#,
            r#"{"type":"GENERIC","step_index":3,"status":"ERROR","content":"Command exited with code 1\nfailed","error":"exit 1"}"#,
            r#"{"type":"SYSTEM_MESSAGE","step_index":4,"content":"[Notice] internal"}"#,
            r#"{"type":"PLANNER_RESPONSE","step_index":5,"created_at":"2026-11-26T00:50:20+08:00","model":"gemini-4-neon-xhigh","content":"已查明根因并修复。","input_tokens":800,"output_tokens":60,"cache_read_tokens":6000}"#,
            r#"{"type":"CHECKPOINT","step_index":6,"content":"summary"}"#,
        ]));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert!(msgs[0].text.starts_with("检查会话线程为何读不到 Antigravity"));
        assert!(msgs[0].text.contains("[附带 1 个附件]"));
        let assistant = &msgs[1];
        assert_eq!(assistant.role, "assistant");
        assert_eq!(assistant.model.as_deref(), Some("gemini-4-neon-xhigh"));
        assert_eq!(assistant.reasoning.as_deref(), Some("先读取 session_threads.rs"));
        assert_eq!(assistant.text, "已查明根因并修复。");
        let tokens = assistant.tokens.as_ref().unwrap();
        assert_eq!((tokens.input, tokens.output, tokens.cache_read, tokens.total), (Some(1800), Some(180), Some(11000), Some(12980)));
        assert_eq!(assistant.tools.len(), 2);
        assert_eq!(assistant.tools[0].name, "read_file");
        assert_eq!(assistant.tools[0].summary, "D:/develop/Jolu/backend/src/session_threads.rs");
        assert_eq!(assistant.tools[0].paths, vec!["D:/develop/Jolu/backend/src/session_threads.rs".to_string()]);
        assert_eq!(assistant.tools[0].failed, Some(false));
        assert_eq!(assistant.tools[0].output.as_deref(), Some("Showing lines 1-50 of 1774"));
        assert_eq!(assistant.tools[1].name, "run_command");
        assert_eq!(assistant.tools[1].summary, "Run unit tests");
        assert_eq!((assistant.tools[1].failed, assistant.tools[1].exit_code), (Some(true), Some(1)));
    }

    #[test]
    fn antigravity_session_list_and_messages_read_via_scan_state_transcript() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("jolu-agy-session-{}-{nonce}", std::process::id()));
        let logs = root.join("brain").join("conv-abc").join(".system_generated").join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let tr = logs.join("transcript.jsonl");
        std::fs::write(
            &tr,
            concat!(
                "{\"type\":\"USER_INPUT\",\"step_index\":0,\"created_at\":\"2026-11-26T00:50:00+08:00\",\"content\":\"<USER_REQUEST>\\n排查 Antigravity 会话读取\\n</USER_REQUEST>\"}\n",
                "{\"type\":\"PLANNER_RESPONSE\",\"step_index\":1,\"created_at\":\"2026-11-26T00:50:10+08:00\",\"model\":\"gemini-4-neon-xhigh\",\"content\":\"已经修复完成。\"}\n",
            ),
        )
        .unwrap();
        let sid = "native:antigravity:default:conv-abc";
        let conn = jolu_store();
        conn.execute(
            "INSERT INTO requests(platform, source, dedup_key, session_id, ts, model, input_tokens, output_tokens, cache_read_tokens, total_tokens, input_known, output_known, cache_read_known, total_known, input_semantics)
             VALUES('antigravity', 'native:antigravity:default', 'k1', ?1, 1795624663996, 'gemini-4-neon-xhigh', 100, 50, 500, 650, 1, 1, 1, 1, 'excludes_cache')",
            [sid],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scan_state(path, offset, mtime, parser_session_id) VALUES(?1, 0, 0, ?2)",
            rusqlite::params![tr.to_string_lossy().as_ref(), sid],
        )
        .unwrap();
        crate::db::set_session_activity(&conn, "antigravity", sid, "done", 1795624663996).unwrap();

        let listed = list(&conn, &SessionFilter { platform: Some("antigravity".into()), ..Default::default() }).unwrap();
        assert_eq!(listed.sessions.len(), 1);
        assert_eq!(listed.files, 1);
        let item = &listed.sessions[0];
        assert!(item.has_source, "Antigravity 会话应标记 has_source=true");
        assert_eq!(item.status, "done");
        assert_eq!(item.title.as_deref(), Some("排查 Antigravity 会话读取"));

        // 支持正文全文检索
        let matched = list(&conn, &SessionFilter { platform: Some("antigravity".into()), search: Some("修复完成".into()), ..Default::default() }).unwrap();
        assert_eq!(matched.sessions.len(), 1);

        let page = messages(&conn, "antigravity", sid, None, 40).unwrap();
        assert!(page.available, "应可读取 Antigravity 消息正文");
        assert_eq!(page.total, 2);
        assert_eq!(page.messages[0].role, "user");
        assert_eq!(page.messages[0].text, "排查 Antigravity 会话读取");
        assert_eq!(page.messages[1].role, "assistant");
        assert_eq!(page.messages[1].text, "已经修复完成。");

        let _ = std::fs::remove_dir_all(root);
    }

    /// 真实本机 Antigravity 对话的只读冒烟（仅显式执行，绝不写入任何文件）。
    #[test]
    #[ignore = "需要本机 Antigravity transcript.jsonl；仅显式只读执行"]
    fn antigravity_local_transcript_readonly_smoke() {
        let home = crate::creds::home().expect("home");
        let roots = crate::antigravity_usage::discover_roots(&home);
        let mut checked = 0usize;
        for root in &roots {
            let brain = root.path.join("brain");
            let Ok(entries) = std::fs::read_dir(&brain) else { continue };
            for entry in entries.flatten() {
                if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let conv_id = entry.file_name().to_string_lossy().into_owned();
                let sid = format!("{}:{conv_id}", root.source_id);
                let Some(path) = crate::antigravity_usage::find_transcript_in(&roots, &sid) else {
                    continue;
                };
                let title = crate::antigravity_usage::read_transcript_title(&path);
                let conn = jolu_store();
                let page = messages(&conn, "antigravity", &sid, None, 40).unwrap();
                println!(
                    "antigravity read-only: conv={conv_id} available={} total={} page={} title_present={}",
                    page.available,
                    page.total,
                    page.messages.len(),
                    title.is_some()
                );
                assert!(page.available, "本机 transcript.jsonl 应可读");
                assert!(page.total > 0, "真实会话应解析出至少 1 条消息");
                assert!(title.is_some(), "应能从首个 USER_INPUT 提取标题");
                checked += 1;
            }
        }
        assert!(checked > 0, "本机应至少找到 1 份 Antigravity transcript.jsonl");
    }
