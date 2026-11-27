#![allow(unused_imports)]
use super::*;

#[cfg(test)]
mod manual_models_tests {
    use super::*;

    #[test]
    fn new_native_api_connections_pass_local_preparation() {
        for (platform,base,service) in [("zcode","https://open.bigmodel.cn/api/anthropic",None),("opencode","https://opencode.ai/zen/go/v1",Some("opencode-go"))] {
            let input=NewConnection{platform:platform.into(),kind:"api".into(),name:"synthetic".into(),secret:Some("synthetic-key".into()),base_url:Some(base.into()),model:Some("synthetic".into()),effort:None,context_1m:None,protocol:None,slug_base:None,service:service.map(str::to_owned)};
            let prepared=prepare_new(input).unwrap_or_else(|e|panic!("{platform}: {e}"));
            let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
            add(&db,prepared).unwrap();let rows=list(&db).unwrap();assert_eq!(rows.len(),1);assert_eq!(rows[0].service.as_deref(),service);assert_eq!(rows[0].model.as_deref(),Some("synthetic"));
            if platform=="zcode"{assert_eq!(rows[0].protocol.as_deref(),Some("anthropic"));}
        }
    }
    #[test]
    /// 上游错误体里的短消息：两种常见形态都取到，其它一律不取
    fn upstream_message_reads_short_reasons_only() {
        // 第三方中转（华龙这类）的形态；403 + 这句话才能告诉用户「Key 被上游停用」
        assert_eq!(
            upstream_message(r#"{"code":"GROUP_DISABLED","message":"API Key 所属分组已停用"}"#).as_deref(),
            Some("API Key 所属分组已停用")
        );
        // OpenAI 官方的嵌套形态
        assert_eq!(
            upstream_message(r#"{"error":{"message":"Incorrect API key provided"}}"#).as_deref(),
            Some("Incorrect API key provided")
        );
        // 空串、无 message、非 JSON、超长：一律不取，用通用文案
        assert!(upstream_message(r#"{"message":"  "}"#).is_none());
        assert!(upstream_message(r#"{"code":"X"}"#).is_none());
        assert!(upstream_message("<html>502 Bad Gateway</html>").is_none());
        assert!(upstream_message(&format!(r#"{{"message":"{}"}}"#, "长".repeat(121))).is_none());
    }

    #[test]
    fn models_endpoint_normalizes_base_and_rejects_non_http() {
        assert_eq!(models_endpoint("https://gw.test", "openai", None).unwrap(), "https://gw.test/v1/models");
        assert_eq!(models_endpoint("https://gw.test/", "openai", None).unwrap(), "https://gw.test/v1/models");
        assert_eq!(models_endpoint("https://gw.test/v1", "openai", None).unwrap(), "https://gw.test/v1/models");
        assert_eq!(models_endpoint("https://gw.test/v1/", "openai", Some("claude-sonnet-4-5")).unwrap(), "https://gw.test/v1/models/claude-sonnet-4-5");
        assert_eq!(
            models_endpoint("https://generativelanguage.googleapis.com/v1beta/openai/", "openai", None).unwrap(),
            "https://generativelanguage.googleapis.com/v1beta/openai/models"
        );
        assert!(models_endpoint("ftp://gw.test", "openai", None).is_err());
        assert!(models_endpoint("not a url", "openai", None).is_err());
    }
}

#[cfg(test)]
mod gateway_tests {
    use super::*;

    #[test]
    fn official_base_urls_are_not_misrouted_to_gateway_usage() {
        for (platform, host) in [("claude", "api.anthropic.com"), ("codex", "api.openai.com")] {
            assert_eq!(normalized_base(platform, "api", Some(format!("https://{host}/v1"))), None);
            let custom = format!("https://{host}.example.test/v1");
            assert_eq!(normalized_base(platform, "api", Some(custom.clone())), Some(custom));
            assert_eq!(normalized_base(platform, "auth", Some("https://gateway.test".into())), None);
        }
    }

    #[test]
    fn local_registration_is_deduplicated_and_does_not_enable_connections() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let input = |key: &str| NewConnection { service: None, platform: "claude".into(), kind: "api".into(), name: "local".into(), secret: Some(key.into()), base_url: Some("https://example.test".into()), model: None, effort: None, context_1m: None, protocol: None , slug_base: None};
        let (id, added) = register_local(&db, input("sk-first-1234")).unwrap();
        assert!(added);
        assert!(selectable_creds(&db, &id).is_err());
        assert_eq!(register_local(&db, input("sk-first-1234")).unwrap(), (id.clone(), false));
        let (other, added) = register_local(&db, input("sk-other-1234")).unwrap();
        assert!(added); assert_ne!(other, id);
        assert_eq!(list(&db).unwrap().len(), 2);
        set_paused(&db, &id, false).unwrap();
        register_local(&db, input("sk-first-1234")).unwrap();
        assert!(selectable_creds(&db, &id).is_ok());
        set_paused(&db, &id, true).unwrap();
        register_local(&db, input("sk-first-1234")).unwrap();
        assert!(creds_of(&db, &id).unwrap().unwrap().paused);
        // 「当前使用」按平台 + 类型 + 凭证原值比对
        assert_eq!(find_by_secret(&db, "claude", "api", "sk-other-1234").unwrap(), Some(other));
        assert_eq!(find_by_secret(&db, "claude", "auth", "sk-other-1234").unwrap(), None);
        assert_eq!(find_by_secret(&db, "codex", "api", "sk-first-1234").unwrap(), None);
    }

    #[test]
    fn paused_connection_cannot_be_reactivated_by_late_query_results() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let id = add(&db, NewConnection { service: None,
            platform: "claude".into(), kind: "api".into(), name: "pause test".into(),
            secret: Some("synthetic-pause-key".into()), base_url: None,
            model: None, effort: None, context_1m: None, protocol: None, slug_base: None,
        }).unwrap();
        db.execute("UPDATE connections SET status = 'paused' WHERE id = ?1", [&id]).unwrap();
        touch_sync(&db, &id).unwrap();
        assert_eq!(list(&db).unwrap()[0].status, "paused");
        assert!(!record_query_status(&db, &id, true).unwrap());
        assert_eq!(list(&db).unwrap()[0].status, "paused");
        set_paused(&db, &id, false).unwrap();
        assert!(record_query_status(&db, &id, false).unwrap());
        assert!(!record_query_status(&db, &id, false).unwrap());
        assert!(record_query_status(&db, &id, true).unwrap());
        set_paused(&db, &id, true).unwrap();
        mark_expired(&db, &id).unwrap();
        assert_eq!(list(&db).unwrap()[0].status, "paused");
    }

    #[test]
    fn pause_preserves_saved_credentials_and_blocks_queries_until_resumed() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let id = add(&db, NewConnection { service: None,
            platform: "codex".into(), kind: "api".into(), name: "saved connection".into(),
            secret: Some("synthetic-preserved-key".into()), base_url: Some("https://example.test".into()),
            model: None, effort: None, context_1m: None, protocol: None, slug_base: None,
        }).unwrap();
        set_paused(&db, &id, true).unwrap();
        assert!(query_creds(&db, &id).is_err());
        let saved = creds_of(&db, &id).unwrap().unwrap();
        assert!(saved.paused);
        assert_eq!(saved.secret.as_deref(), Some("synthetic-preserved-key"));
        assert_eq!(saved.base_url.as_deref(), Some("https://example.test"));
        assert!(!fetch_local(&db, &id).unwrap().ok);
        assert_eq!(list(&db).unwrap().len(), 1);
        set_paused(&db, &id, false).unwrap();
        assert!(query_creds(&db, &id).unwrap().is_some());
        assert_eq!(list(&db).unwrap()[0].status, "connected");
        assert!(set_paused(&db, "missing", true).is_err());
    }

    #[test]
    fn official_native_keys_require_explicit_credentials_and_supported_platform() {
        let input=|platform: &str, kind: &str, secret: Option<&str>| NewConnection { service: None,
            platform:platform.into(), kind:kind.into(), name:"official test".into(),
            secret:secret.map(str::to_string),base_url:None,model:None,effort:None,context_1m:None,protocol:None, slug_base: None,
        };
        for platform in ["gemini","grok"] {
            assert!(prepare_new(input(platform,"api",Some("synthetic-key"))).is_ok());
            assert!(prepare_new(input(platform,"api",None)).is_err());
            assert!(prepare_new(input(platform,"auth",Some("synthetic-key"))).is_err());
        }
        assert!(prepare_new(input("trae","api",Some("synthetic-key"))).is_err());
    }

    #[test]
    fn gateway_accepts_http_and_https_domains_but_rejects_other_schemes() {
        let connection = |base: &str| NewConnection { service: None,
            platform: "claude".into(), kind: "api".into(), name: "HTTP gateway test".into(),
            secret: Some("synthetic-test-key".into()), base_url: Some(base.into()),
            model: None, effort: None, context_1m: None, protocol: None, slug_base: None,
        };
        for base in ["http://api.example.com", "http://192.168.1.2:8080", "http://localhost:8000", "https://api.example.com", "HTTP://api.example.com"] {
            assert!(prepare_new(connection(base)).is_ok(), "should accept {base}");
        }
        for base in ["ftp://api.example.com", "file:///tmp/key", "not-a-url", "http://"] {
            assert!(prepare_new(connection(base)).is_err(), "should reject {base}");
        }
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;

    fn db() -> rusqlite::Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn claude_api(name: &str, secret: &str) -> NewConnection {
        NewConnection { service: None,
            platform: "claude".into(),
            kind: "api".into(),
            name: name.into(),
            secret: Some(secret.into()),
            base_url: None,
            model: None,
            effort: None,
            context_1m: None,
            protocol: None,
            slug_base: None,
        }
    }

    /// 从本机导入的连接：显示名带来源标签，slug 基名单独给（不带标签）
    #[test]
    fn slug_base_keeps_source_label_out_of_the_prefix() {
        let db = db();
        let id = add(&db, NewConnection {
            service: None,
            platform: "custom".into(),
            kind: "api".into(),
            name: "Claude Code · Claude Code 上游".into(),
            secret: Some("sk-import-1234".into()),
            base_url: Some("https://relay.test".into()),
            model: None,
            effort: None,
            context_1m: None,
            protocol: Some("openai".into()),
            slug_base: Some("Claude Code 上游".into()),
        })
        .unwrap();
        let row = &list(&db).unwrap()[0];
        // 显示名保留来源标签
        assert_eq!(row.name, "Claude Code · Claude Code 上游");
        // 前缀不再重复拼 claude-code
        assert_eq!(row.slug.as_deref(), Some("claude-code"), "{id}");
    }

    #[test]
    fn rename_updates_name_only_and_keeps_credentials() {
        let db = db();
        let id = add(&db, claude_api("旧名称", "sk-rename-1234")).unwrap();
        rename_connection(&db, &id, "  新名称  ").unwrap();
        let row = &list(&db).unwrap()[0];
        assert_eq!(row.name, "新名称");
        // 改名只动名称：凭证脱敏标识与状态保持不变
        assert_eq!(row.masked, mask("sk-rename-1234"));
        assert_eq!(row.status, "connected");
        // 改名是纯元数据操作，断开的连接同样允许改名
        set_paused(&db, &id, true).unwrap();
        rename_connection(&db, &id, "断开也能改名").unwrap();
        assert_eq!(list(&db).unwrap()[0].name, "断开也能改名");
    }

    #[test]
    fn rename_rejects_blank_overlong_and_missing_connections() {
        let db = db();
        let id = add(&db, claude_api("原名", "sk-rename-5678")).unwrap();
        assert!(rename_connection(&db, &id, "   ").is_err());
        assert!(rename_connection(&db, &id, &"长".repeat(81)).is_err());
        assert!(rename_connection(&db, "missing", "任意").is_err());
        // 校验失败后原名保持不变
        assert_eq!(list(&db).unwrap()[0].name, "原名");
    }

    #[test]
    fn custom_name_overrides_local_inputs_only_when_valid() {
        let input = |name: &str| NewConnection { service: None,
            platform: "claude".into(),
            kind: "api".into(),
            name: name.into(),
            secret: Some("sk-custom-1234".into()),
            base_url: None,
            model: None,
            effort: None,
            context_1m: None,
            protocol: None, slug_base: None,
        };
        // 用户填了名称：覆盖自动生成名，并去首尾空白
        let renamed = with_custom_name(vec![input("Claude API Key")], Some("  我的连接  ".into())).unwrap();
        assert_eq!(renamed[0].name, "我的连接");
        // 只填空白：视为未填，保留自动生成名
        let kept = with_custom_name(vec![input("Claude API Key")], Some("   ".into())).unwrap();
        assert_eq!(kept[0].name, "Claude API Key");
        // 未填：原样返回
        let none = with_custom_name(vec![input("Claude API Key")], None).unwrap();
        assert_eq!(none[0].name, "Claude API Key");
        // 超长名称：整体拒绝，不让半合法输入落到库里
        assert!(with_custom_name(vec![input("Claude API Key")], Some("长".repeat(81))).is_err());
    }
    #[test]
    fn local_subscription_names_ignore_frontend_custom_names() {
        let input = NewConnection { service: None, platform: "claude".into(), kind: "auth".into(),
            name: "Claude 官方订阅".into(), secret: None, base_url: None, model: None,
            effort: None, context_1m: None, protocol: None , slug_base: None};
        let output = with_custom_name(vec![input], Some("前端任意名称".into())).unwrap();
        assert_eq!(output[0].name, "Claude 官方订阅");
    }
}

#[cfg(test)]
mod query_result_tests {
    use super::*;

    fn connection() -> (Sql, String, Creds) {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let id = add(&db, NewConnection {
            service: None, platform: "claude".into(), kind: "api".into(), name: "query result".into(),
            secret: Some("synthetic-old-key".into()), base_url: Some("https://gateway.example.test".into()),
            model: None, effort: None, context_1m: None, protocol: None, slug_base: None,
        }).unwrap();
        let expected = query_creds(&db, &id).unwrap().unwrap();
        (db, id, expected)
    }

    #[test]
    fn late_unauthorized_from_old_key_cannot_expire_replacement() {
        let (db, id, expected) = connection();
        replace_api_key(&db, &id, "synthetic-new-key").unwrap();
        let result = record_query_result(&db, &id, &expected, Some(false));
        assert_eq!(list(&db).unwrap()[0].status, "connected");
        assert!(result.is_err());
    }

    #[test]
    fn deleted_or_paused_connection_rejects_every_late_result() {
        for outcome in [None, Some(true), Some(false)] {
            let (db, id, expected) = connection();
            set_paused(&db, &id, true).unwrap();
            assert!(record_query_result(&db, &id, &expected, outcome).is_err());
            assert_eq!(list(&db).unwrap()[0].status, "paused");
            remove(&db, &id).unwrap();
            assert!(record_query_result(&db, &id, &expected, outcome).is_err());
        }
    }

    #[test]
    fn query_target_changes_invalidate_even_non_status_results() {
        for statement in [
            "UPDATE connections SET secret='synthetic-new-key' WHERE id=?1",
            "UPDATE connections SET base_url='https://other.example.test' WHERE id=?1",
            "UPDATE connections SET platform='codex' WHERE id=?1",
            "UPDATE connections SET kind='auth' WHERE id=?1",
            "UPDATE connections SET model='different-model' WHERE id=?1",
        ] {
            let (db, id, expected) = connection();
            db.execute(statement, [&id]).unwrap();
            assert!(record_query_result(&db, &id, &expected, None).is_err());
            assert_eq!(list(&db).unwrap()[0].status, "connected");
        }
    }

    #[test]
    fn unchanged_query_updates_status_and_rename_does_not_invalidate() {
        let (db, id, expected) = connection();
        rename_connection(&db, &id, "renamed").unwrap();
        assert!(!record_query_result(&db, &id, &expected, None).unwrap());
        assert!(record_query_result(&db, &id, &expected, Some(false)).unwrap());
        assert_eq!(list(&db).unwrap()[0].status, "expired");
        assert!(!record_query_result(&db, &id, &expected, None).unwrap());
        assert_eq!(list(&db).unwrap()[0].status, "expired");
        assert!(record_query_result(&db, &id, &expected, Some(true)).unwrap());
        assert!(!record_query_result(&db, &id, &expected, Some(true)).unwrap());
        let row = &list(&db).unwrap()[0];
        assert_eq!(row.name, "renamed");
        assert_eq!(row.status, "connected");
        let synced: Option<i64> = db.query_row("SELECT last_sync_ms FROM connections WHERE id=?1", [&id], |row| row.get(0)).unwrap();
        assert!(synced.is_some());
    }
}

#[cfg(test)]
mod removal_reference_tests {
    use super::*;

    fn connection(db: &Sql, name: &str) -> String {
        add(db, NewConnection {
            service: None, platform: "claude".into(), kind: "api".into(), name: name.into(),
            secret: Some("synthetic-removal-key".into()), base_url: None,
            model: None, effort: None, context_1m: None, protocol: None, slug_base: None,
        }).unwrap()
    }

    fn waiting_task(db: &Sql, connection_id: &str) -> (i64, crate::automation::TaskInput) {
        crate::automation::ensure_schema(db).unwrap();
        let input = crate::automation::TaskInput {
            name: "snapshot reference".into(), platform: "claude".into(), connection_id: connection_id.into(),
            model: None, cwd: std::env::temp_dir().to_string_lossy().into_owned(), session_id: None,
            prompt: "synthetic task".into(), skills: vec![], launch: "quota".into(), schedule: None,
            auto_resume: true, wait_deadline: None, serial_queue: true,
        };
        let task = crate::automation::create_task(db, &input, 1_000).unwrap();
        (task, input)
    }

    #[test]
    fn edited_task_does_not_release_its_active_run_snapshot_connection() {
        for status in ["queued", "waiting", "running", "attention"] {
            let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
            let original = connection(&db, "A");
            let replacement = connection(&db, "B");
            let (task, mut input) = waiting_task(&db, &original);
            input.connection_id = replacement.clone();
            crate::automation::update_task(&db, task, &input, 2_000).unwrap();
            db.execute("UPDATE automation_runs SET status=?1 WHERE task_id=?2", params![status, task]).unwrap();
            let result = remove(&db, &original);
            assert!(result.is_err(), "active snapshot must protect its connection: {status}");
            assert!(creds_of(&db, &original).unwrap().is_some());
            let snapshot_connection: String = db.query_row("SELECT json_extract(task_snapshot, '$.connection_id') FROM automation_runs WHERE task_id=?1", [task], |row| row.get(0)).unwrap();
            assert_eq!(snapshot_connection, original);
        }
    }

    #[test]
    fn active_and_paused_tasks_keep_their_current_connection() {
        for state in ["active", "paused"] {
            let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
            let id = connection(&db, "task account");
            let (task, _) = waiting_task(&db, &id);
            db.execute("UPDATE automation_runs SET status='completed' WHERE task_id=?1", [task]).unwrap();
            db.execute("UPDATE automation_tasks SET state=?1 WHERE id=?2", params![state, task]).unwrap();
            assert!(remove(&db, &id).is_err(), "resumable task must protect its connection: {state}");
            assert!(creds_of(&db, &id).unwrap().is_some());
        }
    }

    #[test]
    fn terminal_history_does_not_block_removal_and_is_preserved() {
        for status in ["completed", "failed", "cancelled"] {
            let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
            let original = connection(&db, "A");
            let replacement = connection(&db, "B");
            let (task, mut input) = waiting_task(&db, &original);
            input.connection_id = replacement;
            crate::automation::update_task(&db, task, &input, 2_000).unwrap();
            db.execute("UPDATE automation_runs SET status=?1 WHERE task_id=?2", params![status, task]).unwrap();
            remove(&db, &original).unwrap();
            assert!(creds_of(&db, &original).unwrap().is_none());
            let count: i64 = db.query_row("SELECT COUNT(*) FROM automation_runs WHERE task_id=?1", [task], |row| row.get(0)).unwrap();
            assert_eq!(count, 1);
        }
    }

    #[test]
    fn unreferenced_connection_can_be_removed_without_automation_tables() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let id = connection(&db, "unreferenced");
        remove(&db, &id).unwrap();
        assert!(creds_of(&db, &id).unwrap().is_none());
        crate::automation::ensure_schema(&db).unwrap();
        let id = connection(&db, "empty automation");
        remove(&db, &id).unwrap();
        assert!(creds_of(&db, &id).unwrap().is_none());
    }
}

#[cfg(test)]
mod local_sync_tests {
    use super::*;

    fn db() -> rusqlite::Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn claude_api(name: &str, secret: &str, base: Option<&str>) -> NewConnection {
        NewConnection { service: None,
            platform: "claude".into(),
            kind: "api".into(),
            name: name.into(),
            secret: Some(secret.into()),
            base_url: base.map(str::to_string),
            model: None,
            effort: None,
            context_1m: None,
            protocol: None, slug_base: None,
        }
    }

    #[test]
    fn update_refreshes_secret_and_base_url_in_place() {
        let db = db();
        let id = add(&db, claude_api("旧中转", "sk-old-1234", Some("https://old.example.test"))).unwrap();
        let result = apply_local(
            &db,
            &id,
            Some(LocalPair { secret: "sk-new-4567".into(), base_url: Some("https://new.example.test".into()) }),
            vec![],
        )
        .unwrap();
        assert!(result.ok && result.changed, "{result:?}");
        assert_eq!(result.masked.as_deref(), Some("sk-****4567"));
        let row = &list(&db).unwrap()[0];
        assert_eq!(row.masked, "sk-****4567");
        assert_eq!(row.base_url.as_deref(), Some("https://new.example.test"));
        assert_eq!(row.status, "connected");
    }

    #[test]
    fn update_reports_no_change_when_local_matches_after_normalization() {
        let db = db();
        let id = add(&db, claude_api("同一中转", "sk-same-1234", Some("https://same.example.test"))).unwrap();
        let result = apply_local(
            &db,
            &id,
            Some(LocalPair { secret: "sk-same-1234".into(), base_url: Some("https://same.example.test/".into()) }),
            vec![],
        )
        .unwrap();
        assert!(result.ok && !result.changed, "{result:?}");
    }

    #[test]
    fn update_without_local_secret_keeps_connection_untouched() {
        let db = db();
        let id = add(&db, claude_api("不变化", "sk-keep-1234", Some("https://keep.example.test"))).unwrap();
        let result = apply_local(&db, &id, None, vec![]).unwrap();
        assert!(!result.ok && !result.changed, "{result:?}");
        let row = &list(&db).unwrap()[0];
        assert_eq!(row.masked, mask("sk-keep-1234"));
        assert_eq!(row.base_url.as_deref(), Some("https://keep.example.test"));
    }

    #[test]
    fn update_refuses_when_another_same_kind_connection_holds_a_different_key() {
        let db = db();
        add(&db, claude_api("A", "sk-aaa-1111", None)).unwrap();
        let b = add(&db, claude_api("B", "sk-bbb-2222", None)).unwrap();
        let result = apply_local(&db, &b, Some(LocalPair { secret: "sk-ccc-3333".into(), base_url: None }), vec![]).unwrap();
        assert!(!result.ok && !result.changed, "{result:?}");
        let row = list(&db).unwrap().into_iter().find(|c| c.id == b).unwrap();
        assert_eq!(row.masked, mask("sk-bbb-2222"));
    }

    #[test]
    fn sync_updates_the_unambiguous_connection_and_reports_changed() {
        let db = db();
        add(&db, claude_api("Claude API Key", "sk-old-9999", None)).unwrap();
        let changed = sync_from_local_with(&db, &|platform, kind| {
            matches!((platform, kind), ("claude", "api"))
                .then(|| LocalPair { secret: "sk-new-8888".into(), base_url: None })
        })
        .unwrap();
        assert!(changed);
        assert_eq!(list(&db).unwrap()[0].masked, mask("sk-new-8888"));
    }

    #[test]
    fn sync_skips_when_local_missing_ambiguous_or_paused() {
        let db = db();
        let local = |secret: &str| Some(LocalPair { secret: secret.into(), base_url: None });
        let a = add(&db, claude_api("A", "sk-aaa-1111", None)).unwrap();
        // 本机没读到：不动
        assert!(!sync_from_local_with(&db, &|_, _| None).unwrap());
        // 同类型两条：分不清本机配置属于哪条，不动
        let b = add(&db, claude_api("B", "sk-bbb-2222", None)).unwrap();
        assert!(!sync_from_local_with(&db, &|_, _| local("sk-new-3333")).unwrap());
        // 暂停的连接不参与，也不会因此误更新另一条
        set_paused(&db, &b, true).unwrap();
        assert!(!sync_from_local_with(&db, &|_, _| local("sk-new-4444")).unwrap());
        assert_eq!(creds_of(&db, &a).unwrap().unwrap().secret.as_deref(), Some("sk-aaa-1111"));
        assert_eq!(creds_of(&db, &b).unwrap().unwrap().secret.as_deref(), Some("sk-bbb-2222"));
    }
}

// 任务 3.4 开始：自定义供应商与协议鉴权
#[cfg(test)]
mod custom_provider_tests {
    use super::*;

    fn custom(protocol: Option<&str>, base: Option<&str>, secret: Option<&str>) -> NewConnection {
        NewConnection { service: None,
            platform: "custom".into(),
            kind: "api".into(),
            name: "公司网关".into(),
            secret: secret.map(Into::into),
            base_url: base.map(Into::into),
            model: Some("gpt-4.1-mini".into()),
            effort: None,
            context_1m: None,
            protocol: protocol.map(Into::into), slug_base: None,
        }
    }

    #[test]
    fn ollama_cloud_api_key_is_allowed_without_faking_native_auth() {
        let prepared = prepare_new(NewConnection { service: None,
            platform: "ollama".into(),
            kind: "api".into(),
            name: "Ollama Cloud".into(),
            secret: Some("synthetic-ollama-key".into()),
            base_url: Some("https://ollama.com/v1".into()),
            model: Some("gpt-oss:120b".into()),
            effort: None,
            context_1m: None,
            protocol: Some("openai".into()), slug_base: None,
        }).unwrap();
        assert_eq!(prepared.platform, "ollama");
        assert_eq!(prepared.base_url.as_deref(), Some("https://ollama.com/v1"));
    }

    fn headers(protocol: &str) -> reqwest::header::HeaderMap {
        let client = reqwest::blocking::Client::new();
        apply_auth(client.get("https://gw.test/v1/models"), protocol, "synthetic-key").build().unwrap().headers().clone()
    }

    #[test]
    fn anthropic_protocol_headers() {
        let h = headers("anthropic");
        assert_eq!(h.get("x-api-key").unwrap(), "synthetic-key");
        assert!(h.contains_key("anthropic-version"));
        assert!(!h.contains_key("authorization"));
        let h = headers("openai");
        assert_eq!(h.get("authorization").unwrap(), "Bearer synthetic-key");
        assert!(!h.contains_key("x-api-key"));
    }

    /// 本机一次性 HTTP 桩：按给定状态与响应体回复，并把收到的请求原文（含请求体）交回给测试检查。
    fn serve_once(status: &'static str, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        serve_once_with(status, body, String::new())
    }

    /// 返回 3xx 与 Location 的桩，用于确认客户端不跟随重定向
    fn serve_once_with_location(status: &'static str, location: String) -> (String, std::thread::JoinHandle<String>) {
        serve_once_with(status, "{}", format!("Location: {location}\r\n"))
    }

    fn serve_once_with(status: &'static str, body: &'static str, extra_headers: String) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            let header_end = loop {
                if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") { break at + 4; }
                let n = stream.read(&mut buf).unwrap();
                if n == 0 { break request.len(); }
                request.extend_from_slice(&buf[..n]);
            };
            let head = String::from_utf8_lossy(&request[..header_end]).to_lowercase();
            let length = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
            while request.len() < header_end + length {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 { break; }
                request.extend_from_slice(&buf[..n]);
            }
            let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            stream.write_all(reply.as_bytes()).unwrap();
            stream.write_all(body.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).to_lowercase()
        });
        (base, handle)
    }

    fn serve_models_once() -> (String, std::thread::JoinHandle<String>) {
        serve_once("200 OK", r#"{"data":[{"id":"model-a"},{"id":"model-b"},{"id":"model-a"}]}"#)
    }

    #[test]
    fn verify_model_sends_one_minimal_request_per_protocol() {
        let (base, server) = serve_once("200 OK", r#"{"id":"msg_1"}"#);
        assert_eq!(verify_model("anthropic", &base, "synthetic-key", " claude-x "), Ok(()));
        let request = server.join().unwrap();
        assert!(request.starts_with("post /v1/messages "), "{request}");
        assert!(request.contains("x-api-key: synthetic-key") && !request.contains("authorization:"));
        assert!(request.contains(r#""model":"claude-x""#) && request.contains(r#""max_tokens":1"#));

        let (base, server) = serve_once("200 OK", r#"{"id":"chatcmpl-1"}"#);
        assert_eq!(verify_model("openai", &format!("{base}/v1"), "synthetic-key", "gpt-x"), Ok(()));
        let request = server.join().unwrap();
        assert!(request.starts_with("post /v1/chat/completions "), "{request}");
        assert!(request.contains("authorization: bearer synthetic-key") && request.contains(r#""model":"gpt-x""#));
    }

    #[test]
    fn verify_model_explains_rejections_without_echoing_the_key() {
        let (base, server) = serve_once("401 Unauthorized", r#"{"error":{"message":"invalid api key"}}"#);
        let err = verify_model("openai", &base, "synthetic-key", "gpt-x").unwrap_err();
        server.join().unwrap();
        assert!(err.contains("401") && err.contains("API Key") && !err.contains("synthetic-key"), "{err}");

        let (base, server) = serve_once("404 Not Found", r#"{"error":{"message":"model not found"}}"#);
        let err = verify_model("openai", &base, "synthetic-key", "gpt-x").unwrap_err();
        server.join().unwrap();
        assert!(err.contains("404") && err.contains("model not found"), "{err}");

        assert!(verify_model("openai", "https://gw.test", "synthetic-key", "  ").is_err());
    }

    #[test]
    fn versioned_openai_bases_are_not_given_another_v1() {
        let models = |base: &str, protocol: &str| models_endpoint(base, protocol, None).unwrap();
        // OpenAI 兼容：以 /v<数字> 结尾视为已带版本（智谱 /paas/v4、火山 /api/v3 等）
        assert_eq!(models("https://open.example.cn/api/paas/v4", "openai"), "https://open.example.cn/api/paas/v4/models");
        assert_eq!(models("https://open.example.cn/api/paas/v4/", "openai"), "https://open.example.cn/api/paas/v4/models");
        assert_eq!(models("https://generativelanguage.googleapis.com/v1beta/openai", "openai"), "https://generativelanguage.googleapis.com/v1beta/openai/models");
        assert_eq!(models("https://gw.test", "openai"), "https://gw.test/v1/models");
        assert_eq!(models("https://gw.test/api/v3beta", "openai"), "https://gw.test/api/v3beta/v1/models");
        assert_eq!(api_endpoint("https://ark.example.com/api/v3", "openai", "/chat/completions").unwrap(), "https://ark.example.com/api/v3/chat/completions");
        // Anthropic 兼容保持原规则：只有 /v1（或 /openai）结尾不再补 /v1
        assert_eq!(models("https://open.example.cn/api/anthropic", "anthropic"), "https://open.example.cn/api/anthropic/v1/models");
        assert_eq!(models("https://gw.test/v1", "anthropic"), "https://gw.test/v1/models");
        assert_eq!(models("https://gw.test/api/v4", "anthropic"), "https://gw.test/api/v4/v1/models");
    }

    #[test]
    fn official_prefilled_bases_keep_their_models_endpoint() {
        // providerCatalog.ts 的官方预填地址；协议与 fetch_models 一样由 auth_protocol 决定
        let models = |platform: &str, base: &str| models_endpoint(base, auth_protocol(platform, None), None).unwrap();
        assert_eq!(models("codex", "https://api.openai.com/v1"), "https://api.openai.com/v1/models");
        assert_eq!(models("grok", "https://api.x.ai/v1"), "https://api.x.ai/v1/models");
        assert_eq!(models("claude", "https://api.anthropic.com"), "https://api.anthropic.com/v1/models");
    }

    #[test]
    fn redirects_are_refused_without_following_or_leaking_the_key() {
        // 跳转目标：若客户端跟随重定向，这里会收到带 Key 的第二次请求
        let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let location = format!("http://{}/v1/models", target.local_addr().unwrap());
        let (base, server) = serve_once_with_location("302 Found", location.clone());
        let err = fetch_models("custom", Some("anthropic"), &base, "synthetic-key").unwrap_err();
        server.join().unwrap();
        assert!(err.contains("重定向") && err.contains("302") && err.contains("最终地址"), "{err}");
        assert!(!err.contains(&location) && !err.contains("synthetic-key"), "{err}");

        let (base, server) = serve_once_with_location("307 Temporary Redirect", location.clone());
        let err = verify_model("openai", &base, "synthetic-key", "gpt-x").unwrap_err();
        server.join().unwrap();
        assert!(err.contains("重定向") && err.contains("307") && !err.contains(&location), "{err}");
        assert!(matches!(target.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock), "不应向跳转目标发送请求");
    }

    #[test]
    fn service_error_message_is_scrubbed_of_the_key() {
        let (base, server) = serve_once("400 Bad Request", r#"{"error":{"message":"bad key synthetic-key for model"}}"#);
        let err = verify_model("openai", &base, "synthetic-key", "gpt-x").unwrap_err();
        server.join().unwrap();
        assert!(err.contains("bad key **** for model") && !err.contains("synthetic-key"), "{err}");
        // 连接失败的报错不带 URL（URL 里可能有 userinfo / query 凭证）；端口 9 在本机无人监听
        let err = fetch_models("custom", Some("openai"), "http://user:pass@127.0.0.1:9/v1?key=secret-q", "synthetic-key").unwrap_err();
        assert!(!err.contains("pass") && !err.contains("secret-q"), "{err}");
        let err = verify_model("openai", "http://user:pass@127.0.0.1:9/v1?key=secret-q", "synthetic-key", "gpt-x").unwrap_err();
        assert!(!err.contains("pass") && !err.contains("secret-q"), "{err}");
    }

    #[test]
    fn custom_key_change_requires_a_new_model() {
        assert!(check_custom_key_change("custom", None).is_err());
        assert!(check_custom_key_change("custom", Some("  ")).is_err());
        assert_eq!(check_custom_key_change("custom", Some("gpt-x")), Ok(()));
        assert_eq!(check_custom_key_change("codex", None), Ok(()));
    }

    #[test]
    fn fetch_models_sends_selected_protocol_headers_to_custom_service() {
        let (base, server) = serve_models_once();
        assert_eq!(fetch_models("custom", Some("anthropic"), &base, "synthetic-key").unwrap(), vec!["model-a", "model-b"]);
        let request = server.join().unwrap();
        assert!(request.starts_with("get /v1/models "), "{request}");
        assert!(request.contains("x-api-key: synthetic-key") && request.contains("anthropic-version:"));
        assert!(!request.contains("authorization:"));

        let (base, server) = serve_models_once();
        assert_eq!(fetch_models("custom", Some("openai"), &format!("{base}/v1/"), "synthetic-key").unwrap().len(), 2);
        let request = server.join().unwrap();
        assert!(request.starts_with("get /v1/models "), "{request}");
        assert!(request.contains("authorization: bearer synthetic-key") && !request.contains("x-api-key"));
    }

    #[test]
    fn auth_protocol_follows_platform_then_saved_protocol() {
        assert_eq!(auth_protocol("claude", None), "anthropic");
        assert_eq!(auth_protocol("codex", None), "openai");
        assert_eq!(auth_protocol("custom", Some("anthropic")), "anthropic");
        assert_eq!(auth_protocol("custom", Some("openai")), "openai");
        assert_eq!(auth_protocol("custom", None), "openai");
    }

    #[test]
    fn custom_connection_requires_protocol_base_and_explicit_key() {
        assert!(prepare_new(custom(Some("openai"), Some("https://gw.test/v1/"), Some(" sk-custom-1234 "))).is_ok());
        assert!(prepare_new(custom(None, Some("https://gw.test/v1"), Some("sk-custom-1234"))).is_err());
        assert!(prepare_new(custom(Some("gemini"), Some("https://gw.test/v1"), Some("sk-custom-1234"))).is_err());
        assert!(prepare_new(custom(Some("openai"), None, Some("sk-custom-1234"))).is_err());
        assert!(prepare_new(custom(Some("openai"), Some("https://gw.test/v1"), None)).is_err());
        assert!(prepare_new(NewConnection { kind: "auth".into(), ..custom(Some("openai"), Some("https://gw.test"), Some("sk-custom-1234")) }).is_err());
    }

    #[test]
    fn custom_connection_saves_protocol_and_model() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let input = prepare_new(custom(Some("anthropic"), Some("https://gw.test/"), Some("sk-custom-1234"))).unwrap();
        let id = add(&db, input).unwrap();
        let dto = list(&db).unwrap().into_iter().find(|c| c.id == id).unwrap();
        assert_eq!((dto.platform.as_str(), dto.protocol.as_deref(), dto.base_url.as_deref()), ("custom", Some("anthropic"), Some("https://gw.test")));
        assert_eq!(dto.model.as_deref(), Some("gpt-4.1-mini"));
        // 恢复查询 / 更换 Key 的校验靠保存的模型（D-11）
        assert_eq!(creds_of(&db, &id).unwrap().unwrap().model.as_deref(), Some("gpt-4.1-mini"));
    }
}
// 任务 3.4 结束

#[cfg(test)]
mod account_email_tests {
    use super::*;
    #[test]
    fn readonly_list_resolves_only_unknown_matching_auth_identities() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir()
            .join(format!("jolu-readonly-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.db");
        let writer = crate::db::open(&path).unwrap();
        writer.execute_batch(
            "INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms,account_email) VALUES
             ('matched','codex','auth','Matched','synthetic-match','****',1,NULL),
             ('saved','claude','auth','Saved','synthetic-saved','****',2,'saved@example.test'),
             ('different','claude','auth','Different','synthetic-different','****',3,NULL),
             ('api','codex','api','API','synthetic-api','****',4,NULL),
             ('missing','codex','auth','Missing',NULL,'****',5,NULL);"
        ).unwrap();
        let reader = crate::db::open_reader(&path).unwrap();
        let mut looked_up = Vec::new();
        let rows = list_with_account_lookup(&reader, |platform, secret| {
            looked_up.push((platform.to_string(), secret.to_string()));
            (platform == "codex" && secret == "synthetic-match")
                .then(|| "matched@example.test".to_string())
        }).unwrap();
        assert_eq!(rows[0].account_email.as_deref(), Some("matched@example.test"));
        assert_eq!(rows[1].account_email.as_deref(), Some("saved@example.test"));
        assert!(rows[2..].iter().all(|row| row.account_email.is_none()));
        assert_eq!(looked_up, vec![
            ("codex".to_string(), "synthetic-match".to_string()),
            ("claude".to_string(), "synthetic-different".to_string()),
        ]);
        let stored: Option<String> = writer.query_row(
            "SELECT account_email FROM connections WHERE id='matched'", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(stored, None, "列表查询不得回写邮箱");
        assert!(reader.execute("UPDATE connections SET name='unexpected'", []).is_err());
        let json = serde_json::to_string(&rows).unwrap();
        assert!(!json.contains("synthetic-"));
        assert!(serde_json::to_value(&rows).unwrap().as_array().unwrap()
            .iter().all(|row| row.get("secret").is_none()));
        drop(reader);
        drop(writer);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn account_email_survives_rename_pause_and_dto_has_no_secret() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let id = add(&db, NewConnection { service: None, platform:"codex".into(), kind:"auth".into(), name:"Work".into(), secret:Some("synthetic-identity-only".into()), base_url:None, model:None, effort:None, context_1m:None, protocol:None , slug_base: None}).unwrap();
        db.execute("UPDATE connections SET account_email='person@example.test', name='Renamed' WHERE id=?1", [&id]).unwrap();
        set_paused(&db, &id, true).unwrap();
        let rows = list(&db).unwrap();
        assert_eq!(rows[0].account_email.as_deref(), Some("person@example.test"));
        assert_eq!(rows[0].name, "Renamed");
        let dto = serde_json::to_value(&rows[0]).unwrap();
        assert!(dto.get("secret").is_none());
        assert_eq!(dto["account_email"], "person@example.test");
        let refreshed = apply_local(&db, &id, Some(LocalPair { secret: "synthetic-replacement-identity".into(), base_url: None }), vec![]).unwrap();
        assert!(!refreshed.ok && !refreshed.changed);
        assert_eq!(list(&db).unwrap()[0].account_email.as_deref(), Some("person@example.test"));

    }

    #[test]
    fn serve_enabled_requires_capable_platform_and_roundtrips() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        // API 连接可开
        let api_id = add(&db, NewConnection { service: None, platform:"claude".into(), kind:"api".into(), name:"Kimi Key".into(), secret:Some("synthetic-key".into()), base_url:Some("https://api.moonshot.cn/anthropic".into()), model:None, effort:None, context_1m:None, protocol:Some("anthropic".into()) , slug_base: None}).unwrap();
        assert!(list(&db).unwrap()[0].serve_enabled == false, "新连接默认不对外服务");
        set_serve(&db, &api_id, true).unwrap();
        assert!(list(&db).unwrap()[0].serve_enabled);
        set_serve(&db, &api_id, false).unwrap();
        assert!(!list(&db).unwrap()[0].serve_enabled);
        // claude / codex 订阅可开
        let claude_id = add(&db, NewConnection { service: None, platform:"claude".into(), kind:"auth".into(), name:"Claude 官方".into(), secret:Some("synthetic-auth".into()), base_url:None, model:None, effort:None, context_1m:None, protocol:None , slug_base: None}).unwrap();
        set_serve(&db, &claude_id, true).unwrap();
        // 所有官方订阅都可选择；未就绪状态另行展示，不锁死来源开关。
        let gemini_id = add(&db, NewConnection { service: None, platform:"gemini".into(), kind:"auth".into(), name:"Gemini 订阅".into(), secret:Some("synthetic-auth".into()), base_url:None, model:None, effort:None, context_1m:None, protocol:None , slug_base: None}).unwrap();
        set_serve(&db, &gemini_id, true).unwrap();
        for platform in ["grok", "opencode", "workbuddy", "qoder", "trae", "zcode"] {
            db.execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES(?1,?1,'auth',?1,'synthetic-only',1,?1)", [platform]).unwrap();
            set_serve(&db, platform, true).unwrap();
            let enabled: bool = db.query_row("SELECT serve_enabled FROM connections WHERE id=?1", [platform], |r| r.get(0)).unwrap();
            assert!(enabled);
            set_serve(&db, platform, false).unwrap();
        }
        // 关闭不受能力限制（幂等清理路径）
        set_serve(&db, &gemini_id, false).unwrap();
        // 不存在的连接
        assert_eq!(set_serve(&db, "no-such", true).unwrap_err(), "连接不存在");
    }
}

#[cfg(test)]
mod auth_identity_sync_tests {
    use super::*;
    #[test]
    fn lone_subscription_is_not_overwritten_by_another_login() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        for platform in ["claude", "codex"] {
            db.execute("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES(?1,?1,'auth','original','original-auth','****',1)", [platform]).unwrap();
            let result = apply_local(&db, platform, Some(LocalPair { secret: "different-auth".into(), base_url: None }), vec![]).unwrap();
            assert!(!result.ok && !result.changed);
            assert_eq!(creds_of(&db, platform).unwrap().unwrap().secret.as_deref(), Some("original-auth"));
        }
    }
}

#[cfg(test)]
mod current_local_tests {
    use super::*;
    #[test]
    fn current_match_requires_kind_key_and_endpoint_without_oauth_fallback() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        for (id, kind, key, base) in [
            ("oauth", "auth", "oauth-key", None),
            ("first-gateway", "api", "shared-key", Some("https://first.test/v1")),
            ("second-gateway", "api", "shared-key", Some("https://second.test/v1")),
            ("official", "api", "official-key", None),
        ] {
            db.execute("INSERT INTO connections(id,platform,kind,name,secret,base_url,masked,created_ms) VALUES(?1,'codex',?2,?1,?3,?4,'****',1)", params![id,kind,key,base]).unwrap();
        }
        let mut local = crate::current_connection::LocalConnection { platform:"codex".into(), kind:"api".into(), secret:"unregistered-key".into(), base_url:None, protocol:None, source_agent:None, masked_override:None, detected_email:None };
        assert!(find_current_local(&db, &local).unwrap().is_none());
        local.secret="oauth-key".into();
        assert!(find_current_local(&db, &local).unwrap().is_none(), "API不能命中残留Auth");
        local.secret="shared-key".into();local.base_url=Some(" https://second.test/v1/ ".into());
        assert_eq!(find_current_local(&db, &local).unwrap().as_deref(), Some("second-gateway"));
        local.secret="official-key".into();local.base_url=Some("https://api.openai.com/v1/".into());
        assert_eq!(find_current_local(&db, &local).unwrap().as_deref(), Some("official"));
    }
}
