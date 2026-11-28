//! 本地代理与模型入口网关宿主模块。
//!
//! - [`deps`]：零上层网关依赖的 `ProxyDeps`、平台识别、去重键提取与待关联计时队列；
//! - [`observe`]：纯观测模式透传与零缓冲计时；
//! - [`host`]：监听绑定、路由切换与 CLI 配置自愈。

pub(crate) mod config;
pub(crate) mod deps;
pub(crate) mod host;
pub(crate) mod observe;

#[cfg(test)]
pub(crate) use deps::pending_len;
#[cfg(test)]
pub(crate) use deps::test_support;
pub use deps::flush_pending;
pub use deps::stop;
pub(crate) use deps::{
    detached_deps, extract_dedup_key, extract_served_model, queue_pending_timing,
    queue_pending_timing_with_models, running_deps, running_port, ProxyDeps, HEAD_CAP,
};
pub use host::{
    on_config_changed, set_gateway_port, set_proxy, shutdown_and_restore, start, startup, status,
    suggest_port, GatewayPortOutcome, ProxyStatus,
};
pub(crate) use host::register_quota_refresher;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::extract::DefaultBodyLimit;
    use axum::http::{HeaderMap, StatusCode};
    use axum::Router;
    use futures_util::StreamExt;

    use super::deps::{
        detect_platform, extract_dedup_key, flush_entries, now_ms, ClaudeSessions, KeyIndex,
        PendingTiming, ProxyDeps, ProxyRuntime, FALLBACK_THRESHOLD, PENDING_TTL_MS,
        REQUEST_BODY_LIMIT,
    };
    use super::host::{
        bind_listener, follow_external_configs, format_bind_error, heal_observed_orphans,
        probe_port, suggest_port,
    };
    use super::observe::{build_upstream_url, forward};
    use crate::proxy_config::{is_self_base, ProxyPlatform};

    #[test]
    fn format_bind_error_explains_windows_permission_denied() {
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "os error 10013");
        let message = format_bind_error("127.0.0.1", 12731, &error);
        assert!(message.contains("12731") && message.contains("系统保留"), "{message}");
    }

    #[test]
    fn suggest_port_returns_bindable_port_different_from_current() {
        let picked = suggest_port("127.0.0.1", 18731).unwrap();
        assert_ne!(picked, 18731);
        assert!(picked >= 1024);
        assert!(probe_port("127.0.0.1", picked).is_ok());
    }

    #[test]
    fn probe_port_rejects_already_bound_port() {
        let taken = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let error = probe_port("127.0.0.1", port).unwrap_err();
        assert!(error.contains("已被占用"), "{error}");
        drop(taken);
    }

    #[tokio::test]
    async fn relay_refuses_an_occupied_port_and_observe_falls_back_to_a_random_one() {
        let taken = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = taken.local_addr().unwrap().port();
        let error = bind_listener("127.0.0.1", port, crate::gateway::Mode::Relay)
            .await
            .unwrap_err();
        assert!(error.contains("已被占用") && error.contains("固定端口"), "{error}");
        let (_listener, actual) =
            bind_listener("127.0.0.1", port, crate::gateway::Mode::Observe)
                .await
                .unwrap();
        assert_ne!(actual, port, "观测模式换随机端口继续");
        drop(taken);
    }

    fn isolated_configs(test: &str, check: impl FnOnce(&std::path::Path)) {
        const ROOT: &str = "JOLU_PROXY_CONFIG_TEST_ROOT";
        if let Some(root) = std::env::var_os(ROOT) {
            check(std::path::Path::new(&root));
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "jolu-proxy-{}-{test}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(dir.join("claude")).unwrap();
        std::fs::create_dir_all(dir.join("codex")).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("proxy::tests::{test}"), "--nocapture"])
            .env(ROOT, &dir)
            .env("CLAUDE_CONFIG_DIR", dir.join("claude"))
            .env("CODEX_HOME", dir.join("codex"))
            .output()
            .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn config_runtime(mode: crate::gateway::Mode) -> ProxyRuntime {
        let (deps, _rx) = test_deps(
            Arc::new(Mutex::new(memory_db())),
            HashMap::from([
                (ProxyPlatform::Claude, "https://old-claude.example".into()),
                (ProxyPlatform::Codex, "https://old-codex.example/v1".into()),
            ]),
            None,
        );
        ProxyRuntime {
            port: 12731,
            mode,
            shutdown: deps.shutdown.clone(),
            deps,
        }
    }

    fn write_configs(dir: &std::path::Path, base: &str, token: &str) {
        std::fs::write(
            dir.join("claude/settings.json"),
            serde_json::json!({
                "env": {"ANTHROPIC_BASE_URL": base, "ANTHROPIC_AUTH_TOKEN": token},
                "model": "fixture-model"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.join("codex/config.toml"),
            format!(
                "model_provider = \"fixture\"\nmodel = \"fixture-model\"\n[model_providers.fixture]\nbase_url = \"{base}\"\nenv_key = \"FIXTURE_API_KEY\"\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn config_follow_respects_runtime_mode_and_keeps_credentials() {
        isolated_configs("config_follow_respects_runtime_mode_and_keeps_credentials", |dir| {
            for mode in [crate::gateway::Mode::Relay, crate::gateway::Mode::Observe] {
                let store = crate::settings::Store::load(&dir.join(format!("{mode:?}")));
                store.try_update(|s| s.proxy_mode = "observe".into()).unwrap();
                write_configs(dir, "https://new.example/v1/", "fixture-upstream-key");
                let changed = follow_external_configs(&store, &config_runtime(mode));
                let claude: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(dir.join("claude/settings.json")).unwrap())
                        .unwrap();
                let codex = std::fs::read_to_string(dir.join("codex/config.toml"))
                    .unwrap()
                    .parse::<toml_edit::DocumentMut>()
                    .unwrap();
                let expected = if mode == crate::gateway::Mode::Relay {
                    "https://new.example/v1/"
                } else {
                    "http://127.0.0.1:12731"
                };
                assert_eq!(claude["env"]["ANTHROPIC_BASE_URL"], expected);
                assert_eq!(codex["model_providers"]["fixture"]["base_url"].as_str(), Some(expected));
                assert_eq!(claude["env"]["ANTHROPIC_AUTH_TOKEN"], "fixture-upstream-key");
                assert_eq!(claude["model"], "fixture-model");
                assert_eq!(codex["model_providers"]["fixture"]["env_key"].as_str(), Some("FIXTURE_API_KEY"));
                assert_eq!(changed, mode == crate::gateway::Mode::Observe);
                assert_eq!(
                    store.get().proxy_claude_upstream.as_deref(),
                    if changed { Some("https://new.example/v1") } else { None }
                );
            }
        });
    }

    #[test]
    fn config_healing_respects_gateway_wiring_and_observe_orphans() {
        isolated_configs("config_healing_respects_gateway_wiring_and_observe_orphans", |dir| {
            for (mode, running) in [
                (crate::gateway::Mode::Relay, true),
                (crate::gateway::Mode::Relay, false),
                (crate::gateway::Mode::Observe, false),
            ] {
                let mut settings = crate::settings::Settings::default();
                settings.proxy_mode = if mode == crate::gateway::Mode::Relay {
                    "relay"
                } else {
                    "observe"
                }
                .into();
                settings.proxy_port = 12731;
                settings.proxy_claude_upstream = Some("https://old-claude.example".into());
                settings.proxy_codex_upstream = Some("https://old-codex.example/v1".into());
                write_configs(dir, "http://127.0.0.1:12731", "jolu-claude-code.fixture");
                let runtime = running.then(|| config_runtime(mode));
                heal_observed_orphans(&settings, runtime.as_ref());
                let expected = if mode == crate::gateway::Mode::Relay {
                    "http://127.0.0.1:12731"
                } else {
                    "https://old-claude.example"
                };
                assert_eq!(
                    crate::proxy_config::read_current_base(ProxyPlatform::Claude).unwrap().as_deref(),
                    Some(expected)
                );
                let expected = if mode == crate::gateway::Mode::Relay {
                    "http://127.0.0.1:12731"
                } else {
                    "https://old-codex.example/v1"
                };
                assert_eq!(
                    crate::proxy_config::read_current_base(ProxyPlatform::Codex).unwrap().as_deref(),
                    Some(expected)
                );
            }
        });
    }

    fn memory_db() -> rusqlite::Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn record(key: &str) -> crate::db::RequestRecord {
        crate::db::RequestRecord {
            platform: "claude".into(),
            source: crate::collector::SOURCE_LOCAL.into(),
            dedup_key: key.into(),
            session_id: None,
            ts: 1,
            model: None,
            input_tokens: Some(1),
            output_tokens: Some(1),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            total_tokens: Some(2),
            effort: None,
        }
    }

    fn test_deps(
        db: Arc<Mutex<rusqlite::Connection>>,
        upstreams: HashMap<ProxyPlatform, String>,
        fallback: Option<Arc<AtomicU32>>,
    ) -> (Arc<ProxyDeps>, tokio::sync::watch::Receiver<bool>) {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let deps = Arc::new(ProxyDeps {
            client,
            db,
            upstreams,
            pending: Arc::new(Mutex::new(Vec::new())),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
            fallback_enabled: fallback.is_some(),
            key_index: Mutex::new(KeyIndex::default()),
            shutdown: Arc::new(shutdown_tx),
            on_fallback: Arc::new(move |reason: &str| {
                assert!(reason.contains("上游不可达"));
                if let Some(hits) = &fallback {
                    hits.fetch_add(1, Ordering::Relaxed);
                }
            }),
            pool_index: None,
            gateway_default_connection: None,
            affinity: None,
            cooldowns: None,
            claude_sessions: Some(Mutex::new(ClaudeSessions::new())),
            circuit_threshold: 5,
            live_circuit_threshold: None,
            on_circuit: Arc::new(|_| {}),
            on_activity: Arc::new(|_, _| {}),
            on_quota_stale: Arc::new(|_| {}),
            gateway_auth_relaxed: false,
            gateway_lan_shared: false,
            request_uids: Default::default(),
        });
        (deps, shutdown_rx)
    }

    async fn spawn_proxy(deps: Arc<ProxyDeps>) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new()
            .fallback(forward)
            .layer(DefaultBodyLimit::max(REQUEST_BODY_LIMIT))
            .with_state(deps);
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    async fn spawn_chunked_upstream(
        first_delay_ms: u64,
        second_delay_ms: u64,
    ) -> (std::net::SocketAddr, tokio::sync::mpsc::UnboundedReceiver<&'static str>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<&'static str>();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let _ = socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n",
                )
                .await;
            tokio::time::sleep(Duration::from_millis(first_delay_ms)).await;
            let first = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream\"}}\n\n";
            let _ = socket.write_all(format!("{:x}\r\n", first.len()).as_bytes()).await;
            let _ = socket.write_all(first).await;
            let _ = socket.write_all(b"\r\n").await;
            tokio::time::sleep(Duration::from_millis(second_delay_ms)).await;
            event_tx.send("upstream_done").ok();
            let last = b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
            let _ = socket.write_all(format!("{:x}\r\n", last.len()).as_bytes()).await;
            let _ = socket.write_all(last).await;
            let _ = socket.write_all(b"\r\n0\r\n\r\n").await;
        });
        (addr, event_rx)
    }

    async fn spawn_static_upstream(status_line: &str, body: &str) -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let _ = socket.write_all(response.as_bytes()).await;
        });
        addr
    }

    #[test]
    fn platform_detection_prefers_headers_then_paths() {
        let mut headers = HeaderMap::new();
        headers.insert("anthropic-version", "2023-06-01".parse().unwrap());
        assert_eq!(detect_platform(&headers, "/v1/messages"), Some(ProxyPlatform::Claude));

        let mut headers = HeaderMap::new();
        headers.insert("originator", "codex_cli_rs".parse().unwrap());
        assert_eq!(detect_platform(&headers, "/v1/responses"), Some(ProxyPlatform::Codex));

        let headers = HeaderMap::new();
        assert_eq!(detect_platform(&headers, "/v1/messages?beta=true"), Some(ProxyPlatform::Claude));
        assert_eq!(detect_platform(&headers, "/v1/chat/completions"), Some(ProxyPlatform::Codex));
        assert_eq!(detect_platform(&headers, "/backend-api/codex/responses"), Some(ProxyPlatform::Codex));
        assert_eq!(detect_platform(&headers, "/v1/unknown"), None);
    }

    #[test]
    fn dedup_key_extraction_covers_sse_and_plain_json() {
        let claude_sse = b"event: ping\ndata: {\"type\":\"ping\"}\n\nevent: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01ABC\",\"model\":\"claude\"}}\n\n";
        assert_eq!(extract_dedup_key(ProxyPlatform::Claude, claude_sse).as_deref(), Some("msg_01ABC"));
        let claude_json = br#"{"id":"msg_01XYZ","type":"message","usage":{}}"#;
        assert_eq!(extract_dedup_key(ProxyPlatform::Claude, claude_json).as_deref(), Some("msg_01XYZ"));
        let codex_sse = b"data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_AA\"}}\n\ndata: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"item_BB\"},\"response\":{\"id\":\"resp_AA\"}}\n\n";
        assert_eq!(extract_dedup_key(ProxyPlatform::Codex, codex_sse).as_deref(), Some("resp_AA"));
        let codex_json = br#"{"id":"resp_CC","object":"response"}"#;
        assert_eq!(extract_dedup_key(ProxyPlatform::Codex, codex_json).as_deref(), Some("resp_CC"));
        assert_eq!(extract_dedup_key(ProxyPlatform::Claude, b"event: done\n"), None);
        assert_eq!(extract_dedup_key(ProxyPlatform::Claude, br#"{"type":"ping"}"#), None);
        assert_eq!(extract_dedup_key(ProxyPlatform::Codex, b""), None);
    }

    #[test]
    fn upstream_url_keeps_path_and_query_and_trims_base_slash() {
        let uri: axum::http::Uri = "/v1/messages?beta=true".parse().unwrap();
        assert_eq!(build_upstream_url("https://gw.test/", &uri), "https://gw.test/v1/messages?beta=true");
        let uri: axum::http::Uri = "/v1/responses".parse().unwrap();
        assert_eq!(build_upstream_url("https://gw.test", &uri), "https://gw.test/v1/responses");
    }

    #[test]
    fn self_base_detection_matches_only_local_proxy_port() {
        assert!(is_self_base("http://127.0.0.1:12731", 12731));
        assert!(is_self_base("http://127.0.0.1:12731/", 12731));
        assert!(!is_self_base("http://127.0.0.1:9999", 12731));
        assert!(!is_self_base("https://gw.test", 12731));
    }

    #[test]
    fn pending_entries_flush_when_scan_lands_and_expire_after_ttl() {
        let mut conn = memory_db();
        crate::db::insert_records(&mut conn, &[record("msg_late")]).unwrap();

        let mut queue = vec![
            PendingTiming {
                connection_id: None,
                upstream_model: None,
                served_model: None,
                dedup_key: "msg_late".into(),
                first_token_ms: 120,
                duration_ms: 900,
                status_code: 200,
                created_ms: now_ms(),
            },
            PendingTiming {
                connection_id: None,
                upstream_model: None,
                served_model: None,
                dedup_key: "msg_expired".into(),
                first_token_ms: 1,
                duration_ms: 2,
                status_code: 200,
                created_ms: now_ms() - PENDING_TTL_MS - 1,
            },
        ];
        flush_entries(&conn, &mut queue).unwrap();
        assert!(queue.is_empty());
        let timing: (Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT first_token_ms, duration_ms FROM requests WHERE dedup_key = 'msg_late'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(timing, (Some(120), Some(900)));
    }

    #[test]
    fn proxied_request_is_attributed_to_matching_api_key_connection() {
        let mut conn = memory_db();
        crate::db::insert_records(&mut conn, &[record("msg_attr")]).unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms) VALUES('claude-abc','claude','api','网关','sk-test-key',1)", []).unwrap();
        let db = Arc::new(Mutex::new(conn));
        let (deps, _rx) = test_deps(db.clone(), HashMap::from([(ProxyPlatform::Claude, "http://127.0.0.1:9".to_string())]), None);
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "sk-test-key".parse().unwrap());
        assert_eq!(deps.connection_for(ProxyPlatform::Claude, &headers).as_deref(), Some("claude-abc"));
        headers.insert("x-api-key", "sk-other".parse().unwrap());
        assert_eq!(deps.connection_for(ProxyPlatform::Claude, &headers), None);
        let mut bearer = HeaderMap::new();
        bearer.insert("authorization", "Bearer sk-test-key".parse().unwrap());
        assert_eq!(deps.connection_for(ProxyPlatform::Codex, &bearer), None);
        assert_eq!(deps.connection_for(ProxyPlatform::Claude, &bearer).as_deref(), Some("claude-abc"));
        let mut queue = vec![PendingTiming {
            dedup_key: "msg_attr".into(),
            first_token_ms: 1,
            duration_ms: 2,
            status_code: 200,
            created_ms: now_ms(),
            connection_id: Some("claude-abc".into()),
            upstream_model: None,
            served_model: None,
        }];
        flush_entries(&db.lock().unwrap(), &mut queue).unwrap();
        let attributed: Option<String> = db.lock().unwrap().query_row("SELECT connection_id FROM requests WHERE dedup_key='msg_attr'", [], |r| r.get(0)).unwrap();
        assert_eq!(attributed.as_deref(), Some("claude-abc"));
        assert_eq!(crate::db::attributed_request_count(&db.lock().unwrap(), "claude-abc").unwrap(), 1);
        assert_eq!(crate::db::attributed_request_count(&db.lock().unwrap(), "claude-none").unwrap(), 0);
    }

    #[test]
    fn pending_entry_kept_until_record_arrives() {
        let conn = memory_db();
        let mut queue = vec![PendingTiming {
            connection_id: None,
            upstream_model: None,
            served_model: None,
            dedup_key: "msg_wait".into(),
            first_token_ms: 50,
            duration_ms: 400,
            status_code: 200,
            created_ms: now_ms(),
        }];
        flush_entries(&conn, &mut queue).unwrap();
        assert_eq!(queue.len(), 1);

        let mut conn = conn;
        crate::db::insert_records(&mut conn, &[record("msg_wait")]).unwrap();
        flush_entries(&conn, &mut queue).unwrap();
        assert!(queue.is_empty());
        let status: Option<i64> = conn
            .query_row("SELECT status_code FROM requests WHERE dedup_key = 'msg_wait'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(status, Some(200));
    }

    #[tokio::test]
    async fn streaming_is_forwarded_chunk_by_chunk_and_timings_are_recorded() {
        let (upstream_addr, mut event_rx) = spawn_chunked_upstream(150, 500).await;

        let db = Arc::new(Mutex::new(memory_db()));
        let (deps, _shutdown_rx) = test_deps(
            db.clone(),
            HashMap::from([(ProxyPlatform::Claude, format!("http://{upstream_addr}"))]),
            None,
        );
        let proxy_addr = spawn_proxy(deps.clone()).await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .post(format!("{proxy_addr}/v1/messages"))
            .header("anthropic-version", "2023-06-01")
            .header("x-api-key", "test-key")
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers().get("x-jolu-proxy").and_then(|v| v.to_str().ok()), Some("1"));
        assert_eq!(response.headers().get("content-type").and_then(|v| v.to_str().ok()), Some("text/event-stream"));

        let mut stream = response.bytes_stream();
        let first = stream.next().await.unwrap().unwrap();
        assert!(first.windows(b"message_start".len()).any(|w| w == b"message_start"));
        assert!(event_rx.try_recv().is_err());
        while stream.next().await.is_some() {}

        let queue = deps.pending.lock().unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].dedup_key, "msg_stream");
        assert_eq!(queue[0].status_code, 200);
        assert!(queue[0].first_token_ms >= 0);
        assert!(queue[0].duration_ms >= queue[0].first_token_ms);

        let mut conn = db.lock().unwrap();
        crate::db::insert_records(&mut conn, &[record("msg_stream")]).unwrap();
        drop(queue);
        flush_entries(&conn, &mut deps.pending.lock().unwrap()).unwrap();
        let timing: (Option<i64>, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT first_token_ms, duration_ms, status_code FROM requests WHERE dedup_key = 'msg_stream'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(timing.2, Some(200));
        assert!(timing.0.unwrap() >= 0 && timing.1.unwrap() >= timing.0.unwrap());
    }

    #[tokio::test]
    async fn upstream_error_status_and_body_pass_through_unchanged() {
        let body = "{\"error\":{\"code\":\"rate_limited\"}}";
        let addr = spawn_static_upstream("429 Too Many Requests", body).await;

        let db = Arc::new(Mutex::new(memory_db()));
        let (deps, _shutdown_rx) = test_deps(
            db,
            HashMap::from([(ProxyPlatform::Codex, format!("http://{addr}"))]),
            None,
        );
        let proxy_addr = spawn_proxy(deps).await;

        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("{proxy_addr}/v1/responses"))
            .header("originator", "codex_cli_rs")
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 429);
        let text = response.text().await.unwrap();
        assert_eq!(text, body);
    }

    #[tokio::test]
    async fn unreachable_upstream_yields_502_and_triggers_fallback_once() {
        let db = Arc::new(Mutex::new(memory_db()));
        let fallback_hits = Arc::new(AtomicU32::new(0));
        let (deps, mut shutdown_rx) = test_deps(
            db,
            HashMap::from([(ProxyPlatform::Claude, "http://127.0.0.1:1".into())]),
            Some(fallback_hits.clone()),
        );
        let proxy_addr = spawn_proxy(deps.clone()).await;

        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        for attempt in 1..=FALLBACK_THRESHOLD {
            let response = client
                .post(format!("{proxy_addr}/v1/messages"))
                .header("anthropic-version", "2023-06-01")
                .body("{}")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY, "第 {attempt} 次应拿到 502");
        }
        assert_eq!(fallback_hits.load(Ordering::Relaxed), 1);
        shutdown_rx.changed().await.unwrap();
        assert!(*shutdown_rx.borrow());
        deps.note_upstream_unreachable("上游不可达：again".into());
        assert_eq!(fallback_hits.load(Ordering::Relaxed), 1);
    }
}
