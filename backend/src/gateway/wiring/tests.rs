    use super::*;
    use crate::gateway::wiring_store::TierModels;

    // 不注入档位「关于」的测试桩：档位 about 断言在专项测试里做
    fn no_about(_: &str) -> (Option<String>, Option<String>) { (None, None) }

    #[test]
    fn claude_wire_and_restore_round_trip_preserves_other_fields() {
        let original = r#"{"env":{"SOME_KEY":"keep","ANTHROPIC_BASE_URL":"https://old"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", None, None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["SOME_KEY"], "keep");
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:12731");
        assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], "jolu-claude-code");
        // 还原回原值
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://old");
        assert_eq!(value["env"]["SOME_KEY"], "keep");
        assert!(value["env"].get("ANTHROPIC_AUTH_TOKEN").is_none(), "原本没有的键还原后要删掉");
    }

    #[test]
    fn claude_wire_writes_main_model_into_tiers_and_top_level() {
        // magpie writeTiers 语义：主模型写顶层 model、四档 env 与 small-fast 全部跟随；
        // ANTHROPIC_MODEL 必须删除——它压过 /model 的选择，写了切换只在本会话生效。
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["model"], "zcode/glm-4.6", "主模型在 settings 顶层");
        assert!(value["env"].get("ANTHROPIC_MODEL").is_none(), "ANTHROPIC_MODEL 要删掉");
        for tier in ["opus", "sonnet", "haiku"] {
            assert_eq!(value["env"][claude_tier_env(tier)], "zcode/glm-4.6", "{tier} 档跟随主模型");
        }
        // B3：Fable 档跟随别家主模型时留空（magpie #2090）
        assert!(value["env"].get(claude_tier_env("fable")).is_none(), "Fable 档不写别家模型");
        assert!(value["env"].get("ANTHROPIC_DEFAULT_FABLE_MODEL_NAME").is_none());
        assert_eq!(value["env"]["ANTHROPIC_SMALL_FAST_MODEL"], "zcode/glm-4.6", "后台小任务也要能路由");
        // 每档写入显示名与描述（/model 菜单的档位标签），旧「关于」值被覆盖不再张冠李戴
        for tier in ["opus", "sonnet", "haiku"] {
            assert_eq!(value["env"][format!("{}{}", claude_tier_env(tier), "_NAME")], "zcode/glm-4.6 显示名");
            assert_eq!(value["env"][format!("{}{}", claude_tier_env(tier), "_DESCRIPTION")], "zcode/glm-4.6 的描述，用于 /model 菜单");
        }
    }

    #[test]
    fn claude_wire_stales_tier_about_keys_and_restores_them() {
        // 用户原来给 opus 档写过 _NAME：接线换模型后要清掉（不然 /model 显示错模型），
        // 断开时还原成用户写的。
        let name_key = format!("{}{}", claude_tier_env("opus"), "_NAME");
        let original = serde_json::json!({"env": {&name_key: "我的自定义 Opus"}});
        let snapshot = claude_snapshot(serde_json::to_string(&original).unwrap().as_str(), None, None, &[]).unwrap();
        // 无显示名（label=None）：旧「关于」值清掉，/model 退回显示模型 id
        let wired = claude_wire(
            &serde_json::to_string(&original).unwrap(),
            "http://127.0.0.1:12731",
            "tok",
            Some("zcode/glm-4.6"),
            None,
            None,
            None,
        None, &no_about, &ClaudeExtras::default())
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        // label 缺省时退回模型 id 本身——旧「关于」值照样被覆盖，不再指错模型
        assert_eq!(value["env"][&name_key], "zcode/glm-4.6");
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"][&name_key], "我的自定义 Opus");
    }

    #[test]
    fn claude_subagent_model_three_states() {
        // 给了 subagent 模型：写进 env
        let wired = claude_wire(
            "{}", "http://127.0.0.1:12731", "tok",
            Some("zcode/glm-4.6"), None, None,
            Some("kimi/kimi-k2.5"),
        None, &no_about, &ClaudeExtras::default())
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "kimi/kimi-k2.5");

        // 没给：删键（跟随主模型，Claude Code 的缺省）
        let original = r#"{"env":{"CLAUDE_CODE_SUBAGENT_MODEL":"old/model"}}"#;
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert!(value["env"].get("CLAUDE_CODE_SUBAGENT_MODEL").is_none(), "不给就删键＝跟随主模型");

        // 断开还原：用户原来的 subagent 模型要回来
        let original = r#"{"env":{"CLAUDE_CODE_SUBAGENT_MODEL":"user/own-sub"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), None, None, Some("zcode/glm-4.6"), None, &no_about, &ClaudeExtras::default()).unwrap();
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "user/own-sub");
    }

    #[test]
    fn claude_tiers_own_model_writes_tier_env_with_effort_and_own_about() {
        // 批 4 档位自选（magpie tier_own + tierWith 同款）：
        // 自选档写 `model[:effort]`（可带 [1m]），「关于」按该档自己的模型取；
        // 未自选的档照旧跟随主模型。
        let tiers = TierModels {
            opus: None,
            sonnet: None,
            haiku: Some("zcode/glm-5.3-flash:low".into()),
            fable: Some("zcode/glm-5.3-flash".into()),
            auto_mode: Some("kimi/kimi-k2.5".into()),
        };
        let tier_about = |value: &str| -> (Option<String>, Option<String>) {
            // 模拟调用方按 models.dev 查档位模型的显示名
            let name = if value.starts_with("zcode/glm-5.3-flash") { "GLM-5.3-Flash" } else { "Kimi K2.5" };
            (Some(name.to_string()), Some(format!("{name} 的描述")))
        };
        let wired = claude_wire(
            "{}", "http://127.0.0.1:12731", "tok",
            Some("zcode/glm-4.6"), Some("GLM-4.6 主显示名"), Some("主模型描述"),
            Some("zcode/glm-5.3-flash"), Some(&tiers), &tier_about, &ClaudeExtras::default(),
        ).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        // 自选档：写入值带 effort 编码，「关于」用档位模型自己的名字
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "zcode/glm-5.3-flash:low");
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME"], "GLM-5.3-Flash", "/model 菜单显示档位实际用的模型");
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"], "zcode/glm-5.3-flash", "不带 effort 就裸模型名");
        // 未自选档：照旧跟随主模型，「关于」也用主模型的
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "zcode/glm-4.6");
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL_NAME"], "GLM-4.6 主显示名");
        // Auto 判别：写了 CLAUDE_CODE_AUTO_MODE_MODEL
        assert_eq!(value["env"]["CLAUDE_CODE_AUTO_MODE_MODEL"], "kimi/kimi-k2.5");
        // 子代理照常
        assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "zcode/glm-5.3-flash");
    }

    #[test]
    fn claude_tiers_none_keeps_follow_behavior_and_clears_auto_mode() {
        // 全跟随（tiers = None，等价旧快照升级）：行为与批 4 之前完全一致，
        // 且 CLAUDE_CODE_AUTO_MODE_MODEL 要删键——Claude Code 自己挑分类器模型
        let original = r#"{"env":{"CLAUDE_CODE_AUTO_MODE_MODEL":"stale/model","CLAUDE_CODE_SUBAGENT_MODEL":"zcode/glm-5.3-flash"}}"#;
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), None, None, Some("zcode/glm-5.3-flash"), None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        for tier in ["opus", "sonnet", "haiku"] {
            assert_eq!(value["env"][claude_tier_env(tier)], "zcode/glm-4.6", "{tier} 跟随主模型");
        }
        assert!(value["env"].get(claude_tier_env("fable")).is_none(), "Fable 跟随别家主模型时留空");
        assert!(value["env"].get("CLAUDE_CODE_AUTO_MODE_MODEL").is_none(), "Auto 判别没自选就删键");
        assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "zcode/glm-5.3-flash");
    }

    #[test]
    fn claude_read_model_follows_magpie_priority_and_resolves_aliases() {
        // 顶层 model 优先（/model 写这里）
        let text = r#"{"model":"kimi/kimi-k2.5","env":{"ANTHROPIC_MODEL":"x/y","ANTHROPIC_DEFAULT_SONNET_MODEL":"z/w"}}"#;
        assert_eq!(claude_read_model(text), Some(("kimi/kimi-k2.5".into(), None)));
        // 别名：按档位 env 解析成真实模型；best / opusplan 按 opus
        let text = r#"{"model":"opus","env":{"ANTHROPIC_DEFAULT_OPUS_MODEL":"zcode/glm-5.3[1m]"}}"#;
        assert_eq!(claude_read_model(text), Some(("opus".into(), Some("zcode/glm-5.3[1m]".into()))));
        let text = r#"{"model":"best","env":{"ANTHROPIC_DEFAULT_OPUS_MODEL":"a/b"}}"#;
        assert_eq!(claude_read_model(text).unwrap().1.as_deref(), Some("a/b"));
        // 没有顶层 model：ANTHROPIC_MODEL → 默认档
        assert_eq!(claude_read_model(r#"{"env":{"ANTHROPIC_DEFAULT_SONNET_MODEL":"z/w"}}"#).unwrap().0, "z/w");
        assert_eq!(claude_read_model(r#"{"env":{}}"#), None);
        // 比较忽略 [1m] / :effort
        assert_eq!(model_key("zcode/GLM-5.3[1m]:high"), model_key("zcode/glm-5.3"));
    }

    #[test]
    fn claude_fable_follows_only_fable_family_or_own_pick() {
        // 主模型本身是 Fable 家族：跟随照写
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", Some("claude/claude-fable-5-2"), None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"], "claude/claude-fable-5-2");
        // 用户自选 Fable 档：照写
        let tiers = TierModels { opus: None, sonnet: None, haiku: None, fable: Some("kimi/kimi-k2.5".into()), auto_mode: None };
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", Some("zcode/glm-5.3"), None, None, None, Some(&tiers), &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"], "kimi/kimi-k2.5", "自选档照写");
        // 用户原来有 Fable 档：接线期间移除，断开时还原
        let original = r#"{"env":{"ANTHROPIC_DEFAULT_FABLE_MODEL":"claude-fable-5-2"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-5.3"), None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&wired).unwrap()["env"].get("ANTHROPIC_DEFAULT_FABLE_MODEL").is_none());
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&restored).unwrap()["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"], "claude-fable-5-2");
    }

    #[test]
    fn claude_extras_write_read_back_and_restore() {
        // 用户原有自己的最大输出；上下文窗口原本没有
        let original = r#"{"env":{"CLAUDE_CODE_MAX_OUTPUT_TOKENS":"64000","OTHER":"keep"}}"#;
        let extras = ClaudeExtras {
            max_output: Some(131_072),
            context_window: Some(1_000_000),
            compact_window: Some(272_000),
            capabilities: Some("zcode/glm-5.3=effort,xhigh_effort,max_effort".into()),
            previous: Default::default(),
        };
        let snapshot = claude_snapshot(original, None, None, &[]).and_then(|s| drop_our_extras(s, &extras.previous)).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-5.3"), None, None, None, None, &no_about, &extras).unwrap();
        // 写入 → 读回
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"][CLAUDE_MAX_OUTPUT], "131072", "B1：绕开 32000 默认");
        assert_eq!(value["env"][CLAUDE_MAX_CONTEXT], "1000000", "B2：大窗口不再按 200K");
        assert_eq!(value["env"][CLAUDE_COMPACT_WINDOW], "272000", "B5");
        assert_eq!(value["env"][CLAUDE_MODEL_CAPABILITIES], "zcode/glm-5.3=effort,xhigh_effort,max_effort", "B6");
        assert_eq!(value["env"][CLAUDE_AUTO_MODE_SERVER], "0", "B7：别家主模型关掉 auto review");
        // 断开还原：用户的 64000 回来，我们加的键删干净，其它键不动
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"][CLAUDE_MAX_OUTPUT], "64000");
        assert_eq!(value["env"]["OTHER"], "keep");
        for key in [CLAUDE_MAX_CONTEXT, CLAUDE_COMPACT_WINDOW, CLAUDE_MODEL_CAPABILITIES, CLAUDE_AUTO_MODE_SERVER] {
            assert!(value["env"].get(key).is_none(), "{key} 原本没有，还原后删掉");
        }
        // 重新接线：上次写下的派生值不被当成用户原值
        let applied = claude_extra_env("zcode/glm-5.3", &extras);
        let resnap = claude_snapshot(&wired, Some("zcode/glm-5.3"), None, &[]).and_then(|s| drop_our_extras(s, &applied));
        assert!(resnap.as_ref().and_then(|s| s.get("env")).map_or(true, |env| env.get(CLAUDE_MAX_CONTEXT).is_none() && env.get(CLAUDE_AUTO_MODE_SERVER).is_none()), "{resnap:?}");
    }

    #[test]
    fn claude_extras_unknown_values_are_not_guessed_and_stale_ones_are_removed() {
        // Claude 家族主模型：不关 auto review；元数据缺失：窗口与输出都不写
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", Some("claude/claude-sonnet-5[1m]"), None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        for key in CLAUDE_EXTRA_KEYS {
            assert!(value["env"].get(key).is_none(), "{key} 不猜");
        }
        // 换成元数据缺失的模型：上次我们写的窗口删掉，用户自己改过的值不动
        let previous: std::collections::BTreeMap<String, String> =
            [(CLAUDE_MAX_CONTEXT.to_string(), "1000000".to_string()), (CLAUDE_MAX_OUTPUT.to_string(), "131072".to_string())].into();
        let current = r#"{"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":"1000000","CLAUDE_CODE_MAX_OUTPUT_TOKENS":"50000"}}"#;
        let extras = ClaudeExtras { previous, ..Default::default() };
        let wired = claude_wire(current, "http://127.0.0.1:12731", "tok", Some("other/unknown-model"), None, None, None, None, &no_about, &extras).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert!(value["env"].get(CLAUDE_MAX_CONTEXT).is_none(), "旧模型的窗口不能留给新模型");
        assert_eq!(value["env"][CLAUDE_MAX_OUTPUT], "50000", "用户自己改过的值不动");
    }

    #[test]
    fn tier_models_serde_round_trip_and_legacy_default() {
        // 旧快照（无 tiers 字段）反序列化为 None = 全跟随；新快照往返一致
        let legacy: Applied = serde_json::from_str(
            r#"{"base":"http://127.0.0.1:12731","model":"zcode/glm-4.6"}"#,
        ).unwrap();
        assert!(legacy.tiers.is_none(), "旧快照没有档位自选");
        let applied = Applied {
            base: "http://127.0.0.1:12731".into(), token: None, provider: None, mode: None,
            model: Some("zcode/glm-4.6".into()), label: None, subagent_model: None,
            tiers: Some(TierModels {
                opus: None, sonnet: None,
                haiku: Some("zcode/glm-5.3-flash:low".into()), fable: None, auto_mode: None,
            }),
            extra_env: Default::default(),
        };
        let json = serde_json::to_string(&applied).unwrap();
        let back: Applied = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tiers.as_ref().and_then(|t| t.haiku.as_deref()), Some("zcode/glm-5.3-flash:low"));
        assert_eq!(back.tiers.as_ref().and_then(|t| t.opus.as_deref()), None);
    }

    #[test]
    fn rewire_snapshot_does_not_capture_our_own_tier_and_auto_values() {
        // 用户原本没有任何模型配置 → 没有存快照；重新接线时文件里全是我们写的值
        let tiers = TierModels { opus: Some("claude-me/claude-opus-4-5:high".into()), sonnet: None, haiku: None, fable: None, auto_mode: Some("zcode/glm-5.3-flash".into()) };
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code.abcd1234", Some("zcode/glm-5.3"), Some("GLM 5.3"), None, None, Some(&tiers), &|v: &str| (Some(format!("{v} 名")), None), &ClaudeExtras::default()).unwrap();
        let applied = Applied { tiers: Some(tiers.clone()), extra_env: claude_extra_env("zcode/glm-5.3", &ClaudeExtras::default()), ..Default::default() };
        let snapshot = claude_snapshot(&wired, Some("zcode/glm-5.3"), Some("GLM 5.3"), &applied.written_values())
            .and_then(|saved| drop_our_extras(saved, &applied.extra_env));
        assert!(snapshot.is_none(), "自选档位值、Auto 值、自选档位的显示名都是我们写的：{snapshot:?}");
        // 断开：没有快照时这些键也要删干净
        let restored = claude_restore(&wired, None, Some("zcode/glm-5.3"), Some("GLM 5.3"), &applied.written_values()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value.get("env").map_or(true, |env| env.get("CLAUDE_CODE_AUTO_MODE_MODEL").is_none() && env.get("ANTHROPIC_DEFAULT_OPUS_MODEL").is_none()), "{restored}");
    }

    #[test]
    fn tiers_declare_their_effort_capabilities() {
        let tiers = TierModels { opus: Some("c/claude-opus-4-5".into()), sonnet: None, haiku: Some("c/claude-haiku-4-5".into()), fable: None, auto_mode: None };
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-5.3"), None, None, None, Some(&tiers), &no_about, &ClaudeExtras::default()).unwrap();
        let env = &serde_json::from_str::<serde_json::Value>(&wired).unwrap()["env"];
        assert_eq!(env["ANTHROPIC_DEFAULT_OPUS_MODEL_SUPPORTED_CAPABILITIES"], "effort", "Opus 4.5 只到 high");
        assert!(env.get("ANTHROPIC_DEFAULT_HAIKU_MODEL_SUPPORTED_CAPABILITIES").is_none(), "Haiku 不收 effort，不写");
        assert_eq!(env["ANTHROPIC_DEFAULT_SONNET_MODEL_SUPPORTED_CAPABILITIES"], "effort,xhigh_effort,max_effort", "跟随主模型（别家模型）给全集");
    }

    #[test]
    fn effort_level_is_fitted_to_the_new_main_model_and_restored_on_unwire() {
        assert_eq!(fit_effort("c/claude-opus-4-5", "max"), Some(Some("high".into())), "不收 max 时降到 high");
        assert_eq!(fit_effort("c/claude-opus-4-6", "xhigh"), Some(Some("high".into())), "跳过不收的 xhigh");
        assert_eq!(fit_effort("c/claude-haiku-4-5", "high"), Some(None), "不收 effort 就删掉");
        assert_eq!(fit_effort("zcode/glm-5.3", "max"), None, "收的不动");
        let original = r#"{"effortLevel":"max","env":{"KEEP":"1"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", Some("c/claude-opus-4-5"), None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&wired).unwrap()["effortLevel"], "high");
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&restored).unwrap()["effortLevel"], "max", "断开还原用户原值");
    }

    #[test]
    fn a_kept_client_key_still_counts_as_our_wiring() {
        let text = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-ck-laptop-key"}}"#;
        assert!(!ours("claude-code", text, 12731, "abcd1234"), "不是 Jolu 令牌，单看文件不算");
        assert!(ours_with("claude-code", text, 12731, "abcd1234", Some("jolu-ck-laptop-key")), "接线记录里保留的就是它");
        assert!(!ours_with("claude-code", text, 12731, "abcd1234", Some("")), "空值不算");
    }

    #[test]
    fn top_level_model_and_env_model_together_raise_a_notice() {
        assert_eq!(claude_conflicts(r#"{"model":"opus","env":{"ANTHROPIC_MODEL":"sonnet"}}"#).len(), 1);
        assert!(claude_conflicts(r#"{"model":"opus"}"#).is_empty());
        assert!(claude_conflicts(r#"{"env":{"ANTHROPIC_MODEL":"sonnet"}}"#).is_empty());
        assert!(claude_conflicts("not json").is_empty());
    }

    #[test]
    fn claude_snapshot_excludes_our_own_subagent_write() {
        // 重新接线时，文件里的 subagent 模型是我们上次写的，不是用户原值
        let rewire = r#"{"env":{"CLAUDE_CODE_SUBAGENT_MODEL":"zcode/glm-4.6"}}"#;
        let snapshot = claude_snapshot(rewire, None, None, &["zcode/glm-4.6".to_string()]);
        assert!(snapshot.is_none(), "我们写的 subagent 模型不该进快照");
    }

    #[test]
    fn claude_wire_stashes_and_removes_cloud_switches() {
        // 云开关会把请求引去 Bedrock/Vertex、绕过 ANTHROPIC_BASE_URL：接线期间移除，断开还原
        let original = r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1","AWS_REGION":"us-east-1"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert!(value["env"].get("CLAUDE_CODE_USE_BEDROCK").is_none(), "云开关接线期间要移除");
        assert_eq!(value["env"]["AWS_REGION"], "us-east-1", "云专属变量不动");
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"]["CLAUDE_CODE_USE_BEDROCK"], "1");
    }

    #[test]
    fn claude_wire_without_model_leaves_existing_mapping_untouched() {
        // 不给模型时不写也不删：用户可能已经把映射配在别处（环境变量 / 命令行）。
        let original = r#"{"env":{"ANTHROPIC_MODEL":"already/set"}}"#;
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", None, None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "already/set");
        assert!(value["env"].get("ANTHROPIC_SMALL_FAST_MODEL").is_none(), "没给模型就不该凭空加键");
        assert!(value.get("model").is_none(), "没给模型就不动顶层 model");
    }

    #[test]
    fn claude_wire_ignores_blank_model() {
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", Some("   "), Some("    显示名"), Some("    的描述，用于 /model 菜单"), None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert!(value.get("model").is_none(), "空白模型名不应写成空字符串");
    }

    #[test]
    fn claude_restore_puts_back_the_original_model_mapping() {
        // 用户本来就有自己的档位映射：断开接线必须还原成他的，而不是删掉。
        let original = r#"{"env":{"ANTHROPIC_MODEL":"user/own","ANTHROPIC_SMALL_FAST_MODEL":"user/fast","ANTHROPIC_DEFAULT_OPUS_MODEL":"user/opus"},"model":"user-main"}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["model"], "zcode/glm-4.6", "顶层 model 换成主模型");
        assert!(value["env"].get("ANTHROPIC_MODEL").is_none());
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "user/own");
        assert_eq!(value["env"]["ANTHROPIC_SMALL_FAST_MODEL"], "user/fast");
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "user/opus");
        assert_eq!(value["model"], "user-main", "顶层 model 还原");
    }

    #[test]
    fn claude_restore_removes_model_keys_we_added() {
        // 用户原本没有模型映射：断开后要删掉，不能留下指向网关来源的模型名。
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about, &ClaudeExtras::default()).unwrap();
        // prior_model 来自接线记录的 applied：断开时靠它识别「顶层 model / 档位键是我们写的」
        let restored = claude_restore(&wired, None, Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value.get("env").is_none(), "全部是网关写入的键，env 段要清空");
        assert!(value.get("model").is_none(), "顶层 model 也要清掉");
    }

    #[test]
    fn claude_restore_accepts_legacy_flat_snapshot() {
        // 升级兼容：旧版接线记录的快照是 env 键的平面表，没有 env 子对象。
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", None, None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let legacy = serde_json::json!({"ANTHROPIC_BASE_URL": "https://old.example"});
        let restored = claude_restore(&wired, Some(&legacy), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://old.example");
        assert!(value["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    }

    #[test]
    fn claude_snapshot_records_top_level_model() {
        let original = r#"{"model":"user-main","env":{"ANTHROPIC_BASE_URL":"https://old"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        assert_eq!(snapshot["model"], "user-main");
        assert_eq!(snapshot["env"]["ANTHROPIC_BASE_URL"], "https://old");
    }

    #[test]
    fn claude_restore_cleans_empty_env() {
        let wired = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code"}}"#;
        let restored = claude_restore(wired, None, None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value.get("env").is_none(), "只装了网关配置的 env 段要整个清掉");
    }

    #[test]
    fn snapshot_filters_any_gateway_value() {
        // 重新接线时，上一次写进去的网关地址 / 令牌 / 主模型不能被当成用户原来的上游
        let rewire = r#"{"model":"zcode/glm-4.6","env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code.abcdef12","ANTHROPIC_DEFAULT_OPUS_MODEL":"zcode/glm-4.6","KEEP":"1"}}"#;
        // 快照只收接线键清单里的用户原值；KEEP 这种接线不触碰的键不在清单里，本就不收
        let snapshot = claude_snapshot(rewire, Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), &[]);
        assert!(snapshot.is_none(), "管理范围内的键全是网关值时应视为没有原值");
        let mixed = r#"{"env":{"ANTHROPIC_BASE_URL":"https://real.example","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code"}}"#;
        let snapshot = claude_snapshot(mixed, None, None, &[]).unwrap();
        assert_eq!(snapshot["env"]["ANTHROPIC_BASE_URL"], "https://real.example");
        assert!(snapshot["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    }

    #[test]
    fn codex_provider_mode_round_trip_keeps_comments_and_writes_token() {
        let original = "# 注释要保留\nmodel_provider = \"openai\"\n\n[model_providers.openai]\nname = \"OpenAI\"\n";
        let snapshot = codex_snapshot(original).unwrap();
        assert_eq!(snapshot["model_provider"], "openai");
        let wired = codex_wire(
            original,
            "http://127.0.0.1:12731",
            "jolu-gateway",
            false,
            Some("jolu-codex.abcd1234"),
        )
        .unwrap();
        assert!(wired.contains("# 注释要保留"), "{wired}");
        assert!(wired.contains("jolu-gateway"), "{wired}");
        assert!(wired.contains("wire_api = \"responses\""), "{wired}");
        assert!(wired.contains("experimental_bearer_token = \"jolu-codex.abcd1234\""), "{wired}");
        assert!(ours("codex", &wired, 12731, "abcd1234"));
        // 别的实例的 token 要能认出来
        assert_eq!(foreign_marker("codex", &wired, "ffffffff").as_deref(), Some("abcd1234"));
        let restored = codex_restore(&wired, Some(&snapshot), "jolu-gateway").unwrap();
        assert!(restored.contains("# 注释要保留"), "{restored}");
        assert!(restored.contains("model_provider = \"openai\""), "{restored}");
        assert!(!restored.contains("jolu-gateway"), "还原要删掉 Jolu 的 provider：{restored}");
        assert!(!restored.contains("experimental_bearer_token"), "{restored}");
        // 重新接线不能把 jolu-gateway 当原值
        assert_eq!(codex_snapshot(&wired), None);
    }

    #[test]
    fn codex_oauth_mode_keeps_builtin_provider_and_restores_it() {
        // 已登录 ChatGPT：只写顶层 openai_base_url，不覆盖 model_provider
        let original = "model_provider = \"openai\"\nmodel = \"gpt-5.5\"\n";
        let snapshot = codex_snapshot(original).unwrap();
        let wired = codex_wire(original, "http://127.0.0.1:12731", "jolu-gateway", true, None).unwrap();
        assert!(wired.contains("openai_base_url = \"http://127.0.0.1:12731/backend-api/codex\""), "{wired}");
        assert!(!wired.contains("model_provider = \"jolu-gateway\""), "{wired}");
        assert!(!wired.contains("[model_providers.jolu-gateway]"), "{wired}");
        assert!(ours("codex", &wired, 12731, "abcd1234"));
        assert!(!wired.contains("model_provider ="), "OAuth 模式要用内置 provider：{wired}");
        let restored = codex_restore(&wired, Some(&snapshot), "jolu-gateway").unwrap();
        assert!(!restored.contains("openai_base_url"), "原本没有该键，还原时要删掉：{restored}");
        assert!(restored.contains("model_provider = \"openai\""), "{restored}");
        // 原本有 openai_base_url 的用户要能原样拿回
        let original = "model_provider = \"openai\"\nopenai_base_url = \"https://relay.example\"\n";
        let snapshot = codex_snapshot(original).unwrap();
        let wired = codex_wire(original, "http://127.0.0.1:12731", "jolu-gateway", true, None).unwrap();
        let restored = codex_restore(&wired, Some(&snapshot), "jolu-gateway").unwrap();
        assert!(restored.contains("openai_base_url = \"https://relay.example\""), "{restored}");
    }

    #[test]
    fn codex_restore_keeps_provider_switched_by_user() {
        let wired = "model_provider = \"jolu-gateway\"\n[model_providers.jolu-gateway]\nbase_url = \"http://127.0.0.1:12731\"\n";
        let switched = wired.replace("model_provider = \"jolu-gateway\"", "model_provider = \"openai\"");
        let restored = codex_restore(&switched, Some(&serde_json::json!({"model_provider":"zhipu"})), "jolu-gateway").unwrap();
        assert!(restored.contains("model_provider = \"openai\""), "用户切换后的 provider 不能被改回去：{restored}");
        assert!(!restored.contains("jolu-gateway"));
    }

    #[test]
    fn opencode_and_zcode_add_and_remove_own_entry_only() {
        let base = r#"{"provider":{"existing":{"options":{"baseURL":"https://keep"}}}}"#;
        let wired = opencode_wire(base, "http://127.0.0.1:12731", "jolu-opencode").unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["provider"]["existing"]["options"]["baseURL"], "https://keep");
        assert_eq!(value["provider"]["jolu"]["options"]["baseURL"], "http://127.0.0.1:12731");
        assert_eq!(opencode_current_base(&wired).as_deref(), Some("http://127.0.0.1:12731"));
        assert_eq!(opencode_current_token(&wired).as_deref(), Some("jolu-opencode"));
        let restored = opencode_restore(&wired, None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value["provider"].get("jolu").is_none());
        assert!(value["provider"].get("existing").is_some(), "别人的 provider 不能动");

        let wired = zcode_rules_wire("{}", "http://127.0.0.1:12731", "jolu-zcode", &[]).unwrap();
        assert!(wired.contains("127.0.0.1:12731"));
        assert_eq!(zcode_current_base(&wired).as_deref(), Some("http://127.0.0.1:12731"));
        let restored = zcode_rules_restore(&wired, None).unwrap();
        assert!(!restored.contains("jolu"));
    }

    #[test]
    fn zcode_rules_add_our_provider_and_model_limits_and_restore_leaves_the_rest() {
        let existing = r#"{"schemaVersion":1,"config":{"providerOrder":["builtin:bigmodel"],
            "providerConfigRules":{"providerRules":[{"providerId":"builtin:bigmodel","providerName":"BigModel","config":{"group":"g"}}]},
            "modelConfigRules":{"providerModelRules":[{"providerId":"builtin:bigmodel","modelId":"GLM-5.3"}],
              "manualProviderModelRules":[{"providerId":"jolu","modelId":"kimi/k2","config":{"mine":true}}]}}}"#;
        let models = vec![
            ZcodeModel { id: "kimi/k2".into(), name: "Kimi K2".into(), context: 256_000, output: 32_768, image: false },
            ZcodeModel { id: "group/main".into(), name: "group/main".into(), context: 0, output: 0, image: true },
        ];
        let wired = zcode_rules_wire(existing, "http://127.0.0.1:12731", "jolu-zcode.abcd1234", &models).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["config"]["providerOrder"], serde_json::json!(["builtin:bigmodel", "jolu"]));
        let rules = value["config"]["providerConfigRules"]["providerRules"].as_array().unwrap();
        assert_eq!(rules.len(), 2, "别人的 provider 规则保留");
        assert_eq!(rules[1]["config"]["api"]["baseUrl"], "http://127.0.0.1:12731");
        assert_eq!(rules[1]["config"]["personalModelIds"], serde_json::json!(["kimi/k2", "group/main"]));
        let model_rules = value["config"]["modelConfigRules"]["providerModelRules"].as_array().unwrap();
        assert_eq!(model_rules.len(), 3);
        assert_eq!(model_rules[1]["config"]["optionSpecs"]["maxOutputTokens"]["max"], 32_768, "知道输出上限才写，防 32000 截断");
        assert!(model_rules[2]["config"].get("optionSpecs").is_none(), "不知道就不编");
        assert_eq!(model_rules[2]["config"]["properties"]["contextWindow"], 200_000);
        assert_eq!(model_rules[2]["config"]["properties"]["inputFormat"]["supportsImage"], true);
        assert_eq!(value["config"]["modelConfigRules"]["manualProviderModelRules"][0]["config"]["mine"], true, "用户手动设的不动");
        assert_eq!(zcode_current_token(&wired).as_deref(), Some("jolu-zcode.abcd1234"));
        // 重复接线不重复添加
        let again: serde_json::Value = serde_json::from_str(&zcode_rules_wire(&wired, "http://127.0.0.1:12731", "t", &models).unwrap()).unwrap();
        assert_eq!(again["config"]["providerConfigRules"]["providerRules"].as_array().unwrap().len(), 2);
        assert_eq!(again["config"]["providerOrder"].as_array().unwrap().len(), 2);
        // 断开：只去掉自己的
        let restored: serde_json::Value = serde_json::from_str(&zcode_rules_restore(&wired, None).unwrap()).unwrap();
        assert_eq!(restored["config"]["providerOrder"], serde_json::json!(["builtin:bigmodel"]));
        assert_eq!(restored["config"]["providerConfigRules"]["providerRules"].as_array().unwrap().len(), 1);
        assert_eq!(restored["config"]["modelConfigRules"]["providerModelRules"].as_array().unwrap().len(), 1);
        assert_eq!(restored["config"]["modelConfigRules"]["manualProviderModelRules"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn zcode_config_json_keeps_a_provider_the_user_turned_off() {
        let existing = r#"{"provider":{"builtin:zai":{"name":"Z.ai"},"jolu":{"enabled":false}}}"#;
        let models = vec![ZcodeModel { id: "kimi/k2".into(), name: "Kimi K2".into(), context: 0, output: 8_192, image: false }];
        let wired: serde_json::Value = serde_json::from_str(&zcode_config_wire(existing, "http://127.0.0.1:12731", "t", &models).unwrap()).unwrap();
        assert_eq!(wired["provider"]["jolu"]["enabled"], false);
        assert_eq!(wired["provider"]["jolu"]["models"]["kimi/k2"]["limit"]["output"], 8_192);
        assert!(wired["provider"].get("builtin:zai").is_some());
        let restored: serde_json::Value = serde_json::from_str(&zcode_config_restore(&wired.to_string(), None).unwrap()).unwrap();
        assert!(restored["provider"].get("jolu").is_none() && restored["provider"].get("builtin:zai").is_some());
    }

    // ---------- Kimi（config.toml；参照 magpie kimi.go 的行为） ----------

    fn kimi_models() -> Vec<ZcodeModel> {
        vec![
            ZcodeModel { id: "zcode/GLM-5.3".into(), name: "GLM-5.3".into(), context: 200_000, output: 32_768, image: true },
            ZcodeModel { id: "grok/grok-4".into(), name: "grok/grok-4".into(), context: 0, output: 0, image: false },
        ]
    }

    /// 读接线后 config.toml 里 `models."jolu/<id>"` 表（不存在时 panic，便于断言）。
    fn kimi_table(text: &str, id: &str) -> toml_edit::Table {
        text.parse::<toml_edit::DocumentMut>()
            .unwrap()
            .get("models")
            .and_then(|models| models.get(&format!("jolu/{id}")))
            .and_then(|item| item.as_table())
            .cloned()
            .unwrap_or_else(|| panic!("缺少 models.\"jolu/{id}\" 表：{text}"))
    }

    fn kimi_str(table: &toml_edit::Table, key: &str) -> String {
        table.get(key).and_then(|value| value.as_str()).unwrap_or_default().to_string()
    }

    #[test]
    fn kimi_wire_writes_provider_tables_default_and_round_trips() {
        let models = kimi_models();
        // 空配置首次接线：目录不存在也要能写成完整 config.toml
        let (wired, snapshot) = (kimi_wire("", "http://127.0.0.1:12731", "jolu-kimi.abcd1234", None, &models, false).unwrap(), kimi_snapshot(""));
        assert!(snapshot.is_none(), "原来没有 default_model 就不用存");
        let document = wired.parse::<toml_edit::DocumentMut>().unwrap();
        let provider = kimi_provider_read(&document).unwrap();
        assert_eq!(kimi_str(provider, "type"), "kimi");
        assert_eq!(kimi_str(provider, "base_url"), "http://127.0.0.1:12731");
        assert_eq!(kimi_str(provider, "api_key"), "jolu-kimi.abcd1234");
        // 第一个模型：全字段；上下文/输出/图片都来自目录
        let first = kimi_table(&wired, "zcode/GLM-5.3");
        assert_eq!(kimi_str(&first, "provider"), "jolu");
        assert_eq!(kimi_str(&first, "model"), "zcode/GLM-5.3", "发给上游的是网关模型 id，网关按它选路");
        assert_eq!(kimi_str(&first, "display_name"), "GLM-5.3", "目录给了别称");
        assert_eq!(first.get("max_context_size").and_then(|v| v.as_integer()), Some(200_000));
        assert_eq!(first.get("max_output_size").and_then(|v| v.as_integer()), Some(32_768));
        let caps: Vec<String> = first.get("capabilities").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        assert!(caps.contains(&"image_in".to_string()) && caps.contains(&"tool_use".to_string()), "{caps:?}");
        // 第二个模型：目录缺上下文退 128000（magpie kimiContext）、无输出上限、名字等于 id 就不写 display_name
        let second = kimi_table(&wired, "grok/grok-4");
        assert_eq!(second.get("max_context_size").and_then(|v| v.as_integer()), Some(128_000));
        assert!(second.get("display_name").is_none());
        assert!(second.get("max_output_size").is_none());
        let caps: Vec<String> = second.get("capabilities").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        assert_eq!(caps, vec!["tool_use".to_string()], "{caps:?}");
        // 默认模型 = 清单第一个
        assert_eq!(document.get("default_model").and_then(|v| v.as_str()), Some("jolu/zcode/GLM-5.3"));
        // 状态判定：base + token + default_model 都是才算已接入
        assert!(ours_with("kimi", &wired, 12731, "abcd1234", None));
        // 断开：原来没有 default_model → 连键一起删；我们的表与 provider 不留残余
        let restored = kimi_restore(&wired, None).unwrap();
        let document = restored.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(kimi_provider_read(&document).is_none());
        assert!(document.get("default_model").is_none());
        let leftover = document.get("models").map(|models| models.as_table().map(|t| t.iter().any(|(name, _)| name.starts_with("jolu/"))).unwrap_or(false)).unwrap_or(false);
        assert!(!leftover, "不能留下 jolu 模型表：{restored}");
    }

    #[test]
    fn kimi_wire_keeps_user_keys_and_restore_brings_back_their_default() {
        let original = "default_model = \"mine/m1\"\n\
            \n\
            [providers.jolu]\n\
            base_url = \"https://direct\"\n\
            api_key = \"user-key\"\n\
            custom_headers = { X-Flag = \"1\" }\n\
            \n\
            [models.\"mine/m1\"]\n\
            provider = \"mine\"\n\
            model = \"m1\"\n";
        let snapshot = kimi_snapshot(original).unwrap();
        assert_eq!(snapshot["default_model"], "mine/m1", "用户自己的 default_model 要存快照");
        let wired = kimi_wire(original, "http://127.0.0.1:12731", "jolu-kimi.abcd1234", None, &kimi_models(), false).unwrap();
        let document = wired.parse::<toml_edit::DocumentMut>().unwrap();
        let provider = kimi_provider_read(&document).unwrap();
        assert_eq!(kimi_str(provider, "base_url"), "http://127.0.0.1:12731");
        assert_eq!(kimi_str(provider, "api_key"), "jolu-kimi.abcd1234");
        assert_eq!(provider.get("custom_headers").and_then(|h| h.get("X-Flag")).and_then(|v| v.as_str()), Some("1"), "用户自加的键不能丢（magpie #2030）");
        assert!(document.get("models").and_then(|models| models.get("mine/m1")).is_some(), "用户自己的模型表不能动");
        assert_eq!(document.get("default_model").and_then(|v| v.as_str()), Some("jolu/zcode/GLM-5.3"), "接线就把默认模型引到网关");
        // 断开：default_model 还原回用户原值（那张表还在），其余同上
        let restored = kimi_restore(&wired, Some(&snapshot)).unwrap();
        let document = restored.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(document.get("default_model").and_then(|v| v.as_str()), Some("mine/m1"));
        assert!(kimi_provider_read(&document).is_none());
        assert!(document.get("models").and_then(|models| models.get("mine/m1")).is_some());
    }

    #[test]
    fn kimi_wire_legacy_omits_the_keys_old_kimi_cli_cannot_read() {
        let wired = kimi_wire("", "http://127.0.0.1:12731", "t", None, &kimi_models(), true).unwrap();
        // 旧版仍认 image_in（magpie 同款），但不写 tool_use；max_output_size 是新版键
        let first = kimi_table(&wired, "zcode/GLM-5.3");
        let caps: Vec<String> = first.get("capabilities").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        assert_eq!(caps, vec!["image_in".to_string()], "{caps:?}");
        assert!(first.get("max_output_size").is_none(), "旧版没有这个键");
        // 不带图片的模型在旧版下连 capabilities 都不写
        let second = kimi_table(&wired, "grok/grok-4");
        assert!(second.get("capabilities").is_none());
    }

    #[test]
    fn kimi_restore_leaves_a_default_the_user_switched_to() {
        let wired = kimi_wire("", "http://127.0.0.1:12731", "t", None, &kimi_models(), false).unwrap();
        // 接线后用户在 Kimi 里把默认模型换成自己的：断开不能改回去（外部意图优先）
        let user_changed = wired.replace("default_model = \"jolu/zcode/GLM-5.3\"", "default_model = \"mine/m1\"");
        let restored = kimi_restore(&user_changed, Some(&serde_json::json!({"default_model": "mine/m1"}))).unwrap();
        assert_eq!(restored.parse::<toml_edit::DocumentMut>().unwrap().get("default_model").and_then(|v| v.as_str()), Some("mine/m1"));
    }

    #[test]
    fn kimi_restore_drops_a_default_whose_table_is_gone() {
        // 快照里的默认模型指向一张已经不存在的表：不能写出悬空引用（magpie ownModel 判断）
        let wired = kimi_wire("", "http://127.0.0.1:12731", "t", None, &kimi_models(), false).unwrap();
        let restored = kimi_restore(&wired, Some(&serde_json::json!({"default_model": "mine/gone"}))).unwrap();
        assert!(restored.parse::<toml_edit::DocumentMut>().unwrap().get("default_model").is_none());
    }

    #[test]
    fn a_write_that_does_not_parse_is_rolled_back_to_the_previous_content() {
        let dir = std::env::temp_dir().join(format!("jolu-write-verify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"keep":true}"#).unwrap();
        let error = write_atomic(&path, b"{ broken".to_vec()).unwrap_err();
        assert!(error.contains("校验失败") && error.contains("JSON"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"keep":true}"#, "放回原内容");
        write_atomic(&path, br#"{"ok":1}"#.to_vec()).unwrap();
        let toml = dir.join("config.toml");
        assert!(write_atomic(&toml, b"a = [".to_vec()).unwrap_err().contains("TOML"));
        assert!(!toml.exists(), "原来没有的文件写坏了就删掉");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn gemini_takeover_switches_auth_and_restores_it() {
        let env = "# gemini 配置\nOTHER=1\nGOOGLE_GEMINI_BASE_URL=https://old\nGEMINI_API_KEY=user-key\n";
        let settings =
            r#"{"security":{"auth":{"selectedType":"oauth-personal"}},"model":{"name":"gemini-2.5-pro"}}"#;
        let plan = gemini_plan(env, settings, "http://127.0.0.1:12731", "jolu-gemini.abcd1234").unwrap();
        assert!(plan.env.contains("# gemini 配置"), "{}", plan.env);
        assert!(plan.env.contains("OTHER=1"), "{}", plan.env);
        assert!(plan.env.contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:12731"), "{}", plan.env);
        assert!(plan.env.contains("GEMINI_API_KEY=jolu-gemini.abcd1234"), "{}", plan.env);
        let wired_settings: serde_json::Value = serde_json::from_str(&plan.settings).unwrap();
        assert_eq!(wired_settings["security"]["auth"]["selectedType"], "gemini-api-key");
        assert_eq!(wired_settings["model"]["name"], "gemini-2.5-pro", "其他设置不能动");

        let snapshot = plan.snapshot.clone();
        assert_eq!(snapshot["env"]["GEMINI_API_KEY"], "user-key");
        assert_eq!(snapshot["settings"]["selectedType"], "oauth-personal");
        assert_eq!(snapshot["env"]["GOOGLE_GEMINI_BASE_URL"], "https://old");

        let (env, settings) = gemini_restore(&plan.env, &plan.settings, Some(&snapshot));
        assert!(env.contains("GOOGLE_GEMINI_BASE_URL=https://old"), "{env}");
        assert!(env.contains("GEMINI_API_KEY=user-key"), "{env}");
        assert!(env.contains("OTHER=1"), "{env}");
        let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(settings["security"]["auth"]["selectedType"], "oauth-personal");
        assert_eq!(settings["model"]["name"], "gemini-2.5-pro");

        // 原本没有这些键：还原时删除，不能留下 jolu 令牌
        let plan = gemini_plan("OTHER=1\n", "", "http://127.0.0.1:12731", "jolu-gemini.abcd1234").unwrap();
        let (env, settings) = gemini_restore(&plan.env, &plan.settings, Some(&plan.snapshot));
        assert!(!env.contains("GOOGLE_GEMINI_BASE_URL"), "{env}");
        assert!(!env.contains("GEMINI_API_KEY"), "{env}");
        let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
        assert!(settings.pointer("/security/auth/selectedType").is_none(), "{settings}");

        // 旧版快照只有 value：只还原地址，不改 settings
        let plan = gemini_plan(env.as_str(), settings.to_string().as_str(), "http://127.0.0.1:12731", "jolu-gemini.abcd1234").unwrap();
        let (_, legacy_settings) = gemini_restore(&plan.env, &plan.settings, Some(&serde_json::json!({"value":"https://legacy"})));
        assert_eq!(legacy_settings, plan.settings, "旧快照不能改 auth");
    }

    #[test]
    fn gemini_refuses_invalid_settings_instead_of_wiping_it() {
        assert!(gemini_plan("A=1\n", "{not json", "http://127.0.0.1:12731", "jolu-gemini.abcd1234").is_err());
        let (_, settings) = gemini_restore(
            "A=1\n",
            "{not json",
            Some(&serde_json::json!({"env": {}, "settings": {"selectedType": "oauth-personal"}})),
        );
        assert_eq!(settings, "{not json", "坏文件不能被覆盖");
    }

    #[test]
    fn gateway_pointing_check_matches_only_our_port() {
        assert!(points_at_gateway("http://127.0.0.1:12731", 12731));
        assert!(points_at_gateway("http://127.0.0.1:12731/", 12731));
        assert!(points_at_gateway("http://localhost:12731", 12731));
        assert!(!points_at_gateway("http://127.0.0.1:12732", 12731));
        assert!(!points_at_gateway("https://api.anthropic.com", 12731));
    }

    #[test]
    fn token_carries_instance_suffix() {
        assert_eq!(token_for("claude-code", "abcd1234"), "jolu-claude-code.abcd1234");
        assert_eq!(token_for("codex", "abcd1234"), "jolu-codex.abcd1234");
    }

    #[test]
    fn rewiring_keeps_the_original_snapshot() {
        let computed = Some(serde_json::json!({"ANTHROPIC_BASE_URL":"https://current"}));
        assert_eq!(
            effective_snapshot(Some(r#"{"ANTHROPIC_BASE_URL":"https://original"}"#), computed.clone()).as_deref(),
            Some(r#"{"ANTHROPIC_BASE_URL":"https://original"}"#),
            "重新应用不能拿当前文件重新快照"
        );
        assert_eq!(
            effective_snapshot(None, computed).as_deref(),
            Some(r#"{"ANTHROPIC_BASE_URL":"https://current"}"#)
        );
        assert_eq!(effective_snapshot(None, None), None);
    }

    #[test]
    fn foreign_marker_rejects_another_instance() {
        let text = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code.ffffffff"}}"#;
        assert_eq!(foreign_marker("claude-code", text, "abcd1234").as_deref(), Some("ffffffff"));
        assert_eq!(foreign_marker("claude-code", text, "ffffffff"), None);
        // 旧版没有实例后缀：按本实例处理，允许覆盖
        let legacy = r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"jolu-claude-code"}}"#;
        assert_eq!(foreign_marker("claude-code", legacy, "abcd1234"), None);
    }

    #[test]
    fn agent_whitelist_covers_catalog_platforms() {
        // 全部 14 家（含 antigravity / qwen / workbuddy），一家都不能漏
        for agent in [
            "claude-code", "codex", "zcode", "gemini", "opencode", "kimi", "dsh", "pi",
            "pencil", "qoder", "qoder-cn", "qwen", "antigravity", "workbuddy",
        ] {
            assert!(is_agent(agent), "{agent} 应可接线");
        }
        // 5.11：Qoder CLI 接入的是 Jolu 网关令牌（写进它的 settings.json），不代管 Qoder 账号的密钥。
        // 2026-11-21：WorkBuddy 的 models.json 是顶层数组，已按 magpie `buddyWrite` 实现逐项插入
        // 与 disabled 状态保留（见 `wiring_workbuddy.rs`），它现在可接线；改用 Cline 锁「不接」。
        assert!(!is_agent("cline"), "Cline 要写系统凭据库，仍不接");
    }

    /// 文件级链路：接线 → 状态探测为已接线 → 还原 → 状态回到未接线。
    /// 用临时目录注入配置路径，不碰真实用户配置。
    #[test]
    fn file_level_wire_status_unwire_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "jolu-wiring-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("settings.json");
        std::fs::write(&config, r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"},"keep":1}"#).unwrap();
        let backups = dir.join("backups");

        // 直接调纯函数模拟 wire/unwire（真实 wire() 依赖用户目录，测不到临时路径）
        let original = std::fs::read_to_string(&config).unwrap();
        let snapshot = claude_snapshot(&original, None, None, &[]).unwrap();
        let wired = claude_wire(&original, "http://127.0.0.1:12731", "jolu-claude-code.abcd1234", None, None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        std::fs::write(&config, &wired).unwrap();
        assert!(ours("claude-code", &wired, 12731, "abcd1234"));
        assert!(!ours("claude-code", &wired, 12732, "abcd1234"));

        // 还原
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        std::fs::write(&config, &restored).unwrap();
        let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://api.anthropic.com", "还原回原上游");
        assert_eq!(value["keep"], 1, "无关字段必须保留");

        // 备份目录可创建（真实 wire 会用到）
        std::fs::create_dir_all(&backups).unwrap();
        assert!(backups.is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 外部已改：还原前发现当前值不是网关地址 → 返回 false 且不改文件。
    #[test]
    fn unwire_skips_when_external_value_took_over() {
        let external = r#"{"env":{"ANTHROPIC_BASE_URL":"https://custom.example"}}"#;
        let current = claude_current_base(external);
        assert!(
            current.is_some_and(|c| !points_at_gateway(&c, 12731) && !is_gateway_value(&c)),
            "外部值不应被判定为网关地址，因此还原会跳过"
        );
        let ours_text = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code.abcd1234"}}"#;
        assert!(ours("claude-code", ours_text, 12731, "abcd1234"));
    }

    // ─────────────────── 全部可接线工具的全量往返（子进程 + 临时 home） ───────────────────
    //
    // 为什么必须是子进程：`plan_wire` / `plan_unwire` 走 `creds::home()`（读 `USERPROFILE`
    // / `HOME`），直接在本进程里改这些环境变量会污染并行运行的其他测试（进程级状态，
    // 且 `set_var` 在多线程下未定义行为）。所以父测试只负责起子进程，子进程把 home 指到
    // 临时目录后再跑生产函数，桌面入口和真实工具仍需另行验证。
    //
    // 为什么只读真机、不点桌面：真机桌面实例没有重定向 home，点接入会写用户的真实配置。
    // 现有 `agent-icons-kimi-preview.e2e.mjs` 因此只做「写入预览」不做往返。这里补的正是
    // 那条缺口：覆盖注册表里**每一家**的写入 → 状态 → 还原闭环（家数由 `agents()` 决定，
    // 新增一家自动纳入——没有 `seed_original` 样本时测试会 panic 提示补上）。

    /// 父：给每家起一个干净的子进程（各自临时 home），子进程失败时把 stderr 带出来。
    #[test]
    fn every_agent_round_trips_through_production_plan() {
        if std::env::var_os("JOLU_WIRING_ROUNDTRIP_CHILD").is_some() {
            return;
        }
        let mut failed = Vec::new();
        for agent in agents() {
            let root = std::env::temp_dir().join(format!(
                "jolu-roundtrip-{}-{}",
                agent,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let home = root.join("home");
            std::fs::create_dir_all(&home).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    &format!("{}::round_trip_one_agent_child", module_path!().trim_start_matches("jolu_lib::")),
                    "--nocapture",
                ])
                .env("JOLU_WIRING_ROUNDTRIP_CHILD", "1")
                .env("JOLU_ROUNDTRIP_AGENT", agent)
                .env("JOLU_ROUNDTRIP_HOME", &home)
                // 把 home 与所有能改配置落点的变量一起指进临时目录，避免任何一家
                // 顺着真实环境变量写出去（各家覆盖变量见 `wiring_specs` 的 path_* 与各 dir 函数）
                .env("USERPROFILE", &home)
                .env("HOME", &home)
                .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
                .env("CODEX_HOME", home.join(".codex"))
                .env("XDG_CONFIG_HOME", home.join(".config"))
                .env("KIMI_CODE_HOME", home.join(".kimi-code"))
                .env("KIMI_SHARE_DIR", home.join(".kimi-share"))
                .env("ZCODE_HOME", home.join(".zcode"))
                .env("DSH_HOME", home.join(".dsh"))
                .env("PI_CODING_AGENT_DIR", home.join(".pi/agent"))
                .env("QODER_CONFIG_DIR", home.join(".qoder"))
                .env("QODERCN_CONFIG_DIR", home.join(".qoder-cn"))
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if !output.status.success() {
                failed.push(format!("{agent}：\n{stderr}"));
            }
            let _ = std::fs::remove_dir_all(&root);
        }
        assert!(failed.is_empty(), "以下工具的往返失败：\n{}", failed.join("\n\n"));
    }

    /// 子进程：对一家工具跑接线、状态检查和配置还原。
    ///
    /// 使用实际保存的接线记录，并检查计划涉及的所有文件。
    #[test]
    fn round_trip_one_agent_child() {
        let Ok(agent) = std::env::var("JOLU_ROUNDTRIP_AGENT") else { return };
        let Ok(home) = std::env::var("JOLU_ROUNDTRIP_HOME").map(std::path::PathBuf::from) else { return };
        let agent = agent.as_str();
        let port = 12731u16;
        let instance = "abcd1234";
        let base = format!("http://127.0.0.1:{port}");

        // 1. 造一份包含用户原有配置的样本。
        let path = config_path(agent).unwrap_or_else(|| panic!("{agent} 算不出配置路径"));
        seed_original(agent, &home, &path);
        // dsh 的主文件会随已有 profile 改变，准备样本后重新取得。
        let path = config_path(agent).unwrap();

        // 2. 走生产 plan_wire
        let catalog = vec![
            ZcodeModel { id: "zcode/glm-5.3".into(), name: "GLM-5.3".into(), context: 200_000, output: 32_768, image: true },
            ZcodeModel { id: "kimi/k2".into(), name: "Kimi K2".into(), context: 256_000, output: 32_768, image: false },
        ];
        let chosen = ModelChoice {
            model: Some("zcode/glm-5.3"),
            label: Some("GLM-5.3"),
            description: None,
            prior_model: None,
            prior_label: None,
            prior_values: &[],
            subagent_model: None,
            tiers: None,
        };
        let ctx = WireContext {
            agent,
            base: base.clone(),
            token: token_for(agent, instance),
            instance_short: instance,
            catalog: &catalog,
            prior_snapshot: None,
            chosen,
            claude: None,
        };
        let plan = plan_wire(&ctx).unwrap_or_else(|error| panic!("{agent} 生成接线计划失败：{error}"));
        assert!(!plan.files.is_empty(), "{agent} 的计划里没有文件");
        let originals: Vec<_> = plan.files.iter().map(|(file, _)| (file.clone(), std::fs::read(file).ok())).collect();

        // 3. 落盘（与 wire_agents 同一条 commit_plan）
        let backup_dir = home.join("gateway-backups");
        let conn = crate::db::open(Path::new(":memory:")).unwrap();
        commit_plan(plan, &backup_dir, |snapshot, applied| {
            super::store::mark_pending(&conn, agent, instance, "relay", snapshot, applied, applied.token.as_deref())
                .map_err(|error| error.to_string())
        }).unwrap_or_else(|error| panic!("{agent} 写盘失败：{error}"));
        super::store::mark_wired(&conn, agent, 42).unwrap();

        // 4. 状态必须是「已接线」（走注册表的 is_wired，这条同时验 token_match / extra）
        let recorded = super::store::load(&conn, agent).unwrap();
        let status = status_with(agent, port, Some(&recorded));
        assert!(status.wired, "{agent} 写完配置文件后 status 仍报未接线：{:?}", status.reason);
        let wired_text = std::fs::read_to_string(&path).unwrap();
        if (super::specs::spec(agent).unwrap().token)(&home, &wired_text).is_some() {
            assert_eq!(foreign_marker(agent, &wired_text, "ffffffff").as_deref(), Some(instance));
            assert_eq!(foreign_marker(agent, &wired_text, instance), None);
        }

        // 5. 断开：用真实记录（含快照）走 plan_unwire，再逐文件应用
        assert!(unwire(agent, port, Some(&recorded)).unwrap_or_else(|error| panic!("{agent} 还原失败：{error}")));
        super::store::mark_idle(&conn, agent).unwrap();

        // 6. 还原后必须把用户的配置还回来
        //
        // 判据是**语义等价**，不是字节相等：生产实现走 `serde_json::Value` / `toml_edit`
        // 解析后重新序列化，键序与缩进会规范化（原始 `{"a":1,"b":2}` 会变成 pretty 排版
        // 且按字母序）。这是既有行为、不是本次重构引入，也不影响任何工具读取。
        // 所以这里比「解析后的值相等」+「用户原始键都在」+「我们的痕迹走干净」三件事。
        for (file, original) in originals {
            let restored = std::fs::read(&file).ok();
            match (original, restored) {
                (Some(before), Some(after)) => {
                    let before_text = String::from_utf8_lossy(&before).to_string();
                    let after_text = String::from_utf8_lossy(&after).to_string();
                    if file.extension().is_some_and(|ext| ext == "yml" || ext == "yaml") {
                        assert_eq!(before_text, after_text, "{agent} 文件 {} 未还原", file.display());
                    } else {
                        assert_semantically_restored(agent, &before_text, &after_text);
                    }
                }
                (None, after) => {
                    // 原本没有这个文件：还原后也不该留下我们的东西
                    if let Some(after) = after {
                        let text = String::from_utf8_lossy(&after);
                        assert!(
                            !text.contains("12731") && !text.contains("jolu-"),
                            "{agent} 原本没有配置文件，还原后却残留了网关内容：{text}"
                        );
                    }
                }
                (Some(_), None) => panic!("{agent} 原本有配置文件，还原后文件被删了"),
            }
        }

        // 7. 还原后状态回到未接线
        let after_status = status_with(agent, port, None);
        assert!(!after_status.wired, "{agent} 还原后 status 仍报已接线");
        if agent == "dsh" {
            let plan = plan_wire(&ctx).unwrap();
            commit_plan(plan, &backup_dir, |snapshot, applied| {
                super::store::mark_pending(&conn, agent, instance, "relay", snapshot, applied, applied.token.as_deref())
                    .map_err(|error| error.to_string())
            }).unwrap();
            let dsh_home = super::dsh::dsh_dir_public(&home);
            std::fs::write(dsh_home.join(".env"), "JOLU_API_KEY=changed-env\n").unwrap();
            std::fs::write(dsh_home.join(".credentials.yaml"), "version: 1\nrefs:\n  JOLU_API_KEY: changed-creds\n").unwrap();
            let record = super::store::load(&conn, agent).unwrap();
            assert!(unwire(agent, port, Some(&record)).unwrap());
            assert!(std::fs::read_to_string(dsh_home.join(".env")).unwrap().contains("JOLU_API_KEY=changed-env"));
            assert!(std::fs::read_to_string(dsh_home.join(".credentials.yaml")).unwrap().contains("JOLU_API_KEY: changed-creds"));
        }
    }

    /// 还原是否「把用户的东西还回来了」：按格式解析后比较值，并额外确认
    /// 网关地址 / Jolu 令牌 / 我们建的 provider 段都不在了。
    ///
    /// 为什么分格式：JSON 与 TOML 的解析器都在生产依赖里（`serde_json` / `toml_edit`），
    /// 直接用它们比较就等价于「工具读到的内容一致」，比字符串比对更贴近真实语义。
    /// `.env` 没有解析库也不需要：按行取键值比就行（注释行与空行不参与比较）。
    fn assert_semantically_restored(agent: &str, before: &str, after: &str) {
        let trimmed = before.trim_start();
        // 判格式：以 { / [ 开头是 JSON；否则看是不是「每行 KEY=VALUE」的 dotenv
        // （gemini 的 .env）；剩下按 TOML 处理。
        let format = if trimmed.starts_with('{') || trimmed.starts_with('[') {
            "json"
        } else if is_dotenv(before) {
            "dotenv"
        } else {
            "toml"
        };
        match format {
            "json" => {
                let expected: serde_json::Value = serde_json::from_str(before)
                    .unwrap_or_else(|error| panic!("{agent} 原始配置不是有效 JSON：{error}"));
                let actual: serde_json::Value = serde_json::from_str(after)
                    .unwrap_or_else(|error| panic!("{agent} 还原结果不是有效 JSON：{error}\n{after}"));
                assert_eq!(expected, actual, "{agent} 还原后的值与原始配置不一致\n原始：{before}\n还原：{after}");
            }
            "dotenv" => {
                // .env 按行取键值比较：注释与空行不参与（生产写入只动目标键，注释原样保留）
                let parse = |text: &str| -> std::collections::BTreeMap<String, String> {
                    text.lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty() && !line.starts_with('#'))
                        .filter_map(|line| line.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                        .collect()
                };
                assert_eq!(parse(before), parse(after), "{agent} 还原后 .env 键值不一致\n原始：{before}\n还原：{after}");
            }
            _ => {
                let expected: toml_edit::DocumentMut = before
                    .parse()
                    .unwrap_or_else(|error| panic!("{agent} 原始配置不是有效 TOML：{error}"));
                let actual: toml_edit::DocumentMut = after
                    .parse()
                    .unwrap_or_else(|error| panic!("{agent} 还原结果不是有效 TOML：{error}\n{after}"));
                assert_eq!(
                    expected.to_string().trim(),
                    actual.to_string().trim(),
                    "{agent} 还原后的值与原始配置不一致\n原始：{before}\n还原：{after}"
                );
            }
        }
        // 不管哪种格式，网关痕迹都必须走干净
        assert!(!after.contains("127.0.0.1:12731"), "{agent} 还原后仍指向网关：{after}");
        assert!(!after.contains("jolu-"), "{agent} 还原后仍留着 Jolu 令牌：{after}");
    }

    /// 子进程用：判断一段文本是不是 dotenv（每行 `KEY=VALUE` 或注释）。
    ///
    /// 只看第一行有效内容：TOML 的键值行不会出现「裸 `=` 而无引号且键名是大写常量」的形态，
    /// 而 `.env` 全是这个形态，足够区分。
    fn is_dotenv(text: &str) -> bool {
        let Some(first) = text.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#')) else {
            return false;
        };
        let Some((key, _)) = first.split_once('=') else { return false };
        !key.is_empty()
            && key.chars().all(|c| c.is_ascii_uppercase() || c == '_')
            && !first.contains('"')
            && !first.contains('\'')
    }

    /// 子进程用：给一家工具写一份「用户原有配置」（含干扰项），让还原有东西可还。
    fn seed_original(agent: &str, home: &Path, path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let text = match agent {
            "claude-code" => r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com","ANTHROPIC_AUTH_TOKEN":"sk-mine"},"theme":"dark"}"#,
            "codex" => "model = \"gpt-5\"\nmodel_provider = \"openai\"\n",
            "zcode" => {
                std::fs::write(path.parent().unwrap().join("config.json"), r#"{"keep":1}"#).unwrap();
                r#"{"config":{"providerConfigRules":{"providerRules":[]}},"keep":1}"#
            }
            "gemini" => {
                std::fs::write(path.parent().unwrap().join("settings.json"), r#"{"security":{"auth":{"selectedType":"oauth-personal"}},"keep":true}"#).unwrap();
                "GOOGLE_GEMINI_BASE_URL=https://generativelanguage.googleapis.com\nGEMINI_API_KEY=my-own-key\n"
            }
            "opencode" => r#"{"theme":"x","provider":{"mine":{"options":{"baseURL":"https://a"}}}}"#,
            "kimi" => {
                // default_model 必须指向一个**存在**的表：生产语义是「原表还在才还原默认」，
                // 指向不存在的表时它会按兜底删掉 default_model（那是另一个分支，另有用例覆盖）。
                "default_model = \"moonshot/kimi-k2\"\n[providers.moonshot]\nbase_url = \"https://api.moonshot.cn/v1\"\napi_key = \"sk-mine\"\n[models.\"moonshot/kimi-k2\"]\nmodel = \"kimi-k2\"\nmax_context_size = 256000\n"
            }
            "pi" => {
                // 双文件：models.json 与 settings.json 都要有原值
                std::fs::write(
                    crate::gateway::wiring_pi::settings_path(home),
                    r#"{"defaultProvider":"anthropic","defaultModel":"claude-sonnet-4"}"#,
                ).unwrap();
                r#"{"providers":{"anthropic":{"baseUrl":"https://api.anthropic.com/v1","apiKey":"sk-mine"}}}"#
            }
            "pencil" | "qoder" | "qoder-cn" => r#"{"providers":{"mine":{"baseUrl":"https://a"}},"model":{"name":"qwen3-coder","reasoningEffort":"high"}}"#,
            "dsh" => {
                // 多文件：一个 profile 的 patch（带注释，验行级编辑不丢注释）
                let dsh_home = crate::gateway::wiring_dsh::dsh_dir_public(home);
                let patch_dir = dsh_home.join("profiles").join("web");
                std::fs::create_dir_all(&patch_dir).unwrap();
                std::fs::write(
                    patch_dir.join("cordis.patch.yml"),
                    "# 用户自己的注释要保住\n- id: my-own-entry\n  name: \"@x/y\"\n  config:\n    foo: 1\n",
                ).unwrap();
                // .env 也放一个用户自己的变量
                std::fs::write(dsh_home.join(".env"), "MY_OWN_KEY=abc\nJOLU_API_KEY=my-env-key\n").unwrap();
                std::fs::write(dsh_home.join(".credentials.yaml"), "version: 1\nrefs:\n  JOLU_API_KEY: \"my: creds key\"\n  OTHER: keep\nrecords:\n  - id: 1\n").unwrap();
                let second = dsh_home.join("profiles").join("other");
                std::fs::create_dir_all(&second).unwrap();
                std::fs::write(second.join("cordis.patch.yml"), "# 另一个 profile\n- id: keep\n  name: user\n").unwrap();
                return;
            }
            "antigravity" => {
                // 用户自己的自定义模型 + 非 gemini 的 modelProvider；断开要原样还回来
                r#"{"customModelsConfig":{"customModels":{"mine":{"apiProvider":"API_PROVIDER_GOOGLE_GEMINI","modelName":"gemini-3-pro"}}},"model":"mine","modelProvider":"vertex","trustedWorkspaces":["/w"]}"#
            }
            "qwen" => {
                // 用户自己的一条 provider + 自己的 env 键 + 自己的默认模型
                r#"{"modelProviders":{"openai":[{"baseUrl":"https://my-own","envKey":"MY_KEY","id":"my-model","name":"My Model"}]},"env":{"MY_KEY":"sk-mine"},"model":{"name":"my-model","baseUrl":"https://my-own"},"keep":1}"#
            }
            "workbuddy" => {
                // 顶层数组：用户自己的一条 + 一个关掉的开关
                r#"[{"id":"my-own","name":"My Own","vendor":"Custom","apiKey":"sk-mine","url":"https://my-own/v1/chat/completions"},{"id":"off-one","name":"Off","vendor":"Custom","apiKey":"sk-2","url":"https://off","disabled":true}]"#
            }
            _ => panic!("{agent} 没有原始配置样本：给 `seed_original` 补一份"),
        };
        std::fs::write(path, text).unwrap();
    }

