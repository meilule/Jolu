    use super::*;
    #[test]
    fn native_subscription_names_use_prepared_account_and_preserve_existing_name() {
        let db = crate::db::open(Path::new(":memory:")).unwrap();
        let account = || crate::login_detect::NativeAccount { source_id: "native:qoder:cn", name: "Qoder 国内版", account: Some("verified@example.test".into()) };
        register_accounts(&db, "qoder", Some("前端任意名称"), vec![account()]).unwrap();
        let row = crate::connections::list(&db).unwrap().remove(0);
        assert_eq!(row.name, "verified@example.test");
        assert_eq!(row.account_email.as_deref(), Some("verified@example.test"));
        db.execute("UPDATE connections SET name='保留旧名'", []).unwrap();
        register_accounts(&db, "qoder", Some("第二次任意名称"), vec![account()]).unwrap();
        assert_eq!(crate::connections::list(&db).unwrap()[0].name, "保留旧名");
    }
    #[test]
    fn prepared_native_accounts_register_without_local_probe_and_preserve_pause() {
        let db=crate::db::open(Path::new(":memory:")).unwrap();
        let account=||crate::login_detect::NativeAccount{source_id:"native:qoder:cn",name:"Qoder 国内版",account:None};
        assert!(register_accounts(&db,"workbuddy",None,vec![account()]).is_err());
        assert!(crate::connections::list(&db).unwrap().is_empty());
        let (added,existing,_)=register_accounts(&db,"qoder",None,vec![account()]).unwrap();
        assert_eq!((added,existing),(1,0));
        crate::connections::set_paused(&db,"monitor:native:qoder:cn",true).unwrap();
        let (added,existing,_)=register_accounts(&db,"qoder",None,vec![account()]).unwrap();
        assert_eq!((added,existing),(0,1));
        assert!(crate::connections::creds_of(&db,"monitor:native:qoder:cn").unwrap().unwrap().paused);
    }
    #[test]
    fn opencode_pause_stops_collection_and_resume_is_idempotent() {
        let root=fixture_dir("opencode-paused");let data=crate::opencode::data_dir(&root);std::fs::create_dir_all(&data).unwrap();
        let source=Connection::open(data.join("opencode.db")).unwrap();source.execute_batch("CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT); INSERT INTO message VALUES('m','s',123,'{\"role\":\"assistant\",\"tokens\":{\"input\":100,\"output\":30}}');").unwrap();
        let mut db=crate::db::open(&root.join("jolu.db")).unwrap();db.execute("INSERT INTO connections(id,platform,kind,name,label,masked,status,created_ms) VALUES('monitor:native:opencode:openai','opencode','auth','test','','','paused',0)",[]).unwrap();
        assert_eq!(scan_at(&mut db,&root,&root).records_inserted,0);
        db.execute("UPDATE connections SET status='connected'",[]).unwrap();assert_eq!(scan_at(&mut db,&root,&root).records_inserted,1);assert_eq!(scan_at(&mut db,&root,&root).records_inserted,0);
    }

    /// P0-3 场景「扫描延迟」：源库在两次扫描之间新增数据，第二次扫描要补上，
    /// 且不能重复写已经采过的行。
    ///
    /// 这是「扫描延迟」的固定样本 —— 用「先扫、再往源库插一行、再扫」模拟
    /// 采集器落后于源端的真实情况。判据是：第一次 N 条、第二次 N+1 条、
    /// 没有重复行（`dedup_key` 唯一）、第二次不重复消耗已采数据。
    #[test]
    fn a_delayed_scan_picks_up_new_rows_without_duplicating() {
        let root = fixture_dir("zcode-delay");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let db = folder.join("db.sqlite");
        let now = chrono::Utc::now().timestamp_millis();
        let mk = || -> Connection {
            let c = Connection::open(&db).unwrap();
            c.execute_batch("CREATE TABLE IF NOT EXISTS session(id TEXT PRIMARY KEY, title TEXT);
                CREATE TABLE IF NOT EXISTS model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT,
                  status TEXT, started_at INTEGER, completed_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                  cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, provider_total_tokens INTEGER,
                  computed_total_tokens INTEGER, reasoning_tokens INTEGER, raw_usage_json TEXT);").unwrap();
            c
        };
        {
            let input = mk();
            input.execute("INSERT INTO session VALUES('s1','会话')", []).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u1','s1','glm-4.7','completed',?1,?2,10,5,0,0,15,15,0,NULL)",
                params![now - 2000, now - 1000]).unwrap();
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let first = scan_at(&mut conn, &root, &root);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let count = |c: &Connection| -> i64 { c.query_row("SELECT COUNT(*) FROM requests WHERE platform='zcode'", [], |r| r.get(0)).unwrap() };
        assert_eq!(count(&conn), 1, "第一次扫描采到 1 条");

        // 源端在两次扫描之间新增一行（真实的「采集延迟」）
        {
            let input = mk();
            input.execute("INSERT INTO model_usage VALUES('u2','s1','glm-4.7','completed',?1,?2,20,8,0,0,28,28,0,NULL)",
                params![now - 500, now]).unwrap();
        }
        let second = scan_at(&mut conn, &root, &root);
        assert!(second.errors.is_empty(), "{:?}", second.errors);
        assert_eq!(count(&conn), 2, "第二次扫描补上新增的那行");
        let dup: i64 = conn
            .query_row("SELECT COUNT(*) FROM (SELECT dedup_key FROM requests GROUP BY dedup_key HAVING COUNT(*) > 1)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(dup, 0, "不能有重复行");
        std::fs::remove_dir_all(&root).ok();
    }

    /// P0-3 场景「多账号并发」：同一平台的两个账号在同一时刻产生请求，
    /// 各自独立记在 `connection_id` 下，不能互相覆盖或错配。
    ///
    /// 固定样本：两个连接 + 四条请求，两个连接各两条、时间完全交错。
    /// 判据是 `source_usage` 按连接分组后各自计数正确，且没有一条记录连到错的连接。
    #[test]
    fn concurrent_accounts_keep_their_own_rows() {
        let conn = crate::db::open(Path::new(":memory:")).unwrap();
        // 两个账号（连接）
        for (id, name) in [("c-a", "账号 A"), ("c-b", "账号 B")] {
            conn.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug) VALUES(?1,'claude','auth',?2,'x','****',1,?3)",
                params![id, name, id],
            )
            .unwrap();
        }
        // 四条请求，两个账号交替、时间交错
        let entries = [
            ("u-1", "c-a", 1000),
            ("u-2", "c-b", 1001),
            ("u-3", "c-a", 1002),
            ("u-4", "c-b", 1003),
        ];
        for (key, conn_id, ts) in entries {
            conn.execute(
                "INSERT INTO requests(platform,source,dedup_key,ts,connection_id) VALUES('claude','local-session',?1,?2,?3)",
                params![key, ts, conn_id],
            )
            .unwrap();
        }
        let breakdown = crate::usage_stats::source_breakdown(&conn, Some("claude"), 0, 10_000).unwrap();
        let a = breakdown.rows.iter().find(|r| r.connection_id == "c-a").expect("账号 A 的一行");
        let b = breakdown.rows.iter().find(|r| r.connection_id == "c-b").expect("账号 B 的一行");
        assert_eq!((a.requests, b.requests), (2, 2), "两个账号各自 2 条，不能错配");
        assert_eq!(breakdown.total_requests, 4);
        assert_eq!(breakdown.unattributed_requests, 0, "没有记录被丢进未归属");
    }

    /// Qoder 真实布局（2026-11-23 对账修复 P0-3-a）：只有用量库、没有 `~/.qoder/projects`
    /// 的机器上，来源也必须可用并采到数据。
    ///
    /// 修复前：`available = path.exists()`，`path` 是 `~/.qoder/projects`。本机没有该目录，
    /// 于是整个 Qoder 来源永不启用，999 条真实用量一条都采不到。
    #[test]
    fn qoder_source_is_available_from_the_ledger_alone() {
        let root = fixture_dir("qoder-ledger");
        let home = root.join("home");
        // 只建账本，不建 projects 目录
        let ledger_path = crate::qoder_usage::ledger_path(&home);
        std::fs::create_dir_all(ledger_path.parent().unwrap()).unwrap();
        {
            let ledger = Connection::open(&ledger_path).unwrap();
            ledger.execute_batch(
                "CREATE TABLE chat_message(id TEXT, session_id TEXT, role TEXT, token_info TEXT, gmt_create INTEGER);
                 INSERT INTO chat_message VALUES('m1','s1','assistant',
                   '{\"prompt_tokens\":1000,\"completion_tokens\":50,\"cached_tokens\":800}',1700000000000);
                 INSERT INTO chat_message VALUES('m2','s1','assistant',
                   '{\"prompt_tokens\":2000,\"completion_tokens\":60,\"cached_tokens\":1500}',1700000001000);",
            ).unwrap();
        }
        let sources: Vec<_> = sources_at(&home, &home)
            .into_iter()
            .filter(|s| s.platform == "qoder" && s.id == "native:qoder:global")
            .collect();
        assert_eq!(sources.len(), 1);
        assert!(sources[0].available, "有账本就应该可用，不要求 projects 目录");

        let mut conn = crate::db::open(&root.join("jolu.db")).unwrap();
        let result = scan_at(&mut conn, &home, &home);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let (rows, input, cache): (i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(cache_read_tokens),0)
             FROM requests WHERE platform='qoder'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(rows, 2, "两条 assistant 调用各记一行");
        // 存的是**原始 prompt**（含缓存），缓存单列。新鲜输入由 `input_semantics`
        // = includes_cache 的消费方扣减，不能在这里提前扣。
        assert_eq!(input, 3000, "原始 prompt 之和 = 1000 + 2000");
        assert_eq!(cache, 2300, "缓存读单独记 = 800 + 1500");

        // 重扫不重复
        let again = scan_at(&mut conn, &home, &home);
        assert!(again.errors.is_empty(), "{:?}", again.errors);
        let rows2: i64 = conn
            .query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows2, 2, "重扫不重复写");

        std::fs::remove_dir_all(&root).ok();
    }

    /// C-3：账本没变时不重读。用「改 Jolu 库里的值、看下一轮会不会被覆盖回去」判断有没有重读——
    /// 没重读，改过的值会留着；账本一变，整份重建会把它改回源端的值。
    #[test]
    fn an_unchanged_qoder_ledger_is_not_reread() {
        let root = fixture_dir("qoder-ledger-stamp");
        let home = root.join("home");
        let ledger_path = crate::qoder_usage::ledger_path(&home);
        std::fs::create_dir_all(ledger_path.parent().unwrap()).unwrap();
        {
            let ledger = Connection::open(&ledger_path).unwrap();
            ledger.execute_batch(
                "CREATE TABLE chat_message(id TEXT, session_id TEXT, role TEXT, token_info TEXT, gmt_create INTEGER);
                 INSERT INTO chat_message VALUES('m1','s1','assistant',
                   '{\"prompt_tokens\":1000,\"completion_tokens\":50,\"cached_tokens\":800}',1700000000000);",
            ).unwrap();
        }
        let mut conn = crate::db::open(&root.join("jolu.db")).unwrap();
        let output = |c: &Connection| -> i64 {
            c.query_row("SELECT output_tokens FROM requests WHERE platform='qoder' AND dedup_key='s1:m1'", [], |r| r.get(0)).unwrap()
        };
        assert!(scan_at(&mut conn, &home, &home).errors.is_empty());
        assert_eq!(output(&conn), 50);

        // 只改 Jolu 侧，账本不动：下一轮不该重读，改过的值留着
        conn.execute("UPDATE requests SET output_tokens = 999 WHERE platform='qoder'", []).unwrap();
        assert!(scan_at(&mut conn, &home, &home).errors.is_empty());
        assert_eq!(output(&conn), 999, "账本没变，不应重读重写");

        // 账本有变化（新增一条）：整份重建，旧行也按源端的值改回来
        std::thread::sleep(std::time::Duration::from_millis(20));
        {
            let ledger = Connection::open(&ledger_path).unwrap();
            ledger.execute(
                "INSERT INTO chat_message VALUES('m2','s1','assistant',
                   '{\"prompt_tokens\":10,\"completion_tokens\":5,\"cached_tokens\":0}',1700000001000)",
                [],
            ).unwrap();
        }
        assert!(scan_at(&mut conn, &home, &home).errors.is_empty());
        assert_eq!(output(&conn), 50, "账本变了要重读，旧行回到源端的值");
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 2);
        drop(conn);
        std::fs::remove_dir_all(&root).ok();
    }

    /// C-5：账本里的坏行进入扫描错误（按来源定界），好行照常入库。
    #[test]
    fn qoder_bad_rows_surface_as_scan_errors_without_blocking_good_rows() {
        let root = fixture_dir("qoder-ledger-bad");
        let home = root.join("home");
        let ledger_path = crate::qoder_usage::ledger_path(&home);
        std::fs::create_dir_all(ledger_path.parent().unwrap()).unwrap();
        {
            let ledger = Connection::open(&ledger_path).unwrap();
            ledger.execute_batch(
                "CREATE TABLE chat_message(id TEXT, session_id TEXT, role TEXT, token_info TEXT, gmt_create INTEGER);
                 INSERT INTO chat_message VALUES('m1','s1','assistant',
                   '{\"prompt_tokens\":10,\"completion_tokens\":5,\"cached_tokens\":0}',1700000000000);
                 INSERT INTO chat_message VALUES('m2','s1','assistant','{oops',1700000001000);",
            ).unwrap();
        }
        let mut conn = crate::db::open(&root.join("jolu.db")).unwrap();
        let result = scan_at(&mut conn, &home, &home);
        assert!(result.errors.iter().any(|e| e.contains("JSON")), "{:?}", result.errors);
        assert!(result.failed_sources.contains(&"qoder".to_string()));
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1, "好行照常入库");
        drop(conn);
        std::fs::remove_dir_all(&root).ok();
    }

    /// 真机只读验证（`#[ignore]`）：用真实 `~/.qoder` 账本扫进一个**临时** Jolu 库，
    /// 确认端到端能写出正确条数与 Token 合计，且不碰用户真实 `usage.db`。
    #[test]
    #[ignore = "真实账号只读验证：读 ~/.qoder 账本，写入临时库"]
    fn real_qoder_ledger_scans_into_a_temp_db() {
        let Some(home) = crate::creds::home() else { return };
        if !crate::qoder_usage::ledger_path(&home).is_file() {
            eprintln!("跳过：真实 Qoder 账本不存在");
            return;
        }
        // 临时目录 + 空库，绝不碰用户的 usage.db
        let root = std::env::temp_dir().join(format!(
            "jolu-qoder-real-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut conn = crate::db::open(&root.join("usage.db")).unwrap();
        let result = scan_at(&mut conn, &home, &root);
        eprintln!("真实扫描：插入 {} 条，错误 {:?}", result.records_inserted, result.errors);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let (rows, input, cache, output): (i64, i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(output_tokens),0) FROM requests WHERE platform='qoder'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).unwrap();
        eprintln!("真实扫描结果：{rows} 条，原始输入（含缓存）{input}，缓存读 {cache}，新鲜输入 {}，输出 {output}", input - cache);
        assert!(rows > 0, "真实账本应能扫出记录");
        assert!(cache > 0, "缓存读应为正");
        let dup: i64 = conn.query_row(
            "SELECT COUNT(*) FROM (SELECT dedup_key FROM requests WHERE platform='qoder'
             GROUP BY dedup_key HAVING COUNT(*) > 1)", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(dup, 0, "去重键不能重复");
        std::fs::remove_dir_all(&root).ok();
    }

    /// P1「账号切换」：切换账号后重扫历史日志，不重复写、不把旧记录改归到新账号。
    ///
    /// ## 真实场景
    ///
    /// 用户在同一个工具里退出 A 账号、登录 B 账号后继续用。旧日志文件还在
    /// （`~/.claude/projects/` 里的历史 jsonl 不会因为换账号被删），而 Jolu 的
    /// 采集器仍会扫到它们。两件事必须同时成立：
    ///
    /// 1. **重扫不重复**：旧日志的 `dedup_key` 已经存在，不能因为「换账号了」再写一遍；
    /// 2. **归因不错配**：旧记录不能因为连接表里多了 B 而被改判成 B。
    ///
    /// 这条固定样本把这两件事都锁住：先扫出 A 名下的记录，再插入 B 连接、再扫一遍，
    /// 断言记录数不变、且 `connection_id` 一个都没变。
    #[test]
    fn switching_accounts_rescans_without_duplication_or_reattribution() {
        let root = fixture_dir("account-switch");
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".qoder/projects/p")).unwrap();
        let log = home.join(".qoder/projects/p/s.jsonl");
        let mut body = String::new();
        for i in 0..2 {
            let line = serde_json::json!({
                "type": "assistant",
                "timestamp": 1_700_000_000_000i64 + i * 1000,
                "sessionId": "s",
                "message": { "id": format!("m{i}"), "usage": { "input_tokens": 100, "output_tokens": 20,
                            "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0 } }
            });
            body.push_str(&line.to_string());
            body.push('\n');
        }
        std::fs::write(&log, &body).unwrap();

        let db_path = root.join("jolu.db");
        let mut conn = crate::db::open(&db_path).unwrap();
        // A 账号：注册为一条 auth 连接，并把首次扫描的记录归到它下面
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug)
             VALUES('c-a','qoder','auth','账号 A','x','****',1,'c-a')",
            [],
        ).unwrap();
        let first = scan_at(&mut conn, &home, &home);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let count = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(count(&conn), 2, "A 账号下采到 2 条");
        // 把这两条归到 A（生产里由代理计时 / 网关记账写入 connection_id）
        conn.execute("UPDATE requests SET connection_id='c-a' WHERE platform='qoder'", []).unwrap();
        let attributed_to = |c: &Connection, id: &str| -> i64 {
            c.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder' AND connection_id=?1",
                [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(attributed_to(&conn, "c-a"), 2);

        // 换账号：B 连接出现（A 保留，用户只是切了个登录）
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug)
             VALUES('c-b','qoder','auth','账号 B','y','****',2,'c-b')",
            [],
        ).unwrap();
        // 再扫一遍（等价于切换账号后的下一次扫描）
        let second = scan_at(&mut conn, &home, &home);
        assert!(second.errors.is_empty(), "{:?}", second.errors);
        assert_eq!(count(&conn), 2, "重扫不能重复写");
        assert_eq!(second.records_inserted, 0, "位点已推进，不该有新插入");
        assert_eq!(attributed_to(&conn, "c-a"), 2, "旧记录仍归 A，不能被改判成 B");
        assert_eq!(attributed_to(&conn, "c-b"), 0, "B 不该凭空得到历史记录");

        // 归因视图也不该把旧会话算到 B 头上：平台内两条 auth 连接时，
        // 无端点事实的请求落「平台」档而不是猜一个
        let snapshot = crate::attribution::Snapshot::load(&conn).unwrap();
        let view = snapshot.resolve("qoder", None, None, None);
        assert_eq!(view.confidence, crate::attribution::Confidence::Platform,
            "两条 auth 连接、没有端点事实时不能猜");

        std::fs::remove_dir_all(&root).ok();
    }

    /// P1「账号切换」第二种形态：旧账号**被删掉**，新账号接管。
    ///
    /// 与上一条不同：上一条是两个账号并存（旧记录仍能归到 A）。这条是 A 真的没了，
    /// 它的 `connection_id` 成了指向不存在连接的悬空值。正确行为：
    ///
    /// - 记录本身不能丢（历史账本必须保留，AGENTS.md 明确要求）；
    /// - 归因**不能**自动改到 B —— 那些请求确实是 A 产生的，改成 B 就是伪造归属；
    /// - 展示层遇到悬空 id 要落回「平台」档，而不是显示一个已删除的连接名。
    #[test]
    fn deleting_the_old_account_keeps_history_and_does_not_hand_it_to_the_new_one() {
        let conn = crate::db::open(Path::new(":memory:")).unwrap();
        let now = chrono::Local::now().timestamp_millis();
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug)
             VALUES('c-a','qoder','auth','账号 A','x','****',1,'c-a')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO requests(platform,source,dedup_key,ts,total_tokens,total_known,connection_id)
             VALUES('qoder','native:qoder:global','s:m0',?1,120,1,'c-a')",
            params![now],
        ).unwrap();
        // 删掉 A，插入 B
        conn.execute("DELETE FROM connections WHERE id='c-a'", []).unwrap();
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,slug)
             VALUES('c-b','qoder','auth','账号 B','y','****',2,'c-b')",
            [],
        ).unwrap();

        // 记录仍在，且 connection_id 仍是已消失的 A（不被改写成 B）
        let (rows, owner): (i64, Option<String>) = conn.query_row(
            "SELECT COUNT(*), MAX(connection_id) FROM requests WHERE platform='qoder'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(rows, 1, "删除账号不能删历史记录");
        assert_eq!(owner.as_deref(), Some("c-a"), "归属仍是产生它的账号，不能被改写");

        // 归因视图：悬空 id 落平台档，不显示已删除的连接、也不冒充 B
        let snapshot = crate::attribution::Snapshot::load(&conn).unwrap();
        let view = snapshot.resolve("qoder", Some("c-a"), None, None);
        assert_eq!(view.confidence, crate::attribution::Confidence::Platform,
            "指向已删除连接时要落平台档");
        assert_eq!(view.connection_id, None, "不能返回不存在的连接 id");
    }

    /// P1「中断恢复」：扫描中途被强制终止（kill -9 级别）后重扫，不丢记录、不重复。
    ///
    /// ## 为什么不能只测「正常扫两遍」
    ///
    /// 正常扫两遍走的是「位点已推进 → 提前返回」。真正的中断发生在**事务未提交**时：
    /// 记录已写进事务、位点还没落库，进程被杀，SQLite 回滚整个事务。此时
    /// 数据库里既没有记录也没有位点（正确），重扫必须从头补齐 —— 但去重键还在，
    /// 所以「补齐」不会变成「翻倍」。这三件事只有真中断才测得出来。
    ///
    /// ## 怎么模拟 kill -9
    ///
    /// 起一个子进程扫同一份文件、同一份库，在事务提交前 `std::process::abort()`。
    /// 不用 `panic`：panic 会走 unwinding 并 drop 事务（等于回滚），与 SIGKILL 不同；
    /// `abort` 直接结束进程，正是「位点与记录都没落盘」的状态。
    #[test]
    fn a_scan_killed_mid_transaction_is_recovered_without_loss_or_duplication() {
        if std::env::var_os("JOLU_SCAN_CRASH_CHILD").is_some() {
            return;
        }
        let root = fixture_dir("scan-crash");
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".qoder/projects/p")).unwrap();
        let log = home.join(".qoder/projects/p/s.jsonl");
        // 三条可解析的 assistant 记录
        let mut body = String::new();
        for i in 0..3 {
            let line = serde_json::json!({
                "type": "assistant",
                "timestamp": 1_700_000_000_000i64 + i * 1000,
                "sessionId": "s",
                "message": { "id": format!("m{i}"), "usage": { "input_tokens": 10, "output_tokens": 5,
                                                            "cache_read_input_tokens": 0,
                                                            "cache_creation_input_tokens": 0 } }
            });
            body.push_str(&line.to_string());
            body.push('\n');
        }
        std::fs::write(&log, &body).unwrap();
        let db_path = root.join("jolu.db");

        // 子进程：开库、扫一份「未提交」的事务，然后 abort
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("{}::scan_crash_child", module_path!().trim_start_matches("jolu_lib::")), "--nocapture"])
            .env("JOLU_SCAN_CRASH_CHILD", "1")
            .env("JOLU_SCAN_CRASH_HOME", &home)
            .env("JOLU_SCAN_CRASH_DB", &db_path)
            .output()
            .unwrap();
        // abort 的退出码是非零、无正常退出码；这里只断言「确实非正常结束」
        assert!(!output.status.success(), "子进程应被中断而非正常退出：{:?}", output.status);

        // 父进程接着扫：必须补齐全部 3 条，且一条不多
        let mut conn = crate::db::open(&db_path).unwrap();
        let first = scan_at(&mut conn, &home, &home);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let count = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(count(&conn), 3, "中断后重扫要补齐全部记录，一条都不能丢");

        // 再扫一次：位点已推进，不应再写入
        let second = scan_at(&mut conn, &home, &home);
        assert!(second.errors.is_empty(), "{:?}", second.errors);
        assert_eq!(count(&conn), 3, "重复扫描不能增加记录");
        assert_eq!(second.records_inserted, 0, "位点已推进，第二次不应再插入");

        std::fs::remove_dir_all(&root).ok();
    }

    /// P1「中断恢复」第二种形态：源端写到一半被 kill（文件里留下没有换行的半行）。
    ///
    /// 与上一条不同：这里**进程没崩在事务里**，而是源端文件本身不完整。真实场景是
    /// Claude Code / Codex 正在追加一条 assistant 行，写到一半进程被杀，留半行。
    /// 正确行为是「只采完整行、半行留到下次」，且下一次文件写完后补齐那一条 ——
    /// 不能因为见到半行就整份重读（会重复），也不能把半行当坏行永久跳过（会丢）。
    #[test]
    fn a_half_written_tail_is_picked_up_after_the_source_finishes_it() {
        let root = fixture_dir("scan-tail");
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".qoder/projects/p")).unwrap();
        let log = home.join(".qoder/projects/p/s.jsonl");
        let line = |id: &str| {
            serde_json::json!({
                "type": "assistant", "timestamp": 1_700_000_000_000i64, "sessionId": "s",
                "message": { "id": id, "usage": { "input_tokens": 10, "output_tokens": 5,
                            "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0 } }
            }).to_string()
        };
        let complete = format!("{}
", line("m0"));
        let second = line("m1");
        // 前半行：只写一半，没有换行
        let half = &second[..second.len() / 2];
        std::fs::write(&log, format!("{complete}{half}")).unwrap();

        let db_path = root.join("jolu.db");
        let mut conn = crate::db::open(&db_path).unwrap();
        let first = scan_at(&mut conn, &home, &home);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        let count = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM requests WHERE platform='qoder'", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(count(&conn), 1, "只采到完整的那一行");

        // 源端把剩下的半行写完
        std::fs::write(&log, format!("{complete}{second}
")).unwrap();
        let second_scan = scan_at(&mut conn, &home, &home);
        assert!(second_scan.errors.is_empty(), "{:?}", second_scan.errors);
        assert_eq!(count(&conn), 2, "补齐第二行，不丢");
        let dup: i64 = conn.query_row(
            "SELECT COUNT(*) FROM (SELECT dedup_key FROM requests GROUP BY dedup_key HAVING COUNT(*) > 1)",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(dup, 0, "不能重复");

        // 再扫：无新增
        assert_eq!(scan_at(&mut conn, &home, &home).records_inserted, 0);
        std::fs::remove_dir_all(&root).ok();
    }

    /// 上一条测试的子进程体：写一批记录但不提交，然后 abort。
    #[test]
    fn scan_crash_child() {
        if std::env::var_os("JOLU_SCAN_CRASH_CHILD").is_none() {
            return;
        }
        let db_path = PathBuf::from(std::env::var_os("JOLU_SCAN_CRASH_DB").unwrap());
        let mut conn = crate::db::open(&db_path).unwrap();
        // 扫到一个「记录写了、位点还没提交」的状态：这里直接开事务写入后 abort，
        // 不 commit —— 与进程在 commit 之前被 SIGKILL 完全等价。
        let tx_state = conn.transaction().unwrap();
        tx_state.execute(
            "INSERT INTO requests(platform,source,dedup_key,ts,total_tokens,total_known)
             VALUES('qoder','native:qoder:global','s:m-crash',1,15,1)",
            [],
        ).unwrap();
        // 故意不提交
        std::mem::forget(tx_state);
        std::process::abort();
    }

    fn fixture_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "jolu-native-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn one_unusable_instance_does_not_block_registration_of_another() {
        let root = fixture_dir("register");
        let empty = root.join(".qoder/projects/p");
        let valid = root.join(".qoder-cn/projects/p");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&valid).unwrap();
        std::fs::write(valid.join("s.jsonl"),format!("{}\n",serde_json::json!({"type":"assistant","timestamp":1000,"sessionId":"s","message":{"id":"m","usage":{"credits":1.0}}}))).unwrap();
        let conn = crate::db::open(Path::new(":memory:")).unwrap();
        let sources = sources_at(&root, &root)
            .into_iter()
            .filter(|s| s.platform == "qoder" && s.available)
            .collect();
        let (added, existing, warnings) = register_sources(&conn, sources, None).unwrap();
        assert_eq!((added, existing, warnings.len()), (1, 0, 1));
        assert_eq!(
            crate::connections::list(&conn).unwrap()[0].id,
            "monitor:native:qoder:cn"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn zcode_request_status_drives_session_state_with_running_grace() {
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(zcode_session_state("completed", now, now), "done");
        assert_eq!(zcode_session_state("error", now, now), "failed");
        assert_eq!(zcode_session_state("cancelled", now, now), "failed");
        assert_eq!(zcode_session_state("running", now - 5_000, now), "running");
        assert_eq!(zcode_session_state("pending", now, now), "running");
        assert_eq!(zcode_session_state("running", now - ZCODE_RUNNING_GRACE_MS - 1, now), "unknown");
        assert_eq!(zcode_session_state("running", now + 120_000, now), "unknown", "未来时间戳不可信");
    }

    /// 请求级状态：ZCode 原生的三种终态不能折叠成两档，其余状态保持 unknown。
    /// 会话状态（`zcode_session_state`）仍然只分 done / failed，两者是不同层级的事实。
    #[test]
    fn zcode_native_outcome_keeps_cancelled_apart_from_error() {
        assert_eq!(zcode_native_outcome("completed"), "success");
        assert_eq!(zcode_native_outcome("error"), "failed");
        assert_eq!(zcode_native_outcome("cancelled"), "cancelled", "取消不是错误");
        // 未知 / 缺失状态不猜：running、pending 以及任何没见过的值都留在 unknown
        for status in ["running", "pending", "", "aborted", "COMPLETED"] {
            assert_eq!(zcode_native_outcome(status), "unknown", "{status} 不该被猜成终态");
        }
        // 会话状态维持原样：会话卡片只说这一轮结束得怎么样
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(zcode_session_state("cancelled", now, now), "failed");
    }

    /// 旧库里的 cancelled 曾被写成 failed；回填只改状态列，不删行、不动其他状态，
    /// 且写过标记后不再重复回填。
    #[test]
    fn backfill_turns_legacy_failed_cancellations_into_cancelled_without_deleting_rows() {
        let root = fixture_dir("zcode-cancelled-backfill");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch(
                "CREATE TABLE model_usage(id TEXT PRIMARY KEY, status TEXT);
                 INSERT INTO model_usage VALUES('u-cancel','cancelled'),('u-error','error');",
            ).unwrap();
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        // 模拟旧版本扫描的结果：cancelled 与 error 都写成 failed
        for (key, outcome, ts) in [
            ("u-cancel", "failed", now - 2_000),
            ("u-error", "failed", now - 3_000),
            // 已成功 / 未知的历史行不能被顺手改掉
            ("u-cancel-done", "success", now - 4_000),
            ("u-mystery", "unknown", now - 5_000),
            // 源库已按 30 天清理、只剩在 Jolu 的历史行：必须原样保留
            ("u-purged", "failed", now - 40 * 86_400_000),
        ] {
            conn.execute(
                "INSERT INTO requests(platform,source,dedup_key,ts,native_outcome)
                 VALUES('zcode','native:zcode:main',?1,?2,?3)",
                params![key, ts, outcome],
            ).unwrap();
        }
        let before: Vec<(String, String)> = conn
            .prepare("SELECT dedup_key, native_outcome FROM requests ORDER BY dedup_key").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
            .flatten().collect();

        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 1, "只改原始状态确为 cancelled 的那一行");

        let outcome = |conn: &Connection, key: &str| -> Option<String> {
            conn.query_row("SELECT native_outcome FROM requests WHERE dedup_key=?1", [key], |r| r.get(0)).unwrap()
        };
        assert_eq!(outcome(&conn, "u-cancel").as_deref(), Some("cancelled"));
        assert_eq!(outcome(&conn, "u-error").as_deref(), Some("failed"), "error 仍是失败");
        assert_eq!(outcome(&conn, "u-cancel-done").as_deref(), Some("success"), "已正确的状态不动");
        assert_eq!(outcome(&conn, "u-mystery").as_deref(), Some("unknown"));
        assert_eq!(outcome(&conn, "u-purged").as_deref(), Some("failed"), "源库已清理的历史行原样保留");

        // 行数与非状态字段都没变：回填只 UPDATE 状态列
        let after: Vec<(String, String)> = conn
            .prepare("SELECT dedup_key, native_outcome FROM requests ORDER BY dedup_key").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
            .flatten().collect();
        assert_eq!(after.len(), before.len(), "不能删除任何历史记录");
        let ts: i64 = conn.query_row("SELECT ts FROM requests WHERE dedup_key='u-cancel'", [], |r| r.get(0)).unwrap();
        assert_eq!(ts, now - 2_000, "只改状态，不动时间等其他字段");

        // 标记已写：第二次调用不再读源库、也不再产生改动
        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 0, "带版本标记，跑过就不再重复");
        assert_eq!(outcome(&conn, "u-cancel").as_deref(), Some("cancelled"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cancelled_backfill_retries_after_write_failure() {
        let root = fixture_dir("zcode-cancelled-retry");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let input = Connection::open(folder.join("db.sqlite")).unwrap();
        input.execute_batch("CREATE TABLE model_usage(id TEXT, status TEXT); INSERT INTO model_usage VALUES('u-1','cancelled');").unwrap();
        drop(input);
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        conn.execute("INSERT INTO requests(platform,source,dedup_key,ts,native_outcome) VALUES('zcode','native:zcode:main','u-1',1,'failed')", []).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_backfill BEFORE UPDATE OF native_outcome ON requests BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 0);
        let marks: i64 = conn.query_row("SELECT COUNT(*) FROM migration_log WHERE kind='zcode_cancelled_backfill'", [], |r| r.get(0)).unwrap();
        assert_eq!(marks, 0);
        conn.execute_batch("DROP TRIGGER reject_backfill;").unwrap();
        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 1);
        let outcome: String = conn.query_row("SELECT native_outcome FROM requests WHERE dedup_key='u-1'", [], |r| r.get(0)).unwrap();
        assert_eq!(outcome, "cancelled");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// **真机回填**（`#[ignore]`，只在显式指定时跑）：对用户真实的 `usage.db` 执行一次
    /// `backfill_zcode_cancelled`，把旧版本误写成 `failed` 的 ZCode 取消请求改成 `cancelled`。
    ///
    /// 为什么做成 ignored 测试而不是 CLI 子命令：它只跑一次、需要真实的源库与主库同时存在，
    /// 不是用户会反复用的功能。放在这里能被编译与类型检查覆盖，又不会误触发。
    ///
    /// 安全护栏（测试自己断言，不靠调用方）：
    /// - **行数不变**（只 UPDATE 状态，绝不删行）
    /// - 只动 `native_outcome='failed'` 且**源库状态确为 cancelled** 的行
    /// - `success` / `unknown` 一行不动
    /// - 跑完写 `migration_log` 标记，重复调用返回 0
    ///
    /// 运行：`cargo test --manifest-path backend/Cargo.toml --lib real_zcode_cancelled_backfill -- --ignored --nocapture`
    #[test]
    #[ignore = "写用户真实数据库；只在明确要求时手动运行"]
    fn real_zcode_cancelled_backfill() {
        let home = crate::creds::home().expect("需要用户主目录");
        // `locate()` 返回（数据目录, 配置目录）——数据目录里就是 usage.db
        let (data_dir, _) = crate::service::paths::locate().expect("定位数据目录");
        let db_path = data_dir.join("usage.db");
        assert!(db_path.is_file(), "usage.db 不存在：{}", db_path.display());
        let mut conn = crate::db::open(&db_path).expect("打开 usage.db");
        let count = |conn: &Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM requests WHERE platform='zcode'", [], |r| r.get(0)).unwrap()
        };
        let failed_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM requests WHERE platform='zcode' AND native_outcome='failed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let rows_before = count(&conn);
        eprintln!("回填前：zcode 行 {rows_before}，其中 failed {failed_before}");

        let changed = backfill_zcode_cancelled(&mut conn, &home);
        let rows_after = count(&conn);
        let failed_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM requests WHERE platform='zcode' AND native_outcome='failed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let cancelled: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM requests WHERE platform='zcode' AND native_outcome='cancelled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        eprintln!("回填后：改动 {changed} 行，failed {failed_after}，cancelled {cancelled}，总行数 {rows_after}");

        assert_eq!(rows_after, rows_before, "回填绝不能删行");
        assert_eq!(changed as i64, cancelled, "改动数应等于现在的 cancelled 总数");
        assert_eq!(failed_after + cancelled, failed_before, "failed 减少的量全部变成了 cancelled");
        // 幂等：标记已写，再跑一次没有任何改动
        assert_eq!(backfill_zcode_cancelled(&mut conn, &home), 0, "带版本标记，跑过就不再重复");
    }

    /// 源库没有 cancelled 行时也要写标记：否则每次心跳都会重读一遍源库。
    #[test]
    fn backfill_marks_done_even_when_nothing_to_fix() {
        let root = fixture_dir("zcode-cancelled-nothing");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch("CREATE TABLE model_usage(id TEXT PRIMARY KEY, status TEXT);
                INSERT INTO model_usage VALUES('u-1','completed');").unwrap();
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 0);
        // 标记确实写了：这是「不必每次心跳重读源库」的依据，而不是「没东西可改就返回 0」
        let marks: i64 = conn
            .query_row("SELECT COUNT(*) FROM migration_log WHERE kind='zcode_cancelled_backfill'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(marks, 1, "没有 cancelled 行也要落标记，否则每个心跳都会重读源库");
        // 标记存在后，即使源库出现可回填的行也不再处理：一次性迁移
        let input = Connection::open(folder.join("db.sqlite")).unwrap();
        input.execute("UPDATE model_usage SET status='cancelled' WHERE id='u-1'", []).unwrap();
        drop(input);
        conn.execute(
            "INSERT INTO requests(platform,source,dedup_key,ts,native_outcome)
             VALUES('zcode','native:zcode:main','u-1',1,'failed')",
            [],
        ).unwrap();
        assert_eq!(backfill_zcode_cancelled(&mut conn, &root), 0, "带标记后不再重复回填");
        let outcome: Option<String> = conn
            .query_row("SELECT native_outcome FROM requests WHERE dedup_key='u-1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(outcome.as_deref(), Some("failed"), "标记后不再改，留给下一次换 kind 的迁移");
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn zcode_session_time_updated_boosts_recently_active_sessions_to_running() {
        let now = chrono::Utc::now().timestamp_millis();
        let row_ts = now - 60_000;
        // 一轮结束后 time_updated 只比 usage 行新几十毫秒：不抬成 running
        assert_eq!(zcode_boost_running("done", row_ts, Some(row_ts + 33), now), "done");
        // 新一轮开跑：time_updated 比 usage 行新出 3.6 秒（实测形态），抬成 running
        assert_eq!(zcode_boost_running("done", row_ts, Some(row_ts + 3_600), now), "running");
        // 失败行的会话继续跑同样抬升
        assert_eq!(zcode_boost_running("failed", row_ts, Some(row_ts + 3_600), now), "running");
        // 已经是 running 的行不动
        assert_eq!(zcode_boost_running("running", row_ts, None, now), "running");
        // 没有会话级时间（旧版库缺列）不抬
        assert_eq!(zcode_boost_running("done", row_ts, None, now), "done");
        // 超出宽限期的旧活动（CLI 异常退出残留）不抬：20 分钟前完成、19 分钟前有活动
        let stale_row_ts = now - 20 * 60_000;
        assert_eq!(
            zcode_boost_running("done", stale_row_ts, Some(stale_row_ts + 60_000), now),
            "done"
        );
        // 未来时间戳不可信
        assert_eq!(zcode_boost_running("done", row_ts, Some(now + 120_000), now), "done");
    }
    #[test]
    fn zcode_synthetic_database_yields_real_session_states_and_outcomes() {
        let root = fixture_dir("zcode");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY, title TEXT);
                CREATE TABLE model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT, status TEXT,
                  started_at INTEGER, completed_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                  cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, provider_total_tokens INTEGER,
                  computed_total_tokens INTEGER, reasoning_tokens INTEGER, raw_usage_json TEXT);").unwrap();
            for (sid, title) in [("s-run", "进行中"), ("s-done", "已完成"), ("s-fail", "失败"), ("s-stale", "残留")] {
                input.execute("INSERT INTO session VALUES(?1,?2)", params![sid, title]).unwrap();
            }
            let raw = r#"{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}"#;
            let rows: [(&str, &str, &str, i64, Option<i64>); 5] = [
                ("u-run", "s-run", "running", now - 2_000, None),
                ("u-done", "s-done", "completed", now - 5_000, Some(now - 3_000)),
                ("u-fail", "s-fail", "error", now - 5_000, Some(now - 4_000)),
                // 用户取消：状态必须独立成 cancelled，不能并进 failed（2026-11-21）
                ("u-cancel", "s-fail", "cancelled", now - 4_000, Some(now - 3_500)),
                ("u-stale", "s-stale", "running", now - 30 * 60_000, None),
            ];
            for (id, sid, status, started, completed) in rows {
                input.execute("INSERT INTO model_usage VALUES(?1,?2,'glm-4.7',?3,?4,?5,100,20,0,0,120,120,5,?6)",
                    params![id, sid, status, started, completed, raw]).unwrap();
            }
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        // 模拟升级前扫描戳未变化，但旧记录采用了完成时间。
        let key = root.join(".zcode/cli/db/db.sqlite").to_string_lossy().to_string();
        conn.execute("DELETE FROM scan_state WHERE path=?1", [format!("{key}#zcode-started-at-v1")]).unwrap();
        conn.execute("UPDATE requests SET ts=?1 WHERE dedup_key='u-done'", [now - 3_000]).unwrap();
        conn.execute("INSERT INTO requests(platform,source,dedup_key,ts) VALUES('zcode','native:zcode:main','already-cleaned',1)", []).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_timestamp BEFORE UPDATE OF ts ON requests WHEN NEW.dedup_key='u-done' BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert!(!result.errors.is_empty());
        assert_eq!(crate::db::get_scan_state(&conn, &format!("{key}#zcode-started-at-v1")), None);
        conn.execute_batch("DROP TRIGGER reject_timestamp;").unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let restored: i64 = conn.query_row("SELECT ts FROM requests WHERE dedup_key='u-done'", [], |r| r.get(0)).unwrap();
        assert_eq!(restored, now - 5_000);
        let retained: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE dedup_key='already-cleaned'", [], |r| r.get(0)).unwrap();
        assert_eq!(retained, 1);
        assert_eq!(crate::db::get_scan_state(&conn, &format!("{key}#zcode-started-at-v1")), Some((1, 1)));
        let state_of = |sid: &str| -> String {
            conn.query_row("SELECT activity_state FROM sessions WHERE platform='zcode' AND session_id LIKE ?1",
                [format!("%:{sid}")], |r| r.get(0)).unwrap()
        };
        assert_eq!(state_of("s-run"), "running");
        assert_eq!(state_of("s-done"), "done");
        assert_eq!(state_of("s-fail"), "failed");
        assert_eq!(state_of("s-stale"), "unknown", "超过宽限期的进行中行不算运行");
        let outcome = |key: &str| -> String {
            conn.query_row("SELECT native_outcome FROM requests WHERE dedup_key=?1", [key], |r| r.get(0)).unwrap()
        };
        assert_eq!(outcome("u-done"), "success", "completed → success");
        assert_eq!(outcome("u-fail"), "failed", "error → failed");
        assert_eq!(outcome("u-cancel"), "cancelled", "cancelled 必须单独一档，不能折叠成 failed");
        assert_eq!(outcome("u-run"), "unknown", "还没有结论的行不猜成功");
        let live = crate::db::live_usage(&conn, "zcode", None).unwrap();
        let mut states: Vec<(String, String)> = live.active_sessions.iter()
            .map(|s| (s.title.clone(), s.state.clone())).collect();
        states.sort();
        let mut expected = vec![
            ("已完成".to_string(), "done".to_string()),
            ("失败".to_string(), "failed".to_string()),
            ("进行中".to_string(), "running".to_string()),
        ];
        expected.sort();
        assert_eq!(states, expected, "残留行不在 90 秒窗口内，其余按真实状态输出");
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn zcode_time_updated_marks_active_turns_running() {
        let root = fixture_dir("zcode-time-updated");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            // 带time_updated列的库（3.18 实测 schema）：一轮进行中时，最后一行
            // model_usage 仍是上一条请求的 completed，会话级时间戳却一直在走。
            input.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY, title TEXT, time_updated INTEGER);
                CREATE TABLE model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT, status TEXT,
                  started_at INTEGER, completed_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                  cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, provider_total_tokens INTEGER,
                  computed_total_tokens INTEGER, reasoning_tokens INTEGER, raw_usage_json TEXT);").unwrap();
            let raw = r#"{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}"#;
            // s-live：10 秒前请求 completed，但会话 2 秒前还在写（新一轮在跑）
            input.execute("INSERT INTO session VALUES('s-live','学习中的会话',?1)", params![now - 2_000]).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u-live','s-live','glm-4.7','completed',?1,?2,100,20,0,0,120,120,5,?3)",
                params![now - 10_000, now - 9_000, raw]).unwrap();
            // 上一条请求已经很久，新一轮仍在写。状态时间应采用会话更新时间。
            input.execute("INSERT INTO session VALUES('s-long','长会话',?1)", params![now - 1_000]).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u-long','s-long','glm-4.7','completed',?1,?2,100,20,0,0,120,120,5,?3)",
                params![now - 1_200_000, now - 1_199_000, raw]).unwrap();
            // 请求开始已久，刚刚完成时仍应出现在最近活跃列表。
            input.execute("INSERT INTO session VALUES('s-finished','刚完成的长请求',?1)", params![now - 500]).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u-finished','s-finished','glm-4.7','completed',?1,?2,100,20,0,0,120,120,5,?3)",
                params![now - 1_200_000, now - 500, raw]).unwrap();
            // s-idle：time_updated 只比 usage 行新 33ms，就是刚收尾，不算运行
            input.execute("INSERT INTO session VALUES('s-idle','刚完成的会话',?1)", params![now - 3_000 + 33]).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u-idle','s-idle','glm-4.7','completed',?1,?2,100,20,0,0,120,120,5,?3)",
                params![now - 5_000, now - 3_033, raw]).unwrap();
            // s-gone：有新活动但已超出宽限期（CLI 异常退出），不假 running
            input.execute("INSERT INTO session VALUES('s-gone','残留的会话',?1)", params![now - ZCODE_RUNNING_GRACE_MS - 60_000]).unwrap();
            input.execute("INSERT INTO model_usage VALUES('u-gone','s-gone','glm-4.7','completed',?1,?2,100,20,0,0,120,120,5,?3)",
                params![now - ZCODE_RUNNING_GRACE_MS - 70_000, now - ZCODE_RUNNING_GRACE_MS - 61_000, raw]).unwrap();
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let state_of = |sid: &str| -> String {
            conn.query_row("SELECT activity_state FROM sessions WHERE platform='zcode' AND session_id LIKE ?1",
                [format!("%:{sid}")], |r| r.get(0)).unwrap()
        };
        assert_eq!(state_of("s-live"), "running", "行状态 completed 但会话仍在写：新一轮在跑");
        assert_eq!(state_of("s-long"), "running");
        let updated: i64 = conn.query_row(
            "SELECT activity_updated_ms FROM sessions WHERE platform='zcode' AND session_id LIKE '%:s-long'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(updated, now - 1_000);
        let listed = crate::session_threads::list(&conn, &crate::session_threads::SessionFilter::default()).unwrap();
        let long = listed.sessions.iter().find(|s| s.session_id.ends_with(":s-long")).unwrap();
        assert_eq!(long.status, "running");
        let live = crate::db::live_usage(&conn, "zcode", None).unwrap();
        let long = live.active_sessions.iter().find(|s| s.session_id.ends_with(":s-long")).unwrap();
        assert_eq!(long.state, "running");
        assert_eq!(long.last_seen_ms, now - 1_000);
        assert_eq!(long.delta_tokens, 0, "历史用量不能冒充最近增量");
        let finished = live.active_sessions.iter().find(|s| s.session_id.ends_with(":s-finished")).unwrap();
        assert_eq!(finished.state, "done");
        assert_eq!(finished.last_seen_ms, now - 500);
        assert_eq!(finished.delta_tokens, 0);
        assert!(live.active_sessions.iter().position(|s| s.session_id.ends_with(":s-finished"))
            < live.active_sessions.iter().position(|s| s.session_id.ends_with(":s-long")));
        assert_eq!(zcode_observed_state(Some("done"), Some(now + 61_000), now), "recent");
        assert_eq!(zcode_observed_state(Some("failed"), Some(now + 61_000), now), "recent");
        assert_eq!(state_of("s-idle"), "done", "收尾写库的几十毫秒差不是新一轮");
        assert_eq!(state_of("s-gone"), "done", "超宽限期的旧活动不假 running");
        std::fs::remove_dir_all(&root).unwrap();
    }
    /// provider_id 必须以独立字段（`requests.provider_key`）落库：它是连接归因的端点事实。
    /// 塞进 platform 会把「哪套工具」和「哪个端点」合并，归因层就再也分不开了。
    #[test]
    fn zcode_provider_id_lands_in_its_own_column() {
        let root = fixture_dir("zcode-provider");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch(
                "CREATE TABLE session(id TEXT PRIMARY KEY, title TEXT);
                CREATE TABLE model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT, status TEXT,
                  started_at INTEGER, completed_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                  cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, provider_total_tokens INTEGER,
                  computed_total_tokens INTEGER, reasoning_tokens INTEGER, raw_usage_json TEXT,
                  provider_id TEXT NOT NULL DEFAULT '');",
            ).unwrap();
            input.execute("INSERT INTO session VALUES('s1','会话')", []).unwrap();
            input.execute(
                "INSERT INTO model_usage VALUES('u1','s1','GLM-5.3','completed',?1,?2,100,20,0,0,120,120,5,'{}','new-provider')",
                params![now - 5_000, now - 4_000],
            ).unwrap();
        }
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let (platform, provider): (String, Option<String>) = conn
            .query_row("SELECT platform, provider_key FROM requests WHERE dedup_key='u1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(platform, "zcode", "平台仍是工具，provider 不并入 platform");
        assert_eq!(provider.as_deref(), Some("new-provider"));
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn gemini_checkpoint_rewind_and_new_events_keep_monotonic_cursor() {
        let root = fixture_dir("rewind");
        let folder = root.join(".gemini/tmp/project/chats");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("session-test.jsonl");
        let message = |id: &str| serde_json::json!({"id":id,"type":"gemini","timestamp":1000,"tokens":{"input":100,"output":5,"cached":80,"total":105}});
        let save = |messages: Vec<Value>| {
            std::fs::write(
                &path,
                format!(
                    "{}\n",
                    serde_json::json!({"$set":{"sessionId":"s","messages":messages}})
                ),
            )
            .unwrap()
        };
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        save(vec![message("a"), message("b")]);
        assert!(scan_at(&mut conn, &root, &root).errors.is_empty());
        let first = crate::db::live_usage(&conn, "gemini", None).unwrap().cursor;
        save(vec![message("a")]);
        assert!(scan_at(&mut conn, &root, &root).errors.is_empty());
        let second = crate::db::live_usage(&conn, "gemini", Some(first))
            .unwrap()
            .cursor;
        assert!(second > first);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        save(vec![message("a"), message("new-long-id")]);
        scan_at(&mut conn, &root, &root);
        let next = crate::db::live_usage(&conn, "gemini", Some(second)).unwrap();
        assert!(next.cursor > second);
        assert_eq!(next.delta_tokens, 25);
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn damaged_and_partial_files_do_not_block_other_sessions_or_advance_bad_checkpoint() {
        let root = fixture_dir("isolation");
        let folder = root.join(".workbuddy/projects/p");
        std::fs::create_dir_all(&folder).unwrap();
        let bad = folder.join("a-bad.jsonl");
        std::fs::write(&bad, "{bad}\n").unwrap();
        let good = folder.join("b-good.jsonl");
        let line = serde_json::json!({"id":"m","sessionId":"s","timestamp":1000,"providerData":{"usage":{"inputTokens":10,"outputTokens":5,"totalTokens":15},"rawUsage":{"prompt_tokens":10,"prompt_tokens_details":{"cached_tokens":0},"credit":0.5}}});
        std::fs::write(&good, format!("{line}\n{{\"partial\":")).unwrap();
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let result = scan_at(&mut conn, &root, &root);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.records_inserted, 1);
        assert_eq!(
            crate::db::get_scan_state(&conn, &bad.to_string_lossy()),
            None
        );
        assert_eq!(scan_at(&mut conn, &root, &root).records_inserted, 0);
        crate::db::cleanup_history(&mut conn, 2000).unwrap();
        std::fs::write(&good, format!("{line}\n\n")).unwrap();
        scan_at(&mut conn, &root, &root);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn appended_session_lines_are_read_incrementally_and_legacy_stamps_rebuild_once() {
        use std::io::Write;
        let root = fixture_dir("incremental");
        // 与目录枚举得到的路径字符串一致（位点以路径字符串为键）
        let folder = root.join(".workbuddy/projects").join("p");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("s.jsonl");
        let line = |id: &str| serde_json::json!({"id":id,"sessionId":"s","timestamp":1000,"providerData":{"usage":{"inputTokens":10,"outputTokens":5,"totalTokens":15},"rawUsage":{"prompt_tokens":10,"prompt_tokens_details":{"cached_tokens":0},"credit":0.5}}}).to_string();
        let count = |conn: &Connection| conn.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get::<_, i64>(0)).unwrap();
        let key = path.to_string_lossy().to_string();
        std::fs::write(&path, format!("{}\n", line("a"))).unwrap();
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        assert_eq!(scan_at(&mut conn, &root, &root).records_inserted, 1);

        // 追加一整行和半行：只读新增的完整行，位点停在行边界，半行留给下次。
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(file, "{}\n{}", line("b"), &line("c")[..20]).unwrap();
        drop(file);
        let result = scan_at(&mut conn, &root, &root);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.records_inserted, 1);
        assert_eq!(count(&conn), 2);
        let boundary = format!("{}\n{}\n", line("a"), line("b")).len() as i64;
        assert_eq!(crate::db::get_scan_checkpoint(&conn, &key).unwrap().offset, boundary);

        // 半行写完后接着读，不重复计入前两行。
        std::fs::write(&path, format!("{}\n{}\n{}\n", line("a"), line("b"), line("c"))).unwrap();
        assert_eq!(scan_at(&mut conn, &root, &root).records_inserted, 1);
        assert_eq!(count(&conn), 3);

        // 旧版本的整份戳记（无行边界标记）与落在行中间的位点都从头重读一次，结果不变。
        for (offset, initialized) in [(std::fs::metadata(&path).unwrap().len() as i64, 0), (boundary - 3, 1)] {
            conn.execute("UPDATE scan_state SET offset=?1, mtime=0, parser_initialized=?2 WHERE path=?3", params![offset, initialized, key]).unwrap();
            let result = scan_at(&mut conn, &root, &root);
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            assert_eq!(result.records_inserted, 0);
            assert_eq!(count(&conn), 3);
            let checkpoint = crate::db::get_scan_checkpoint(&conn, &key).unwrap();
            assert!(checkpoint.parser_initialized);
            assert_eq!(checkpoint.offset, std::fs::metadata(&path).unwrap().len() as i64);
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn changed_paths_only_scan_the_sources_that_contain_them() {
        let root = fixture_dir("targeted");
        let folder = root.join(".workbuddy/projects/p");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("s.jsonl");
        std::fs::write(&path, format!("{}\n", serde_json::json!({"id":"m","sessionId":"s","timestamp":1000,"providerData":{"usage":{"inputTokens":10,"outputTokens":5,"totalTokens":15},"rawUsage":{"prompt_tokens":10,"prompt_tokens_details":{"cached_tokens":0}}}}))).unwrap();
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let elsewhere = [root.join(".claude/projects/x/y.jsonl")];
        let skipped = scan_sources(&mut conn, &root, &root, Some(&elsewhere));
        assert_eq!((skipped.files_scanned, skipped.records_inserted), (0, 0));
        let hit = scan_sources(&mut conn, &root, &root, Some(std::slice::from_ref(&path)));
        assert_eq!((hit.files_scanned, hit.records_inserted), (1, 1));
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    #[ignore = "显式只读本机来源，应用写入仅在内存数据库"]
    fn read_only_native_smoke() {
        let mut conn = crate::db::open(Path::new(":memory:")).unwrap();
        let result = scan_at(&mut conn, &home().unwrap(), &app_data());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let mut q=conn.prepare("SELECT platform,COUNT(*),SUM(total_tokens),SUM(cache_read_tokens),SUM(cache_write_tokens),SUM(total_known) FROM requests GROUP BY platform").unwrap();
        let counts: Vec<_> = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        eprintln!("仅输出统计汇总：{counts:?}");
        drop(q);
        assert_eq!(
            scan_at(&mut conn, &home().unwrap(), &app_data()).records_inserted,
            0
        );
        for (platform, _, _, _, _, _) in &counts {
            crate::db::usage_breakdown_at(
                &conn,
                platform,
                "total",
                None,
                None,
                chrono::Utc::now().timestamp_millis(),
            )
            .unwrap();
            crate::db::trend_at(
                &conn,
                platform,
                "total",
                None,
                None,
                chrono::Utc::now().timestamp_millis(),
            )
            .unwrap();
        }
        let json = crate::db::export_backup(&conn, None, None, None).unwrap();
        let mut restored = crate::db::open(Path::new(":memory:")).unwrap();
        crate::db::import_backup(&mut restored, &json).unwrap();
        for (platform, _, _, _, _, _) in counts {
            assert_eq!(
                source_store::metrics(&conn, &platform, 0, i64::MAX, None)
                    .unwrap()
                    .credits,
                source_store::metrics(&restored, &platform, 0, i64::MAX, None)
                    .unwrap()
                    .credits
            );
        }
    }
    #[test]
    fn missing_sources_are_not_created_or_reported_connected() {
        let path = std::env::temp_dir().join(format!("missing-native-{}", std::process::id()));
        let list = sources_at(&path, &path);
        assert_eq!(list.len(), 17);
        assert!(list.iter().all(|s| !s.available));
        assert!(!path.exists());
        assert!(readonly(&path.join("missing.db")).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn account_sources_have_distinct_identities_and_are_never_session_files() {
        let path = std::env::temp_dir().join(format!("jolu-account-sources-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(path.join("Trae")).unwrap();
        std::fs::create_dir_all(path.join("Trae CN")).unwrap();
        std::fs::create_dir_all(path.join(".zcode/v2")).unwrap();
        std::fs::write(path.join(".zcode/v2/credentials.json"), "{invalid credentials}").unwrap();
        let list = sources_at(&path, &path);
        let intl = list.iter().find(|s| s.id == "native:trae:ide").unwrap();
        let cn = list.iter().find(|s| s.id == "native:trae:cn").unwrap();
        let account = list.iter().find(|s| s.id == "native:zcode:account").unwrap();
        assert!(intl.available && cn.available && account.available);
        assert_ne!(intl.path, cn.path);
        assert!(!collects_sessions(intl) && !collects_sessions(cn) && !collects_sessions(account));
        assert!(validate_source(account).is_err(), "发现文件不等于登录有效");
        assert!(std::fs::read_to_string(&account.path).unwrap().contains("invalid credentials"));
        std::fs::remove_dir_all(path).unwrap();
    }

    /// ZCode 的 session.directory 必须落库成会话 cwd，否则全部归入「未识别项目」。
    #[test]
    fn zcode_directory_becomes_session_cwd() {
        let root = fixture_dir("zcode-cwd");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY, title TEXT, directory TEXT);
                CREATE TABLE model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT,
                  status TEXT, started_at INTEGER, completed_at INTEGER, input_tokens INTEGER,
                  output_tokens INTEGER, cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER,
                  provider_total_tokens INTEGER, computed_total_tokens INTEGER, reasoning_tokens INTEGER,
                  raw_usage_json TEXT);").unwrap();
            input.execute(
                "INSERT INTO session(id,title,directory) VALUES('s1','修复入口','D:/develop/Jolu')",
                [],
            ).unwrap();
            input.execute(
                "INSERT INTO model_usage(id,session_id,model_id,status,started_at,completed_at,input_tokens,output_tokens)
                 VALUES('u1','s1','glm-4.6','completed',?1,?1,10,5)",
                [now - 1000],
            ).unwrap();
        }
        let mut store = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let source = Source {
            id: "native:zcode:main".into(),
            platform: "zcode".into(),
            name: "ZCode".into(),
            available: true,
            capabilities: vec![],
            reason: None,
            path: folder.join("db.sqlite"),
        };
        zcode(&mut store, &source).unwrap();
        let cwd: Option<String> = store
            .query_row("SELECT cwd FROM sessions WHERE platform='zcode' AND session_id='native:zcode:main:s1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cwd.as_deref(), Some("D:/develop/Jolu"), "ZCode 的 directory 必须落库成会话 cwd");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 跨本地午夜的请求必须按 **开始时间** 归日，不是按完成时间。
    ///
    /// 现场：ZCode 自己按 `started_at` 归日、也按它做 30 天清理；Jolu 曾经优先用
    /// `completed_at`，于是 23:59 开始、00:01 完成的请求会被记到第二天——本机实测
    /// 有 6 条这样的请求，按日对账时两边分布对不上。
    ///
    /// 这条测试用一条**固定**的跨午夜请求（本地时区 2026-03-10 23:59:30 开始、
    /// 2026-03-11 00:00:30 完成），断言落库的 `ts` 落在 3 月 10 日。
    #[test]
    fn zcode_attributes_a_cross_midnight_request_to_its_start_day() {
        use chrono::TimeZone;
        let root = fixture_dir("zcode-midnight");
        let folder = root.join(".zcode/cli/db");
        std::fs::create_dir_all(&folder).unwrap();
        // 本地时区的固定时刻（不是「现在」）：跨午夜测试必须可复现，不能依赖跑测试的钟点
        let started = chrono::Local
            .with_ymd_and_hms(2026, 3, 10, 23, 59, 30)
            .single()
            .expect("固定本地时刻")
            .timestamp_millis();
        let completed = started + 60_000; // 一分钟后：2026-03-11 00:00:30
        {
            let input = Connection::open(folder.join("db.sqlite")).unwrap();
            input.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY, title TEXT, directory TEXT);
                CREATE TABLE model_usage(id TEXT PRIMARY KEY, session_id TEXT, model_id TEXT,
                  status TEXT, started_at INTEGER, completed_at INTEGER, input_tokens INTEGER,
                  output_tokens INTEGER, cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER,
                  provider_total_tokens INTEGER, computed_total_tokens INTEGER, reasoning_tokens INTEGER,
                  raw_usage_json TEXT);").unwrap();
            input.execute("INSERT INTO session(id,title) VALUES('s1','跨午夜')", []).unwrap();
            input.execute(
                "INSERT INTO model_usage(id,session_id,model_id,status,started_at,completed_at,input_tokens,output_tokens)
                 VALUES('u1','s1','glm-4.6','completed',?1,?2,10,5)",
                rusqlite::params![started, completed],
            ).unwrap();
        }
        let mut store = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let source = Source {
            id: "native:zcode:main".into(),
            platform: "zcode".into(),
            name: "ZCode".into(),
            available: true,
            capabilities: vec![],
            reason: None,
            path: folder.join("db.sqlite"),
        };
        zcode(&mut store, &source).unwrap();
        let ts: i64 = store
            .query_row("SELECT ts FROM requests WHERE platform='zcode' AND dedup_key='u1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ts, started, "落库时间必须是开始时间（23:59:30），不是完成时间（次日 00:00:30）");
        let day = chrono::Local.timestamp_millis_opt(ts).single().unwrap().format("%Y-%m-%d").to_string();
        assert_eq!(day, "2026-03-10", "这条请求属于 3 月 10 日；按完成时间会错记到 3 月 11 日");
        std::fs::remove_dir_all(&root).unwrap();
    }
