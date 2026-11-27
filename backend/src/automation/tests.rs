use std::path::Path;
use chrono::{NaiveDate, TimeZone};
use rusqlite::{params, Connection};
use serde_json::Value as Json;
use crate::notify::TaskNotice;
use crate::quota::QuotaState;
    use super::*;
    use chrono::FixedOffset;

    fn shanghai() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    fn ms(tz: &FixedOffset, y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        tz.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp_millis()
    }

    fn sched(freq: &str, time: &str, weekday: Option<u32>, date: Option<&str>) -> Schedule {
        Schedule { freq: freq.into(), time: time.into(), weekday, date: date.map(str::to_string), timezone: "Asia/Shanghai".into() }
    }

    /// 测试用的夏令时时区（2026 年纽约规则）：3 月 8 日 02:00–03:00 本地时间不存在，
    /// 11 月 1 日 01:00–02:00 出现两次。项目不依赖 chrono-tz，这里只实现 next_fire 用到的换算。
    #[derive(Clone, Copy, Debug)]
    struct NewYork2026;

    impl TimeZone for NewYork2026 {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> Self { NewYork2026 }
        fn offset_from_local_date(&self, local: &NaiveDate) -> chrono::LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&local.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, local: &chrono::NaiveDateTime) -> chrono::LocalResult<FixedOffset> {
            let (est, edt) = (FixedOffset::west_opt(5 * 3600).unwrap(), FixedOffset::west_opt(4 * 3600).unwrap());
            let gap = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap().and_hms_opt(2, 0, 0).unwrap();
            let fold = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(1, 0, 0).unwrap();
            let hour = chrono::Duration::hours(1);
            if *local >= gap && *local < gap + hour { return chrono::LocalResult::None; }
            if *local >= fold && *local < fold + hour { return chrono::LocalResult::Ambiguous(edt, est); }
            chrono::LocalResult::Single(if *local >= gap && *local < fold { edt } else { est })
        }
        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, utc: &chrono::NaiveDateTime) -> FixedOffset {
            let start = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap().and_hms_opt(7, 0, 0).unwrap();
            let end = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(6, 0, 0).unwrap();
            if *utc >= start && *utc < end { FixedOffset::west_opt(4 * 3600).unwrap() } else { FixedOffset::west_opt(5 * 3600).unwrap() }
        }
    }

    /// 4.7：夏令时跳过的时刻不补跑，重复的时刻只运行一次（路线图 P9 的约定）。
    #[test]
    fn next_fire_skips_the_spring_gap_and_runs_once_in_the_fall_repeat() {
        let tz = NewYork2026;
        let utc = |y, mo, d, h, mi| chrono::Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp_millis();
        // 3 月 8 日 02:30 不存在：不在 03:30 补跑，直接到 3 月 9 日 02:30（EDT，UTC 06:30）
        let before_gap = utc(2026, 3, 7, 20, 0);
        assert_eq!(next_fire(&tz, &sched("daily", "02:30", None, None), before_gap), Some(utc(2026, 3, 9, 6, 30)));
        // 11 月 1 日 01:30 出现两次：先跑第一次（EDT，UTC 05:30）……
        let before_fold = utc(2026, 10, 31, 16, 0);
        let first = next_fire(&tz, &sched("daily", "01:30", None, None), before_fold).unwrap();
        assert_eq!(first, utc(2026, 11, 1, 5, 30));
        // ……第一次之后不再按第二个 01:30（EST，UTC 06:30）再跑，下一次是 11 月 2 日
        assert_eq!(next_fire(&tz, &sched("daily", "01:30", None, None), first), Some(utc(2026, 11, 2, 6, 30)));
        // 每周任务落在跳过的时刻上：那一周不补，顺延到下一周
        assert_eq!(next_fire(&tz, &sched("weekly", "02:30", Some(7), None), before_gap), Some(utc(2026, 3, 15, 6, 30)));
        // 单次任务定在不存在的时刻：如实给不出时间（保存时 validate 拒绝并提示换时间），不悄悄挪到别的时刻
        assert_eq!(next_fire(&tz, &sched("once", "02:30", None, Some("2026-03-08")), before_gap), None);
    }

    #[test]
    fn next_fire_handles_daily_weekly_and_once() {
        let tz = shanghai();
        // 2026-10-22 是周四
        let now = ms(&tz, 2026, 10, 22, 10, 0);
        assert_eq!(next_fire(&tz, &sched("daily", "09:00", None, None), now), Some(ms(&tz, 2026, 10, 23, 9, 0)));
        assert_eq!(next_fire(&tz, &sched("daily", "18:30", None, None), now), Some(ms(&tz, 2026, 10, 22, 18, 30)));
        assert_eq!(next_fire(&tz, &sched("weekly", "09:00", Some(1), None), now), Some(ms(&tz, 2026, 10, 26, 9, 0)));
        assert_eq!(next_fire(&tz, &sched("weekly", "11:00", Some(4), None), now), Some(ms(&tz, 2026, 10, 22, 11, 0)));
        assert_eq!(next_fire(&tz, &sched("once", "09:00", None, Some("2026-10-22")), now), None);
        assert_eq!(next_fire(&tz, &sched("once", "09:00", None, Some("2026-10-24")), now), Some(ms(&tz, 2026, 10, 24, 9, 0)));
    }

    #[test]
    fn editing_task_preserves_existing_run_identity_and_execution() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_700_000_000_000;
        let mut original = input("quota");
        original.model = Some("original-model".into());
        original.skills = vec!["original-skill".into()];
        original.wait_deadline = Some(now + 100_000);
        let task_id = create_task(&conn, &original, now).unwrap();
        let run_id = active_run(&conn, task_id).unwrap().unwrap().id;
        let mut edited = original.clone();
        edited.name = "修改后的任务".into();
        edited.platform = if original.platform == "claude" { "codex".into() } else { "claude".into() };
        edited.connection_id = "different-account".into();
        edited.cwd = std::env::current_dir().unwrap().display().to_string();
        edited.prompt = "新指令只用于新运行".into();
        edited.skills = vec!["new-skill".into()];
        edited.model = Some("new-model".into());
        edited.auto_resume = false;
        edited.wait_deadline = Some(now + 10);
        update_task(&conn, task_id, &edited, now + 1).unwrap();
        let detail = run_detail(&conn, run_id).unwrap();
        assert_eq!(detail.run.platform, original.platform);
        assert_eq!(detail.run.connection_id, original.connection_id);
        assert_eq!(detail.run.task_name, original.name);
        let config = detail.task.unwrap();
        let task_view = task_detail(&conn, task_id).unwrap();
        assert_eq!(task_view.item.task.prompt, edited.prompt, "编辑继续读取当前任务");
        assert_eq!(task_view.execution.unwrap().prompt, original.prompt, "执行详情读取运行快照");
        assert_eq!(config.cwd, original.cwd);
        assert_eq!(config.prompt, original.prompt);
        assert_eq!(config.skills, original.skills);
        assert_eq!(config.model, original.model);
        assert!(config.auto_resume);
        assert_eq!(due_checks(&conn, now + 20).unwrap(), (vec![original.connection_id], vec![]));
        assert!(apply_account_check(&conn, &config.connection_id, None, now + 20, &QuotaVerdict::simple("available", "可用"), now + 21).unwrap());
        assert!(claim_run(&conn, run_id).unwrap());
        let progress = Progress { session_id: Some("old-session".into()), ..Progress::default() };
        finish(&conn, run_id, &Outcome::Limit { resets_at: Some(now + 1000), message: "usage limit".into() }, &progress, now + 22).unwrap();
        let resumed = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(resumed.status, "waiting", "编辑不改变已有运行的自动续跑选项");
        assert_eq!(resumed.session_id.as_deref(), Some("old-session"));
    }

    #[test]
    fn editing_task_keeps_history_and_next_run_uses_new_configuration() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_700_000_000_000;
        let original = input("now");
        let task_id = create_task(&conn, &original, now).unwrap();
        let old_run = active_run(&conn, task_id).unwrap().unwrap().id;
        finish(&conn, old_run, &Outcome::Completed, &Progress::default(), now + 1).unwrap();
        let mut edited = original.clone();
        edited.connection_id = "new-account".into();
        edited.prompt = "新运行指令".into();
        update_task(&conn, task_id, &edited, now + 2).unwrap();
        act(&conn, task_id, "run", now + 3).unwrap();
        let new_run = active_run(&conn, task_id).unwrap().unwrap().id;
        assert_eq!(run_detail(&conn, old_run).unwrap().task.unwrap().prompt, original.prompt);
        assert_eq!(run_detail(&conn, new_run).unwrap().task.unwrap().prompt, edited.prompt);
        let filter = RunFilter { range: "all".into(), status: None, connection: Some(original.connection_id), task: None };
        let history = runs(&conn, &filter, 1, now + 4).unwrap();
        assert_eq!(history.rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![old_run]);
    }

    #[test]
    fn editing_running_task_keeps_original_account_queue_and_cwd_lock() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_700_000_000_000;
        let running = queued(&conn, "c1", r"D:\original", true, now);
        set_status(&conn, running, "running");
        let same_account = queued(&conn, "c1", r"D:\other", true, now + 1);
        let same_cwd = queued(&conn, "c2", r"D:\original", false, now + 2);
        let task_id = load_run(&conn, running).unwrap().unwrap().task_id;
        let mut edited = input("now");
        edited.connection_id = "c2".into();
        edited.serial_queue = false;
        update_task(&conn, task_id, &edited, now + 3).unwrap();
        assert!(startable(&conn).unwrap().is_empty(), "执行中的原账号串行位置和原目录锁不能被编辑释放");
        let waiting = load_run(&conn, same_account).unwrap().unwrap();
        assert_eq!(queue_ahead(&conn, &waiting).unwrap(), 1);
        let queue = task_detail(&conn, task_id).unwrap().queue;
        assert_eq!(queue.len(), 2, "任务详情按当前运行的原账号列队列");
        finish(&conn, running, &Outcome::Completed, &Progress::default(), now + 4).unwrap();
        assert_eq!(startable(&conn).unwrap(), vec![same_account, same_cwd]);
    }

    #[test]
    fn legacy_runs_keep_raw_history_without_guessing_execution_config() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_700_000_000_000;
        let task_id = create_task(&conn, &input("now"), now).unwrap();
        let old_run = active_run(&conn, task_id).unwrap().unwrap().id;
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2, exec_ms=500, account='原账号', session_id='original-session', result='原结果' WHERE id=?1", params![old_run, now - 1000]).unwrap();
        // 模拟发布前的真实表：没有快照列；重复迁移不补猜测参数、不改结果和会话。
        conn.execute_batch("ALTER TABLE automation_runs DROP COLUMN task_snapshot").unwrap();
        ensure_schema(&conn).unwrap();
        ensure_schema(&conn).unwrap();
        assert!(startable(&conn).unwrap().is_empty());
        assert!(!claim_run(&conn, old_run).unwrap());
        recover(&conn, now + 1).unwrap();
        recover(&conn, now + 2).unwrap();
        let detail = run_detail(&conn, old_run).unwrap();
        assert!(detail.task.is_none());
        assert_eq!(detail.run.platform, "unknown");
        assert_eq!(detail.run.connection_id, "");
        assert_eq!(detail.run.account.as_deref(), Some("原账号"));
        assert_eq!(detail.run.session_id.as_deref(), Some("original-session"));
        assert_eq!(detail.run.result.as_deref(), Some("原结果"));
        assert_eq!(detail.run.status, "attention");
        assert_eq!(detail.run.exec_started_at, None, "迁移后不能继续累计运行时间");
        assert_eq!(detail.run.exec_ms, 500);
        assert_eq!(detail.run.block_kind.as_deref(), Some("snapshot_missing"));
        assert_eq!(detail.events.iter().filter(|e| e.title == "原始执行配置未知").count(), 1);
        assert!(act(&conn, task_id, "retry", now + 3).unwrap_err().contains("没有原始执行配置"));
        assert!(due_checks(&conn, now + 4).unwrap().0.is_empty());
        act(&conn, task_id, "cancel", now + 4).unwrap();
        let cancelled = load_run(&conn, old_run).unwrap().unwrap();
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(cancelled.exec_started_at, None);
        assert_eq!(cancelled.exec_ms, 500);
        conn.execute("UPDATE automation_runs SET status='completed' WHERE id=?1", [old_run]).unwrap();
        recover(&conn, now + 5).unwrap();
        assert_eq!(load_run(&conn, old_run).unwrap().unwrap().status, "completed");
    }

    #[test]
    fn quota_verdict_separates_blocked_unknown_and_reauth() {
        let w = |key: &str, used: Option<f64>, resets: Option<&str>| crate::quota::QuotaWindow {
            key: key.into(), window_name: format!("{key} 窗口"), used_percent: used, amount_text: None, remaining_text: None, resets_at: resets.map(str::to_string),
        };
        let blocked = verdict(&QuotaState::Ok { windows: vec![w("5h", Some(100.0), Some("2026-10-22T10:30:00Z")), w("7d", Some(58.0), None)], plan: None, account: None });
        assert_eq!(blocked.state, "blocked");
        assert_eq!(blocked.resets_at, parse_reset("2026-10-22T10:30:00Z"));
        assert!(blocked.windows[0].blocking && !blocked.windows[1].blocking);
        let unknown_reset = verdict(&QuotaState::Ok { windows: vec![w("7d", Some(100.0), None)], plan: None, account: None });
        assert_eq!((unknown_reset.state.as_str(), unknown_reset.resets_at), ("blocked", None));
        assert_eq!(verdict(&QuotaState::Ok { windows: vec![w("5h", Some(40.0), None)], plan: None, account: None }).state, "available");
        assert_eq!(verdict(&QuotaState::Failed { reason: "网络".into() }).state, "unknown");
        assert_eq!(verdict(&QuotaState::Unauthorized { reason: "过期".into() }).state, "reauth");
        assert_eq!(verdict(&QuotaState::Unsupported { reason: "API Key".into() }).state, "unsupported");
    }

    #[test]
    fn claude_stream_is_parsed_and_classified() {
        let mut p = Progress::default();
        p.feed("claude", r#"{"type":"system","subtype":"init","session_id":"s-1"}"#);
        let logs = p.feed("claude", r#"{"type":"assistant","message":{"content":[{"type":"text","text":"开始检查\n第二行"},{"type":"tool_use","name":"Edit","input":{"file_path":"src/LoginForm.tsx"}}]}}"#);
        assert_eq!(logs, vec!["开始检查".to_string(), "调用 Edit · src/LoginForm.tsx".to_string()]);
        p.feed("claude", r#"{"type":"result","subtype":"success","is_error":false,"result":"任务完成","session_id":"s-1","usage":{"input_tokens":100,"cache_read_input_tokens":900,"cache_creation_input_tokens":0,"output_tokens":50}}"#);
        assert_eq!(p.session_id.as_deref(), Some("s-1"));
        assert_eq!((p.input, p.cache_read, p.output), (Some(1000), Some(900), Some(50)));
        assert_eq!(p.files, vec!["src/LoginForm.tsx".to_string()]);
        assert_eq!(classify(&p, true, "", false), Outcome::Completed);

        let mut limited = Progress::default();
        limited.feed("claude", r#"{"type":"result","subtype":"success","is_error":true,"result":"Claude AI usage limit reached|1761300000"}"#);
        assert_eq!(classify(&limited, false, "", false), Outcome::Limit { resets_at: Some(1_761_300_000_000), message: "Claude AI usage limit reached|1761300000".into() });
        assert_eq!(classify(&limited, false, "", true), Outcome::Cancelled);
    }

    #[test]
    fn codex_events_are_parsed_and_classified() {
        let mut p = Progress::default();
        p.feed("codex", r#"{"type":"thread.started","thread_id":"t-9"}"#);
        p.feed("codex", r#"{"type":"item.completed","item":{"id":"i1","type":"file_change","changes":[{"path":"a.ts","kind":"update"}]}}"#);
        p.feed("codex", r#"{"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"已完成"}}"#);
        p.feed("codex", r#"{"type":"turn.completed","usage":{"input_tokens":1200,"cached_input_tokens":800,"output_tokens":90,"reasoning_output_tokens":10}}"#);
        assert_eq!(p.session_id.as_deref(), Some("t-9"));
        assert_eq!((p.input, p.cache_read, p.output), (Some(1200), Some(800), Some(90)));
        assert_eq!(p.result.as_deref(), Some("已完成"));
        assert_eq!(classify(&p, true, "", false), Outcome::Completed);

        let mut auth = Progress::default();
        auth.feed("codex", r#"{"type":"turn.failed","error":{"message":"401 Unauthorized: please log in again"}}"#);
        assert!(matches!(classify(&auth, false, "", false), Outcome::Reauth(_)));
        let failed = Progress::default();
        assert_eq!(classify(&failed, false, "boom", false), Outcome::Failed("boom".into()));
        assert_eq!(build_args("codex", Some("gpt-5"), Some("t-9")), vec!["exec", "--json", "-m", "gpt-5", "resume", "t-9", "-"]);
        assert_eq!(build_args("claude", None, Some("s-1")), vec!["-p", "--output-format", "stream-json", "--verbose", "--resume", "s-1"]);
    }

    fn input(launch: &str) -> TaskInput {
        TaskInput {
            name: "完善登录页面".into(),
            platform: "claude".into(),
            connection_id: "c1".into(),
            model: Some("claude-sonnet-4-5".into()),
            cwd: std::env::temp_dir().display().to_string(),
            session_id: None,
            prompt: "完善表单校验".into(),
            skills: vec![],
            launch: launch.into(),
            schedule: None,
            auto_resume: true,
            wait_deadline: None,
            serial_queue: true,
        }
    }

    #[test]
    fn waiting_connections_list_accounts_with_unpaused_waiting_runs() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        assert!(waiting_connections(&conn).unwrap().is_empty());
        let first = create_task(&conn, &input("quota"), 1_000).unwrap();
        let mut other = input("quota");
        other.connection_id = "c2".into();
        create_task(&conn, &other, 1_001).unwrap();
        create_task(&conn, &input("now"), 1_002).unwrap();
        let mut ids = waiting_connections(&conn).unwrap();
        ids.sort();
        assert_eq!(ids, ["c1", "c2"], "同一账号去重；立即执行的任务不算等额度");
        conn.execute("UPDATE automation_tasks SET state='paused' WHERE id=?1", [first]).unwrap();
        assert_eq!(waiting_connections(&conn).unwrap(), ["c2"], "暂停的任务不算在等");
    }

    #[test]
    fn run_waits_for_quota_resumes_and_queues_per_account() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let first = create_task(&conn, &input("now"), now).unwrap();
        let second = create_task(&conn, &input("now"), now + 1).unwrap();
        let queue = startable(&conn).unwrap();
        assert_eq!(queue.len(), 1, "同一账号只启动队首");
        let run_id = queue[0];
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run_id, now]).unwrap();
        let p = Progress { session_id: Some("s-1".into()), input: Some(10), output: Some(2), ..Default::default() };
        finish(&conn, run_id, &Outcome::Limit { resets_at: Some(now + 60_000), message: "usage limit".into() }, &p, now + 5_000).unwrap();
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!((run.status.as_str(), run.exec_ms, run.waits, run.resume_pending), ("waiting", 5_000, 1, true));
        assert!(startable(&conn).unwrap().is_empty(), "等待额度的运行挡住同账号后续任务");
        assert_eq!(queue_ahead(&conn, &active_run(&conn, second).unwrap().unwrap()).unwrap(), 1);

        let failed = QuotaVerdict::simple("unknown", "网络不可用");
        apply_check(&conn, run_id, &failed, now + 70_000).unwrap();
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!((run.status.as_str(), run.block_kind.as_deref()), ("waiting", Some("check_failed")), "检查失败不当作额度耗尽");
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(apply_check(&conn, run_id, &ok, now + 100_000).unwrap());
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!((run.status.as_str(), run.wait_ms), ("queued", 95_000));
        assert_eq!(startable(&conn).unwrap(), vec![run_id]);

        act(&conn, first, "cancel", now + 200_000).unwrap();
        assert_eq!(load_run(&conn, run_id).unwrap().unwrap().status, "cancelled");
        assert_eq!(startable(&conn).unwrap().len(), 1, "取消后轮到下一个任务");
    }

    /* ── 任务 2.6：关闭同账号排队（D-09） ── */

    /// 新建一个立即执行的任务，返回它的运行；工作目录直接写库（`startable` 不访问磁盘）
    fn queued(conn: &Connection, connection: &str, cwd: &str, serial: bool, now: i64) -> i64 {
        let mut i = input("now");
        i.connection_id = connection.into();
        i.serial_queue = serial;
        let task = create_task(conn, &i, now).unwrap();
        // 队列夹具使用不必存在的合成路径；同时修改新运行快照，模拟创建时已选定该路径。
        conn.execute("UPDATE automation_runs SET task_snapshot=json_set(task_snapshot, '$.cwd', ?2) WHERE task_id=?1", params![task, cwd]).unwrap();
        active_run(conn, task).unwrap().unwrap().id
    }

    fn set_status(conn: &Connection, run: i64, status: &str) {
        conn.execute("UPDATE automation_runs SET status=?2 WHERE id=?1", params![run, status]).unwrap();
    }

    #[test]
    fn startable_skips_serial_for_opt_out() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let a = queued(&conn, "c1", r"D:\work\a", true, now);
        let b = queued(&conn, "c1", r"D:\work\b", true, now + 1);
        let c = queued(&conn, "c1", r"D:\work\c", false, now + 2);
        set_status(&conn, a, "running");
        assert_eq!(startable(&conn).unwrap(), vec![c], "排队任务等 A，关闭排队的 C 直接放行");
        set_status(&conn, c, "running");
        set_status(&conn, a, "completed");
        assert_eq!(startable(&conn).unwrap(), vec![b], "A 完成后轮到排队的 B，不受并行的 C 影响");
        assert_eq!(queue_ahead(&conn, &load_run(&conn, b).unwrap().unwrap()).unwrap(), 0, "队列位置只统计排队任务");
    }

    #[test]
    fn same_cwd_always_serial() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let a = queued(&conn, "c1", r"D:\Work\Proj", true, now);
        let c = queued(&conn, "c1", "d:/work/proj/", false, now + 1);
        let _d = queued(&conn, "c1", "d:/work/proj", false, now + 2);
        set_status(&conn, a, "running");
        assert!(startable(&conn).unwrap().is_empty(), "同一工作目录（大小写、斜杠不同）不并行");
        set_status(&conn, a, "completed");
        assert_eq!(startable(&conn).unwrap(), vec![c], "同一目录一次只放行一个");
    }

    #[test]
    fn parallel_cap_per_account() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        for i in 0..3 {
            let run = queued(&conn, "c1", &format!(r"D:\work\p{i}"), false, now + i);
            set_status(&conn, run, "running");
        }
        let _fourth = queued(&conn, "c1", r"D:\work\p3", false, now + 3);
        let other = queued(&conn, "c2", r"D:\work\p4", false, now + 4);
        assert_eq!(startable(&conn).unwrap(), vec![other], "同账号最多 3 个并行，其他账号不受影响");
    }

    #[test]
    fn quota_wait_blocks_whole_account() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let r1 = queued(&conn, "c1", r"D:\work\r1", false, now);
        let r2 = queued(&conn, "c1", r"D:\work\r2", false, now + 1);
        set_status(&conn, r1, "running");
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![r2, now]).unwrap();
        let p = Progress { session_id: Some("s-2".into()), ..Default::default() };
        finish(&conn, r2, &Outcome::Limit { resets_at: Some(now + 60_000), message: "usage limit".into() }, &p, now + 5_000).unwrap();
        assert_eq!(load_run(&conn, r2).unwrap().unwrap().status, "waiting");
        let _serial = queued(&conn, "c1", r"D:\work\s", true, now + 2);
        let _parallel = queued(&conn, "c1", r"D:\work\n", false, now + 3);
        let other = queued(&conn, "c2", r"D:\work\x", false, now + 4);
        assert_eq!(startable(&conn).unwrap(), vec![other], "额度等待挡住同账号全部新任务，其他账号照常");
        assert_eq!(load_run(&conn, r1).unwrap().unwrap().status, "running", "正在运行的任务跑完当前这一轮");
    }

    #[test]
    fn recovery_releases_serial_head_first() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let n1 = queued(&conn, "c1", r"D:\work\n1", false, now);
        let mut waiting = input("quota");
        waiting.connection_id = "c1".into();
        let head_task = create_task(&conn, &waiting, now + 1).unwrap();
        let head = active_run(&conn, head_task).unwrap().unwrap().id;
        let _b = queued(&conn, "c1", r"D:\work\b", true, now + 2);
        let n2 = queued(&conn, "c1", r"D:\work\n2", false, now + 3);
        let _n3 = queued(&conn, "c1", r"D:\work\n3", false, now + 4);
        assert!(startable(&conn).unwrap().is_empty(), "等待额度期间整个账号不放行");
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(apply_check(&conn, head, &ok, now + 10_000).unwrap());
        assert_eq!(startable(&conn).unwrap(), vec![head, n1, n2], "先放行排队队首，再放行并行任务，总数不超过 3");
    }

    #[test]
    fn quota_checks_merge_per_account() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let mut w = input("quota");
        w.serial_queue = false;
        let w1 = active_run(&conn, create_task(&conn, &w, now).unwrap()).unwrap().unwrap().id;
        let w2 = active_run(&conn, create_task(&conn, &w, now + 1).unwrap()).unwrap().unwrap().id;
        w.connection_id = "c2".into();
        let w3 = active_run(&conn, create_task(&conn, &w, now + 2).unwrap()).unwrap().unwrap().id;
        let (due, _) = due_checks(&conn, now + 10).unwrap();
        assert_eq!(due, vec!["c1".to_string(), "c2".to_string()], "同账号只检查一次");
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(apply_account_check(&conn, "c1", None, now + 10, &ok, now + 20).unwrap());
        let status = |id| load_run(&conn, id).unwrap().unwrap().status;
        assert_eq!((status(w1).as_str(), status(w2).as_str(), status(w3).as_str()), ("queued", "queued", "waiting"), "一次检查结果写入该账号全部等待中的运行");
    }

    #[test]
    fn task_detail_queue_lists_serial_only() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let a = queued(&conn, "c1", r"D:\work\a", true, now);
        let _c = queued(&conn, "c1", r"D:\work\c", false, now + 1);
        let _b = queued(&conn, "c1", r"D:\work\b", true, now + 2);
        set_status(&conn, a, "running");
        let task_a = load_run(&conn, a).unwrap().unwrap().task_id;
        let names: Vec<(i64, String)> = task_detail(&conn, task_a).unwrap().queue.into_iter().map(|q| (q.task_id, q.status)).collect();
        assert_eq!(names, vec![(1, "running".to_string()), (3, "queued".to_string())], "同账号队列不列关闭排队的任务");
    }

    #[test]
    fn same_cwd_serial_across_accounts() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let a = queued(&conn, "c1", r"D:\Work\Shared", true, now);
        let _b = queued(&conn, "c2", "d:/work/shared/", false, now + 1);
        set_status(&conn, a, "running");
        assert!(startable(&conn).unwrap().is_empty(), "不同账号在同一工作目录（大小写、斜杠不同）也不同时运行");
    }

    #[test]
    fn parallel_cap_counts_running_serial() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let s = queued(&conn, "c1", r"D:\work\s", true, now);
        let n1 = queued(&conn, "c1", r"D:\work\n1", false, now + 1);
        let n2 = queued(&conn, "c1", r"D:\work\n2", false, now + 2);
        for run in [s, n1, n2] { set_status(&conn, run, "running"); }
        let _fourth = queued(&conn, "c1", r"D:\work\n3", false, now + 3);
        assert!(startable(&conn).unwrap().is_empty(), "1 个排队任务 + 2 个并行任务在运行时，第 4 个并行任务不放行");
    }

    #[test]
    fn stale_check_skips_runs_waiting_after_query_start() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let mut w = input("quota");
        w.serial_queue = false;
        let before = active_run(&conn, create_task(&conn, &w, now).unwrap()).unwrap().unwrap().id;
        let checked_from = now + 10;
        let after = active_run(&conn, create_task(&conn, &w, now + 20).unwrap()).unwrap().unwrap().id;
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(apply_account_check(&conn, "c1", None, checked_from, &ok, now + 30).unwrap());
        let status = |id| load_run(&conn, id).unwrap().unwrap().status;
        assert_eq!((status(before).as_str(), status(after).as_str()), ("queued", "waiting"), "查询开始后才进入等待的运行不使用这次的旧结果");
        assert!(!apply_account_check(&conn, "c1", Some(after), checked_from, &ok, now + 40).unwrap(), "手动检查的运行同样只认查询开始前的等待");
        assert_eq!(status(after), "waiting");
    }

    #[test]
    fn paused_waiting_run_neither_blocks_nor_wakes() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let paused_task = create_task(&conn, &input("quota"), now).unwrap();
        let paused = active_run(&conn, paused_task).unwrap().unwrap().id;
        act(&conn, paused_task, "pause", now + 1).unwrap();
        let q = queued(&conn, "c1", r"D:\work\q", true, now + 2);
        assert_eq!(startable(&conn).unwrap(), vec![q], "暂停任务的等待运行不挡账号");
        let (due, _) = due_checks(&conn, now + 10).unwrap();
        assert!(due.is_empty(), "暂停任务的等待运行不触发额度检查");
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(!apply_account_check(&conn, "c1", None, now + 10, &ok, now + 20).unwrap());
        assert_eq!(load_run(&conn, paused).unwrap().unwrap().status, "waiting", "合并检查不唤醒暂停任务的等待运行");
    }

    #[test]
    fn serial_queue_column_migrates_with_default_on() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE automation_tasks (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, platform TEXT NOT NULL, connection_id TEXT NOT NULL, model TEXT, cwd TEXT NOT NULL, session_id TEXT, prompt TEXT NOT NULL, skills TEXT NOT NULL DEFAULT '[]', launch TEXT NOT NULL, schedule TEXT, auto_resume INTEGER NOT NULL DEFAULT 1, wait_deadline INTEGER, state TEXT NOT NULL DEFAULT 'active', next_run_at INTEGER, missed_at INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             INSERT INTO automation_tasks (name, platform, connection_id, cwd, prompt, launch, created_at, updated_at) VALUES ('旧任务', 'claude', 'c1', 'D:/work', '执行', 'now', 1, 1);",
        ).unwrap();
        ensure_schema(&conn).unwrap();
        ensure_schema(&conn).unwrap();
        assert!(load_task(&conn, 1).unwrap().unwrap().serial_queue, "旧任务默认保持同账号排队");
        let parsed: TaskInput = serde_json::from_value(serde_json::json!({
            "name": "a", "platform": "claude", "connection_id": "c1", "model": null, "cwd": "D:/work", "session_id": null,
            "prompt": "p", "launch": "now", "schedule": null, "auto_resume": true, "wait_deadline": null
        })).unwrap();
        assert!(parsed.serial_queue, "未传字段时默认开启");
    }

    /// 额度耗尽：Claude Code 与 Codex 各一段输出喂给 `classify`，任务进入等待并记下重置时间。
    /// 样例出处（2026-10-22 核对，本机 Claude Code 2.1.308、codex-cli 0.162.0）：
    /// - Claude：`rate_limit_event` 的字段与 `status: "rejected"`（已到限额）、重置时间为 Unix 秒，见官方文档
    ///   https://code.claude.com/docs/en/agent-sdk/python 的 RateLimitEvent / RateLimitInfo（CLI 原始字段名 `resetsAt`，
    ///   取自 claude.exe 内置的 `{status, resetsAt, rateLimitType}` 序列化代码）；提示文本 `You've hit your <限额名> · resets <时间>`
    ///   与限额名 `session limit` 取自 claude.exe 内置字符串。旧格式 `Claude AI usage limit reached|<时间戳>` 在 2.1.308 中已不存在。
    ///   整行 JSON（result 的 `is_error: true`）按上述字段构造。
    /// - Codex：提示文本 `You’ve hit your usage limit. Upgrade to Pro (…), visit … to purchase more credits or try again at <时间>.`
    ///   与跨天时间格式 `%b %-d{st|nd|rd|th}, %Y %-I:%M %p`（本机时区）取自 codex.exe 内置字符串；
    ///   JSONL 外壳（`error`、`turn.failed`）按 `Progress::feed` 现有的解析规则构造。
    #[test]
    fn classify_tells_quota_rate_limit_and_balance_apart() {
        let p = Progress::default();
        let rate = classify(&p, false, "API Error: 429 Too Many Requests", false);
        assert!(matches!(rate, Outcome::RateLimited { .. }), "{rate:?}");
        let quota = classify(&p, false, "Claude AI usage limit reached|1761300000", false);
        assert!(matches!(quota, Outcome::Limit { .. }), "额度用完的说法优先：{quota:?}");
        let weekly = classify(&p, false, "You've hit your weekly limit. 429", false);
        assert!(matches!(weekly, Outcome::Limit { .. }), "{weekly:?}");
        let balance = classify(&p, false, "Your credit balance is too low to access the Anthropic API.", false);
        assert_eq!(balance, Outcome::Balance("Your credit balance is too low to access the Anthropic API.".into()));
        assert!(matches!(classify(&p, false, "余额不足，请充值", false), Outcome::Balance(_)));
    }

    #[test]
    fn rate_limited_runs_retry_soon_and_balance_needs_attention() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        create_task(&conn, &input("now"), now).unwrap();
        let run_id = startable(&conn).unwrap()[0];
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run_id, now]).unwrap();
        let p = Progress { session_id: Some("s-1".into()), ..Default::default() };
        finish(&conn, run_id, &Outcome::RateLimited { retry_at: None, message: "429".into() }, &p, now + 10).unwrap();
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!((run.status.as_str(), run.block_kind.as_deref()), ("waiting", Some("rate_limit")));
        assert_eq!(run.resets_at, Some(now + 10 + RATE_LIMIT_RETRY_MS), "没给重试时间时一分钟后再试，不等额度重置");

        let mut other = input("now");
        other.connection_id = "c2".into();
        create_task(&conn, &other, now + 1).unwrap();
        let second = startable(&conn).unwrap().into_iter().find(|id| *id != run_id).unwrap();
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![second, now]).unwrap();
        finish(&conn, second, &Outcome::Balance("insufficient balance".into()), &p, now + 20).unwrap();
        let run = load_run(&conn, second).unwrap().unwrap();
        assert_eq!((run.status.as_str(), run.block_kind.as_deref()), ("attention", Some("balance")));
    }

    #[test]
    fn classify_quota_exhausted_enters_waiting() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let running = |task: i64| {
            let run_id = active_run(&conn, task).unwrap().unwrap().id;
            conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run_id, now]).unwrap();
            run_id
        };

        let claude = create_task(&conn, &input("now"), now).unwrap();
        let run_id = running(claude);
        let text = "You've hit your session limit · resets 3pm (Asia/Shanghai)";
        let mut p = Progress::default();
        p.feed("claude", r#"{"type":"system","subtype":"init","session_id":"s-limit"}"#);
        p.feed("claude", r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1792738800,"rateLimitType":"five_hour"},"uuid":"u-1","session_id":"s-limit"}"#);
        p.feed("claude", &serde_json::json!({"type":"result","subtype":"success","is_error":true,"result":text,"session_id":"s-limit"}).to_string());
        let outcome = classify(&p, false, "", false);
        assert_eq!(outcome, Outcome::Limit { resets_at: Some(1_792_738_800_000), message: text.into() });
        finish(&conn, run_id, &outcome, &p, now + 5_000).unwrap();
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(
            (run.status.as_str(), run.block_kind.as_deref(), run.resets_at, run.next_check_at, run.resume_pending, run.session_id.as_deref()),
            ("waiting", Some("quota"), Some(1_792_738_800_000), Some(1_792_738_830_000), true, Some("s-limit")),
        );

        let mut codex_input = input("now");
        codex_input.platform = "codex".into();
        codex_input.connection_id = "c2".into();
        codex_input.model = None;
        let codex = create_task(&conn, &codex_input, now).unwrap();
        let run_id = running(codex);
        let text = "You’ve hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit https://chatgpt.com/settings/usage to purchase more credits or try again at Oct 23rd, 2026 3:05 PM.";
        let mut p = Progress::default();
        p.feed("codex", r#"{"type":"thread.started","thread_id":"t-limit"}"#);
        p.feed("codex", &serde_json::json!({"type":"error","message":text}).to_string());
        p.feed("codex", &serde_json::json!({"type":"turn.failed","error":{"message":text}}).to_string());
        let reset = Local.with_ymd_and_hms(2026, 10, 23, 15, 5, 0).earliest().unwrap().timestamp_millis();
        let outcome = classify(&p, false, "", false);
        assert_eq!(outcome, Outcome::Limit { resets_at: Some(reset), message: text.into() });
        finish(&conn, run_id, &outcome, &p, now + 5_000).unwrap();
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(
            (run.status.as_str(), run.resets_at, run.resume_pending, run.session_id.as_deref()),
            ("waiting", Some(reset), true, Some("t-limit")),
        );
    }

    /// `rate_limit_event` 只在文本没有给出判断时兜底；超额用量仍可用（`overageStatus` 为 allowed / allowed_warning）时会话会继续，
    /// 不算额度耗尽。字段名：官方 RateLimitInfo 的 `overage_status`，CLI 原始输出为 `overageStatus`（claude.exe 2.1.308 内置代码）。
    /// 认证与 5xx 的提示文本为构造样例。
    #[test]
    fn rate_limit_event_does_not_hide_other_failures() {
        let rejected = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1792738800,"rateLimitType":"five_hour"},"uuid":"u-1","session_id":"s-1"}"#;
        let overage = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1792738800,"rateLimitType":"five_hour","overageStatus":"allowed","isUsingOverage":true},"uuid":"u-1","session_id":"s-1"}"#;
        let result = |text: &str| serde_json::json!({"type":"result","subtype":"success","is_error":true,"result":text,"session_id":"s-1"}).to_string();

        // 主额度耗尽后又因认证过期失败：按文本判为需要重新登录
        let mut p = Progress::default();
        p.feed("claude", rejected);
        p.feed("claude", &result("OAuth token has expired. Please run /login"));
        assert!(matches!(classify(&p, false, "", false), Outcome::Reauth(_)));

        // 超额用量可用：不置限额标记；成功即完成，普通失败仍是失败
        let mut p = Progress::default();
        p.feed("claude", overage);
        assert!(!p.rate_limited);
        p.feed("claude", r#"{"type":"result","subtype":"success","is_error":false,"result":"OK","session_id":"s-1"}"#);
        assert_eq!(classify(&p, true, "", false), Outcome::Completed);
        let mut p = Progress::default();
        p.feed("claude", overage);
        p.feed("claude", &result("API Error: 500 Internal server error"));
        assert_eq!(classify(&p, false, "", false), Outcome::Failed("API Error: 500 Internal server error".into()));

        // 文本没有判断依据时，rejected 事件兜底判为额度耗尽并带上重置时间
        let mut p = Progress::default();
        p.feed("claude", rejected);
        p.feed("claude", r#"{"type":"result","subtype":"error_during_execution","is_error":true,"session_id":"s-1"}"#);
        assert_eq!(
            classify(&p, false, "", false),
            Outcome::Limit { resets_at: Some(1_792_738_800_000), message: "执行器返回 error_during_execution".into() },
        );
    }

    /// 样例为真实记录：2026-10-22 16:02 Codex 在非 Git 目录执行时 stderr 的原文（codex-cli 0.162.0）
    #[test]
    fn codex_untrusted_dir_needs_attention() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let mut i = input("now");
        i.platform = "codex".into();
        i.model = None;
        let id = create_task(&conn, &i, now).unwrap();
        let run_id = active_run(&conn, id).unwrap().unwrap().id;
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run_id, now]).unwrap();
        let stderr = "Not inside a trusted directory and --skip-git-repo-check was not specified.";
        let outcome = classify(&Progress::default(), false, stderr, false);
        assert!(!matches!(outcome, Outcome::Failed(_)), "不应归为运行失败：{outcome:?}");
        finish(&conn, run_id, &outcome, &Progress::default(), now + 1_000).unwrap();
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(
            (run.status.as_str(), run.block_kind.as_deref(), run.reason.as_deref(), run.error.as_deref()),
            ("attention", Some("cwd_missing"), Some("Codex 只能在 Git 仓库或已信任的目录中执行。请把工作目录改为 Git 仓库，或先在 Codex 中信任该目录。"), Some(stderr)),
        );
        assert_eq!(run.ended_at, None, "需要处理的运行仍可重新排队");
    }

    #[test]
    fn codex_retry_time_handles_same_day_and_dated_formats() {
        let tz = shanghai();
        let now = ms(&tz, 2026, 10, 22, 14, 0);
        let msg = |at: &str| format!("You’ve hit your usage limit. Upgrade to Plus to continue using Codex (https://chatgpt.com/explore/plus), or try again at {at}.");
        assert_eq!(codex_retry_at(&tz, &msg("3:05 PM"), now), Some(ms(&tz, 2026, 10, 22, 15, 5)));
        // 当天的时刻已经过去时视为次日
        assert_eq!(codex_retry_at(&tz, &msg("9:30 AM"), now), Some(ms(&tz, 2026, 10, 23, 9, 30)));
        // 提示只精确到分钟：恰好在重置的那一分钟内（或刚过几分钟）仍按当天算，不推迟 24 小时
        assert_eq!(codex_retry_at(&tz, &msg("2:00 PM"), now + 30_000), Some(ms(&tz, 2026, 10, 22, 14, 0)));
        assert_eq!(codex_retry_at(&tz, &msg("1:56 PM"), now), Some(ms(&tz, 2026, 10, 22, 13, 56)));
        assert_eq!(codex_retry_at(&tz, &msg("1:50 PM"), now), Some(ms(&tz, 2026, 10, 23, 13, 50)));
        assert_eq!(codex_retry_at(&tz, &msg("Nov 1st, 2026 12:00 AM"), now), Some(ms(&tz, 2026, 11, 1, 0, 0)));
        assert_eq!(codex_retry_at(&tz, "You’ve hit your usage limit. or try again later.", now), None);
        assert!(is_limit("You've hit your weekly limit · resets Oct 27, 9am"));
    }

    #[test]
    fn schedules_fire_skip_overlap_and_mark_missed() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let tz = shanghai();
        let now = ms(&tz, 2026, 10, 22, 9, 0);
        let mut i = input("schedule");
        i.schedule = Some(sched("daily", "09:00", None, None));
        let id = create_task(&conn, &i, now - 3_600_000).unwrap();
        conn.execute("UPDATE automation_tasks SET next_run_at=?2 WHERE id=?1", params![id, now]).unwrap();
        assert!(fire_schedules(&conn, &tz, now + 1_000).unwrap());
        assert!(active_run(&conn, id).unwrap().is_some());
        assert_eq!(load_task(&conn, id).unwrap().unwrap().next_run_at, Some(ms(&tz, 2026, 10, 23, 9, 0)));
        // 次日太晚才发现：标「已错过」，不自动执行
        let late = ms(&tz, 2026, 10, 23, 10, 0);
        conn.execute("UPDATE automation_runs SET status='completed'", []).unwrap();
        fire_schedules(&conn, &tz, late).unwrap();
        let task = load_task(&conn, id).unwrap().unwrap();
        assert_eq!(task.missed_at, Some(ms(&tz, 2026, 10, 23, 9, 0)));
        assert!(active_run(&conn, id).unwrap().is_none());
        recover(&conn, late).unwrap();
    }

    /* ── 任务 2.9：删除任务 ── */

    fn count(conn: &Connection, sql: &str, id: i64) -> i64 {
        conn.query_row(sql, [id], |r| r.get(0)).unwrap()
    }

    #[test]
    fn delete_task_removes_runs_keeps_usage() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        conn.execute_batch("CREATE TABLE requests (id INTEGER PRIMARY KEY, platform TEXT NOT NULL, ts INTEGER NOT NULL); INSERT INTO requests (platform, ts) VALUES ('claude', 1);").unwrap();
        let now = 1_000_000;
        let id = create_task(&conn, &input("now"), now).unwrap();
        let first = active_run(&conn, id).unwrap().unwrap().id;
        set_status(&conn, first, "completed");
        act(&conn, id, "run", now + 10).unwrap();
        let second = active_run(&conn, id).unwrap().unwrap().id;
        add_event(&conn, second, now + 20, "log", "执行日志", None).unwrap();
        let other = create_task(&conn, &input("now"), now + 30).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM automation_runs WHERE task_id = ?1", id), 2);
        assert!(count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id IN (SELECT id FROM automation_runs WHERE task_id = ?1)", id) >= 3);

        delete_task(&mut conn, id).unwrap();
        assert!(load_task(&conn, id).unwrap().is_none());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM automation_runs WHERE task_id = ?1", id), 0);
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM automation_events WHERE run_id IN (?1, ?2)", params![first, second], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "两条运行的事件都已删除");
        let usage: i64 = conn.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0)).unwrap();
        assert_eq!(usage, 1, "用量统计保留");
        assert!(load_task(&conn, other).unwrap().is_some() && active_run(&conn, other).unwrap().is_some(), "其他任务不受影响");
        assert_eq!(delete_task(&mut conn, id).unwrap_err(), "任务不存在");
    }

    #[test]
    fn delete_running_task_rejected() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let id = create_task(&conn, &input("now"), now).unwrap();
        let run = active_run(&conn, id).unwrap().unwrap().id;
        set_status(&conn, run, "running");
        let events = count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id = ?1", run);
        assert_eq!(delete_task(&mut conn, id).unwrap_err(), "任务正在运行，请先停止");
        assert!(load_task(&conn, id).unwrap().is_some());
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "running", "数据不变");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id = ?1", run), events);
    }

    #[test]
    fn delete_waiting_task_cancels_wait() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let waiting = create_task(&conn, &input("quota"), now).unwrap();
        let wait_run = active_run(&conn, waiting).unwrap().unwrap().id;
        let next = queued(&conn, "c1", r"D:\work\next", true, now + 1);
        assert!(startable(&conn).unwrap().is_empty(), "等待额度挡住同账号");
        assert_eq!(due_checks(&conn, now + 10).unwrap().0, vec!["c1".to_string()]);

        delete_task(&mut conn, waiting).unwrap();
        assert!(load_run(&conn, wait_run).unwrap().is_none());
        assert_eq!(startable(&conn).unwrap(), vec![next], "删除后不再挡住账号，也不会被启动");
        assert!(due_checks(&conn, now + 20).unwrap().0.is_empty(), "等待检查列表里不再出现");
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(!apply_account_check(&conn, "c1", Some(wait_run), now, &ok, now + 30).unwrap(), "拿旧 id 检查不会复活运行");

        let queued_task = load_run(&conn, next).unwrap().unwrap().task_id;
        delete_task(&mut conn, queued_task).unwrap();
        assert!(startable(&conn).unwrap().is_empty(), "排队中的任务删除后不再启动");
        assert!(!claim_run(&conn, next).unwrap(), "调度线程拿着旧 id 也认领不到");
    }

    #[test]
    fn claim_run_only_takes_queued() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let run = queued(&conn, "c1", r"D:\work\a", true, 1_000_000);
        assert!(claim_run(&conn, run).unwrap());
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "running");
        assert!(!claim_run(&conn, run).unwrap(), "已认领的运行不重复启动");

        // 续跑：执行中额度耗尽 → 等待（resume_pending）→ 额度恢复转回排队，沿用原运行 ID，可以再次认领
        let now = 1_000_000;
        conn.execute("UPDATE automation_runs SET exec_started_at=?2 WHERE id=?1", params![run, now]).unwrap();
        let p = Progress { session_id: Some("s-1".into()), ..Default::default() };
        finish(&conn, run, &Outcome::Limit { resets_at: Some(now + 60_000), message: "usage limit".into() }, &p, now + 5_000).unwrap();
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(apply_check(&conn, run, &ok, now + 100_000).unwrap());
        let resumed = load_run(&conn, run).unwrap().unwrap();
        assert_eq!((resumed.status.as_str(), resumed.resume_pending), ("queued", true));
        assert!(claim_run(&conn, run).unwrap(), "续跑的运行能被认领");
    }

    #[test]
    fn claim_run_skips_paused_task() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let run = queued(&conn, "c1", r"D:\work\a", true, now);
        let task = load_run(&conn, run).unwrap().unwrap().task_id;
        act(&conn, task, "pause", now + 1).unwrap();
        assert!(!claim_run(&conn, run).unwrap(), "暂停后认领失败");
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "queued", "仍保持排队，恢复后再启动");
        act(&conn, task, "resume", now + 2).unwrap();
        assert!(claim_run(&conn, run).unwrap(), "恢复后可以认领");
    }

    #[test]
    fn set_attention_ignores_cancelled_and_deleted_runs() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let run = queued(&conn, "c1", r"D:\work\a", true, now);
        let task = load_run(&conn, run).unwrap().unwrap().task_id;
        act(&conn, task, "cancel", now + 1).unwrap();
        let events = count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id = ?1", run);
        set_attention(&conn, run, "tool_missing", "未检测到执行器", now + 2).unwrap();
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "cancelled", "已取消的运行不改回需要处理");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id = ?1", run), events, "也不写事件");

        let other = queued(&conn, "c1", r"D:\work\b", true, now + 3);
        let other_task = load_run(&conn, other).unwrap().unwrap().task_id;
        delete_task(&mut conn, other_task).unwrap();
        set_attention(&conn, other, "tool_missing", "未检测到执行器", now + 4).unwrap();
        assert!(load_run(&conn, other).unwrap().is_none());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id = ?1", other), 0, "已删除的运行不留孤儿事件");
    }

    #[test]
    fn stop_requested_before_register_kills_on_register() {
        let mut procs = Procs::default();
        procs.clear_stop(7);
        assert_eq!(procs.request_stop(7), None, "进程还没登记：只留下停止标记");
        assert!(procs.register(7, 4242), "登记时发现停止请求，需要立即结束");
        assert!(procs.exited(7), "退出时按已停止收尾");
        assert!(!procs.exited(7), "标记随退出清除");

        // 正常顺序：先登记再停止，返回进程 ID 由调用方结束
        assert!(!procs.register(8, 5151));
        assert_eq!(procs.request_stop(8), Some(5151));
        assert!(procs.exited(8));

        // 上一轮遗留的标记（续跑沿用运行 ID）在认领前清掉，不会误杀新一轮
        procs.request_stop(9);
        procs.clear_stop(9);
        assert!(!procs.register(9, 6161));
        assert!(!procs.exited(9));
    }

    /* ── 任务 3.3：自动化选择 Skills（D-08） ── */

    fn option(name: &str) -> crate::skills::SkillOption {
        crate::skills::SkillOption { name: name.into(), description: None, scope: "global".into(), project: None }
    }

    #[test]
    fn skill_prefix_formats() {
        let names = vec!["a".to_string(), "b".to_string()];
        assert_eq!(skill_prefix("codex", &names), "$a $b\n\n");
        assert_eq!(skill_prefix("claude", &names), "本任务请使用以下 Skills：a、b\n\n");
        assert_eq!(skill_prefix("claude", &[]), "");
        assert_eq!(skill_prefix("codex", &[]), "");
    }

    #[test]
    fn validate_rejects_too_many_skills() {
        let now = 1_000_000;
        let mut i = input("now");
        i.skills = (0..11).map(|n| format!("skill-{n}")).collect();
        assert_eq!(validate(&i, now).unwrap_err(), "最多选择 10 个 Skills");
        i.skills.truncate(10);
        assert_eq!(validate(&i, now), Ok(()));
        i.skills = vec!["Bad Name".into()];
        assert_eq!(validate(&i, now).unwrap_err(), "Skill 名称无效：Bad Name");
    }

    #[test]
    fn run_blocks_when_skill_missing() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let mut i = input("now");
        i.skills = vec!["x".into(), "y".into()];
        let task = create_task(&conn, &i, now).unwrap();
        let run = active_run(&conn, task).unwrap().unwrap().id;
        let selected = load_task(&conn, task).unwrap().unwrap().skills;
        assert_eq!(selected, vec!["x".to_string(), "y".to_string()], "选择随任务保存");
        let available = vec![option("y"), option("z")];
        assert_eq!(missing_skills(&selected, &available), vec!["x".to_string()]);
        assert!(missing_skills(&selected, &[option("x"), option("y")]).is_empty());

        assert!(!skills_ready(&conn, run, &selected, &available, now + 1).unwrap(), "缺少 Skill 时不放行");
        let r = load_run(&conn, run).unwrap().unwrap();
        assert_eq!((r.status.as_str(), r.block_kind.as_deref(), r.reason.as_deref()), ("attention", Some("skill_missing"), Some("Skill「x」已停用或不存在")));
        assert_eq!(r.started_at, None, "没有启动进程");
        assert!(!claim_run(&conn, run).unwrap(), "需要处理的运行不会被认领拉起");

        // 续跑前同样检查：重新排队后 Skill 仍缺失，再次转为需要处理
        act(&conn, task, "retry", now + 2).unwrap();
        assert!(!skills_ready(&conn, run, &selected, &[option("y")], now + 3).unwrap());
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "attention");
        act(&conn, task, "retry", now + 4).unwrap();
        assert!(skills_ready(&conn, run, &selected, &[option("x"), option("y")], now + 5).unwrap(), "Skill 都在时放行");
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "queued");
        assert_eq!(missing_reason(&["x".into(), "z".into()]), "Skill「x」、「z」已停用或不存在");
    }

    /// 控制方裁决：挂在已取消 / 已完成任务下的排队运行不再放行（与 `claim_run` 的 `state = 'active'` 一致）。
    /// 孤儿运行按旧库遗留数据直接写库构造（修复后 `finish` 不再产生它们）
    #[test]
    fn startable_skips_runs_of_inactive_tasks() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let a = queued(&conn, "c1", r"D:\work\a", true, now);
        let task_a = load_run(&conn, a).unwrap().unwrap().task_id;
        conn.execute("UPDATE automation_tasks SET state='cancelled' WHERE id=?1", [task_a]).unwrap();
        set_status(&conn, a, "waiting");
        let b = queued(&conn, "c1", r"D:\work\b", true, now + 3);
        assert_eq!(startable(&conn).unwrap(), vec![b], "已取消任务的等待运行不挡同账号");
        set_status(&conn, a, "queued");
        assert_eq!(startable(&conn).unwrap(), vec![b], "已取消任务的排队运行不放行");
        assert_eq!(queue_ahead(&conn, &load_run(&conn, b).unwrap().unwrap()).unwrap(), 0, "队列位置不计已取消任务");
        let task_b = load_run(&conn, b).unwrap().unwrap().task_id;
        let queue: Vec<i64> = task_detail(&conn, task_b).unwrap().queue.into_iter().map(|q| q.task_id).collect();
        assert_eq!(queue, vec![task_b], "同账号队列不列已取消任务");

        let c = queued(&conn, "c2", r"D:\work\c", true, now + 4);
        let task_c = load_run(&conn, c).unwrap().unwrap().task_id;
        conn.execute("UPDATE automation_tasks SET state='finished' WHERE id=?1", [task_c]).unwrap();
        assert_eq!(startable(&conn).unwrap(), vec![b], "已完成任务的排队运行不放行");

        // 取消后进程还没退出的运行仍占着工作目录
        let d = queued(&conn, "c3", r"D:\work\d", true, now + 5);
        let task_d = load_run(&conn, d).unwrap().unwrap().task_id;
        set_status(&conn, d, "running");
        act(&conn, task_d, "cancel", now + 6).unwrap();
        let _e = queued(&conn, "c4", r"D:\work\d", true, now + 7);
        assert_eq!(startable(&conn).unwrap(), vec![b], "同一工作目录仍串行");

        // 也仍计入同账号并行上限：3 个执行中（其中 1 个所属任务已取消）时第 4 个不放行
        let mut running = Vec::new();
        for i in 0..3 {
            let r = queued(&conn, "c5", &format!(r"D:\work\cap{i}"), false, now + 10 + i);
            set_status(&conn, r, "running");
            running.push(r);
        }
        act(&conn, load_run(&conn, running[0]).unwrap().unwrap().task_id, "cancel", now + 20).unwrap();
        let fourth = queued(&conn, "c5", r"D:\work\cap3", false, now + 21);
        assert!(!startable(&conn).unwrap().contains(&fourth), "已取消但仍在执行的运行占用并行名额");
    }

    #[test]
    fn finish_on_cancelled_task_ends_as_cancelled() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let p = Progress { session_id: Some("s-1".into()), ..Default::default() };
        let cases = [
            Outcome::Limit { resets_at: Some(now + 60_000), message: "usage limit".into() },
            Outcome::Reauth("401".into()),
            Outcome::UntrustedDir("Not inside a trusted directory".into()),
        ];
        for (i, outcome) in cases.iter().enumerate() {
            let run = queued(&conn, "c1", &format!(r"D:\work\f{i}"), false, now + i as i64);
            let task = load_run(&conn, run).unwrap().unwrap().task_id;
            conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run, now]).unwrap();
            act(&conn, task, "cancel", now + 1).unwrap();
            assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "running", "取消时执行中的运行等进程结束");
            finish(&conn, run, outcome, &p, now + 5_000).unwrap();
            let r = load_run(&conn, run).unwrap().unwrap();
            assert_eq!((r.status.as_str(), r.ended_at, r.resume_pending, r.block_kind), ("cancelled", Some(now + 5_000), false, None), "{outcome:?} 按已取消收尾");
            assert_eq!((r.exec_ms, r.session_id.as_deref()), (5_000, Some("s-1")), "执行时长与会话照常记录");
        }
        assert!(due_checks(&conn, now + 10_000).unwrap().0.is_empty(), "不会对它们做额度检查");

        // 完成是真实结果：照常记为完成，任务保持已取消
        let run = queued(&conn, "c1", r"D:\work\done", false, now + 10);
        let task = load_run(&conn, run).unwrap().unwrap().task_id;
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run, now]).unwrap();
        act(&conn, task, "cancel", now + 11).unwrap();
        finish(&conn, run, &Outcome::Completed, &p, now + 12).unwrap();
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "completed");
        assert_eq!(load_task(&conn, task).unwrap().unwrap().state, "cancelled");
    }

    #[test]
    fn due_checks_skip_cancelled_tasks() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let orphan_task = create_task(&conn, &input("quota"), now).unwrap();
        conn.execute("UPDATE automation_tasks SET state='cancelled' WHERE id=?1", [orphan_task]).unwrap();
        let mut other = input("quota");
        other.connection_id = "c2".into();
        create_task(&conn, &other, now + 1).unwrap();
        assert_eq!(due_checks(&conn, now + 10).unwrap().0, vec!["c2".to_string()], "已取消任务遗留的等待运行不再触发额度检查");
    }

    #[test]
    fn run_prompt_adds_prefix_for_first_run_and_resume() {
        let skills = vec!["a".to_string(), "b".to_string()];
        assert_eq!(run_prompt("claude", &skills, "完善表单", false), "本任务请使用以下 Skills：a、b\n\n完善表单");
        assert_eq!(run_prompt("codex", &skills, "完善表单", true), format!("$a $b\n\n{RESUME_PROMPT}"));
        assert_eq!(run_prompt("claude", &[], "完善表单", false), "完善表单");
        assert_eq!(run_prompt("codex", &[], "完善表单", true), RESUME_PROMPT);
    }

    #[test]
    fn validate_rejects_duplicate_skills() {
        let mut i = input("now");
        i.skills = vec!["a".into(), "b".into(), "a".into()];
        assert_eq!(validate(&i, 1_000_000).unwrap_err(), "Skills 不能重复选择");
    }

    // 任务 2.5 开始
    fn notice_of(conn: &Connection, run: i64) -> Option<TaskNotice> {
        notice_for(&load_run(conn, run).unwrap().unwrap())
    }

    #[test]
    fn notices_follow_run_outcome() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let p = Progress { session_id: Some("s-1".into()), ..Default::default() };
        let running = |cwd: &str, at: i64| {
            let run = queued(&conn, "c1", cwd, false, at);
            assert_eq!(notice_of(&conn, run), None, "排队中不通知");
            conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run, at]).unwrap();
            assert_eq!(notice_of(&conn, run), None, "执行中不通知");
            run
        };

        let done = running(r"D:\work\n1", now);
        finish(&conn, done, &Outcome::Completed, &p, now + 1).unwrap();
        assert_eq!(notice_of(&conn, done), Some(TaskNotice::Completed));

        let failed = running(r"D:\work\n2", now + 2);
        finish(&conn, failed, &Outcome::Failed("进程退出码 1".into()), &p, now + 3).unwrap();
        assert_eq!(notice_of(&conn, failed), Some(TaskNotice::Failed { reason: "进程退出码 1".into() }));

        let limited = running(r"D:\work\n3", now + 4);
        finish(&conn, limited, &Outcome::Limit { resets_at: Some(now + 60_000), message: "usage limit".into() }, &p, now + 5).unwrap();
        assert_eq!(load_run(&conn, limited).unwrap().unwrap().status, "waiting");
        assert_eq!(notice_of(&conn, limited), None, "额度耗尽进入等待不通知，恢复或超时时再通知");

        let reauth = running(r"D:\work\n4", now + 6);
        finish(&conn, reauth, &Outcome::Reauth("401".into()), &p, now + 7).unwrap();
        assert!(matches!(notice_of(&conn, reauth), Some(TaskNotice::Failed { reason }) if reason.starts_with("需要重新登录")), "需要处理归入失败");

        let stopped = running(r"D:\work\n5", now + 8);
        finish(&conn, stopped, &Outcome::Cancelled, &p, now + 9).unwrap();
        assert_eq!(notice_of(&conn, stopped), None, "停止运行不通知");

        let cancelled = running(r"D:\work\n6", now + 10);
        act(&conn, load_run(&conn, cancelled).unwrap().unwrap().task_id, "cancel", now + 11).unwrap();
        finish(&conn, cancelled, &Outcome::Reauth("401".into()), &p, now + 12).unwrap();
        assert_eq!(notice_of(&conn, cancelled), None, "任务已取消的收尾不通知");

        let blocked = queued(&conn, "c1", r"D:\work\n7", false, now + 13);
        set_attention(&conn, blocked, "cwd_missing", "项目目录不可访问，无法在原目录执行。", now + 14).unwrap();
        assert_eq!(notice_of(&conn, blocked), Some(TaskNotice::Failed { reason: "项目目录不可访问，无法在原目录执行。".into() }));
    }

    #[test]
    fn wait_timeout_reports_expired_runs_as_failed() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let mut i = input("quota");
        i.wait_deadline = Some(now + 1_000);
        let task = create_task(&conn, &i, now).unwrap();
        let run = active_run(&conn, task).unwrap().unwrap().id;
        assert_eq!(notice_of(&conn, run), None, "等待额度中不通知");
        assert!(due_checks(&conn, now + 10).unwrap().1.is_empty(), "截止前不超时");
        let (_, expired) = due_checks(&conn, now + 2_000).unwrap();
        assert_eq!(expired, vec![run], "返回超时的运行，供调度线程发通知");
        assert!(matches!(notice_of(&conn, run), Some(TaskNotice::Failed { reason }) if reason.starts_with("等待超时")));
        assert!(due_checks(&conn, now + 3_000).unwrap().1.is_empty(), "只超时一次");
    }
    fn resume_waiting(conn: &Connection, cwd: &str, at: i64, deadline: Option<i64>) -> i64 {
        let run = queued(conn, "c1", cwd, false, at);
        if let Some(d) = deadline {
            conn.execute("UPDATE automation_runs SET task_snapshot=json_set(task_snapshot, '$.wait_deadline', ?2) WHERE id=?1", params![run, d]).unwrap();
        }
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run, at]).unwrap();
        let p = Progress { session_id: Some("s-1".into()), ..Default::default() };
        finish(conn, run, &Outcome::Limit { resets_at: Some(at + 60_000), message: "usage limit".into() }, &p, at + 1).unwrap();
        let r = load_run(conn, run).unwrap().unwrap();
        assert_eq!((r.status.as_str(), r.resume_pending), ("waiting", true));
        run
    }

    fn retry(conn: &Connection, run: i64, at: i64) {
        let task = load_run(conn, run).unwrap().unwrap().task_id;
        act(conn, task, "retry", at).unwrap();
        let r = load_run(conn, run).unwrap().unwrap();
        assert_eq!((r.status.as_str(), r.resume_pending), ("queued", true), "重新排队后仍按续跑执行");
    }

    #[test]
    fn manual_requeue_after_attention_is_not_quota_resume() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };

        let timeout = resume_waiting(&conn, r"D:\work\r1", now, Some(now + 1_000));
        assert_eq!(due_checks(&conn, now + 2_000).unwrap().1, vec![timeout]);
        retry(&conn, timeout, now + 3_000);
        assert!(!take_resume_confirmed(&conn, timeout).unwrap(), "等待超时后手动重新排队：没有确认过额度，不发「任务已继续」");

        let reauth = resume_waiting(&conn, r"D:\work\r2", now + 10, None);
        apply_check(&conn, reauth, &QuotaVerdict::simple("reauth", "401"), now + 20).unwrap();
        assert_eq!(load_run(&conn, reauth).unwrap().unwrap().status, "attention");
        retry(&conn, reauth, now + 30);
        assert!(!take_resume_confirmed(&conn, reauth).unwrap(), "需要重新登录后手动重新排队不发");

        let unsupported = resume_waiting(&conn, r"D:\work\r3", now + 40, None);
        conn.execute("UPDATE automation_runs SET resume_pending=0 WHERE id=?1", [unsupported]).unwrap();
        apply_check(&conn, unsupported, &QuotaVerdict::simple("unsupported", "API Key"), now + 50).unwrap();
        assert_eq!(load_run(&conn, unsupported).unwrap().unwrap().status, "attention");
        conn.execute("UPDATE automation_runs SET resume_pending=1 WHERE id=?1", [unsupported]).unwrap();
        retry(&conn, unsupported, now + 60);
        assert!(!take_resume_confirmed(&conn, unsupported).unwrap(), "无法读取额度后手动重新排队不发");

        // 额度确认可用、启动前又转为需要处理（如账号不匹配）：标记随之作废
        let blocked = resume_waiting(&conn, r"D:\work\r4", now + 70, None);
        assert!(apply_check(&conn, blocked, &ok, now + 80).unwrap());
        set_attention(&conn, blocked, "account_mismatch", "不是当前登录的账号", now + 90).unwrap();
        retry(&conn, blocked, now + 100);
        assert!(!take_resume_confirmed(&conn, blocked).unwrap(), "转为需要处理后再手动重新排队不发");

        // 续跑中被中断（Jolu 退出）转为需要处理，再手动重新排队
        let interrupted = resume_waiting(&conn, r"D:\work\r5", now + 110, None);
        assert!(apply_check(&conn, interrupted, &ok, now + 120).unwrap());
        assert!(claim_run(&conn, interrupted).unwrap());
        assert!(take_resume_confirmed(&conn, interrupted).unwrap(), "认领后取走标记");
        recover(&conn, now + 130).unwrap();
        retry(&conn, interrupted, now + 140);
        assert!(!take_resume_confirmed(&conn, interrupted).unwrap(), "中断后手动重新排队不发");
    }

    #[test]
    fn quota_confirmed_resume_notifies_once() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        let run = resume_waiting(&conn, r"D:\work\q1", now, None);
        assert!(!take_resume_confirmed(&conn, run).unwrap(), "还在等待额度");
        assert!(apply_check(&conn, run, &ok, now + 100_000).unwrap());
        assert!(take_resume_confirmed(&conn, run).unwrap(), "额度检查确认可用后续跑：发一次「任务已继续」");
        assert!(!take_resume_confirmed(&conn, run).unwrap(), "标记只用一次");

        // 不支持读额度：到达预计恢复时间直接尝试继续，同样算确认
        let guess = resume_waiting(&conn, r"D:\work\q2", now + 10, None);
        assert!(apply_check(&conn, guess, &QuotaVerdict::simple("unsupported", "API Key"), now + 200_000).unwrap());
        assert!(take_resume_confirmed(&conn, guess).unwrap());
        assert!(!take_resume_confirmed(&conn, guess).unwrap());
    }

    #[test]
    fn resume_confirmed_column_migrates_idempotently() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let run = queued(&conn, "c1", r"D:\work\m1", false, 1_000_000);
        // 模拟旧库：运行表还没有这一列
        conn.execute_batch("ALTER TABLE automation_runs DROP COLUMN resume_confirmed").unwrap();
        ensure_schema(&conn).unwrap();
        ensure_schema(&conn).unwrap();
        let value: i64 = conn.query_row("SELECT resume_confirmed FROM automation_runs WHERE id = ?1", [run], |r| r.get(0)).unwrap();
        assert_eq!(value, 0, "旧运行默认未确认");
        assert!(!take_resume_confirmed(&conn, run).unwrap());
    }
    // 任务 2.5 结束
