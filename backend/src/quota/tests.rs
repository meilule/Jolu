#![allow(unused_imports)]
use super::*;

#[cfg(test)]
mod codex_tests {
    use super::*;
    #[test]
    #[ignore = "需要本机 Codex Auth 和网络；仅显式执行"]
    fn local_auth_read_only_smoke() {
        match local_codex_quota() {
            QuotaState::Ok { windows, .. } => {
                assert!(!windows.is_empty());
                for w in windows { println!("{} used={:?} has_reset={}", w.key, w.used_percent, w.resets_at.is_some()); }
            }
            other => panic!("额度查询未成功: {other:?}"),
        }
    }
    #[test]
    fn reset_credit_summary_preserves_zero_and_missing_counts() {
        let result = parse_codex_usage(r#"{"plan_type":"pro","rate_limit":{"primary_window":{"used_percent":0,"limit_window_seconds":18000}},"rate_limit_reset_credits":{"available_count":1,"applicable_available_count":0}}"#);
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["account"]["reset_credits"]["available_count"], 1);
        assert_eq!(json["account"]["reset_credits"]["applicable_available_count"], 0);
        assert_eq!(json["account"]["subscription_status"], "unknown");
    }

    #[test]
    fn malformed_optional_reset_metadata_does_not_hide_valid_quota() {
        for credits in ["null", "[]", r#"{"available_count":-1}"#, r#"{"available_count":"unknown"}"#] {
            let body = format!(r#"{{"rate_limit":{{"primary_window":{{"used_percent":0,"limit_window_seconds":18000}}}},"rate_limit_reset_credits":{credits}}}"#);
            let QuotaState::Ok { account: Some(account), windows, .. } = parse_codex_usage(&body) else { panic!("额度不应被可选元数据拖累") };
            assert!(account.reset_credits.is_none());
            assert_eq!(windows[0].used_percent, Some(0.0));
        }
        let result = ResetCredits::parse(&serde_json::json!({"available_count":0})).unwrap();
        assert_eq!(result.available_count, Some(0));
        assert_eq!(result.applicable_available_count, None);
    }

    #[test]
    fn weekly_primary_is_not_mislabeled_as_five_hours() {
        let QuotaState::Ok { windows, plan, .. } = parse_codex_usage(r#"{"plan_type":"pro","rate_limit":{"primary_window":{"used_percent":56,"limit_window_seconds":604800,"reset_at":1789805546},"secondary_window":null}}"#) else { panic!("解析失败") };
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].key, "7d");
        assert_eq!(windows[0].used_percent, Some(56.0));
        assert!(windows[0].resets_at.is_some());
        assert_eq!(plan.as_deref(), Some("pro"));
    }

    #[test]
    fn dual_windows_preserve_real_zero_and_unknown_values() {
        let QuotaState::Ok { windows, .. } = parse_codex_usage(r#"{"rate_limit":{"primary_window":{"used_percent":0,"limit_window_seconds":18000,"reset_at":null},"secondary_window":{"used_percent":null,"limit_window_seconds":604800,"reset_at":null}}}"#) else { panic!("解析失败") };
        assert_eq!(windows[0].key, "5h");
        assert_eq!(windows[0].used_percent, Some(0.0));
        assert_eq!(windows[1].key, "7d");
        assert_eq!(windows[1].used_percent, None);
    }
}

#[cfg(test)]
mod official_connection_tests {
    use super::*;

    #[test]
    #[ignore = "需要用户授权读取本机 Claude API 配置及其实际网关，仅输出状态"]
    fn local_claude_gateway_read_only_smoke() {
        let key = crate::creds::read_secret("claude", "api").expect("缺少本机 Claude Key");
        let base = crate::creds::local_base_url("claude", "api").expect("当前测试要求配置网关");
        let data = fetch_s2(&base, &key).unwrap_or_else(|(code, _)| panic!("HTTP/parse status {code}"));
        assert!(s2_can_validate(&data), "未返回可识别的数据");
        assert!(data.balance.is_some_and(f64::is_finite), "余额字段未知");
        println!("Claude gateway: validated=true; balance_present=true; currency={}", data.unit.as_deref().unwrap_or("unknown"));
        assert!(matches!(s2_quota_from_usage(data), QuotaState::Ok { .. } | QuotaState::Unsupported { .. }));
    }

