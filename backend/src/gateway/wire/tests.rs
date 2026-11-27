//! `gateway::wire` 单元测试。

    use super::*;
    use std::sync::Arc;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::Response;
    use crate::gateway::activity::RouteMeta;
    use crate::gateway::adapter::registry::with_thinking_repair;
    use crate::gateway::route;
    use crate::proxy::ProxyDeps;
    use super::dispatch::{with_tool_loop, with_tool_loop_within};

    /// C-1：数据库被占着时，写流水不卡住请求线程；锁放开后这一行照样写进库。
    #[test]
    fn route_record_does_not_block_on_a_busy_database_and_still_lands() {
        let mut deps = Arc::try_unwrap(crate::proxy::test_support::bare_deps())
            .unwrap_or_else(|_| panic!("test dependencies must have a single owner"));
        let (emitted_tx, emitted_rx) = std::sync::mpsc::channel();
        deps.on_activity = Arc::new(move |_, _| {
            emitted_tx.send(()).unwrap();
        });
        let deps = Arc::new(deps);
        let guard = deps.db.lock().unwrap();
        let started = std::time::Instant::now();
        let meta = RouteMeta { requested_model: "fixture/model".into(), ..Default::default() };
        let row = record_route(&deps, 42, &meta, None, None, Some(503), "failed", std::time::Instant::now());
        assert!(started.elapsed() < std::time::Duration::from_millis(200), "请求线程不能等数据库锁");
        emitted_rx.recv_timeout(std::time::Duration::from_secs(2)).expect("结束事件先于写库发出");
        assert_eq!(row.get(), None, "锁还被占着，这一行还没写");
        drop(guard);
        crate::gateway::route_log::flush();
        assert!(row.get().is_some(), "锁放开后写入线程补上这一行");
        let conn = deps.db.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM gateway_route_log WHERE request_id = 42 AND result = 'failed'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    /// F-4：走**真实重试循环**的失败转移——A 返回 503，B 返回 200。
    ///
    /// P2-3 的两条固定样本是直接 `INSERT INTO gateway_route_log`，只验证
    /// `day_overview` 怎么分组。这条补上另一半：经过 `wire::handle` 的重试循环，
    /// 确认「多次尝试共用 request_uid / 每次尝试各写一行 / 只有成功才绑亲和 /
    /// 失败成员被休置」这四条真的成立。
    #[tokio::test]
    async fn a_failing_member_hands_over_to_the_next_and_the_log_shows_both_attempts() {
        crate::gateway::route_log::flush();
        let deps = crate::proxy::test_support::bare_deps();

        // A：503（可重试）；B：200 + 最小 Chat Completions JSON。
        // 用 axum 起假上游而不是手写 HTTP：手写响应容易在超时/半包上出问题，
        // 会把「上游不可达」的 502 混进来，掩盖真正要测的转移逻辑。
        let failing = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let failing_base = format!("http://{}", failing.local_addr().unwrap());
        tokio::spawn(async move {
            let service = axum::Router::new().fallback(|| async {
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            });
            let _ = axum::serve(failing, service).await;
        });
        let healthy = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let healthy_base = format!("http://{}", healthy.local_addr().unwrap());
        tokio::spawn(async move {
            let service = axum::Router::new().fallback(|| async {
                (
                    [("content-type", "application/json")],
                    r#"{"id":"chatcmpl-f4","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":1,"total_tokens":6}}"#,
                )
            });
            let _ = axum::serve(healthy, service).await;
        });

        for (id, base) in [("f4-a", &failing_base), ("f4-b", &healthy_base)] {
            deps.db.lock().unwrap().execute(
                "INSERT INTO connections(id,platform,kind,name,secret,base_url,protocol,status,serve_enabled,created_ms,slug)
                 VALUES(?1,'custom','api',?1,'fixture-token',?2,'openai','connected',1,1,?1)",
                rusqlite::params![id, base],
            ).unwrap();
        }
        {
            let conn = deps.db.lock().unwrap();
            let members = vec![
                crate::gateway::routing_groups::GroupMember {
                    slug: "f4-a".into(), model: "fixture-model".into(),
                    paused_until_ms: None, weight: None, tier: None,
                },
                crate::gateway::routing_groups::GroupMember {
                    slug: "f4-b".into(), model: "fixture-model".into(),
                    paused_until_ms: None, weight: None, tier: None,
                },
            ];
            crate::gateway::routing_groups::create(&conn, "f4-failover", "转移测试", "order", &members, None).unwrap();
        }

        // 带 codex 身份与会话标识，以便验证亲和
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer jolu-codex.f4".parse().unwrap());
        headers.insert("session-id", "f4-session".parse().unwrap());
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            handle(
                CallerProtocol::Chat,
                deps.clone(),
                headers,
                axum::body::Bytes::from_static(
                    br#"{"model":"group/f4-failover","messages":[{"role":"user","content":"hi"}]}"#,
                ),
                "/v1/chat/completions",
                None,
            ),
        )
        .await
        .expect("请求应在 10 秒内完成");
        assert_eq!(response.status(), StatusCode::OK, "B 成功，最终响应应是 200");

        // 流水：等写入线程落库
        crate::gateway::route_log::flush();
        let rows: Vec<(String, String, Option<i64>, Option<String>)> = {
            let conn = deps.db.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT source, result, request_uid, decision FROM gateway_route_log ORDER BY id",
                )
                .unwrap();
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows
        };
        assert_eq!(rows.len(), 2, "两次尝试各一行，实际 {rows:?}");
        assert_eq!(rows[0].0, "f4-a");
        assert_eq!(rows[0].1, "failed", "A 返回 503 应记为 failed");
        assert_eq!(rows[1].0, "f4-b");
        assert_eq!(rows[1].1, "ok");
        assert_eq!(rows[0].2, rows[1].2, "两次尝试共用同一个 request_uid");
        assert!(rows[0].2.is_some(), "request_uid 应已分配");
        for (source, _, _, decision) in &rows {
            let decision = decision.as_deref().unwrap_or_default();
            assert!(decision.contains("分组 f4-failover"), "{source} 的决策解释缺分组：{decision}");
            assert!(decision.contains("策略 order"), "{source} 的决策解释缺策略：{decision}");
        }

        // 统计：1 次请求、1 次转移、0 次错误
        {
            let conn = deps.db.lock().unwrap();
            let now = chrono::Local::now().timestamp_millis();
            let day = crate::gateway::route_log::day_overview(&conn, now - 60_000, 0, now + 60_000).unwrap();
            assert_eq!(day.today, 1, "两次尝试是一次请求");
            assert_eq!(day.reroutes, 1, "发生了一次转移");
            assert_eq!(day.errors, 0, "最终成功，不算错误");
        }

        // 亲和：绑定到 B。会话键由 `session_key` 生成——`session-id` 头映射成
        // `hdr:codex-session:<值>`，不是「平台:值」。
        {
            let affinity = deps.affinity.as_ref().unwrap().lock().unwrap();
            assert_eq!(
                affinity.get("hdr:codex-session:f4-session"),
                Some("f4-b"),
                "成功的成员获得亲和",
            );
        }

        // 休置：A 刚失败过，下一次请求应先试 B
        {
            let conn = deps.db.lock().unwrap();
            let now = chrono::Local::now().timestamp_millis();
            let ordered = crate::gateway::routing_groups::get(&conn, "f4-failover").unwrap().unwrap();
            // 直接看休置状态：A 应处于休置中
            let a_rested = ordered.members.iter().find(|m| m.slug == "f4-a").map(|m| {
                crate::gateway::routing_groups::is_rested("f4-failover", m, now)
            }).unwrap_or(false);
            assert!(a_rested, "失败的成员应被休置，避免下一个请求又先撞上它");
        }
    }

    /// F-4 第二条：两个成员都返回 503 时，仍是一次请求（2 行流水、1 个 request_uid），
    /// `day_overview` 记 1 次错误，最终响应是最后一次尝试的状态。
    #[tokio::test]
    async fn all_members_failing_still_counts_as_one_error_request() {
        crate::gateway::route_log::flush();
        let deps = crate::proxy::test_support::bare_deps();

        let mut bases = Vec::new();
        for _ in 0..2 {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            bases.push(format!("http://{}", listener.local_addr().unwrap()));
            tokio::spawn(async move {
                let service = axum::Router::new().fallback(|| async {
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                });
                let _ = axum::serve(listener, service).await;
            });
        }
        for (index, base) in bases.iter().enumerate() {
            let id = format!("f4x-{index}");
            deps.db.lock().unwrap().execute(
                "INSERT INTO connections(id,platform,kind,name,secret,base_url,protocol,status,serve_enabled,created_ms,slug)
                 VALUES(?1,'custom','api',?1,'fixture-token',?2,'openai','connected',1,1,?1)",
                rusqlite::params![id, base],
            ).unwrap();
        }
        {
            let conn = deps.db.lock().unwrap();
            let members: Vec<_> = (0..2)
                .map(|index| crate::gateway::routing_groups::GroupMember {
                    slug: format!("f4x-{index}"),
                    model: "fixture-model".into(),
                    paused_until_ms: None,
                    weight: None,
                    tier: None,
                })
                .collect();
            crate::gateway::routing_groups::create(&conn, "f4x-all-fail", "全失败", "order", &members, None).unwrap();
        }

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            handle(
                CallerProtocol::Chat,
                deps.clone(),
                HeaderMap::new(),
                axum::body::Bytes::from_static(
                    br#"{"model":"group/f4x-all-fail","messages":[{"role":"user","content":"hi"}]}"#,
                ),
                "/v1/chat/completions",
                None,
            ),
        )
        .await
        .expect("请求应在 10 秒内完成");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "最后一次尝试的状态");

        crate::gateway::route_log::flush();
        let conn = deps.db.lock().unwrap();
        let rows: Vec<(String, String, Option<i64>)> = {
            let mut stmt = conn
                .prepare("SELECT source, result, request_uid FROM gateway_route_log ORDER BY id")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(rows.len(), 2, "两次尝试各一行");
        assert!(rows.iter().all(|(_, result, _)| result == "failed"), "两次都失败");
        assert_eq!(rows[0].2, rows[1].2, "共用 request_uid");
        let now = chrono::Local::now().timestamp_millis();
        let day = crate::gateway::route_log::day_overview(&conn, now - 60_000, 0, now + 60_000).unwrap();
        assert_eq!(day.today, 1, "两次尝试是一次请求");
        assert_eq!(day.errors, 1, "最终失败算一次错误请求");
        assert_eq!(day.reroutes, 1);
    }

    /// P2 并发：起一个假的 Chat 上游，记录收到的请求数；`abort_stream` 时先发一段 SSE 再中途断开。
    async fn counting_upstream(abort_stream: bool) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let counter = hits.clone();
        tokio::spawn(async move {
            let service = axum::Router::new().fallback(move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if abort_stream {
                        // 先给出 200 与一段 SSE，再让 body 报错：上游在流中途断开。
                        // 第二段延迟后才报错：否则 hyper 可能在发出响应头之前就读到错误，
                        // 表现成「响应头阶段失败」（502），测不到「流已开始」这一档。
                        use futures_util::StreamExt;
                        let first = futures_util::stream::once(async {
                            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
                                b"data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"par\"}}]}\n\n",
                            ))
                        });
                        let broken = futures_util::stream::once(async {
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                            Err::<axum::body::Bytes, _>(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "upstream dropped"))
                        });
                        return axum::response::Response::builder()
                            .status(200)
                            .header("content-type", "text/event-stream")
                            .body(axum::body::Body::from_stream(first.chain(broken)))
                            .unwrap();
                    }
                    axum::response::Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(
                            r#"{"id":"chatcmpl-p2","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":1,"total_tokens":6}}"#,
                        ))
                        .unwrap()
                }
            });
            let _ = axum::serve(listener, service).await;
        });
        (base, hits)
    }

    fn add_group(deps: &Arc<ProxyDeps>, slug: &str, strategy: &str, members: &[(&str, &str)]) {
        let conn = deps.db.lock().unwrap();
        for (id, base) in members {
            conn.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,base_url,protocol,status,serve_enabled,created_ms,slug)
                 VALUES(?1,'custom','api',?1,'fixture-token',?2,'openai','connected',1,1,?1)",
                rusqlite::params![id, base],
            )
            .unwrap();
        }
        let list: Vec<_> = members
            .iter()
            .map(|(id, _)| crate::gateway::routing_groups::GroupMember {
                slug: (*id).into(),
                model: "fixture-model".into(),
                paused_until_ms: None,
                weight: None,
                tier: None,
            })
            .collect();
        crate::gateway::routing_groups::create(&conn, slug, slug, strategy, &list, None).unwrap();
    }

    async fn send_chat(deps: &Arc<ProxyDeps>, slug: &str, session: Option<&str>, stream: bool) -> Response {
        let mut headers = HeaderMap::new();
        if let Some(session) = session {
            headers.insert("authorization", "Bearer jolu-codex.p2".parse().unwrap());
            headers.insert("session-id", session.parse().unwrap());
        }
        let body = format!(
            r#"{{"model":"group/{slug}","stream":{stream},"messages":[{{"role":"user","content":"hi"}}]}}"#
        );
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            handle(CallerProtocol::Chat, deps.clone(), headers, axum::body::Bytes::from(body), "/v1/chat/completions", None),
        )
        .await
        .expect("请求应在 10 秒内完成");
        if response.status() != StatusCode::OK && !stream {
            let status = response.status();
            let text = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap_or_default();
            panic!("网关返回 {status}：{}", String::from_utf8_lossy(&text));
        }
        response
    }

    /// P2 验收「并发」：分组层面多个请求同时轮转。
    ///
    /// `rotate` 的游标是进程内共享状态，并发请求各自拿号；断言两点：
    /// 1. 8 个同时到达的请求被平均分到两个成员（4 / 4），没有因为抢游标而挤到同一个成员；
    /// 2. 每个请求一行流水、各自独立的 `request_uid`，统计记 8 次请求、0 次转移——
    ///    并发不能把不同请求合并，也不能把一次请求拆成多次。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_requests_rotate_evenly_and_each_counts_once() {
        crate::gateway::route_log::flush();
        let deps = crate::proxy::test_support::bare_deps();
        let (base_a, hits_a) = counting_upstream(false).await;
        let (base_b, hits_b) = counting_upstream(false).await;
        add_group(&deps, "p2-rotate", "rotate", &[("p2r-a", &base_a), ("p2r-b", &base_b)]);

        let calls: Vec<_> = (0..8)
            .map(|_| {
                let deps = deps.clone();
                tokio::spawn(async move {
                    // 读完 body：中途丢掉响应会被 `CancelWatch` 记成取消
                    let response = send_chat(&deps, "p2-rotate", None, false).await;
                    let status = response.status();
                    axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
                    status
                })
            })
            .collect();
        for call in calls {
            assert_eq!(call.await.unwrap(), StatusCode::OK);
        }
        let (a, b) = (hits_a.load(Ordering::SeqCst), hits_b.load(Ordering::SeqCst));
        assert_eq!((a, b), (4, 4), "并发轮转应平均分配，实际 A={a} B={b}");

        crate::gateway::route_log::flush();
        let conn = deps.db.lock().unwrap();
        let (rows, uids): (i64, i64) = conn
            .query_row("SELECT COUNT(*), COUNT(DISTINCT request_uid) FROM gateway_route_log", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((rows, uids), (8, 8), "8 个请求各一行、各一个身份");
        let now = chrono::Local::now().timestamp_millis();
        let day = crate::gateway::route_log::day_overview(&conn, now - 60_000, 0, now + 60_000).unwrap();
        assert_eq!((day.today, day.reroutes, day.errors), (8, 0, 0));
    }

    /// P2 验收「并发」：多个会话同时进入同一分组，亲和各自绑定、互不串号。
    ///
    /// 第一轮 4 个会话并发请求（`rotate` 把它们分到两个成员），每个会话绑到服务它的成员；
    /// 第二轮同样 4 个会话再并发请求一次，每个会话必须回到第一轮的成员——
    /// 并发绑定不能让会话 A 拿到会话 B 的亲和。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_sessions_keep_their_own_affinity() {
        crate::gateway::route_log::flush();
        let deps = crate::proxy::test_support::bare_deps();
        let (base_a, _) = counting_upstream(false).await;
        let (base_b, _) = counting_upstream(false).await;
        add_group(&deps, "p2-affinity", "rotate", &[("p2a-a", &base_a), ("p2a-b", &base_b)]);
        let sessions = ["p2-s1", "p2-s2", "p2-s3", "p2-s4"];

        async fn round(deps: &Arc<ProxyDeps>, sessions: &[&'static str]) {
            let calls: Vec<_> = sessions
                .iter()
                .map(|session| {
                    let deps = deps.clone();
                    let session = *session;
                    tokio::spawn(async move {
                        let response = send_chat(&deps, "p2-affinity", Some(session), false).await;
                        let status = response.status();
                        axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
                        status
                    })
                })
                .collect();
            for call in calls {
                assert_eq!(call.await.unwrap(), StatusCode::OK);
            }
        }
        round(&deps, &sessions).await;
        let bound_after_first: Vec<Option<String>> = {
            let affinity = deps.affinity.as_ref().unwrap().lock().unwrap();
            sessions
                .iter()
                .map(|s| affinity.get(&format!("hdr:codex-session:{s}")).map(str::to_string))
                .collect()
        };
        assert!(bound_after_first.iter().all(Option::is_some), "每个会话成功后都要绑定：{bound_after_first:?}");
        let distinct: std::collections::HashSet<_> = bound_after_first.iter().flatten().collect();
        assert_eq!(distinct.len(), 2, "轮转应把 4 个会话分到两个成员：{bound_after_first:?}");

        round(&deps, &sessions).await;
        crate::gateway::route_log::flush();
        let conn = deps.db.lock().unwrap();
        for (session, first) in sessions.iter().zip(&bound_after_first) {
            let served: Vec<String> = {
                let mut stmt = conn
                    .prepare("SELECT source FROM gateway_route_log WHERE session_id = ?1 ORDER BY id")
                    .unwrap();
                stmt.query_map([session], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
            };
            assert_eq!(served.len(), 2, "{session} 两轮各一行：{served:?}");
            assert_eq!(Some(&served[1]), first.as_ref(), "{session} 第二轮必须回到自己绑定的成员：{served:?}");
        }
    }

    /// P2-3「流式中断下的失败转移」：上游已经开始流式输出后中途断开，**不**换下一个成员。
    ///
    /// 这是有意的设计，不是遗漏：响应头与部分内容已经交给了 Agent，此时再换成员重发，
    /// Agent 会收到两段拼接的回答，上游也会被计费两次。所以只在响应头阶段
    /// （`is_retryable_status`）做转移；流中途的失败由 `CancelWatch` 把那行改判为 `failed`，
    /// 让统计如实记一次失败请求。这条测试锁住三件事：B 一次都没被调用、流水只有 A 一行、
    /// 那一行最终是 `failed`。
    #[tokio::test]
    async fn a_stream_that_breaks_midway_is_not_retried_on_the_next_member() {
        crate::gateway::route_log::flush();
        let mut deps = Arc::try_unwrap(crate::proxy::test_support::bare_deps())
            .unwrap_or_else(|_| panic!("test dependencies must have a single owner"));
        let ends = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
        let sink = ends.clone();
        deps.on_activity = Arc::new(move |name, value| {
            if name == crate::gateway::activity::END_EVENT {
                sink.lock().unwrap().push(value);
            }
        });
        let deps = Arc::new(deps);
        let (base_a, hits_a) = counting_upstream(true).await;
        let (base_b, hits_b) = counting_upstream(false).await;
        add_group(&deps, "p2-midstream", "order", &[("p2m-a", &base_a), ("p2m-b", &base_b)]);

        let response = send_chat(&deps, "p2-midstream", None, true).await;
        if response.status() != StatusCode::OK {
            let status = response.status();
            let text = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap_or_default();
            panic!("DEBUG {status} {}", String::from_utf8_lossy(&text));
        }
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await;
        assert!(body.is_err(), "上游中途断开，Agent 读 body 应得到错误而不是一个看似完整的回答");
        assert_eq!(hits_a.load(Ordering::SeqCst), 1);
        assert_eq!(hits_b.load(Ordering::SeqCst), 0, "流已开始后不能换成员重发（会重复输出、重复计费）");

        crate::gateway::route_log::flush();
        let conn = deps.db.lock().unwrap();
        let rows: Vec<(String, String)> = {
            let mut stmt = conn.prepare("SELECT source, result FROM gateway_route_log ORDER BY id").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
        };
        assert_eq!(rows, vec![("p2m-a".to_string(), "failed".to_string())], "只有 A 一行，且改判为 failed");
        let now = chrono::Local::now().timestamp_millis();
        let day = crate::gateway::route_log::day_overview(&conn, now - 60_000, 0, now + 60_000).unwrap();
        assert_eq!((day.today, day.reroutes, day.errors), (1, 0, 1), "一次请求、没有转移、一次错误");
        // P4 审计 D2 情形 B：改判后补发同 request_id 的结束事件，前端据此把那次 ok 换成失败
        let ends = ends.lock().unwrap();
        let results: Vec<&str> = ends.iter().filter_map(|e| e["result"].as_str()).collect();
        assert_eq!(results, ["ok", "failed"], "响应头阶段一次 ok，改判后再补一次 failed：{ends:?}");
        assert_eq!(ends[0]["request_id"], ends[1]["request_id"]);
    }

    /// P3 盘点 S1 / S7：转 Chat 时省略的服务端工具照常省略，但这次尝试的决策说明里写明省略了什么。
    #[tokio::test]
    async fn server_tools_dropped_in_conversion_are_written_to_the_log() {
        crate::gateway::route_log::flush();
        let deps = crate::proxy::test_support::bare_deps();
        let (base, hits) = counting_upstream(false).await;
        add_group(&deps, "p3-drop", "order", &[("p3d-a", &base)]);

        let cases: [(CallerProtocol, &str, &str, &str); 2] = [
            (
                CallerProtocol::Responses,
                "/v1/responses",
                r#"{"model":"group/p3-drop","input":"hi","tools":[{"type":"function","name":"shell","parameters":{"type":"object"}},{"type":"web_search"},{"type":"file_search"}]}"#,
                "web_search、file_search",
            ),
            (
                CallerProtocol::Anthropic,
                "/v1/messages",
                r#"{"model":"group/p3-drop","max_tokens":16,"messages":[{"role":"user","content":"hi"}],"tools":[{"name":"Read","input_schema":{"type":"object"}},{"type":"bash_20250124","name":"bash"}]}"#,
                "bash",
            ),
        ];
        for (protocol, path, body, dropped) in cases {
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                handle(protocol, deps.clone(), HeaderMap::new(), axum::body::Bytes::from(body), path, None),
            )
            .await
            .expect("请求应在 10 秒内完成");
            assert_eq!(response.status(), StatusCode::OK, "{path} 照常转发，不因省略工具而拒绝");
            axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            crate::gateway::route_log::flush();
            let decision: Option<String> = deps
                .db
                .lock()
                .unwrap()
                .query_row("SELECT decision FROM gateway_route_log ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
                .unwrap();
            let decision = decision.unwrap_or_default();
            assert!(decision.contains("分组 p3-drop"), "原有的分组解释要保留：{decision}");
            assert!(decision.contains(&format!("已省略服务端工具：{dropped}")), "{path}：{decision}");
        }
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn plain_tools_are_not_reported_as_dropped() {
        let anthropic = serde_json::json!({"tools": [{"name": "Read", "input_schema": {}}, {"type": "custom", "name": "x"}, {"name": "DeferredToolPlaceholder", "type": "x"}]});
        assert!(crate::gateway::translate::anthropic_tools_dropped_for_chat(&anthropic).is_empty());
        let responses = serde_json::json!({"tools": [{"type": "function", "name": "a"}, {"type": "custom", "name": "b"}]});
        assert!(crate::gateway::responses_chat::builtin_tools_dropped_for_chat(&responses).is_empty());
    }

    /// F-1：`pace` 分组里缺剩余量的成员，选路时触发一次后台补查；1 分钟内不重复。
    ///
    /// 为什么必须在请求路径上触发：后端没有定时查额度的任务，`finish_quota_query`
    /// 只在有人调 `connection_quota` 时才跑。主窗口不在供应商页时，分组里往往只有
    /// 灵动岛那一条连接有新数据——不补查的话 `pace` 永远只有一条连接的数据可用。
    ///
    /// 回调必须非阻塞：这里用 mpsc 收集连接 id，请求本身照常完成。
    #[tokio::test]
    async fn pace_group_triggers_one_background_quota_refresh_per_member_within_a_minute() {
        crate::gateway::quota_snapshot::forget_stale_triggers();
        let mut deps = Arc::try_unwrap(crate::proxy::test_support::bare_deps())
            .unwrap_or_else(|_| panic!("test dependencies must have a single owner"));
        let (stale_tx, stale_rx) = std::sync::mpsc::channel::<String>();
        deps.on_quota_stale = Arc::new(move |connection_id: &str| {
            stale_tx.send(connection_id.to_string()).ok();
        });
        let deps = Arc::new(deps);

        // 两个假上游：都返回 200。这里只关心「有没有触发补查」，不关心转发结果。
        let mut upstreams = Vec::new();
        for _ in 0..2 {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move {
                loop {
                    let Ok((mut socket, _)) = listener.accept().await else { break };
                    use tokio::io::AsyncWriteExt;
                    let _ = socket
                        .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}")
                        .await;
                }
            });
            upstreams.push(base);
        }

        // 两条支持额度查询的订阅连接（zcode: subscription_quota），故意都不写快照
        for (index, base) in upstreams.iter().enumerate() {
            let id = format!("pace-source-{index}");
            deps.db.lock().unwrap().execute(
                "INSERT INTO connections(id,platform,kind,name,secret,base_url,protocol,status,serve_enabled,created_ms,slug)
                 VALUES(?1,'zcode','auth',?1,'fixture-token',?2,'openai','connected',1,1,?1)",
                rusqlite::params![id, base],
            ).unwrap();
        }
        {
            let conn = deps.db.lock().unwrap();
            let members = vec![
                crate::gateway::routing_groups::GroupMember {
                    slug: "pace-source-0".into(), model: "glm-5.3".into(),
                    paused_until_ms: None, weight: None, tier: None,
                },
                crate::gateway::routing_groups::GroupMember {
                    slug: "pace-source-1".into(), model: "glm-5.3".into(),
                    paused_until_ms: None, weight: None, tier: None,
                },
            ];
            crate::gateway::routing_groups::create(&conn, "pace-probe", "补查探测", "pace", &members, None).unwrap();
        }

        let request = || {
            handle(
                CallerProtocol::Chat,
                deps.clone(),
                HeaderMap::new(),
                axum::body::Bytes::from_static(
                    br#"{"model":"group/pace-probe","messages":[{"role":"user","content":"hi"}]}"#,
                ),
                "/v1/chat/completions",
                None,
            )
        };
        // 第一次：两个成员都没有剩余量 → 两个连接都应被触发
        // 补查在**选路阶段**触发（回调是同步 send），不需要等响应完成；给 300ms 足够。
        let _ = tokio::time::timeout(std::time::Duration::from_millis(300), request()).await;
        let mut first: Vec<String> = Vec::new();
        while let Ok(id) = stale_rx.try_recv() {
            first.push(id);
        }
        first.sort();
        assert_eq!(
            first,
            vec!["pace-source-0".to_string(), "pace-source-1".to_string()],
            "两个缺数据的成员都要后台补查，实际收到 {first:?}",
        );

        // 第二次（1 分钟内）：冷却中，不应再触发
        // 补查在**选路阶段**触发（回调是同步 send），不需要等响应完成；给 300ms 足够。
        let _ = tokio::time::timeout(std::time::Duration::from_millis(300), request()).await;
        let mut second: Vec<String> = Vec::new();
        while let Ok(id) = stale_rx.try_recv() {
            second.push(id);
        }
        assert!(second.is_empty(), "1 分钟内不应重复补查，实际收到 {second:?}");
        crate::gateway::quota_snapshot::forget_stale_triggers();
    }

    #[tokio::test]
    async fn closing_or_pausing_a_source_rejects_warmed_pool_before_upstream() {
        let deps=crate::proxy::test_support::bare_deps();
        let listener=tokio::net::TcpListener::bind(("127.0.0.1",0)).await.unwrap();
        let base=format!("http://{}",listener.local_addr().unwrap());
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,base_url,protocol,status,serve_enabled,created_ms,slug) VALUES('isolation-source','custom','api','isolated','fixture-token',?1,'openai','connected',1,1,'isolated')",[base]).unwrap();
        let pool=route::pool_snapshot(&deps.db,deps.pool_index().unwrap());
        assert_eq!(pool.len(),1);
        for (enabled,status) in [(false,"connected"),(true,"paused")] {
            deps.db.lock().unwrap().execute("UPDATE connections SET serve_enabled=?1,status=?2 WHERE id='isolation-source'",rusqlite::params![enabled,status]).unwrap();
            let response=tokio::time::timeout(std::time::Duration::from_millis(500),handle(CallerProtocol::Chat,deps.clone(),HeaderMap::new(),axum::body::Bytes::from_static(br#"{"model":"isolated/fixture","messages":[{"role":"user","content":"test"}]}"#),"/v1/chat/completions",None)).await;
            assert!(response.is_ok(),"不应等待上游；缓存中的来源已经关闭或暂停");
            assert_eq!(response.unwrap().status(),StatusCode::SERVICE_UNAVAILABLE);
            assert!(tokio::time::timeout(std::time::Duration::from_millis(20),listener.accept()).await.is_err(),"关闭后的来源不能收到新请求");
        }
    }
    use std::sync::atomic::{AtomicU32, Ordering};
    fn json_response(value: serde_json::Value) -> Response {
        Response::builder().status(StatusCode::OK).header("content-type", "application/json").body(axum::body::Body::from(value.to_string())).unwrap()
    }

    /// P3 盘点 D2：超过整体时限就不再开新一轮，用已有结果收尾、去掉悬空的接管调用并说明。
    #[tokio::test]
    async fn the_tool_loop_stops_when_its_time_budget_is_spent() {
        let body = axum::body::Bytes::from(serde_json::json!({
            "model": "m", "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "web_search_20250305", "name": "web_search"}]
        }).to_string());
        let rounds = std::sync::atomic::AtomicU32::new(0);
        let response = with_tool_loop_within(&body, std::time::Duration::ZERO, |_| {
            rounds.fetch_add(1, Ordering::SeqCst);
            async {
                json_response(serde_json::json!({"id": "msg", "type": "message", "role": "assistant", "model": "m", "stop_reason": "tool_use",
                    "content": [{"type": "text", "text": "先搜一下"}, {"type": "tool_use", "id": "s", "name": "web_search", "input": {"query": "q"}}],
                    "usage": {"input_tokens": 3, "output_tokens": 4}}))
            }
        })
        .await;
        assert_eq!(rounds.load(Ordering::SeqCst), 1, "时限已到，不再执行搜索、不再问下一轮");
        let message: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(message["stop_reason"], "end_turn", "去掉接管调用后不能留着 tool_use 结束原因");
        let content = message["content"].as_array().unwrap();
        assert!(content.iter().all(|block| block["type"] != "tool_use"));
        assert!(content[0]["text"].as_str().unwrap().contains("累计耗时超过 0 秒"));
        assert_eq!(content[1]["text"], "先搜一下", "已有的回答保留");
    }

    #[tokio::test]
    async fn without_server_tools_the_request_goes_once_unchanged() {
        let body = axum::body::Bytes::from(r#"{"model":"m","stream":true,"messages":[],"tools":[{"name":"Read"}]}"#);
        let seen = std::sync::Mutex::new(Vec::<axum::body::Bytes>::new());
        let response = with_tool_loop(&body, |round| {
            seen.lock().unwrap().push(round);
            async { json_response(serde_json::json!({"content": []})) }
        })
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(seen.lock().unwrap().as_slice(), &[body], "原样只发一次");
    }

    #[tokio::test]
    async fn a_round_ending_in_foreign_tools_drops_managed_calls_and_streams_back() {
        let body = axum::body::Bytes::from(serde_json::json!({
            "model": "m", "stream": true, "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "Read", "input_schema": {}}, {"type": "web_search_20250305", "name": "web_search"}]
        }).to_string());
        let sent = std::sync::Mutex::new(Vec::<serde_json::Value>::new());
        let response = with_tool_loop(&body, |round| {
            sent.lock().unwrap().push(serde_json::from_slice(&round).unwrap());
            async {
                json_response(serde_json::json!({"id": "msg", "type": "message", "role": "assistant", "model": "m", "stop_reason": "tool_use",
                    "content": [{"type": "tool_use", "id": "a", "name": "Read", "input": {}}, {"type": "tool_use", "id": "b", "name": "web_search", "input": {"query": "q"}}],
                    "usage": {"input_tokens": 3, "output_tokens": 4}}))
            }
        })
        .await;
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["stream"], false, "每轮按非流式问上游");
        assert!(sent[0]["tools"][1].get("type").is_none(), "服务端工具换成了普通工具");
        assert_eq!(response.headers().get("content-type").unwrap(), "text/event-stream", "客户端要流式就回 SSE");
        let text = String::from_utf8(axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
        assert!(text.contains(r#""name":"Read""#) && !text.contains(r#""name":"web_search""#), "接管的调用已去掉");
    }

    #[tokio::test]
    async fn managed_calls_are_executed_and_the_next_round_answers() {
        // 空数据目录：没有搜索后端，执行结果是「未配置」的错误，照样续问下一轮
        let dir = std::env::temp_dir().join(format!("jolu-tool-loop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let previous = std::env::var_os("JOLU_DATA_DIR");
        std::env::set_var("JOLU_DATA_DIR", &dir);
        let body = axum::body::Bytes::from(serde_json::json!({
            "model": "m", "messages": [{"role": "user", "content": "搜一下"}],
            "tools": [{"type": "web_search_20250305", "name": "web_search"}]
        }).to_string());
        let sent = std::sync::Mutex::new(Vec::<serde_json::Value>::new());
        let response = with_tool_loop(&body, |round| {
            let value: serde_json::Value = serde_json::from_slice(&round).unwrap();
            let first = value["messages"].as_array().unwrap().len() == 1;
            sent.lock().unwrap().push(value);
            async move {
                if first {
                    json_response(serde_json::json!({"content": [{"type": "tool_use", "id": "s1", "name": "web_search", "input": {"query": "rust"}}],
                        "stop_reason": "tool_use", "usage": {"input_tokens": 10, "output_tokens": 2}}))
                } else {
                    json_response(serde_json::json!({"content": [{"type": "text", "text": "没法搜，但我知道"}], "stop_reason": "end_turn",
                        "usage": {"input_tokens": 20, "output_tokens": 5}}))
                }
            }
        })
        .await;
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 2, "执行后续问一轮");
        let second = sent[1]["messages"].as_array().unwrap();
        assert_eq!(second[2]["content"][0]["tool_use_id"], "s1");
        assert_eq!(second[2]["content"][0]["is_error"], true, "没配置搜索后端如实告诉模型");
        assert_eq!(response.headers().get("content-type").unwrap(), "application/json", "客户端没要流式就回 JSON");
        let message: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert!(message["content"][0]["text"].as_str().unwrap().contains("搜索「rust」失败"));
        assert_eq!(message["content"][1]["text"], "没法搜，但我知道");
        assert_eq!((message["usage"]["input_tokens"].as_i64(), message["usage"]["output_tokens"].as_i64()), (Some(30), Some(7)), "用量取两轮之和");
        match previous {
            Some(value) => std::env::set_var("JOLU_DATA_DIR", value),
            None => std::env::remove_var("JOLU_DATA_DIR"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn upstream_errors_inside_the_loop_are_returned_as_is() {
        let body = axum::body::Bytes::from(serde_json::json!({"model": "m", "messages": [], "tools": [{"type": "web_fetch_20250910", "name": "web_fetch"}]}).to_string());
        let response = with_tool_loop(&body, |_| async { refusal_400() }).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    fn refusal_400() -> Response {
        Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(axum::body::Body::from(r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages.1.content.0: Invalid `signature` in `thinking` block"}}"#))
            .unwrap()
    }

    fn signed_by_another_account() -> axum::body::Bytes {
        axum::body::Bytes::from(serde_json::json!({
            "model": "claude-opus-5-5",
            "messages": [
                {"role": "user", "content": "你好"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "另一个账号签的", "signature": "c2lnbmVkLWJ5LWFub3RoZXItYWNjb3VudA"},
                    {"type": "text", "text": "回答"}
                ]},
                {"role": "user", "content": "继续"}
            ]
        }).to_string())
    }

    #[tokio::test]
    async fn claude_refusing_a_thinking_signature_is_mended_once_and_remembered() {
        let sent = std::sync::Mutex::new(Vec::<serde_json::Value>::new());
        let send = |body: axum::body::Bytes| {
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let thinking = value.to_string().contains("\"thinking\"");
            sent.lock().unwrap().push(value);
            async move { if thinking { refusal_400() } else { Response::new(axum::body::Body::from("ok")) } }
        };
        let body = signed_by_another_account();
        let response = with_thinking_repair("repair-owner", Some("https://api.anthropic.com"), "claude-opus-5-5", &body, send).await;
        assert_eq!(response.status(), StatusCode::OK, "剔掉被拒的块后重发成功");
        {
            let sent = sent.lock().unwrap();
            assert_eq!(sent.len(), 2, "只重发一次");
            assert_eq!(sent[1]["messages"][1]["content"].as_array().unwrap().len(), 1);
            assert_eq!(sent[1]["messages"][1]["content"][0]["type"], "text", "text 保留");
        }
        // 同一来源的下一轮：首发就不带被拒过的块
        let response = with_thinking_repair("repair-owner", Some("https://api.anthropic.com"), "claude-opus-5-5", &body, send).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(sent.lock().unwrap().len(), 3, "记住之后不再先挨一次 400");
    }

    #[tokio::test]
    async fn repeated_refusal_and_other_errors_are_returned_as_is() {
        let calls = AtomicU32::new(0);
        let always_refuse = |_body: axum::body::Bytes| {
            calls.fetch_add(1, Ordering::Relaxed);
            async { refusal_400() }
        };
        let body = signed_by_another_account();
        let response = with_thinking_repair("repair-owner-loop", Some("https://api.anthropic.com"), "claude-opus-5-5", &body, always_refuse).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "剔过仍拒：原样交回 400");
        assert_eq!(calls.load(Ordering::Relaxed), 2, "最多两次，不循环");
        let text = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(String::from_utf8_lossy(&text).contains("Invalid `signature`"), "错误体原样保留");
        // 非 Claude 上游：不剔、不重试
        let calls = AtomicU32::new(0);
        let refuse = |_body: axum::body::Bytes| {
            calls.fetch_add(1, Ordering::Relaxed);
            async { refusal_400() }
        };
        let response = with_thinking_repair("repair-owner-glm", Some("https://open.bigmodel.cn/api/anthropic"), "glm-5.3", &body, refuse).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }


    fn deps_with_circuit(threshold: u32, hits: Arc<AtomicU32>, enabled: bool) -> Arc<ProxyDeps> {
        let mut deps = Arc::try_unwrap(crate::proxy::test_support::bare_deps()).unwrap_or_else(|_| {
            // bare_deps 是 Arc，拿不到所有权时重建一份最小 deps 供熔断计数测试
            crate::proxy::test_support::bare_deps();
            unreachable!()
        });
        deps.circuit_threshold = threshold;
        deps.fallback_enabled = enabled;
        deps.on_circuit = Arc::new(move |_| {
            hits.fetch_add(1, Ordering::Relaxed);
        });
        Arc::new(deps)
    }

    #[test]
    fn circuit_counts_consecutive_failures_and_resets_on_success() {
        let hits = Arc::new(AtomicU32::new(0));
        let deps = deps_with_circuit(3, hits.clone(), true);
        note_outcome(&deps, true, "a");
        note_outcome(&deps, true, "b");
        assert_eq!(hits.load(Ordering::Relaxed), 0, "未达阈值不熔断");
        note_outcome(&deps, true, "c");
        assert_eq!(hits.load(Ordering::Relaxed), 1, "达阈值熔断一次");
        // 熔断后计数归零，不会连续触发
        note_outcome(&deps, true, "d");
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        // 成功清零
        note_outcome(&deps, false, "");
        note_outcome(&deps, true, "e");
        assert_eq!(hits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn circuit_is_off_when_auto_escape_disabled() {
        let hits = Arc::new(AtomicU32::new(0));
        let deps = deps_with_circuit(1, hits.clone(), false);
        for _ in 0..5 {
            note_outcome(&deps, true, "x");
        }
        assert_eq!(hits.load(Ordering::Relaxed), 0, "关闭自动逃生时绝不自动切断");
    }

    #[test]
    fn agent_from_headers_reads_token_originator_and_user_agent() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "jolu-claude-code.abcd1234".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("claude-code"));
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer jolu-codex.abcd1234".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("codex"));
        let mut headers = HeaderMap::new();
        headers.insert("originator", "codex_cli_rs".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("codex"));
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "GeminiCLI/0.34.0/... (win32; x64)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("gemini"));
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "CloudCodeVSCode/0.65.0 (aidev_client; os_type=Windows; proxy_client=geminicli)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("gemini"));
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "claude-cli/2.1.0 (external, cli)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("claude-code"));
        assert_eq!(agent_from_headers(&HeaderMap::new()).agent, None);
    }

    #[test]
    fn codex_masquerading_as_claude_code_is_corrected() {
        // OpenAI /codex:review 插件在 Claude Code 里跑 codex app-server：
        // originator 和 UA 都写 "Claude Code/..."，但 Codex 每轮都带这两个头
        // （现场记录 wyztony on X：Codex 的调用量从来没记上）
        let mut headers = HeaderMap::new();
        headers.insert("originator", "Claude Code/0.162.1".parse().unwrap());
        headers.insert("user-agent", "Claude Code/0.162.1 (external, cli)".parse().unwrap());
        headers.insert("x-codex-window-id", "win-1".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("codex"), "有 Codex 特有头 + UA 写 Claude Code → 是 Codex");
        // turn-metadata 也能单独触发
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "Claude Code/0.162.1".parse().unwrap());
        headers.insert("x-codex-turn-metadata", "{}".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("codex"));
        // 没有 Codex 特有头：还是 Claude Code（不误判）
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "claude-cli/2.1.114 (external, cli)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("claude-code"));
    }

    #[test]
    fn vscode_agent_host_resolves_to_codex() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "vscode_agent_host/1.2.3".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("codex"));
    }

    #[test]
    fn identified_by_carries_the_matching_rule() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "jolu-claude-code.abcd".parse().unwrap());
        let identity = agent_from_headers(&headers);
        assert_eq!(identity.identified_by, Some("token"), "token 识别的标注 token");
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "codex_cli_rs/0.1".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).identified_by, Some("ua"));
    }

    /// 通用 UA 表：13 家里 magpie 声明过 UA 的 9 家都要认出来。
    ///
    /// 这条锁住的是「接线成功但流水认不出是谁」——本机实测有 103 条 agent 为 NULL，
    /// 其中 ZCode 42 条、WorkBuddy 53 条。
    #[test]
    fn every_declared_ua_prefix_names_its_agent() {
        for (ua, want) in [
            ("claude-cli/2.1.114 (external, cli)", "claude-code"),
            ("claude-code/1.0", "claude-code"),
            ("codex_cli_rs/0.1", "codex"),
            ("codex/1.0", "codex"),
            ("acp-extension-codex/0.1", "codex"),
            ("GeminiCLI/0.34.0", "gemini"),
            ("gemini-cli/0.1", "gemini"),
            ("opencode/1.0", "opencode"),
            ("zcode/1.2.3", "zcode"),
            // 大小写不敏感：真实 UA 有写 ZCode 的
            ("ZCode/1.2.3", "zcode"),
            ("workbuddy/1.0", "workbuddy"),
            ("qwencode/1.0", "qwen"),
            ("kimicli/1.0", "kimi"),
            ("deepseek-harness/1.0", "dsh"),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("user-agent", ua.parse().unwrap());
            assert_eq!(
                agent_from_headers(&headers).agent.as_deref(),
                Some(want),
                "UA {ua} 应认成 {want}"
            );
        }
        // 子串不该误匹配：`my-zcode-fork` 不是 ZCode
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "my-zcode-fork/1.0".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent, None, "只在产品名上精确匹配，不做子串");
        // Gemini 的 proxy_client 在中段，要单独匹配
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "CloudCodeVSCode/1.0 (proxy_client=geminicli)".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("gemini"));
    }

    #[test]
    fn unidentified_ua_carries_first_segment_for_diagnostics() {
        // 识别不出：给 UA 首段（到 / 或空格），让用户报问题时直接看出是哪个工具
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "some-new-tool/1.0 (darwin; arm64)".parse().unwrap());
        let identity = agent_from_headers(&headers);
        assert_eq!(identity.agent, None);
        assert_eq!(identity.unidentified_ua.as_deref(), Some("some-new-tool"), "取 UA 首段");
        // 空 UA：不填
        assert_eq!(agent_from_headers(&HeaderMap::new()).unidentified_ua, None);
    }

    #[test]
    fn session_identity_only_trusts_native_session_ids() {
        let none = HeaderMap::new();
        // Claude Code：JSON user_id 里的 session_id；旧格式 _session_ 后缀；专用头优先
        let body = br#"{"metadata":{"user_id":"{\"device_id\":\"d\",\"session_id\":\"ses-1\"}"}}"#;
        let got = session_identity(Some("claude-code"), &none, body).unwrap();
        assert_eq!((got.platform.as_str(), got.session_id.as_str()), ("claude", "ses-1"));
        let legacy = br#"{"metadata":{"user_id":"user_abc_account_11-22_session_33-44"}}"#;
        assert_eq!(session_identity(Some("claude-code"), &none, legacy).unwrap().session_id, "33-44");
        let mut headers = HeaderMap::new();
        headers.insert("x-claude-code-session-id", "hdr-ses".parse().unwrap());
        assert_eq!(session_identity(Some("claude-code"), &headers, body).unwrap().session_id, "hdr-ses");
        // 普通 user_id 不是会话
        assert!(session_identity(Some("claude-code"), &none, br#"{"metadata":{"user_id":"someone"}}"#).is_none());

        // Codex：只认根会话头；请求级 ID 与缓存键不算
        let mut headers = HeaderMap::new();
        headers.insert("session-id", "root-1".parse().unwrap());
        headers.insert("thread-id", "child-1".parse().unwrap());
        let got = session_identity(Some("codex"), &headers, b"{}").unwrap();
        assert_eq!((got.platform.as_str(), got.session_id.as_str()), ("codex", "root-1"));
        let mut headers = HeaderMap::new();
        headers.insert("x-client-request-id", "req-1".parse().unwrap());
        assert!(session_identity(Some("codex"), &headers, br#"{"prompt_cache_key":"cache-1"}"#).is_none());

        // OpenCode：本会话 ID；子会话归到父会话；只有亲和头不算
        let mut headers = HeaderMap::new();
        headers.insert("x-opencode-session-id", "ses_child".parse().unwrap());
        assert_eq!(session_identity(Some("opencode"), &headers, b"{}").unwrap().session_id, "opencode:ses_child");
        headers.insert("x-opencode-parent-session-id", "ses_root".parse().unwrap());
        let got = session_identity(Some("opencode"), &headers, b"{}").unwrap();
        assert_eq!((got.platform.as_str(), got.session_id.as_str()), ("opencode", "opencode:ses_root"));
        let mut headers = HeaderMap::new();
        headers.insert("x-opencode-session", "affinity-root".parse().unwrap());
        assert!(session_identity(Some("opencode"), &headers, b"{}").is_none());
        let mut headers = HeaderMap::new();
        headers.insert("x-opencode-client", "cli".parse().unwrap());
        assert_eq!(agent_from_headers(&headers).agent.as_deref(), Some("opencode"));

        // 识别不出 Agent，或别的 Agent 只有通用头：不猜
        let mut headers = HeaderMap::new();
        headers.insert("session-id", "root-1".parse().unwrap());
        assert!(session_identity(None, &headers, body).is_none());
        assert!(session_identity(Some("gemini"), &headers, body).is_none());
    }

    #[test]
    fn session_key_prefers_anthropic_metadata_user_id() {
        let body = br#"{"model":"kimi/kimi-k2","metadata":{"user_id":"user-abc"}}"#;
        let key = session_key(CallerProtocol::Anthropic, &HeaderMap::new(), body, "kimi/kimi-k2");
        assert_eq!(key, "user:user-abc");
        // Claude Code 把 user_id 写成 JSON：取里面的 session_id
        let body = br#"{"model":"kimi/kimi-k2","metadata":{"user_id":"{\"device_id\":\"d\",\"session_id\":\"ses-1\"}"}}"#;
        let key = session_key(CallerProtocol::Anthropic, &HeaderMap::new(), body, "kimi/kimi-k2");
        assert_eq!(key, "user:ses-1");
        // 完全没有会话字段 → 退化为令牌 + slug
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer jolu-codex".parse().unwrap());
        let body = br#"{"model":"codex/gpt-5.5"}"#;
        let key = session_key(CallerProtocol::Chat, &headers, body, "codex/gpt-5.5");
        assert_eq!(key, "tok:jolu-codex:codex");
    }

    #[test]
    fn session_key_prefers_codex_and_generic_session_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("session-id", "root-session".parse().unwrap());
        headers.insert("thread-id", "child-thread".parse().unwrap());
        headers.insert("x-client-request-id", "child-thread".parse().unwrap());
        let body = br#"{"model":"codex/gpt-5.5","prompt_cache_key":"cache-1"}"#;
        assert_eq!(
            session_key(CallerProtocol::Responses, &headers, body, "codex/gpt-5.5"),
            "hdr:codex-session:root-session",
            "session-id 是根会话，优先于子线程"
        );

        let mut headers = HeaderMap::new();
        headers.insert("session-id", "root-session".parse().unwrap());
        assert_eq!(
            session_key(CallerProtocol::Chat, &headers, br#"{"prompt_cache_key":"cache-1"}"#, "codex/m"),
            "hdr:codex-session:root-session"
        );

        let mut headers = HeaderMap::new();
        headers.insert("x-session-affinity", "affinity-1".parse().unwrap());
        assert_eq!(
            session_key(CallerProtocol::Chat, &headers, b"{}", "codex/m"),
            "hdr:affinity:affinity-1"
        );

        // 只有请求体里的 prompt_cache_key（Codex 的线程 id）也能区分会话
        assert_eq!(
            session_key(CallerProtocol::Chat, &HeaderMap::new(), br#"{"prompt_cache_key":"cache-9"}"#, "codex/m"),
            "body:prompt-cache:cache-9"
        );

        // 显式覆盖头优先于所有 CLI 字段
        let mut headers = HeaderMap::new();
        headers.insert("x-jolu-session", "explicit".parse().unwrap());
        headers.insert("session-id", "root-session".parse().unwrap());
        assert_eq!(
            session_key(CallerProtocol::Chat, &headers, b"{}", "codex/m"),
            "hdr:x-jolu-session:explicit"
        );
    }