    #[test]
    fn wallet_only_gateway_is_valid_without_quota_windows() {
        let data: S2Usage = serde_json::from_str(r#"{"balance":0,"unit":"USD","isValid":true}"#).unwrap();
        assert!(s2_can_validate(&data));
        assert!(matches!(s2_quota_from_usage(data), QuotaState::Unsupported { .. }));
        let invalid: S2Usage = serde_json::from_str(r#"{"balance":20,"isValid":false}"#).unwrap();
        assert!(!s2_can_validate(&invalid));
        assert!(!s2_can_validate(&serde_json::from_str("{}").unwrap()));
    }

    #[test]
    fn sub2api_missing_usage_stays_unknown_and_expiry_is_not_a_reset() {
        let usage: S2Usage = serde_json::from_str(r#"{
          "unit":"USD",
          "subscription":{
            "daily_limit_usd":10,
            "weekly_usage_usd":0,
            "weekly_limit_usd":70,
            "expires_at":"2026-10-01T00:00:00Z"
          }
        }"#).unwrap();
        let QuotaState::Ok { windows, account, .. } = s2_quota_from_usage(usage) else {
            panic!("应生成订阅窗口")
        };
        assert_eq!(windows[0].used_percent, None);
        assert_eq!(windows[0].amount_text.as_deref(), Some("— / 10.00 USD"));
        assert_eq!(windows[0].resets_at, None);
        assert_eq!(windows[1].used_percent, Some(0.0));
        assert_eq!(windows[1].resets_at, None);
        let account = account.expect("订阅到期元数据不能丢失");
        assert_eq!(account.expires_at.as_deref(), Some("2026-10-01T00:00:00+00:00"));
        assert_eq!(account.subscription_source, "live");
        assert_eq!(account.subscription_status, "unknown");
    }

    #[test]
    fn sub2api_invalid_expiry_is_not_used_as_a_subscription_date() {
        let usage: S2Usage = serde_json::from_str(r#"{"subscription":{"expires_at":"invalid"}}"#).unwrap();
        let QuotaState::Ok { account: Some(account), .. } = s2_quota_from_usage(usage) else { panic!() };
        assert_eq!(account.expires_at, None);
    }

    #[test]
    fn gateway_without_billing_endpoint_reports_unsupported_not_failure() {
        // 404 = 网关没有该端点（Claude 式反代、本地中转都没有账单接口），
        // 属于「不支持」而非「查询失败」，否则用户会误以为是配置错误
        let reason = "该网关未提供 sub2api 账单接口（HTTP 404）".to_string();
        assert!(matches!(s2_balance_err(404, reason.clone()), BalanceState::Unsupported { .. }));
        assert!(matches!(s2_quota_err(404, reason), QuotaState::Unsupported { .. }));
        // 其余错误码语义保持不变
        assert!(matches!(s2_balance_err(500, "服务端错误".into()), BalanceState::Failed { .. }));
        assert!(matches!(s2_quota_err(500, "服务端错误".into()), QuotaState::Failed { .. }));
        assert!(matches!(s2_balance_err(401, "凭证失效".into()), BalanceState::Unauthorized { .. }));
        assert!(matches!(s2_balance_err(429, "限频".into()), BalanceState::RateLimited { .. }));
    }

    #[test]
    fn four_official_connection_modes_have_separate_capabilities() {
        assert_eq!(
            official_capabilities("claude", "auth"),
            OfficialCapabilities { quota: true, admin_usage: false, local_sessions: true }
        );
        assert_eq!(
            official_capabilities("claude", "api"),
            OfficialCapabilities { quota: false, admin_usage: true, local_sessions: true }
        );
        assert_eq!(
            official_capabilities("codex", "auth"),
            OfficialCapabilities { quota: true, admin_usage: false, local_sessions: true }
        );
        assert_eq!(
            official_capabilities("codex", "api"),
            OfficialCapabilities { quota: false, admin_usage: true, local_sessions: true }
        );
    }

    #[test]
    fn anthropic_admin_usage_counts_each_input_class_once() {
        let parsed = parse_anthropic_admin_usage(r#"{
          "data": [{"results": [{
            "uncached_input_tokens": 1500,
            "cache_read_input_tokens": 200,
            "cache_creation": {
              "ephemeral_1h_input_tokens": 30,
              "ephemeral_5m_input_tokens": 70
            },
            "output_tokens": 500
          }]}]
        }"#).expect("valid Anthropic usage response");

        assert_eq!(parsed.input_tokens, 1800);
        assert_eq!(parsed.output_tokens, 500);
        assert_eq!(parsed.cache_read_tokens, 200);
        assert_eq!(parsed.cache_write_tokens, 100);
        assert_eq!(parsed.total_tokens, 2300);
    }

    #[test]
    fn openai_admin_usage_does_not_add_cached_input_twice() {
        let parsed = parse_openai_admin_usage(r#"{
          "data": [{"results": [{
            "input_tokens": 1000,
            "output_tokens": 500,
            "input_cached_tokens": 800
          }]}]
        }"#).expect("valid OpenAI usage response");

        assert_eq!(parsed.input_tokens, 1000);
        assert_eq!(parsed.output_tokens, 500);
        assert_eq!(parsed.cache_read_tokens, 800);
        assert_eq!(parsed.cache_write_tokens, 0);
        assert_eq!(parsed.total_tokens, 1500);
    }

    #[test]
    fn admin_cost_parsers_keep_provider_units_straight() {
        let anthropic = parse_anthropic_admin_cost(r#"{
          "data": [{"results": [{"amount": "123.45", "currency": "USD"}]}]
        }"#).expect("valid Anthropic cost response");
        let openai = parse_openai_admin_cost(r#"{
          "data": [{"results": [{"amount": {"value": 1.2345, "currency": "usd"}}]}]
        }"#).expect("valid OpenAI cost response");

        assert!((anthropic - 1.2345).abs() < f64::EPSILON);
        assert!((openai - 1.2345).abs() < f64::EPSILON);
    }
    #[test]
    fn malformed_admin_responses_are_not_reported_as_zero_usage_or_cost() {
        assert!(matches!(denied_admin_usage(HttpOutcome::Body("{\"data\":[]}".into())), ApiUsageState::Forbidden { .. }));
        assert!(matches!(denied_admin_usage(HttpOutcome::Unauthorized("expired".into())), ApiUsageState::Unauthorized { .. }));
        assert!(matches!(denied_admin_usage(HttpOutcome::Failed("offline".into())), ApiUsageState::Failed { .. }));
        assert!(matches!(denied_admin_usage(HttpOutcome::Body("{}".into())), ApiUsageState::Failed { .. }));
        for body in ["{}", "{\"data\":[{}]}", "{\"data\":[{\"results\":[{}]}]}", "{\"data\":[],\"has_more\":true}"] {
            assert!(parse_anthropic_admin_usage(body).is_err(), "Claude usage accepted {body}");
            assert!(parse_openai_admin_usage(body).is_err(), "OpenAI usage accepted {body}");
            assert!(parse_anthropic_admin_cost(body).is_err(), "Claude cost accepted {body}");
            assert!(parse_openai_admin_cost(body).is_err(), "OpenAI cost accepted {body}");
        }
        assert_eq!(parse_openai_admin_usage("{\"data\":[]}").unwrap().total_tokens, 0);
        assert_eq!(parse_anthropic_admin_cost("{\"data\":[]}").unwrap(), 0.0);
        assert!(matches!(parse_codex_usage("{}"), QuotaState::Failed { .. }));
        assert!(validate_models_body("{}").is_err());
        assert!(validate_models_body("{\"error\":\"invalid key\"}").is_err());
        assert!(validate_models_body("{\"data\":[]}").is_ok());
        assert_eq!(pct(-1.0, 100.0), None);
        assert_eq!(pct(0.0, 100.0), Some(0.0));
        assert!(parse_openai_admin_cost("{\"data\":[{\"results\":[{\"amount\":{\"value\":-1}}]}]}").is_err());
    }

}

// 任务 3.4 开始：自定义供应商的保存校验
#[cfg(test)]
mod custom_validation_tests {
    use super::*;

    fn no_quota() -> Result<(), String> {
        panic!("自定义供应商不应调用额度或余额接口")
    }

    #[test]
    fn custom_validation_uses_models() {
        assert_eq!(validate_connection_with("custom", "api", Some("gpt-4.1-mini"), no_quota), Ok(()));
        // 没有获取到模型、也没有手填模型时不能保存，同样不去查额度
        assert!(validate_connection_with("custom", "api", None, no_quota).is_err());
        assert!(validate_connection_with("custom", "api", Some("  "), no_quota).is_err());
        assert!(validate_connection_with("custom", "auth", Some("m"), no_quota).is_err());
    }

    #[test]
    fn other_platforms_keep_remote_validation() {
        assert_eq!(validate_connection_with("codex", "api", Some("m"), || Err("远程校验".into())), Err("远程校验".into()));
        assert_eq!(validate_connection_with("claude", "api", None, || Ok(())), Ok(()));
    }
}
// 任务 3.4 结束
